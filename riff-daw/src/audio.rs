use std::convert::From;
use std::sync::{Arc, Mutex};
use jack::{AudioOut, Client, ClientStatus, Control, Frames, MidiIn, MidiOut, NotificationHandler, Port, PortId, ProcessHandler, ProcessScope, RawMidi};
use rb::RbConsumer;
use vst::api::{TimeInfo, TimeInfoFlags};
use vst::event::MidiEvent;

use log::*;
use parking_lot::RwLock;
use crate::domain::{AudioBlock, TRANSPORT};

use crate::{AudioConsumerDetails, AudioLayerInwardEvent, AudioLayerOutwardEvent, DAWUtils, MidiConsumerDetails, SampleData, TrackBackgroundProcessorMode};
use crate::constants::{EVENT_BUFFER_SIZE, BLOCK_SIZE_MAX};
use crate::event::AudioLayerTimeCriticalOutwardEvent;

const MAX_MIDI_BYTES: usize = 3;

#[derive(Copy, Clone)]
pub struct MidiCopy {
    len: usize,
    data: [u8; MAX_MIDI_BYTES],
    time: Frames,
}

impl From<RawMidi<'_>> for MidiCopy {
    fn from(midi: RawMidi<'_>) -> Self {
        let len = std::cmp::min(MAX_MIDI_BYTES, midi.bytes.len());
        let mut data = [0; MAX_MIDI_BYTES];
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
    xruns: Arc<std::sync::atomic::AtomicUsize>,
}

