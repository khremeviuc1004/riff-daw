use std::sync::{Arc, Mutex};
use jack::{AudioOut, Client, ClientStatus, Control, Frames, MidiIn, MidiOut, NotificationHandler, Port, PortId, ProcessHandler, ProcessScope, RawMidi};
use rb::RbConsumer;
use vst::api::{TimeInfo, TimeInfoFlags};
use vst::event::MidiEvent;

use log::*;
use parking_lot::RwLock;
use crate::alsa::{AlsaMidiInEvent, AlsaMidiOut};
use crate::domain::{AudioBlock, MidiOutPort, TRANSPORT};

use crate::{AudioConsumerDetails, AudioLayerInwardEvent, AudioLayerOutwardEvent, DAWUtils, MidiConsumerDetails, SampleData, TrackBackgroundProcessorMode};
use crate::constants::{EVENT_BUFFER_SIZE, BLOCK_SIZE_MAX};
use crate::event::AudioLayerTimeCriticalOutwardEvent;

/// Longest raw midi message the audio layer decodes (MMC system exclusive is
/// six bytes; anything longer is truncated).
pub const MIDI_IN_MAX_BYTES: usize = 8;

/// A raw midi event handed to the engine for a cycle, abstracted over the
/// backend that captured it. `time` is the frame offset inside the cycle;
/// alsa events are timestamped with the wall clock instant they were
/// received (`when`) and the engine converts them to frame offsets against
/// the start of the cycle it is processing.
#[derive(Copy, Clone)]
pub struct MidiInEvent {
    pub time: u32,
    pub len: usize,
    pub bytes: [u8; MIDI_IN_MAX_BYTES],
    pub when: Option<std::time::Instant>,
}

pub const MIDI_IN_EVENT_EMPTY: MidiInEvent = MidiInEvent { time: 0, len: 0, bytes: [0; MIDI_IN_MAX_BYTES], when: None };

impl MidiInEvent {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    /// Build from a jack raw midi event (precise in-cycle frame time).
    pub fn from_jack(event: &RawMidi<'_>) -> Self {
        let len = std::cmp::min(MIDI_IN_MAX_BYTES, event.bytes.len());
        let mut bytes = [0u8; MIDI_IN_MAX_BYTES];
        bytes[..len].copy_from_slice(&event.bytes[..len]);
        MidiInEvent { time: event.time as u32, len, bytes, when: None }
    }

    /// Build from raw bytes captured by another backend (the engine converts
    /// `when` into a frame offset at the start of the cycle).
    pub fn from_bytes(event: &AlsaMidiInEvent) -> Self {
        let len = std::cmp::min(MIDI_IN_MAX_BYTES, event.len);
        let mut bytes = [0u8; MIDI_IN_MAX_BYTES];
        bytes[..len].copy_from_slice(&event.bytes[..len]);
        MidiInEvent { time: 0, len, bytes, when: Some(event.when) }
    }
}

/// The planar master output buffers the engine mixes into each cycle. The
/// backend owns them and moves the frames to the sound hardware (jack port
/// buffers or an alsa pcm write) after the cycle.
pub struct MasterBuffers {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

impl MasterBuffers {
    pub fn new(frames: usize) -> Self {
        Self {
            left: vec![0.0; frames],
            right: vec![0.0; frames],
        }
    }

