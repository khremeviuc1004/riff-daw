/// The ALSA audio/midi backend.
///
/// Audio: a dedicated realtime loop thread drives the engine - one engine
/// cycle per pcm period; the master buffers are interleaved and written to
/// the playback device (the blocking `writei` paces the cycle against the
/// hardware clock; underruns are recovered in place and counted in the
/// shared xrun counter).
///
/// Midi: the alsa sequencer. Outgoing track midi drains into per-track seq
/// ports (named by track uuid) on the "riff-daw" client and is scheduled on
/// the queue with a relative real-time stamp so in-block frame offsets are
/// preserved; the sequencer delivers them to whatever device ports the
/// tracks are subscribed to. Incoming midi is captured by a small frontend
/// thread (the "riff-daw-midi-in" client with the `midi_in` /
/// `midi_control_in` ports subscribed from the configuration) and handed to
/// whichever thread runs the engine cycle through bounded channels.
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use alsa::card::Iter as CardIter;
use alsa::pcm::{Access, Format, HwParams, IO, PCM};
use alsa::{Direction, ValueOr};
use alsa::seq::{Addr, ClientIter, EventType, MidiEvent as SeqMidiEvent, PortCap, PortSubscribe, PortType, QueueTempo, Seq};
use crossbeam_channel::{Receiver, Sender};
use log::*;

use crate::audio::{AudioEngine, MasterBuffers, MidiInEvent, MidiOutSink, MIDI_IN_MAX_BYTES, MIDI_IN_EVENT_EMPTY};
use crate::constants::EVENT_BUFFER_SIZE;
use crate::domain::{MidiConsumerDetails, MidiOutPort};

const EPIPE: i32 = libc::EPIPE;
const EINTR: i32 = libc::EINTR;
const SND_SEQ_EVENT_FILTER_SOFT: i32 = 0x1;
const SND_SEQ_EVENT_FILTER_HARD: i32 = 0x2;
// the kernel sequencer system client - never a device to connect to
const SND_SEQ_CLIENT_SYSTEM: i32 = 0;

/// A raw midi message captured by the alsa sequencer input frontend. The
/// engine converts `when` into a frame offset inside the current cycle.
#[derive(Copy, Clone)]
pub struct AlsaMidiInEvent {
    pub bytes: [u8; MIDI_IN_MAX_BYTES],
    pub len: usize,
    pub when: std::time::Instant,
}

/// The snd_midi_event coder is plain per-stream byte state; the wrapper it
/// lives in is only ever touched by the thread that owns it (the engine
/// cycle thread or the input frontend thread), never concurrently.
pub struct SendSeqMidiEvent(SeqMidiEvent);

unsafe impl Send for SendSeqMidiEvent {}

/// Open the playback device and negotiate the hardware parameters (sample
/// format, rate, period). The requested block size becomes the period size
/// (or its nearest supported value) so one engine cycle == one period.
/// Returns (pcm, actual sample rate, actual period in frames).
pub fn open_playback(device: &str, want_rate: i32, want_block: i32) -> Result<(PCM, i32, i32), String> {
    let pcm = match PCM::new(device, Direction::Playback, false) {
        Ok(pcm) => pcm,
        Err(e) => return Err(format!("{}", e)),
    };

    let mut last_err = String::from("no sample format could be negotiated");
    for format in [Format::float(), Format::s32(), Format::s16()] {
        let hwp = match HwParams::any(&pcm) {
            Ok(h) => h,
            Err(e) => return Err(format!("{}", e)),
        };
        if hwp.set_access(Access::RWInterleaved).is_err()
            || hwp.set_channels(2).is_err()
            || hwp.set_rate(want_rate as u32, ValueOr::Nearest).is_err()
            || hwp.set_format(format).is_err() {
            last_err = format!("the device does not support the {:?} format at {} Hz", format, want_rate);
            continue;
        }
        // the device is not obliged to grant the requested period size
        // (drivers constrain it) - ask for the nearest supported value; the
        // caller reconciles the configuration with what was actually granted
        // exactly like the jack server's buffer size adjustments are
        let granted_period = match hwp.set_period_size_near(want_block as alsa::pcm::Frames, ValueOr::Nearest) {
            Ok(period) => period,
            Err(_) => {
                last_err = format!("the device does not support the {} block size", want_block);
                continue;
            }
        };
        if hwp.set_buffer_size_near(granted_period * 2).is_err() {
            last_err = format!("the device does not support the double period buffer for a {} frame block", want_block);
            continue;
        }
        if pcm.hw_params(&hwp).is_err() {
            last_err = format!("{:?} parameters were rejected by the device", format);
            continue;
        }
        let actual_rate = hwp.get_rate().unwrap_or(want_rate as u32) as i32;
        let actual_period = hwp.get_period_size().unwrap_or(granted_period) as i32;

        let swp = match pcm.sw_params_current() {
            Ok(swp) => swp,
            Err(e) => {
                last_err = format!("{}", e);
                continue;
            }
        };
        // start playing as soon as the first full cycle of frames is written;
        // wake the writer once per period so the loop paces itself against
        // the hardware clock
        let _ = swp.set_start_threshold(actual_period as alsa::pcm::Frames);
        let _ = swp.set_avail_min(actual_period as alsa::pcm::Frames);
        if pcm.sw_params(&swp).is_err() {
            last_err = "software parameters were rejected by the device".to_string();
            continue;
        }

        info!("ALSA: opened playback device \"{}\" at sample rate={} period frames={} format={:?}",
             device, actual_rate, actual_period, format);
        // the negotiated params object borrows the pcm - release it before
        // handing the pcm over to the audio loop
        drop(swp);
        drop(hwp);
        return Ok((pcm, actual_rate, actual_period));
    }
    Err(last_err)
}

