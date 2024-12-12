use std::collections::BTreeMap;
use std::convert::From;
use std::ops::IndexMut;
use std::sync::{Arc, Mutex};
use crossbeam_channel::TrySendError;
use jack::{AudioOut, Client, ClientStatus, Control, Frames, MidiIn, MidiOut, NotificationHandler, Port, PortId, ProcessHandler, ProcessScope, RawMidi};
use rb::RbConsumer;
use vst::api::{TimeInfo, TimeInfoFlags};
use vst::event::MidiEvent;

use log::*;

use crate::domain::{AudioBlock, TRANSPORT};
use crate::{AudioConsumerDetails, AudioLayerInwardEvent, AudioLayerOutwardEvent, DAWUtils, MidiConsumerDetails, SampleData, TrackBackgroundProcessorMode};
use crate::event::AudioLayerTimeCriticalOutwardEvent;

const MAX_MIDI: usize = 3;


const CHANNELS: usize = 2;
const FRAMES: u32 = 64;
const SAMPLE_HZ: f64 = 44_100.0;

#[derive(Copy, Clone)]
pub struct MidiCopy {
    len: usize,
    data: [u8; MAX_MIDI],
    time: Frames,
}

impl From<RawMidi<'_>> for MidiCopy {
    fn from(midi: RawMidi<'_>) -> Self {
        let len = std::cmp::min(MAX_MIDI, midi.bytes.len());
        let mut data = [0; MAX_MIDI];
        data[..len].copy_from_slice(&midi.bytes[..len]);
        MidiCopy {
            len,
            data,
            time: midi.time,
        }
    }
}

impl std::fmt::Debug for MidiCopy {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "Midi {{ time: {}, len: {}, data: {:?} }}",
            self.time,
            self.len,
            &self.data[..self.len]
        )
    }
}

pub struct JackNotificationHandler {
    jack_midi_sender: crossbeam_channel::Sender<AudioLayerOutwardEvent>,
}

impl JackNotificationHandler {
    pub fn new(jack_midi_sender: crossbeam_channel::Sender<AudioLayerOutwardEvent>) -> Self {
        JackNotificationHandler {
            jack_midi_sender,
        }
    }

    fn notify_about_connected_ports(&mut self, port_a_name: &String, port_b_name: &String) {
        match self.jack_midi_sender.try_send(AudioLayerOutwardEvent::JackConnect(port_b_name.clone(), port_a_name.clone())) {
            Ok(_) => {}
            Err(_) => {
                debug!("Audio: problem notifying of new jack connection from={} to={}", port_b_name, port_a_name);
            }
        }
    }
}

impl NotificationHandler for JackNotificationHandler {
    fn thread_init(&self, _: &Client) {
        debug!("JACK: async thread started");
    }

    fn shutdown(&mut self, status: ClientStatus, reason: &str) {
        debug!(
            "JACK: shutdown with status {:?} because \"{}\"",
            status, reason
        );
        if status == ClientStatus::CLIENT_ZOMBIE {
            match self.jack_midi_sender.try_send(AudioLayerOutwardEvent::JackRestartRequired) {
                Err(_) => {

                }
                _ => {}
            }
        }
    }

    fn freewheel(&mut self, _: &Client, is_enabled: bool) {
        debug!(
            "JACK: freewheel mode is {}",
            if is_enabled { "on" } else { "off" }
        );
    }

    fn sample_rate(&mut self, _: &Client, sample_rate: Frames) -> Control {
        debug!("JACK: sample rate changed to {}", sample_rate);
        Control::Continue
    }

    fn client_registration(&mut self, _: &Client, name: &str, is_reg: bool) {
        debug!(
            "JACK: {} client with name \"{}\"",
            if is_reg { "registered" } else { "unregistered" },
            name
        );
    }

    fn port_registration(&mut self, _: &Client, port_id: PortId, is_reg: bool) {
        debug!(
            "JACK: {} port with id {}",
            if is_reg { "registered" } else { "unregistered" },
            port_id
        );
    }

    fn port_rename(
        &mut self,
        _: &Client,
        port_id: PortId,
        old_name: &str,
        new_name: &str,
    ) -> Control {
        debug!(
            "JACK: port with id {} renamed from {} to {}",
            port_id, old_name, new_name
        );
        Control::Continue
    }