    pub fn ensure_frames(&mut self, frames: usize) {
        if self.left.len() != frames {
            self.left.resize(frames, 0.0);
            self.right.resize(frames, 0.0);
        }
    }
}

/// Where the engine writes the midi events drained from the tracks' rings
/// this cycle. The sink matches the active midi backend; a track consumer's
/// port variant that does not match the sink is skipped (its events are
/// dropped for the cycle).
pub enum MidiOutSink<'a> {
    Jack(&'a ProcessScope),
    Alsa(&'a mut AlsaMidiOut),
    None,
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
    unsafe fn shutdown(&mut self, status: ClientStatus, reason: &str) {
        // the jack server has taken this client out - sustained overruns end with jack
        // timing the client out and never calling the callback again: the transport
        // freezes (stop does not stop the glitching output, play does nothing, the
        // position stops advancing) while the sound card replays the last buffer the
        // callback wrote. Any shutdown status means the same thing for playback: ask
        // the app to rebuild the client so the audio layer self-heals
        warn!("JACK: client shut down with status {:?} because \"{}\" - requesting client restart.", status, reason);
        let _ = self.jack_midi_sender.try_send(AudioLayerOutwardEvent::AudioLayerRestartRequired);
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

/// The master audio engine: mixes every track's blocks into the master bus,
/// moves the tracks' midi to the backend, captures live midi, advances the
/// transport and applies the volume/pan/tempo/configuration inward events.
/// It knows nothing about jack or alsa - a cycle runs against plain planar
/// output buffers, pre-gathered midi input events and a midi output sink
/// supplied by whichever backend is driving the cycle.
pub struct AudioEngine {
    audio_blocks: Vec<AudioBlock>, // being careful not to do heap allocation in the audio cycle when reading from the track consumers
    audio_block_pool: Vec<AudioBlock>,
    midi_buffer: [(u32, u8, u8, u8, bool); EVENT_BUFFER_SIZE],
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
    keep_alive: bool,
    preview_sample: Option<SampleData>,
    preview_sample_current_frame: i32,
    vst_host_time_info: Arc<RwLock<TimeInfo>>,
    // audio health telemetry: wall-clock cycle timing, slow cycle / xrun counters and
    // a shared counter that the backends bump on every overrun
    cycle_count: u64,
    slow_cycles: u64,
    last_cycle_start: Option<std::time::Instant>,
    xruns: Arc<std::sync::atomic::AtomicUsize>,
    limiter_cycles: u32,
    limiter_cycles_window: u32,
}

impl AudioEngine {
    pub fn new(rx_to_audio: crossbeam_channel::Receiver<AudioLayerInwardEvent>,
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

        AudioEngine {
            audio_blocks: vec![],
            audio_block_pool,
            midi_buffer: [(0, 0, 0, 0, false); EVENT_BUFFER_SIZE],
            audio_consumers: vec![],
            midi_consumers: vec![],
            play: false,
            block: -1,
            blocks_total: 0,
            play_position_in_frames: 0,
            sample_rate_in_frames: sample_rate_in_frames as f64,
            tempo,
            block_size: block_size as f64,
            frames_per_beat: AudioEngine::frames_per_beat_calc(sample_rate_in_frames as f64, tempo),
            process_producers: true,
            master_volume: 1.0,
            master_pan: 0.0,
            rx_to_audio,
            jack_midi_sender,
            jack_midi_sender_ui,
            jack_time_critical_midi_sender,
            coast,
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

    pub fn new_with_consumers(rx_to_audio: crossbeam_channel::Receiver<AudioLayerInwardEvent>,
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

        AudioEngine {
            audio_blocks: vec![],
            audio_block_pool,
            midi_buffer: [(0, 0, 0, 0, false); EVENT_BUFFER_SIZE],
            audio_consumers,
            midi_consumers,
            play: false,
            block: -1,
            blocks_total: 0,
            play_position_in_frames: 0,
            sample_rate_in_frames: sample_rate_in_frames as f64,
            tempo,
            block_size: block_size as f64,
            frames_per_beat: AudioEngine::frames_per_beat_calc(sample_rate_in_frames as f64, tempo),
            process_producers: true,
            master_volume: 1.0,
            master_pan: 0.5,
            rx_to_audio,
            jack_midi_sender,
            jack_midi_sender_ui,
            jack_time_critical_midi_sender,
            coast,
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
        // formatting and allocation have no business in the audio cycle.
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

    fn handle_inward_events(&mut self) {
        // drain every pending event in one pass - the old single try_recv meant a burst
        // of transport events (rapid play/stop/seek cycles) could sit queued behind the
        // others for many cycles, leaving the audio transport position and the track
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
                    self.rebuild_presentation();
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
                    // never block the realtime thread on a lock the ui or a track
                    // thread may be holding: the position is re-published every cycle
                    // by update_position_and_notify, so a skipped publish here only
                    // delays it by one cycle - a blocked write here delays (or with
                    // parking_lot's writer-priority handoff, deadlocks into) the whole
                    // graph and the backend overruns
                    if let Some(mut transport) = TRANSPORT.get().try_write() {
                        transport.position_in_frames = self.play_position_in_frames;
                    }
                    self.reset_audio_watchdogs();
                }
                AudioLayerInwardEvent::Seek(start_block) => {
                    // rewind/scrub from the UI - move the transport to the same
                    // block the track background processors have been seeked to, the
                    // transport position must never run on from a seek while the UI
                    // thinks it has been rewound.
                    let start_block = start_block.max(0);
                    self.block = start_block;
                    self.play_position_in_frames = start_block as u32 * self.block_size as u32;
                    if let Some(mut transport) = TRANSPORT.get().try_write() {
                        transport.position_in_frames = self.play_position_in_frames;
                    }
                    self.reset_audio_watchdogs();

                    let ppq_pos = (self.play_position_in_frames as f64 * self.tempo) / (60.0 * self.sample_rate_in_frames);
                    if let Some(mut time_info) = self.vst_host_time_info.try_write() {
                        time_info.sample_pos = self.play_position_in_frames as f64;
                        time_info.ppq_pos = ppq_pos;
                    }
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
                    self.frames_per_beat = AudioEngine::frames_per_beat_calc(self.sample_rate_in_frames, self.tempo);
                }
                AudioLayerInwardEvent::SampleRate(new_sample_rate) => {
                    self.sample_rate_in_frames = new_sample_rate;
                    self.frames_per_beat = AudioEngine::frames_per_beat_calc(self.sample_rate_in_frames, self.tempo);
                }
                AudioLayerInwardEvent::BlockSize(new_block_size) => {
                    self.block_size = new_block_size;
                    self.frames_per_beat = AudioEngine::frames_per_beat_calc(self.sample_rate_in_frames, self.tempo);
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
                    self.rebuild_presentation();
                }
                AudioLayerInwardEvent::TrackPresentation(track_uuid, inherent_blocks) => {
                    if let Some(consumer_detail) = self.audio_consumers.iter_mut().find(|consumer_detail| *consumer_detail.track_id() == track_uuid) {
                        if consumer_detail.presentation_inherent_blocks != inherent_blocks {
                            consumer_detail.presentation_inherent_blocks = inherent_blocks;
                            self.rebuild_presentation();
                        }
                    }
                }
                AudioLayerInwardEvent::NewMidiOutPortForTrack(track_uuid, midi_out_port) => {
                    debug!("Audio layer received: AudioLayerInwardEvent::NewMidiOutPortForTrack");
                    if let Some(midi_consumer_details) = self.midi_consumers.iter_mut().find(|midi_consumer_details| midi_consumer_details.track_uuid().clone() == track_uuid.clone()) {
                        midi_consumer_details.set_midi_out_port(Some(midi_out_port));
                    }
                    else {
                        // show an error message dialogue??
                        debug!("Failed to add a midi out port for track={}", track_uuid.as_str());
                    }
                }
                AudioLayerInwardEvent::PreviewSample(sample_data) => {
                    // the data arrives pre-loaded from the ui thread - nothing here
                    // may touch the disk or the allocator beyond the pointer swap
                    debug!("Audio layer received preview sample data.");
                    self.preview_sample = Some(*sample_data);
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
            // a transport jump (play from a position, seek, stop) must never bleed
            // old-position content out of the presentation delay queues
            audio_consumer_details.pending_blocks.clear();
        }
    }

    /// Recompute the master presentation from the consumers' inherent delays.
    /// Tracks whose plugin chains are fed by inter-track audio routing cannot be
    /// input-offset (their routed input arrives in real time - the source cannot
    /// hand over its future audio early), so their whole contribution is late by
    /// their chain's reported plugin latency. The mix aligns everything by holding
    /// each consumer's blocks in its presentation queue until the common
    /// reference: the largest inherent delay present (P). Every track then lands
    /// in time with every other, the entire mix simply being P blocks late.
    /// Only called from control-path inward events (consumer add/remove and
    /// presentation updates), never from the realtime mix loop.
    fn rebuild_presentation(&mut self) {
        const PRESENTATION_MAX_DELAY_BLOCKS: usize = 64;
        let mut presentation: usize = 0;
        for audio_consumer_details in self.audio_consumers.iter() {
            presentation = presentation.max(audio_consumer_details.presentation_inherent_blocks.max(0) as usize);
        }
        let presentation = presentation.min(PRESENTATION_MAX_DELAY_BLOCKS);

        for audio_consumer_details in self.audio_consumers.iter_mut() {
            audio_consumer_details.presentation_delay_blocks =
                presentation.saturating_sub(audio_consumer_details.presentation_inherent_blocks.max(0) as usize);
            audio_consumer_details.pending_blocks.clear();
        }

        // queued blocks live outside the pool for the delay span - make sure the
        // pool can still hand one scratch block to every consumer each cycle.
        // (this Vec growth runs on control events, which is where the rest of the
        // control-path allocation in the audio layer already happens)
        let pool_needed = self.audio_consumers.len() * (presentation + 2);
        while self.audio_block_pool.len() < pool_needed {
            self.audio_block_pool.push(AudioBlock::default());
        }
    }

    fn zero_buffers(&mut self, buffers: &mut MasterBuffers) {
        // zero the master buffers
        for index in 0..buffers.right.len() {
            match buffers.left.get_mut(index) {
                Some(left) => *left = 0.0,
                None => (),
            }
            match buffers.right.get_mut(index) {
                Some(right) => *right = 0.0,
                None => (),
            }
        }
    }

    fn process_audio(&mut self, buffers: &mut MasterBuffers) {
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
            // block per track per cycle. The track background producers are paced by
            // this consumption (they write_blocking into two slot ring buffers), so one
            // block per track per cycle keeps every track locked to the cycle rate.
            //
            // The previous design only mixed a block once *every* track had delivered a
            // block for the same block number. A single slow or dead producer then either
            // starved the whole master output (total silence) or - with the partial mix
            // fallback - mixed time skewed blocks in separate cycles, which came out as
            // crackling, robotic audio and a wrong tempo on dense projects. A track that
            // cannot keep up now only misses its own block instead of wrecking the mix.
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
                        let mut queued = false;
                        if read_audio_block {
                            // repetition watchdog: a producer whose plugin has died may
                            // keep shipping bit-identical non-silent blocks - the same
                            // audio block machine-gunned at the cycle rate sounds
                            // exactly like harsh noise. Drop a consumer's blocks once
                            // its deliveries repeat unchanged, until the content changes.
                            let (signature, block_peak) = AudioEngine::audio_block_signature(&audio_block);
                            if !audio_consumer_details.observe_repetition(signature, block_peak > 1e-6) {
                                audio_consumer_details.pending_blocks.push_back((audio_block, block_peak));
                                queued = true;
                            }
                        }
                        if !queued {
                            self.audio_block_pool.push(audio_block);
                        }
                    }

                    // presentation delay line (inter-track audio routing latency):
                    // queued blocks are only mixed once the queue has been drained
                    // deeper than the consumer's presentation delay. A routed-fed
                    // track carries its chain's inherent delay (its input cannot be
                    // read ahead - the source cannot hand over future audio early),
                    // every other track is held back to match, so the whole mix
                    // presents the timeline P blocks late but perfectly in time.
                    let mut mix_this_block = false;
                    let mut peak: f32 = 0.0;
                    let mut presented: Option<AudioBlock> = None;
                    if audio_consumer_details.pending_blocks.len() > audio_consumer_details.presentation_delay_blocks {
                        if let Some((audio_block, block_peak)) = audio_consumer_details.pending_blocks.pop_front() {
                            mix_this_block = true;
                            peak = block_peak;
                            presented = Some(audio_block);
                        }
                    }
                    audio_consumer_details.observe_cycle(mix_this_block || audio_consumer_details.in_presentation_fill(), peak);
                    if let Some(audio_block) = presented {
                        if mix_this_block {
                            for (index, (left, right)) in buffers.left.iter_mut().zip(buffers.right.iter_mut()).enumerate() {
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

                        // always return the presented scratch block to the pool - a
                        // consumer that had nothing to read must not drain the pool
                        // one block per cycle.
                        self.audio_block_pool.push(audio_block);
                    }
                }
            }
        }

        let _ = self.jack_midi_sender_ui.try_send(AudioLayerOutwardEvent::MasterChannelLevels(master_channel_left_level, master_channel_right_level));

        self.process_preview_sample(buffers, frames_written, &mut number_of_consumers, left_pan, right_pan);

        // master safety limiter: the master bus is a raw sum of every track with no
        // ceiling. A dense arrangement (this song builds from 1 to ~17 simultaneous
        // tracks, and percussive peaks arrive together) drives the sum far past 1.0
        // and the output hard clips every single cycle - which comes out of the
        // speakers not as "loud" but as harsh continuous noise, beginning exactly
        // where the arrangement gets dense. Soft-saturate anything over 0.8 so the
        // bus can never ship clipped garbage; material under the threshold is
        // untouched, so normal levels are unchanged.
        const THRESHOLD: f32 = 0.8;
        const HEADROOM: f32 = 1.0 - THRESHOLD;
        let mut limited_this_cycle = false;
        for (left, right) in buffers.left.iter_mut().zip(buffers.right.iter_mut()) {
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

    fn process_preview_sample(&mut self, buffers: &mut MasterBuffers, frames_written: usize, number_of_consumers: &mut f32, left_pan: f32, right_pan: f32) {
        // mix straight into the master buffers: the old version pushed every preview
        // frame through two scratch Vecs (a heap allocation per cycle on the
        // realtime thread) before writing them out
        let preview_sample_current_frame = self.preview_sample_current_frame() as usize;
        let mut sample_length = 0usize;
        let mut sample_channels = 0usize;
        let mut has_sample = false;
        if let Some(sample) = self.preview_sample_mut() {
            has_sample = true;
            sample_channels = sample.channels() as usize;
            sample_length = sample.samples().len();
        }
        if has_sample {
            let scale = 1.0 / *number_of_consumers * self.master_volume * 2.0;
            for frame in 0..frames_written {
                let left_channel_sample_index = (frame + preview_sample_current_frame) * sample_channels;
                let right_channel_sample_index = left_channel_sample_index + 1;
                // copy the two values out (a reborrow of the buffer per frame
                // is just a pointer read) instead of holding the sample borrow and
                // the buffer borrow at the same time
                let frames = if left_channel_sample_index < sample_length {
                    self.preview_sample().as_ref().map(|sample| {
                        if sample_channels == 1 {
                            let value = *sample.samples().get(left_channel_sample_index).unwrap_or(&0.0);
                            (value, value)
                        }
                        else {
                            let left = *sample.samples().get(left_channel_sample_index).unwrap_or(&0.0);
                            let right = *sample.samples().get(right_channel_sample_index).unwrap_or(&0.0);
                            (left, right)
                        }
                    })
                } else {
                    None
                };
                if let Some((left_channel, right_channel)) = frames {
                    if frame < buffers.left.len() && frame < buffers.right.len() {
                        buffers.left[frame] += left_channel * scale * left_pan;
                        buffers.right[frame] += right_channel * scale * right_pan;
                    }
                }
            }
        }

        self.set_preview_sample_current_frame(self.preview_sample_current_frame() + frames_written as i32);
    }

    fn process_midi_out(&mut self, sink: &mut MidiOutSink) {
        for midi_consumer_detail in self.midi_consumers.iter_mut() {
            let consumer_midi = midi_consumer_detail.consumer_mut();
            match consumer_midi.read(&mut self.midi_buffer) {
                Ok(read) => if read > 0 {
                    // debug!("Audio layer received some midi events: {}", read);
                    if let Some(midi_output_port) = midi_consumer_detail.midi_out_port_mut() {
                        match (midi_output_port, &mut *sink) {
                            (MidiOutPort::Jack(port), MidiOutSink::Jack(process_scope)) => {
                                let mut midi_out_writer = port.writer(process_scope);
                                for count in 0..read {
                                    let (frames, byte1, byte2, byte3, active) = self.midi_buffer[count];
                                    if active {
                                        // debug!("Audio layer sending a midi event: {}, {}, {}, {}", frames, byte1, byte2, byte3);
                                        let bytes = [byte1, byte2, byte3];
                                        let event = RawMidi { time: frames, bytes:  &bytes};
                                        let _ = midi_out_writer.write(&event);
                                    }
                                    else {
                                        break;
                                    }
                                }
                            }
                            (MidiOutPort::Alsa(dest), MidiOutSink::Alsa(alsa_out)) => {
                                let sample_rate = self.sample_rate_in_frames;
                                for count in 0..read {
                                    let (frames, byte1, byte2, byte3, active) = self.midi_buffer[count];
                                    if active {
                                        alsa_out.schedule_midi(dest, frames, byte1, byte2, byte3, sample_rate);
                                    }
                                    else {
                                        break;
                                    }
                                }
                                let _ = alsa_out.seq.drain_output();
                            }
                            _ => (),
                        }
                    }
                },
                Err(_) => (),
            }
        }
    }

    /// Convert an event timestamped with a wall clock instant (alsa events -
    /// the sequencer does not hand the engine cycle-aligned frame offsets)
    /// into a frame offset inside the cycle being processed. The captured
    /// events were received during the window between the previous cycle
    /// start and this one, so they are mapped onto the frames just elapsed:
    /// an event received right before this cycle began lands near the end of
    /// the previous cycle's window.
    fn event_frame(&self, event: &MidiInEvent) -> u32 {
        if let Some(when) = event.when {
            if let Some(cycle_start) = self.last_cycle_start {
                if let Some(back) = cycle_start.checked_duration_since(when) {
                    let block_frames = self.block_size.max(0.0) as u32;
                    let back_frames = (back.as_micros() as f64 * self.sample_rate_in_frames / 1_000_000.0) as u32;
                    return block_frames.saturating_sub(back_frames);
                }
                return 0;
            }
            0
        }
        else {
            event.time
        }
    }

    fn process_midi_in(&mut self, events: &[MidiInEvent]) {
        for event in events.iter() {
            let mut delta_frames = 0;

            if self.play && self.block > -1 {
                delta_frames = self.block * self.block_size as i32 /* + event.time as i32 */;
            }

            let bytes = event.bytes();

            if bytes.len() >= 3 && 144 <= bytes[0] && bytes[0] <= 159 && bytes[2] > 0 { // note on
                let note_on = MidiEvent {
                    data: [bytes[0], bytes[1], bytes[2]],
                    delta_frames,
                    live: true,
                    note_length: None,
                    note_offset: None,
                    detune: 0,
                    note_off_velocity: 0,
                };

                let _ = self.jack_time_critical_midi_sender.try_send(AudioLayerTimeCriticalOutwardEvent::MidiEvent(note_on));
            }
            else if (bytes.len() >= 3 && 128 <= bytes[0] && bytes[0] <= 143)
                || (bytes.len() >= 3 && 144 <= bytes[0] && bytes[0] <= 159 && bytes[2] == 0) { // note off (a zero velocity note on is a note off)
                let note_off = MidiEvent {
                    data: [128 + (bytes[0] & 0x0F), bytes[1], bytes[2]],
                    delta_frames,
                    live: true,
                    note_length: None,
                    note_offset: None,
                    detune: 0,
                    note_off_velocity: 0,
                };

                let _ = self.jack_time_critical_midi_sender.try_send(AudioLayerTimeCriticalOutwardEvent::MidiEvent(note_off));
            }
            else if bytes.len() >= 3 && 176 <= bytes[0] && bytes[0]  <= 191  { // controllers
                let controller = Self::create_midi_event(bytes, &mut delta_frames);

                let _ = self.jack_time_critical_midi_sender.try_send(AudioLayerTimeCriticalOutwardEvent::MidiEvent(controller));
            }
            else if bytes.len() >= 3 && 224 <= bytes[0] && bytes[0]  <= 239  { // pitch bend
                let pitch_bend = Self::create_midi_event(bytes, &mut delta_frames);

                let _ = self.jack_time_critical_midi_sender.try_send(AudioLayerTimeCriticalOutwardEvent::MidiEvent(pitch_bend));
            }
            else if bytes.len() >= 3 && 160 <= bytes[0] && bytes[0]  <= 175  { // polyphonic key pressure
                let polyphonic_key_pressure = Self::create_midi_event(bytes, &mut delta_frames);

                let _ = self.jack_time_critical_midi_sender.try_send(AudioLayerTimeCriticalOutwardEvent::MidiEvent(polyphonic_key_pressure));
            }
            else if bytes.len() >= 2 && 208 <= bytes[0] && bytes[0]  <= 223  { // channel pressure (2 byte message)
                let channel_pressure = MidiEvent {
                    data: [bytes[0], bytes[1], 0],
                    delta_frames,
                    live: true,
                    note_length: None,
                    note_offset: None,
                    detune: 0,
                    note_off_velocity: 0,
                };

                let _ = self.jack_time_critical_midi_sender.try_send(AudioLayerTimeCriticalOutwardEvent::MidiEvent(channel_pressure));
            }
            else if bytes.len() >= 2 && 192 <= bytes[0] && bytes[0]  <= 207  { // program change (2 byte message)
                let program_change = MidiEvent {
                    data: [bytes[0], bytes[1], 0],
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

    fn create_midi_event(bytes: &[u8], delta_frames: &mut i32) -> MidiEvent {
        MidiEvent {
            data: [bytes[0], bytes[1], bytes[2]],
            delta_frames: *delta_frames,
            live: true,
            note_length: None,
            note_offset: None,
            detune: 0,
            note_off_velocity: 0,
        }
    }

    fn process_midi_control_in(&mut self, events: &[MidiInEvent]) {
        for event in events.iter() {
            let event_time = self.event_frame(event);
            let mut delta_frames = 0;

            if self.play && self.block > -1 {
                delta_frames = self.block * self.block_size as i32 + event_time as i32;
            }

            let bytes = event.bytes();

            if bytes.len() >= 3 && 144 <= bytes[0] && bytes[0] <= 159 { // note on
                let note_on = MidiEvent {
                    data: [144, bytes[1], bytes[2]],
                    delta_frames,
                    live: true,
                    note_length: None,
                    note_offset: None,
                    detune: 0,
                    note_off_velocity: 0,
                };


                let _ = self.jack_midi_sender_ui.try_send(AudioLayerOutwardEvent::MidiControlEvent(note_on));
            }
            else if bytes.len() >= 3 && 128 <= bytes[0] && bytes[0] <= 143 { // note off
                let note_off = MidiEvent {
                    data: [128, bytes[1], bytes[2]],
                    delta_frames,
                    live: true,
                    note_length: None,
                    note_offset: None,
                    detune: 0,
                    note_off_velocity: 0,
                };

                let _ = self.jack_midi_sender_ui.try_send(AudioLayerOutwardEvent::MidiControlEvent(note_off));
            }
            else if bytes.len() >= 3 && 176 <= bytes[0] && bytes[0]  <= 191  { // controllers
                let controller = Self::create_midi_event(bytes, &mut delta_frames);
                if controller.data[0] as i32 >= 176 && (controller.data[0] as i32 <= (176 + 15)) {
                    let _ = self.jack_time_critical_midi_sender.try_send(AudioLayerTimeCriticalOutwardEvent::TrackVolumePanLevel(controller));
                }
                else {
                    let _ = self.jack_midi_sender.try_send(AudioLayerOutwardEvent::MidiControlEvent(controller));
                }
            }
            else if bytes.len() == 6 && bytes[0] == 240 && bytes[1] == 127 && bytes[3] == 6 {
                let mmc_sysex_bytes: [u8; 6] = [240, 127, bytes[2], 6, bytes[4], 247];
                // try_send, never send: a blocking channel send on the realtime
                // thread parks the audio cycle whenever the consumer pauses
                match self.jack_midi_sender.try_send(AudioLayerOutwardEvent::GeneralMMCEvent(mmc_sysex_bytes)) {
                    Ok(_) => {}
                    Err(err) => {
                        debug!("audio layer problem sending General MMC event: {:?}", err);
                    }
                }
            }
            else if bytes.len() == 3 {
                debug!("audio layer - received a unknown message: {} {} {}", bytes[0], bytes[1], bytes[2]);
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

            // the realtime thread must never park on a lock: the ui thread takes the
            // transport write lock while playing back edits, and plugins read the host
            // time info on their (real-time-priority) track threads for the length of a
            // plugin block. A blocking write here stalls the cycle until the deadline
            // is missed, the backend overruns and - after sustained misses - takes the
            // whole audio layer out. Skipping a publish costs one cycle of position
            // latency, blocking costs the session.
            if let Some(mut transport) = TRANSPORT.get().try_write() {
                transport.position_in_frames = self.play_position_in_frames;
            }

            {
                let ppq_pos = (self.play_position_in_frames as f64 * self.tempo) / (60.0 * self.sample_rate_in_frames);
                let mut flags = TimeInfoFlags::TRANSPORT_CHANGED.bits();
                flags |= TimeInfoFlags::TRANSPORT_PLAYING.bits(); // transport playing
                flags |= TimeInfoFlags::TEMPO_VALID.bits(); // tempo valid
                flags |= TimeInfoFlags::TIME_SIG_VALID.bits(); // time signature valid
                flags |= TimeInfoFlags::PPQ_POS_VALID.bits(); // ppq pos valid

                if let Some(mut time_info) = self.vst_host_time_info.try_write() {
                    time_info.flags = flags;
                    time_info.sample_pos = self.play_position_in_frames as f64;
                    time_info.ppq_pos = ppq_pos;
                }
            }

            if self.play_position_in_frames % self.frames_per_beat < self.block_size as u32 {
                let _ = self.jack_midi_sender_ui.try_send(AudioLayerOutwardEvent::PlayPositionInFrames(self.play_position_in_frames));
            }
        }
        else if !self.play { // not playing anymore
            // try_lock (was: blocking write) - the time info read lock is held by
            // track threads inside plugin blocks, and the transport stopped flag must
            // never be gated on lock contention in the realtime cycle: a blocked write
            // here is a missed deadline and eventually a timed out audio layer
            if let Some(mut time_info) = self.vst_host_time_info.try_write() {
                if time_info.flags & (1 << 1) == 2 { // transport playing need to stop
                    let mut flags = TimeInfoFlags::TRANSPORT_CHANGED.bits(); // transport changed

                    flags |= TimeInfoFlags::TEMPO_VALID.bits(); // tempo valid
                    flags |= TimeInfoFlags::TIME_SIG_VALID.bits(); // time signature valid
                    flags |= TimeInfoFlags::PPQ_POS_VALID.bits(); // ppq pos valid
                    time_info.flags = flags;
                }
                else if time_info.flags & 1 == 1 { // transport changed but not playing
                    let mut flags = 0; // transport not changed

                    flags |= TimeInfoFlags::TEMPO_VALID.bits(); // tempo valid
                    flags |= TimeInfoFlags::TIME_SIG_VALID.bits(); // time signature valid
                    flags |= TimeInfoFlags::PPQ_POS_VALID.bits(); // ppq pos valid
                    time_info.flags = flags;
                }
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

    /// Count an overrun observed by the backend (alsa underrun recovery),
    /// sharing the counter the health report reads.
    pub fn count_xrun(&mut self) {
        self.xruns.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// One audio layer cycle: the backend calls this once per cycle with its
    /// master buffers (the backend moves the frames to the hardware afterwards),
    /// the raw midi events it captured for this cycle and a sink for the tracks'
    /// outgoing midi. Returns false when the layer has been asked to shut down.
    pub fn process_cycle(&mut self,
                         buffers: &mut MasterBuffers,
                         midi_in_events: &[MidiInEvent],
                         midi_control_in_events: &[MidiInEvent],
                         sink: &mut MidiOutSink,
                         frames: usize) -> bool {
        // publish the cycle heartbeat for the UI pump's stalled-layer watchdog
        // (see domain::JACK_LAST_CYCLE_MILLIS) - a relaxed store of a vDSO
        // clock read, no allocation, no syscall that can block
        let heartbeat_millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as u64)
            .unwrap_or(0);
        crate::domain::JACK_LAST_CYCLE_MILLIS.store(heartbeat_millis, std::sync::atomic::Ordering::Relaxed);

        // publish this thread's scheduling priority (this runs on the backend's
        // realtime thread) once, so the track threads can keep themselves below it
        if crate::domain::JACK_THREAD_PRIORITY.load(std::sync::atomic::Ordering::Relaxed) == 0 {
            match thread_priority::get_current_thread_priority() {
                Ok(thread_priority::ThreadPriority::Crossplatform(value)) => {
                    let value: u8 = value.into();
                    if value == 0 {
                        debug!("Audio: callback thread is not running realtime - track threads will drop to normal scheduling.");
                        crate::domain::JACK_THREAD_PRIORITY.store(-1, std::sync::atomic::Ordering::Relaxed);
                    }
                    else {
                        debug!("Audio: callback thread running with realtime priority {} - track threads will keep below it.", value);
                        crate::domain::JACK_THREAD_PRIORITY.store(value as i32, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                Ok(other) => {
                    debug!("Audio: callback thread scheduling is {:?} - track threads will drop to normal scheduling.", other);
                    crate::domain::JACK_THREAD_PRIORITY.store(-1, std::sync::atomic::Ordering::Relaxed);
                }
                Err(_) => {
                    crate::domain::JACK_THREAD_PRIORITY.store(-1, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }

        // cycle wall-clock timing: if a cycle starts later than the nominal period the
        // engine is not keeping up and the backend is overrunning (overruns are the
        // classic mechanism for the audio layer starting to play noise part way through
        // a long, heavy arrangement)
        let cycle_start = std::time::Instant::now();
        if let Some(prev_cycle_start) = self.last_cycle_start {
            let expected_micros = self.block_size / self.sample_rate_in_frames * 1_000_000.0;
            if cycle_start.duration_since(prev_cycle_start).as_micros() as f64 > expected_micros * 1.6 {
                self.slow_cycles += 1;
            }
        }
        self.last_cycle_start = Some(cycle_start);

        // handle inward events on every cycle - transport events must take effect
        // immediately so the audio layer position and the track processors never diverge
        self.handle_inward_events();

        buffers.ensure_frames(frames);
        self.zero_buffers(buffers);

        if self.process_producers {
            self.process_audio(buffers);
            self.process_midi_out(sink);
            self.process_midi_in(midi_in_events);
            self.process_midi_control_in(midi_control_in_events);
            self.update_position_and_notify()
        }

        self.check_for_coast();
        self.report_audio_health();

        self.keep_alive
    }
}

/// The jack backend: owns the engine plus the jack ports, gathers the raw midi
/// arriving on the jack ports (and from the alsa midi frontend when the midi
/// backend is alsa), runs one engine cycle per jack callback and copies the
/// mixed master buffers into the jack output port buffers.
pub struct JackProcessHandler {
    pub engine: AudioEngine,
    out_l: Port<AudioOut>,
    out_r: Port<AudioOut>,
    /// DAW:midi_out monitor port - never written by the engine, it exists so the
    /// port is present in the jack graph (the Port handle must stay alive)
    #[allow(dead_code)]
    midi_out: Port<MidiOut>,
    midi_in: Port<MidiIn>,
    midi_control_in: Port<MidiIn>,
    buffers: MasterBuffers,
    midi_in_scratch: [MidiInEvent; EVENT_BUFFER_SIZE],
    midi_control_in_scratch: [MidiInEvent; EVENT_BUFFER_SIZE],
    // when the midi backend is alsa but the audio still runs on the jack cycle,
    // this handler also drives the alsa sequencer output and drains the alsa
    // sequencer input frontend
    alsa_midi_out: Option<AlsaMidiOut>,
    alsa_midi_in_rx: Option<crossbeam_channel::Receiver<AlsaMidiInEvent>>,
    alsa_midi_control_in_rx: Option<crossbeam_channel::Receiver<AlsaMidiInEvent>>,
}

impl JackProcessHandler {
    pub fn new(client: &Client, engine: AudioEngine) -> Result<Self, jack::Error> {
        let frames = client.buffer_size() as usize;
        Ok(JackProcessHandler {
            engine,
            out_l: client.register_port("out_l", AudioOut::default())?,
            out_r: client.register_port("out_r", AudioOut::default())?,
            midi_out: client.register_port("midi_out", MidiOut::default())?,
            midi_in: client.register_port("midi_in", MidiIn::default())?,
            midi_control_in: client.register_port("midi_control_in", MidiIn::default())?,
            buffers: MasterBuffers::new(frames),
            midi_in_scratch: [MIDI_IN_EVENT_EMPTY; EVENT_BUFFER_SIZE],
            midi_control_in_scratch: [MIDI_IN_EVENT_EMPTY; EVENT_BUFFER_SIZE],
            alsa_midi_out: None,
            alsa_midi_in_rx: None,
            alsa_midi_control_in_rx: None,
        })
    }

    /// Attach the alsa sequencer output frontend (midi backend = alsa while
    /// the audio layer runs on jack).
    pub fn set_alsa_midi(&mut self,
                         alsa_midi_out: Option<AlsaMidiOut>,
                         alsa_midi_in_rx: Option<crossbeam_channel::Receiver<AlsaMidiInEvent>>,
                         alsa_midi_control_in_rx: Option<crossbeam_channel::Receiver<AlsaMidiInEvent>>) {
        self.alsa_midi_out = alsa_midi_out;
        self.alsa_midi_in_rx = alsa_midi_in_rx;
        self.alsa_midi_control_in_rx = alsa_midi_control_in_rx;
    }

    /// Fill a scratch array with the alsa sequencer events queued since the
    /// last cycle (the input frontend thread timestamps them, the engine
    /// converts them to in-cycle frame offsets).
    fn drain_alsa_midi(rx: &Option<crossbeam_channel::Receiver<AlsaMidiInEvent>>,
                       scratch: &mut [MidiInEvent; EVENT_BUFFER_SIZE]) -> usize {
        if let Some(rx) = rx {
            let mut count = 0;
            while count < scratch.len() {
                match rx.try_recv() {
                    Ok(event) => {
                        scratch[count] = MidiInEvent::from_bytes(&event);
                        count += 1;
                    }
                    Err(_) => break,
                }
            }
            count
        }
        else {
            0
        }
    }
}

impl ProcessHandler for JackProcessHandler {
    fn process(&mut self, _client: &Client, process_scope: &ProcessScope) -> Control {
        let frames = process_scope.n_frames() as usize;

        // gather this cycle's raw midi: the jack ports when the midi backend is
        // jack, the alsa sequencer frontend when it is alsa
        let mut midi_in_count = 0;
        let mut midi_control_in_count = 0;
        if self.alsa_midi_out.is_some() || self.alsa_midi_in_rx.is_some() || self.alsa_midi_control_in_rx.is_some() {
            midi_in_count = Self::drain_alsa_midi(&self.alsa_midi_in_rx, &mut self.midi_in_scratch);
            midi_control_in_count = Self::drain_alsa_midi(&self.alsa_midi_control_in_rx, &mut self.midi_control_in_scratch);
        }
        else {
            for event in self.midi_in.iter(process_scope) {
                if midi_in_count < EVENT_BUFFER_SIZE {
                    self.midi_in_scratch[midi_in_count] = MidiInEvent::from_jack(&event);
                    midi_in_count += 1;
                }
            }
            for event in self.midi_control_in.iter(process_scope) {
                if midi_control_in_count < EVENT_BUFFER_SIZE {
                    self.midi_control_in_scratch[midi_control_in_count] = MidiInEvent::from_jack(&event);
                    midi_control_in_count += 1;
                }
            }
        }

        // service the alsa midi control commands (per-track port create/destroy,
        // device subscriptions) and choose the sink the tracks' midi drains into
        {
            if let Some(alsa_out) = self.alsa_midi_out.as_mut() {
                alsa_out.service_commands();
            }
        }
        let alsa_out_present = self.alsa_midi_out.is_some();
        let keep_alive = {
            let mut sink = if alsa_out_present {
                MidiOutSink::Alsa(self.alsa_midi_out.as_mut().unwrap())
            }
            else {
                MidiOutSink::Jack(process_scope)
            };
            self.engine.process_cycle(&mut self.buffers,
                                      &self.midi_in_scratch[..midi_in_count],
                                      &self.midi_control_in_scratch[..midi_control_in_count],
                                      &mut sink,
                                      frames)
        };

        // hand the mixed master buffers to the jack ports
        let out_left = self.out_l.as_mut_slice(process_scope);
        let out_right = self.out_r.as_mut_slice(process_scope);
        let copy_frames = frames.min(self.buffers.left.len());
        out_left[..copy_frames].copy_from_slice(&self.buffers.left[..copy_frames]);
        out_right[..copy_frames].copy_from_slice(&self.buffers.right[..copy_frames]);

        if keep_alive {
            Control::Continue
        }
        else {
            Control::Quit
        }
    }
}