/// ALSA playback devices offered in the preferences: the "default" device
/// plus a direct and a plug hw device per sound card. Returns
/// (device name, display name) pairs.
pub fn alsa_playback_devices() -> Vec<(String, String)> {
    let mut devices = vec![("default".to_string(), "Default (system default device)".to_string())];
    for card in CardIter::new() {
        let card = match card {
            Ok(card) => card,
            Err(_) => continue,
        };
        let index = card.get_index();
        let longname = card.get_longname().unwrap_or_else(|_| format!("Card {}", index));
        devices.push((format!("hw:{}", index), format!("{} (direct hardware)", longname)));
        devices.push((format!("plughw:{}", index), format!("{} (converted plug)", longname)));
    }
    devices
}

/// Parse a comma separated list of alsa sequencer port addresses ("20:0,72:1").
pub fn parse_midi_addr_list(list: &str) -> Vec<Addr> {
    let mut addrs = vec![];
    for part in list.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some(idx) = part.find(':') {
            let client = part[..idx].trim().parse::<i32>();
            let port = part[idx + 1..].trim().split(':').next().unwrap_or("").split_whitespace().next().unwrap_or("").parse::<i32>();
            if let (Ok(client), Ok(port)) = (client, port) {
                addrs.push(Addr { client, port });
            }
        }
    }
    addrs
}

/// Enumerate alsa sequencer ports that accept midi (write-capable and
/// subscribable): hardware synths' input ports and software receivers.
/// The strings start with the "client:port" address followed by a
/// description, so the track routing combo can connect by address.
pub fn alsa_midi_output_devices() -> Vec<String> {
    let mut devices = vec![];
    let seq = match Seq::open(None, None, true) {
        Ok(seq) => seq,
        Err(e) => {
            warn!("ALSA midi: could not open the sequencer to enumerate devices: {}", e);
            return devices;
        }
    };
    let our_client = seq.client_id().unwrap_or(-1);
    for client in ClientIter::new(&seq) {
        let client_id = client.get_client();
        if client_id == our_client || client_id == SND_SEQ_CLIENT_SYSTEM {
            continue;
        }
        let client_name = client.get_name().unwrap_or("?").to_string();
        for port in alsa::seq::PortIter::new(&seq, client_id) {
            let caps = port.get_capability();
            if caps.contains(PortCap::WRITE) && caps.contains(PortCap::SUBS_WRITE) {
                let port_name = port.get_name().unwrap_or("?").to_string();
                devices.push(format!("{}:{} {}:{}", port.get_client(), port.get_port(), client_name, port_name));
            }
        }
    }
    devices
}

/// Commands executed by the alsa midi output frontend on the engine thread
/// (the sequencer handle is not shareable, so per-track port creation and
/// device subscriptions are queued to the thread that owns it).
pub enum AlsaMidiCommand {
    CreateTrackPort { track_uuid: String, reply: Sender<Option<i32>> },
    RemoveTrackPort { track_uuid: String },
    SubscribeTrack { track_uuid: String, dest: Addr },
    UnsubscribeTrack { track_uuid: String, dest: Addr },
}