    fn ports_connected(
        &mut self,
        client: &Client,
        port_id_a: PortId,
        port_id_b: PortId,
        are_connected: bool,
    ) {
        debug!(
            "JACK: ports with id {} and {} are {}",
            port_id_a,
            port_id_b,
            if are_connected {
                "connected"
            } else {
                "disconnected"
            }
        );
        if are_connected {
            let port_a = client.port_by_id(port_id_a);
            let port_b = client.port_by_id(port_id_b);

            debug!("Jack client name: {}", client.name());

            if let Some(port) = port_a {
                if let Ok(port_a_name) = port.name() {
                    debug!("Jack port: name={}, flags={:?}", port_a_name, port.flags());
                    if let Some(port) = port_b {
                        if let Ok(port_b_name) = port.name() {
                            let input_output = format!("{:?}", port.flags());
                            debug!("Jack port: name={}, flags={}", port_b_name, input_output );
                            if input_output.as_str() == "IS_OUTPUT" {
                                self.notify_about_connected_ports(&port_b_name, &port_a_name);
                                debug!("Audio: jack connection from={} to={}", port_b_name, port_a_name);
                            }
                            else {
                                self.notify_about_connected_ports(&port_a_name, &port_b_name);
                                debug!("Audio: jack connection from={} to={}", port_a_name, port_b_name);
                            }
                        }
                        else {
                            debug!("Jack port has no name.");
                        }
                    }
                }
                else {
                    debug!("Jack port has no name.");
                }
            }
        }
        else {

        }
    }

    fn graph_reorder(&mut self, _: &Client) -> Control {
        debug!("JACK: graph reordered");
        Control::Continue
    }

    fn xrun(&mut self, _: &Client) -> Control {
        debug!("JACK: under run occurred");
        Control::Continue
    }
}

pub struct Audio {
    audio_buffer_right: [f32; 1024],
    audio_buffer_left: [f32; 1024],
    audio_blocks: Vec<AudioBlock>, // being careful not to do heap allocation in the jack callback method when reading from the track consumers
    audio_block_pool: Vec<AudioBlock>,
    block_number_buffer: BTreeMap<i32, BTreeMap<i32, AudioBlock>>,
    btree_map_pool: Vec<BTreeMap<i32, AudioBlock>>,
    stuck_block_number: i32,
    stuck_block_number_attempt_count: i32,
    jack_midi_buffer: [(u32, u8, u8, u8, bool); 1024],
    out_l: Port<AudioOut>,
    out_r: Port<AudioOut>,
    midi_out: Port<MidiOut>,
    midi_in: Port<MidiIn>,
    midi_control_in: Port<MidiIn>,
    audio_consumers: Vec<AudioConsumerDetails<AudioBlock>>,
    midi_consumers: Vec<MidiConsumerDetails<(u32, u8, u8, u8, bool)>>,
    play: bool,
    block: i32,
    blocks_total: i32,
    play_position_in_frames: u32,
    sample_rate_in_frames: f64,
    tempo: f64,
    block_size: f64,
    frames_per_beat: u32,
    low_priority_processing_delay_counter: i32,
    process_producers: bool,
    master_volume: f32,
    master_pan: f32,
    low_priority_processing_delay_count: i32,
    rx_to_audio: crossbeam_channel::Receiver<AudioLayerInwardEvent>,
    jack_midi_sender: crossbeam_channel::Sender<AudioLayerOutwardEvent>,
    jack_midi_sender_ui: crossbeam_channel::Sender<AudioLayerOutwardEvent>,
    jack_time_critical_midi_sender: crossbeam_channel::Sender<AudioLayerTimeCriticalOutwardEvent>,
    coast: Arc<Mutex<TrackBackgroundProcessorMode>>,
    custom_midi_out_ports: Vec<Port<MidiOut>>,
    keep_alive: bool,
    preview_sample: Option<SampleData>,
    preview_sample_current_frame: i32,
    vst_host_time_info: Arc<parking_lot::RwLock<TimeInfo>>,
}

