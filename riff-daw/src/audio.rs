use std::convert::From;
use std::sync::{Arc, Mutex};

use jack::{AudioOut, Client, ClientStatus, Control, Frames, MidiIn, MidiOut, NotificationHandler, Port, PortId, ProcessHandler, ProcessScope, RawMidi};
use rb::RbConsumer;
use vst::api::{TimeInfo, TimeInfoFlags};
use vst::event::MidiEvent;

use crate::{AudioConsumerDetails, AudioLayerInwardEvent, AudioLayerOutwardEvent, DAWUtils, MidiConsumerDetails, SampleData, TrackBackgroundProcessorMode};

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
                println!("Audio: problem notifying of new jack connection from={} to={}", port_b_name, port_a_name);
            }
        }
    }
}

impl NotificationHandler for JackNotificationHandler {
    fn thread_init(&self, _: &Client) {
        println!("JACK: async thread started");
    }

    fn shutdown(&mut self, status: ClientStatus, reason: &str) {
        println!(
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
        println!(
            "JACK: freewheel mode is {}",
            if is_enabled { "on" } else { "off" }
        );
    }

    fn sample_rate(&mut self, _: &Client, sample_rate: Frames) -> Control {
        println!("JACK: sample rate changed to {}", sample_rate);
        Control::Continue
    }

    fn client_registration(&mut self, _: &Client, name: &str, is_reg: bool) {
        println!(
            "JACK: {} client with name \"{}\"",
            if is_reg { "registered" } else { "unregistered" },
            name
        );
    }

    fn port_registration(&mut self, _: &Client, port_id: PortId, is_reg: bool) {
        println!(
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
        println!(
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
        println!(
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

            println!("Jack client name: {}", client.name());

            if let Some(port) = port_a {
                if let Ok(port_a_name) = port.name() {
                    println!("Jack port: name={}, flags={:?}", port_a_name, port.flags());
                    if let Some(port) = port_b {
                        if let Ok(port_b_name) = port.name() {
                            let input_output = format!("{:?}", port.flags());
                            println!("Jack port: name={}, flags={}", port_b_name, input_output );
                            if input_output.as_str() == "IS_OUTPUT" {
                                self.notify_about_connected_ports(&port_b_name, &port_a_name);
                                println!("Audio: jack connection from={} to={}", port_b_name, port_a_name);
                            }
                            else {
                                self.notify_about_connected_ports(&port_a_name, &port_b_name);
                                println!("Audio: jack connection from={} to={}", port_a_name, port_b_name);
                            }
                        }
                        else {
                            println!("Jack port has no name.");
                        }
                    }
                }
                else {
                    println!("Jack port has no name.");
                }
            }
        }
        else {

        }
    }

    fn graph_reorder(&mut self, _: &Client) -> Control {
        println!("JACK: graph reordered");
        Control::Continue
    }

    fn xrun(&mut self, _: &Client) -> Control {
        println!("JACK: under run occurred");
        Control::Continue
    }
}

pub struct Audio {
    audio_buffer_right: [f32; 1024],
    audio_buffer_left: [f32; 1024],
    jack_midi_buffer: [(u32, u8, u8, u8, bool); 1024],
    out_l: Port<AudioOut>,
    out_r: Port<AudioOut>,
    midi_out: Port<MidiOut>,
    midi_in: Port<MidiIn>,
    midi_control_in: Port<MidiIn>,
    audio_consumers: Vec<AudioConsumerDetails<f32>>,
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
               coast: Arc<Mutex<TrackBackgroundProcessorMode>>,
               vst_host_time_info: Arc<parking_lot::RwLock<TimeInfo>>,
    ) -> Self {
        Audio {
            audio_buffer_right: [0.0f32; 1024],
            audio_buffer_left: [0.0f32; 1024],
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
                              coast: Arc<Mutex<TrackBackgroundProcessorMode>>,
                              audio_consumers: Vec<AudioConsumerDetails<f32>>,
                              midi_consumers: Vec<MidiConsumerDetails<(u32, u8, u8, u8, bool)>>,
                              vst_host_time_info: Arc<parking_lot::RwLock<TimeInfo>>,
    ) -> Self {
        Audio {
            audio_buffer_right: [0.0f32; 1024],
            audio_buffer_left: [0.0f32; 1024],
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
                    // println!("*************Added an audio consumer: {}", self.audio_consumers.len());
                }
                AudioLayerInwardEvent::NewMidiConsumer(midi_consumer_detail) => {
                    self.midi_consumers.push(midi_consumer_detail);
                    println!("*************Added a midi consumer: {}", self.midi_consumers.len());
                }
                AudioLayerInwardEvent::Play(start_play, number_of_blocks, start_block) => {
                    // info!(root_logger, "*************Jack start play received: number_of_blocks={}", number_of_blocks);
                    self.play = start_play;
                    self.block = start_block;
                    self.play_position_in_frames = self.block as u32 * self.block_size as u32;
                    self.blocks_total = number_of_blocks;
                }
                AudioLayerInwardEvent::Stop => {
                    // info!(root_logger, "*************Jack stop play received.");
                    self.play = false;
                    if self.block > -1 {
                        self.block = 0;
                    }
                    self.play_position_in_frames = 0;
                }
                AudioLayerInwardEvent::ExtentsChange(number_of_blocks) => {
                    // info!(root_logger, "*************Jack extents change received: number_of_blocks={}", number_of_blocks);
                    self.blocks_total = number_of_blocks;
                }
                AudioLayerInwardEvent::Tempo(new_tempo) => {
                    // info!(root_logger, "*************Jack tempo received: tempo={}", tempo);
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
                    println!("Jack layer received: AudioLayerInwardEvent::NewMidiOutPortForTrack");
                    if let Some(midi_consumer_details) = self.midi_consumers.iter_mut().find(|midi_consumer_details| midi_consumer_details.track_uuid().clone() == track_uuid.clone()) {
                        midi_consumer_details.set_midi_out_port(Some(midi_out_port));
                    }
                    else {
                        // show an error message dialogue??
                        println!("Failed to add a midi out port for track={}", track_uuid.as_str());
                    }
                }
                AudioLayerInwardEvent::PreviewSample(file_name) => {
                    println!("Audio layer received preview sample: file name = {}", file_name);
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
            let out_left = self.out_l.as_mut_slice(process_scope);
            let out_right = self.out_r.as_mut_slice(process_scope);

            for index in 0..self.audio_consumers.len() {
                match self.audio_consumers.get_mut(index) {
                    Some(consumer) => {
                        let consumer_right = consumer.consumer_right_mut();
                        match consumer_right.read(&mut self.audio_buffer_right) {
                            Ok(read) => if read > 0 {
                                let mut count = 0;
                                for x in out_right.iter_mut() {
                                    if count < read {
                                        // info!(root_logger, "consumer_right index {}: {}", index, buffer_right[count]);
                                        *x += (self.audio_buffer_right[count]) as f32 * self.master_volume * 2.0 * right_pan;
                                        // print!("{} ", buffer_right[count]);
                                        if *x > master_channel_right_level {
                                            master_channel_right_level += *x;
                                        }
                                        count += 1;
                                    } else {
                                        break;
                                    }
                                }
                            },
                            Err(_) => (), //info!(root_logger, "Problem reading from consumer right channel!"),
                        }
                        let consumer_left = consumer.consumer_left_mut();
                        match consumer_left.read(&mut self.audio_buffer_left) {
                            Ok(read) => if read > 0 {
                                let mut count = 0;
                                for x in out_left.iter_mut() {
                                    if count < read {
                                        *x += (self.audio_buffer_left[count]) as f32 * self.master_volume * 2.0 * left_pan;
                                        // print!("{} ", buffer_left[count]);
                                        if *x > master_channel_left_level {
                                            master_channel_left_level += *x;
                                        }
                                        count += 1;
                                    } else {
                                        break;
                                    }
                                }
                            },
                            Err(_) => (), //info!(root_logger, "Problem reading from consumer left channel!"),
                        }
                    }
                    None => (), //info!(root_logger, "Problem getting consumer detail from consumers"),
                }
            }
        }

        let _ = self.jack_midi_sender.try_send(AudioLayerOutwardEvent::MasterChannelLevels(master_channel_left_level, master_channel_right_level));

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
            out_left[frame] += (*left_sample / *number_of_consumers) as f32 * self.master_volume * 2.0 * left_pan;
            out_right[frame] += (*right_samples.get(frame).unwrap() / *number_of_consumers) as f32 * self.master_volume * 2.0 * right_pan;
            frame += 1;
        }
    }

    fn process_midi_out(&mut self, process_scope: &ProcessScope) {
        for midi_consumer_detail in self.midi_consumers.iter_mut() {
            let consumer_midi = midi_consumer_detail.consumer_mut();
            match consumer_midi.read(&mut self.jack_midi_buffer) {
                Ok(read) => if read > 0 {
                    // println!("Jack audio received some midi events: {}", read);
                    if let Some(midi_output_port) = midi_consumer_detail.midi_out_port_mut() {
                        let mut midi_out_writer = midi_output_port.writer(process_scope.clone());
                        for count in 0..read {
                            let (frames, byte1, byte2, byte3, active) = self.jack_midi_buffer[count];
                            if active {
                                // println!("Jack audio sending a midi event: {}, {}, {}, {}", frames, byte1, byte2, byte3);
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

                let _ = self.jack_midi_sender.try_send(AudioLayerOutwardEvent::MidiEvent(note_on));
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

                let _ = self.jack_midi_sender.try_send(AudioLayerOutwardEvent::MidiEvent(note_off));
            }
            else if event.bytes.len() >= 3 && 176 <= event.bytes[0] && event.bytes[0]  <= 191  { // controllers
                let controller = Self::create_midi_event(event, &mut delta_frames);

                let _ = self.jack_midi_sender.try_send(AudioLayerOutwardEvent::MidiEvent(controller));
            }
            else if event.bytes.len() >= 3 && 224 <= event.bytes[0] && event.bytes[0]  <= 239  { // pitch bend
                let pitch_bend = Self::create_midi_event(event, &mut delta_frames);

                let _ = self.jack_midi_sender.try_send(AudioLayerOutwardEvent::MidiEvent(pitch_bend));
            }
            else if event.bytes.len() >= 3 && 160 <= event.bytes[0] && event.bytes[0]  <= 175  { // polyphonic key pressure
                let polyphonic_key_pressure = Self::create_midi_event(event, &mut delta_frames);

                let _ = self.jack_midi_sender.try_send(AudioLayerOutwardEvent::MidiEvent(polyphonic_key_pressure));
            }
            else if event.bytes.len() >= 3 && 208 <= event.bytes[0] && event.bytes[0]  <= 223  { // channel pressure
                let channel_pressure = Self::create_midi_event(event, &mut delta_frames);

                let _ = self.jack_midi_sender.try_send(AudioLayerOutwardEvent::MidiEvent(channel_pressure));
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


                let _ = self.jack_midi_sender.try_send(AudioLayerOutwardEvent::MidiControlEvent(note_on));
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

                let _ = self.jack_midi_sender.try_send(AudioLayerOutwardEvent::MidiControlEvent(note_off));
            }
            else if event.bytes.len() >= 3 && 176 <= event.bytes[0] && event.bytes[0]  <= 191  { // controllers
                let controller = Self::create_midi_event(event, &mut delta_frames);

                let _ = self.jack_midi_sender.try_send(AudioLayerOutwardEvent::MidiControlEvent(controller));
            }
            else if event.bytes.len() >= 3 && event.bytes.len() == 6 && event.bytes[0] == 240 && event.bytes[1] == 127 && event.bytes[3] == 6 {
                let mmc_sysex_bytes: [u8; 6] = [240, 127, event.bytes[2], 6, event.bytes[4], 247];
                let _ = self.jack_midi_sender.try_send(AudioLayerOutwardEvent::GeneralMMCEvent(mmc_sysex_bytes));
            }
            else if event.bytes.len() == 3 {
                println!("jack - received a unknown message: {} {} {}", event.bytes[0], event.bytes[1], event.bytes[2]);
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
                let _ = self.jack_midi_sender.try_send(AudioLayerOutwardEvent::PlayPositionInFrames(self.play_position_in_frames));
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

    pub fn get_all_audio_consumers(&mut self) -> Vec<AudioConsumerDetails<f32>> {
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