/// The alsa sequencer midi output frontend, owned by whichever thread runs
/// the engine cycle (the alsa audio loop or the jack callback when only the
/// midi backend is alsa).
pub struct AlsaMidiOut {
    pub seq: Seq,
    pub client: i32,
    queue: i32,
    commands_rx: Receiver<AlsaMidiCommand>,
    encoder: SendSeqMidiEvent,
    track_ports: HashMap<String, i32>,
}

impl AlsaMidiOut {
    pub fn new(commands_rx: Receiver<AlsaMidiCommand>) -> Result<Self, String> {
        let seq = Seq::open(None, None, false).map_err(|e| format!("{}", e))?;
        seq.set_client_name(c"riff-daw").map_err(|e| format!("{}", e))?;
        let client = seq.client_id().map_err(|e| format!("{}", e))?;
        let queue = seq.alloc_named_queue(c"riff-daw").map_err(|e| format!("{}", e))?;
        let tempo = QueueTempo::empty().map_err(|e| format!("{}", e))?;
        tempo.set_ppq(96);
        tempo.set_tempo(500_000);
        seq.set_queue_tempo(queue, &tempo).map_err(|e| format!("{}", e))?;
        // the queue clock is what the relative real-time event stamps are
        // measured against; it must be running for scheduled events to go out
        seq.control_queue(queue, EventType::Start, 0, None).map_err(|e| format!("{}", e))?;
        let encoder = SendSeqMidiEvent(SeqMidiEvent::new(64).map_err(|e| format!("{}", e))?);
        Ok(AlsaMidiOut {
            seq,
            client,
            queue,
            commands_rx,
            encoder,
            track_ports: HashMap::new(),
        })
    }

    /// Create the seq ports for the tracks carried by salvaged midi consumers
    /// (a backend restart/rebuild keeps the track threads' rings but the old
    /// ports died with the old layer) and re-apply the saved device
    /// subscriptions.
    pub fn setup_track_ports(&mut self,
                             consumers: &mut Vec<MidiConsumerDetails<(u32, u8, u8, u8, bool)>>,
                             subscriptions: &HashMap<String, Addr>) {
        for consumer in consumers.iter_mut() {
            let track_uuid = consumer.track_uuid().clone();
            match self.create_track_port(&track_uuid) {
                Some(port) => {
                    consumer.set_midi_out_port(Some(MidiOutPort::Alsa(Addr { client: self.client, port })));
                    if let Some(dest) = subscriptions.get(&track_uuid) {
                        self.subscribe(&track_uuid, *dest);
                    }
                }
                None => {
                    warn!("ALSA midi: could not create the output port for track={}", track_uuid.as_str());
                    consumer.set_midi_out_port(None);
                }
            }
        }
    }

    fn create_track_port(&mut self, track_uuid: &str) -> Option<i32> {
        if let Some(port) = self.track_ports.get(track_uuid) {
            return Some(*port);
        }
        let name = std::ffi::CString::new(track_uuid.to_string()).ok()?;
        // the track port *emits* midi: it is readable and subscribable for
        // reading by the destination ports
        match self.seq.create_simple_port(&name,
                                          PortCap::READ | PortCap::SUBS_READ,
                                          PortType::MIDI_GENERIC | PortType::SOFTWARE | PortType::APPLICATION) {
            Ok(port) => {
                let port = port as i32;
                self.track_ports.insert(track_uuid.to_string(), port);
                info!("ALSA midi: created track output port {}:{}", self.client, port);
                Some(port)
            }
            Err(e) => {
                warn!("ALSA midi: creating the track output port failed: {}", e);
                None
            }
        }
    }

    fn subscribe(&mut self, track_uuid: &str, dest: Addr) {
        let port = match self.track_ports.get(track_uuid) {
            Some(port) => *port,
            None => {
                warn!("ALSA midi: subscribing track {} but it has no output port yet", track_uuid);
                return;
            }
        };
        let ps = match PortSubscribe::empty() {
            Ok(ps) => ps,
            Err(_) => return,
        };
        ps.set_sender(Addr { client: self.client, port });
        ps.set_dest(dest);
        ps.set_time_real(true);
        match self.seq.subscribe_port(&ps) {
            Ok(_) => info!("ALSA midi: subscribed track {} to {}:{}", track_uuid, dest.client, dest.port),
            Err(e) => warn!("ALSA midi: subscribing track {} to {}:{} failed: {}", track_uuid, dest.client, dest.port, e),
        }
    }