impl Audio {
    pub fn new(client: &Client,
               rx_to_audio: crossbeam_channel::Receiver<AudioLayerInwardEvent>,
               jack_midi_sender: crossbeam_channel::Sender<AudioLayerOutwardEvent>,
               jack_midi_sender_ui: crossbeam_channel::Sender<AudioLayerOutwardEvent>,
               jack_time_critical_midi_sender: crossbeam_channel::Sender<AudioLayerTimeCriticalOutwardEvent>,
               coast: Arc<Mutex<TrackBackgroundProcessorMode>>,
               vst_host_time_info: Arc<parking_lot::RwLock<TimeInfo>>,
    ) -> Self {
        let audio_block_pool: Vec<AudioBlock> = (0..2500).map(|_| AudioBlock::default()).collect();
        let btree_map_pool: Vec<BTreeMap<i32, AudioBlock>> = (0..100).map(|_| BTreeMap::new()).collect();
        Audio {
            audio_buffer_right: [0.0f32; 1024],
            audio_buffer_left: [0.0f32; 1024],
            audio_blocks: vec![],
            audio_block_pool,
            block_number_buffer: BTreeMap::new(),
            btree_map_pool,
            stuck_block_number: 0,
            stuck_block_number_attempt_count: 0,
            jack_midi_buffer: [(0, 0, 0, 0, false); 1024],
            out_l: client.register_port("out_l", AudioOut::default()).unwrap(),
            out_r: client.register_port("out_r", AudioOut::default()).unwrap(),
            midi_out: client.register_port("midi_out", MidiOut::default()).unwrap(),
            midi_in: client.register_port("midi_in", MidiIn::default()).unwrap(),
            midi_control_in: client.register_port("midi_control_in", MidiIn::default()).unwrap(),
            audio_consumers: vec![],
            midi_consumers: vec![],
            play: false,
            block: -1,
            blocks_total: 0,
            play_position_in_frames: 0,
            sample_rate_in_frames: 44100.0,
            tempo: 140.0,
            block_size: 1024.0,
            frames_per_beat: Audio::frames_per_beat_calc(44100.0, 140.0),
            low_priority_processing_delay_counter: 0,
            process_producers: true,
            master_volume: 1.0,
            master_pan: 0.0,
            low_priority_processing_delay_count: 5,
            rx_to_audio,
            jack_midi_sender,
            jack_midi_sender_ui,
            jack_time_critical_midi_sender,
            coast,
            custom_midi_out_ports: vec![],
            keep_alive: true,
            preview_sample: None,
            preview_sample_current_frame: 0,
            vst_host_time_info,
        }
    }

    pub fn new_with_consumers(client: &Client,
                              rx_to_audio: crossbeam_channel::Receiver<AudioLayerInwardEvent>,
                              jack_midi_sender: crossbeam_channel::Sender<AudioLayerOutwardEvent>,
                              jack_midi_sender_ui: crossbeam_channel::Sender<AudioLayerOutwardEvent>,
                              jack_time_critical_midi_sender: crossbeam_channel::Sender<AudioLayerTimeCriticalOutwardEvent>,
                              coast: Arc<Mutex<TrackBackgroundProcessorMode>>,
                              audio_consumers: Vec<AudioConsumerDetails<AudioBlock>>,
                              midi_consumers: Vec<MidiConsumerDetails<(u32, u8, u8, u8, bool)>>,
                              vst_host_time_info: Arc<parking_lot::RwLock<TimeInfo>>,
    ) -> Self {
        let audio_block_pool: Vec<AudioBlock> = (0..2500).map(|_| AudioBlock::default()).collect();
        let btree_map_pool: Vec<BTreeMap<i32, AudioBlock>> = (0..100).map(|_| BTreeMap::new()).collect();
        Audio {
            audio_buffer_right: [0.0f32; 1024],
            audio_buffer_left: [0.0f32; 1024],
            audio_blocks: vec![],
            audio_block_pool,
            block_number_buffer: BTreeMap::new(),
            btree_map_pool,
            stuck_block_number: 0,
            stuck_block_number_attempt_count: 0,
            jack_midi_buffer: [(0, 0, 0, 0, false); 1024],
            out_l: client.register_port("out_l", AudioOut::default()).unwrap(),
            out_r: client.register_port("out_r", AudioOut::default()).unwrap(),
            midi_out: client.register_port("midi_out", MidiOut::default()).unwrap(),
            midi_in: client.register_port("midi_in", MidiIn::default()).unwrap(),
            midi_control_in: client.register_port("midi_control_in", MidiIn::default()).unwrap(),
            audio_consumers,
            midi_consumers,
            play: false,
            block: -1,
            blocks_total: 0,
            play_position_in_frames: 0,
            sample_rate_in_frames: 44100.0,
            tempo: 140.0,
            block_size: 1024.0,
            frames_per_beat: Audio::frames_per_beat_calc(44100.0, 140.0),
            low_priority_processing_delay_counter: 0,
            process_producers: true,
            master_volume: 1.0,
            master_pan: 0.5,
            low_priority_processing_delay_count: 5,
            rx_to_audio,
            jack_midi_sender,
            jack_midi_sender_ui,
            jack_time_critical_midi_sender,
            coast,
            custom_midi_out_ports: vec![],
            keep_alive: true,
            preview_sample: None,
            preview_sample_current_frame: 0,
            vst_host_time_info,
        }
    }

    pub fn frames_per_beat_calc(sample_rate_in_frames: f64, tempo: f64) -> u32 {
        (sample_rate_in_frames / tempo * 60.0) as u32
    }