impl JackNotificationHandler {
    pub fn new(jack_midi_sender: crossbeam_channel::Sender<AudioLayerOutwardEvent>,
               xruns: Arc<std::sync::atomic::AtomicUsize>) -> Self {
        JackNotificationHandler {
            jack_midi_sender,
            xruns,
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
        // this runs on the jack process thread - set the denormal flush flags for it too,
        // a denormal-poisoned master mix stalls the cycle and jack overruns (noise)
        crate::domain::flush_denormals_to_zero();
        debug!("JACK: async thread started");
    }
    fn shutdown(&mut self, status: ClientStatus, reason: &str) {
        // the jack server has taken this client out - sustained overruns end with jack
        // timing the client out and never calling the callback again: the transport
        // freezes (stop does not stop the glitching output, play does nothing, the
        // position stops advancing) while the sound card replays the last buffer the
        // callback wrote. Any shutdown status means the same thing for playback: ask
        // the app to rebuild the client so the audio layer self-heals
        warn!("JACK: client shut down with status {:?} because \"{}\" - requesting client restart.", status, reason);
        let _ = self.jack_midi_sender.try_send(AudioLayerOutwardEvent::JackRestartRequired);
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
        self.xruns.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        debug!("JACK: under run occurred");
        Control::Continue
    }
}

pub struct Audio {
    audio_blocks: Vec<AudioBlock>, // being careful not to do heap allocation in the jack callback method when reading from the track consumers
    audio_block_pool: Vec<AudioBlock>,
    jack_midi_buffer: [(u32, u8, u8, u8, bool); EVENT_BUFFER_SIZE],
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
    process_producers: bool,
    master_volume: f32,
    master_pan: f32,
    rx_to_audio: crossbeam_channel::Receiver<AudioLayerInwardEvent>,
    jack_midi_sender: crossbeam_channel::Sender<AudioLayerOutwardEvent>,
    jack_midi_sender_ui: crossbeam_channel::Sender<AudioLayerOutwardEvent>,
    jack_time_critical_midi_sender: crossbeam_channel::Sender<AudioLayerTimeCriticalOutwardEvent>,
    coast: Arc<Mutex<TrackBackgroundProcessorMode>>,
    custom_midi_out_ports: Vec<Port<MidiOut>>,
    keep_alive: bool,
    preview_sample: Option<SampleData>,
    preview_sample_current_frame: i32,
    vst_host_time_info: Arc<RwLock<TimeInfo>>,
    // audio health telemetry: wall-clock cycle timing, slow cycle / xrun counters and
    // a shared counter that the notification handler bumps on every jack overrun
    cycle_count: u64,
    slow_cycles: u64,
    last_cycle_start: Option<std::time::Instant>,
    xruns: Arc<std::sync::atomic::AtomicUsize>,
    limiter_cycles: u32,
    limiter_cycles_window: u32,
}

impl Audio {
    pub fn new(client: &Client,
               rx_to_audio: crossbeam_channel::Receiver<AudioLayerInwardEvent>,
               jack_midi_sender: crossbeam_channel::Sender<AudioLayerOutwardEvent>,
               jack_midi_sender_ui: crossbeam_channel::Sender<AudioLayerOutwardEvent>,
               jack_time_critical_midi_sender: crossbeam_channel::Sender<AudioLayerTimeCriticalOutwardEvent>,
               coast: Arc<Mutex<TrackBackgroundProcessorMode>>,
               vst_host_time_info: Arc<RwLock<TimeInfo>>,
               sample_rate_in_frames: i32,
               block_size: i32,
               tempo: f64,
               xruns: Arc<std::sync::atomic::AtomicUsize>,
    ) -> Self {
        let audio_block_pool: Vec<AudioBlock> = (0..2500).map(|_| AudioBlock::default()).collect();

        Audio {
            audio_blocks: vec![],
            audio_block_pool,
            jack_midi_buffer: [(0, 0, 0, 0, false); EVENT_BUFFER_SIZE],
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
            sample_rate_in_frames: sample_rate_in_frames as f64,
            tempo,
            block_size: block_size as f64,
            frames_per_beat: Audio::frames_per_beat_calc(sample_rate_in_frames as f64, tempo),
            process_producers: true,
            master_volume: 1.0,
            master_pan: 0.0,
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
            cycle_count: 0,
            slow_cycles: 0,
            last_cycle_start: None,
            xruns,
            limiter_cycles: 0,
            limiter_cycles_window: 0,
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
                              vst_host_time_info: Arc<RwLock<TimeInfo>>,
                              sample_rate_in_frames: i32,
                              block_size: i32,
                              tempo: f64,
                              xruns: Arc<std::sync::atomic::AtomicUsize>,
    ) -> Self {
        let audio_block_pool: Vec<AudioBlock> = (0..2500).map(|_| AudioBlock::default()).collect();

        Audio {
            audio_blocks: vec![],
            audio_block_pool,
            jack_midi_buffer: [(0, 0, 0, 0, false); EVENT_BUFFER_SIZE],
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
            sample_rate_in_frames: sample_rate_in_frames as f64,
            tempo,
            block_size: block_size as f64,
            frames_per_beat: Audio::frames_per_beat_calc(sample_rate_in_frames as f64, tempo),
            process_producers: true,
            master_volume: 1.0,
            master_pan: 0.5,
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
            cycle_count: 0,
            slow_cycles: 0,
            last_cycle_start: None,
            xruns,
            limiter_cycles: 0,
            limiter_cycles_window: 0,
        }
    }

    /// Periodic audio-layer health report (~every 512 cycles). Names the trouble:
    /// a consumer that is missing cycles (a track thread starved by its own DSP), a
    /// consumer shipping sustained hot blocks (a diverging plugin) or rising
    /// slow_cycles/xruns (the whole graph is over budget) - so the noise onset can
    /// be attributed to a specific track/plugin instead of the guesswork.
    fn report_audio_health(&mut self) {
        self.cycle_count += 1;
        if self.cycle_count % 512 != 0 || self.audio_consumers.is_empty() {
            return;
        }
        // while stopped/coasting consumers legitimately deliver nothing - reset the
        // window counters quietly and report nothing
        if !self.play {
            for consumer_details in self.audio_consumers.iter_mut() {
                consumer_details.begin_health_window();
            }
            return;
        }

        let xruns = self.xruns.load(std::sync::atomic::Ordering::Relaxed);
        let mut anomalies: u32 = 0;
        let mut details_lines: Vec<String> = vec![];
        for consumer_details in self.audio_consumers.iter() {
            if consumer_details.max_missing_run() > 2 || consumer_details.max_hot_run() > 8 {
                anomalies += 1;
                details_lines.push(format!("track {} delivered={} max_missing_run={} max_hot_run={} last_peak={:.3}",
                                           consumer_details.track_id(), consumer_details.delivered_blocks(),
                                           consumer_details.max_missing_run(), consumer_details.max_hot_run(),
                                           consumer_details.last_peak()));
            }
        }
        // only ever build/log on the realtime thread when something is wrong:
        // the old periodic 'all nominal' line allocated a String and pushed it
        // into the logger every 512 cycles (~23ms) while playing - log
        // formatting and allocation have no business in the jack cycle.
        // silence from this reporter means healthy.
        if anomalies > 0 || self.slow_cycles > 10 || self.limiter_cycles_window > 128 {
            warn!("Audio health: cycles={} slow_cycles={} xruns={} limiter_cycles={}/512 consumers={} anomalous=[{}]",
                   self.cycle_count, self.slow_cycles, xruns, self.limiter_cycles_window, self.audio_consumers.len(), details_lines.join(" | "));
        }
        self.limiter_cycles_window = 0;

        for consumer_details in self.audio_consumers.iter_mut() {
            consumer_details.begin_health_window();
        }
    }

    pub fn frames_per_beat_calc(sample_rate_in_frames: f64, tempo: f64) -> u32 {
        (sample_rate_in_frames / tempo * 60.0) as u32
    }

    fn handle_inward_events(&mut self, _client: &Client) {
        // drain every pending event in one pass - the old single try_recv meant a burst
        // of transport events (rapid play/stop/seek cycles) could sit queued behind the
        // others for many jack cycles, leaving the jack transport position and the track
        // background processors playing from different positions.
        let mut events_processed = 0;
        while events_processed < 128 {
            let event = match self.rx_to_audio.try_recv() {
                Ok(event) => event,
                Err(_) => break,
            };
            events_processed += 1;
            match event {
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
                    self.block = start_block.max(0);
                    self.play_position_in_frames = self.block as u32 * self.block_size as u32;
                    self.blocks_total = number_of_blocks;
                    TRANSPORT.get().write().position_in_frames = self.play_position_in_frames;
                    self.reset_audio_watchdogs();
                }
                AudioLayerInwardEvent::Seek(start_block) => {
                    // rewind/scrub from the UI - move the jack transport to the same
                    // block the track background processors have been seeked to, the
                    // transport position must never run on from a seek while the UI
                    // thinks it has been rewound.
                    let start_block = start_block.max(0);
                    self.block = start_block;
                    self.play_position_in_frames = start_block as u32 * self.block_size as u32;
                    TRANSPORT.get().write().position_in_frames = self.play_position_in_frames;
                    self.reset_audio_watchdogs();

                    let ppq_pos = (self.play_position_in_frames as f64 * self.tempo) / (60.0 * self.sample_rate_in_frames);
                    let mut time_info = self.vst_host_time_info.write();
                    time_info.sample_pos = self.play_position_in_frames as f64;
                    time_info.ppq_pos = ppq_pos;
                }
                AudioLayerInwardEvent::Stop => {
                    // debug!(root_logger, "*************Jack stop play received.");
                    self.play = false;
                    if self.block > -1 {
                        self.block = 0;
                    }
                    self.play_position_in_frames = 0;
                    self.reset_audio_watchdogs();
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
            }
        }
    }

    /// Cheap content signature of a delivered audio block for the repetition watchdog,
    /// plus the block's peak absolute level (health telemetry). Sampling every 8th frame
    /// keeps this a couple of hundred ops per track per cycle - a stale/dead producer
    /// replaying the same block produces the same signature while producing real music
    /// does not.
    fn audio_block_signature(audio_block: &AudioBlock) -> (u64, f32) {
        let mut signature: u64 = 0xcbf29ce484222325;
        let mut peak: f32 = 0.0;
        let mut index = 0;
        while index < BLOCK_SIZE_MAX as usize {
            let left = audio_block.audio_data_left[index];
            let right = audio_block.audio_data_right[index];
            let left_abs = if left.is_finite() { left.abs() } else { f32::MAX };
            let right_abs = if right.is_finite() { right.abs() } else { f32::MAX };
            if left_abs > peak { peak = left_abs; }
            if right_abs > peak { peak = right_abs; }
            signature = signature.rotate_left(7) ^ (left.to_bits() as u64);
            signature = signature.wrapping_mul(0x00000100000001b3);
            signature = signature.rotate_left(7) ^ (right.to_bits() as u64);
            signature = signature.wrapping_mul(0x00000100000001b3);
            index += 8;
        }
        (signature, peak)
    }

    /// Transport moved - every track's repetition state is meaningless now (replaying
    /// the same passage from the top legitimately produces the same blocks again).
    fn reset_audio_watchdogs(&mut self) {
        for audio_consumer_details in self.audio_consumers.iter_mut() {
            audio_consumer_details.reset_watchdog();
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
            // Mix every track directly into the master bus, consuming at most one audio
            // block per track per jack cycle. The track background producers are paced by
            // this consumption (they write_blocking into two slot ring buffers), so one
            // block per track per cycle keeps every track locked to the jack cycle rate.
            //
            // The previous design only mixed a block once *every* track had delivered a
            // block for the same block number. A single slow or dead producer then either
            // starved the whole master output (total silence) or - with the partial mix
            // fallback - mixed time skewed blocks in separate cycles, which came out as
            // crackling, robotic audio and a wrong tempo on dense projects. A track that
            // cannot keep up now only misses its own block instead of wrecking the mix.
            let out_left = self.out_l.as_mut_slice(process_scope);
            let out_right = self.out_r.as_mut_slice(process_scope);

            for audio_consumer_details in self.audio_consumers.iter_mut() {
                let consumer = audio_consumer_details.consumer();
                if let Some(audio_block) = self.audio_block_pool.pop() {
                    self.audio_blocks.push(audio_block);

                    let mut read_audio_block = false;
                    match consumer.read(&mut self.audio_blocks) {
                        Ok(read) => read_audio_block = read == 1,
                        Err(_) => (),
                    }

                    if let Some(audio_block) = self.audio_blocks.pop() {
                        let mut mix_this_block = read_audio_block;
                        let mut peak: f32 = 0.0;
                        if read_audio_block {
                            // repetition watchdog: a producer whose plugin has died may
                            // keep shipping bit-identical non-silent blocks - the same
                            // audio block machine-gunned at the jack cycle rate sounds
                            // exactly like harsh noise. Drop a consumer's blocks once
                            // its deliveries repeat unchanged, until the content changes.
                            let (signature, block_peak) = Audio::audio_block_signature(&audio_block);
                            peak = block_peak;
                            if audio_consumer_details.observe_repetition(signature, block_peak > 1e-6) {
                                mix_this_block = false;
                            }
                        }
                        audio_consumer_details.observe_cycle(mix_this_block, peak);
                        if mix_this_block {
                            for (index, (left, right)) in out_left.iter_mut().zip(out_right.iter_mut()).enumerate() {
                                if index < audio_block.audio_data_left.len() && index < audio_block.audio_data_right.len() {
                                    // a track whose plugin has gone bad fills its blocks with
                                    // NaN/Inf (or runaway oscillation far beyond any musical
                                    // level) - one such sample would poison the whole master
                                    // bus; drop frames that are not sane
                                    let track_left = audio_block.audio_data_left[index];
                                    let track_right = audio_block.audio_data_right[index];
                                    if track_left.is_finite() && track_left.abs() <= 32.0 {
                                        *left += track_left * self.master_volume * 2.0 * left_pan;
                                    }
                                    if track_right.is_finite() && track_right.abs() <= 32.0 {
                                        *right += track_right * self.master_volume * 2.0 * right_pan;
                                    }
                                    if *left > master_channel_left_level {
                                        master_channel_left_level = *left;
                                    }
                                    if *right > master_channel_right_level {
                                        master_channel_right_level = *right;
                                    }
                                } else {
                                    break;
                                }
                            }
                        }
                        // always return the scratch block to the pool - a consumer that
                        // had nothing to read must not drain the pool one block per cycle.
                        self.audio_block_pool.push(audio_block);
                    }
                }
            }
        }

        let _ = self.jack_midi_sender_ui.try_send(AudioLayerOutwardEvent::MasterChannelLevels(master_channel_left_level, master_channel_right_level));

        self.process_preview_sample(process_scope, frames_written, &mut number_of_consumers, left_pan, right_pan);

        // master safety limiter: the master bus is a raw sum of every track with no
        // ceiling. A dense arrangement (this song builds from 1 to ~17 simultaneous
        // tracks, and percussive peaks arrive together) drives the sum far past 1.0
        // and the output hard clips every single cycle - which comes out of the
        // speakers not as "loud" but as harsh continuous noise, beginning exactly
        // where the arrangement gets dense. Soft-saturate anything over 0.8 so the
        // bus can never ship clipped garbage; material under the threshold is
        // untouched, so normal levels are unchanged.
        let out_left = self.out_l.as_mut_slice(process_scope);
        let out_right = self.out_r.as_mut_slice(process_scope);
        const THRESHOLD: f32 = 0.8;
        const HEADROOM: f32 = 1.0 - THRESHOLD;
        let mut limited_this_cycle = false;
        for (left, right) in out_left.iter_mut().zip(out_right.iter_mut()) {
            let l = *left;
            if l >= THRESHOLD || l <= -THRESHOLD {
                limited_this_cycle = true;
                if l.is_finite() {
                    let mag = if l >= 0.0 { l } else { -l };
                    let shaped = THRESHOLD + HEADROOM * ((mag - THRESHOLD) / HEADROOM).tanh();
                    *left = if l >= 0.0 { shaped } else { -shaped };
                }
                else {
                    *left = 0.0;
                }
            }
            let r = *right;
            if r >= THRESHOLD || r <= -THRESHOLD {
                limited_this_cycle = true;
                if r.is_finite() {
                    let mag = if r >= 0.0 { r } else { -r };
                    let shaped = THRESHOLD + HEADROOM * ((mag - THRESHOLD) / HEADROOM).tanh();
                    *right = if r >= 0.0 { shaped } else { -shaped };
                }
                else {
                    *right = 0.0;
                }
            }
        }
        if limited_this_cycle {
            self.limiter_cycles += 1;
            self.limiter_cycles_window += 1;
        }
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
                        let mut midi_out_writer = midi_output_port.writer(process_scope);
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
                delta_frames = self.block * self.block_size as i32 /* + event.time as i32 */;
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
            else if event.bytes.len() >= 2 && 208 <= event.bytes[0] && event.bytes[0]  <= 223  { // channel pressure (2 byte message)
                let channel_pressure = MidiEvent {
                    data: [event.bytes[0], event.bytes[1], 0],
                    delta_frames,
                    live: true,
                    note_length: None,
                    note_offset: None,
                    detune: 0,
                    note_off_velocity: 0,
                };

                let _ = self.jack_time_critical_midi_sender.try_send(AudioLayerTimeCriticalOutwardEvent::MidiEvent(channel_pressure));
            }
            else if event.bytes.len() >= 2 && 192 <= event.bytes[0] && event.bytes[0]  <= 207  { // program change (2 byte message)
                let program_change = MidiEvent {
                    data: [event.bytes[0], event.bytes[1], 0],
                    delta_frames,
                    live: true,
                    note_length: None,
                    note_offset: None,
                    detune: 0,
                    note_off_velocity: 0,
                };

                let _ = self.jack_time_critical_midi_sender.try_send(AudioLayerTimeCriticalOutwardEvent::MidiEvent(program_change));
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
                delta_frames = self.block * self.block_size as i32 + event.time as i32;
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
                self.play_position_in_frames += self.block_size as u32;
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

            if self.play_position_in_frames % self.frames_per_beat < self.block_size as u32 {
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
        // transient lock contention with one of the track threads must never gate the
        // mix for a cycle - keep the last known mode until the lock is actually taken,
        // otherwise a successful try_lock race silently mutes every track for that cycle
        if let Ok(mode) = self.coast.try_lock() {
            self.process_producers = match *mode {
                TrackBackgroundProcessorMode::AudioOut => true,
                TrackBackgroundProcessorMode::Coast => false,
                TrackBackgroundProcessorMode::Render => false,
            };
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

    pub fn get_all_midi_consumers(&mut self) -> Vec<MidiConsumerDetails<(u32, u8, u8, u8, bool)>> {
        let mut consumers = vec![];
        for _ in 0..self.midi_consumers.len() {
            consumers.push(self.midi_consumers.remove(0));
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
        // publish the cycle heartbeat for the UI pump's stalled-client watchdog
        // (see domain::JACK_LAST_CYCLE_MILLIS) - a relaxed store of a vDSO
        // clock read, no allocation, no syscall that can block
        let heartbeat_millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as u64)
            .unwrap_or(0);
        crate::domain::JACK_LAST_CYCLE_MILLIS.store(heartbeat_millis, std::sync::atomic::Ordering::Relaxed);

        // publish this thread's scheduling priority (this runs on the jack callback
        // thread) once, so the track threads can keep themselves below it
        if crate::domain::JACK_THREAD_PRIORITY.load(std::sync::atomic::Ordering::Relaxed) == 0 {
            match thread_priority::get_current_thread_priority() {
                Ok(thread_priority::ThreadPriority::Crossplatform(value)) => {
                    let value: u8 = value.into();
                    if value == 0 {
                        // SCHED_OTHER jack worker (server running without realtime)
                        debug!("JACK: callback thread is not running realtime - track threads will drop to normal scheduling.");
                        crate::domain::JACK_THREAD_PRIORITY.store(-1, std::sync::atomic::Ordering::Relaxed);
                    }
                    else {
                        debug!("JACK: callback thread running with realtime priority {} - track threads will keep below it.", value);
                        crate::domain::JACK_THREAD_PRIORITY.store(value as i32, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                Ok(other) => {
                    debug!("JACK: callback thread scheduling is {:?} - track threads will drop to normal scheduling.", other);
                    crate::domain::JACK_THREAD_PRIORITY.store(-1, std::sync::atomic::Ordering::Relaxed);
                }
                Err(_) => {
                    crate::domain::JACK_THREAD_PRIORITY.store(-1, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }

        // cycle wall-clock timing: if a cycle starts later than the nominal period the
        // engine is not keeping up and jack is overrunning (overruns are the classic
        // mechanism for the audio layer starting to play noise part way through a
        // long, heavy arrangement)
        let cycle_start = std::time::Instant::now();
        if let Some(prev_cycle_start) = self.last_cycle_start {
            let expected_micros = self.block_size / self.sample_rate_in_frames * 1_000_000.0;
            if cycle_start.duration_since(prev_cycle_start).as_micros() as f64 > expected_micros * 1.6 {
                self.slow_cycles += 1;
            }
        }
        self.last_cycle_start = Some(cycle_start);

        // handle inward events on every cycle - transport events must take effect
        // immediately so the jack position and the track processors never diverge
        self.handle_inward_events(client);

        self.zero_jack_buffers(process_scope);

        if self.process_producers {
            self.process_audio(process_scope);
            self.process_midi_out(process_scope);
            self.process_midi_in(process_scope);
            self.process_midi_control_in(process_scope);
            self.update_position_and_notify()
        }

        self.check_for_coast();
        self.report_audio_health();

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