    fn unsubscribe(&mut self, track_uuid: &str, dest: Addr) {
        if let Some(port) = self.track_ports.get(track_uuid) {
            let _ = self.seq.unsubscribe_port(Addr { client: self.client, port: *port }, dest);
        }
    }

    /// Run the queued control commands (called on the engine thread once per
    /// cycle - they are control path events, rare and cheap).
    pub fn service_commands(&mut self) {
        let mut count = 0;
        while count < 64 {
            let command = match self.commands_rx.try_recv() {
                Ok(command) => command,
                Err(_) => break,
            };
            count += 1;
            match command {
                AlsaMidiCommand::CreateTrackPort { track_uuid, reply } => {
                    let port = self.create_track_port(&track_uuid);
                    let _ = reply.send(port);
                }
                AlsaMidiCommand::RemoveTrackPort { track_uuid } => {
                    if let Some(port) = self.track_ports.remove(&track_uuid) {
                        let _ = self.seq.delete_port(port);
                    }
                }
                AlsaMidiCommand::SubscribeTrack { track_uuid, dest } => self.subscribe(&track_uuid, dest),
                AlsaMidiCommand::UnsubscribeTrack { track_uuid, dest } => self.unsubscribe(&track_uuid, dest),
            }
        }
    }

    /// Queue one raw midi message to a track port, scheduled `frame` frames
    /// into the cycle on the queue's real-time clock (the relative stamp
    /// preserves the in-block note timing as closely as the sequencer's
    /// millisecond clock allows).
    pub fn schedule_midi(&mut self, dest: &Addr, frame: u32, byte1: u8, byte2: u8, byte3: u8, sample_rate: f64) {
        let status = byte1 & 0xF0;
        let message: [u8; 3] = match status {
            0xC0 | 0xD0 => [byte1, byte2, 0],   // program change / channel pressure: two data bytes
            0xF0..=0xFF => return,              // system real-time/sysex messages are not routed
            _ => [byte1, byte2, byte3],
        };

        let delay_micros = if sample_rate > 0.0 { (frame as f64 * 1_000_000.0 / sample_rate) as u64 } else { 0 };
        let delay = std::time::Duration::from_micros(delay_micros);

        // split the borrows: the encoder is only touched to build the event,
        // the sequencer output happens on the other fields
        let client = self.client;
        let queue = self.queue;
        let seq = &self.seq;
        let encoder = &mut self.encoder.0;

        let mut remaining: &[u8] = &message;
        while !remaining.is_empty() {
            match encoder.encode(remaining) {
                Ok((consumed, Some(mut event))) => {
                    event.set_dest(Addr { client, port: dest.port });
                    event.schedule_real(queue, true, delay);
                    let _ = seq.event_output(&mut event);
                    if consumed == 0 {
                        break;
                    }
                    remaining = &remaining[consumed..];
                }
                _ => break,
            }
        }
    }
}

/// The alsa sequencer input frontend: the "riff-daw-midi-in" client with the
/// live keys (`midi_in`) and control surface (`midi_control_in`) ports,
/// subscribed to the configured device ports. Runs on its own thread so the
/// engine never blocks on the sequencer.
pub struct AlsaMidiInput {
    stop: Arc<AtomicBool>,
    joiner: Option<std::thread::JoinHandle<()>>,
    /// the frontends keep the channel senders so the channels stay open for
    /// the thread even if a restart momentarily leaves them unattached
    #[allow(dead_code)]
    pub in_tx: Sender<AlsaMidiInEvent>,
    #[allow(dead_code)]
    pub control_tx: Sender<AlsaMidiInEvent>,
    pub in_rx: Receiver<AlsaMidiInEvent>,
    pub control_rx: Receiver<AlsaMidiInEvent>,
}