    fn handle_inward_events(&mut self, _client: &Client) {
        match self.rx_to_audio.try_recv() {
            Ok(event) => match event {
                AudioLayerInwardEvent::NewAudioConsumer(audio_consumer_detail) => {
                    self.audio_consumers.push(audio_consumer_detail);
                    // debug!("*************Added an audio consumer: {}", self.audio_consumers.len());
                }
                AudioLayerInwardEvent::NewMidiConsumer(midi_consumer_detail) => {
                    self.midi_consumers.push(midi_consumer_detail);
                    debug!("*************Added a midi consumer: {}", self.midi_consumers.len());
                }
                AudioLayerInwardEvent::Play(start_play, number_of_blocks, start_block) => {
                    // debug!(root_logger, "*************Jack start play received: number_of_blocks={}", number_of_blocks);
                    self.play = start_play;
                    self.block = start_block;
                    self.play_position_in_frames = self.block as u32 * self.block_size as u32;
                    self.blocks_total = number_of_blocks;
                }
                AudioLayerInwardEvent::Stop => {
                    // debug!(root_logger, "*************Jack stop play received.");
                    self.play = false;
                    if self.block > -1 {
                        self.block = 0;
                    }
                    self.play_position_in_frames = 0;
                    self.block_number_buffer.clear();
                }
                AudioLayerInwardEvent::ExtentsChange(number_of_blocks) => {
                    // debug!(root_logger, "*************Jack extents change received: number_of_blocks={}", number_of_blocks);
                    self.blocks_total = number_of_blocks;
                }
                AudioLayerInwardEvent::Tempo(new_tempo) => {
                    // debug!(root_logger, "*************Jack tempo received: tempo={}", tempo);
                    self.tempo = new_tempo;
                    self.frames_per_beat = Audio::frames_per_beat_calc(self.sample_rate_in_frames, self.tempo);
                }
                AudioLayerInwardEvent::SampleRate(new_sample_rate) => {
                    self.sample_rate_in_frames = new_sample_rate;
                    self.frames_per_beat = Audio::frames_per_beat_calc(self.sample_rate_in_frames, self.tempo);
                }
                AudioLayerInwardEvent::BlockSize(new_block_size) => {
                    self.block_size = new_block_size;
                    self.frames_per_beat = Audio::frames_per_beat_calc(self.sample_rate_in_frames, self.tempo);
                }
                AudioLayerInwardEvent::Volume(volume) => {
                    self.master_volume = volume;
                }
                AudioLayerInwardEvent::Pan(pan) => {
                    self.master_pan = pan;
                }
                AudioLayerInwardEvent::Shutdown => {
                    self.keep_alive = false;
                }
                AudioLayerInwardEvent::RemoveTrack(track_uuid) => {
                    self.audio_consumers.retain(|consumer_detail| *consumer_detail.track_id() != track_uuid);
                    self.midi_consumers.retain(|consumer_detail| *consumer_detail.track_uuid() != track_uuid);
                }
                AudioLayerInwardEvent::NewMidiOutPortForTrack(track_uuid, midi_out_port) => {
                    debug!("Jack layer received: AudioLayerInwardEvent::NewMidiOutPortForTrack");
                    if let Some(midi_consumer_details) = self.midi_consumers.iter_mut().find(|midi_consumer_details| midi_consumer_details.track_uuid().clone() == track_uuid.clone()) {
                        midi_consumer_details.set_midi_out_port(Some(midi_out_port));
                    }
                    else {
                        // show an error message dialogue??
                        debug!("Failed to add a midi out port for track={}", track_uuid.as_str());
                    }
                }
                AudioLayerInwardEvent::PreviewSample(file_name) => {
                    debug!("Audio layer received preview sample: file name = {}", file_name);
                    self.preview_sample = Some(SampleData::new(file_name, self.sample_rate_in_frames as i32));
                    self.preview_sample_current_frame = 0;
                }
            },
            Err(_) => (),
        }
    }

    fn zero_jack_buffers(&mut self, process_scope: &ProcessScope) {
        let out_left = self.out_l.as_mut_slice(process_scope);
        let out_right = self.out_r.as_mut_slice(process_scope);
        // zero the jack buffers
        for index in 0..out_right.len() {
            match out_left.get_mut(index) {
                Some(left) => *left = 0.0,
                None => (),
            }
            match out_right.get_mut(index) {
                Some(right) => *right = 0.0,
                None => (),
            }
        }
    }