impl AlsaMidiInput {
    pub fn start(midi_in_devices: Vec<Addr>, midi_control_in_devices: Vec<Addr>) -> Option<AlsaMidiInput> {
        let seq = match Seq::open(None, None, false) {
            Ok(seq) => seq,
            Err(e) => {
                warn!("ALSA midi in: could not open the sequencer: {}", e);
                return None;
            }
        };
        if let Err(e) = seq.set_client_name(c"riff-daw-midi-in") {
            warn!("ALSA midi in: could not name the client: {}", e);
        }
        // the DAW's input ports accept (are written into by) subscribed senders
        let in_port = match seq.create_simple_port(c"midi_in",
                                                 PortCap::WRITE | PortCap::SUBS_WRITE,
                                                 PortType::MIDI_GENERIC | PortType::SOFTWARE | PortType::APPLICATION) {
            Ok(port) => port as i32,
            Err(e) => {
                warn!("ALSA midi in: could not create the midi_in port: {}", e);
                return None;
            }
        };
        let control_port = match seq.create_simple_port(c"midi_control_in",
                                                      PortCap::WRITE | PortCap::SUBS_WRITE,
                                                      PortType::MIDI_GENERIC | PortType::SOFTWARE | PortType::APPLICATION) {
            Ok(port) => port as i32,
            Err(e) => {
                warn!("ALSA midi in: could not create the midi_control_in port: {}", e);
                return None;
            }
        };
        let client = seq.client_id().unwrap_or(-1);
        let _ = seq.set_client_event_filter(SND_SEQ_EVENT_FILTER_SOFT | SND_SEQ_EVENT_FILTER_HARD);
        let subscribe = |devices: &Vec<Addr>, port: i32| {
            for device in devices.iter() {
                let ps = match PortSubscribe::empty() {
                    Ok(ps) => ps,
                    Err(_) => continue,
                };
                ps.set_sender(*device);
                ps.set_dest(Addr { client, port });
                ps.set_time_real(true);
                match seq.subscribe_port(&ps) {
                    Ok(_) => info!("ALSA midi in: subscribed {}:{} to the midi in port {}:{}", device.client, device.port, client, port),
                    Err(e) => warn!("ALSA midi in: subscribing {}:{} failed: {}", device.client, device.port, e),
                }
            }
        };
        subscribe(&midi_in_devices, in_port);
        subscribe(&midi_control_in_devices, control_port);

        let decoder = match SeqMidiEvent::new(64) {
            Ok(decoder) => decoder,
            Err(e) => {
                warn!("ALSA midi in: could not create the event decoder: {}", e);
                return None;
            }
        };

        let (in_tx, in_rx) = crossbeam_channel::bounded::<AlsaMidiInEvent>(EVENT_BUFFER_SIZE * 4);
        let (control_tx, control_rx) = crossbeam_channel::bounded::<AlsaMidiInEvent>(EVENT_BUFFER_SIZE * 4);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_thread = stop.clone();
        let decoder = SendSeqMidiEvent(decoder);
        let joiner = std::thread::Builder::new()
            .name("AlsaMidiInput".to_string())
            .spawn({
                let in_tx = in_tx.clone();
                let control_tx = control_tx.clone();
                move || {
                    alsa_midi_input_loop(seq, decoder, in_port, control_port, in_tx, control_tx, stop_for_thread);
                }
            });
        let joiner = match joiner {
            Ok(joiner) => joiner,
            Err(_) => return None,
        };
        Some(AlsaMidiInput { stop, joiner: Some(joiner), in_tx, control_tx, in_rx, control_rx })
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(joiner) = self.joiner.take() {
            let _ = joiner.join();
        }
    }
}

impl Drop for AlsaMidiInput {
    fn drop(&mut self) {
        self.stop();
    }
}

fn alsa_midi_input_loop(seq: Seq, decoder: SendSeqMidiEvent, in_port: i32, control_port: i32,
                        in_tx: Sender<AlsaMidiInEvent>, control_tx: Sender<AlsaMidiInEvent>,
                        stop: Arc<AtomicBool>) {
    crate::domain::flush_denormals_to_zero();
    let mut input = seq.input();
    loop {
        if stop.load(Ordering::Relaxed) || !crate::UI_RUNNING.load(Ordering::Relaxed) {
            break;
        }
        let pending = match input.event_input_pending(false) {
            Ok(pending) => pending,
            Err(_) => 0,
        };
        if pending == 0 {
            std::thread::sleep(std::time::Duration::from_millis(2));
            continue;
        }
        let mut count = 0;
        while count < 256 {
            let mut event = match input.event_input() {
                Ok(event) => event,
                Err(_) => break,
            };
            count += 1;
            let dest = event.get_dest();
            let is_control = dest.port == control_port;
            let is_live = dest.port == in_port;
            if !is_control && !is_live {
                continue;
            }
            let mut buffer = [0u8; MIDI_IN_MAX_BYTES];
            match decoder.0.decode(&mut buffer, &mut event) {
                Ok(len) if len > 0 => {
                    let alsa_event = AlsaMidiInEvent { bytes: buffer, len, when: std::time::Instant::now() };
                    let _ = if is_control { control_tx.try_send(alsa_event) } else { in_tx.try_send(alsa_event) };
                }
                _ => (),
            }
        }
    }
}

/// The alsa audio layer handle: the engine loop thread driving the playback
/// device. Stopping it hands the engine (and the tracks' consumers inside
/// it) back for the replacement layer.
pub struct AlsaAudioLayer {
    stop: Arc<AtomicBool>,
    joiner: Option<std::thread::JoinHandle<AudioEngine>>,
}

impl AlsaAudioLayer {
    pub fn start(engine: AudioEngine, pcm: PCM, period_frames: usize, midi_out: Option<AlsaMidiOut>,
                 midi_in_rx: Receiver<AlsaMidiInEvent>, midi_control_in_rx: Receiver<AlsaMidiInEvent>) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_thread = stop.clone();
        let joiner = std::thread::Builder::new()
            .name("AlsaAudioLoop".to_string())
            .spawn(move || {
                alsa_audio_loop(engine, pcm, period_frames, midi_out, midi_in_rx, midi_control_in_rx, stop_for_thread)
            })
            .map_err(|e| e.to_string())?;
        Ok(AlsaAudioLayer { stop, joiner: Some(joiner) })
    }

    /// Stop the loop and take back the engine (its consumers are moved into
    /// the replacement layer by the restart path).
    pub fn stop_and_take_engine(&mut self) -> Option<AudioEngine> {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(joiner) = self.joiner.take() {
            match joiner.join() {
                Ok(engine) => Some(engine),
                Err(_) => {
                    warn!("ALSA: the audio loop thread panicked - its track consumers cannot be salvaged.");
                    None
                }
            }
        }
        else {
            None
        }
    }
}

impl Drop for AlsaAudioLayer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // the loop exits on its own next cycle; at process exit nobody waits
        // for the join
    }
}

fn convert_f32(left: &[f32], right: &[f32], out: &mut Vec<f32>) {
    for frame in 0..left.len() {
        out.push(left[frame]);
        out.push(right[frame]);
    }
}

fn convert_i32(left: &[f32], right: &[f32], out: &mut Vec<i32>) {
    for frame in 0..left.len() {
        out.push((left[frame] * 2_147_483_647.0) as i32);
        out.push((right[frame] * 2_147_483_647.0) as i32);
    }
}

fn convert_i16(left: &[f32], right: &[f32], out: &mut Vec<i16>) {
    for frame in 0..left.len() {
        out.push((left[frame] * 32_767.0) as i16);
        out.push((right[frame] * 32_767.0) as i16);
    }
}

fn alsa_audio_loop(engine: AudioEngine, pcm: PCM, period_frames: usize,
                   midi_out: Option<AlsaMidiOut>,
                   midi_in_rx: Receiver<AlsaMidiInEvent>,
                   midi_control_in_rx: Receiver<AlsaMidiInEvent>,
                   stop: Arc<AtomicBool>) -> AudioEngine {
    crate::domain::flush_denormals_to_zero();

    // match the real-time scheduling a jack callback thread would get (the
    // track threads keep themselves below whatever priority lands here)
    match thread_priority::set_current_thread_priority(thread_priority::ThreadPriority::Crossplatform(95.try_into().unwrap())) {
        Ok(_) => debug!("ALSA: audio loop thread running with priority 95"),
        Err(_) => debug!("ALSA: audio loop thread could not raise its priority - running at the inherited priority"),
    }

    let format = match pcm.hw_params_current().and_then(|h| h.get_format()) {
        Ok(format) => format,
        Err(e) => {
            warn!("ALSA: could not read back the negotiated sample format: {} - stopping the layer.", e);
            return engine;
        }
    };

    if format == Format::float() {
        match pcm.io_f32() {
            Ok(io) => alsa_audio_loop_with(engine, &pcm, &io, period_frames, midi_out, midi_in_rx, midi_control_in_rx, stop, convert_f32),
            Err(e) => { warn!("ALSA: the device rejected the float io: {}", e); engine }
        }
    }
    else if format == Format::s32() {
        match pcm.io_i32() {
            Ok(io) => alsa_audio_loop_with(engine, &pcm, &io, period_frames, midi_out, midi_in_rx, midi_control_in_rx, stop, convert_i32),
            Err(e) => { warn!("ALSA: the device rejected the s32 io: {}", e); engine }
        }
    }
    else {
        match pcm.io_i16() {
            Ok(io) => alsa_audio_loop_with(engine, &pcm, &io, period_frames, midi_out, midi_in_rx, midi_control_in_rx, stop, convert_i16),
            Err(e) => { warn!("ALSA: the device rejected the s16 io: {}", e); engine }
        }
    }
}