    fn process_audio(&mut self, process_scope: &ProcessScope) {
        let frames_written = self.block_size() as usize;
        let mut number_of_consumers = self.audio_consumers.len() as f32;
        let (left_pan, right_pan) = DAWUtils::constant_power_stereo_pan(self.master_pan);
        let mut master_channel_left_level: f32 = 0.0;
        let mut master_channel_right_level: f32 = 0.0;

        if self.preview_sample().is_some() {
            number_of_consumers += 1.0;
        }

        {
            // the following breaks down when a consumer's producer takes too long to send audio blocks
            // the block_number_buffer keeps filling up until all pool resources are consumed and there is no audio

            let out_left = self.out_l.as_mut_slice(process_scope);
            let out_right = self.out_r.as_mut_slice(process_scope);

            // if block number is 0 then dump all previous data and return audio blocks and btree maps to their respective pools - works when loop in the riff views
            if self.block == 0 {
                let block_numbers: Vec<i32> = self.block_number_buffer.keys().map(|key| *key).collect();
                for block_number in block_numbers.iter() {
                    if let Some(mut tracks) = self.block_number_buffer.remove(&block_number) {
                        let track_numbers: Vec<i32> = tracks.keys().map(|key| *key).collect();
                        for track_number in track_numbers.iter() {
                            if let Some(audio_block) = tracks.remove(&track_number) {
                                self.audio_block_pool.push(audio_block);
                            }
                        }
                        self.btree_map_pool.push(tracks);
                    }
                }
            }

            // read the consumers and store the audio blocks appropriately
            // TODO this would work better for track deletes if the track uuid was used as the key
            for (track_key, audio_consumer_details) in self.audio_consumers.iter_mut().enumerate() {
                let consumer = audio_consumer_details.consumer();
                if let Some(mut new_audio_block) = self.audio_block_pool.pop() {
                    self.audio_blocks.push(new_audio_block);

                    match consumer.read(&mut self.audio_blocks) {
                        Ok(read) => {
                            if read == 1 {
                                if let Some(mut audio_block) = self.audio_blocks.pop() {
                                    if self.block_number_buffer.contains_key(&audio_block.block) {
                                        if let Some(tracks) = self.block_number_buffer.get_mut(&audio_block.block) {
                                            tracks.insert(track_key as i32, audio_block);
                                        }
                                    } else if let Some(mut tracks) = self.btree_map_pool.pop() {
                                        let block_number = audio_block.block;
                                        tracks.insert(track_key as i32, audio_block);
                                        self.block_number_buffer.insert(block_number, tracks);
                                    }
                                    else {
                                        self.audio_block_pool.push(audio_block);
                                    }
                                }
                            }
                        }
                        Err(_) => {
                            if let Some(audio_block) = self.audio_blocks.pop() {
                                self.audio_block_pool.push(audio_block);
                            }
                        }
                    }
                }
            }

            let mut key_to_remove = None;
            if let Some((key, track_buffer)) = self.block_number_buffer.first_key_value() {
                if self.stuck_block_number != *key {
                    self.stuck_block_number = *key;
                    self.stuck_block_number_attempt_count = 0;
                }

                if track_buffer.len() == self.audio_consumers.len() {
                    for audio_block in track_buffer.values() {
                        for (index, (left, right)) in out_left.iter_mut().zip(out_right.iter_mut()).enumerate() {
                            if index < audio_block.audio_data_left.len() && index < audio_block.audio_data_right.len() {
                                *left += audio_block.audio_data_left[index] * self.master_volume * 2.0 * left_pan;
                                *right += audio_block.audio_data_right[index] * self.master_volume * 2.0 * right_pan;
                                if *left > master_channel_left_level {
                                    master_channel_left_level += *left;
                                }
                                if *right > master_channel_right_level {
                                    master_channel_right_level += *right;
                                }
                            } else {
                                break;
                            }
                        }
                    }
                    key_to_remove = Some(*key);
                }
                else {
                    self.stuck_block_number_attempt_count += 1;

                    if self.stuck_block_number_attempt_count > 2 {
                        key_to_remove = Some(*key);
                    }
                }
            }

            if let Some(key) = key_to_remove {
                if let Some(mut tracks) = self.block_number_buffer.remove(&key) {
                    let track_numbers: Vec<i32> = tracks.keys().map(|key| *key).collect();
                    for track_number in track_numbers.iter() {
                        if let Some(audio_block) = tracks.remove(&track_number) {
                            self.audio_block_pool.push(audio_block);
                        }
                    }
                    self.btree_map_pool.push(tracks);
                }
            }
            // debug!("btree_map_pool size: {}, block_number_buffer size: {}, audio_block_pool size: {}", self.btree_map_pool.len(), self.block_number_buffer.len(), self.audio_block_pool.len());
        }

        let _ = self.jack_midi_sender_ui.try_send(AudioLayerOutwardEvent::MasterChannelLevels(master_channel_left_level, master_channel_right_level));

        self.process_preview_sample(process_scope, frames_written, &mut number_of_consumers, left_pan, right_pan)
    }

    fn process_preview_sample(&mut self, process_scope: &ProcessScope, frames_written: usize, number_of_consumers: &mut f32, left_pan: f32, right_pan: f32) {
        let preview_sample_current_frame = self.preview_sample_current_frame() as usize;
        let sample_channels = if let Some(sample) = self.preview_sample_mut() {
            sample.channels() as usize
        } else {
            0
        };
        let sample_length = if let Some(sample) = self.preview_sample_mut() {
            sample.samples().len()
        } else {
            0
        };
        let mut left_samples: Vec<f32> = vec![];
        let mut right_samples: Vec<f32> = vec![];
        if let Some(sample) = self.preview_sample_mut() {
            for frame in 0..frames_written {
                let left_channel_sample_index = (frame + preview_sample_current_frame) * sample_channels;
                let right_channel_sample_index = left_channel_sample_index + 1;

                if left_channel_sample_index < sample_length {
                    if sample.channels() == 1 {
                        left_samples.push(*sample.samples().get(left_channel_sample_index).unwrap());
                        right_samples.push(*sample.samples().get(left_channel_sample_index).unwrap());
                    } else {
                        left_samples.push(*sample.samples().get(left_channel_sample_index).unwrap());
                        right_samples.push(*sample.samples().get(right_channel_sample_index).unwrap());
                    }
                }
            }
        }

        self.set_preview_sample_current_frame(self.preview_sample_current_frame() + frames_written as i32);

        let mut frame: usize = 0;
        let out_left = self.out_l.as_mut_slice(process_scope);
        let out_right = self.out_r.as_mut_slice(process_scope);
        for left_sample in left_samples.iter() {
            out_left[frame] += (*left_sample / *number_of_consumers) * self.master_volume * 2.0 * left_pan;
            out_right[frame] += (*right_samples.get(frame).unwrap() / *number_of_consumers) * self.master_volume * 2.0 * right_pan;
            frame += 1;
        }
    }

    fn process_midi_out(&mut self, process_scope: &ProcessScope) {
        for midi_consumer_detail in self.midi_consumers.iter_mut() {
            let consumer_midi = midi_consumer_detail.consumer_mut();
            match consumer_midi.read(&mut self.jack_midi_buffer) {
                Ok(read) => if read > 0 {
                    // debug!("Jack audio received some midi events: {}", read);
                    if let Some(midi_output_port) = midi_consumer_detail.midi_out_port_mut() {
                        let mut midi_out_writer = midi_output_port.writer(process_scope.clone());
                        for count in 0..read {
                            let (frames, byte1, byte2, byte3, active) = self.jack_midi_buffer[count];
                            if active {
                                // debug!("Jack audio sending a midi event: {}, {}, {}, {}", frames, byte1, byte2, byte3);
                                let bytes = [byte1, byte2, byte3];
                                let event = RawMidi { time: frames, bytes:  &bytes};
                                let _ = midi_out_writer.write(&event);
                            }
                            else {
                                break;
                            }
                        }
                    }
                },
                Err(_) => (),
            }
        }
    }

    fn process_midi_in(&mut self, process_scope: &ProcessScope) {
        let midi_in_data = self.midi_in.iter(process_scope);
        for event in midi_in_data {
            let mut delta_frames = 0;

            if self.play && self.block > -1 {
                delta_frames = self.block * 1024 /* + event.time as i32 */;
            }

            if event.bytes.len() >= 3 && 144 <= event.bytes[0] && event.bytes[0] <= 159 { // note on
                let note_on = MidiEvent {
                    data: [144, event.bytes[1], event.bytes[2]],
                    delta_frames,
                    live: true,
                    note_length: None,
                    note_offset: None,
                    detune: 0,
                    note_off_velocity: 0,
                };

                let _ = self.jack_time_critical_midi_sender.try_send(AudioLayerTimeCriticalOutwardEvent::MidiEvent(note_on));
            }
            else if event.bytes.len() >= 3 && 128 <= event.bytes[0] && event.bytes[0] <= 143 { //note off
                let note_off = MidiEvent {
                    data: [128, event.bytes[1], event.bytes[2]],
                    delta_frames,
                    live: true,
                    note_length: None,
                    note_offset: None,
                    detune: 0,
                    note_off_velocity: 0,
                };

                let _ = self.jack_time_critical_midi_sender.try_send(AudioLayerTimeCriticalOutwardEvent::MidiEvent(note_off));
            }
            else if event.bytes.len() >= 3 && 176 <= event.bytes[0] && event.bytes[0]  <= 191  { // controllers
                let controller = Self::create_midi_event(event, &mut delta_frames);

                let _ = self.jack_time_critical_midi_sender.try_send(AudioLayerTimeCriticalOutwardEvent::MidiEvent(controller));
            }
            else if event.bytes.len() >= 3 && 224 <= event.bytes[0] && event.bytes[0]  <= 239  { // pitch bend
                let pitch_bend = Self::create_midi_event(event, &mut delta_frames);

                let _ = self.jack_time_critical_midi_sender.try_send(AudioLayerTimeCriticalOutwardEvent::MidiEvent(pitch_bend));
            }
            else if event.bytes.len() >= 3 && 160 <= event.bytes[0] && event.bytes[0]  <= 175  { // polyphonic key pressure
                let polyphonic_key_pressure = Self::create_midi_event(event, &mut delta_frames);

                let _ = self.jack_time_critical_midi_sender.try_send(AudioLayerTimeCriticalOutwardEvent::MidiEvent(polyphonic_key_pressure));
            }
            else if event.bytes.len() >= 3 && 208 <= event.bytes[0] && event.bytes[0]  <= 223  { // channel pressure
                let channel_pressure = Self::create_midi_event(event, &mut delta_frames);

                let _ = self.jack_time_critical_midi_sender.try_send(AudioLayerTimeCriticalOutwardEvent::MidiEvent(channel_pressure));
            }
        }
    }