fn alsa_audio_loop_with<'a, S: Copy>(mut engine: AudioEngine, pcm: &PCM, io: &IO<'a, S>, period_frames: usize,
                                     mut midi_out: Option<AlsaMidiOut>,
                                     midi_in_rx: Receiver<AlsaMidiInEvent>,
                                     midi_control_in_rx: Receiver<AlsaMidiInEvent>,
                                     stop: Arc<AtomicBool>,
                                     convert: fn(&[f32], &[f32], &mut Vec<S>)) -> AudioEngine {
    let mut buffers = MasterBuffers::new(period_frames);
    let mut interleaved: Vec<S> = Vec::with_capacity(period_frames * 2);
    let mut midi_in_scratch = [MIDI_IN_EVENT_EMPTY; EVENT_BUFFER_SIZE];
    let mut midi_control_in_scratch = [MIDI_IN_EVENT_EMPTY; EVENT_BUFFER_SIZE];
    let mut fatal = false;

    while !fatal {
        if stop.load(Ordering::Relaxed) || !crate::UI_RUNNING.load(Ordering::Relaxed) {
            break;
        }

        let mut midi_in_count = 0;
        while midi_in_count < midi_in_scratch.len() {
            match midi_in_rx.try_recv() {
                Ok(event) => {
                    midi_in_scratch[midi_in_count] = MidiInEvent::from_bytes(&event);
                    midi_in_count += 1;
                }
                Err(_) => break,
            }
        }
        let mut midi_control_in_count = 0;
        while midi_control_in_count < midi_control_in_scratch.len() {
            match midi_control_in_rx.try_recv() {
                Ok(event) => {
                    midi_control_in_scratch[midi_control_in_count] = MidiInEvent::from_bytes(&event);
                    midi_control_in_count += 1;
                }
                Err(_) => break,
            }
        }

        if let Some(midi_out) = midi_out.as_mut() {
            midi_out.service_commands();
        }
        let mut sink = match midi_out.as_mut() {
            Some(midi_out) => MidiOutSink::Alsa(midi_out),
            None => MidiOutSink::None,
        };
        let keep_alive = engine.process_cycle(&mut buffers,
                                              &midi_in_scratch[..midi_in_count],
                                              &midi_control_in_scratch[..midi_control_in_count],
                                              &mut sink,
                                              period_frames);
        if !keep_alive {
            break;
        }

        // interleave and write (the blocking write paces the loop against
        // the hardware clock)
        let frames = period_frames.min(buffers.left.len());
        interleaved.clear();
        convert(&buffers.left[..frames], &buffers.right[..frames], &mut interleaved);
        match io.writei(&interleaved) {
            Ok(_) => (),
            Err(e) => {
                if e.errno() == EPIPE {
                    // underrun: the card started replaying stale frames -
                    // recover (prepare) in place and keep the cycle running,
                    // counted like a jack xrun
                    engine.count_xrun();
                    warn!("ALSA: output underrun - recovering");
                    let _ = pcm.drop();
                    let _ = pcm.prepare();
                }
                else if e.errno() != EINTR {
                    error!("ALSA: the playback device failed: {} - stopping the audio loop.", e);
                    fatal = true;
                }
            }
        }
    }

    // silence the device on the way out (a half-filled buffer would keep
    // playing the last submitted block after the layer stops)
    let _ = pcm.drop();
    engine
}