    fn create_midi_event(event: RawMidi, delta_frames: &mut i32) -> MidiEvent {
        MidiEvent {
            data: [event.bytes[0], event.bytes[1], event.bytes[2]],
            delta_frames: *delta_frames,
            live: true,
            note_length: None,
            note_offset: None,
            detune: 0,
            note_off_velocity: 0,
        }
    }

    fn process_midi_control_in(&mut self, process_scope: &ProcessScope) {
        let midi_control_in_data = self.midi_control_in.iter(process_scope);
        for event in midi_control_in_data {
            let mut delta_frames = 0;

            if self.play && self.block > -1 {
                delta_frames = self.block * 1024 + event.time as i32;
            }

            if event.bytes.len() >= 3 && 144 <= event.bytes[0] && event.bytes[0] <= 159 { // note on
                let note_on = MidiEvent {
                    data: [144, event.bytes[1], event.bytes[2]],
                    delta_frames,
                    live: true,
                    note_length: None,
                    note_offset: None,
                    detune: 0,
                    note_off_velocity: 0,
                };


                let _ = self.jack_midi_sender_ui.try_send(AudioLayerOutwardEvent::MidiControlEvent(note_on));
            }
            else if event.bytes.len() >= 3 && 128 <= event.bytes[0] && event.bytes[0] <= 143 { // note off
                let note_off = MidiEvent {
                    data: [128, event.bytes[1], event.bytes[2]],
                    delta_frames,
                    live: true,
                    note_length: None,
                    note_offset: None,
                    detune: 0,
                    note_off_velocity: 0,
                };

                let _ = self.jack_midi_sender_ui.try_send(AudioLayerOutwardEvent::MidiControlEvent(note_off));
            }
            else if event.bytes.len() >= 3 && 176 <= event.bytes[0] && event.bytes[0]  <= 191  { // controllers
                let controller = Self::create_midi_event(event, &mut delta_frames);
                if controller.data[0] as i32 >= 176 && (controller.data[0] as i32 <= (176 + 15)) {
                    let _ = self.jack_time_critical_midi_sender.try_send(AudioLayerTimeCriticalOutwardEvent::TrackVolumePanLevel(controller));
                }
                else {
                    let _ = self.jack_midi_sender.try_send(AudioLayerOutwardEvent::MidiControlEvent(controller));
                }
            }
            else if event.bytes.len() >= 3 && event.bytes.len() == 6 && event.bytes[0] == 240 && event.bytes[1] == 127 && event.bytes[3] == 6 {
                let mmc_sysex_bytes: [u8; 6] = [240, 127, event.bytes[2], 6, event.bytes[4], 247];
                match self.jack_midi_sender.send(AudioLayerOutwardEvent::GeneralMMCEvent(mmc_sysex_bytes)) {
                    Ok(_) => {}
                    Err(err) => {
                        debug!("jack problem sending General MMC event: {:?}", err);
                    }
                }
            }
            else if event.bytes.len() == 3 {
                debug!("jack - received a unknown message: {} {} {}", event.bytes[0], event.bytes[1], event.bytes[2]);
            }
        }
    }

    fn update_position_and_notify(&mut self) {
        if self.play && self.block > -1 {
            if self.block >= self.blocks_total {
                self.block = 0;
                self.play_position_in_frames = 0;
            } else {
                self.block += 1;
                self.play_position_in_frames += 1024;
            }

            TRANSPORT.get().write().position_in_frames = self.play_position_in_frames;

            {
                let ppq_pos = (self.play_position_in_frames as f64 * self.tempo) / (60.0 * self.sample_rate_in_frames);
                let mut flags = TimeInfoFlags::TRANSPORT_CHANGED.bits();
                flags |= TimeInfoFlags::TRANSPORT_PLAYING.bits(); // transport playing
                flags |= TimeInfoFlags::TEMPO_VALID.bits(); // tempo valid
                flags |= TimeInfoFlags::TIME_SIG_VALID.bits(); // time signature valid
                flags |= TimeInfoFlags::PPQ_POS_VALID.bits(); // ppq position valid
                
                let mut time_info =  self.vst_host_time_info.write();
                time_info.flags = flags;
                time_info.sample_pos = self.play_position_in_frames as f64;
                time_info.ppq_pos = ppq_pos;
            }

            if self.play_position_in_frames % self.frames_per_beat < 1024 {
                let _ = self.jack_midi_sender_ui.try_send(AudioLayerOutwardEvent::PlayPositionInFrames(self.play_position_in_frames));
            }
        }
        else if !self.play { // not playing anymore
            // If we can't get the lock this time maybe we can get it next time
            let mut time_info =  self.vst_host_time_info.write();
            if time_info.flags & (1 << 1) == 2 { // transport playing need to stop
                let mut flags = TimeInfoFlags::TRANSPORT_CHANGED.bits(); // transport changed

                flags |= TimeInfoFlags::TEMPO_VALID.bits(); // tempo valid
                flags |= TimeInfoFlags::TIME_SIG_VALID.bits(); // time signature valid
                flags |= TimeInfoFlags::PPQ_POS_VALID.bits(); // ppq position valid
                time_info.flags = flags;
            }
            else if time_info.flags & 1 == 1 { // transport changed but not playing
                let mut flags = 0; // transport not changed

                flags |= TimeInfoFlags::TEMPO_VALID.bits(); // tempo valid
                flags |= TimeInfoFlags::TIME_SIG_VALID.bits(); // time signature valid
                flags |= TimeInfoFlags::PPQ_POS_VALID.bits(); // ppq position valid
                time_info.flags = flags;
            }
        }
    }

    fn check_for_coast(&mut self) {
        self.process_producers = match self.coast.try_lock() {
            Ok(mode) => match *mode {
                TrackBackgroundProcessorMode::AudioOut => true,
                TrackBackgroundProcessorMode::Coast => false,
                TrackBackgroundProcessorMode::Render => false,
            }
            Err(_) => false
        };
    }

    fn update_low_priority_processing_delay_counter(&mut self) {
        if self.low_priority_processing_delay_counter > self.low_priority_processing_delay_count {
            self.low_priority_processing_delay_counter = 0;
        } else {
            self.low_priority_processing_delay_counter += 1;
        }
    }

    pub fn custom_midi_out_ports(&mut self) -> &mut Vec<Port<MidiOut>> {
        &mut self.custom_midi_out_ports
    }

    pub fn get_all_audio_consumers(&mut self) -> Vec<AudioConsumerDetails<AudioBlock>> {
        let mut consumers = vec![];
        for _ in 0..self.audio_consumers.len() {
            consumers.push(self.audio_consumers.remove(0));
        }
        consumers
    }

    pub fn preview_sample(&self) -> &Option<SampleData> {
        &self.preview_sample
    }

    pub fn preview_sample_mut(&mut self) -> &Option<SampleData> {
        &self.preview_sample
    }

    pub fn preview_sample_current_frame(&self) -> i32 {
        self.preview_sample_current_frame
    }

    pub fn preview_sample_current_frame_mut(&mut self) -> i32 {
        self.preview_sample_current_frame
    }

    pub fn set_preview_sample(&mut self, preview_sample: Option<SampleData>) {
        self.preview_sample = preview_sample;
    }

    pub fn set_preview_sample_current_frame(&mut self, preview_sample_current_frame: i32) {
        self.preview_sample_current_frame = preview_sample_current_frame;
    }

    pub fn block_size(&self) -> f64 {
        self.block_size
    }
}

impl ProcessHandler for Audio {
    fn process(&mut self, client: &Client, process_scope: &ProcessScope) -> Control {
        if self.low_priority_processing_delay_counter >= self.low_priority_processing_delay_count {
            self.handle_inward_events(client.clone());
        }

        self.zero_jack_buffers(process_scope);

        if self.process_producers {
            self.process_audio(process_scope);
            self.process_midi_out(process_scope);
            self.process_midi_in(process_scope);
            self.process_midi_control_in(process_scope);
            self.update_position_and_notify()
        }

        self.check_for_coast();
        self.update_low_priority_processing_delay_counter();

        if self.keep_alive {
            Control::Continue
        }
        else {
            Control::Quit
        }
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn list_audio_devices() {
    }
}
