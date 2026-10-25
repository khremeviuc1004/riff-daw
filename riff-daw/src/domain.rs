use std::{collections::HashMap, sync::{Arc, mpsc::{channel, Receiver, Sender}, Mutex}, time::Duration};
use std::collections::hash_map::Keys;
use std::collections::VecDeque;
use std::default::Default;
use std::io::prelude::*;
use std::ops::Index;
use std::thread;

use clap_sys::{ext::{gui::{clap_window, CLAP_WINDOW_API_X11, clap_window_handle}}, process::clap_process};
use jack::{MidiOut, Port};
use log::*;
use mlua::prelude::LuaUserData;
use parking_lot::RwLock;
use rb::{Consumer, Producer, RB, RbConsumer, RbProducer, SpscRb};
use samplerate_rs::{convert, ConverterType};
use serde::{Deserialize, Serialize};
use simple_clap_host_helper_lib::{host::DAWCallback, plugin::{instance::process::ProcessData, library::PluginLibrary}};
use sndfile::*;
use state::InitCell;
use strum_macros::{Display, EnumString};
use thread_priority::*;
use uuid::Uuid;
use widestring::U16String;
use simple_clap_host_helper_lib::plugin::ext::params::ParamInfo;
use simple_clap_host_helper_lib::plugin::instance::process::Event::{ParamGestureBegin, ParamGestureEnd, ParamValue};
use vst::{api::{TimeInfo, TimeInfoFlags}, buffer::{AudioBuffer, SendEventBuffer}, editor::Editor, event::MidiEvent, host::{Host, HostBuffer, PluginInstance, PluginLoader}, plugin::{HostCanDo, Plugin}};

use crate::{audio_plugin_util::*, constants::CONFIGURATION_FILE_NAME, DAWUtils, event::{AudioLayerInwardEvent, AudioPluginHostOutwardEvent, ClapCallReply, ClapInstanceParts, ClapPluginCall, SendParamInfo, TrackBackgroundProcessorInwardEvent, TrackBackgroundProcessorOutwardEvent, TrackPluginPresetResponse}, GeneralTrackType};
use crate::constants::{BLOCK_SIZE_MAX, EVENT_BUFFER_SIZE};
use crate::event::EventProcessorType;
use crate::state::MidiPolyphonicExpressionNoteId;
use crate::vst3_cxx_bridge::{ffi, Vst3Host};
use crate::vst3_cxx_bridge::ffi::{showPluginEditor, vst3_plugin_get_window_width};

unsafe extern "C" {
    fn gdk_x11_window_get_xid(window: gdk4::Surface) -> u32;
}
pub static TRANSPORT: InitCell<RwLock<Transport>> = InitCell::new();

/// Locks VST2 plugin lifecycle work (instantiation, teardown, preset chunk
/// loading, sample rate/block size changes, editor window creation - all of
/// which happen inside `handle_incoming_events` on a track's background
/// thread) against every thread's per-block plugin dispatch calls.
///
/// Some VST2 frameworks (u-he's shared engine behind Hive/DiscoveryPro etc)
/// re-initialise process-wide state when an entry point is called or a patch
/// is loaded, so a lifecycle call overlapping audio processing of another
/// instance corrupts the framework's internal buffers and segfaults in
/// `processReplacing` (seen just after loading a file, when every track
/// thread is creating plugins and processing at once). Lifecycle takes the
/// write lock, per-block plugin work takes the read lock, so instances of
/// different libraries can still process in parallel with each other.
pub static VST24_PLUGIN_LIFECYCLE_LOCK: std::sync::LazyLock<RwLock<()>> = std::sync::LazyLock::new(|| RwLock::new(()));

pub struct Transport {
    pub playing: bool,
    pub bpm: f64,
    pub sample_rate: f64,
    pub block_size: f64,
    pub position_in_beats: f64,
    pub position_in_frames: u32,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PlayMode {
    Song,
    RiffSet,
    RiffSequence,
    RiffGrid,
    RiffArrangement,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub enum TrackEvent {
    ActiveSense(ActiveSense),
    AfterTouch(AfterTouch),
    ProgramChange(ProgramChange),
    Note(Note),
    NoteOn(NoteOn),
    NoteOff(NoteOff),
    NoteExpression(NoteExpression),
    Controller(Controller),
    PitchBend(PitchBend),
    KeyPressure(KeyPressure),
    AudioPluginParameter(PluginParameter),
    Sample(SampleReference),
    Measure(Measure),
    Tempo(Tempo),
}

// the active sense variant is the default - it carries no musical data and is used
// as the sentinel value in the event routing ring buffers.
impl Default for TrackEvent {
    fn default() -> Self {
        TrackEvent::ActiveSense(ActiveSense::default())
    }
}

impl DAWItemID for TrackEvent {
    fn id(&self) -> String {
        match self {
            TrackEvent::ActiveSense(active_sense) => active_sense.id(),
            TrackEvent::AfterTouch(after_touch) => after_touch.id(),
            TrackEvent::ProgramChange(program_change) => program_change.id(),
            TrackEvent::Note(note) => note.id(),
            TrackEvent::NoteOn(_) => Uuid::nil().to_string(),
            TrackEvent::NoteOff(_) => Uuid::nil().to_string(),
            TrackEvent::Controller(controller) => controller.id(),
            TrackEvent::PitchBend(pitch_bend) => pitch_bend.id(),
            TrackEvent::KeyPressure(key_pressure) => key_pressure.id(),
            TrackEvent::AudioPluginParameter(parameter) => parameter.id(),
            TrackEvent::Sample(_) => Uuid::nil().to_string(),
            TrackEvent::Measure(_) => Uuid::nil().to_string(),
            TrackEvent::Tempo(tempo) => tempo.id(),
            TrackEvent::NoteExpression(note_expression) => note_expression.id(),
        }
    }

    fn id_mut(&mut self) -> String {
        match self {
            TrackEvent::ActiveSense(active_sense) => active_sense.id(),
            TrackEvent::AfterTouch(after_touch) => after_touch.id(),
            TrackEvent::ProgramChange(program_change) => program_change.id(),
            TrackEvent::Note(note) => note.id(),
            TrackEvent::NoteOn(_) => Uuid::nil().to_string(),
            TrackEvent::NoteOff(_) => Uuid::nil().to_string(),
            TrackEvent::Controller(controller) => controller.id(),
            TrackEvent::PitchBend(pitch_bend) => pitch_bend.id(),
            TrackEvent::KeyPressure(key_pressure) => key_pressure.id(),
            TrackEvent::AudioPluginParameter(parameter) => parameter.id(),
            TrackEvent::Sample(_) => Uuid::nil().to_string(),
            TrackEvent::Measure(_) => Uuid::nil().to_string(),
            TrackEvent::Tempo(tempo) => tempo.id(),
            TrackEvent::NoteExpression(note_expression) => note_expression.id(),
        }
    }

    fn set_id(&mut self, uuid: String) {
        match self {
            TrackEvent::ActiveSense(active_sense) => active_sense.set_id(uuid),
            TrackEvent::AfterTouch(after_touch) => after_touch.set_id(uuid),
            TrackEvent::ProgramChange(program_change) => program_change.set_id(uuid),
            TrackEvent::Tempo(tempo) => tempo.set_id(uuid),
            TrackEvent::Note(note) => note.set_id(uuid),
            TrackEvent::Controller(controller) => controller.set_id(uuid),
            TrackEvent::PitchBend(pitch_bend) => pitch_bend.set_id(uuid),
            TrackEvent::KeyPressure(key_pressure) => key_pressure.set_id(uuid),
            TrackEvent::AudioPluginParameter(parameter) => parameter.set_id(uuid),
            TrackEvent::NoteExpression(note_expression) => note_expression.set_id(uuid),
            _ => {}
        }
    }
}

impl DAWItemPosition for TrackEvent {
    fn position(&self) -> f64 {
        match self {
            TrackEvent::ActiveSense(active_sense) => active_sense.position(),
            TrackEvent::AfterTouch(after_touch) => after_touch.position(),
            TrackEvent::ProgramChange(program_change) => program_change.position(),
            TrackEvent::Note(note) => note.position(),
            TrackEvent::NoteOn(note_on) => note_on.position(),
            TrackEvent::NoteOff(note_off) => note_off.position(),
            TrackEvent::Controller(controller) => controller.position(),
            TrackEvent::PitchBend(pitch_bend) => pitch_bend.position(),
            TrackEvent::KeyPressure(key_pressure) => key_pressure.position(),
            TrackEvent::AudioPluginParameter(parameter) => parameter.position(),
            TrackEvent::Sample(sample_reference) => sample_reference.position(),
            TrackEvent::Measure(measure) => measure.position(),
            TrackEvent::Tempo(tempo) => tempo.position(),
            TrackEvent::NoteExpression(note_expression) => note_expression.position(),
        }
    }

    fn set_position(&mut self, time: f64) {
        match self {
            TrackEvent::ActiveSense(active_sense) => active_sense.set_position(time),
            TrackEvent::AfterTouch(after_touch) => after_touch.set_position(time),
            TrackEvent::ProgramChange(program_change) => program_change.set_position(time),
            TrackEvent::Note(note) => note.set_position(time),
            TrackEvent::NoteOn(note_on) => note_on.set_position(time),
            TrackEvent::NoteOff(note_off) => note_off.set_position(time),
            TrackEvent::Controller(controller) => controller.set_position(time),
            TrackEvent::PitchBend(_pitch_bend) => {}
            TrackEvent::KeyPressure(key_pressure) => key_pressure.set_position(time),
            TrackEvent::AudioPluginParameter(parameter) => parameter.set_position(time),
            TrackEvent::Sample(sample_reference) => sample_reference.set_position(time),
            TrackEvent::Measure(measure) => measure.set_position(time),
            TrackEvent::Tempo(tempo) => tempo.set_position(time),
            TrackEvent::NoteExpression(note_expression) => note_expression.set_position(time),
        }
    }
}

impl DAWItemLength for TrackEvent {
    fn length(&self) -> f64 {
        match self {
            TrackEvent::Note(note) => note.length(),
            _ => 0.0,
        }
    }

    fn set_length(&mut self, length: f64) {
        match self {
            TrackEvent::Note(note) => note.set_length(length),
            _ => {}
        }
    }
}

impl TrackEvent {
    pub fn value(&self) -> f64 {
        match self {
            TrackEvent::Note(event) => event.note() as f64,
            TrackEvent::NoteOn(event) => event.note() as f64,
            TrackEvent::NoteOff(event) => event.note() as f64,
            TrackEvent::NoteExpression(event) => event.value(),
            TrackEvent::Controller(event) => event.value() as f64,
            TrackEvent::PitchBend(event) => event.value() as f64,
            TrackEvent::AfterTouch(event) => event.value() as f64,
            TrackEvent::KeyPressure(event) => event.value() as f64,
            TrackEvent::ProgramChange(event) => event.program() as f64,
            TrackEvent::Tempo(event) => event.bpm(),
            TrackEvent::AudioPluginParameter(event) => event.value() as f64,
            _ => 0.0,
        }
    }

    pub fn set_value(&mut self, value: f64) {
        match self {
            TrackEvent::Note(event) => event.set_note(value as i32),
            TrackEvent::NoteOn(event) => event.set_note(value as i32),
            TrackEvent::NoteOff(event) => event.set_note(value as i32),
            TrackEvent::NoteExpression(event) => event.set_value(value),
            TrackEvent::Controller(event) => event.set_value(value as i32),
            TrackEvent::PitchBend(event) => event.set_value(value as i32),
            TrackEvent::AfterTouch(event) => event.set_value(value as i32),
            TrackEvent::KeyPressure(event) => event.set_value(value as i32),
            TrackEvent::ProgramChange(event) => event.set_program(value as i32),
            TrackEvent::Tempo(event) => event.set_bpm(value),
            TrackEvent::AudioPluginParameter(event) => event.set_value(value as f32),
            _ => (),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub enum TrackType {
    InstrumentTrack(InstrumentTrack),
    AudioTrack(AudioTrack),
    MidiTrack(MidiTrack),
}

impl Track for TrackType {
    fn name(&self) -> &str {
        match self {
            TrackType::InstrumentTrack(track) => track.name(),
            TrackType::AudioTrack(track) => track.name(),
            TrackType::MidiTrack(track) => track.name(),
        }
    }

    fn name_mut(&mut self) -> &str {
        match self {
            TrackType::InstrumentTrack(track) => track.name_mut(),
            TrackType::AudioTrack(track) => track.name_mut(),
            TrackType::MidiTrack(track) => track.name_mut(),
        }
    }

    fn set_name(&mut self, name: String) {
        match self {
            TrackType::InstrumentTrack(track) => track.set_name(name),
            TrackType::AudioTrack(track) => track.set_name(name),
            TrackType::MidiTrack(track) => track.set_name(name),
        }
    }
    fn mute(&self) -> bool {
        match self {
            TrackType::InstrumentTrack(track) => track.mute(),
            TrackType::AudioTrack(track) => track.mute(),
            TrackType::MidiTrack(track) => track.mute(),
        }
    }
    fn set_mute(&mut self, mute: bool) {
        match self {
            TrackType::InstrumentTrack(track) => track.set_mute(mute),
            TrackType::AudioTrack(track) => track.set_mute(mute),
            TrackType::MidiTrack(track) => track.set_mute(mute),
        }
    }
    fn solo(&self) -> bool {
        match self {
            TrackType::InstrumentTrack(track) => track.solo(),
            TrackType::AudioTrack(track) => track.solo(),
            TrackType::MidiTrack(track) => track.solo(),
        }
    }
    fn set_solo(&mut self, solo: bool) {
        match self {
            TrackType::InstrumentTrack(track) => track.set_solo(solo),
            TrackType::AudioTrack(track) => track.set_solo(solo),
            TrackType::MidiTrack(track) => track.set_solo(solo),
        }
    }
    fn colour(&self) -> (f64, f64, f64, f64) {
        match self {
            TrackType::InstrumentTrack(track) => track.colour(),
            TrackType::AudioTrack(track) => track.colour(),
            TrackType::MidiTrack(track) => track.colour(),
        }
    }
    fn colour_mut(&mut self) -> (f64, f64, f64, f64) {
        match self {
            TrackType::InstrumentTrack(track) => track.colour_mut(),
            TrackType::AudioTrack(track) => track.colour_mut(),
            TrackType::MidiTrack(track) => track.colour_mut(),
        }
    }
    fn set_colour(&mut self, red: f64, green: f64, blue: f64, alpha: f64) {
        match self {
            TrackType::InstrumentTrack(track) => track.set_colour(red, green, blue, alpha),
            TrackType::AudioTrack(track) => track.set_colour(red, green, blue, alpha),
            TrackType::MidiTrack(track) => track.set_colour(red, green, blue, alpha),
        }
    }
    fn riffs_mut(&mut self) -> &mut Vec<Riff> {
        match self {
            TrackType::InstrumentTrack(track) => track.riffs_mut(),
            TrackType::AudioTrack(track) => track.riffs_mut(),
            TrackType::MidiTrack(track) => track.riffs_mut(),
        }
    }
    fn riff_refs_mut(&mut self) -> &mut Vec<RiffReference> {
        match self {
            TrackType::InstrumentTrack(track) => track.riff_refs_mut(),
            TrackType::AudioTrack(track) => track.riff_refs_mut(),
            TrackType::MidiTrack(track) => track.riff_refs_mut(),
        }
    }
    fn riffs(&self) -> &Vec<Riff> {
        match self {
            TrackType::InstrumentTrack(track) => track.riffs(),
            TrackType::AudioTrack(track) => track.riffs(),
            TrackType::MidiTrack(track) => track.riffs(),
        }
    }
    fn riff_refs(&self) -> &Vec<RiffReference> {
        match self {
            TrackType::InstrumentTrack(track) => track.riff_refs(),
            TrackType::AudioTrack(track) => track.riff_refs(),
            TrackType::MidiTrack(track) => track.riff_refs(),
        }
    }
    fn automation_mut(&mut self) -> &mut Automation {
        match self {
            TrackType::InstrumentTrack(track) => track.automation_mut(),
            TrackType::AudioTrack(track) => track.automation_mut(),
            TrackType::MidiTrack(track) => track.automation_mut(),
        }
    }
    fn automation(&self) -> &Automation {
        match self {
            TrackType::InstrumentTrack(track) => track.automation(),
            TrackType::AudioTrack(track) => track.automation(),
            TrackType::MidiTrack(track) => track.automation(),
        }
    }
    fn uuid(&self) -> Uuid {
        match self {
            TrackType::InstrumentTrack(track) => track.uuid(),
            TrackType::AudioTrack(track) => track.uuid(),
            TrackType::MidiTrack(track) => track.uuid(),
        }
    }
    fn uuid_mut(&mut self) -> &mut Uuid {
        match self {
            TrackType::InstrumentTrack(track) => track.uuid_mut(),
            TrackType::AudioTrack(track) => track.uuid_mut(),
            TrackType::MidiTrack(track) => track.uuid_mut(),
        }
    }
    fn uuid_string(&mut self) -> String {
        match self {
            TrackType::InstrumentTrack(track) => track.uuid_string(),
            TrackType::AudioTrack(track) => track.uuid_string(),
            TrackType::MidiTrack(track) => track.uuid_string(),
        }
    }
    fn set_uuid(&mut self, uuid: Uuid) {
        match self {
            TrackType::InstrumentTrack(track) => track.set_uuid(uuid),
            TrackType::AudioTrack(track) => track.set_uuid(uuid),
            TrackType::MidiTrack(track) => track.set_uuid(uuid),
        }
    }

    fn start_background_processing(&self,
                                   tx_audio: crossbeam_channel::Sender<AudioLayerInwardEvent>,
                                   rx_vst_thread: Receiver<TrackBackgroundProcessorInwardEvent>,
                                   tx_vst_thread: Sender<TrackBackgroundProcessorOutwardEvent>,
                                   track_thread_coast: Arc<Mutex<TrackBackgroundProcessorMode>>,
                                   volume: f32,
                                   pan: f32,
                                   vst_host_time_info: Arc<RwLock<TimeInfo>>,
                                   sample_rate: f64,
                                   block_size: f64,
                                   tempo: f64,
                                   time_signature_numerator: i32,
                                   time_signature_denominator: i32,
    ) {
        match self {
            TrackType::InstrumentTrack(track) => track.start_background_processing(tx_audio, rx_vst_thread, tx_vst_thread, track_thread_coast, volume, pan, vst_host_time_info, sample_rate,
                                                                                   block_size,
                                                                                   tempo,
                                                                                   time_signature_numerator,
                                                                                   time_signature_denominator),
            TrackType::AudioTrack(track) => track.start_background_processing(tx_audio, rx_vst_thread, tx_vst_thread, track_thread_coast, volume, pan, vst_host_time_info, sample_rate,
                                                                              block_size,
                                                                              tempo,
                                                                              time_signature_numerator,
                                                                              time_signature_denominator),
            TrackType::MidiTrack(track) => track.start_background_processing(tx_audio, rx_vst_thread, tx_vst_thread, track_thread_coast, volume, pan, vst_host_time_info, sample_rate,
                                                                             block_size,
                                                                             tempo,
                                                                             time_signature_numerator,
                                                                             time_signature_denominator),
        }
    }

    fn volume(&self) -> f32 {
        match self {
            TrackType::InstrumentTrack(track) => track.volume(),
            TrackType::AudioTrack(track) => track.volume(),
            TrackType::MidiTrack(track) => track.volume(),
        }
    }

    fn volume_mut(&mut self) -> f32 {
        match self {
            TrackType::InstrumentTrack(track) => track.volume_mut(),
            TrackType::AudioTrack(track) => track.volume_mut(),
            TrackType::MidiTrack(track) => track.volume_mut(),
        }
    }

    fn set_volume(&mut self, volume: f32) {
        match self {
            TrackType::InstrumentTrack(track) => track.set_volume(volume),
            TrackType::AudioTrack(track) => track.set_volume(volume),
            TrackType::MidiTrack(track) => track.set_volume(volume),
        }
    }

    fn pan(&self) -> f32 {
        match self {
            TrackType::InstrumentTrack(track) => track.pan(),
            TrackType::AudioTrack(track) => track.pan(),
            TrackType::MidiTrack(track) => track.pan(),
        }
    }

    fn pan_mut(&mut self) -> f32 {
        match self {
            TrackType::InstrumentTrack(track) => track.pan_mut(),
            TrackType::AudioTrack(track) => track.pan_mut(),
            TrackType::MidiTrack(track) => track.pan_mut(),
        }
    }

    fn set_pan(&mut self, pan: f32) {
        match self {
            TrackType::InstrumentTrack(track) => track.set_pan(pan),
            TrackType::AudioTrack(track) => track.set_pan(pan),
            TrackType::MidiTrack(track) => track.set_pan(pan),
        }
    }

    fn midi_routings_mut(&mut self) -> &mut Vec<TrackEventRouting> {
        match self {
            TrackType::InstrumentTrack(track) => track.midi_routings_mut(),
            TrackType::AudioTrack(track) => track.midi_routings_mut(),
            TrackType::MidiTrack(track) => track.midi_routings_mut(),
        }
    }

    fn midi_routings(&self) -> &Vec<TrackEventRouting> {
        match self {
            TrackType::InstrumentTrack(track) => track.midi_routings(),
            TrackType::AudioTrack(track) => track.midi_routings(),
            TrackType::MidiTrack(track) => track.midi_routings(),
        }
    }

    fn audio_routings_mut(&mut self) -> &mut Vec<AudioRouting> {
        match self {
            TrackType::InstrumentTrack(track) => track.audio_routings_mut(),
            TrackType::AudioTrack(track) => track.audio_routings_mut(),
            TrackType::MidiTrack(track) => track.audio_routings_mut(),
        }
    }

    fn audio_routings(&self) -> &Vec<AudioRouting> {
        match self {
            TrackType::InstrumentTrack(track) => track.audio_routings(),
            TrackType::AudioTrack(track) => track.audio_routings(),
            TrackType::MidiTrack(track) => track.audio_routings(),
        }
    }
}

pub trait DAWItemID {
    fn id(&self) -> String;
    fn id_mut(&mut self) -> String;
    fn set_id(&mut self, uuid: String);
}

pub trait DAWItemPosition {
	fn position(&self) -> f64;
	fn set_position(&mut self, time: f64);
}

pub trait DAWItemLength {
	fn length(&self) -> f64;
	fn set_length(&mut self, length: f64);
}

pub trait DAWItemVerticalIndex {
	fn vertical_index(&self) -> i32;
	fn set_vertical_index(&mut self, value: i32);
}

#[derive(Clone, Copy, Serialize, Deserialize, Default)]
pub struct Measure {
	position: f64,
}

impl DAWItemPosition for Measure {
	fn position(&self) -> f64 {
		self.position
	}
	fn set_position(&mut self, time: f64) {
		self.position = time;
	}
}

impl Measure {
	pub fn new(position: f64) -> Measure {
		Measure {
			position,
		}
	}
}

#[derive(Clone, Copy, Serialize, Deserialize, Default, EnumString, PartialEq)]
pub enum NoteExpressionType {
    #[default]
    Volume = 0,
    Pan,
    Tuning,
    Vibrato,
    Expression,
    Pressure,
    Brightness,
}

#[derive(Clone, Copy, Serialize, Deserialize, Default)]
pub struct NoteExpression {
    #[serde(skip_serializing, skip_deserializing, default = "Uuid::new_v4")]
    id: Uuid,
    expression_type: NoteExpressionType,
    port: i16,
    channel: i16,
	position: f64,
	note_id: i32,
    key: i32,
	value: f64,
}

impl DAWItemID for NoteExpression {
    fn id(&self) -> String {
        return self.id.to_string()
    }

    fn set_id(&mut self, uuid: String) {
        match Uuid::parse_str(uuid.as_str()) {
            Ok(uuid) =>self.id = uuid,
            Err(_) => {}
        }
    }

    fn id_mut(&mut self) -> String {
        return self.id.to_string()
    }
}

impl DAWItemPosition for NoteExpression {
	fn position(&self) -> f64 {
		self.position
	}
	fn set_position(&mut self, time: f64) {
		self.position = time;
	}
}

impl NoteExpression {
	pub fn new_with_params(expression_type: NoteExpressionType, port: i16, channel: i16, position: f64, note_id: i32, key: i32, value: f64) -> NoteExpression {
		Self {
            expression_type,
            port,
            channel,
			position,
			note_id,
            key,
			value,
            id: Uuid::new_v4(),
		}
	}

    /// Get a reference to the note's id.
    pub fn note_id(&self) -> i32 {
        self.note_id
    }

    /// Get a reference to the note's expression value.
    pub fn value(&self) -> f64 {
        self.value
    }

    /// Set the note's id.
    pub fn set_note_id(&mut self, note_id: i32) {
        self.note_id = note_id;
    }

    /// Set the note's expression value.
    pub fn set_value(&mut self, value: f64) {
        self.value = value;
    }

    pub fn expression_type(&self) -> &NoteExpressionType {
        &self.expression_type
    }

    pub fn port(&self) -> i16 {
        self.port
    }

    pub fn channel(&self) -> i16 {
        self.channel
    }

    pub fn key(&self) -> i32 {
        self.key
    }

    pub fn set_key(&mut self, key: i32) {
        self.key = key;
    }
}

#[derive(Clone, Copy, Serialize, Deserialize, Default, PartialEq)]
pub struct Note {
    #[serde(skip_serializing, skip_deserializing, default = "Uuid::new_v4")]
    id: Uuid,
    #[serde(default)]
    note_id: i32,
    #[serde(default)]
    port: u16,
    #[serde(default)]
    channel: u16,
	position: f64,
	note: i32,
	velocity: i32,
    length: f64,
    #[serde(default)]
    riff_start_note: bool,
}

impl DAWItemID for Note {
    fn id(&self) -> String {
        self.id.to_string()
    }

    fn set_id(&mut self, uuid: String) {
        match Uuid::parse_str(uuid.as_str()) {
            Ok(uuid) =>self.id = uuid,
            Err(_) => {}
        }
    }

    fn id_mut(&mut self) -> String {
        return self.id.to_string()
    }
}

impl DAWItemPosition for Note {
	fn position(&self) -> f64 {
		self.position
	}
	fn set_position(&mut self, time: f64) {
		self.position = time;
	}
}

impl DAWItemLength for Note {
    fn length(&self) -> f64 {
        self.length
    }

    fn set_length(&mut self, length: f64) {
        self.length = length;
    }
}

impl DAWItemVerticalIndex for Note {
    fn vertical_index(&self) -> i32 {
        self.note()
    }

    fn set_vertical_index(&mut self, value: i32) {
        self.note = value;
    }
}

impl Note {
	pub fn new_with_params(note_id: i32, position: f64, note: i32, velocity: i32, duration: f64) -> Note {
		Note {
            note_id,
            channel: 0,
            port: 0,
			position,
			note,
			velocity,
            length: duration,
            id: Uuid::new_v4(),
            riff_start_note: false,
		}
	}

    /// Get a reference to the note's note.
    pub fn note(&self) -> i32 {
        self.note
    }

    /// Get a reference to the note's velocity.
    pub fn velocity(&self) -> i32 {
        self.velocity
    }

    /// Set the note's note.
    pub fn set_note(&mut self, note: i32) {
        self.note = note;
    }

    /// Set the note's velocity.
    pub fn set_velocity(&mut self, velocity: i32) {
        self.velocity = velocity;
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn port_mut(&mut self) -> &mut u16 {
        &mut self.port
    }

    pub fn set_port(&mut self, port: u16) {
        self.port = port;
    }

    pub fn channel(&self) -> u16 {
        self.channel
    }

    pub fn channel_mut(&mut self) -> &mut u16 {
        &mut self.channel
    }

    pub fn set_channel(&mut self, channel: u16) {
        self.channel = channel;
    }

    pub fn note_id(&self) -> i32 {
        self.note_id
    }

    pub fn set_note_id(&mut self, note_id: i32) {
        self.note_id = note_id;
    }

    pub fn riff_start_note(&self) -> bool {
        self.riff_start_note
    }

    pub fn set_riff_start_note(&mut self, riff_start_note: bool) {
        self.riff_start_note = riff_start_note;
    }
}


#[derive(Clone, Copy, Default, Serialize, Deserialize)]
pub struct NoteOn {
    #[serde(default)]
    note_id: i32,
    #[serde(default)]
    port: u16,
    #[serde(default)]
    channel: u16,
	position: f64,
	note: i32,
	velocity: i32,
}

impl DAWItemPosition for NoteOn {
	fn position(&self) -> f64 {
		self.position
	}
	fn set_position(&mut self, time: f64) {
		self.position = time;
	}
}

impl NoteOn {
	pub fn new_with_params(note_id: i32, position: f64, note: i32, velocity: i32) -> NoteOn {
		NoteOn {
            note_id,
            port: 0,
            channel: 0,
			position,
			note,
			velocity,
		}
	}
    pub fn note(&self) -> i32 {
        self.note
    }

    /// Set the note's note.
    pub fn set_note(&mut self, note: i32) {
        self.note = note;
    }

    pub fn velocity(&self) -> i32 {
        self.velocity
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn port_mut(&mut self) -> &mut u16 {
        &mut self.port
    }

    pub fn set_port(&mut self, port: u16) {
        self.port = port;
    }

    pub fn channel(&self) -> u16 {
        self.channel
    }

    pub fn channel_mut(&mut self) -> &mut u16 {
        &mut self.channel
    }

    pub fn set_channel(&mut self, channel: u16) {
        self.channel = channel;
    }

    pub fn note_id(&self) -> i32 {
        self.note_id
    }

    pub fn set_note_id(&mut self, note_id: i32) {
        self.note_id = note_id;
    }
}

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
pub struct NoteOff {
    #[serde(default)]
    note_id: i32,
    #[serde(default)]
    port: u16,
    #[serde(default)]
    channel: u16,
	position: f64,
	note: i32,
	velocity: i32,
}

impl NoteOff {
	pub fn new_with_params(note_id: i32, position: f64, note: i32, velocity: i32) -> NoteOff {
		NoteOff {
            note_id,
            port: 0,
            channel: 0,
			position,
			note,
			velocity,
		}
	}
    pub fn note(&self) -> i32 {
        self.note
    }

    /// Set the note's note.
    pub fn set_note(&mut self, note: i32) {
        self.note = note;
    }

    pub fn velocity(&self) -> i32 {
        self.velocity
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn port_mut(&mut self) -> &mut u16 {
        &mut self.port
    }

    pub fn set_port(&mut self, port: u16) {
        self.port = port;
    }

    pub fn channel(&self) -> u16 {
        self.channel
    }

    pub fn channel_mut(&mut self) -> &mut u16 {
        &mut self.channel
    }

    pub fn set_channel(&mut self, channel: u16) {
        self.channel = channel;
    }

    pub fn note_id(&self) -> i32 {
        self.note_id
    }

    pub fn set_note_id(&mut self, note_id: i32) {
        self.note_id = note_id;
    }
}

impl DAWItemPosition for NoteOff {
	fn position(&self) -> f64 {
		self.position
	}
	fn set_position(&mut self, time: f64) {
		self.position = time;
	}
}


#[derive(Clone, Copy, Default, Serialize, Deserialize)]
pub struct Controller {
    #[serde(skip_serializing, skip_deserializing, default)]
    id: Uuid,
	position: f64,
	controller: i32,
    value: i32,
}

impl DAWItemID for Controller {
    fn id(&self) -> String {
        self.id.to_string()
    }

    fn set_id(&mut self, uuid: String) {
        match Uuid::parse_str(uuid.as_str()) {
            Ok(uuid) =>self.id = uuid,
            Err(_) => {}
        }
    }

    fn id_mut(&mut self) -> String {
        return self.id.to_string()
    }
}

impl DAWItemPosition for Controller {
	fn position(&self) -> f64 {
		self.position
	}
	fn set_position(&mut self, time: f64) {
		self.position = time;
	}
}

impl Controller {
	pub fn new(position: f64, controller: i32, value: i32) -> Controller {
		Self {
			position,
			controller,
            value,
            id: Uuid::new_v4(),
		}
	}

    /// Get a reference to the controller's controller.
    pub fn controller(&self) -> i32 {
        self.controller
    }

    /// Get a reference to the controller's value.
    pub fn value(&self) -> i32 {
        self.value
    }

    /// Set the controller's controller.
    pub fn set_controller(&mut self, controller: i32) {
        self.controller = controller;
    }

    /// Set the controller's value.
    pub fn set_value(&mut self, value: i32) {
        self.value = value;
    }
}

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
pub struct PitchBend {
    #[serde(skip_serializing, skip_deserializing)]
    id: Uuid,
    position: f64,
    value: i32,
}

impl DAWItemID for PitchBend {
    fn id(&self) -> String {
        self.id.to_string()
    }

    fn set_id(&mut self, uuid: String) {
        match Uuid::parse_str(uuid.as_str()) {
            Ok(uuid) =>self.id = uuid,
            Err(_) => {}
        }
    }

    fn id_mut(&mut self) -> String {
        return self.id.to_string()
    }
}

impl DAWItemPosition for PitchBend {
    fn position(&self) -> f64 {
        self.position
    }
    fn set_position(&mut self, time: f64) {
        self.position = time;
    }
}

impl PitchBend {
    pub fn new(position: f64, value: i32) -> Self {
        Self { 
            position, 
            value,
            id: Uuid::new_v4(),
         }
    }
    pub fn new_from_midi_bytes(position: f64, lsb: u8, msb: u8) -> Self {
        let mut value: u16 = msb as u16;
        value <<= 7;
        value |= lsb as u16;
        Self { 
            position, 
            value: value as i32,
            id: Uuid::new_v4(),
        }
    }
    pub fn value(&self) -> i32 {
        self.value
    }
    pub fn value_mut(&mut self) -> i32 {
        self.value
    }
    pub fn set_value(&mut self, value: i32) {
        self.value = value;
    }
    pub fn set_value_from_midi_bytes(&mut self, lsb: u8, msb: u8) {
        let mut value: u16 = msb as u16;
        value <<= 7;
        value |= lsb as u16;
        self.value = value as i32;
    }
    pub fn midi_bytes_from_value(&self) -> (u8, u8) {
        let value = self.value as u16;
        let lsb: u8 = (value & 127) as u8;
        let msb: u8 = (value >> 7) as u8;

        (lsb, msb)
    }
}

/// MIDI channel pressure (channel aftertouch) - a single pressure value (0-127)
/// applied to the whole channel (status byte 0xDn).
#[derive(Clone, Copy, Default, Serialize, Deserialize)]
pub struct AfterTouch {
    #[serde(skip_serializing, skip_deserializing, default)]
    id: Uuid,
    position: f64,
    value: i32,
}

impl DAWItemID for AfterTouch {
    fn id(&self) -> String {
        self.id.to_string()
    }

    fn set_id(&mut self, uuid: String) {
        match Uuid::parse_str(uuid.as_str()) {
            Ok(uuid) => self.id = uuid,
            Err(_) => {}
        }
    }

    fn id_mut(&mut self) -> String {
        return self.id.to_string()
    }
}

impl DAWItemPosition for AfterTouch {
    fn position(&self) -> f64 {
        self.position
    }
    fn set_position(&mut self, time: f64) {
        self.position = time;
    }
}

impl AfterTouch {
    pub fn new(position: f64, value: i32) -> Self {
        Self {
            position,
            value,
            id: Uuid::new_v4(),
        }
    }

    /// Get the after touch's value.
    pub fn value(&self) -> i32 {
        self.value
    }

    /// Set the after touch's value.
    pub fn set_value(&mut self, value: i32) {
        self.value = value;
    }
}

/// MIDI polyphonic key pressure (per note aftertouch) - a pressure value (0-127)
/// for a specific key (status byte 0xAn).
#[derive(Clone, Copy, Default, Serialize, Deserialize)]
pub struct KeyPressure {
    #[serde(skip_serializing, skip_deserializing, default)]
    id: Uuid,
    position: f64,
    note: i32,
    value: i32,
}

impl DAWItemID for KeyPressure {
    fn id(&self) -> String {
        self.id.to_string()
    }

    fn set_id(&mut self, uuid: String) {
        match Uuid::parse_str(uuid.as_str()) {
            Ok(uuid) => self.id = uuid,
            Err(_) => {}
        }
    }

    fn id_mut(&mut self) -> String {
        return self.id.to_string()
    }
}

impl DAWItemPosition for KeyPressure {
    fn position(&self) -> f64 {
        self.position
    }
    fn set_position(&mut self, time: f64) {
        self.position = time;
    }
}

impl KeyPressure {
    pub fn new(position: f64, note: i32, value: i32) -> Self {
        Self {
            position,
            note,
            value,
            id: Uuid::new_v4(),
        }
    }

    /// Get the key pressure's note.
    pub fn note(&self) -> i32 {
        self.note
    }

    /// Get the key pressure's value.
    pub fn value(&self) -> i32 {
        self.value
    }

    /// Set the key pressure's note.
    pub fn set_note(&mut self, note: i32) {
        self.note = note;
    }

    /// Set the key pressure's value.
    pub fn set_value(&mut self, value: i32) {
        self.value = value;
    }
}

/// MIDI program change - selects an instrument program (0-127) on the channel
/// (status byte 0xCn).
#[derive(Clone, Copy, Default, Serialize, Deserialize)]
pub struct ProgramChange {
    #[serde(skip_serializing, skip_deserializing, default)]
    id: Uuid,
    position: f64,
    program: i32,
}

impl DAWItemID for ProgramChange {
    fn id(&self) -> String {
        self.id.to_string()
    }

    fn set_id(&mut self, uuid: String) {
        match Uuid::parse_str(uuid.as_str()) {
            Ok(uuid) => self.id = uuid,
            Err(_) => {}
        }
    }

    fn id_mut(&mut self) -> String {
        return self.id.to_string()
    }
}

impl DAWItemPosition for ProgramChange {
    fn position(&self) -> f64 {
        self.position
    }
    fn set_position(&mut self, time: f64) {
        self.position = time;
    }
}

impl ProgramChange {
    pub fn new(position: f64, program: i32) -> Self {
        Self {
            position,
            program,
            id: Uuid::new_v4(),
        }
    }

    /// Get the program change's program.
    pub fn program(&self) -> i32 {
        self.program
    }

    /// Set the program change's program.
    pub fn set_program(&mut self, program: i32) {
        self.program = program;
    }
}

/// MIDI active sensing (system real time 0xFE) - a keep alive marker showing the
/// MIDI stream is still running. Carries no data so it only has a position.
#[derive(Clone, Copy, Default, Serialize, Deserialize)]
pub struct ActiveSense {
    #[serde(skip_serializing, skip_deserializing, default)]
    id: Uuid,
    position: f64,
}

impl DAWItemID for ActiveSense {
    fn id(&self) -> String {
        self.id.to_string()
    }

    fn set_id(&mut self, uuid: String) {
        match Uuid::parse_str(uuid.as_str()) {
            Ok(uuid) => self.id = uuid,
            Err(_) => {}
        }
    }

    fn id_mut(&mut self) -> String {
        return self.id.to_string()
    }
}

impl DAWItemPosition for ActiveSense {
    fn position(&self) -> f64 {
        self.position
    }
    fn set_position(&mut self, time: f64) {
        self.position = time;
    }
}

impl ActiveSense {
    pub fn new(position: f64) -> Self {
        Self {
            position,
            id: Uuid::new_v4(),
        }
    }
}

/// A tempo event - carries a new tempo in beats per minute. On playback it is
/// applied to the track's plugin instances (there is no midi message for it)
/// just like the inward tempo event does. Tempo automation is song/riff
/// arrangement level data that gets fed to every track when the song or the
/// arrangement is played.
#[derive(Clone, Copy, Default, Serialize, Deserialize)]
pub struct Tempo {
    #[serde(skip_serializing, skip_deserializing, default)]
    id: Uuid,
    position: f64,
    bpm: f64,
}

impl DAWItemID for Tempo {
    fn id(&self) -> String {
        self.id.to_string()
    }

    fn set_id(&mut self, uuid: String) {
        match Uuid::parse_str(uuid.as_str()) {
            Ok(uuid) => self.id = uuid,
            Err(_) => {}
        }
    }

    fn id_mut(&mut self) -> String {
        return self.id.to_string()
    }
}

impl DAWItemPosition for Tempo {
    fn position(&self) -> f64 {
        self.position
    }
    fn set_position(&mut self, time: f64) {
        self.position = time;
    }
}

impl Tempo {
    pub fn new(position: f64, bpm: f64) -> Self {
        Self {
            position,
            bpm,
            id: Uuid::new_v4(),
        }
    }

    /// Get the tempo's beats per minute.
    pub fn bpm(&self) -> f64 {
        self.bpm
    }

    /// Set the tempo's beats per minute.
    pub fn set_bpm(&mut self, bpm: f64) {
        self.bpm = bpm;
    }
}

/// The tempo automation lane is a 0-127 automation lane like the other midi
/// lanes - this is the bpm that the top of the lane maps to (and from).
pub const TEMPO_LANE_MAX_BPM: f64 = 240.0;

/// Converts an automation lane value (0-127) to a tempo bpm.
pub fn tempo_lane_value_to_bpm(value: f64) -> f64 {
    value * TEMPO_LANE_MAX_BPM / 127.0
}

/// Converts a tempo bpm to an automation lane value (0-127).
pub fn tempo_lane_bpm_to_value(bpm: f64) -> f64 {
    bpm * 127.0 / TEMPO_LANE_MAX_BPM
}


#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct SampleReference {
	position: f64,
    sample_ref_uuid: Uuid,
}

impl DAWItemPosition for SampleReference {
	fn position(&self) -> f64 {
		self.position
	}
	fn set_position(&mut self, time: f64) {
		self.position = time;
	}
}

impl SampleReference {
	pub fn new(position: f64, sample_ref_uuid: String) -> SampleReference {
		Self {
			position,
            sample_ref_uuid: Uuid::parse_str(&sample_ref_uuid).unwrap(),
		}
	}
    pub fn sample_ref_uuid(&self) -> String {
        self.sample_ref_uuid.to_string()
    }
    pub fn sample_ref_uuid_mut(&mut self) -> String {
        self.sample_ref_uuid.to_string()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Sample {
    uuid: Uuid,
    name: String,
    file_name: String,
    sample_data_uuid: String,
}

impl Sample {
    pub fn new(name: String, file: String, sample_data_uuid: String) -> Self {
        Self {
            uuid: Uuid::new_v4(),
            name,
            file_name: file,
            sample_data_uuid,
        }
    }
    pub fn uuid(&self) -> Uuid {
        self.uuid
    }
    pub fn uuid_mut(&mut self) -> Uuid {
        self.uuid
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn name_mut(&mut self) -> &str {
        &self.name
    }
    pub fn file_name(&self) -> &str {
        &self.file_name
    }
    pub fn file_name_mut(&mut self) -> &str {
        &self.file_name
    }
    pub fn sample_data_uuid(&self) -> &str {
        &self.sample_data_uuid
    }
    pub fn sample_data_uuid_mut(&mut self) -> &str {
        &self.sample_data_uuid
    }
}

#[derive(Clone)]
pub struct SampleData {
    uuid: Uuid,
    channels: i32,
    samples: Vec<f32>,
}

impl SampleData {
    pub fn new(wav_file_name: String, sample_rate: i32) -> Self {
        let (channels, samples) = SampleData::load_data(wav_file_name, sample_rate);
        Self {
            uuid: Uuid::new_v4(),
            channels,
            samples,
        }
    }

    pub fn new_with_uuid(uuid: String, wav_file_name: String, sample_rate: i32) -> Self {
        let (channels, samples) = SampleData::load_data(wav_file_name, sample_rate);
        Self {
            uuid: Uuid::parse_str(uuid.as_str()).unwrap(),
            channels,
            samples,
        }
    }

    pub fn load_data(wav_file_name: String, sample_rate: i32) -> (i32, Vec<f32>) {
        if let Ok(mut wav_file) = OpenOptions::ReadOnly(ReadOptions::Auto).from_path(wav_file_name.as_str()) {
            if let Ok(wav_data) = wav_file.read_all_to_vec() {
                if wav_file.get_samplerate() != sample_rate as usize {
                    let resampled_data = convert(wav_file.get_samplerate() as u32, sample_rate as u32, 1, ConverterType::SincBestQuality, &wav_data).unwrap();
                    (wav_file.get_channels() as i32, resampled_data)
                }
                else {
                    (wav_file.get_channels() as i32, wav_data)
                }
            }
            else {
                (2, vec![])
            }
        }
        else {
            (2, vec![])
        }
    }

    pub fn channels(&self) -> i32 {
        self.channels
    }

    pub fn samples(&self) -> &Vec<f32> {
        &self.samples
    }

    pub fn uuid(&self) -> Uuid {
        self.uuid
    }

    pub fn uuid_mut(&mut self) -> Uuid {
        self.uuid
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Riff {
    uuid: Uuid,
	name: String,
	position: f64,
	length: f64,
    colour: Option<(f64, f64, f64, f64)>, // rgba
	events: Vec<TrackEvent>,
    #[serde(skip_serializing, skip_deserializing)]
    vertical_index: i32,
}

impl DAWItemID for Riff {
    fn id(&self) -> String {
        self.uuid().to_string()
    }

    fn set_id(&mut self, uuid: String) {
        if let Ok(uuid) = Uuid::parse_str(uuid.as_str()) {
            self.uuid = uuid;
        }
    }

    fn id_mut(&mut self) -> String {
        self.uuid.to_string()
    }
}

impl DAWItemPosition for Riff {
	fn position(&self) -> f64 {
		self.position
	}
	fn set_position(&mut self, time: f64) {
		self.position = time;
	}
}

impl DAWItemLength for Riff {
    fn length(&self) -> f64 {
        self.length
    }

    fn set_length(&mut self, length: f64) {
        self.length = length;
    }
}

impl DAWItemVerticalIndex for Riff {
    fn vertical_index(&self) -> i32 {
        self.vertical_index
    }

    fn set_vertical_index(&mut self, value: i32) {
        self.vertical_index = value;
    }
}

impl Riff {
    pub fn new_with_name_and_length(uuid: Uuid, name: String, length: f64) -> Riff {
        Riff {
            uuid,
            name,
            position: 0.0,
            length,
            colour: None,
            events: vec![],
            vertical_index: 0,
        }
    }

    pub fn new_with_position_length_and_colour(uuid: Uuid, position: f64, length:f64, colour: Option<(f64, f64, f64, f64)>) -> Riff {
        Riff {
            uuid,
            name: String::new(),
            position,
            length,
            colour,
            events: vec![],
            vertical_index: 0,
        }
    }

    /// Get a mutable reference to the pattern's events.
    pub fn events_mut(&mut self) -> &mut Vec<TrackEvent> {
        &mut self.events
    }

    pub(crate) fn colour(&self) -> &Option<(f64, f64, f64, f64)> {
        &self.colour
    }

    pub fn set_colour(&mut self, colour: Option<(f64, f64, f64, f64)>) {
        self.colour = colour;
    }

    /// Get a reference to the riff's name.
    pub fn name(&self) -> &str {
        self.name.as_ref()
    }

    /// Set the riff's name.
    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }

    /// Get a reference to the riff's events.
    pub fn events(&self) -> &[TrackEvent] {
        self.events.as_ref()
    }

    pub fn events_vec(&self) -> &Vec<TrackEvent> {
        &self.events
    }

    pub fn uuid(&self) -> Uuid {
        self.uuid
    }

    pub fn set_uuid(&mut self, uuid: Uuid) {
        self.uuid = uuid;
    }

    pub fn colour_mut(&mut self) -> &mut Option<(f64, f64, f64, f64)> {
        &mut self.colour
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RiffReferenceMode {
    Normal,
    Start,
    End,
}

impl RiffReferenceMode {
    pub fn normal() -> RiffReferenceMode {
        RiffReferenceMode::Normal
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RiffReference {
    uuid: Uuid,
	position: f64,
	linked_to: String,
    #[serde(default = "RiffReferenceMode::normal")]
    mode: RiffReferenceMode,
    #[serde(skip_serializing, skip_deserializing, default = "String::new")]
    track_id: String,
}

impl DAWItemID for RiffReference {
    fn id(&self) -> String {
        self.uuid().to_string()
    }

    fn id_mut(&mut self) -> String {
        return self.uuid.to_string()
    }

    fn set_id(&mut self, uuid: String) {
        if let Ok(uuid) = Uuid::parse_str(uuid.as_str()) {
            self.uuid = uuid;
        }
    }
}

impl DAWItemPosition for RiffReference {
	fn position(&self) -> f64 {
		self.position
	}
	fn set_position(&mut self, time: f64) {
		self.position = time;
	}
}

impl DAWItemLength for RiffReference {
    fn length(&self) -> f64 {
        0.0
    }

    fn set_length(&mut self, _length: f64) {
        
    }
}

impl DAWItemVerticalIndex for RiffReference {
    fn vertical_index(&self) -> i32 {
        0
    }

    fn set_vertical_index(&mut self, _value: i32) {
        
    }
}

impl RiffReference {
    pub fn new(riff_uuid: String, position: f64) -> RiffReference {
        RiffReference {
            uuid: Uuid::new_v4(),
            position,
            linked_to: riff_uuid,
            mode: RiffReferenceMode::Normal,
            track_id: String::new(),
        }
    }

    /// Get a mutable reference to the riff reference's position.
    pub fn position_mut(&mut self) -> &mut f64 {
        &mut self.position
    }

    /// Get a mutable reference to the riff reference's linked to.
    pub fn linked_to_mut(&mut self) -> &mut String {
        &mut self.linked_to
    }

    /// Set the riff reference's linked to.
    pub fn set_linked_to(&mut self, linked_to: String) {
        self.linked_to = linked_to;
    }

    /// Get a reference to the riff reference's linked to.
    pub fn linked_to(&self) -> String {
        self.linked_to.clone()
    }

    pub fn uuid(&self) -> Uuid {
        self.uuid
    }

    pub fn mode(&self) -> &RiffReferenceMode {
        &self.mode
    }

    pub fn set_mode(&mut self, mode: RiffReferenceMode) {
        self.mode = mode;
    }

    pub fn track_id(&self) -> &str {
        &self.track_id
    }

    pub fn set_track_id(&mut self, track_id: String) {
        self.track_id = track_id;
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RiffSet {
    uuid: Uuid,
    name: String,
    riff_refs: HashMap<String, RiffReference>, // track uuid, riff ref
}

impl RiffSet {
    pub fn new() -> Self {
        RiffSet {
            uuid: Uuid::new_v4(),
            name: "Unknown".to_owned(),
            riff_refs: HashMap::new(),
        }
    }

    pub fn new_with_uuid(uuid: Uuid) -> Self {
        RiffSet {
            uuid,
            name: "Unknown".to_owned(),
            riff_refs: HashMap::new(),
        }
    }

    pub fn uuid(&self) -> String {
        self.uuid.to_string()
    }

    pub fn riff_refs(&self) -> &HashMap<String, RiffReference> {
        &self.riff_refs
    }

    pub fn set_riff_ref_for_track(&mut self, track_uuid: String, riff_ref: RiffReference) {
        self.riff_refs.insert(track_uuid, riff_ref);
    }

    pub fn get_riff_ref_for_track(&self, track_uuid: String) -> Option<&RiffReference> {
        self.riff_refs.get(&track_uuid)
    }

    pub fn get_riff_ref_for_track_mut(&mut self, track_uuid: String) -> Option<&mut RiffReference> {
        self.riff_refs.get_mut(&track_uuid)
    }

    /// Remove a track's riff reference from the set - the track slot becomes silent.
    /// No "empty" placeholder riff or reference is needed to represent silence.
    pub fn remove_riff_ref_for_track(&mut self, track_uuid: String) -> Option<RiffReference> {
        self.riff_refs.remove(&track_uuid)
    }

    pub fn remove_track(&mut self, track_uuid: String) {
        self.riff_refs.remove_entry(&track_uuid);
    }

    pub fn name(&self) -> &str {
        self.name.as_ref()
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RiffSequence {
    uuid: Uuid,
    name: String,
    riff_sets: Vec<RiffItem>,
}

impl RiffSequence {
    pub fn new() -> Self {
        RiffSequence {
            uuid: Uuid::new_v4(),
            name: "Unknown".to_owned(),
            riff_sets: vec![],
        }
    }

    pub fn new_with_uuid(uuid: Uuid) -> Self {
        RiffSequence {
            uuid,
            name: "Unknown".to_owned(),
            riff_sets: vec![],
        }
    }

    pub fn uuid(&self) -> String {
        self.uuid.to_string()
    }

    pub fn riff_sets(&self) -> &Vec<RiffItem> {
        &self.riff_sets
    }

    pub fn add_riff_set(&mut self, reference_uuid: Uuid, riff_set_uuid: String) {
        self.riff_sets.push(RiffItem::new_with_uuid(reference_uuid, RiffItemType::RiffSet, riff_set_uuid));
    }

    pub fn add_riff_set_at_position(&mut self, reference_uuid: Uuid, riff_set_uuid: String, position: usize) {
        self.riff_sets.insert(position, RiffItem::new_with_uuid(reference_uuid, RiffItemType::RiffSet, riff_set_uuid));
    }

    pub fn remove_riff_set(&mut self, reference_uuid: String) {
        self.riff_sets.retain(|current_riff_set_reference| current_riff_set_reference.uuid() != reference_uuid);
    }

    pub fn name(&self) -> &str {
        self.name.as_ref()
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }

    pub fn riff_set_move_left(&mut self, reference_uuid: String) {
        let mut index_1 = -1;
        let mut index_2 = -1;
        let mut count = 0;
        for current_riff_set_reference in self.riff_sets.iter_mut() {
            if current_riff_set_reference.uuid() == reference_uuid {
                index_1 = count;
            }
            else {
                index_2 = count;
            }
            if index_1 > -1 && index_2 > -1 {
                break;
            }

            count += 1;
        }

        if index_1 > -1 && index_2 > -1 {
            self.riff_sets.swap(index_1 as usize, index_2 as usize);
        }
    }

    pub fn riff_set_move_right(&mut self, reference_uuid: String) {
        let mut index_1 = -1;
        let mut index_2 = -1;
        let mut count = self.riff_sets.len() as i32 - 1;
        let mut riff_set_reference_uuids: Vec<String> = self.riff_sets.iter_mut().map(|current_riff_set_reference| current_riff_set_reference.uuid()).collect();

        riff_set_reference_uuids.reverse();
        for current_riff_set_reference_uuid in riff_set_reference_uuids.iter_mut() {
            if *current_riff_set_reference_uuid == reference_uuid {
                index_1 = count;
            }
            else {
                index_2 = count;
            }
            if index_1 > -1 && index_2 > -1 {
                break;
            }

            count -= 1;
        }

        if index_1 > -1 && index_2 > -1 {
            self.riff_sets.swap(index_1 as usize, index_2 as usize);
        }
    }

    pub fn riff_sets_mut(&mut self) -> &mut Vec<RiffItem> {
        &mut self.riff_sets
    }

    pub fn set_uuid(&mut self, uuid: Uuid) {
        self.uuid = uuid;
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RiffGrid {
    uuid: Uuid,
    name: String,
    tracks: HashMap<String, Vec<RiffReference>>,
}

impl RiffGrid {
    pub fn new() -> Self {
        RiffGrid {
            uuid: Uuid::new_v4(),
            name: "Unknown".to_owned(),
            tracks: HashMap::new(),
        }
    }

    pub fn new_with_uuid(uuid: Uuid) -> Self {
        RiffGrid {
            uuid,
            name: "Unknown".to_owned(),
            tracks: HashMap::new(),
        }
    }

    pub fn uuid(&self) -> String {
        self.uuid.to_string()
    }

    pub fn name(&self) -> &str {
        self.name.as_ref()
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }

    pub fn track_riff_references(&self, track_uuid: String) -> Option<&Vec<RiffReference>> {
        self.tracks.get(track_uuid.as_str())
    }

    pub fn track_riff_references_mut(&mut self, track_uuid: String) -> Option<&mut Vec<RiffReference>> {
        self.tracks.get_mut(track_uuid.as_str())
    }

    pub fn tracks(&self) -> Keys<'_, String, Vec<RiffReference>> {
        self.tracks.keys()
    }

    pub fn tracks_mut(&mut self) -> Keys<'_, String, Vec<RiffReference>> {
        self.tracks.keys()
    }


    pub fn add_riff_reference_to_track(&mut self, track_uuid: String, riff_uuid: String, position: f64) {
        let track = if let Some(track) = self.tracks.get_mut(&track_uuid) {
            track
        }
        else {
            let new_track = vec![];
            self.tracks.insert(track_uuid.clone(), new_track);
            self.tracks.get_mut(&track_uuid).unwrap()
        };
        track.push(RiffReference::new(riff_uuid, position));
        track.sort_by(|riff_ref1, riff_ref2| DAWUtils::sort_by_daw_position(riff_ref1, riff_ref2));
    }

    pub fn remove_riff_reference_from_track(&mut self, track_uuid: String, reference_uuid: String) {
        if let Some(track) = self.tracks.get_mut(&track_uuid) {
            track.retain(|riff_reference| riff_reference.uuid().to_string() != reference_uuid);
        }
    }
}



#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub enum RiffItemType {
    RiffSet,
    RiffSequence,
    RiffGrid,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RiffItem {
    uuid: Uuid,
    item_type: RiffItemType,
    item_uuid: String
}

impl RiffItem {
    pub fn new(riff_item_type: RiffItemType, item_uuid: String) -> Self {
        RiffItem {
            uuid: Uuid::new_v4(),
            item_type: riff_item_type,
            item_uuid,
        }
    }

    pub fn new_with_uuid(uuid: Uuid, riff_item_type: RiffItemType, item_uuid: String) -> Self {
        RiffItem {
            uuid,
            item_type: riff_item_type,
            item_uuid,
        }
    }

    pub fn new_with_uuid_string(uuid: String, riff_item_type: RiffItemType, item_uuid: String) -> Self {
        RiffItem {
            uuid: Uuid::parse_str(uuid.as_str()).unwrap(),
            item_type: riff_item_type,
            item_uuid,
        }
    }

    pub fn uuid(&self) -> String {
        self.uuid.to_string()
    }

    pub fn item_type(&self) -> &RiffItemType {
        &self.item_type
    }

    pub fn item_uuid(&self) -> &str {
        &self.item_uuid
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RiffArrangement {
    uuid: Uuid,
    name: String,
    items: Vec<RiffItem>,
    track_automation: HashMap<String, Automation>,
    #[serde(default)]
    tempo_automation: Automation,
    #[serde(default)]
    track_instrument_overrides: HashMap<String, InstrumentOverride>,
    #[serde(default)]
    track_effects_overrides: HashMap<String, EffectsOverride>,
}

/// A per track instrument plugin override for a riff arrangement. When enabled the
/// arrangement's override instrument receives all the events that would normally go
/// to the track's own instrument plugin (except for events addressed to a particular
/// plugin instance by uuid - e.g. automation), and its audio feeds the track's
/// (overridden or not) effects chain.
#[derive(Clone, Serialize, Deserialize)]
pub struct InstrumentOverride {
    enabled: bool,
    instrument: Option<AudioPlugin>,
}

impl InstrumentOverride {
    pub fn new() -> Self {
        InstrumentOverride {
            enabled: false,
            instrument: None,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn instrument(&self) -> Option<&AudioPlugin> {
        self.instrument.as_ref()
    }

    pub fn instrument_mut(&mut self) -> &mut Option<AudioPlugin> {
        &mut self.instrument
    }

    pub fn set_instrument(&mut self, instrument: Option<AudioPlugin>) {
        self.instrument = instrument;
    }
}

/// A per track effects chain override for a riff arrangement. When enabled the
/// arrangement's override effects receive the audio from whichever instrument plugin
/// is active, and events addressed to the track's own effects (by plugin uuid) are
/// never sent to the override effects.
#[derive(Clone, Serialize, Deserialize)]
pub struct EffectsOverride {
    enabled: bool,
    effects: Vec<AudioPlugin>,
}

impl EffectsOverride {
    pub fn new() -> Self {
        EffectsOverride {
            enabled: false,
            effects: vec![],
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn effects(&self) -> &Vec<AudioPlugin> {
        self.effects.as_ref()
    }

    pub fn effects_mut(&mut self) -> &mut Vec<AudioPlugin> {
        &mut self.effects
    }

    pub fn set_effects(&mut self, effects: Vec<AudioPlugin>) {
        self.effects = effects;
    }
}

impl RiffArrangement {
    pub fn new() -> Self {
        RiffArrangement {
            uuid: Uuid::new_v4(),
            name: "Unknown".to_owned(),
            items: vec![],
            track_automation: HashMap::new(),
            tempo_automation: Automation::new(),
            track_instrument_overrides: HashMap::new(),
            track_effects_overrides: HashMap::new(),
        }
    }

    pub fn new_with_uuid(uuid: Uuid) -> Self {
        RiffArrangement {
            uuid,
            name: "Unknown".to_owned(),
            items: vec![],
            track_automation: HashMap::new(),
            tempo_automation: Automation::new(),
            track_instrument_overrides: HashMap::new(),
            track_effects_overrides: HashMap::new(),
        }
    }

    pub fn uuid(&self) -> String {
        self.uuid.to_string()
    }

    pub fn set_uuid(&mut self, uuid: Uuid) {
        self.uuid = uuid;
    }

    pub fn name(&self) -> &str {
        self.name.as_ref()
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }

    pub fn items(&self) -> &Vec<RiffItem> {
        &self.items
    }

    pub fn items_mut(&mut self) -> &mut Vec<RiffItem> {
        &mut self.items
    }

    pub fn add_item(&mut self, item: RiffItem) {
        self.items.push(item);
    }

    pub fn add_item_at_position(&mut self, item: RiffItem, position: usize) {
        self.items.insert(position, item);
    }

    pub fn remove_item(&mut self, item_uuid: String) {
        self.items.retain(|item| item.uuid() != item_uuid);
    }

    pub fn automation_mut(&mut self, track_uuid: &String) -> Option<&mut Automation> {
        self.track_automation.get_mut(track_uuid)
    }

    pub fn automation(&self, track_uuid: &String) -> Option<&Automation> {
        self.track_automation.get(track_uuid)
    }

    pub fn add_track_automation(&mut self, track_uuid: String) {
        self.track_automation.insert(track_uuid, Automation::new());
    }

    pub fn remove_track_automation(&mut self, track_uuid: &String) {
        self.track_automation.remove(track_uuid);
    }

    /// Get a reference to this arrangement's tempo automation. Tempo is not a per
    /// track thing - the arrangement's tempo automation is fed to every track when
    /// the arrangement is played.
    pub fn tempo_automation(&self) -> &Automation {
        &self.tempo_automation
    }

    /// Get a mutable reference to this arrangement's tempo automation.
    pub fn tempo_automation_mut(&mut self) -> &mut Automation {
        &mut self.tempo_automation
    }

    pub fn instrument_override(&self, track_uuid: &String) -> Option<&InstrumentOverride> {
        self.track_instrument_overrides.get(track_uuid)
    }

    pub fn instrument_override_mut(&mut self, track_uuid: &String) -> Option<&mut InstrumentOverride> {
        self.track_instrument_overrides.get_mut(track_uuid)
    }

    pub fn add_track_instrument_override(&mut self, track_uuid: String) {
        self.track_instrument_overrides.entry(track_uuid).or_insert_with(InstrumentOverride::new);
    }

    pub fn remove_track_instrument_override(&mut self, track_uuid: &String) {
        self.track_instrument_overrides.remove(track_uuid);
    }

    pub fn effects_override(&self, track_uuid: &String) -> Option<&EffectsOverride> {
        self.track_effects_overrides.get(track_uuid)
    }

    pub fn effects_override_mut(&mut self, track_uuid: &String) -> Option<&mut EffectsOverride> {
        self.track_effects_overrides.get_mut(track_uuid)
    }

    pub fn add_track_effects_override(&mut self, track_uuid: String) {
        self.track_effects_overrides.entry(track_uuid).or_insert_with(EffectsOverride::new);
    }

    pub fn remove_track_effects_override(&mut self, track_uuid: &String) {
        self.track_effects_overrides.remove(track_uuid);
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AutomationEnvelope {
    event_details: TrackEvent,
    events: Vec<TrackEvent>,
}

impl AutomationEnvelope {
    pub fn new(event_details: TrackEvent) -> Self {
        AutomationEnvelope {
            event_details,
            events: vec![],
        }
    }

    /// Get a reference to the envelope's events.
    #[must_use]
    pub fn events(&self) -> &Vec<TrackEvent> {
        self.events.as_ref()
    }

    /// Get a mutable reference to the envelope's events.
    #[must_use]
    pub fn events_mut(&mut self) -> &mut Vec<TrackEvent> {
        &mut self.events
    }

    /// Set the envelope's events.
    pub fn set_events(&mut self, events: Vec<TrackEvent>) {
        self.events = events;
    }

    pub fn event_details(&self) -> TrackEvent {
        self.event_details
    }
}


#[derive(Clone, Serialize, Deserialize)]
pub struct Automation {
	events: Vec<TrackEvent>, // discrete events
    #[serde(default = "Vec::new")]
    envelopes: Vec<AutomationEnvelope>, // continuous events
}

impl Default for Automation {
    fn default() -> Self {
        Automation::new()
    }
}

impl Automation {
	pub fn new() -> Automation {
		Automation {
            events: vec![],
            envelopes: vec![],
		}
	}

    /// Get a reference to the automation's events.
    #[must_use]
    pub fn events(&self) -> &Vec<TrackEvent> {
        self.events.as_ref()
    }

    /// Get a mutable reference to the automation's events.
    #[must_use]
    pub fn events_mut(&mut self) -> &mut Vec<TrackEvent> {
        &mut self.events
    }

    /// Set the automation's events.
    pub fn set_events(&mut self, events: Vec<TrackEvent>) {
        self.events = events;
    }

    /// Get a reference to the automation's envelopes.
    #[must_use]
    pub fn envelopes(&self) -> &Vec<AutomationEnvelope> {
        self.envelopes.as_ref()
    }

    /// Get a mutable reference to the automation's envelopes.
    #[must_use]
    pub fn envelopes_mut(&mut self) -> &mut Vec<AutomationEnvelope> {
        &mut self.envelopes
    }

    /// Set the automation's envelopes.
    pub fn set_envelopes(&mut self, envelopes: Vec<AutomationEnvelope>) {
        self.envelopes = envelopes;
    }
}

pub struct VstHost {
    shell_id: Option<isize>,
    track_uuid: String,
    plugin_uuid: String,
    instrument: bool,
    sender: Sender<AudioPluginHostOutwardEvent>,
    vst_host_time_info: Arc<RwLock<TimeInfo>>,
    ppq_pos: f64,
    sample_position: f64,
    tempo: f64,
    time_signature_numerator: u32,
    time_signature_denominator: u32,
    track_event_outward_routings: HashMap<String, TrackEventRouting>,
    track_event_outward_ring_buffers: HashMap<String, SpscRb<TrackEvent>>,
    track_event_outward_producers: HashMap<String, Producer<TrackEvent>>,
    block_size: isize,
}

impl VstHost {

    pub fn new(
        track_uuid: String,
        shell_id: Option<isize>,
        sender: Sender<AudioPluginHostOutwardEvent>,
        plugin_uuid: String,
        instrument: bool,
        vst_host_time_info: Arc<RwLock<TimeInfo>>,
        tempo: f64,
        time_signature_numerator: u32,
        time_signature_denominator: u32,
        block_size: isize,
    ) -> VstHost {
        VstHost {
            shell_id,
            track_uuid,
            plugin_uuid,
            instrument,
            sender,
            vst_host_time_info,
            ppq_pos: 0.0,
            sample_position: 0.0,
            tempo,
            time_signature_numerator,
            time_signature_denominator,
            track_event_outward_routings: HashMap::new(),
            track_event_outward_ring_buffers: HashMap::new(),
            track_event_outward_producers: HashMap::new(),
            block_size,
        }
    }

    /// Get the sample host's shell id.
    #[must_use]
    pub fn shell_id(&self) -> Option<isize> {
        self.shell_id
    }

    /// Set the sample host's shell id.
    pub fn set_shell_id(&mut self, shell_id: Option<isize>) {
        self.shell_id = shell_id;
    }

    /// Get a reference to the vst host's track uuid.
    #[must_use]
    pub fn track_uuid(&self) -> &str {
        self.track_uuid.as_ref()
    }

    /// Get the vst host's instrument.
    #[must_use]
    pub fn instrument(&self) -> bool {
        self.instrument
    }

    /// Set the vst host's instrument.
    pub fn set_instrument(&mut self, instrument: bool) {
        self.instrument = instrument;
    }

    pub fn set_ppq_pos(&mut self, ppq_pos: f64) {
        self.ppq_pos = ppq_pos;
    }

    pub fn set_block_size(&mut self, block_size: isize) {
        self.block_size = block_size;
    }

    pub fn set_tempo(&mut self, tempo: f64) {
        self.tempo = tempo;
    }

    pub fn add_track_event_outward_routing(&mut self, track_event_routing: TrackEventRouting, ring_buffer: SpscRb<TrackEvent>, producer: Producer<TrackEvent>) {
        self.track_event_outward_ring_buffers.insert(track_event_routing.uuid(), ring_buffer);
        self.track_event_outward_producers.insert(track_event_routing.uuid(), producer);
        self.track_event_outward_routings.insert(track_event_routing.uuid(), track_event_routing);
    }

    pub fn remove_track_event_outward_routing(&mut self, route_uuid: String) {
        self.track_event_outward_routings.remove(&route_uuid);
        self.track_event_outward_ring_buffers.remove(&route_uuid);
        self.track_event_outward_producers.remove(&route_uuid);
    }

    pub fn set_sample_position(&mut self, sample_position: f64) {
        self.sample_position = sample_position;
    }

    pub fn tempo(&self) -> f64 {
        self.tempo
    }

    pub fn set_time_signature_numerator(&mut self, time_signature_numerator: u32) {
        self.time_signature_numerator = time_signature_numerator;
    }

    pub fn set_time_signature_denominator(&mut self, time_signature_denominator: u32) {
        self.time_signature_denominator = time_signature_denominator;
    }
}

impl Host for VstHost {
    fn automate(&self, index: i32, value: f32) {
        debug!("Vst2 plugin automate data.");
        debug!("Parameter {} had its value changed to {}", index, value);
        match self.sender.send(AudioPluginHostOutwardEvent::Automation(self.track_uuid.clone(), self.plugin_uuid.clone(), self.instrument, index, value)) {
            Ok(_) => (),
            Err(_error) => debug!("Problem sending plugin param automation from vst2 host."),
        }
    }

    fn get_plugin_id(&self) -> i32 {
        debug!("Vst plugin asked for host plugin id???.");
        match self.shell_id() {
            Some(shell_id) => shell_id as i32,
            None => 0,
        }
    }

    fn idle(&self) {
        debug!("Vst plugin asked for host idle.");
        // self.
    }

    fn get_info(&self) -> (isize, String, String) {
        debug!("Vst plugin asked for host info.");
        (8, "vendor string".to_owned(), "product string".to_owned())
    }

    fn process_events(&self, events: &vst::api::Events) {
        // debug!("Vst plugin asked for host to process events.");
        let mut routable_events = vec![];
        for event in events.events() {
            match event {
                vst::prelude::Event::Midi(midi_event) => {
                    // debug!("Midi event: delta_frames={}, data={:?}", midi_event.delta_frames, midi_event.data);
                    routable_events.push(midi_event);
                }
                vst::prelude::Event::SysEx(sysex_event) => debug!("Sysex event: delta_frames={}, data={:?}", sysex_event.delta_frames, sysex_event.payload),
                vst::prelude::Event::Deprecated(_) => debug!("Deprecated event received."),
            }
        }

        let routable_events = DAWUtils::convert_vst_events_to_track_events_with_timing_in_frames(routable_events);
        for (route_uuid, producer) in self.track_event_outward_producers.iter() {
            for event in routable_events.iter() {                
                if let Some(_midi_routing) = self.track_event_outward_routings.get(route_uuid) {
                    let event_array = [event.clone()];
                    let _ = producer.write(&event_array);
                }
            }
        }
    }

    fn get_time_info(&self, _mask: i32) -> Option<TimeInfo> {
        // debug!("Vst plugin asked host for time info.");
        let mut flags = 0;
        
        flags |= TimeInfoFlags::TRANSPORT_CHANGED.bits();
        flags |= TimeInfoFlags::TRANSPORT_PLAYING.bits(); // transport playing
        flags |= TimeInfoFlags::TEMPO_VALID.bits(); // tempo valid
        flags |= TimeInfoFlags::TIME_SIG_VALID.bits(); // time signature valid
        flags |= TimeInfoFlags::PPQ_POS_VALID.bits(); // ppq position valid
        flags |= TimeInfoFlags::BARS_VALID.bits(); // ppq position valid

        let bar = (self.ppq_pos / 4.0) as i32;
        let beat_in_bar = self.ppq_pos as i32 % 4;

        let time_info = TimeInfo {
            sample_pos: self.sample_position,
            sample_rate: self.vst_host_time_info.read().sample_rate,
            nanoseconds: 0.0,
            ppq_pos: self.ppq_pos,
            tempo: self.vst_host_time_info.read().tempo,
            bar_start_pos: bar as f64 + beat_in_bar as f64,
            cycle_start_pos: 0.0,
            cycle_end_pos: 0.0,
            time_sig_numerator: self.vst_host_time_info.read().time_sig_numerator,
            time_sig_denominator: self.vst_host_time_info.read().time_sig_denominator,
            smpte_offset: 0,
            smpte_frame_rate: vst::api::SmpteFrameRate::Smpte24fps,
            samples_to_next_clock: 0,
            flags,
        };
        Some(time_info)
    }

    fn get_block_size(&self) -> isize {
        debug!("Vst plugin asked for host block size.");
        self.block_size
    }

    fn update_display(&self) {
        debug!("Vst plugin asked for host to update the display.");
    }

    fn begin_edit(&self, _index: i32) {}

    fn end_edit(&self, _index: i32) {}

    fn can_do(&self, value: HostCanDo) -> i32 {
        // debug!("Vst plugin asked host whether it can do: {:?}", value);
        match value {
            HostCanDo::SendEvents => 1,
            HostCanDo::SendMidiEvent => 1,
            HostCanDo::SendTimeInfo => 1,
            HostCanDo::ReceiveEvents => 1,
            HostCanDo::ReceiveMidiEvent => 1,
            HostCanDo::ReportConnectionChanges => 0,
            HostCanDo::AcceptIOChanges => 0,
            HostCanDo::SizeWindow => 1,
            HostCanDo::Offline => 0,
            HostCanDo::OpenFileSelector => 0,
            HostCanDo::CloseFileSelector => 0,
            HostCanDo::StartStopProcess => 1,
            HostCanDo::ShellCategory => 1,
            HostCanDo::SendMidiEventFlagIsRealtime => 1,
            HostCanDo::Other(_) => 0,
        }
    }

    fn size_window(&self, index: i32, value: isize) -> i32 {
        debug!("Vst plugin asked host to size the plugin window: {}, {}", index, value);
        match self.sender.send(AudioPluginHostOutwardEvent::SizeWindow(self.track_uuid.clone(), self.plugin_uuid.clone(), self.instrument, index, value as i32)) {
            Ok(_) => (),
            Err(_error) => debug!("Problem sending plugin size window from vst host."),
        }
        1
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginParameterDetail {
    pub index: i32,
    pub name: String,
    pub label: String,
    pub text: String,
}

impl PluginParameterDetail {
    /// Get a reference to the vst plugin parameter's name.
    #[must_use]
    pub fn name(&self) -> &str {
        self.name.as_ref()
    }

    /// Get a reference to the vst plugin parameter's label.
    #[must_use]
    pub fn label(&self) -> &str {
        self.label.as_ref()
    }

    /// Get a reference to the vst plugin parameter's text.
    #[must_use]
    pub fn text(&self) -> &str {
        self.text.as_ref()
    }
}


#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PluginParameter {
    #[serde(skip_serializing, skip_deserializing, default = "Uuid::new_v4")]
    pub id: Uuid,
    pub index: i32,
	pub position: f64,
    pub value: f32,
    pub instrument: bool,
    pub plugin_uuid: Uuid,
}

impl DAWItemID for PluginParameter {
    fn id(&self) -> String {
        self.id.to_string()
    }

    fn set_id(&mut self, uuid: String) {
        match Uuid::parse_str(uuid.as_str()) {
            Ok(uuid) =>self.id = uuid,
            Err(_) => {}
        }
    }

    fn id_mut(&mut self) -> String {
        self.id.to_string()
    }
}

impl DAWItemPosition for PluginParameter {
    fn position(&self) -> f64 {
        self.position
    }

    fn set_position(&mut self, time: f64) {
        self.position = time;
    }
}

impl PluginParameter {

    /// Get the vst plugin parameter's value.
    #[must_use]
    pub fn value(&self) -> f32 {
        self.value
    }

    /// Get the vst plugin parameter's instrument.
    #[must_use]
    pub fn instrument(&self) -> bool {
        self.instrument
    }

    /// Set the vst plugin parameter's value.
    pub fn set_value(&mut self, value: f32) {
        self.value = value;
    }

    /// Get a reference to the vst plugin parameter's plugin uuid.
    #[must_use]
    pub fn plugin_uuid(&self) -> String {
        self.plugin_uuid.to_string()
    }
}

pub trait BackgroundProcessorAudioPlugin {
    fn uuid(&self) -> Uuid;
    fn uuid_mut(&mut self) -> Uuid;

    fn name(&self) -> String;

    fn xid(&self) -> Option<u32>;
    fn set_xid(&mut self, xid: Option<u32>);
    fn xid_mut(&mut self) -> &mut Option<u32>;
    fn get_window_size(&self) -> (i32, i32);

    fn rx_from_host(&self) -> &Receiver<AudioPluginHostOutwardEvent>;
    fn rx_from_host_mut(&mut self) -> &mut Receiver<AudioPluginHostOutwardEvent>;

    fn set_tempo(&mut self, tempo: f64);
    fn tempo(&self) -> f64;

    /// Re-anchor the plugin's notion of the song position to an absolute frame position.
    /// Called on every transport move (play from a block, rewind, scrub) - plugins whose
    /// internal state is driven by the song position (arpeggiators, delays, mod LFOs)
    /// must never free-run away from the host position, or they drift further off on
    /// every play cycle and start producing garbage. Formats that get the position from
    /// the host on every process call (VST2.4) have nothing to do here.
    fn seek_transport(&mut self, _sample_position: i64) {
    }

    /// The plugin's reported group delay in frames (VST2 initialDelay / VST3
    /// getLatency / CLAP latency extension). Zero when the plugin reports none.
    fn latency_frames(&self) -> i64;

    fn stop_processing(&mut self);
    fn shutdown(&mut self);

    /// Tear down the plugin's GUI (if any) before the plugin is stopped/shut down. The
    /// plugin formats require the editor/GUI to be destroyed before the plugin instance
    /// itself: VST2 wants effMClose before effClose, CLAP wants clap_plugin_gui::destroy
    /// before clap_plugin::destroy (u-he plugins abort in destroy() while a GUI is alive).
    /// Dropping an instance with a live editor crashes the plugin.
    fn close_editor(&mut self);

    fn preset_data(&mut self) -> String;
    fn set_preset_data(&mut self, data: String);

    fn sample_rate(&self) -> f64;
    fn set_sample_rate(&mut self, sample_rate: f64);
    /// Reconfigure the plugin while it is loaded: the implementing format-specific code
    /// must follow the plugin protocols - stop/suspend processing, set the new sample
    /// rate and block size, then restart processing.
    fn change_sample_rate_and_block_size(&mut self, sample_rate: f64, block_size: i64);
    fn set_time_signature(&mut self, time_signature_numerator: u32, time_signature_denominator: u32);
}
pub enum BackgroundProcessorAudioPluginType {
    Vst24(BackgroundProcessorVst24AudioPlugin),
    Vst3(BackgroundProcessorVst3AudioPlugin),
    Clap(BackgroundProcessorClapAudioPlugin),
}

impl BackgroundProcessorAudioPlugin for BackgroundProcessorAudioPluginType {
    fn uuid(&self) -> Uuid {
        match self {
            BackgroundProcessorAudioPluginType::Vst24(vst24_plugin) => {
                vst24_plugin.uuid()
            }
            BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                vst3_plugin.uuid()
            }
            BackgroundProcessorAudioPluginType::Clap(clap_plugin) => {
                clap_plugin.uuid()
            }
        }
    }

    fn uuid_mut(&mut self) -> Uuid {
        match self {
            BackgroundProcessorAudioPluginType::Vst24(vst24_plugin) => {
                vst24_plugin.uuid_mut()
            }
            BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                vst3_plugin.uuid_mut()
            }
            BackgroundProcessorAudioPluginType::Clap(clap_plugin) => {
                clap_plugin.uuid_mut()
            }
        }
    }

    fn xid(&self) -> Option<u32> {
        match self {
            BackgroundProcessorAudioPluginType::Vst24(vst24_plugin) => {
                vst24_plugin.xid()
            }
            BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                vst3_plugin.xid()
            }
            BackgroundProcessorAudioPluginType::Clap(clap_plugin) => {
                clap_plugin.xid()
            }
        }
    }

    fn set_xid(&mut self, xid: Option<u32>) {
        match self {
            BackgroundProcessorAudioPluginType::Vst24(vst24_plugin) => {
                vst24_plugin.set_xid(xid);
            }
            BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                vst3_plugin.set_xid(xid);
            }
            BackgroundProcessorAudioPluginType::Clap(clap_plugin) => {
                clap_plugin.set_xid(xid);
            }
        }
    }

    fn xid_mut(&mut self) -> &mut Option<u32> {
        match self {
            BackgroundProcessorAudioPluginType::Vst24(vst24_plugin) => {
                vst24_plugin.xid_mut()
            }
            BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                vst3_plugin.xid_mut()
            }
            BackgroundProcessorAudioPluginType::Clap(clap_plugin) => {
                clap_plugin.xid_mut()
            }
        }
    }

    fn rx_from_host(&self) -> &Receiver<AudioPluginHostOutwardEvent> {
        match &self {
            BackgroundProcessorAudioPluginType::Vst24(vst24_plugin) => {
                vst24_plugin.rx_from_host()
            }
            BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                vst3_plugin.rx_from_host()
            }
            BackgroundProcessorAudioPluginType::Clap(clap_plugin) => {
                clap_plugin.rx_from_host()
            }
        }
    }

    fn rx_from_host_mut(&mut self) -> &mut Receiver<AudioPluginHostOutwardEvent> {
        match self {
            BackgroundProcessorAudioPluginType::Vst24(vst24_plugin) => {
                vst24_plugin.rx_from_host_mut()
            }
            BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                vst3_plugin.rx_from_host_mut()
            }
            BackgroundProcessorAudioPluginType::Clap(clap_plugin) => {
                clap_plugin.rx_from_host_mut()
            }
        }
    }

    fn latency_frames(&self) -> i64 {
        match self {
            BackgroundProcessorAudioPluginType::Vst24(vst24_plugin) => {
                vst24_plugin.latency_frames()
            }
            BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                vst3_plugin.latency_frames()
            }
            BackgroundProcessorAudioPluginType::Clap(clap_plugin) => {
                clap_plugin.latency_frames()
            }
        }
    }

    fn stop_processing(&mut self) {
        match self {
            BackgroundProcessorAudioPluginType::Vst24(vst24_plugin) => {
                vst24_plugin.stop_processing();
            }
            BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                vst3_plugin.stop_processing();
            }
            BackgroundProcessorAudioPluginType::Clap(clap_plugin) => {
                clap_plugin.stop_processing();
            }
        }
    }

    fn shutdown(&mut self) {
        match self {
            BackgroundProcessorAudioPluginType::Vst24(vst24_plugin) => {
                vst24_plugin.shutdown();
            }
            BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                vst3_plugin.shutdown();
            }
            BackgroundProcessorAudioPluginType::Clap(clap_plugin) => {
                clap_plugin.shutdown();
            }
        }
    }

    fn close_editor(&mut self) {
        match self {
            BackgroundProcessorAudioPluginType::Vst24(vst24_plugin) => {
                vst24_plugin.close_editor();
            }
            BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                vst3_plugin.close_editor();
            }
            BackgroundProcessorAudioPluginType::Clap(clap_plugin) => {
                clap_plugin.close_editor();
            }
        }
    }

    fn set_tempo(&mut self, tempo: f64) {
        match self {
            BackgroundProcessorAudioPluginType::Vst24(vst24_plugin) => {
                vst24_plugin.set_tempo(tempo);
            }
            BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                vst3_plugin.set_tempo(tempo);
            }
            BackgroundProcessorAudioPluginType::Clap(clap_plugin) => {
                clap_plugin.set_tempo(tempo);
            }
        }
    }

    fn seek_transport(&mut self, sample_position: i64) {
        match self {
            BackgroundProcessorAudioPluginType::Vst24(_vst24_plugin) => {
                // vst2.4 plugins read the host position on every process call - the
                // track loop keeps it in step with the transport block index already
            }
            BackgroundProcessorAudioPluginType::Vst3(_vst3_plugin) => {
            }
            BackgroundProcessorAudioPluginType::Clap(clap_plugin) => {
                clap_plugin.seek_transport(sample_position);
            }
        }
    }

    fn preset_data(&mut self) -> String {
        match self {
            BackgroundProcessorAudioPluginType::Vst24(vst24_plugin) => {
                vst24_plugin.preset_data()
            }
            BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                vst3_plugin.preset_data()
            }
            BackgroundProcessorAudioPluginType::Clap(clap_plugin) => {
                clap_plugin.preset_data()
            }
        }
    }

    fn set_preset_data(&mut self, data: String) {
        match self {
            BackgroundProcessorAudioPluginType::Vst24(vst24_plugin) => {
                vst24_plugin.set_preset_data(data);
            }
            BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                vst3_plugin.set_preset_data(data);
            }
            BackgroundProcessorAudioPluginType::Clap(clap_plugin) => {
                clap_plugin.set_preset_data(data);
            }
        }
    }

    fn get_window_size(&self) -> (i32, i32) {
        match self {
            BackgroundProcessorAudioPluginType::Vst24(vst24_plugin) => {
                vst24_plugin.get_window_size()
            }
            BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                vst3_plugin.get_window_size()
            }
            BackgroundProcessorAudioPluginType::Clap(clap_plugin) => {
                clap_plugin.get_window_size()
            }
        }
    }

    fn name(&self) -> String {
        match self {
            BackgroundProcessorAudioPluginType::Vst24(vst24_plugin) => {
                vst24_plugin.name()
            }
            BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                vst3_plugin.name()
            }
            BackgroundProcessorAudioPluginType::Clap(clap_plugin) => {
                clap_plugin.name()
            }
        }
    }

    fn tempo(&self) -> f64 {
        match self {
            BackgroundProcessorAudioPluginType::Vst24(vst24_plugin) => {
                vst24_plugin.tempo()
            }
            BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                vst3_plugin.tempo()
            }
            BackgroundProcessorAudioPluginType::Clap(clap_plugin) => {
                clap_plugin.tempo()
            }
        }
    }

    fn sample_rate(&self) -> f64 {
        match self {
            BackgroundProcessorAudioPluginType::Vst24(vst24_plugin) => {
                vst24_plugin.sample_rate()
            }
            BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                vst3_plugin.sample_rate()
            }
            BackgroundProcessorAudioPluginType::Clap(clap_plugin) => {
                clap_plugin.sample_rate()
            }
        }
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        match self {
            BackgroundProcessorAudioPluginType::Vst24(vst24_plugin) => {
                vst24_plugin.set_sample_rate(sample_rate);
            }
            BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                vst3_plugin.set_sample_rate(sample_rate);
            }
            BackgroundProcessorAudioPluginType::Clap(clap_plugin) => {
                clap_plugin.set_sample_rate(sample_rate);
            }
        }
    }

    fn change_sample_rate_and_block_size(&mut self, sample_rate: f64, block_size: i64) {
        match self {
            BackgroundProcessorAudioPluginType::Vst24(vst24_plugin) => {
                vst24_plugin.change_sample_rate_and_block_size(sample_rate, block_size);
            }
            BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                vst3_plugin.change_sample_rate_and_block_size(sample_rate, block_size);
            }
            BackgroundProcessorAudioPluginType::Clap(clap_plugin) => {
                clap_plugin.change_sample_rate_and_block_size(sample_rate, block_size);
            }
        }
    }

    fn set_time_signature(&mut self, time_signature_numerator: u32, time_signature_denominator: u32) {
        match self {
            BackgroundProcessorAudioPluginType::Vst24(vst24_plugin) => {
                vst24_plugin.set_time_signature(time_signature_numerator, time_signature_denominator);
            }
            BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                vst3_plugin.set_time_signature(time_signature_numerator, time_signature_denominator);
            }
            BackgroundProcessorAudioPluginType::Clap(clap_plugin) => {
                clap_plugin.set_time_signature(time_signature_numerator, time_signature_denominator);
            }
        }
    }
}

#[derive()]
pub struct BackgroundProcessorVst24AudioPlugin {
    uuid: Uuid,
    host: Arc<Mutex<VstHost>>,
    vst_plugin_instance: PluginInstance,
    midi_sender: SendEventBuffer,
    xid: Option<u32>,
    rx_from_host: Receiver<AudioPluginHostOutwardEvent>,
    editor: Option<Box<dyn Editor>>,
    vst_host_time_info: Arc<RwLock<TimeInfo>>,
    sample_rate: f64,
}

impl BackgroundProcessorAudioPlugin for BackgroundProcessorVst24AudioPlugin {

    /// Get the vst effect's xid.
    fn xid(&self) -> Option<u32> {
        self.xid
    }

    /// Set the vst effect's xid.
    fn set_xid(&mut self, xid: Option<u32>) {
        self.xid = xid;
        let vst_plugin_instance = self.vst_plugin_instance_mut();
        self.editor = vst_plugin_instance.get_editor();
        if let Some(editor) = self.editor.as_mut() {
            let (_plugin_window_width, _plugin_window_height) = editor.size();
            if let Some(xid) = self.xid.clone() {
                editor.open(xid as *mut _);
            }
        }
    }

    /// Get a mutable reference to the vst effect's xid.
    fn xid_mut(&mut self) -> &mut Option<u32> {
        &mut self.xid
    }

    /// Get a reference to the vst effect's rx from vst host.
    
    fn rx_from_host(&self) -> &Receiver<AudioPluginHostOutwardEvent> {
        &self.rx_from_host
    }

    /// Get a mutable reference to the vst effect's rx from vst host.
    
    fn rx_from_host_mut(&mut self) -> &mut Receiver<AudioPluginHostOutwardEvent> {
        &mut self.rx_from_host
    }

    /// Get the vst effect's uuid.
    
    fn uuid(&self) -> Uuid {
        self.uuid
    }

    fn uuid_mut(&mut self) -> Uuid {
        self.uuid
    }

    fn latency_frames(&self) -> i64 {
        // VST2 reports the group delay on the effect itself (AEffect.initialDelay),
        // captured into Info when the instance was created
        self.vst_plugin_instance().get_info().initial_delay.max(0) as i64
    }

    fn stop_processing(&mut self) {
        self.vst_plugin_instance_mut().stop_process();
    }

    fn shutdown(&mut self) {
        self.vst_plugin_instance_mut().suspend();
    }

    fn close_editor(&mut self) {
        // VST2 protocol: the host must close the editor (effMClose) before shutting the
        // plugin down. vst-rs never closes the editor on its own (there is no Drop impl on
        // its editor, and PluginInstance::drop dispatches effClose), so an instance
        // dropped after the user opened and closed the GUI would be torn down with the
        // editor still open, which crashes plugins.
        if let Some(editor) = self.editor_mut().as_mut() {
            if editor.is_open() {
                editor.close();
            }
        }
        self.editor = None;
        self.xid = None;
    }

    fn set_tempo(&mut self, tempo: f64) {
        if let Ok(mut host) = self.host().lock() {
            host.set_tempo(tempo);
        }
    }

    fn preset_data(&mut self) -> String {
        base64::encode(self.vst_plugin_instance_mut().get_parameter_object().get_preset_data())
    }

    fn set_preset_data(&mut self, data: String) {
        if let Ok(data) = base64::decode(data) {
            self.vst_plugin_instance_mut().get_parameter_object().load_preset_data(data.as_slice());
        }
    }

    fn get_window_size(&self) -> (i32, i32) {
        if let Some(editor) = self.editor() {
            editor.size()
        }
        else {
            (400, 300)
        }
    }

    fn name(&self) -> String {
        self.vst_plugin_instance().get_info().name
    }

    fn tempo(&self) -> f64 {
        if let Ok(host) = self.host().lock() {
            host.tempo()
        }
        else {
            140.0
        }
    }

    fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate;
        self.vst_plugin_instance_mut().set_sample_rate(sample_rate as f32);
    }

    fn change_sample_rate_and_block_size(&mut self, sample_rate: f64, block_size: i64) {
        // VST2 protocol (public.sdk/source/vst2.x/aeffeditor.h, audio plugin header):
        // the host must suspend the plugin (effMKSUSE), then set the new sample rate
        // (effSetSampleRate) and block size (effSetBlockSize) and finally resume
        // processing (effStartSuspend). The plugin keeps its state throughout - no
        // need to stop or reload it.
        {
            let instance = self.vst_plugin_instance_mut();
            instance.suspend();
            instance.set_sample_rate(sample_rate as f32);
            instance.set_block_size(block_size);
            instance.resume();
        }
        self.sample_rate = sample_rate;
        if let Ok(mut host) = self.host().lock() {
            host.set_block_size(block_size as isize);
        }
    }

    fn set_time_signature(&mut self, time_signature_numerator: u32, time_signature_denominator: u32) {
        if let Ok(mut host) = self.host().lock() {
            host.set_time_signature_numerator(time_signature_numerator);
            host.set_time_signature_denominator(time_signature_denominator);
        }
    }
}

impl BackgroundProcessorVst24AudioPlugin {
    pub fn new_with_uuid(
        vst_plugin_loaders: Arc<Mutex<HashMap<String, PluginLoader<VstHost>>>>,
        track_uuid: String,
        uuid: Uuid,
        sub_plugin_id: Option<String>,
        library_path: String,
        vst_host_time_info: Arc<RwLock<TimeInfo>>,
        sample_rate: f64,
        block_size: i64,
        tempo: f64,
        time_signature_numerator: i32,
        time_signature_denominator: i32,
    ) -> Result<Self, String> {
        let (tx_from_vst_host, rx_from_host) = channel::<AudioPluginHostOutwardEvent>();
        let (host, mut vst_plugin_instance) = create_vst24_audio_plugin(
            vst_plugin_loaders,
            library_path.as_str(),
            track_uuid,
            uuid.to_string(),
            sub_plugin_id,
            tx_from_vst_host,
            false,
            vst_host_time_info.clone(),
            sample_rate,
            block_size,
            tempo,
            time_signature_numerator,
            time_signature_denominator,
        )?;
        let midi_sender = SendEventBuffer::new(1);
        vst_plugin_instance.set_sample_rate(sample_rate as f32);
        vst_plugin_instance.set_block_size(block_size);
        // let vst_editor = vst_plugin_instance.get_editor();
        Ok(Self {
            uuid,
            host,
            vst_plugin_instance,
            midi_sender,
            xid: None,
            rx_from_host,
            editor: None,
            vst_host_time_info,
            sample_rate,
        })
    }

    /// Get a reference to the vst effect plugin's host.
    pub fn host(&self) -> Arc<Mutex<VstHost>> {
        self.host.clone()
    }

    /// Set the vst effect plugin's host.
    pub fn set_host(&mut self, host: Arc<Mutex<VstHost>>) {
        self.host = host;
    }

    /// Get a reference to the vst effect plugin's vst plugin instance.
    pub fn vst_plugin_instance(&self) -> &PluginInstance {
        &self.vst_plugin_instance
    }

    /// Set the vst effect plugin's vst plugin instance.
    pub fn set_vst_plugin_instance(&mut self, vst_plugin_instance: PluginInstance) {
        self.vst_plugin_instance = vst_plugin_instance;
    }

    /// Get a mutable reference to the vst effect's host.
    pub fn host_mut(&mut self) -> &mut Arc<Mutex<VstHost>> {
        &mut self.host
    }

    /// Get a mutable reference to the vst effect's vst plugin instance.
    pub fn vst_plugin_instance_mut(&mut self) -> &mut PluginInstance {
        &mut self.vst_plugin_instance
    }

    /// Get a reference to the vst effect's midi sender.
    pub fn midi_sender(&self) -> &SendEventBuffer {
        &self.midi_sender
    }

    /// Set the vst effect's midi sender.
    pub fn set_midi_sender(&mut self, midi_sender: SendEventBuffer) {
        self.midi_sender = midi_sender;
    }

    /// Get a mutable reference to the vst effect's midi sender.
    pub fn midi_sender_mut(&mut self) -> &mut SendEventBuffer {
        &mut self.midi_sender
    }

    /// Get a reference to the vst effect's editor.
    #[must_use]
    pub fn editor(&self) -> Option<&Box<dyn Editor>> {
        self.editor.as_ref()
    }

    /// Get a mutable reference to the vst effect's editor.
    #[must_use]
    pub fn editor_mut(&mut self) -> &mut Option<Box<dyn Editor>> {
        &mut self.editor
    }

    /// Set the vst effect's editor.
    pub fn set_editor(&mut self, editor: Option<Box<dyn Editor>>) {
        self.editor = editor;
    }
}

impl Drop for BackgroundProcessorVst24AudioPlugin {
    fn drop(&mut self) {
        // Safety net for every drop path (instrument change, effect delete, track kill):
        // the plugin instance field is dropped after this, dispatching effClose - the
        // editor must be gone (effMClose) by then or the plugin crashes.
        let _ = BackgroundProcessorAudioPlugin::close_editor(self);
    }
}


#[derive()]
/// The track background processor's handle for a CLAP plugin instance. CLAP splits a
/// plugin's use across two threads: every function except the audio ones
/// (`process`/event queues) - lifecycle, parameters, state, gui - must run on the
/// host's main thread, and plugins enforce this themselves (clap-helpers thread
/// checks answered by the host's thread-check extension; JUCE based GUIs additionally
/// deadlock when created off a real GUI message thread). The instance itself is
/// therefore owned by the application main thread (see `CLAP_REGISTRY` below); this
/// handle keeps only the raw plugin pointer, the audio-side `ProcessData` (safe for
/// the audio thread) and an RPC channel to the main thread.
pub struct BackgroundProcessorClapAudioPlugin {
    track_uuid: String,
    uuid: Uuid,
    // raw plugin handle - the instance itself is owned by the main thread registry
    // (and released via ClapInstanceRelease); only the CLAP audio-thread functions
    // (process/start/stop) are invoked through this on the track thread
    plugin: simple_clap_host_helper_lib::plugin::instance::PluginRawHandle,
    xid: Option<u32>,
    gui_created: bool,
    // parameter descriptors fetched on the main thread at creation so per-block
    // parameter events can be converted without any plugin calls
    param_info: Option<SendParamInfo>,
    tx_from_clap_host: Sender<AudioPluginHostOutwardEvent>,
    rx_from_host: Receiver<AudioPluginHostOutwardEvent>,
    process_data: ProcessData,
    host_receiver: crossbeam_channel::Receiver<DAWCallback>,
    tempo: f64,
    sample_rate: f64,
    block_size: i64,
    // the plugin's reported group delay in frames, fetched on the main thread at
    // creation (and after any re-activation) - clap latency queries are main-thread
    // functions, so the track thread carries the value instead of asking
    latency_frames: i64,
    stop_now: bool,
    param_gesture_begin: HashMap<i32, bool>,
    // outward channel to the application main thread (the RPC bus for ClapCall)
    tx_vst_thread: Sender<TrackBackgroundProcessorOutwardEvent>,
}

/// An entry of the main thread's CLAP plugin registry: the owning wrapper (keeps the
/// host-side instance registered and destroys the plugin on the main thread when
/// removed) plus the state the main thread needs to drive the GUI event loop.
struct ClapRegistryEntry {
    plugin: simple_clap_host_helper_lib::plugin::instance::Plugin,
    gui_open: bool,
}

thread_local! {
    /// CLAP plugin instances, keyed by their raw `clap_plugin` pointer. Only ever
    /// touched from the application main (GTK) thread - the thread_local makes that
    /// a hard guarantee.
    static CLAP_REGISTRY: std::cell::RefCell<HashMap<usize, ClapRegistryEntry>> = std::cell::RefCell::new(HashMap::new());
}

/// Takes ownership of a freshly created CLAP instance on the current (main) thread,
/// records which thread the track background processor runs on (so the plugin's
/// `process()` passes the CLAP audio-thread check), and returns the raw pointer the
/// track side uses as the registry key.
pub fn clap_registry_add(plugin: simple_clap_host_helper_lib::plugin::instance::Plugin, audio_thread: std::thread::ThreadId) -> usize {
    // the helper captured the creating (main) thread as the audio thread at
    // construction; correct it to the real track background thread
    plugin.state.audio_thread.store(Some(audio_thread));
    let plugin_ptr = plugin.as_ptr() as usize;
    CLAP_REGISTRY.with(|registry| registry.borrow_mut().insert(plugin_ptr, ClapRegistryEntry { plugin, gui_open: false }));
    plugin_ptr
}

/// Hands a CLAP plugin instance back to its main thread owner to be destroyed.
pub fn clap_registry_release(plugin_ptr: usize) {
    CLAP_REGISTRY.with(|registry| {
        let removed = registry.borrow_mut().remove(&plugin_ptr);
        if let Some(entry) = removed {
            // at process exit, forget the entry instead of dropping it: running
            // clap_plugin::destroy releases the plugin's shared engine module
            // (u-he shells dlclose it with the last instance), leaving other
            // still-live instances' function tables pointing into unmapped code
            // - that stale-pointer call segfaulted shutdown. The kernel reclaims
            // everything as the process exits.
            if TRACK_THREADS_SHOULD_EXIT.load(std::sync::atomic::Ordering::Relaxed) {
                std::mem::forget(entry);
            }
        }
    });
}

/// Drop every registry entry WITHOUT running `Plugin`'s destructor: used at exit
/// when a track background thread is still alive. A live thread could be inside
/// the plugin's `process()`, so stopping/deactivating/destroying the instance
/// would race it and segfault; the instances are leaked instead (the process is
/// terminating anyway, so the library state simply goes away with it).
pub fn clap_registry_leak_all() {
    CLAP_REGISTRY.with(|registry| {
        let entries: Vec<ClapRegistryEntry> = registry.borrow_mut().drain().map(|(_ptr, entry)| entry).collect();
        for entry in entries {
            std::mem::forget(entry);
        }
    });
}

/// Executes one main-thread CLAP plugin call on behalf of a track background
/// processor. Called from the application main thread's event pump.
pub fn clap_registry_execute(plugin_ptr: usize, call: ClapPluginCall) -> ClapCallReply {
    use simple_clap_host_helper_lib::plugin::ext::gui::Gui;
    use simple_clap_host_helper_lib::plugin::ext::latency::Latency;
    CLAP_REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        let entry = match registry.get_mut(&plugin_ptr) {
            Some(entry) => entry,
            None => {
                debug!("CLAP call for an unknown plugin instance {:#x}.", plugin_ptr);
                return ClapCallReply::Failed;
            }
        };
        let plugin = &entry.plugin;
        match call {
            ClapPluginCall::CreateShowGui(xid) => {
                let Some(gui) = plugin.get_extension::<Gui>() else {
                    debug!("CLAP plugin has no gui extension.");
                    return ClapCallReply::GuiResult(false, 0, 0);
                };
                if !gui.create(plugin, CLAP_WINDOW_API_X11, false) {
                    debug!("CLAP gui: create() failed.");
                    return ClapCallReply::GuiResult(false, 0, 0);
                }
                entry.gui_open = true;
                if !gui.set_scale(plugin, 1.0) {
                    debug!("Failed to successfully set the scale.");
                }
                if gui.can_resize(plugin) {
                    debug!("GIU can resize.");
                }
                else {
                    debug!("GIU can not resize.");
                }
                let window = clap_window {
                    api: CLAP_WINDOW_API_X11.as_ptr(),
                    specific: clap_window_handle { x11: xid }
                };
                if !gui.set_parent(plugin, &window) {
                    debug!("Failed to successfully call clap gui set parent function.");
                }
                if !gui.show(plugin) {
                    debug!("Failed to successfully show the plugin window.");
                }
                let (mut width, mut height) = (0u32, 0u32);
                gui.get_size(plugin, &mut width, &mut height);
                ClapCallReply::GuiResult(true, width as i32, height as i32)
            },
            ClapPluginCall::ReparentShowGui(xid) => {
                let Some(gui) = plugin.get_extension::<Gui>() else {
                    return ClapCallReply::GuiResult(false, 0, 0);
                };
                let window = clap_window {
                    api: CLAP_WINDOW_API_X11.as_ptr(),
                    specific: clap_window_handle { x11: xid }
                };
                if !gui.set_parent(plugin, &window) {
                    debug!("Failed to re-parent the clap gui onto the new plugin window.");
                }
                if !gui.show(plugin) {
                    debug!("Failed to re-show the plugin window.");
                }
                let (mut width, mut height) = (0u32, 0u32);
                gui.get_size(plugin, &mut width, &mut height);
                ClapCallReply::GuiResult(true, width as i32, height as i32)
            },
            ClapPluginCall::DestroyGui => {
                if TRACK_THREADS_SHOULD_EXIT.load(std::sync::atomic::Ordering::Relaxed) {
                    // process exit: skip all plugin calls (see Deactivate above)
                    entry.gui_open = false;
                    return ClapCallReply::Done;
                }
                if let Some(gui) = plugin.get_extension::<Gui>() {
                    let _ = gui.hide(plugin);
                    gui.destroy(plugin);
                }
                entry.gui_open = false;
                ClapCallReply::Done
            },
            ClapPluginCall::GuiSize => {
                if let Some(gui) = plugin.get_extension::<Gui>() {
                    let (mut width, mut height) = (400u32, 300u32);
                    if gui.get_size(plugin, &mut width, &mut height) {
                        return ClapCallReply::Size(width as i32, height as i32);
                    }
                }
                ClapCallReply::Size(400, 300)
            },
            ClapPluginCall::ApplyResize(width, height) => {
                let Some(gui) = plugin.get_extension::<Gui>() else {
                    return ClapCallReply::ResizeResult(width, height);
                };
                // set_size()/adjust_size() are only valid while the GUI is resizable: a
                // fixed-size GUI resizes itself and only asks the host to follow
                if !gui.can_resize(plugin) {
                    return ClapCallReply::ResizeResult(width, height);
                }
                let mut width = width;
                let mut height = height;
                if !gui.adjust_size(plugin, &mut width, &mut height) {
                    debug!("CLAP gui {}x{} resize request could not be adjusted by the plugin, applying it as requested.", width, height);
                }
                if !gui.set_size(plugin, width, height) {
                    debug!("CLAP gui rejected the requested size of {}x{}, keeping its current size.", width, height);
                    let (mut current_width, mut current_height) = (width, height);
                    if gui.get_size(plugin, &mut current_width, &mut current_height) {
                        return ClapCallReply::ResizeResult(current_width, current_height);
                    }
                }
                ClapCallReply::ResizeResult(width, height)
            },
            ClapPluginCall::Deactivate => {
                // idempotent teardown: duplicate track-side handles (own and
                // override instances sharing one CLAP instance) each ask for a
                // deactivate, but a plugin deactivated twice aborts - only
                // dispatch while the instance is actually activated. At process
                // exit, never call into the plugin at all (see the release/stop
                // guards) - just answer so the track thread can move on.
                if !TRACK_THREADS_SHOULD_EXIT.load(std::sync::atomic::Ordering::Relaxed)
                    && plugin.status() == simple_clap_host_helper_lib::plugin::instance::PluginStatus::Activated {
                    plugin.deactivate();
                }
                ClapCallReply::Done
            },
            ClapPluginCall::Activate(sample_rate, block_size) => {
                if plugin.status() != simple_clap_host_helper_lib::plugin::instance::PluginStatus::Deactivated {
                    return ClapCallReply::Done;
                }
                match plugin.activate(sample_rate, 1, block_size as usize) {
                    Ok(_) => (),
                    Err(error) => debug!("CLAP: problem reactivating the plugin with the new audio configuration: {:?}", error),
                }
                ClapCallReply::Done
            },
            ClapPluginCall::RecreateBuffers(block_size) => {
                use simple_clap_host_helper_lib::plugin::ext::audio_ports::AudioPorts;
                match plugin.get_extension::<AudioPorts>() {
                    Some(audio_ports) => match audio_ports.config(plugin) {
                        Ok(audio_ports_config) => {
                            let (input_buffers, output_buffers) = audio_ports_config.create_buffers(block_size as usize);
                            ClapCallReply::Buffers(input_buffers, output_buffers)
                        },
                        Err(error) => {
                            debug!("CLAP: problem getting the 'audio-ports' config for the new block size: {:?}", error);
                            ClapCallReply::Failed
                        }
                    },
                    None => ClapCallReply::Failed,
                }
            },
            ClapPluginCall::SavePreset => {
                use simple_clap_host_helper_lib::plugin::ext::state::State;
                match plugin.get_extension::<State>() {
                    Some(plugin_state) => match plugin_state.save(plugin) {
                        Ok(preset_data) => ClapCallReply::Preset(preset_data),
                        Err(_) => ClapCallReply::Preset(vec![]),
                    },
                    None => ClapCallReply::Preset(vec![]),
                }
            },
            ClapPluginCall::LoadPreset(preset_data) => {
                use simple_clap_host_helper_lib::plugin::ext::state::State;
                if let Some(plugin_state) = plugin.get_extension::<State>() {
                    let _ = plugin_state.load(plugin, &preset_data);
                }
                ClapCallReply::Done
            },
            ClapPluginCall::GetLatency => {
                let frames = match plugin.get_extension::<Latency>() {
                    Some(latency) => latency.get_latency(plugin) as i64,
                    None => 0,
                };
                ClapCallReply::Latency(frames)
            },
            ClapPluginCall::EnumerateParams => {
                match params_snapshot(plugin) {
                    Some((info, rows)) => ClapCallReply::Params(SendParamInfo(info), rows),
                    None => ClapCallReply::Failed,
                }
            }
        }
    })
}

/// The plugin's parameter info map plus (id, name, value) rows - fetched on the main
/// thread where the `params` extension functions may be called.
fn params_snapshot(plugin: &simple_clap_host_helper_lib::plugin::instance::Plugin) -> Option<(simple_clap_host_helper_lib::plugin::ext::params::ParamInfo, Vec<(i32, String, f32)>)> {
    use simple_clap_host_helper_lib::plugin::ext::params::Params;
    let params = plugin.get_extension::<Params>()?;
    let info = params.info(plugin).ok()?;
    let mut rows = vec![];
    for (param_id, param) in info.iter() {
        rows.push((*param_id as i32, param.name.clone(), params.get(plugin, *param_id).unwrap_or(0.0) as f32));
    }
    Some((info, rows))
}

/// Drives the GUI side of every registered CLAP plugin whose editor is open. Must be
/// called periodically from the application main thread: CLAP's timer and posix fd
/// support callbacks (and `on_main_thread` for plugin request_callback calls) belong
/// on the host's main/GUI thread - this is what keeps JUCE based plugin editors
/// painting and handling input.
pub fn clap_registry_pump() {
    use simple_clap_host_helper_lib::plugin::ext::{posix_fd_support::PosixFDSupport, timer_support::TimerSupport};
    CLAP_REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        for entry in registry.values_mut() {
            if !entry.gui_open {
                // still honour pending on_main_thread callbacks even with no gui open
                if entry.plugin.state.requested_callback.compare_exchange(true, false, std::sync::atomic::Ordering::SeqCst, std::sync::atomic::Ordering::SeqCst).is_ok() {
                    unsafe {
                        let plugin_ptr = entry.plugin.as_ptr();
                        if let Some(on_main_thread) = (*plugin_ptr).on_main_thread {
                            on_main_thread(plugin_ptr);
                        }
                    }
                }
                continue;
            }
            if let Some(timer_support) = entry.plugin.get_extension::<TimerSupport>() {
                timer_support.on_timer(&entry.plugin, 0);
            }
            if let Some(posix_fd_support) = entry.plugin.get_extension::<PosixFDSupport>() {
                posix_fd_support.on_fd(&entry.plugin, 0, 0);
            }
            if entry.plugin.state.requested_callback.compare_exchange(true, false, std::sync::atomic::Ordering::SeqCst, std::sync::atomic::Ordering::SeqCst).is_ok() {
                unsafe {
                    let plugin_ptr = entry.plugin.as_ptr();
                    if let Some(on_main_thread) = (*plugin_ptr).on_main_thread {
                        on_main_thread(plugin_ptr);
                    }
                }
            }
        }
    });
}

impl BackgroundProcessorAudioPlugin for BackgroundProcessorClapAudioPlugin {

    /// Get the vst effect's xid.
    fn xid(&self) -> Option<u32> {
        self.xid
    }

    /// Set the vst effect's xid. The GUI calls themselves run on the application main
    /// thread via ClapCall RPCs (CLAP gui functions are main-thread functions and the
    /// JUCE toolkits behind them deadlock when created off a GUI message thread).
    fn set_xid(&mut self, xid: Option<u32>) {
        let previous_xid = self.xid;
        self.xid = xid;
        let Some(parent_xid) = self.xid else {
            // No host window: tear the GUI down if one was created.
            self.destroy_gui();
            return;
        };
        if self.gui_created {
            if previous_xid == Some(parent_xid) {
                // The GUI is already alive and parented to this window id - creating it
                // again for a re-shown host window would leak it, the plugin only
                // supports one.
                return;
            }
            // The embedding window itself changed (e.g. the host window was recreated
            // for another arrangement sharing this plugin instance) - re-parent and
            // re-show the existing GUI so the new window isn't left blank.
            self.call_clap(ClapPluginCall::ReparentShowGui(parent_xid as u64));
            return;
        }
        match self.call_clap(ClapPluginCall::CreateShowGui(parent_xid as u64)) {
            ClapCallReply::GuiResult(true, _, _) => self.gui_created = true,
            _ => debug!("CLAP: the gui could not be created for plugin {}.", self.uuid),
        }
    }

    /// Get a mutable reference to the vst effect's xid.
    fn xid_mut(&mut self) -> &mut Option<u32> {
        &mut self.xid
    }

    /// Get a reference to the vst effect's rx from vst host.
    
    fn rx_from_host(&self) -> &Receiver<AudioPluginHostOutwardEvent> {
        &self.rx_from_host
    }

    /// Get a mutable reference to the vst effect's rx from vst host.
    
    fn rx_from_host_mut(&mut self) -> &mut Receiver<AudioPluginHostOutwardEvent> {
        &mut self.rx_from_host
    }

    /// Get the vst effect's uuid.
    
    fn uuid(&self) -> Uuid {
        self.uuid
    }

    fn uuid_mut(&mut self) -> Uuid {
        self.uuid
    }

    fn latency_frames(&self) -> i64 {
        self.latency_frames.max(0)
    }

    fn stop_processing(&mut self) {
        self.stop_now = true;
        // start/stop_processing are CLAP's AUDIO-thread functions - call them right
        // here (running them on the main thread trips the plugins' thread checks).
        // At process exit, skip the call entirely: other tracks' instances may have
        // already been released, and a plugin shell that sees its last instance
        // destroyed unloads its shared engine module - the remaining instances'
        // clap_plugin function tables then point into unmapped code (this
        // segfaulted on shutdown with u-he Continua sharing the AM_ engine).
        // The thread is leaving its loop anyway, so processing simply stops.
        if crate::domain::TRACK_THREADS_SHOULD_EXIT.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        self.plugin.stop_processing();
    }

    fn shutdown(&mut self) {
        // deactivate is a main-thread function; the plugin instance itself is destroyed
        // by the main thread when the registry entry is released (see the Drop impl
        // below). At process exit, never call into the plugin (see stop_processing
        // above) - the instance is leaked instead.
        if crate::domain::TRACK_THREADS_SHOULD_EXIT.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        self.call_clap(ClapPluginCall::Deactivate);
    }

    fn close_editor(&mut self) {
        // CLAP protocol: the GUI must be destroyed (clap_plugin_gui::destroy) before the
        // plugin instance itself is destroyed (clap_plugin::destroy runs when the main
        // thread drops the instance). Dropping an instance whose GUI was ever created
        // makes the plugin abort (this crashed u-he ACE when another instrument was
        // selected after closing its GUI window).
        self.xid = None;
        self.destroy_gui();
    }

    fn set_tempo(&mut self, tempo: f64) {
        self.tempo = tempo;
        self.process_data.config.tempo = tempo;
        self.process_data.transport_info.tempo = tempo;
    }

    fn seek_transport(&mut self, sample_position: i64) {
        // re-anchor the CLAP song position (the plugin's tempo synced modulation and
        // delay/arp state is driven from it) to the host transport
        self.process_data.seek_transport(sample_position);
    }

    fn preset_data(&mut self) -> String {
        match self.call_clap(ClapPluginCall::SavePreset) {
            ClapCallReply::Preset(preset_data) => base64::encode(preset_data),
            _ => "".to_string(),
        }
    }

    fn set_preset_data(&mut self, data: String) {
        if let Ok(preset_data) = base64::decode(data) {
            self.call_clap(ClapPluginCall::LoadPreset(preset_data));
        }
    }

    fn get_window_size(&self) -> (i32, i32) {
        // Once the GUI has been created, ask the plugin for its own idea of the window
        // size instead of guessing - the plugin may have requested a size (e.g. after a
        // preset change) before the host window was resized. get_size() is a main-thread
        // gui call, so it runs over the RPC.
        if self.gui_created {
            if let ClapCallReply::Size(width, height) = self.call_clap(ClapPluginCall::GuiSize) {
                return (width, height);
            }
        }
        (400, 300)
    }

    fn name(&self) -> String {
        "To be done".to_string()
    }

    fn tempo(&self) -> f64 {
        self.tempo
    }

    fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate;
        self.process_data.config.sample_rate = sample_rate;
    }

    fn change_sample_rate_and_block_size(&mut self, sample_rate: f64, block_size: i64) {
        // CLAP protocol: the audio setup (sample rate and frame sizes) is given at
        // activate time - to change it the host must stop processing, deactivate the
        // plugin and reactivate it with the new sample rate and buffer sizes. All of
        // those are main-thread functions, so they run over the RPC; only the host
        // audio buffers (owned by this thread's ProcessData) are rebuilt locally from
        // the freshly sized buffers the main thread allocates.
        use simple_clap_host_helper_lib::plugin::instance::process::{AudioBuffers, OutOfPlaceAudioBuffers};

        self.plugin.stop_processing();
        self.call_clap(ClapPluginCall::Deactivate);

        if let ClapCallReply::Buffers(input_buffers, output_buffers) = self.call_clap(ClapPluginCall::RecreateBuffers(block_size)) {
            match OutOfPlaceAudioBuffers::new(input_buffers, output_buffers) {
                Ok(audio_buffers) => self.process_data.buffers = AudioBuffers::OutOfPlace(audio_buffers),
                Err(error) => debug!("CLAP: problem recreating the audio buffers for the new block size: {:?}", error),
            }
        }

        self.process_data.config.sample_rate = sample_rate;
        self.call_clap(ClapPluginCall::Activate(sample_rate, block_size));
        self.plugin.start_processing();

        // group delay is audio-configuration dependent - re-query it on the main
        // thread so the track's latency compensation follows the reactivation
        if let ClapCallReply::Latency(frames) = self.call_clap(ClapPluginCall::GetLatency) {
            self.latency_frames = frames;
        }

        self.sample_rate = sample_rate;
        self.block_size = block_size;
    }

    fn set_time_signature(&mut self, time_signature_numerator: u32, time_signature_denominator: u32) {
        self.process_data.config.time_sig_numerator = time_signature_numerator as u16;
        self.process_data.config.time_sig_denominator = time_signature_denominator as u16;
    }
}

impl BackgroundProcessorClapAudioPlugin {
    /// Builds the track-side handle from the parts the application main thread
    /// created (the plugin instance itself stays registered with the main thread).
    /// Also fetches the parameter snapshot over the RPC so per-block parameter
    /// events never need to call into the plugin from this thread.
    #[allow(clippy::too_many_arguments)]
    pub fn new_from_parts(
        track_uuid: String,
        uuid: Uuid,
        parts: ClapInstanceParts,
        sample_rate: f64,
        block_size: i64,
        tempo: f64,
        tx_vst_thread: Sender<TrackBackgroundProcessorOutwardEvent>,
    ) -> Self {
        let (tx_from_clap_host, rx_from_host) = channel::<AudioPluginHostOutwardEvent>();
        let mut plugin = Self {
            track_uuid,
            uuid,
            plugin: parts.plugin,
            xid: None,
            gui_created: false,
            param_info: None,
            tx_from_clap_host,
            rx_from_host,
            process_data: parts.process_data,
            host_receiver: parts.host_receiver,
            tempo,
            sample_rate,
            block_size,
            latency_frames: parts.latency_frames,
            stop_now: false,
            param_gesture_begin: HashMap::new(),
            tx_vst_thread,
        };
        if let ClapCallReply::Params(info, _rows) = plugin.call_clap(ClapPluginCall::EnumerateParams) {
            plugin.param_info = Some(info);
        }
        plugin
    }

    /// Sends a main-thread CLAP call over the outward channel and blocks for the
    /// reply. The plugin's non-audio functions are main-thread only; calling them
    /// here (the realtime track thread) trips the plugins' own thread checks and,
    /// for gui functions, deadlocks their JUCE message loops.
    fn call_clap(&self, call: ClapPluginCall) -> ClapCallReply {
        // bounded(1): the crossbeam SyncSender in the event keeps the reply channel
        // Send (a std mpsc SyncSender is not), so the reply can travel over the
        // track's crossbeam outward channel
        let (tx_reply, rx_reply) = std::sync::mpsc::sync_channel(1);
        match self.tx_vst_thread.send(TrackBackgroundProcessorOutwardEvent::ClapCall(self.plugin.as_ptr() as usize, call, tx_reply)) {
            Ok(_) => (),
            Err(error) => {
                debug!("Could not send a CLAP call to the main thread for plugin {}: {:?}", self.uuid, error);
                return ClapCallReply::Failed;
            }
        }
        match rx_reply.recv_timeout(std::time::Duration::from_secs(20)) {
            Ok(reply) => reply,
            Err(_) => {
                debug!("Timed out waiting for the main thread to run a CLAP call for plugin {}.", self.uuid);
                ClapCallReply::Failed
            }
        }
    }

    pub fn process_events(&self, events: &Vec<TrackEvent>) {
        let track_clap_events = DAWUtils::convert_events_with_timing_in_frames_to_clap(events, 0);
        let mut clap_input_events = self.process_data.input_events.events.lock();
        for event in track_clap_events {
            clap_input_events.push(event);
        }
    }

    pub fn process_param_events(&self, events: &Vec<&PluginParameter>, param_info: &ParamInfo) {
        let track_clap_events = DAWUtils::convert_param_events_with_timing_in_frames_to_clap(events, 0, param_info);
        let mut clap_input_events = self.process_data.input_events.events.lock();
        for event in track_clap_events {
            clap_input_events.push(event);
        }
    }

    pub fn process(&mut self, background_processor_buffer: &mut AudioBuffer<f32>, uses_input: bool) {
        if self.stop_now {
            return;
        }
        unsafe {
            // the process() entry point is the one plugin function that runs on the
            // audio thread - call the raw vtable pointer directly (the main-thread-only
            // Plugin wrapper lives in the registry, not here)
            let clap_plugin_ptr = self.plugin.as_ptr();
            if let Some(process) = (*clap_plugin_ptr).process {
                if self.stop_now {
                    return;
                }

                if uses_input {
                    // copy the input data across
                    let audio_input_buffer = self.process_data.buffers.inputs_mut_ref();
                    let channel = &mut audio_input_buffer[0];
                    
                    let (inputs, _) = background_processor_buffer.split();
                    let background_processor_left_channel = inputs.get(0);
                    let background_processor_right_channel = inputs.get(1);
                    
                    {
                        let channel1 = &mut channel[0];
                        for index in 0..self.block_size as usize {
                            channel1[index] = background_processor_left_channel[index];
                        }
                    }

                    {
                        let channel2 = &mut channel[1];
                        for index in 0..self.block_size as usize {
                            channel2[index] = background_processor_right_channel[index];
                        }
                    }
                }

                let num_samples = self.process_data.buffers.len();
                let (inputs, outputs) = self.process_data.buffers.io_buffers();
        
                let process_data = clap_process {
                    steady_time: self.process_data.sample_pos as i64,
                    frames_count: num_samples as u32,
                    transport: &self.process_data.transport_info,
                    audio_inputs: if inputs.is_empty() {
                        std::ptr::null()
                    } else {
                        inputs.as_ptr()
                    },
                    audio_outputs: if outputs.is_empty() {
                        std::ptr::null_mut()
                    } else {
                        outputs.as_mut_ptr()
                    },
                    audio_inputs_count: inputs.len() as u32,
                    audio_outputs_count: outputs.len() as u32,
                    in_events: &self.process_data.input_events.vtable,
                    out_events: &self.process_data.output_events.vtable,
                };

                if self.stop_now {
                    return;
                }

                let process_status = process(clap_plugin_ptr, &process_data);

                // check for output events
                for event in self.process_data.output_events.events.lock().iter() {
                    match event {
                        ParamValue(param) => {
                            debug!("CLAP_EVENT_PARAM_VALUE received: {:?}", param);
                            if let Some(has_begun) = self.param_gesture_begin.get(&(param.param_id as i32)) {
                                if *has_begun {
                                    debug!("CLAP_EVENT_PARAM_VALUE sending AudioPluginHostOutwardEvent::Automation.");
                                    let _ = self.tx_from_clap_host.send(AudioPluginHostOutwardEvent::Automation(self.track_uuid.clone(), self.uuid.to_string(), false, param.param_id as i32, param.value as f32));
                                }
                            }
                        }
                        ParamGestureBegin(param_gesture) => {
                            debug!("CLAP_EVENT_PARAM_GESTURE_BEGIN received: {:?}", param_gesture);
                            self.param_gesture_begin.insert(param_gesture.param_id as i32, true);
                        }
                        ParamGestureEnd(param_gesture) => {
                            debug!("CLAP_EVENT_PARAM_GESTURE_END received: {:?}", param_gesture);
                            self.param_gesture_begin.remove(&(param_gesture.param_id as i32));
                        }
                        _ => {}
                    }
                }

                if self.stop_now {
                    return;
                }

                let (_, mut outputs) = background_processor_buffer.split();
                let background_processor_left_channel = outputs.get_mut(0);
                let background_processor_right_channel = outputs.get_mut(1);

                if process_status == clap_sys::process::CLAP_PROCESS_ERROR {
                    // CLAP_PROCESS_ERROR - the plugin has stopped or died. its output
                    // buffers are no longer being updated; copying them back would replay
                    // the last good block over and over (a machine-gunned block sounds
                    // exactly like harsh noise) - silence the track instead
                    debug!("CLAP plugin {} returned CLAP_PROCESS_ERROR from process - track output silenced.", self.uuid);
                    background_processor_left_channel.fill(0.0);
                    background_processor_right_channel.fill(0.0);
                }
                else {
                    // take the plugin's output and immediately clear it - a plugin that
                    // keeps returning a continue status but stops writing its output
                    // (died internally, gated itself to sleep) would otherwise have the
                    // same stale block copied back every cycle - the machine gunned
                    // audio that sounds like harsh noise part way through playback
                    let audio_output_buffer = self.process_data.buffers.outputs_mut_ref();
                    let channel = &mut audio_output_buffer[0];
                    let (channel1_slot, channel_rest) = channel.split_at_mut(1);
                    let channel1 = &mut channel1_slot[0];
                    let channel2 = &mut channel_rest[0];
                    for index in 0..self.block_size as usize {
                        background_processor_left_channel[index] = channel1[index];
                        background_processor_right_channel[index] = channel2[index];
                        channel1[index] = 0.0;
                        channel2[index] = 0.0;
                    }
                }
            }

            self.process_data.clear_events();
            self.process_data.advance_transport(self.block_size as u32);
        }
    }
    /// The CLAP plugin parameter snapshot fetched from the main thread at creation -
    /// used to convert parameter events on the audio thread without plugin calls.
    pub fn clap_param_info(&self) -> Option<&simple_clap_host_helper_lib::plugin::ext::params::ParamInfo> {
        self.param_info.as_ref().map(|info| &info.0)
    }



    /// Destroy the plugin GUI if one was created. Idempotent: plugins (u-he) are fine
    /// with destroy() on a never-created GUI but skipping it keeps the state honest.
    fn destroy_gui(&mut self) {
        if self.gui_created {
            self.gui_created = false;
            // at process exit never call into the plugin (a released sibling
            // instance may already have unloaded the plugin's shared engine -
            // see stop_processing above); the window and instance are reclaimed
            // with the process
            if crate::domain::TRACK_THREADS_SHOULD_EXIT.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            self.call_clap(ClapPluginCall::DestroyGui);
        }
    }

    /// The host's answer to a plugin calling `clap_host_gui::request_resize()` - a plugin
    /// uses it when it wants a different window size (preset change, tabbed GUI, ...).
    /// The CLAP protocol requires the host to actually apply the new size by calling
    /// `clap_plugin_gui::set_size()`, respecting the plugin's own resize constraints
    /// (`get_resize_hints`/`adjust_size`); resizing only the host window is not enough
    /// since X11 never resizes a parent's child windows and CLAP-conformant plugins wait
    /// for the host to confirm the size. Returns the (possibly plugin-adjusted) size so
    /// the host window can be resized to match.
    pub fn handle_gui_resize(&self, width: u32, height: u32) -> (u32, u32) {
        // request_resize() can arrive at any time - including during plugin load before
        // the host has ever created a GUI (u-he ACE does exactly this). The GUI size
        // functions are only valid on a created GUI, plugins (u-he) log an error and
        // abort when they are called on a non-existent one, so just relay the requested
        // size to the host window in that case.
        if !self.gui_created {
            debug!("CLAP gui resize request of {}x{} before the GUI was created, sizing the host window only.", width, height);
            return (width, height);
        }
        // the gui size functions are main-thread calls - apply them over the RPC and
        // take the plugin's confirmed size back
        match self.call_clap(ClapPluginCall::ApplyResize(width, height)) {
            ClapCallReply::ResizeResult(width, height) => (width, height),
            _ => (width, height),
        }
    }
}

impl Drop for BackgroundProcessorClapAudioPlugin {
    fn drop(&mut self) {
        // hand the main thread's registry entry back so the plugin can be destroyed
        // there (clap_plugin::destroy is a main-thread function and the plugin aborts
        // if the instance goes away with a live GUI - close_editor already destroyed
        // it in the explicit teardown paths). Fire and forget: the release event is
        // queued behind this instance's other calls on the same channel.
        let _ = self.tx_vst_thread.send(TrackBackgroundProcessorOutwardEvent::ClapInstanceRelease(self.plugin.as_ptr() as usize));
    }
}


#[derive()]
pub struct BackgroundProcessorVst3AudioPlugin {
    track_uuid: String,
    daw_plugin_uuid: Uuid,
    vst3_plugin_uid: String,
    library_path: String,
    xid: Option<u32>,
    tx_from_host: Sender<AudioPluginHostOutwardEvent>,
    rx_from_host: Receiver<AudioPluginHostOutwardEvent>,
    instrument: bool,
    sample_rate: f64,
    block_size: i64,
    tempo: f64,
    time_signature_numerator: i32,
    time_signature_denominator: i32,
}

impl BackgroundProcessorAudioPlugin for BackgroundProcessorVst3AudioPlugin {
    fn uuid(&self) -> Uuid {
        self.daw_plugin_uuid.clone()
    }

    fn uuid_mut(&mut self) -> Uuid {
        self.daw_plugin_uuid.clone()
    }

    fn name(&self) -> String {
        ffi::getVstPluginName(self.daw_plugin_uuid.to_string())
    }

    fn latency_frames(&self) -> i64 {
        ffi::vst3_plugin_get_latency(self.daw_plugin_uuid.to_string()).max(0) as i64
    }

    fn xid(&self) -> Option<u32> {
        self.xid.clone()
    }

    fn set_xid(&mut self, xid: Option<u32>) {
        self.xid = xid;
        if let Some(xid) = self.xid.as_ref() {
            let vst3host = Box::new(Vst3Host(self.track_uuid.clone(), self.daw_plugin_uuid.to_string(), self.instrument, self.tx_from_host.clone()));
            showPluginEditor(
                self.daw_plugin_uuid.to_string(),
                *xid,
                vst3host,
                |context: Box<Vst3Host>, new_window_width: i32, new_window_height: i32| {
                    debug!("Vst3 plugin window resize.");
                    debug!("New size: width={},  height={}", new_window_width, new_window_height);
                    match context.3.send(AudioPluginHostOutwardEvent::SizeWindow(context.0.clone(), context.1.clone(), context.2, new_window_width, new_window_height)) {
                        Ok(_) => (),
                        Err(_error) => debug!("Problem sending plugin window resize from vst3 plugin."),
                    }
                    context
                }
            );
        }
    }

    fn xid_mut(&mut self) -> &mut Option<u32> {
        &mut self.xid
    }

    fn get_window_size(&self) -> (i32, i32) {
        (
            vst3_plugin_get_window_width(self.daw_plugin_uuid.to_string()) as i32,
            ffi::vst3_plugin_get_window_height(self.daw_plugin_uuid.to_string()) as i32
        )
    }

    fn rx_from_host(&self) -> &Receiver<AudioPluginHostOutwardEvent> {
        &self.rx_from_host
    }

    fn rx_from_host_mut(&mut self) -> &mut Receiver<AudioPluginHostOutwardEvent> {
        &mut self.rx_from_host
    }

    fn set_tempo(&mut self, tempo: f64) {
        self.tempo = tempo;
        ffi::vst3_plugin_change_tempo(self.daw_plugin_uuid.to_string(), tempo);
    }

    fn tempo(&self) -> f64 {
        self.tempo
    }

    fn stop_processing(&mut self) {
        ffi::setProcessing(self.daw_plugin_uuid.to_string(), false);
    }

    fn shutdown(&mut self) {
        ffi::setProcessing(self.daw_plugin_uuid.to_string(), false);
        ffi::setActive(self.daw_plugin_uuid.to_string(), false);
        ffi::vst3_plugin_remove(self.daw_plugin_uuid.to_string());
    }

    fn close_editor(&mut self) {
        // The VST3 C++ bridge never frees the plug view (vst3_plugin_remove keeps the
        // plugin and its view alive), so no editor can outlive the instance - just forget
        // the window id here; vst3_plugin_remove (shutdown) stops the editor run loop.
        self.xid = None;
    }

    fn preset_data(&mut self) -> String {
        let mut data: [u8; 1000000] = [0; 1000000];
        let data_length = data.len() as u32;
        let bytes_written = ffi::vst3_plugin_get_preset(self.daw_plugin_uuid.to_string(), &mut data, data_length);

        if bytes_written > 0 {
            return base64::encode(&data[0..bytes_written as usize]);
        }
        "".to_string()
    }

    fn set_preset_data(&mut self, data: String) {
        if let Ok(mut data) = base64::decode(&data) {
            debug!("vst3 set_preset_data - data length={}", data.len());
            ffi::vst3_plugin_set_preset(self.daw_plugin_uuid.to_string(), &mut data);
        }
    }

    fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate;
    }

    fn change_sample_rate_and_block_size(&mut self, sample_rate: f64, block_size: i64) {
        // VST3 protocol: setProcessing(false), setActive(false), setupProcessing with a
        // new ProcessSetup carrying the sample rate and max samples per block, then
        // setActive(true), setProcessing(true) - done in the C++ bridge.
        ffi::vst3_plugin_change_sample_rate(self.daw_plugin_uuid.to_string(), sample_rate, block_size as i32);
        self.sample_rate = sample_rate;
        self.block_size = block_size;
    }

    fn set_time_signature(&mut self, time_signature_numerator: u32, time_signature_denominator: u32) {
        self.time_signature_numerator = time_signature_numerator as i32;
        self.time_signature_denominator = time_signature_denominator as i32;
        ffi::vst3_plugin_change_time_signature(self.daw_plugin_uuid.to_string(), time_signature_numerator, time_signature_denominator);
    }
}

impl BackgroundProcessorVst3AudioPlugin {
    pub fn new_with_uuid(
        track_uuid: String,
        daw_plugin_uuid: Uuid,
        vst3_plugin_uid: String,
        library_path: String,
        instrument: bool,
        sample_rate: f64,
        block_size: i64,
        tempo: f64,
        time_signature_numerator: i32,
        time_signature_denominator: i32,
    ) -> Result<Self, String> {
        let (tx_from_host, rx_from_host) = channel::<AudioPluginHostOutwardEvent>();
        let vst3host = Box::new(Vst3Host(track_uuid.clone(), daw_plugin_uuid.to_string(), instrument, tx_from_host.clone()));
        if !create_vst3_audio_plugin(
            library_path.clone(),
            daw_plugin_uuid.to_string(),
            vst3_plugin_uid.clone(),
            vst3host,
            sample_rate,
            block_size,
            tempo,
            time_signature_numerator,
            time_signature_denominator,
        ) {
            return Err(format!("Could not create an instance of VST3 plugin '{}' (uid={}) from '{}'.", daw_plugin_uuid, vst3_plugin_uid, library_path));
        }
        Ok(Self {
            track_uuid: track_uuid.clone(),
            daw_plugin_uuid: daw_plugin_uuid.clone(),
            vst3_plugin_uid,
            library_path,
            xid: None,
            tx_from_host: tx_from_host.clone(),
            rx_from_host,
            instrument,
            sample_rate,
            block_size,
            tempo,
            time_signature_numerator,
            time_signature_denominator,
        })
    }

    pub fn repaint(&self) {
        ffi::vst3_plugin_get_window_refresh(self.daw_plugin_uuid.to_string());
    }

    pub fn process_events(&self, events: &Vec<TrackEvent>) {
        for event in events {
            match event {
                TrackEvent::ActiveSense(_) => {}
                TrackEvent::AfterTouch(after_touch) => {
                    debug!("After touch: value={}", after_touch.value());
                    ffi::addEvent(self.daw_plugin_uuid.to_string(), ffi::EventType::AfterTouch, event.position() as i32, after_touch.value() as u32, 0, 0, 0.0);
                }
                TrackEvent::ProgramChange(program_change) => {
                    debug!("Program change: program={}", program_change.program());
                    ffi::addEvent(self.daw_plugin_uuid.to_string(), ffi::EventType::ProgramChange, event.position() as i32, program_change.program() as u32, 0, 0, 0.0);
                }
                TrackEvent::Note(_) => {}
                TrackEvent::NoteOn(note) => {
                    debug!("Note on: note={}, velocity={}", note.note, note.velocity);
                    ffi::addEvent(self.daw_plugin_uuid.to_string(), ffi::EventType::NoteOn, event.position() as i32, note.note as u32, note.velocity as u32, note.note_id(), 0.0);
                }
                TrackEvent::NoteOff(note) => {
                    debug!("Note off: note={}, velocity={}", note.note, note.velocity);
                    ffi::addEvent(self.daw_plugin_uuid.to_string(), ffi::EventType::NoteOff, event.position() as i32, note.note as u32, note.velocity as u32, note.note_id(), 0.0);
                }
                TrackEvent::NoteExpression(note_expression) => {
                    debug!("NoteExpression: type={}, note_id={}, value={}", note_expression.expression_type().clone() as i32, note_expression.note_id(), note_expression.value());
                    ffi::addEvent(self.daw_plugin_uuid.to_string(), ffi::EventType::NoteExpression, event.position() as i32, note_expression.expression_type().clone() as u32, 0, note_expression.note_id(), note_expression.value());
                }
                TrackEvent::Controller(controller) => {
                    debug!("Controller: type={}, value={}", controller.controller(), controller.value());
                    ffi::addEvent(self.daw_plugin_uuid.to_string(), ffi::EventType::Controller, event.position() as i32, controller.controller() as u32, controller.value() as u32, 0, 0.0);
                }
                TrackEvent::PitchBend(pitch_bend) => {
                    debug!("Pitch Bend: value={}", pitch_bend.value());
                    ffi::addEvent(self.daw_plugin_uuid.to_string(), ffi::EventType::PitchBend, event.position() as i32, 0, 0, pitch_bend.value(), 0.0);
                }
                TrackEvent::KeyPressure(key_pressure) => {
                    debug!("Key pressure: note={}, value={}", key_pressure.note(), key_pressure.value());
                    ffi::addEvent(self.daw_plugin_uuid.to_string(), ffi::EventType::KeyPressureAfterTouch, event.position() as i32, key_pressure.note() as u32, key_pressure.value() as u32, 0, 0.0);
                }
                TrackEvent::AudioPluginParameter(_) => {}
                TrackEvent::Sample(_) => {}
                TrackEvent::Measure(_) => {}
                // tempo has no vst3 event type - it is applied to the plugin's
                // process context time info by the track event processing
                TrackEvent::Tempo(_) => {}
            }
        }
    }

    pub fn process_param_events(&self, events: &Vec<&PluginParameter>) {
        for event in events {
            debug!("Parameter: id={}, value={}", event.index, event.value);
            ffi::addEvent(self.daw_plugin_uuid.to_string(), ffi::EventType::Parameter, event.position() as i32, event.index as u32, (event.value * 127.0) as u32, 0, 0.0);
        }
    }

    pub fn process(&mut self, background_processor_buffer: &mut AudioBuffer<f32>) {
        let (input_buffer, mut output_buffer) = background_processor_buffer.split();
        let channel1input_buffer = input_buffer.get(0);
        let channel2input_buffer = input_buffer.get(1);
        let channel1output_buffer = output_buffer.get_mut(0);
        let channel2output_buffer = output_buffer.get_mut(1);
        let processed = ffi::vst3_plugin_process(self.daw_plugin_uuid.to_string(), channel1input_buffer, channel2input_buffer, channel1output_buffer, channel2output_buffer);
        if !processed {
            // the plugin instance is gone or its process failed - the output slices
            // were left untouched, copying them on would replay the previous block
            // every cycle; silence them instead
            debug!("VST3 plugin {} process failed - track output silenced.", self.daw_plugin_uuid);
            let (_, mut output_buffer) = background_processor_buffer.split();
            output_buffer.get_mut(0).fill(0.0);
            output_buffer.get_mut(1).fill(0.0);
        }
    }

    pub fn get_parameter_count(&self) -> i32 {
        ffi::vst3_plugin_get_parameter_count(self.daw_plugin_uuid.to_string())
    }

    pub fn get_parameter(&self, index: i32) -> (i32, String, Uuid, String, String, f32, String) {
        let mut id: u32 = 0;
        let mut title: [u16; 128] = [0; 128];
        let mut short_title: [u16; 128] = [0; 128];
        let mut units: [u16; 128] = [0; 128];
        let mut step_count = 0;
        let mut default_normalised_value = 0.0;
        let mut unit_id = 0;
        let mut flags = 0;

        ffi::vst3_plugin_get_parameter_info(self.daw_plugin_uuid.to_string(), index, &mut id, &mut title, &mut short_title, &mut units, &mut step_count, &mut default_normalised_value, &mut unit_id, &mut flags);
        let converted_title = Self::convert_vst3_string128(title);
        let converted_short_title = Self::convert_vst3_string128(short_title);
        let converted_units = Self::convert_vst3_string128(units);
        // if index < 128 {
        //     debug!("Parameter: index={} ,id={} ,title={} ,short title={} ,units={} ,step_count={} ,default normalised value={} ,unit id={} ,flags={}", index, id, converted_title, converted_short_title, converted_units, step_count, default_normalised_value, unit_id, flags);
        // }
        (id as i32, self.track_uuid.clone(), self.daw_plugin_uuid.clone(), converted_title, converted_short_title, 1.0, converted_units)
    }

    fn convert_vst3_string128(vst3_string128: [u16; 128]) -> String {
        let mut u16_string = U16String::from_vec(vst3_string128);
        for (pos, result) in u16_string.chars().enumerate() {
            if let Ok(character) = result {
                if character == '\0' {
                    u16_string.truncate(pos);
                    break;
                }
            }
        }

        return if let Ok(converted_string) = u16_string.to_string() {
            converted_string
        } else {
            "Can't convert".to_string()
        }
    }
}

#[derive(Serialize, Deserialize, Clone, EnumString, Display)]
pub enum AudioPluginType {
    VST24,
    VST3,
    CLAP,
}

#[derive(Serialize, Deserialize)]
pub enum AudioPluginCategory {
    Synthesizer,
    Effect,
    MidiGenerator
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AudioPlugin {
    uuid: Uuid,
	name: String,
    descriptive_name: String,
    format: String,
    category: String,
    manufacturer: String,
    version: String,
    file: String,
    uid: String,
    is_instrument: bool,
    file_time: String,
    info_update_time: String,
    num_inputs: i32,
    num_outputs: i32,
    plugin_type: String,
    sub_plugin_id: Option<String>,
    preset_data: String,
}

impl AudioPlugin {
	pub fn new() -> AudioPlugin {
		AudioPlugin {
            uuid: Uuid::new_v4(),
			name: String::from("Unknown"),
			descriptive_name: String::from("Unknown"),
			format: String::from("Unknown"),
			category: String::from("Unknown"),
			manufacturer: String::from("Unknown"),
			version: String::from("Unknown"),
			file: String::from("Unknown"),
			uid: String::from("Unknown"),
			is_instrument: false,
			file_time: String::from("Unknown"),
			info_update_time: String::from("Unknown"),
			num_inputs: 0,
			num_outputs: 0,
			plugin_type: String::from("Unknown"),
			sub_plugin_id: None,
            preset_data: String::from(""),
		}
	}

	pub fn new_with_uuid(uuid: Uuid, name: String, file: String, sub_plugin_id: Option<String>, plugin_type: String) -> AudioPlugin {
		AudioPlugin {
            uuid,
			name,
			descriptive_name: String::from("Unknown"),
			format: String::from("Unknown"),
			category: String::from("Unknown"),
			manufacturer: String::from("Unknown"),
			version: String::from("Unknown"),
			file,
			uid: String::from("Unknown"),
			is_instrument: false,
			file_time: String::from("Unknown"),
			info_update_time: String::from("Unknown"),
			num_inputs: 0,
			num_outputs: 0,
			plugin_type,
			sub_plugin_id,
            preset_data: String::from(""),
		}
	}

    pub fn set_file(&mut self, file: String) {
        self.file = file;
    }

    /// Rebuild a plugin load descriptor from a persisted AudioPlugin domain object, so
    /// an arrangement's instrument/effects overrides (and undo/redo) can send the track
    /// background thread its plugin load message from the graph data alone.
    pub fn as_scanned_plugin(&self) -> Option<ScannedPlugin> {
        use std::str::FromStr;
        match AudioPluginType::from_str(self.plugin_type.as_str()) {
            Ok(audio_plugin_stack) => Some(ScannedPlugin {
                name: self.name.clone(),
                path: self.file.clone(),
                id: self.uid.clone(),
                sub_id: self.sub_plugin_id.clone(),
                audio_plugin_stack,
            }),
            Err(_) => None,
        }
    }

    pub fn file(&self) -> &str {
        self.file.as_ref()
    }

    /// Get a reference to the vst audio plugin's preset data.
    pub fn preset_data(&self) -> &str {
        self.preset_data.as_ref()
    }

    /// Set the vst audio plugin's preset data.
    pub fn set_preset_data(&mut self, preset_data: String) {
        self.preset_data = preset_data;
    }

    /// Get a mutable reference to the vst audio plugin's preset data.
    pub fn preset_data_mut(&mut self) -> &mut String {
        &mut self.preset_data
    }

    /// Set the vst audio plugin's sub plugin id.
    pub fn set_sub_plugin_id(&mut self, sub_plugin_id: Option<String>) {
        self.sub_plugin_id = sub_plugin_id;
    }

    /// Get a reference to the vst audio plugin's sub plugin id.
    #[must_use]
    pub fn sub_plugin_id(&self) -> &Option<String> {
        &self.sub_plugin_id
    }

    /// Get a reference to the instrument track's uuid.
    pub fn uuid(&self) -> Uuid {
        self.uuid
    }

    /// Get a mutable reference to the instrument track's uuid.
    pub fn uuid_mut(&mut self) -> &mut Uuid {
        &mut self.uuid
    }

    /// Set the instrument track's uuid.
    pub fn set_uuid(&mut self, uuid: Uuid) {
        self.uuid = uuid;
    }

    /// Get a reference to the vst audio plugin's name.
    #[must_use]
    pub fn name(&self) -> &str {
        self.name.as_ref()
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }

    pub fn plugin_type(&self) -> &str {
        self.plugin_type.as_ref()
    }

    pub fn plugin_type_mut(&mut self) -> &mut String {
        &mut self.plugin_type
    }

    pub fn set_plugin_type(&mut self, plugin_type: String) {
        self.plugin_type = plugin_type;
    }

    pub fn uid(&self) -> &str {
        &self.uid
    }

    pub fn set_uid(&mut self, uid: String) {
        self.uid = uid;
    }

    /// Get the vst audio plugin's instrument flag.
    pub fn is_instrument(&self) -> bool {
        self.is_instrument
    }

    /// Set the vst audio plugin's instrument flag.
    pub fn set_is_instrument(&mut self, is_instrument: bool) {
        self.is_instrument = is_instrument;
    }

    /// Get a reference to the vst audio plugin's format.
    pub fn format(&self) -> &str {
        self.format.as_ref()
    }

    /// Set the vst audio plugin's format.
    pub fn set_format(&mut self, format: String) {
        self.format = format;
    }

    /// Get a reference to the vst audio plugin's descriptive name.
    pub fn descriptive_name(&self) -> &str {
        self.descriptive_name.as_ref()
    }

    /// Set the vst audio plugin's descriptive name.
    pub fn set_descriptive_name(&mut self, descriptive_name: String) {
        self.descriptive_name = descriptive_name;
    }

    /// Get a reference to the vst audio plugin's category.
    pub fn category(&self) -> &str {
        self.category.as_ref()
    }

    /// Set the vst audio plugin's category.
    pub fn set_category(&mut self, category: String) {
        self.category = category;
    }

    /// Get a reference to the vst audio plugin's manufacturer.
    pub fn manufacturer(&self) -> &str {
        self.manufacturer.as_ref()
    }

    /// Set the vst audio plugin's manufacturer.
    pub fn set_manufacturer(&mut self, manufacturer: String) {
        self.manufacturer = manufacturer;
    }

    /// Get a reference to the vst audio plugin's version.
    pub fn version(&self) -> &str {
        self.version.as_ref()
    }

    /// Set the vst audio plugin's version.
    pub fn set_version(&mut self, version: String) {
        self.version = version;
    }
}

#[derive(PartialEq, Eq)]
pub enum TrackBackgroundProcessorMode {
    AudioOut,
    Coast,
    Render
}

/// Set the FTZ (flush-to-zero) and DAZ (denormals-are-zero) bits in this thread's MXCSR.
/// Plugin reverb/delay tails decay towards zero and spend minutes drifting through the
/// denormal range - x86 FP costs for denormals are enormous and every track thread and
/// the jack callback burning that CPU starves the audio cycle (jack overruns and re-
/// outputs the last buffer - the whole audio layer comes out as repeating noise). The
/// flags are per thread, so every audio thread must set them for itself.
pub fn flush_denormals_to_zero() {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        let csr = std::arch::x86_64::_mm_getcsr();
        std::arch::x86_64::_mm_setcsr(csr | 0x8040); // FTZ bit 15 | DAZ bit 6
    }
}

/// The scheduling priority of the thread that runs the jack process callback: 0 until
/// the first jack cycle has run, -1 when jack is not running under real time
/// scheduling, otherwise the crossplatform priority value jack's worker has.
pub static JACK_THREAD_PRIORITY: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Set once by the application main thread when the process is about to exit.
/// The track background processor threads' blocking ring-buffer writes check
/// this: at exit the downstream consumer (the jack callback) is already gone,
/// so a plain `write_blocking` would park a quiescent track thread forever -
/// it would never reach its `keep_alive` loop exit, and the plugin would still
/// be `Processing` when the main thread destroys the CLAP instances (which is
/// what trips the plugins' thread checks and segfaults on shutdown).
pub static TRACK_THREADS_SHOULD_EXIT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Number of live track background processor threads. `main()` waits for this
/// to reach zero after sending the `Kill` events so plugin instances are only
/// destroyed once no thread can possibly be inside `process()` anymore.
/// Wall-clock milliseconds of the jack callback's last process cycle, stored
/// from the realtime thread (one relaxed store per cycle, vDSO clock read only).
/// The UI pump's stalled-client watchdog watches it: jackd can silently take a
/// repeatedly overrunning client out of the cycle graph without ever sending
/// the shutdown notification - the device then keeps replaying the last
/// submitted block and nothing recovers on its own. Playback running with no
/// cycles for a while means exactly that, so the DAW has to notice and
/// re-trigger the client restart itself.
pub static JACK_LAST_CYCLE_MILLIS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Rate-limits the watchdog's restart requests so a genuinely stopped server
/// is not hammered at the UI pump's 125Hz tick rate.
pub static LAST_JACK_RESTART_TRIGGER_MILLIS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Consecutive watchdog restarts that have NOT brought the cycle stream back
/// (reset whenever a heartbeat arrives). Past a few attempts the client
/// restarts are proven useless and the only remaining cause is the jack
/// server itself wedging its audio driver loop - the watchdog then backs off
/// and surfaces that diagnosis instead of hammering the server.
pub static JACK_STALLED_RESTART_ATTEMPTS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Wall-clock ms until which stall detection is paused after a client rebuild
/// (a rebuild deliberately closes and reopens the connection - the server
/// needs time to re-wire the graph before the callback runs again, and the
/// heartbeat is stale by design during that window; detecting it as a fresh
/// stall fed a restart storm where every rebuild's graph churn caused another
/// xrun wave and another rebuild).
pub static JACK_RESTART_GRACE_UNTIL_MILLIS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Start of the current uninterrupted-stability window (0 when not stable
/// yet). The stall escalation is only cleared after the cycles have run
/// continuously for 15s - a rebuild that cycles for a second before stalling
/// again must not re-arm unlimited restarts.
pub static JACK_STABLE_CYCLES_SINCE_MILLIS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

static LIVE_TRACK_THREAD_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub fn live_track_thread_count() -> usize {
    LIVE_TRACK_THREAD_COUNT.load(std::sync::atomic::Ordering::SeqCst)
}

/// Keeps the track-thread liveness count incremented for as long as the guard
/// exists - created first in the thread closure, dropped last (after the
/// helper and its plugins have been released and their release events queued).
pub struct TrackThreadLivenessGuard;

impl TrackThreadLivenessGuard {
    pub fn new() -> Self {
        LIVE_TRACK_THREAD_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        TrackThreadLivenessGuard
    }
}

impl Drop for TrackThreadLivenessGuard {
    fn drop(&mut self) {
        LIVE_TRACK_THREAD_COUNT.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// `write_blocking` that gives up when the application is exiting, so a track
/// thread waiting on a vanished consumer still reaches its loop-exit check.
fn write_blocking_or_exit<T, P: rb::RbProducer<T>>(producer: &P, data: &[T]) {
    loop {
        if TRACK_THREADS_SHOULD_EXIT.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        match producer.write_blocking_timeout(data, Duration::from_millis(50)) {
            Ok(_) => return,
            Err(rb::RbError::TimedOut) => continue,
            Err(_) => return,
        }
    }
}

/// Reconcile this track thread's scheduling priority with the jack callback thread's.
/// The track threads are spawned at a very high real time priority (95) - when that is
/// at or above the jack worker thread's priority (jackd's default is around 10) the
/// track threads preempt the jack cycle itself as soon as the mix gets dense: jack
/// misses its deadlines, jackd times the client out and stops calling it - the
/// transport freezes (stop does not stop the glitching, play does not restart, time
/// does not advance) while the sound card replays the last buffer forever. A track
/// thread must stay strictly below the jack callback thread. Returns true once
/// reconciled (the caller only has to ask until this says true).
pub fn reconcile_track_thread_priority() -> bool {
    use std::sync::atomic::Ordering;
    let jack_priority = JACK_THREAD_PRIORITY.load(Ordering::Relaxed);
    if jack_priority == 0 {
        return false;
    }
    match thread_priority::get_current_thread_priority() {
        Ok(ThreadPriority::Crossplatform(current)) => {
            let current: u8 = current.into();
            if jack_priority < 0 {
                // jack is running without real time scheduling - a FIFO priority track
                // thread would starve the whole session, drop to normal scheduling
                let result = thread_priority::set_thread_priority_and_policy(
                    thread_priority::thread_native_id(),
                    ThreadPriority::Min,
                    ThreadSchedulePolicy::Normal(NormalThreadSchedulePolicy::Normal));
                if result.is_ok() {
                    debug!("Track thread dropped to normal scheduling (jack callback thread is not real time).");
                }
            }
            else if (current as i32) >= jack_priority {
                if jack_priority > 1 {
                    // stay strictly below the jack callback thread: preempting jack is
                    // what starves the cycle and gets the client timed out
                    let target = (jack_priority - 1).max(1) as u8;
                    if let Ok(value) = thread_priority::ThreadPriorityValue::try_from(target) {
                        if thread_priority::set_current_thread_priority(ThreadPriority::Crossplatform(value)).is_ok() {
                            debug!("Track thread scheduling priority dropped from {} to {} to stay below the jack callback thread ({}).", current, target, jack_priority);
                        }
                    }
                }
                else {
                    // jack holds only FIFO priority 1 - there is no lower real time slot,
                    // so drop out of real time scheduling altogether
                    let _ = thread_priority::set_thread_priority_and_policy(
                        thread_priority::thread_native_id(),
                        ThreadPriority::Min,
                        ThreadSchedulePolicy::Normal(NormalThreadSchedulePolicy::Normal));
                    debug!("Track thread dropped to normal scheduling (jack callback thread at FIFO priority {}).", jack_priority);
                }
            }
            true
        }
        _ => true,
    }
}

pub struct TrackBackgroundProcessorHelper {
    pub track_uuid: String,
    pub vst_event_blocks: Option<Vec<Vec<MidiEvent>>>,
    pub vst_event_blocks_transition_to: Option<Vec<Vec<MidiEvent>>>,
    pub jack_midi_out_immediate_events: Vec<MidiEvent>,
    pub mute: bool,
    pub midi_sender: SendEventBuffer,
    pub instrument_plugin_initial_delay: i32,
    // the inherent presentation delay last reported to the jack layer for this track
    pub last_sent_presentation_inherent_blocks: i32,
    pub instrument_plugin_instances: Vec<BackgroundProcessorAudioPluginType>,
    pub pending_preset_response_sender: Option<Sender<TrackPluginPresetResponse>>,
    pub effect_plugin_instances: Vec<BackgroundProcessorAudioPluginType>,
    // riff arrangement override plugin instances - switched in by the SetOverride*Active
    // events when a riff arrangement with instrument/effects overrides enabled plays
    pub override_instrument_plugin_instances: Vec<BackgroundProcessorAudioPluginType>,
    pub override_effect_plugin_instances: Vec<BackgroundProcessorAudioPluginType>,
    pub override_instrument_active: bool,
    pub override_effects_active: bool,
    pub vst_editor: Option<Box<dyn Editor>>,
    pub vst_effect_editors: HashMap<String, Box<dyn Editor>>,
    pub request_effect_params: bool,
    pub request_effect_params_for_uuid: String,
    // override effect plugin uuids whose parameter details still have to be reported
    // back after a SetOverrideEffects load (one request per poll, like the single
    // request_effect_params slot above)
    pub pending_override_effect_param_requests: Vec<String>,
    pub tx_audio: crossbeam_channel::Sender<AudioLayerInwardEvent>,
    pub rx_vst_thread: Receiver<TrackBackgroundProcessorInwardEvent>,
    pub tx_vst_thread: Sender<TrackBackgroundProcessorOutwardEvent>,
    pub track_thread_coast: Arc<Mutex<TrackBackgroundProcessorMode>>,
    pub keep_alive: bool,
    pub jack_midi_out_buffer: [(u32, u8, u8, u8, bool); EVENT_BUFFER_SIZE],
    pub volume: f32,
    pub pan: f32,
    sample: Option<SampleData>, // might need sample references - each is tied to a midi note number and started and stopped by note on and off messages
    pub sample_current_frame: i32,
    pub sample_is_playing: bool,
    pub track_type: GeneralTrackType,
    pub vst_host_time_info: Arc<RwLock<TimeInfo>>,
    pub track_events_inward_routings: HashMap<String, TrackEventRouting>,
    pub track_events_inward_consumers: HashMap<String, Consumer<TrackEvent>>,
    pub track_events_outward_routings: HashMap<String, TrackEventRouting>,
    pub track_events_outward_ring_buffers: HashMap<String, SpscRb<TrackEvent>>,
    pub track_events_outward_producers: HashMap<String, Producer<TrackEvent>>,

    pub audio_inward_routings: HashMap<String, AudioRouting>,
    pub audio_inward_consumers: HashMap<String, (Consumer<f32>, Consumer<f32>)>,
    pub audio_outward_routings: HashMap<String, AudioRouting>,
    pub audio_outward_ring_buffers: HashMap<String, (SpscRb<f32>, SpscRb<f32>)>,
    pub audio_outward_producers: HashMap<String, (Producer<f32>, Producer<f32>)>,

    pub event_processor: Box<dyn TrackEventProcessor>,

    pub sample_rate: f64,
    pub block_size: f64,
    pub tempo: f64,
    pub time_signature_numerator: i32,
    pub time_signature_denominator: i32,
}

impl TrackBackgroundProcessorHelper {
    pub fn new(track_uuid: String,
               tx_audio: crossbeam_channel::Sender<AudioLayerInwardEvent>,
               rx_vst_thread: Receiver<TrackBackgroundProcessorInwardEvent>,
               tx_vst_thread: Sender<TrackBackgroundProcessorOutwardEvent>,
               track_thread_coast: Arc<Mutex<TrackBackgroundProcessorMode>>,
               volume: f32,
               pan: f32,
               track_type: GeneralTrackType,
               vst_host_time_info: Arc<RwLock<TimeInfo>>,
               event_processor: Box<dyn TrackEventProcessor>,
               sample_rate: f64,
               block_size: f64,
               tempo: f64,
               time_signature_numerator: i32,
               time_signature_denominator: i32,
    ) -> Self {
        Self {
            track_uuid,
            vst_event_blocks: None,
            vst_event_blocks_transition_to: None,
            jack_midi_out_immediate_events: vec![],
            mute: false,
            midi_sender: SendEventBuffer::new(EVENT_BUFFER_SIZE),
            instrument_plugin_initial_delay: 0,
            last_sent_presentation_inherent_blocks: 0,
            instrument_plugin_instances: vec![],
            pending_preset_response_sender: None,
            effect_plugin_instances: vec![],
            override_instrument_plugin_instances: vec![],
            override_effect_plugin_instances: vec![],
            override_instrument_active: false,
            override_effects_active: false,
            vst_editor: None,
            vst_effect_editors: HashMap::new(),
            request_effect_params: false,
            request_effect_params_for_uuid: "".to_string(),
            pending_override_effect_param_requests: vec![],
            tx_audio,
            rx_vst_thread,
            tx_vst_thread,
            track_thread_coast,
            keep_alive: true,
            jack_midi_out_buffer: [(0, 0, 0, 0, false); EVENT_BUFFER_SIZE],
            volume,
            pan,
            sample: None,
            sample_current_frame: 0,
            sample_is_playing: false,
            track_type,
            vst_host_time_info,
            track_events_inward_routings: HashMap::new(),
            track_events_inward_consumers: HashMap::new(),
            track_events_outward_routings: HashMap::new(),
            track_events_outward_ring_buffers: HashMap::new(),
            track_events_outward_producers: HashMap::new(),
            audio_inward_routings: HashMap::new(),
            audio_inward_consumers: HashMap::new(),
            audio_outward_routings: HashMap::new(),
            audio_outward_ring_buffers: HashMap::new(),
            audio_outward_producers: HashMap::new(),

            event_processor,

            sample_rate,
            block_size,
            tempo,
            time_signature_numerator,
            time_signature_denominator,
        }
    }

    /// Instantiate a live plugin instance for this track thread - shared by the
    /// track's own instrument/effects load events and the riff arrangement
    /// instrument/effects override load events.
    /// Creates a CLAP plugin instance on the application main thread (the plugin
    /// binds its GUI toolkit and main-thread checks to the creating thread) and
    /// wraps the returned audio-side parts in a track-side handle. Blocks until the
    /// main thread's event pump has loaded the plugin (a cold plugin library load
    /// can take a moment).
    fn create_clap_audio_plugin_instance(&self, uuid: Uuid, scanned_plugin: &ScannedPlugin) -> Result<BackgroundProcessorClapAudioPlugin, String> {
        let (tx_reply, rx_reply) = std::sync::mpsc::sync_channel(1);
        match self.tx_vst_thread.send(TrackBackgroundProcessorOutwardEvent::ClapInstanceCreate(self.track_uuid.clone(), uuid, scanned_plugin.path.clone(), clap_plugin_id(scanned_plugin), std::thread::current().id(), self.sample_rate, self.block_size as i64, self.tempo, self.time_signature_numerator, self.time_signature_denominator, tx_reply)) {
            Ok(_) => (),
            Err(error) => return Err(format!("Could not send the CLAP instance creation request to the main thread: {error:?}")),
        }
        let parts = match rx_reply.recv_timeout(std::time::Duration::from_secs(60)) {
            Ok(Ok(parts)) => parts,
            Ok(Err(error)) => return Err(error),
            Err(_) => return Err("Timed out waiting for the main thread to create the CLAP plugin instance.".to_string()),
        };
        Ok(BackgroundProcessorClapAudioPlugin::new_from_parts(self.track_uuid.clone(), uuid, parts, self.sample_rate, self.block_size as i64, self.tempo, self.tx_vst_thread.clone()))
    }

    pub fn create_audio_plugin_instance(
        &self,
        vst24_plugin_loaders: Arc<Mutex<HashMap<String, PluginLoader<VstHost>>>>,
        clap_plugin_loaders: Arc<Mutex<HashMap<String, PluginLibrary>>>,
        uuid: Uuid,
        scanned_plugin: &ScannedPlugin,
        is_instrument: bool,
    ) -> Result<BackgroundProcessorAudioPluginType, String> {
        if let AudioPluginType::VST24 = scanned_plugin.audio_plugin_stack {
            BackgroundProcessorVst24AudioPlugin::new_with_uuid(
                vst24_plugin_loaders,
                self.track_uuid.clone(),
                uuid,
                scanned_plugin.sub_id.clone(),
                scanned_plugin.path.clone(),
                self.vst_host_time_info.clone(),
                self.sample_rate,
                self.block_size as i64,
                self.tempo,
                self.time_signature_numerator,
                self.time_signature_denominator,
            ).map(BackgroundProcessorAudioPluginType::Vst24)
        }
        else if let AudioPluginType::CLAP = scanned_plugin.audio_plugin_stack {
            self.create_clap_audio_plugin_instance(uuid, scanned_plugin).map(BackgroundProcessorAudioPluginType::Clap)
        }
        else {
            BackgroundProcessorVst3AudioPlugin::new_with_uuid(
                self.track_uuid.clone(),
                uuid,
                scanned_plugin.id.clone(),
                scanned_plugin.path.clone(),
                is_instrument,
                self.sample_rate,
                self.block_size as i64,
                self.tempo,
                self.time_signature_numerator,
                self.time_signature_denominator,
            ).map(BackgroundProcessorAudioPluginType::Vst3)
        }
    }

    pub fn handle_incoming_events(&mut self) -> bool {
        // drain all pending messages (the old single try_recv meant each queued event
        // could wait one audio block (~46ms) behind the next - badly delayed presets,
        // routings and transport changes when several arrive in a burst).
        // returns whether any message was consumed so callers can detect activity.
        let mut had_events = false;
        loop {
        match self.rx_vst_thread.try_recv() {
            Ok(message) => {
                had_events = true;
                match message {
                TrackBackgroundProcessorInwardEvent::SetEventProcessorType(event_processor_type) => {
                    match event_processor_type {
                        EventProcessorType::RiffBufferEventProcessor => {
                            self.event_processor = Box::new(RiffBufferTrackEventProcessor::new(self.block_size));
                        }
                        EventProcessorType::BlockEventProcessor => {
                            self.event_processor = Box::new(BlockBufferTrackEventProcessor::new());
                        }
                    }
                }
                TrackBackgroundProcessorInwardEvent::SetEvents((event_blocks, param_event_blocks), transition_to) => {
                    let mut event_count = 0;
                    for event_block in event_blocks.iter() {
                        event_count += event_block.len();
                    }
                    debug!("Received Audio Plugin ThreadEvent::SetEvents(event_blocks): event block count={}, event count={}", event_blocks.len(), event_count);
                    if self.event_processor.track_event_blocks().is_none() {
                        self.event_processor.set_track_event_blocks(Some(event_blocks));
                    }
                    else if transition_to {
                        self.event_processor.set_track_event_blocks_transition_to(Some(event_blocks));
                    }
                    else {
                        self.event_processor.set_track_event_blocks(Some(event_blocks));
                    }
                    self.event_processor.set_param_event_blocks(Some(param_event_blocks));
                },
                TrackBackgroundProcessorInwardEvent::Play(start_at_block_number) => {
                    match std::thread::current().name() {
                        Some(thread_name) => {
                            debug!("*************{} thread received play", thread_name);
                        },
                        None => (),
                    };
                    self.event_processor.set_play(true);
                    self.event_processor.set_block_index(start_at_block_number);

                    // the sample must not carry over a mid-sample read position from
                    // whatever was left playing when the transport last stopped - a
                    // note on event at the new start re-arms the sample from zero
                    self.sample_is_playing = false;
                    self.sample_current_frame = 0;

                    // plugins with song position driven state (arp/mono/delay/mod) start
                    // every play from the transport position, never from wherever their
                    // own free running counter had got to
                    self.seek_plugin_transports(start_at_block_number);

                    self.stop_all_playing_notes();
                },
                TrackBackgroundProcessorInwardEvent::Stop => {
                    match std::thread::current().name() {
                        Some(thread_name) => {
                            debug!("*************{} thread received stop", thread_name);
                        },
                        None => (),
                    };
                    self.event_processor.set_play(false);
                    self.vst_event_blocks = None;
                    self.vst_event_blocks_transition_to = None;
                    self.event_processor.set_block_index(-1);

                    // stop the sample reader too - otherwise the audio track thread
                    // keeps advancing the read cursor and pumping the old sample while
                    // the transport is stopped, so the next play starts out of sync
                    self.sample_is_playing = false;
                    self.sample_current_frame = 0;

                    // flush sounding notes through the plugin that is producing them
                    // (the override if active) before the override switches are dropped
                    self.stop_all_playing_notes();

                    // the arrangement override plugins are only switched in while the
                    // arrangement itself is playing - any live work resumes on the track's
                    // own instrument/effects once stopped
                    self.override_instrument_active = false;
                    self.override_effects_active = false;
                },
                TrackBackgroundProcessorInwardEvent::GotoStart => {
                    match std::thread::current().name() {
                        Some(thread_name) => {
                            debug!("*************{} thread received goto start", thread_name);
                        },
                        None => (),
                    };
                    if *self.event_processor.block_index() > -1 {
                        self.event_processor.set_block_index(0);
                        match std::thread::current().name() {
                            Some(thread_name) => {
                                debug!("*************{} thread goto start - block index set to 0", thread_name);
                            },
                            None => (),
                        };

                        // rewind the sample reader with the block index so the audio
                        // does not keep running on from where it was when the transport
                        // rewound underneath it
                        self.sample_is_playing = false;
                        self.sample_current_frame = 0;

                        self.seek_plugin_transports(0);
                    }
                },
                TrackBackgroundProcessorInwardEvent::MoveBack => {
                    match std::thread::current().name() {
                        Some(thread_name) => {
                            debug!("*************{} thread received move back", thread_name);
                        },
                        None => (),
                    };
                    if *self.event_processor.block_index() > 0 {
                        self.event_processor.set_block_index(self.event_processor.block_index() - 1);
                        self.sample_is_playing = false;
                        self.sample_current_frame = 0;
                        self.seek_plugin_transports(*self.event_processor.block_index());
                    }
                },
                TrackBackgroundProcessorInwardEvent::Pause => {
                    if *self.event_processor.play() {
                        self.event_processor.set_play(false);
                        // freeze the sample reader where it is with nothing pumping -
                        // on resume the note events re-arm the sample from the new
                        // block start rather than continuing a stale mid-sample cursor
                        self.sample_is_playing = false;
                        self.sample_current_frame = 0;
                    }
                },
                TrackBackgroundProcessorInwardEvent::MoveForward => () /* if let Some(event_blocks) = vst_event_blocks {
                            if  (self.block_index + 1) < self.event_blocks.len() as i32 {
                                self.block_index += 1;
                            }
                        } */,
                TrackBackgroundProcessorInwardEvent::GotoEnd => () /* if let Some(event_blocks) = vst_event_blocks {
                            if self.block_index > -1 {
                                self.block_index = self.event_blocks.len() as i32 -1;
                            }
                        } */,
                TrackBackgroundProcessorInwardEvent::Kill => {
                    for effect in self.effect_plugin_instances.iter_mut() {
                        effect.close_editor();
                        effect.stop_processing();
                        effect.shutdown();
                    }
                    for effect in self.override_effect_plugin_instances.iter_mut() {
                        effect.close_editor();
                        effect.stop_processing();
                        effect.shutdown();
                    }
                    if let Some(instrument_plugin) = self.instrument_plugin_instances.get_mut(0) {
                        instrument_plugin.close_editor();
                        instrument_plugin.stop_processing();
                        instrument_plugin.shutdown();
                    }
                    if let Some(instrument_plugin) = self.override_instrument_plugin_instances.get_mut(0) {
                        instrument_plugin.close_editor();
                        instrument_plugin.stop_processing();
                        instrument_plugin.shutdown();
                    }
                    self.keep_alive = false;
                },
                TrackBackgroundProcessorInwardEvent::AddEffect(vst24_plugin_loaders, clap_plugin_loaders, uuid, scanned_plugin) => {
                    let plugin_instance: Result<BackgroundProcessorAudioPluginType, String> = if let AudioPluginType::VST24 = scanned_plugin.audio_plugin_stack {
                        BackgroundProcessorVst24AudioPlugin::new_with_uuid(
                            vst24_plugin_loaders,
                            self.track_uuid.clone(),
                            uuid,
                            scanned_plugin.sub_id.clone(),
                            scanned_plugin.path.clone(),
                            self.vst_host_time_info.clone(),
                            self.sample_rate,
                            self.block_size as i64,
                            self.tempo,
                            self.time_signature_numerator,
                            self.time_signature_denominator,
                        ).map(BackgroundProcessorAudioPluginType::Vst24)
                    }
                    else if let AudioPluginType::CLAP = scanned_plugin.audio_plugin_stack {
                        self.create_clap_audio_plugin_instance(uuid, &scanned_plugin).map(BackgroundProcessorAudioPluginType::Clap)
                    }
                    else {
                        BackgroundProcessorVst3AudioPlugin::new_with_uuid(
                            self.track_uuid.clone(),
                            uuid,
                            scanned_plugin.id.clone(),
                            scanned_plugin.path.clone(),
                            false,
                            self.sample_rate,
                            self.block_size as i64,
                            self.tempo,
                            self.time_signature_numerator,
                            self.time_signature_denominator,
                        ).map(BackgroundProcessorAudioPluginType::Vst3)
                    };

                    match plugin_instance {
                        Ok(plugin_instance) => {
                            self.effect_plugin_instances.push(plugin_instance);
                            self.request_effect_params = true;
                            self.request_effect_params_for_uuid.clear();
                            self.request_effect_params_for_uuid.push_str(uuid.to_string().as_str());
                        },
                        Err(error) => {
                            // the track thread must stay alive (and keep sending silent
                            // blocks to the jack layer) even if a plugin cannot be loaded.
                            let message = format!("Track '{}' could not load effect plugin '{}' from '{}': {}", self.track_uuid, scanned_plugin.name, scanned_plugin.path, error);
                            error!("{}", message);
                            eprintln!("{}", message);
                        },
                    }
                }
                TrackBackgroundProcessorInwardEvent::DeleteEffect(uuid) => {
                    for effect in self.effect_plugin_instances.iter_mut() {
                        if effect.uuid().to_string() == uuid {
                            effect.close_editor();
                            effect.stop_processing();
                            effect.shutdown();
                            self.vst_effect_editors.remove(&effect.uuid().to_string());
                        }
                    }
                    self.effect_plugin_instances.retain(|effect| {
                        effect.uuid().to_string() != uuid
                    });
                    for effect in self.override_effect_plugin_instances.iter_mut() {
                        if effect.uuid().to_string() == uuid {
                            effect.close_editor();
                            effect.stop_processing();
                            effect.shutdown();
                            self.vst_effect_editors.remove(&effect.uuid().to_string());
                        }
                    }
                    self.override_effect_plugin_instances.retain(|effect| {
                        effect.uuid().to_string() != uuid
                    });
                }
                TrackBackgroundProcessorInwardEvent::ChangeInstrument(vst24_plugin_loaders, clap_plugin_loaders, uuid, scanned_plugin) => {
                    if let Some(mut plugin_instance_to_delete) = self.instrument_plugin_instances.pop() {
                        // The GUI/editor has to be destroyed before the plugin instance is
                        // shut down and dropped, otherwise the plugin aborts (CLAP) or
                        // crashes (VST2) when it is torn down with a live editor.
                        plugin_instance_to_delete.close_editor();
                        match plugin_instance_to_delete {
                            BackgroundProcessorAudioPluginType::Vst24(mut vst24_plugin) => {
                                vst24_plugin.stop_processing();
                                vst24_plugin.shutdown();
                            }
                            BackgroundProcessorAudioPluginType::Vst3(mut vst3_plugin) => {
                                vst3_plugin.shutdown();
                            }
                            BackgroundProcessorAudioPluginType::Clap(mut clap_plugin) => {
                                clap_plugin.stop_processing();
                                clap_plugin.shutdown();
                            }
                        }
                    }

                    let plugin_instance_to_add: Result<BackgroundProcessorAudioPluginType, String> = if let AudioPluginType::VST24 = scanned_plugin.audio_plugin_stack {
                        BackgroundProcessorVst24AudioPlugin::new_with_uuid(
                            vst24_plugin_loaders,
                            self.track_uuid.clone(),
                            uuid,
                            scanned_plugin.sub_id.clone(),
                            scanned_plugin.path.clone(),
                            self.vst_host_time_info.clone(),
                            self.sample_rate,
                            self.block_size as i64,
                            self.tempo,
                            self.time_signature_numerator,
                            self.time_signature_denominator,
                        ).map(BackgroundProcessorAudioPluginType::Vst24)
                    }
                    else if let AudioPluginType::CLAP = scanned_plugin.audio_plugin_stack {
                        self.create_clap_audio_plugin_instance(uuid, &scanned_plugin).map(BackgroundProcessorAudioPluginType::Clap)
                    }
                    else {
                        BackgroundProcessorVst3AudioPlugin::new_with_uuid(
                            self.track_uuid.clone(),
                            uuid,
                            scanned_plugin.id.clone(),
                            scanned_plugin.path.clone(),
                            true,
                            self.sample_rate,
                            self.block_size as i64,
                            self.tempo,
                            self.time_signature_numerator,
                            self.time_signature_denominator,
                        ).map(BackgroundProcessorAudioPluginType::Vst3)
                    };

                    match plugin_instance_to_add {
                        Ok(plugin_instance_to_add) => {
                            // the track panel displays the loaded instrument's name - the
                            // project load/ChangeInstrument event has to report it back for
                            // every plugin format.
                            let instrument_name = BackgroundProcessorAudioPlugin::name(&plugin_instance_to_add);
                            match self.tx_vst_thread.send(TrackBackgroundProcessorOutwardEvent::InstrumentName(instrument_name)) {
                                Ok(_) => debug!("Sent instrument name to main processing loop."),
                                Err(_) => debug!("Failed to send instrument name to main processing loop."),
                            }

                            // FIXME the following commented out line causes a crash - or kills the track thread
                            // self.instrument_plugin_instances.clear();
                            self.instrument_plugin_instances.push(plugin_instance_to_add);
                            self.handle_request_instrument_plugin_parameters();
                        },
                        Err(error) => {
                            // the track thread must stay alive (and keep sending silent
                            // blocks to the jack layer) even if an instrument plugin cannot
                            // be loaded, otherwise this track's missing audio blocks starve
                            // the whole master mix.
                            let message = format!("Track '{}' could not load instrument plugin '{}' from '{}': {}", self.track_uuid, scanned_plugin.name, scanned_plugin.path, error);
                            error!("{}", message);
                            eprintln!("{}", message);
                            match self.tx_vst_thread.send(TrackBackgroundProcessorOutwardEvent::InstrumentName(format!("{} (not loaded)", scanned_plugin.name))) {
                                Ok(_) => (),
                                Err(_) => debug!("Failed to send instrument name to main processing loop."),
                            }
                        },
                    }
                }
                TrackBackgroundProcessorInwardEvent::SetOverrideInstrument(vst24_plugin_loaders, clap_plugin_loaders, override_instrument) => {
                    // idempotent - the sync path resends the whole desired override state and a
                    // reload of the same plugin instance would drop its captured preset state
                    let already_loaded = match (&override_instrument, self.override_instrument_plugin_instances.get(0)) {
                        (Some((uuid, _scanned_plugin, _preset)), Some(instance)) => instance.uuid() == *uuid,
                        (None, None) => true,
                        _ => false,
                    };
                    if !already_loaded {
                        if let Some(mut plugin_instance_to_delete) = self.override_instrument_plugin_instances.pop() {
                            plugin_instance_to_delete.close_editor();
                            plugin_instance_to_delete.stop_processing();
                            plugin_instance_to_delete.shutdown();
                        }

                        if let Some((uuid, scanned_plugin, preset_data)) = override_instrument {
                            match self.create_audio_plugin_instance(vst24_plugin_loaders, clap_plugin_loaders, uuid, &scanned_plugin, true) {
                                Ok(mut plugin_instance) => {
                                    if !preset_data.is_empty() {
                                        plugin_instance.set_preset_data(preset_data);
                                    }
                                    self.override_instrument_plugin_instances.push(plugin_instance);
                                },
                                Err(error) => {
                                    let message = format!("Track '{}' could not load override instrument plugin '{}' from '{}': {}", self.track_uuid, scanned_plugin.name, scanned_plugin.path, error);
                                    error!("{}", message);
                                    eprintln!("{}", message);
                                },
                            }
                        }

                        // refresh the parameter details so the automation views can offer the
                        // override plugin's parameters
                        self.handle_request_instrument_plugin_parameters();
                    }
                }
                TrackBackgroundProcessorInwardEvent::SetOverrideEffects(vst24_plugin_loaders, clap_plugin_loaders, override_effects) => {
                    // remove the override effects that are no longer wanted
                    for effect in self.override_effect_plugin_instances.iter_mut() {
                        if !override_effects.iter().any(|(uuid, _scanned_plugin, _preset)| *uuid == effect.uuid()) {
                            effect.close_editor();
                            effect.stop_processing();
                            effect.shutdown();
                            self.vst_effect_editors.remove(&effect.uuid().to_string());
                        }
                    }
                    self.override_effect_plugin_instances.retain(|effect| {
                        override_effects.iter().any(|(uuid, _scanned_plugin, _preset)| *uuid == effect.uuid())
                    });

                    // add the missing ones and reorder the list to match the arrangement
                    let mut new_instances: Vec<BackgroundProcessorAudioPluginType> = vec![];
                    for (uuid, scanned_plugin, preset_data) in override_effects {
                        if let Some(position) = self.override_effect_plugin_instances.iter().position(|effect| effect.uuid() == uuid) {
                            new_instances.push(self.override_effect_plugin_instances.remove(position));
                        }
                        else {
                            match self.create_audio_plugin_instance(vst24_plugin_loaders.clone(), clap_plugin_loaders.clone(), uuid, &scanned_plugin, false) {
                                Ok(mut plugin_instance) => {
                                    if !preset_data.is_empty() {
                                        plugin_instance.set_preset_data(preset_data);
                                    }
                                    // request the parameter details for the newly added
                                    // override effect so its automation parameters are on
                                    // offer in the automation views
                                    self.pending_override_effect_param_requests.push(uuid.to_string());
                                    new_instances.push(plugin_instance);
                                },
                                Err(error) => {
                                    let message = format!("Track '{}' could not load override effect plugin '{}' from '{}': {}", self.track_uuid, scanned_plugin.name, scanned_plugin.path, error);
                                    error!("{}", message);
                                    eprintln!("{}", message);
                                },
                            }
                        }
                    }
                    self.override_effect_plugin_instances = new_instances;
                }
                TrackBackgroundProcessorInwardEvent::SetOverrideInstrumentActive(active) => {
                    self.override_instrument_active = active && !self.override_instrument_plugin_instances.is_empty();
                }
                TrackBackgroundProcessorInwardEvent::SetOverrideEffectsActive(active) => {
                    self.override_effects_active = active && !self.override_effect_plugin_instances.is_empty();
                }
                TrackBackgroundProcessorInwardEvent::SetPresetData(instrument_preset_data, effect_presets) => {
                    // an empty string means "no captured state" (e.g. a chunkless plugin
                    // or a failed capture) - applying it would reset the plugin.
                    if !instrument_preset_data.is_empty() {
                        if let Some(instrument_plugin) = self.instrument_plugin_instances.get_mut(0) {
                            instrument_plugin.set_preset_data(instrument_preset_data);
                        }
                    }
                    let mut index = 0;
                    for effect in self.effect_plugin_instances.iter_mut() {
                        match effect_presets.get(index) {
                            Some(effect_preset_data) => {
                                if !effect_preset_data.is_empty() {
                                    effect.set_preset_data(effect_preset_data.clone());
                                }
                            },
                            None => debug!("Could not set preset for effect at index: {}", index),
                        }
                        index += 1;
                    }
                },
                TrackBackgroundProcessorInwardEvent::RequestPresetData(response_sender) => {
                    // keep only the newest request - one response satisfies it and any
                    // earlier (superseded) request's receiver will simply be dropped,
                    // letting its waiter fail cleanly instead of getting stale data.
                    self.pending_preset_response_sender = Some(response_sender);
                    debug!("Track audio - Received RequestPresetData for track: {}", self.track_uuid.clone());
                },
                TrackBackgroundProcessorInwardEvent::Mute => {
                    match std::thread::current().name() {
                        Some(thread_name) => {
                            debug!("*************{} thread received mute", thread_name);
                        },
                        None => (),
                    };
                    self.event_processor.set_mute(true);
                    self.mute = true;
                    self.stop_all_playing_notes();
                },
                TrackBackgroundProcessorInwardEvent::Unmute => {
                    match std::thread::current().name() {
                        Some(thread_name) => {
                            debug!("*************{} thread received unmute", thread_name);
                        },
                        None => (),
                    };
                    self.event_processor.set_mute(false);
                    self.mute = false;
                },
                TrackBackgroundProcessorInwardEvent::PlayNoteImmediate(note, midi_channel) => {
                    debug!("Track background processor: Received play note immediate.");

                    let note_on = NoteOn::new_with_params(MidiPolyphonicExpressionNoteId::ALL as i32, 0.0, note, 127);
                    self.event_processor.audio_plugin_immediate_events_mut().push(TrackEvent::NoteOn(note_on));

                    let note_on = MidiEvent {
                        data: [144 + (midi_channel as u8), note as u8, 127_u8],
                        delta_frames: 0,
                        live: true,
                        note_length: None,
                        note_offset: None,
                        detune: 0,
                        note_off_velocity: 0,
                    };
                    self.jack_midi_out_immediate_events.push(note_on);
                },
                TrackBackgroundProcessorInwardEvent::StopNoteImmediate(note, midi_channel) => {
                    debug!("Track background processor layer received stop note immediate.");

                    let note_off = NoteOff::new_with_params(MidiPolyphonicExpressionNoteId::ALL as i32, 0.0, note, 127);
                    self.event_processor.audio_plugin_immediate_events_mut().push(TrackEvent::NoteOff(note_off));

                    let note_off = MidiEvent {
                        data: [128 + (midi_channel as u8), note as u8, 127_u8],
                        delta_frames: 0,
                        live: true,
                        note_length: None,
                        note_offset: None,
                        detune: 0,
                        note_off_velocity: 0,
                    };
                    self.jack_midi_out_immediate_events.push(note_off);
                },
                TrackBackgroundProcessorInwardEvent::Loop(loop_on) => {
                    self.event_processor.set_play_loop_on(loop_on);
                    debug!("Track background processor received loop on/off: on={}", self.event_processor.play_loop_on());
                },
                TrackBackgroundProcessorInwardEvent::LoopExtents(left_block_index, right_block_index) => {
                    self.event_processor.set_play_left_block_index(left_block_index);
                    self.event_processor.set_play_right_block_index(right_block_index);
                    debug!("Track background processor received extents: left={}, right={}", self.event_processor.play_left_block_index(), self.event_processor.play_right_block_index());
                },
                TrackBackgroundProcessorInwardEvent::RequestInstrumentParameters => self.handle_request_instrument_plugin_parameters(),
                TrackBackgroundProcessorInwardEvent::RequestEffectParameters(effect_uuid) => {
                    self.request_effect_params = true;
                    self.request_effect_params_for_uuid.clear();
                    self.request_effect_params_for_uuid.push_str(effect_uuid.as_str());
                },
                TrackBackgroundProcessorInwardEvent::SetInstrumentWindowId(xid) => {
                    let instrument_plugin_uuid = if let Some(instrument_plugin) = self.instrument_plugin_instances.get(0) {
                        instrument_plugin.uuid().to_string()
                    }
                    else {
                        "".to_string()
                    };
                    if let Some(instrument_plugin) = self.instrument_plugin_instances.get_mut(0) {
                        instrument_plugin.set_xid(Some(xid));
                        let (plugin_window_width, plugin_window_height) = instrument_plugin.get_window_size();
                        match self.tx_vst_thread.send(TrackBackgroundProcessorOutwardEvent::InstrumentPluginWindowSize(self.track_uuid.clone(), instrument_plugin_uuid.clone(), plugin_window_width, plugin_window_height)) {
                            Ok(_) => debug!("Instrument plugin window size sent for: track={}, instrument={}, name={}.", self.track_uuid.clone(), instrument_plugin_uuid, instrument_plugin.name()),
                            Err(error) => debug!("Problem sending plugin window size from VST thread to state: {}", error),
                        }
                    }
                },
                TrackBackgroundProcessorInwardEvent::SetOverrideInstrumentWindowId(xid) => {
                    let instrument_plugin_uuid = if let Some(instrument_plugin) = self.override_instrument_plugin_instances.get(0) {
                        instrument_plugin.uuid().to_string()
                    }
                    else {
                        "".to_string()
                    };
                    if let Some(instrument_plugin) = self.override_instrument_plugin_instances.get_mut(0) {
                        instrument_plugin.set_xid(Some(xid));
                        let (plugin_window_width, plugin_window_height) = instrument_plugin.get_window_size();
                        match self.tx_vst_thread.send(TrackBackgroundProcessorOutwardEvent::InstrumentPluginWindowSize(self.track_uuid.clone(), instrument_plugin_uuid.clone(), plugin_window_width, plugin_window_height)) {
                            Ok(_) => debug!("Override instrument plugin window size sent for: track={}, instrument={}, name={}.", self.track_uuid.clone(), instrument_plugin_uuid, instrument_plugin.name()),
                            Err(error) => debug!("Problem sending override plugin window size from VST thread to state: {}", error),
                        }
                    }
                },
                TrackBackgroundProcessorInwardEvent::SetEffectWindowId(effect_uuid, xid) => {
                    debug!("Received - VstThreadInwardEvent::SetEffectWindowId({}, {})", effect_uuid, xid);
                    for effect in self.effect_plugin_instances.iter_mut().chain(self.override_effect_plugin_instances.iter_mut()) {
                        if effect.uuid().to_string() == effect_uuid {
                            effect.set_xid(Some(xid));
                            let (plugin_window_width, plugin_window_height) = effect.get_window_size();
                            match self.tx_vst_thread.send(TrackBackgroundProcessorOutwardEvent::EffectPluginWindowSize(self.track_uuid.clone(), effect_uuid.clone(), plugin_window_width, plugin_window_height)) {
                                Ok(_) => debug!("Effect plugin window size sent sent for: track={}, effect={}.", self.track_uuid.clone(), effect_uuid),
                                Err(error) => debug!("Problem sending effect plugin window size from VST thread to state: {}", error),
                            }
                        }
                    }
                },
                TrackBackgroundProcessorInwardEvent::SetBlockPosition(block_position) => {
                    self.event_processor.set_block_index(block_position);
                    let _all_note_offs: Vec<MidiEvent> = Vec::new();

                    // scrubbing moves the block index under the running audio - rewind
                    // the sample reader with it so the tracks never keep playing from
                    // the old position after the seek
                    self.sample_is_playing = false;
                    self.sample_current_frame = 0;

                    self.seek_plugin_transports(block_position);

                    self.stop_all_playing_notes();
                },
                TrackBackgroundProcessorInwardEvent::Volume(volume) => {
                    self.volume = volume;
                }
                TrackBackgroundProcessorInwardEvent::Pan(pan) => {
                    self.pan = pan;
                }
                TrackBackgroundProcessorInwardEvent::PlayControllerImmediate(controller_type, value, midi_channel) => {
                    debug!("Track background processor: Received play controller immediate.");

                    let controller = Controller::new(0.0, controller_type, value);
                    self.event_processor.audio_plugin_immediate_events_mut().push(TrackEvent::Controller(controller));

                    let controller = MidiEvent {
                        data: [176 + (midi_channel as u8), controller_type as u8, value as u8],
                        delta_frames: 0,
                        live: true,
                        note_length: None,
                        note_offset: None,
                        detune: 0,
                        note_off_velocity: 0,
                    };
                    self.jack_midi_out_immediate_events.push(controller);
                }
                TrackBackgroundProcessorInwardEvent::PlayPitchBendImmediate(lsb, msb, midi_channel) => {
                    debug!("Track background processor: Received play pitch bend immediate.");

                    let pitch_bend = PitchBend::new_from_midi_bytes(0.0, lsb as u8, msb as u8);
                    self.event_processor.audio_plugin_immediate_events_mut().push(TrackEvent::PitchBend(pitch_bend));

                    let pitch_bend = MidiEvent {
                        data: [224 + (midi_channel as u8), lsb as u8, msb as u8],
                        delta_frames: 0,
                        live: true,
                        note_length: None,
                        note_offset: None,
                        detune: 0,
                        note_off_velocity: 0,
                    };
                    self.jack_midi_out_immediate_events.push(pitch_bend);
                }
                TrackBackgroundProcessorInwardEvent::PlayAfterTouchImmediate(value, midi_channel) => {
                    debug!("Track background processor: Received play after touch immediate.");

                    let after_touch = AfterTouch::new(0.0, value);
                    self.event_processor.audio_plugin_immediate_events_mut().push(TrackEvent::AfterTouch(after_touch));

                    let after_touch = MidiEvent {
                        data: [208 + (midi_channel as u8), value as u8, 0],
                        delta_frames: 0,
                        live: true,
                        note_length: None,
                        note_offset: None,
                        detune: 0,
                        note_off_velocity: 0,
                    };
                    self.jack_midi_out_immediate_events.push(after_touch);
                }
                TrackBackgroundProcessorInwardEvent::PlayKeyPressureImmediate(note, value, midi_channel) => {
                    debug!("Track background processor: Received play key pressure immediate.");

                    let key_pressure = KeyPressure::new(0.0, note, value);
                    self.event_processor.audio_plugin_immediate_events_mut().push(TrackEvent::KeyPressure(key_pressure));

                    let key_pressure = MidiEvent {
                        data: [160 + (midi_channel as u8), note as u8, value as u8],
                        delta_frames: 0,
                        live: true,
                        note_length: None,
                        note_offset: None,
                        detune: 0,
                        note_off_velocity: 0,
                    };
                    self.jack_midi_out_immediate_events.push(key_pressure);
                }
                TrackBackgroundProcessorInwardEvent::PlayProgramChangeImmediate(program, midi_channel) => {
                    debug!("Track background processor: Received play program change immediate.");

                    let program_change = ProgramChange::new(0.0, program);
                    self.event_processor.audio_plugin_immediate_events_mut().push(TrackEvent::ProgramChange(program_change));

                    let program_change = MidiEvent {
                        data: [192 + (midi_channel as u8), program as u8, 0],
                        delta_frames: 0,
                        live: true,
                        note_length: None,
                        note_offset: None,
                        detune: 0,
                        note_off_velocity: 0,
                    };
                    self.jack_midi_out_immediate_events.push(program_change);
                }
                TrackBackgroundProcessorInwardEvent::SetSample(sample_data) => {
                    self.set_sample(Some(sample_data));
                    self.sample_is_playing = false;
                    self.sample_current_frame = 0;
                }
                TrackBackgroundProcessorInwardEvent::SetInstrumentParameter(_param_index, _param_value) => {
                    if let Some(_instrument_plugin) = self.instrument_plugin_instances.get_mut(0) {
                        // let _vst_plugin_instance: &mut PluginInstance = instrument_plugin.vst_plugin_instance_mut();
                        // vst_plugin_instance.get_params().set_parameter(param_index, param_value);
                        // vst_plugin_instance.get_parameter_object().set_parameter(param_index, param_value);
                    }
                }
                TrackBackgroundProcessorInwardEvent::Tempo(tempo) => {
                    self.tempo = tempo;
                    if let Some(instrument_plugin) = self.instrument_plugin_instances.get_mut(0) {
                        instrument_plugin.set_tempo(tempo);
                    }
                    if let Some(instrument_plugin) = self.override_instrument_plugin_instances.get_mut(0) {
                        instrument_plugin.set_tempo(tempo);
                    }
                    for effect in self.effect_plugin_instances.iter_mut().chain(self.override_effect_plugin_instances.iter_mut()) {
                        effect.set_tempo(tempo);
                    }
                }
                TrackBackgroundProcessorInwardEvent::TimeSignatureChange(numerator, denominator) => {
                    if let Some(instrument_plugin) = self.instrument_plugin_instances.get_mut(0) {
                        instrument_plugin.set_time_signature(numerator, denominator);
                    }
                    if let Some(instrument_plugin) = self.override_instrument_plugin_instances.get_mut(0) {
                        instrument_plugin.set_time_signature(numerator, denominator);
                    }
                    for effect in self.effect_plugin_instances.iter_mut().chain(self.override_effect_plugin_instances.iter_mut()) {
                        effect.set_time_signature(numerator, denominator);
                    }
                }
                TrackBackgroundProcessorInwardEvent::AudioConfigurationChange(sample_rate, block_size) => {
                    debug!("Track background processor received audio configuration change: sample_rate={}, block_size={}", sample_rate, block_size);

                    // stop this track's playback and flush any sounding notes before the
                    // plugins are reconfigured.
                    self.event_processor.set_play(false);
                    self.stop_all_playing_notes();
                    self.event_processor.set_block_index(-1);

                    // reconfigure the plugins following their format specific protocol
                    // (stop/suspend -> set rate & size -> restart/resume).
                    if let Some(instrument_plugin) = self.instrument_plugin_instances.get_mut(0) {
                        instrument_plugin.change_sample_rate_and_block_size(sample_rate, block_size);
                    }
                    if let Some(instrument_plugin) = self.override_instrument_plugin_instances.get_mut(0) {
                        instrument_plugin.change_sample_rate_and_block_size(sample_rate, block_size);
                    }
                    for effect in self.effect_plugin_instances.iter_mut().chain(self.override_effect_plugin_instances.iter_mut()) {
                        effect.change_sample_rate_and_block_size(sample_rate, block_size);
                    }

                    // update this track's audio configuration
                    self.sample_rate = sample_rate;
                    self.block_size = block_size as f64;
                    self.event_processor.set_block_size(block_size as f64);
                }
                TrackBackgroundProcessorInwardEvent::AddTrackEventSendRouting(track_event_routing, ring_buffer, producer) => {
                    match &track_event_routing.source {
                        TrackEventRoutingNodeType::Track(_) => {
                            self.track_events_outward_ring_buffers.insert(track_event_routing.uuid(), ring_buffer);
                            self.track_events_outward_producers.insert(track_event_routing.uuid(), producer);
                            self.track_events_outward_routings.insert(track_event_routing.uuid(), track_event_routing);
                        }
                        TrackEventRoutingNodeType::Instrument(_, _) => {
                            // add the routing to the plugin if required
                            if let Some(instrument_plugin) = self.instrument_plugin_instances.get_mut(0) {
                                match instrument_plugin {
                                    BackgroundProcessorAudioPluginType::Vst24(vst_24_plugin) => {
                                        if let Ok(mut vst_host) = vst_24_plugin.host_mut().lock() {
                                            vst_host.add_track_event_outward_routing(track_event_routing, ring_buffer, producer);
                                        }
                                    }
                                    BackgroundProcessorAudioPluginType::Vst3(_vst3_plugin) => {}
                                    BackgroundProcessorAudioPluginType::Clap(_) => {

                                    }
                                }
                            }
                        }
                        TrackEventRoutingNodeType::Effect(_, _) => {
                            // Not sure if this is actually a reality
                        }
                    }
            
                }
                TrackBackgroundProcessorInwardEvent::RemoveTrackEventSendRouting(route_uuid) => {
                    // remove the routing from the vst host
                    if let Some(instrument_plugin) = self.instrument_plugin_instances.get_mut(0) {
                        match instrument_plugin {
                            BackgroundProcessorAudioPluginType::Vst24(vst_24_plugin) => {
                                if let Ok(mut vst_host) = vst_24_plugin.host_mut().lock() {
                                    vst_host.remove_track_event_outward_routing(route_uuid);
                                }
                            }
                            BackgroundProcessorAudioPluginType::Vst3(_vst3_plugin) => {}
                            BackgroundProcessorAudioPluginType::Clap(_) => {

                            }
                        }
                    }
                }
                TrackBackgroundProcessorInwardEvent::AddTrackEventReceiveRouting(track_event_routing, track_event_source) => {
                    self.add_track_event_inward_routing(track_event_routing, track_event_source);
                }
                TrackBackgroundProcessorInwardEvent::RemoveTrackEventReceiveRouting(route_uuid) => {
                    self.remove_track_event_inward_routing(route_uuid);
                }
                TrackBackgroundProcessorInwardEvent::UpdateTrackEventSendRouting(route_uuid, midi_routing) => {
                    self.track_events_outward_routings.insert(route_uuid, midi_routing);
                }
                TrackBackgroundProcessorInwardEvent::UpdateTrackEventReceiveRouting(route_uuid, midi_routing) => {
                    self.track_events_inward_routings.insert(route_uuid, midi_routing);
                }
                TrackBackgroundProcessorInwardEvent::AddAudioSendRouting(audio_routing, ring_buffers, producers) => {
                    self.audio_outward_ring_buffers.insert(audio_routing.uuid(), ring_buffers);
                    self.audio_outward_producers.insert(audio_routing.uuid(), producers);
                    self.audio_outward_routings.insert(audio_routing.uuid(), audio_routing);
                    // match &audio_routing.source {
                    //     AudioRoutingNodeType::Track(_) => {
                    //         self.audio_outward_ring_buffers.insert(audio_routing.uuid(), ring_buffer);
                    //         self.audio_outward_producers.insert(audio_routing.uuid(), producer);
                    //         self.audio_outward_routings.insert(audio_routing.uuid(), audio_routing);
                    //     }
                    //     AudioRoutingNodeType::Instrument(_, _, _, _) => {
                    //     }
                    //     AudioRoutingNodeType::Effect(_, _, _, _) => {
                    //         // Not sure if this is actually a reality
                    //     }
                    // }
                }
                TrackBackgroundProcessorInwardEvent::RemoveAudioSendRouting(_route_uuid) => {
                }
                TrackBackgroundProcessorInwardEvent::AddAudioReceiveRouting(audio_routing, audio_source) => {
                    self.add_audio_inward_routing(audio_routing, audio_source);
                }
                TrackBackgroundProcessorInwardEvent::RemoveAudioReceiveRouting(route_uuid) => {
                    self.remove_audio_inward_routing(route_uuid);
                }
                }
            },
            Err(_) => break,
        }
        }
        had_events
    }

    fn stop_all_playing_notes(&mut self) {
        if !self.event_processor.playing_notes().is_empty() {
            let use_override_instrument = self.override_instrument_active && !self.override_instrument_plugin_instances.is_empty();
            let instrument_plugin = if use_override_instrument {
                self.override_instrument_plugin_instances.get_mut(0)
            }
            else {
                self.instrument_plugin_instances.get_mut(0)
            };
            if let Some(instrument_plugin) = instrument_plugin {
                match instrument_plugin {
                    BackgroundProcessorAudioPluginType::Vst24(vst_24_plugin) => {
                        let mut all_note_offs: Vec<MidiEvent> = Vec::new();
            
                        for note in self.event_processor.playing_notes().iter() {
                            let note_off = MidiEvent {
                                data: [128, *note as u8, 0_u8],
                                delta_frames: 0,
                                live: true,
                                note_length: None,
                                note_offset: None,
                                detune: 0,
                                note_off_velocity: 0,
                            };
                            all_note_offs.push(note_off);
                        }
                        debug!("Sending note off events to the VST instrument: {}", all_note_offs.len());
                        self.midi_sender.store_events(all_note_offs);
                        vst_24_plugin.vst_plugin_instance_mut().process_events(self.midi_sender.events())
                    }
                    BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                        let mut all_note_offs: Vec<TrackEvent> = Vec::new();

                        for note in self.event_processor.playing_notes().iter() {
                            let note_off = NoteOff::new_with_params(MidiPolyphonicExpressionNoteId::ALL as i32, 0.0, *note, 0);
                            all_note_offs.push(TrackEvent::NoteOff(note_off));
                        }
                        debug!("Sending note off events to the VST3 instrument: {}", all_note_offs.len());
                        vst3_plugin.process_events(&all_note_offs);
                    }
                    BackgroundProcessorAudioPluginType::Clap(clap_plugin) => {
                        let mut all_note_offs: Vec<TrackEvent> = Vec::new();
            
                        for note in self.event_processor.playing_notes().iter() {
                            let note_off = NoteOff::new_with_params(MidiPolyphonicExpressionNoteId::ALL as i32, 0.0, *note, 0);
                            all_note_offs.push(TrackEvent::NoteOff(note_off));
                        }
                        debug!("Sending note off events to the CLAP instrument: {}", all_note_offs.len());
                        clap_plugin.process_events(&all_note_offs);
                    }
                }
            }
            self.event_processor.playing_notes_mut().clear();
        }
    }

    pub fn refresh_instrument_plugin_editor(&mut self) {
        for instrument_plugin in self.instrument_plugin_instances.iter_mut().chain(self.override_instrument_plugin_instances.iter_mut()) {
            match instrument_plugin {
                BackgroundProcessorAudioPluginType::Vst24(vst24_plugin) => {
                    if let Some(editor) = vst24_plugin.editor_mut().as_mut() {
                        if editor.is_open() {
                            // editor.idle(); // doesn't do anything
                            vst24_plugin.vst_plugin_instance_mut().editor_idle();
                        }
                    }
                }
                BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                    vst3_plugin.repaint();
                }
                BackgroundProcessorAudioPluginType::Clap(_) => {

                }
            }
        }
    }

    pub fn refresh_effect_plugin_editors(&mut self) {
        for effect_plugin in self.effect_plugin_instances.iter_mut().chain(self.override_effect_plugin_instances.iter_mut()) {
            match effect_plugin {
                BackgroundProcessorAudioPluginType::Vst24(vst24_plugin) => {
                    if let Some(editor) = vst24_plugin.editor_mut().as_mut() {
                        if editor.is_open() {
                            // editor.idle(); // doesn't do anything
                            vst24_plugin.vst_plugin_instance_mut().editor_idle();
                        }
                    }
                }
                BackgroundProcessorAudioPluginType::Vst3(_vst3_plugin) => {}
                BackgroundProcessorAudioPluginType::Clap(_) => {

                }
            }
        }
    }

    /// True when any plugin on this track has a GUI editor attached. Tracks with open
    /// editors must not go quiescent: VST2 editors need editor_idle pumping and CLAP/VST3
    /// windows need their timer callbacks, both of which are tied to the plugin being
    /// processed. VST3/CLAP only expose a sticky xid, so this is conservative for them.
    pub fn plugin_editor_open(&mut self) -> bool {
        for instrument_plugin in self.instrument_plugin_instances.iter_mut().chain(self.override_instrument_plugin_instances.iter_mut()) {
            match instrument_plugin {
                BackgroundProcessorAudioPluginType::Vst24(vst24_plugin) => {
                    if let Some(editor) = vst24_plugin.editor_mut().as_mut() {
                        if editor.is_open() {
                            return true;
                        }
                    }
                }
                BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                    if vst3_plugin.xid().is_some() {
                        return true;
                    }
                }
                BackgroundProcessorAudioPluginType::Clap(clap_plugin) => {
                    if clap_plugin.xid().is_some() {
                        return true;
                    }
                }
            }
        }
        for effect_plugin in self.effect_plugin_instances.iter_mut().chain(self.override_effect_plugin_instances.iter_mut()) {
            match effect_plugin {
                BackgroundProcessorAudioPluginType::Vst24(vst24_plugin) => {
                    if let Some(editor) = vst24_plugin.editor_mut().as_mut() {
                        if editor.is_open() {
                            return true;
                        }
                    }
                }
                BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                    if vst3_plugin.xid().is_some() {
                        return true;
                    }
                }
                BackgroundProcessorAudioPluginType::Clap(clap_plugin) => {
                    if clap_plugin.xid().is_some() {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Move every plugin instance owned by this track thread out of the helper
    /// and drop it. Call while holding the VST24 plugin lifecycle WRITE lock:
    /// dropping a wrapper dispatches effClose, and plugins instantiated from the
    /// same library share process-global state, so an effClose must never
    /// overlap with another track thread's process() (this raced and segfaulted
    /// inside the JUCE VST2 wrapper of a u-he plugin during shutdown).
    ///
    /// When the application is exiting (`TRACK_THREADS_SHOULD_EXIT`) the
    /// instances are instead leaked: effClose can make a plugin tear down
    /// process-global code (u-he shells dlclose their shared engine module when
    /// they believe the last instance is gone), and any other track thread - or
    /// a plugin with a miscounted instance teardown - is then left holding
    /// function pointers into unloaded code (SIGSEGV at an unmapped address on
    /// the previous shutdown path). The kernel reclaims everything as the
    /// process exits, so leaking at shutdown is both safe and crash-free; the
    /// plugins were already stopped and suspended by the Kill handler.
    pub fn release_plugin_instances(&mut self) {
        let leak = TRACK_THREADS_SHOULD_EXIT.load(std::sync::atomic::Ordering::Relaxed);
        let mut lists = [
            &mut self.effect_plugin_instances,
            &mut self.override_effect_plugin_instances,
            &mut self.instrument_plugin_instances,
            &mut self.override_instrument_plugin_instances,
        ];
        for list in lists.iter_mut() {
            let instances = std::mem::take(*list);
            if leak {
                for instance in instances {
                    std::mem::forget(instance);
                }
            }
            // mid-session releases (file close / track removal) drop normally so
            // the instances are actually closed while the engine stays consistent
            // for the tracks that remain alive
        }
    }

    pub fn handle_host_events_from_plugins(&self) -> bool {
        let mut had_events = false;
        for instrument_plugin in self.instrument_plugin_instances.iter().chain(self.override_instrument_plugin_instances.iter()) {
            match instrument_plugin {
                BackgroundProcessorAudioPluginType::Vst24(_) => {
                    while let Ok(event) = instrument_plugin.rx_from_host().try_recv() {
                        had_events = true;
                        match event {
                            AudioPluginHostOutwardEvent::Automation(_track_uuid, plugin_uuid, is_instrument, param_index, param_value) => {
                                match self.tx_vst_thread.send(TrackBackgroundProcessorOutwardEvent::Automation(self.track_uuid.clone(), plugin_uuid, is_instrument, param_index, param_value)) {
                                    Ok(_) => (),
                                    Err(error) => debug!("Problem relaying instrument VstHost automation from VST thread to state: {}", error),
                                }
                            }
                            AudioPluginHostOutwardEvent::SizeWindow(_, plugin_uuid, _, width, height) => {
                                match self.tx_vst_thread.send(TrackBackgroundProcessorOutwardEvent::InstrumentPluginWindowSize(self.track_uuid.clone(), plugin_uuid, width, height)) {
                                    Ok(_) => (),
                                    Err(error) => debug!("Problem relaying instrument VstHost size window from VST thread to state: {}", error),
                                }
                            }
                        }
                    }
                }
                BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                    while let Ok(event) = vst3_plugin.rx_from_host().try_recv() {
                        had_events = true;
                        match event {
                            AudioPluginHostOutwardEvent::Automation(_track_uuid, plugin_uuid, is_instrument, param_index, param_value) => {
                                match self.tx_vst_thread.send(TrackBackgroundProcessorOutwardEvent::Automation(self.track_uuid.clone(), plugin_uuid, is_instrument, param_index, param_value)) {
                                    Ok(_) => (),
                                    Err(error) => debug!("Problem relaying instrument Vst3Host automation from VST3 thread to state: {}", error),
                                }
                            }
                            AudioPluginHostOutwardEvent::SizeWindow(_, plugin_uuid, _, width, height) => {
                                match self.tx_vst_thread.send(TrackBackgroundProcessorOutwardEvent::InstrumentPluginWindowSize(self.track_uuid.clone(), plugin_uuid, width, height)) {
                                    Ok(_) => (),
                                    Err(error) => debug!("Problem relaying instrument Vst3Host size window from VST3 thread to state: {}", error),
                                }
                            }
                        }
                    }
                }
                BackgroundProcessorAudioPluginType::Clap(clap_plugin) => {
                    // this first event receive is a bit bogus because it should really happen inside the host but calling the clap plugin process method is done outside the host
                    while let Ok(event) = clap_plugin.rx_from_host().try_recv() {
                        had_events = true;
                        if let AudioPluginHostOutwardEvent::Automation(track_uuid, plugin_uuid, _is_instrument, param_index, param_value) = event {
                            match self.tx_vst_thread.send(TrackBackgroundProcessorOutwardEvent::Automation(track_uuid, plugin_uuid, true, param_index, param_value)) {
                                Ok(_) => (),
                                Err(error) => debug!("Problem relaying instrument Clap Host automation from CLAP thread to state: {}", error),
                            }
                        }
                    }
                    while let Ok(message) = clap_plugin.host_receiver.try_recv() {
                        had_events = true;
                        match message {
                            DAWCallback::PluginGuiWindowRequestResize(width, height) => {
                                // apply the resize to the plugin GUI first (CLAP wants the
                                // host to answer request_resize() with set_size()), then
                                // grow/shrink the host window to the resulting size
                                let (width, height) = clap_plugin.handle_gui_resize(width, height);
                                match self.tx_vst_thread.send(TrackBackgroundProcessorOutwardEvent::InstrumentPluginWindowSize(self.track_uuid.clone(), clap_plugin.uuid().to_string(), width as i32, height as i32)) {
                                    Ok(_) => (),
                                    Err(error) => debug!("Problem relaying instrument Clap Host size window from CLAP thread to state: {}", error),
                                }
                            }
                        }
                    }
                }
            }
        }

        for effect_plugin in self.effect_plugin_instances.iter().chain(self.override_effect_plugin_instances.iter()) {
            match effect_plugin {
                BackgroundProcessorAudioPluginType::Vst24(_) => {
                    while let Ok(event) = effect_plugin.rx_from_host().try_recv() {
                        had_events = true;
                        match event {
                            AudioPluginHostOutwardEvent::Automation(_track_uuid, plugin_uuid, is_instrument, param_index, param_value) => {
                                match self.tx_vst_thread.send(TrackBackgroundProcessorOutwardEvent::Automation(self.track_uuid.clone(), plugin_uuid, is_instrument, param_index, param_value)) {
                                    Ok(_) => (),
                                    Err(error) => debug!("Problem relaying effect VstHost automation from VST thread to state: {}", error),
                                }
                            },
                            AudioPluginHostOutwardEvent::SizeWindow(_, plugin_uuid, _, width, height) => {
                                match self.tx_vst_thread.send(TrackBackgroundProcessorOutwardEvent::EffectPluginWindowSize(self.track_uuid.clone(), plugin_uuid, width, height)) {
                                    Ok(_) => (),
                                    Err(error) => debug!("Problem relaying effect VstHost size window from VST thread to state: {}", error),
                                }
                            },
                        }
                    }
                }
                BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                    while let Ok(event) = vst3_plugin.rx_from_host().try_recv() {
                        had_events = true;
                        match event {
                            AudioPluginHostOutwardEvent::Automation(_track_uuid, plugin_uuid, is_instrument, param_index, param_value) => {
                                match self.tx_vst_thread.send(TrackBackgroundProcessorOutwardEvent::Automation(self.track_uuid.clone(), plugin_uuid, is_instrument, param_index, param_value)) {
                                    Ok(_) => (),
                                    Err(error) => debug!("Problem relaying effect Vst3Host automation from VST3 thread to state: {}", error),
                                }
                            }
                            AudioPluginHostOutwardEvent::SizeWindow(_, plugin_uuid, _, width, height) => {
                                match self.tx_vst_thread.send(TrackBackgroundProcessorOutwardEvent::EffectPluginWindowSize(self.track_uuid.clone(), plugin_uuid, width, height)) {
                                    Ok(_) => (),
                                    Err(error) => debug!("Problem relaying effect Vst3Host size window from VST3 thread to state: {}", error),
                                }
                            }
                        }
                    }
                }
                BackgroundProcessorAudioPluginType::Clap(clap_plugin) => {
                    // this first event receive is a bit bogus because it should really happen inside the host but calling the clap plugin process method is done outside the host
                    while let Ok(event) = clap_plugin.rx_from_host().try_recv() {
                        had_events = true;
                        if let AudioPluginHostOutwardEvent::Automation(track_uuid, plugin_uuid, _is_instrument, param_index, param_value) = event {
                            match self.tx_vst_thread.send(TrackBackgroundProcessorOutwardEvent::Automation(track_uuid, plugin_uuid, true, param_index, param_value)) {
                                Ok(_) => (),
                                Err(error) => debug!("Problem relaying instrument Clap Host automation from CLAP thread to state: {}", error),
                            }
                        }
                    }
                    while let Ok(message) = clap_plugin.host_receiver.try_recv() {
                        had_events = true;
                        match message {
                            DAWCallback::PluginGuiWindowRequestResize(width, height) => {
                                // apply the resize to the plugin GUI first (CLAP wants the
                                // host to answer request_resize() with set_size()), then
                                // grow/shrink the host window to the resulting size
                                let (width, height) = clap_plugin.handle_gui_resize(width, height);
                                match self.tx_vst_thread.send(TrackBackgroundProcessorOutwardEvent::EffectPluginWindowSize(self.track_uuid.clone(), clap_plugin.uuid().to_string(), width as i32, height as i32)) {
                                    Ok(_) => (),
                                    Err(error) => debug!("Problem relaying effect Clap Host size window from CLAP thread to state: {}", error),
                                }
                            }
                        }
                    }
                }
            }
        }
        had_events
    }

    pub fn handle_request_plugin_preset_data(&mut self) {
        if let Some(response_sender) = self.pending_preset_response_sender.take() {
            let mut effect_presets = vec![];

            let instrument_preset = if let Some(instrument_plugin) = self.instrument_plugin_instances.get_mut(0) {
                instrument_plugin.preset_data()
            }
            else {
                String::from("")
            };

            for effect in self.effect_plugin_instances.iter_mut() {
                // uuid keyed so the save path can match presets to effects by identity
                // rather than position (a failed/removed effect used to shift every
                // later preset onto the wrong plugin).
                effect_presets.push((effect.uuid().to_string(), effect.preset_data()));
            }

            let override_instrument_preset = if let Some(instrument_plugin) = self.override_instrument_plugin_instances.get_mut(0) {
                instrument_plugin.preset_data()
            }
            else {
                String::from("")
            };

            let override_instrument_plugin_uuid = match self.override_instrument_plugin_instances.get(0) {
                Some(instrument_plugin) => instrument_plugin.uuid().to_string(),
                None => "".to_string(),
            };

            let mut override_effect_presets = vec![];
            for effect in self.override_effect_plugin_instances.iter_mut() {
                override_effect_presets.push((effect.uuid().to_string(), effect.preset_data()));
            }

            let response = TrackPluginPresetResponse {
                track_uuid: self.track_uuid.clone(),
                instrument_preset_data: instrument_preset,
                effect_presets,
                override_instrument_plugin_uuid,
                override_instrument_preset_data: override_instrument_preset,
                override_effect_presets,
            };

            match response_sender.send(response) {
                Ok(_) => debug!("Preset data sent for track uuid: {}", self.track_uuid.clone()),
                Err(error) => debug!("Problem sending preset data from VST thread to state: {:?}", error),
            }
        }
    }

    pub fn handle_request_instrument_plugin_parameters(&mut self) {
        debug!("Requested vst instrument parameter details");

        // send a parameter details event per loaded instrument instance (the track's own
        // instrument and any riff arrangement override instrument) - the receiving side
        // keys the parameter list by plugin uuid, so the automation views can offer the
        // parameters of either plugin.
        for instrument_plugin in self.instrument_plugin_instances.iter_mut().chain(self.override_instrument_plugin_instances.iter_mut()) {
            let instrument_plugin_uuid = instrument_plugin.uuid();
            let mut plugin_parameters = vec![];
            match instrument_plugin {
                BackgroundProcessorAudioPluginType::Vst24(instrument_plugin) => {
                    let instrument_info = instrument_plugin.vst_plugin_instance_mut().get_info();
                    let params = instrument_plugin.vst_plugin_instance_mut().get_parameter_object();
        
                    for index in 0..instrument_info.parameters {
                        // param index, track uuid, instrument uuid, param name, param label, param value, param text
                        plugin_parameters.push((index, self.track_uuid.clone(), instrument_plugin.uuid(), params.get_parameter_name(index), params.get_parameter_label(index), params.get_parameter(index), params.get_parameter_text(index)));
                    }
                }
                BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                    let parameterCount = vst3_plugin.get_parameter_count();

                    for index in 0..parameterCount {
                        let mut parameter = vst3_plugin.get_parameter(index);
                        parameter.1 = self.track_uuid.clone();
                        parameter.2 = vst3_plugin.uuid();
                        plugin_parameters.push(parameter);
                    }
                }
                BackgroundProcessorAudioPluginType::Clap(instrument_plugin) => {
                    // params::info()/get() are main-thread calls - fetch the rows over the RPC
                    if let ClapCallReply::Params(_info, rows) = instrument_plugin.call_clap(ClapPluginCall::EnumerateParams) {
                        for (param_id, name, value) in rows {
                            debug!("Parameter: index={}, param={}", param_id, name);
                            plugin_parameters.push((
                                param_id,
                                self.track_uuid.clone(),
                                instrument_plugin.uuid(),
                                name.clone(),
                                name,
                                value,
                                "".to_string()
                            ));
                        }
                    }
                }
            }

            match self.tx_vst_thread.send(TrackBackgroundProcessorOutwardEvent::InstrumentParameters(plugin_parameters)) {
                Ok(_) => debug!("Plugin parameter details sent for: uuid={}.", instrument_plugin_uuid),
                Err(error) => debug!("Problem sending instrument parameters data from VST thread to state: {}", error),
            }
        }
    }

    pub fn handle_request_effect_plugins_parameters(&mut self) {
        if self.request_effect_params {
            debug!("Requested vst effect parameter details");
            let mut plugin_parameters = vec![];
            for effect in self.effect_plugin_instances.iter_mut().chain(self.override_effect_plugin_instances.iter_mut()) {
                if effect.uuid().to_string() == self.request_effect_params_for_uuid {
                    match effect {
                        BackgroundProcessorAudioPluginType::Vst24(effect) => {
                            let effect_info = effect.vst_plugin_instance().get_info();
                            let params = effect.vst_plugin_instance_mut().get_parameter_object();
                            for index in 0..effect_info.parameters {
                                plugin_parameters.push((self.request_effect_params_for_uuid.clone(), index, params.get_parameter_name(index), params.get_parameter_label(index), params.get_parameter(index), params.get_parameter_text(index)));
                            }
                        }
                        BackgroundProcessorAudioPluginType::Vst3(_vst3_plugin) => {}
                        BackgroundProcessorAudioPluginType::Clap(effect) => {
                            // params::info()/get() are main-thread calls - fetch the rows over the RPC
                            if let ClapCallReply::Params(_info, rows) = effect.call_clap(ClapPluginCall::EnumerateParams) {
                                for (param_id, name, value) in rows {
                                    debug!("Parameter: index={}, param={}", param_id, name);
                                    // vector of plugin uuid, param index, param name, param label, param value, param text
                                    plugin_parameters.push((
                                        effect.uuid().to_string(),
                                        param_id,
                                        name.clone(),
                                        name,
                                        value,
                                        "".to_string()
                                    ));
                                }
                            }
                        }
                    }
                    break;
                }
            }
            match self.tx_vst_thread.send(TrackBackgroundProcessorOutwardEvent::EffectParameters(plugin_parameters)) {
                Ok(_) => debug!("Effect parameter details sent."),
                Err(error) => debug!("Problem sending effect parameters data from VST thread to state: {}", error),
            }
            self.request_effect_params = false;
        }

        // drain one queued override effect parameter request per poll
        if !self.request_effect_params {
            if let Some(next_override_effect_uuid) = self.pending_override_effect_param_requests.first().cloned() {
                self.pending_override_effect_param_requests.remove(0);
                self.request_effect_params = true;
                self.request_effect_params_for_uuid.clear();
                self.request_effect_params_for_uuid.push_str(next_override_effect_uuid.as_str());
            }
        }
    }

    /// Re-anchor every live plugin instance's song position to the frame position of
    /// the given block - called on each transport move (play from a block, rewind,
    /// scrub) so no plugin's tempo synced state ever free-runs away from the host.
    pub fn seek_plugin_transports(&mut self, block_index: i32) {
        let sample_position = (block_index.max(0) as i64) * (self.block_size as i64);
        for instrument_plugin in self.instrument_plugin_instances.iter_mut()
            .chain(self.override_instrument_plugin_instances.iter_mut()) {
            instrument_plugin.seek_transport(sample_position);
        }
        for effect_plugin in self.effect_plugin_instances.iter_mut()
            .chain(self.override_effect_plugin_instances.iter_mut()) {
            effect_plugin.seek_transport(sample_position);
        }
    }

    /// Total reported group delay of the active plugin chain (instrument plus
    /// effects, honouring the riff-arrangement override selection) in frames.
    pub fn track_plugin_latency_frames(&self) -> i64 {
        let mut total: i64 = 0;

        let use_override_instrument = self.override_instrument_active && !self.override_instrument_plugin_instances.is_empty();
        let instruments = if use_override_instrument { &self.override_instrument_plugin_instances } else { &self.instrument_plugin_instances };
        for instrument in instruments {
            total += instrument.latency_frames();
        }

        let use_override_effects = self.override_effects_active && !self.override_effect_plugin_instances.is_empty();
        let effects = if use_override_effects { &self.override_effect_plugin_instances } else { &self.effect_plugin_instances };
        for effect in effects {
            total += effect.latency_frames();
        }

        total
    }

    /// Move the plugin input stream's extraction ahead of the wall clock by whole
    /// blocks worth of the chain's reported group delay, and keep the jack layer's
    /// presentation timing informed for the inter-track audio routing case. Called
    /// every cycle because plugins and routings come and go without reconfiguring
    /// the transport.
    fn update_plugin_lookahead(&mut self) {
        let latency_frames = self.track_plugin_latency_frames().max(0) as f64;

        // a track fed by inter-track audio routing must NOT be input-offset: its
        // routed input arrives in real time (the source track cannot hand over its
        // future audio early), so offsetting the track's own samples/automation
        // would pull them ahead of the audio actually flowing through the chain.
        // Such a track instead presents its (chain-delayed) output late as a whole,
        // and reports that inherent delay for the master mix to align everything to.
        let routed_feed = !self.audio_inward_routings.is_empty();
        let lookahead_blocks = if !routed_feed && self.block_size > 0.0 {
            (latency_frames / self.block_size).ceil() as i32
        }
        else {
            0
        };
        self.event_processor.set_plugin_lookahead_blocks(lookahead_blocks);

        let inherent_blocks = if routed_feed && self.block_size > 0.0 {
            (latency_frames / self.block_size).ceil() as i32
        }
        else {
            0
        };
        if inherent_blocks != self.last_sent_presentation_inherent_blocks {
            self.last_sent_presentation_inherent_blocks = inherent_blocks;
            let _ = self.tx_audio.send(AudioLayerInwardEvent::TrackPresentation(self.track_uuid.clone(), inherent_blocks));
        }
    }

    pub fn process_plugin_events(&mut self) {
        self.update_plugin_lookahead();

        // get the events for this block
        let (mut events, param_events) = self.event_processor.process_events();
        // if !events.is_empty() {
        //     debug!("{} - process_plugin_events: event count={}", std::thread::current().name().unwrap_or_else(|| "unknown track"), events.len());
        // }

        // route outgoing events
        if events.len() > 0 {
            for (route_uuid, _) in self.track_events_outward_routings.iter() {
                if let Some(producer) = self.track_events_outward_producers.get_mut(route_uuid) {
                    let mut buffer: [TrackEvent; 1] = [TrackEvent::default()];

                    for event in events.iter() {
                        buffer[0] = event.clone();
                        let _ = producer.write(&buffer);
                    }
                }
            }
        }

        for (route_uuid, track_event_routing) in self.track_events_inward_routings.iter() {
            let mut buffer: [TrackEvent; 1] = [TrackEvent::default()];

            match track_event_routing.destination.clone() {
                TrackEventRoutingNodeType::Track(_) => {
                    if let Some(track_midi_input_consumer) = self.track_events_inward_consumers.get_mut(route_uuid) {
                        loop {
                            match track_midi_input_consumer.read(&mut buffer) {
                                Ok(entries_read) => {
                                    if entries_read == 1 {
                                        events.push(buffer[0].clone());
                                        debug!("TrackBackgroundProcessorHelper.process_plugin_events consumed a track event.")
                                    }
                                    else {
                                        break;
                                    }
                                }
                                Err(_) => {
                                    break;
                                }
                            }
                        }
                    }
                }
                TrackEventRoutingNodeType::Instrument(_, _) => {
                    if let Some(instrument_midi_input_consumer) = self.track_events_inward_consumers.get_mut(route_uuid) {
                        loop {
                            match instrument_midi_input_consumer.read(&mut buffer) {
                                Ok(entries_read) => {
                                    if entries_read == 1 {
                                        events.push(buffer[0].clone());
                                        debug!("TrackBackgroundProcessorHelper.process_plugin_events consumed an instrument event.")
                                    }
                                    else {
                                        break;
                                    }
                                }
                                Err(_) => {
                                    break;
                                }
                            }
                        }
                    }
                }
                TrackEventRoutingNodeType::Effect(_, effect_uuid) => { // process midi events routed to effect
                    let mut effect_events = vec![];

                    if let Some(effect_midi_input_consumer) = self.track_events_inward_consumers.get_mut(route_uuid) {
                        loop {
                            match effect_midi_input_consumer.read(&mut buffer) {
                                Ok(entries_read) => {
                                    if entries_read == 1 {
                                        effect_events.push(buffer[0].clone());
                                        debug!("TrackBackgroundProcessorHelper.process_plugin_events consumed an effect event.")
                                    }
                                    else {
                                        break;
                                    }
                                }
                                Err(_) => {
                                    break;
                                }
                            }
                        }
                        if !effect_events.is_empty() && !self.mute {
                            // routed events address a specific effect plugin by uuid - look
                            // in the track's effects first, then in the arrangement's override
                            // effects (events addressed to a non-override effect are never
                            // delivered to an override effect and vice versa)
                            let mut effect_plugin = self.effect_plugin_instances.iter_mut().find(|effect| effect.uuid().to_string() == effect_uuid);
                            if effect_plugin.is_none() {
                                effect_plugin = self.override_effect_plugin_instances.iter_mut().find(|effect| effect.uuid().to_string() == effect_uuid);
                            }
                            if let Some(effect_plugin) = effect_plugin {
                                let plugin_uuid = effect_plugin.uuid().to_string();
                                match effect_plugin {
                                    BackgroundProcessorAudioPluginType::Vst24(effect_plugin) => {
                                        let vst_plugin_instance = effect_plugin.vst_plugin_instance_mut();
                                        self.midi_sender.store_events(DAWUtils::convert_events_with_timing_in_frames_to_vst(&effect_events, 0));
                                        for event in effect_events.iter() {
                                            if let TrackEvent::AudioPluginParameter(plugin_parameter) = event {
                                                if plugin_uuid == plugin_parameter.plugin_uuid() {
                                                    vst_plugin_instance.get_parameter_object().set_parameter(plugin_parameter.index, plugin_parameter.value);
                                                }
                                            }
                                        }
                                        vst_plugin_instance.process_events(self.midi_sender.events());
                                    }
                                    BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                                        vst3_plugin.process_events(&effect_events);
                                    }
                                    BackgroundProcessorAudioPluginType::Clap(effect_plugin) => {
                                        debug!("{} - process_plugin_events: sending events to clap effect plugin, muted={}", std::thread::current().name().unwrap_or_else(|| "unknown track"), self.mute);
                                        effect_plugin.process_events(&effect_events);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // tempo events have no midi representation - consumed here by configuring
        // the track and its plugin instances exactly as the inward tempo event does
        // (the events are in position order so the last one in the block wins)
        for event in events.iter() {
            if let TrackEvent::Tempo(tempo) = event {
                self.tempo = tempo.bpm();
                if let Some(instrument_plugin) = self.instrument_plugin_instances.get_mut(0) {
                    instrument_plugin.set_tempo(tempo.bpm());
                }
                if let Some(instrument_plugin) = self.override_instrument_plugin_instances.get_mut(0) {
                    instrument_plugin.set_tempo(tempo.bpm());
                }
                for effect in self.effect_plugin_instances.iter_mut().chain(self.override_effect_plugin_instances.iter_mut()) {
                    effect.set_tempo(tempo.bpm());
                }
            }
        }

        // when the riff arrangement's instrument override is active the override plugin
        // instance receives all the events that would normally go to the track's own
        // instrument - except plugin parameter events addressed (by plugin uuid) to a
        // particular other plugin instance, which stay with that plugin
        let use_override_instrument = self.override_instrument_active && !self.override_instrument_plugin_instances.is_empty();
        let instrument_events: Vec<TrackEvent> = if use_override_instrument {
            let override_plugin_uuid = match self.override_instrument_plugin_instances.get(0) {
                Some(instrument_plugin) => instrument_plugin.uuid().to_string(),
                None => "".to_string(),
            };
            events.into_iter().filter(|event| match event {
                TrackEvent::AudioPluginParameter(plugin_parameter) => plugin_parameter.plugin_uuid() == override_plugin_uuid,
                _ => true,
            }).collect()
        }
        else {
            std::mem::take(&mut events)
        };

        if !instrument_events.is_empty() && !self.mute {
            let instrument_plugin = if use_override_instrument {
                self.override_instrument_plugin_instances.get_mut(0)
            }
            else {
                self.instrument_plugin_instances.get_mut(0)
            };
            if let Some(instrument_plugin) = instrument_plugin {
                let plugin_uuid = instrument_plugin.uuid().to_string();
                match instrument_plugin {
                    BackgroundProcessorAudioPluginType::Vst24(instrument_plugin) => {
                        let vst_midi_events = DAWUtils::convert_events_with_timing_in_frames_to_vst(
                            &instrument_events, 
                            0);
                        let vst_plugin_instance = instrument_plugin.vst_plugin_instance_mut();
                        self.midi_sender.store_events(vst_midi_events);
                        for event in instrument_events.iter() {
                            if let TrackEvent::AudioPluginParameter(plugin_parameter) = event {
                                if plugin_uuid == plugin_parameter.plugin_uuid() {
                                    debug!("Sent a plugin parameter to the instrument plugin.");
                                    vst_plugin_instance.get_parameter_object().set_parameter(plugin_parameter.index, plugin_parameter.value);
                                }
                            }
                        }
                        vst_plugin_instance.process_events(self.midi_sender.events());
                    }
                    BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                        vst3_plugin.process_events(&instrument_events);
                    }
                    BackgroundProcessorAudioPluginType::Clap(instrument_plugin) => {
                        // debug!("{} - process_plugin_events: sending events to clap instrument plugin, muted={}", std::thread::current().name().unwrap_or_else(|| "unknown track"), self.mute);
                        instrument_plugin.process_events(&instrument_events);
                    }
                }
            }
        }

        if !param_events.is_empty() && !self.mute {
            // plugin parameter events remain addressed to their own plugin instance (by
            // plugin uuid) - the track's instrument/effects and the arrangement's override
            // instrument/effects each only receive the parameters that refer to them, so
            // automation created against a non-override plugin is never sent to an
            // override plugin and vice versa
            for instrument_plugin in self.instrument_plugin_instances.iter_mut().chain(self.override_instrument_plugin_instances.iter_mut()) {
                Self::apply_plugin_param_events(instrument_plugin, &param_events);
            }

            // send parameters to the track effects (and the arrangement's override effects)
            for effect_plugin in self.effect_plugin_instances.iter_mut().chain(self.override_effect_plugin_instances.iter_mut()) {
                Self::apply_plugin_param_events(effect_plugin, &param_events);
            }
        }
    }

    /// Apply plugin parameter (automation) events to a single live plugin instance,
    /// filtered by the instance's plugin uuid.
    fn apply_plugin_param_events(plugin: &mut BackgroundProcessorAudioPluginType, param_events: &[PluginParameter]) {
        match plugin {
            BackgroundProcessorAudioPluginType::Vst24(plugin) => {
                let plugin_uuid = plugin.uuid().to_string();
                let vst_plugin_instance = plugin.vst_plugin_instance_mut();
                let param_object = vst_plugin_instance.get_parameter_object();
                for event in param_events.iter() {
                    if event.plugin_uuid() == plugin_uuid {
                        param_object.set_parameter(event.index, event.value());
                    }
                }
            }
            BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                let plugin_param_events: Vec<&PluginParameter> = param_events.iter().filter(|param| param.plugin_uuid() == vst3_plugin.uuid().to_string()).collect();
                vst3_plugin.process_param_events(&plugin_param_events);
            }
            BackgroundProcessorAudioPluginType::Clap(plugin) => {
                // params::info() is a main-thread call - use the snapshot taken at
                // creation so parameter events stay cheap on this thread
                let plugin_param_events: Vec<&PluginParameter> = param_events.iter().filter(|param| param.plugin_uuid() == plugin.uuid().to_string()).collect();
                if !plugin_param_events.is_empty() {
                    if let Some(param_info) = plugin.clap_param_info() {
                        plugin.process_param_events(&plugin_param_events, param_info);
                    }
                }
            }
        }
    }

    pub fn process_audio_events(&mut self) {
        self.update_plugin_lookahead();

        let (events, param_events) = self.event_processor.process_events();

        if !events.is_empty() && !self.mute {
            // look at the events and determine when to start playing or stop a sample.
            // FIXME this only has block resolution at the moment - will need a start delay and end delay field members to get frame accuracy
            for event in events.iter() {
                if let TrackEvent::NoteOn(_) = event {
                    self.sample_is_playing = true;
                    self.sample_current_frame = 0;
                }
                else if let TrackEvent::NoteOff(_) = event {
                    self.sample_is_playing = false;
                    self.sample_current_frame = 0;
                }
            }
        }

        if !param_events.is_empty() && !self.mute {
            // send parameters to the track effects (and the arrangement's override effects)
            for effect_plugin in self.effect_plugin_instances.iter_mut().chain(self.override_effect_plugin_instances.iter_mut()) {
                Self::apply_plugin_param_events(effect_plugin, &param_events);
            }
        }
    }

    pub fn process_jack_midi_out_events(&mut self, producer: &mut Producer<(u32, u8, u8, u8, bool)>) {
        // get the events for this block
        let (track_events, _) = self.event_processor.process_events();
        let mut jack_events: Vec<(u32, u8, u8, u8, bool)> = vec![];

        for event in track_events.iter() {
            match event {
                TrackEvent::NoteOn(note_on) => jack_events.push((
                    note_on.position() as u32,
                    144 + note_on.channel() as u8,
                    note_on.note() as u8,
                    note_on.velocity() as u8,
                    false,
                )),
                TrackEvent::NoteOff(note_off) => jack_events.push((
                    note_off.position() as u32,
                    128 + note_off.channel() as u8,
                    note_off.note() as u8,
                    0,
                    false,
                )),
                _ => (),
            }
        }

        if !self.jack_midi_out_immediate_events.is_empty() {
            for event in self.jack_midi_out_immediate_events.iter() {
                jack_events.push((event.delta_frames as u32, event.data[0], event.data[1], event.data[2], true));
            }
        }
        self.jack_midi_out_immediate_events.clear();

        // zero the buffer
        for index in 0..self.jack_midi_out_buffer.len() {
            self.jack_midi_out_buffer[index].0 = 0;
            self.jack_midi_out_buffer[index].1 = 0;
            self.jack_midi_out_buffer[index].2 = 0;
            self.jack_midi_out_buffer[index].3 = 0;
            self.jack_midi_out_buffer[index].4 = false;
        }

        if !jack_events.is_empty() && !self.mute && !self.coast() {
            for index in 0..jack_events.len() {
                if index < self.jack_midi_out_buffer.len() {
                    // debug!("Copied event to the jack midi buffer: {}", events.len());
                    self.jack_midi_out_buffer[index].0 = jack_events[index].0;
                    self.jack_midi_out_buffer[index].1 = jack_events[index].1;
                    self.jack_midi_out_buffer[index].2 = jack_events[index].2;
                    self.jack_midi_out_buffer[index].3 = jack_events[index].3;
                    self.jack_midi_out_buffer[index].4 = true;
                }
            }
        }

        write_blocking_or_exit(&*producer, &self.jack_midi_out_buffer);
    }

    pub fn coast(&self) -> bool {
        match self.track_thread_coast.lock() {
            Ok(mode) => match *mode {
                TrackBackgroundProcessorMode::AudioOut => false,
                TrackBackgroundProcessorMode::Coast => true,
                TrackBackgroundProcessorMode::Render => true,
            }
            Err(_) => false
        }
    }

    pub fn send_render_audio_consumer_details_to_app(&self, track_render_audio_consumer_details: AudioConsumerDetails<AudioBlock>) {
        match self.tx_vst_thread.send(TrackBackgroundProcessorOutwardEvent::TrackRenderAudioConsumer(track_render_audio_consumer_details)) {
            Ok(_) => (),
            Err(_) => debug!("AudioPlugin could not send render audio consumer detail."),
        }
    }

    pub fn send_audio_consumer_details_to_jack(&self, audio_consumer_details: AudioConsumerDetails<AudioBlock>) {
        match self.tx_audio.send(AudioLayerInwardEvent::NewAudioConsumer(audio_consumer_details)) {
            Ok(_) => (),
            Err(_) => debug!("AudioPlugin could not send audio consumer detail."),
        }
    }

    pub fn send_midi_consumer_details_to_jack(&self, midi_consumer_details: MidiConsumerDetails<(u32, u8, u8, u8, bool)>) {
        match self.tx_audio.send(AudioLayerInwardEvent::NewMidiConsumer(midi_consumer_details)) {
            Ok(_) => debug!("Sent midi consumer detail to the audio layer."),
            Err(_) => debug!("AudioPlugin could not send midi consumer detail."),
        }
    }

    pub fn process_sample(&mut self, audio_buffer: &mut AudioBuffer<f32>, block_size: i32, left_pan: f32, right_pan: f32) {
        if self.sample_is_playing {
            let sample_channels = if let Some(sample) = self.sample() {
                sample.channels() as usize
            } else {
                0
            };
            let sample_length = if let Some(sample) = self.sample() {
                sample.samples().len()
            } else {
                0
            };

            // let mut left_samples: Vec<f32> = vec![];
            // let mut right_samples: Vec<f32> = vec![];
            let sample_current_frame = self.sample_current_frame;
            let volume = self.volume;
            let (_, mut outputs_32) = audio_buffer.split();
            let out_left = outputs_32.get_mut(0);
            let out_right = outputs_32.get_mut(1);
            if let Some(sample) = self.sample_mut() {
                for frame in 0..block_size {
                    let left_channel_sample_index = (frame + sample_current_frame) as usize * sample_channels;
                    let right_channel_sample_index = left_channel_sample_index + 1;

                    if left_channel_sample_index < sample_length {
                        if sample.channels() == 1 {
                            // left_samples.push(*sample.samples().get(left_channel_sample_index).unwrap());
                            // right_samples.push(*sample.samples().get(left_channel_sample_index).unwrap());
                            out_left[frame as usize] += *sample.samples().get(left_channel_sample_index).unwrap() * volume * 2.0 * left_pan;
                            out_right[frame as usize] += *sample.samples().get(left_channel_sample_index).unwrap() * volume * 2.0 * right_pan;
                        } else {
                            // left_samples.push(*sample.samples().get(left_channel_sample_index).unwrap());
                            // right_samples.push(*sample.samples().get(right_channel_sample_index).unwrap());
                            out_left[frame as usize] += *sample.samples().get(left_channel_sample_index).unwrap() * volume * 2.0 * left_pan;
                            out_right[frame as usize] += *sample.samples().get(right_channel_sample_index).unwrap() * volume * 2.0 * right_pan;
                        }
                    }
                }
            }

            self.sample_current_frame += block_size;

            // let mut frame: usize = 0;
            // let (_, mut outputs_32) = audio_buffer.split();
            // let out_left = outputs_32.get_mut(0);
            // let out_right = outputs_32.get_mut(1);
            // for left_sample in left_samples.iter() {
            //     out_left[frame] += *left_sample * self.volume * 2.0 * left_pan;
            //     out_right[frame] += *right_samples.get(frame).unwrap() as f32 * self.volume * 2.0 * right_pan;
            //     frame += 1;
            // }
        }
    }

    // pub fn process_plugin_audio(&mut self, audio_buffer: &mut AudioBuffer<f32>, audio_buffer_swapped: &mut AudioBuffer<f32>, producer_left: Producer<f32>, producer_right: Producer<f32>) {
    //     if let Some(instrument_plugin) = self.instrument_plugin_instances.get_mut(0) {
    //         let vst_plugin_instance = instrument_plugin.vst_plugin_instance_mut();
    //         vst_plugin_instance.process(audio_buffer);
    //     }
    //
    //     let mut swap = true;
    //     for effect in self.effect_plugin_instances.iter_mut() {
    //         let audio_buffer_in_use = if swap {
    //             audio_buffer_swapped
    //         }
    //         else {
    //             audio_buffer
    //         };
    //         swap = !swap;
    //         effect.vst_plugin_instance_mut().process(audio_buffer_in_use);
    //     }
    //
    //     // transfer to the ring buffer
    //     let (_, mut outputs_32) = audio_buffer.split();
    //
    //     let coast = if let Ok(zzzcoast) = self.track_thread_coast.lock() {
    //         *zzzcoast
    //     }
    //     else {
    //         false
    //     };
    //
    //     // debug!(" - Writing to producer...");
    //     if !coast {
    //         // debug!("blocking: {}...", track_uuid.clone());
    //         producer_left.write_blocking(outputs_32.get_mut(0));
    //         producer_right.write_blocking(outputs_32.get_mut(1));
    //         // debug!("unblocked: {}", track_uuid.clone());
    //     }
    //     else {
    //         thread::sleep(Duration::from_millis(100));
    //     }
    // }
    pub fn sample(&self) -> &Option<SampleData> {
        &self.sample
    }
    pub fn sample_mut(&mut self) -> &Option<SampleData> {
        &self.sample
    }
    pub fn set_sample(&mut self, sample: Option<SampleData>) {
        self.sample = sample;
    }

    pub fn add_track_event_inward_routing(&mut self, track_event_routing: TrackEventRouting, track_event_source: Consumer<TrackEvent>) {
        self.track_events_inward_consumers.insert(track_event_routing.uuid(), track_event_source);
        self.track_events_inward_routings.insert(track_event_routing.uuid(), track_event_routing);
    }

    pub fn remove_track_event_inward_routing(&mut self, route_uuid: String) {
        self.track_events_inward_consumers.remove(&route_uuid);
        self.track_events_inward_routings.remove(&route_uuid);
    }

    pub fn add_audio_inward_routing(&mut self, audio_routing: AudioRouting, audio_sources: (Consumer<f32>, Consumer<f32>)) {
        self.audio_inward_consumers.insert(audio_routing.uuid(), audio_sources);
        self.audio_inward_routings.insert(audio_routing.uuid(), audio_routing);
    }

    pub fn remove_audio_inward_routing(&mut self, route_uuid: String) {
        self.audio_inward_consumers.remove(&route_uuid);
        self.audio_inward_routings.remove(&route_uuid);
    }
}

#[derive(Clone, Copy)]
pub struct AudioBlock {
    pub block: i32,
    pub audio_data_left: [f32; BLOCK_SIZE_MAX as usize],
    pub audio_data_right: [f32; BLOCK_SIZE_MAX as usize],
}

impl Default for AudioBlock {
    fn default() -> Self {
        Self { 
            block: 0, 
            audio_data_left: [0.0f32; BLOCK_SIZE_MAX as usize],
            audio_data_right: [0.0f32; BLOCK_SIZE_MAX as usize] }
    }
}

pub trait TrackBackgroundProcessor {
    fn start_processing(&self,
                        track_uuid: String,
                        tx_audio: crossbeam_channel::Sender<AudioLayerInwardEvent>,
                        rx_track_background_thread: Receiver<TrackBackgroundProcessorInwardEvent>,
                        tx_track_background_thread: Sender<TrackBackgroundProcessorOutwardEvent>,
                        track_thread_coast: Arc<Mutex<TrackBackgroundProcessorMode>>,
                        volume: f32,
                        pan: f32,
                        vst_host_time_info: Arc<RwLock<TimeInfo>>,
                        sample_rate: f64,
                        block_size: f64,
                        tempo: f64,
                        time_signature_numerator: i32,
                        time_signature_denominator: i32,
    );
}

#[derive(Default)]
pub struct InstrumentTrackBackgroundProcessor {
}

impl InstrumentTrackBackgroundProcessor {
    pub fn new() -> Self {
        InstrumentTrackBackgroundProcessor {}
    }
}

impl TrackBackgroundProcessor for InstrumentTrackBackgroundProcessor {
    fn start_processing(&self,
                        track_uuid: String,
                        tx_audio: crossbeam_channel::Sender<AudioLayerInwardEvent>,
                        rx_track_background_thread: Receiver<TrackBackgroundProcessorInwardEvent>,
                        tx_track_background_thread: Sender<TrackBackgroundProcessorOutwardEvent>,
                        track_thread_coast: Arc<Mutex<TrackBackgroundProcessorMode>>,
                        volume: f32,
                        pan: f32,
                        vst_host_time_info: Arc<RwLock<TimeInfo>>,
                        sample_rate: f64,
                        block_size: f64,
                        tempo: f64,
                        time_signature_numerator: i32,
                        time_signature_denominator: i32,
    ) {
        match ThreadBuilder::default()
            .name(format!("InstrumentTrackBackgroundProcessor: {}", track_uuid.as_str()))
            .priority(ThreadPriority::Crossplatform(95.try_into().unwrap()))
            .spawn(move |result| {
                match result {
                    Ok(_) => debug!("Thread set to max priority: 95."),
                    Err(error) => debug!("Could not set thread to max priority: {:?}.", error),
                }

                // keep denormal FP arithmetic from silently eating this thread's cpu
                crate::domain::flush_denormals_to_zero();
                // counted live until this closure's locals (including the helper and
                // its plugin handles) are gone - main() waits for zero before letting
                // the CLAP registry destroy instances
                let _liveness_guard = crate::domain::TrackThreadLivenessGuard::new();

                const HOST_BUFFER_CHANNELS: usize = 32;

                let render_ring_buffer_block: SpscRb<AudioBlock> = SpscRb::new(2);
                let render_producer_block = render_ring_buffer_block.producer();
                let render_consumer_block = render_ring_buffer_block.consumer();
                let track_render_audio_consumer_details =
                    AudioConsumerDetails::<AudioBlock>::new(track_uuid.clone(), render_consumer_block);

                // Live monitoring ring: the producer free-runs and keeps this ring full,
                // so every slot is one audio block of pure latency on the jack output
                // (up to ~46ms per slot at 2048/44.1k). A single slot still hand-offs the
                // block one cycle ahead of the jack consumption; a producer that is late
                // only misses its own block (the mixer tolerates an empty read).
                let ring_buffer_block: SpscRb<AudioBlock> = SpscRb::new(1);
                let producer_ring_buffer_block = ring_buffer_block.producer();
                let consumer_ring_buffer_block = ring_buffer_block.consumer();
                let audio_consumer_details = AudioConsumerDetails::<AudioBlock>::new(track_uuid.clone(), consumer_ring_buffer_block);

                let mut host_buffer: HostBuffer<f32> = HostBuffer::new(HOST_BUFFER_CHANNELS, HOST_BUFFER_CHANNELS);
                let mut host_buffer_swapped: HostBuffer<f32> = HostBuffer::new(HOST_BUFFER_CHANNELS, HOST_BUFFER_CHANNELS);
                let mut inputs: Vec<Vec<f32>> = vec![vec![0.0; BLOCK_SIZE_MAX as usize]; HOST_BUFFER_CHANNELS];
                let mut outputs: Vec<Vec<f32>> = vec![vec![0.0; BLOCK_SIZE_MAX as usize]; HOST_BUFFER_CHANNELS];
                let mut audio_buffer = host_buffer.bind(&inputs, &mut outputs);
                let mut audio_buffer_swapped = host_buffer_swapped.bind(&outputs, &mut inputs);

                const ITERATIONS_UNTIL_REFRESH_PLUGIN_EDITORS: i32 = 5;
                let mut iteration_count = 0;

                // DSP budget telemetry: a track whose plugin dispatch routinely eats most
                // of the block period is what drags the whole graph into jack overruns
                // (the audio layer going to noise part way into a long arrangement).
                let mut dsp_slow_cycles: u32 = 0;
                let mut dsp_window_cycles: u32 = 0;

                let mut track_background_processor_helper =
                    TrackBackgroundProcessorHelper::new(
                        track_uuid.clone(),
                        tx_audio.clone(),
                        rx_track_background_thread,
                        tx_track_background_thread.clone(),
                        track_thread_coast.clone(),
                        volume,
                        pan,
                        GeneralTrackType::InstrumentTrack,
                        vst_host_time_info,
                        Box::new(RiffBufferTrackEventProcessor::new(block_size as f64)),
                        sample_rate,
                        block_size,
                        tempo,
                        time_signature_numerator,
                        time_signature_denominator,
                    );

                let mut routed_audio_left_buffer: [f32; BLOCK_SIZE_MAX as usize] = [0.0; BLOCK_SIZE_MAX as usize];
                let mut routed_audio_right_buffer: [f32; BLOCK_SIZE_MAX as usize] = [0.0; BLOCK_SIZE_MAX as usize];

                let mut priority_reconciled = false;

                let audio_block = AudioBlock::default();
                let mut audio_block_buffer = vec![audio_block];

                track_background_processor_helper.send_render_audio_consumer_details_to_app(track_render_audio_consumer_details);
                track_background_processor_helper.send_audio_consumer_details_to_jack(audio_consumer_details);
                // track_background_processor_helper.send_midi_consumer_details_to_jack(midi_consumer_details);

                // P5 quiescence: after the transport has been stopped and this track has
                // produced several consecutive silent blocks, stop running the plugins
                // (the dsp is the expensive part) but keep sending silent blocks at the
                // block rate - the jack layer waits for a block from every consumer before
                // it mixes, so a track that stopped sending altogether would starve every
                // other track's live monitoring. Any inbound event (note immediate,
                // transport play, preset/param requests, routing changes...), any relayed
                // plugin host event or an open plugin editor wakes the track back up on the
                // next poll - no slower than the event polling that happens today.
                const SILENT_BLOCKS_BEFORE_QUIESCE: i32 = 8;
                const SILENCE_THRESHOLD: f32 = 1e-5;
                let mut silent_block_run: i32 = 0;
                let mut quiescent = false;

                loop {
                    // this thread must never outrank the jack callback thread (see helper)
                    if !priority_reconciled {
                        priority_reconciled = crate::domain::reconcile_track_thread_priority();
                    }

                    // the Kill event handler stops the plugins and clears keep_alive:
                    // leave the loop so no thread can be inside process() when the
                    // application main thread destroys the plugin instances at exit
                    if !track_background_processor_helper.keep_alive {
                        info!("Track background processor thread for track {} got the Kill and is leaving its loop.", track_uuid.as_str());
                        break;
                    }

                    // lifecycle events (plugin instantiate/teardown/preset/configure) hold
                    // the write lock so no other track thread can be dispatching to a VST2
                    // plugin while process-wide plugin state is being (re)initialised.
                    let had_inbound_events = {
                        let _vst24_lifecycle_guard = crate::domain::VST24_PLUGIN_LIFECYCLE_LOCK.write();
                        track_background_processor_helper.handle_incoming_events()
                    };

                    // the Kill may have arrived in THIS pass: its handler just
                    // stopped/suspended/closed the plugins - falling through
                    // would dispatch process() onto those closed instances
                    // (which is what segfaulted inside the JUCE VST2 wrapper),
                    // so leave the loop right now instead of at the next check.
                    if !track_background_processor_helper.keep_alive {
                        info!("Track background processor thread for track {} got the Kill and is leaving its loop.", track_uuid.as_str());
                        break;
                    }

                    {
                        // editor idling and preset/parameter fetches are non-audio plugin
                        // calls - keep them out the way of any lifecycle work in progress.
                        let _vst24_dispatch_guard = crate::domain::VST24_PLUGIN_LIFECYCLE_LOCK.read();

                        if iteration_count % ITERATIONS_UNTIL_REFRESH_PLUGIN_EDITORS == 0 {
                            track_background_processor_helper.refresh_instrument_plugin_editor();
                            track_background_processor_helper.refresh_effect_plugin_editors();
                        }

                        track_background_processor_helper.handle_request_plugin_preset_data();
                        track_background_processor_helper.handle_request_effect_plugins_parameters();
                    }
                    iteration_count += 1;

                    let had_plugin_host_events = track_background_processor_helper.handle_host_events_from_plugins();

                    if quiescent {
                        let track_wakeup_needed = had_inbound_events
                            || had_plugin_host_events
                            || *track_background_processor_helper.event_processor.play()
                            || track_background_processor_helper.plugin_editor_open();
                        if track_wakeup_needed {
                            quiescent = false;
                            silent_block_run = 0;
                        }
                        else {
                            let quiet_mode = match track_thread_coast.lock() {
                                Ok(mode) => match *mode {
                                    TrackBackgroundProcessorMode::AudioOut => TrackBackgroundProcessorMode::AudioOut,
                                    TrackBackgroundProcessorMode::Coast => TrackBackgroundProcessorMode::Coast,
                                    TrackBackgroundProcessorMode::Render => TrackBackgroundProcessorMode::Render,
                                }
                                Err(_) => TrackBackgroundProcessorMode::AudioOut,
                            };
                            match quiet_mode {
                                TrackBackgroundProcessorMode::Render => {
                                    // a render is in flight - stay awake and keep processing
                                    quiescent = false;
                                    silent_block_run = 0;
                                }
                                TrackBackgroundProcessorMode::AudioOut => {
                                    let audio_block = audio_block_buffer.get_mut(0).unwrap();
                                    audio_block.block = *track_background_processor_helper.event_processor.block_index();
                                    audio_block.audio_data_left.fill(0.0);
                                    audio_block.audio_data_right.fill(0.0);
                                    write_blocking_or_exit(&producer_ring_buffer_block, &audio_block_buffer);
                                    let _ = tx_track_background_thread.send(TrackBackgroundProcessorOutwardEvent::ChannelLevels(track_uuid.clone(), 0.0, 0.0));
                                }
                                TrackBackgroundProcessorMode::Coast => {
                                    thread::sleep(Duration::from_millis(100));
                                }
                            }
                            continue;
                        }
                    }
                    // track_background_processor_helper.dump_play_info();
                    // track_background_processor_helper.process_plugin_events();
                    let mut swap = true;
                    {
                    // the per-block plugin dispatch calls read (and are read by) the same
                    // framework state lifecycle work re-initialises - read lock while any
                    // track thread might be instantiating or loading a VST2 plugin.
                    let _vst24_process_guard = crate::domain::VST24_PLUGIN_LIFECYCLE_LOCK.read();
                    let dsp_start = std::time::Instant::now();
                    track_background_processor_helper.process_plugin_events();

                    // zero the live output channels for the block about to be produced.
                    // a plugin that has died or stopped (or one that never writes, like a
                    // track with effects but no instrument) would otherwise leave the
                    // previous block's audio sitting in these buffers to be replayed every
                    // cycle - machine-gunned audio that sounds like harsh noise.
                    {
                        let (_, mut channels) = audio_buffer.split();
                        channels.get_mut(0).fill(0.0);
                        channels.get_mut(1).fill(0.0);
                    }
                    {
                        let (_, mut channels) = audio_buffer_swapped.split();
                        channels.get_mut(0).fill(0.0);
                        channels.get_mut(1).fill(0.0);
                    }

                    // couldn't push the following into a member method because of lifetime issues
                    // the audio configuration (sample rate, block size) and tempo are read from
                    // the helper on every iteration so that an AudioConfigurationChange event
                    // received by handle_incoming_events takes effect immediately.
                    let current_sample_rate = track_background_processor_helper.sample_rate;
                    let current_block_size = track_background_processor_helper.block_size as i32;
                    let current_tempo = track_background_processor_helper.tempo;
                    let ppq_pos = ((track_background_processor_helper.event_processor.block_index() * (track_background_processor_helper.block_size as i32)) as f64  * current_tempo / (60.0 * current_sample_rate)) + 1.0;
                    let sample_position = (track_background_processor_helper.event_processor.block_index() * (track_background_processor_helper.block_size as i32)) as f64;

                    // the riff arrangement instrument override instance is processed in
                    // place of the track's own instrument while the override is switched on
                    let use_override_instrument = track_background_processor_helper.override_instrument_active && !track_background_processor_helper.override_instrument_plugin_instances.is_empty();
                    let instrument_plugin = if use_override_instrument {
                        track_background_processor_helper.override_instrument_plugin_instances.get_mut(0)
                    }
                    else {
                        track_background_processor_helper.instrument_plugin_instances.get_mut(0)
                    };
                    if let Some(instrument_plugin) = instrument_plugin {
                        match instrument_plugin {
                            BackgroundProcessorAudioPluginType::Vst24(instrument_plugin) => {
                                if let Ok(mut vst_host) = instrument_plugin.host_mut().lock() {
                                    vst_host.set_ppq_pos(ppq_pos);
                                    vst_host.set_sample_position(sample_position);
                                }
                                let vst_plugin_instance = instrument_plugin.vst_plugin_instance_mut();
                                vst_plugin_instance.process(&mut audio_buffer, current_block_size);
                            }
                            BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                                vst3_plugin.process(&mut audio_buffer);
                            }
                            BackgroundProcessorAudioPluginType::Clap(instrument_plugin) => {
                                // the CLAP gui timer/fd pumping runs on the application
                                // main thread (clap_registry_pump) - those are main-thread
                                // plugin calls
                                instrument_plugin.process(&mut audio_buffer, false);
                            }
                        }
                    }

                    // the arrangement's override effects chain processes the instrument audio
                    // in place of the track's own effects while the override is switched on
                    let mut swap = true;
                    let use_override_effects = track_background_processor_helper.override_effects_active && !track_background_processor_helper.override_effect_plugin_instances.is_empty();
                    let active_effect_plugin_instances = if use_override_effects {
                        &mut track_background_processor_helper.override_effect_plugin_instances
                    }
                    else {
                        &mut track_background_processor_helper.effect_plugin_instances
                    };
                    for effect in active_effect_plugin_instances.iter_mut() {
                        // handle audio data routed to this effect
                        for audio_route_uuid in track_background_processor_helper.audio_inward_routings.iter().find(|(_, audio_route)| match &audio_route.destination {
                            AudioRoutingNodeType::Track(_) => false,
                            AudioRoutingNodeType::Instrument(_, _, _, _) => false,
                            // parse (no allocation) and compare as Uuid instead of
                            // formatting both sides to String per audio block
                            AudioRoutingNodeType::Effect(_, effect_uuid, _, _) => match effect_uuid.parse::<Uuid>() {
                                Ok(parsed_uuid) => parsed_uuid == effect.uuid(),
                                Err(_) => false,
                            },
                        }).map(|(_, audio_routing)| audio_routing.uuid()).iter() {
                            if let Some((consumer_left, consumer_right)) = track_background_processor_helper.audio_inward_consumers.get_mut(audio_route_uuid) {
                                let (_, mut outputs_32) = audio_buffer.split();

                                for index in 0..routed_audio_left_buffer.len() {
                                    routed_audio_left_buffer[index] = 0.0;
                                    routed_audio_right_buffer[index] = 0.0;
                                }

                                if let Ok(read) = consumer_left.read(&mut routed_audio_left_buffer) {
                                    let left_channel = outputs_32.get_mut(2);

                                    for index in 0..read {
                                        left_channel[index] = routed_audio_left_buffer[index];
                                    }
                                }

                                if let Ok(read) = consumer_right.read(&mut routed_audio_right_buffer) {
                                    let right_channel = outputs_32.get_mut(3);

                                    for index in 0..read {
                                        right_channel[index] = routed_audio_right_buffer[index];
                                    }
                                }
                            }
                        }

                        let audio_buffer_in_use = if swap {
                            &mut audio_buffer_swapped
                        }
                        else {
                            &mut audio_buffer
                        };
                        swap = !swap;

                        match effect {
                            BackgroundProcessorAudioPluginType::Vst24(effect) => {
                                if let Ok(mut vst_host) = effect.host_mut().lock() {
                                    vst_host.set_ppq_pos(ppq_pos);
                                    vst_host.set_sample_position(sample_position);
                                }
                                effect.vst_plugin_instance_mut().process(audio_buffer_in_use, current_block_size);
                            }
                            BackgroundProcessorAudioPluginType::Vst3(vst3_plugin) => {
                                vst3_plugin.process(audio_buffer_in_use);
                            }
                            BackgroundProcessorAudioPluginType::Clap(effect) => {
                                // the CLAP gui timer/fd pumping runs on the application
                                // main thread (clap_registry_pump)
                                effect.process(audio_buffer_in_use, true);
                            }
                        }
                    }
                    // measure the per-block plugin dispatch cost against the block period
                    dsp_window_cycles += 1;
                    let block_period_micros = track_background_processor_helper.block_size / track_background_processor_helper.sample_rate * 1_000_000.0;
                    if dsp_start.elapsed().as_micros() as f64 > block_period_micros * 0.7 {
                        dsp_slow_cycles += 1;
                    }
                    if dsp_window_cycles >= 512 {
                        if dsp_slow_cycles > 96 {
                            debug!("Track {} plugin dispatch exceeded 70% of the block period in {} of {} cycles - this track risks starving the jack cycle", track_uuid, dsp_slow_cycles, dsp_window_cycles);
                        }
                        dsp_slow_cycles = 0;
                        dsp_window_cycles = 0;
                    }
                    } // end of the read-locked per-block plugin dispatch section

                    let mode = match track_thread_coast.lock() {
                        Ok(mode) => match *mode {
                            TrackBackgroundProcessorMode::AudioOut => TrackBackgroundProcessorMode::AudioOut,
                            TrackBackgroundProcessorMode::Coast => TrackBackgroundProcessorMode::Coast,
                            TrackBackgroundProcessorMode::Render => TrackBackgroundProcessorMode::Render,
                        }
                        Err(_) => TrackBackgroundProcessorMode::AudioOut,
                    };

                    // swap to the last used audio buffer
                    let (left_pan, right_pan) = DAWUtils::constant_power_stereo_pan(track_background_processor_helper.pan);
                    let mut left_channel_level: f32 = 0.0;
                    let mut right_channel_level: f32 = 0.0;
                    let audio_buffer_in_use = if !swap {
                        &mut audio_buffer_swapped
                    }
                    else {
                        &mut audio_buffer
                    };
                    // route to other audio destinations
                    for (_, (producer_left, producer_right)) in track_background_processor_helper.audio_outward_producers.iter_mut() {
                        let (_, mut outputs_32) = audio_buffer_in_use.split();
                        let left_channel = outputs_32.get_mut(0);
                        let mut index = 0;

                        for index in 0..routed_audio_left_buffer.len() {
                            if index >= track_background_processor_helper.block_size as usize { break; }
                            routed_audio_left_buffer[index] = 0.0;
                            routed_audio_right_buffer[index] = 0.0;
                        }

                        for left_frame in left_channel.iter() {
                            if index >= track_background_processor_helper.block_size as usize { break; }
                            routed_audio_left_buffer[index] = *left_frame;
                            index += 1;
                        }

                        write_blocking_or_exit(&*producer_left, &routed_audio_left_buffer);

                        index = 0;
                        let right_channel = outputs_32.get_mut(1);
                        for right_frame in right_channel.iter() {
                            if index >= track_background_processor_helper.block_size as usize { break; }
                            routed_audio_right_buffer[index] = *right_frame;
                            index += 1;
                        }

                        write_blocking_or_exit(&*producer_right, &routed_audio_right_buffer);
                    }

                    // transfer to the ring buffer
                    if mode == TrackBackgroundProcessorMode::AudioOut || mode == TrackBackgroundProcessorMode::Render {
                        let audio_block = audio_block_buffer.get_mut(0).unwrap();

                        audio_block.block = *track_background_processor_helper.event_processor.block_index();

                        // zero the whole block first - only the first block_size frames
                        // get written below, and this scratch block comes out of a shared
                        // recycled pool where the tail frames still carry other tracks'
                        // audio from previous cycles
                        audio_block.audio_data_left.fill(0.0);
                        audio_block.audio_data_right.fill(0.0);

                        let (_, mut outputs_32) = audio_buffer_in_use.split();
                        let left_channel = outputs_32.get_mut(0);
                        for (index, left_frame) in left_channel.iter_mut().enumerate() {
                            if index >= track_background_processor_helper.block_size as usize { break; }
                            *left_frame *= track_background_processor_helper.volume;
                            *left_frame *= left_pan;
                            if *left_frame > left_channel_level {
                                left_channel_level = *left_frame;
                            }
                            audio_block.audio_data_left[index] = *left_frame;
                        }
                        let right_channel = outputs_32.get_mut(1);
                        for (index, right_frame) in right_channel.iter_mut().enumerate() {
                            if index >= track_background_processor_helper.block_size as usize { break; }
                            *right_frame *= track_background_processor_helper.volume;
                            *right_frame *= right_pan;
                            if *right_frame > right_channel_level {
                                right_channel_level = *right_frame;
                            }
                            audio_block.audio_data_right[index] = *right_frame;
                        }

                        if mode == TrackBackgroundProcessorMode::AudioOut {
                            write_blocking_or_exit(&producer_ring_buffer_block, &audio_block_buffer);
                        }
                        else {
                            write_blocking_or_exit(&render_producer_block, &audio_block_buffer);
                        }

                        // might be a good idea to do this every x number of blocks
                        let _ = tx_track_background_thread.send(TrackBackgroundProcessorOutwardEvent::ChannelLevels(track_uuid.clone(), left_channel_level, right_channel_level));

                        // P5: consider going quiescent - only when the transport is
                        // stopped, feeding the normal output (not a render) and several
                        // consecutive blocks have been silent (reverb/echo tails decayed).
                        if !*track_background_processor_helper.event_processor.play()
                            && mode == TrackBackgroundProcessorMode::AudioOut
                            && left_channel_level < SILENCE_THRESHOLD
                            && right_channel_level < SILENCE_THRESHOLD {
                            silent_block_run += 1;
                            if silent_block_run >= SILENT_BLOCKS_BEFORE_QUIESCE
                                && !track_background_processor_helper.plugin_editor_open() {
                                quiescent = true;
                            }
                        }
                        else {
                            silent_block_run = 0;
                        }
                    }
                    else if mode == TrackBackgroundProcessorMode::Coast {
                        thread::sleep(Duration::from_millis(100));
                    }
                } // end loop

                // destroy the plugin instances of this track thread while the
                // lifecycle write lock is held, so their effClose dispatches
                // cannot overlap with any other track thread's process()
                {
                    let _vst24_lifecycle_guard = crate::domain::VST24_PLUGIN_LIFECYCLE_LOCK.write();
                    track_background_processor_helper.release_plugin_instances();
                }

                debug!("#####################Dropped out of Vst loop.")
            }) {
            Ok(_) => (),
            Err(error) => debug!("{:?}", error),
        }
    }
}

#[derive(Default)]
pub struct AudioTrackBackgroundProcessor{
}

impl AudioTrackBackgroundProcessor {

    pub fn new() -> Self {
        Self {
        }
    }
}

impl TrackBackgroundProcessor for AudioTrackBackgroundProcessor {
    fn start_processing(&self,
                        track_uuid: String,
                        tx_audio: crossbeam_channel::Sender<AudioLayerInwardEvent>,
                        rx_track_background_thread: Receiver<TrackBackgroundProcessorInwardEvent>,
                        tx_track_background_thread: Sender<TrackBackgroundProcessorOutwardEvent>,
                        track_thread_coast: Arc<Mutex<TrackBackgroundProcessorMode>>,
                        volume: f32,
                        pan: f32,
                        vst_host_time_info: Arc<RwLock<TimeInfo>>,
                        sample_rate: f64,
                        block_size: f64,
                        tempo: f64,
                        time_signature_numerator: i32,
                        time_signature_denominator: i32,
    ) {
        match ThreadBuilder::default()
            .name(format!("AudioTrackBackgroundProcessor: {}", track_uuid.as_str()))
            .priority(ThreadPriority::Crossplatform(95.try_into().unwrap()))
            .spawn(move |result| {
                match result {
                    Ok(_) => debug!("Thread set to max priority: 95."),
                    Err(error) => debug!("Could not set thread to max priority: {:?}.", error),
                }

                // keep denormal FP arithmetic from silently eating this thread's cpu
                crate::domain::flush_denormals_to_zero();
                // counted live until this closure's locals (including the helper and
                // its plugin handles) are gone - main() waits for zero before letting
                // the CLAP registry destroy instances
                let _liveness_guard = crate::domain::TrackThreadLivenessGuard::new();

                const HOST_BUFFER_CHANNELS: usize = 32;

                let render_ring_buffer_block: SpscRb<AudioBlock> = SpscRb::new(20);
                let render_producer_block = render_ring_buffer_block.producer();
                let render_consumer_block = render_ring_buffer_block.consumer();
                let track_render_audio_consumer_details = AudioConsumerDetails::<AudioBlock>::new(track_uuid.clone(), render_consumer_block);

                // Live monitoring ring: a single slot paces this producer exactly to the
                // jack cycle rate (the previous capacity of 20 let the sample read cursor
                // free-run up to twenty blocks ahead of the instrument tracks' capacity-1
                // rings, so every audio track sat a varying number of blocks behind the
                // rest of the mix - the flam between the percussive tracks). The queue
                // depth at transport restart is now deterministic for every track type.
                let ring_buffer_block: SpscRb<AudioBlock> = SpscRb::new(1);
                let producer_ring_buffer_block = ring_buffer_block.producer();
                let consumer_ring_buffer_block = ring_buffer_block.consumer();
                let audio_consumer_details = AudioConsumerDetails::<AudioBlock>::new(track_uuid.clone(), consumer_ring_buffer_block);

                let mut host_buffer: HostBuffer<f32> = HostBuffer::new(HOST_BUFFER_CHANNELS, HOST_BUFFER_CHANNELS);
                let mut host_buffer_swapped: HostBuffer<f32> = HostBuffer::new(HOST_BUFFER_CHANNELS, HOST_BUFFER_CHANNELS);
                let mut inputs = vec![vec![0.0; BLOCK_SIZE_MAX as usize]; HOST_BUFFER_CHANNELS];
                let mut outputs = vec![vec![0.0; BLOCK_SIZE_MAX as usize]; HOST_BUFFER_CHANNELS];
                let mut audio_buffer = host_buffer.bind(&inputs, &mut outputs);
                let mut audio_buffer_swapped = host_buffer_swapped.bind(&outputs, &mut inputs);

                let mut track_background_processor_helper =
                    TrackBackgroundProcessorHelper::new(
                        track_uuid.clone(),
                        tx_audio.clone(),
                        rx_track_background_thread,
                        tx_track_background_thread.clone(),
                        track_thread_coast.clone(),
                        volume,
                        pan,
                        GeneralTrackType::AudioTrack,
                        vst_host_time_info,
                        Box::new(RiffBufferTrackEventProcessor::new(block_size)),
                        sample_rate,
                        block_size,
                        tempo,
                        time_signature_numerator,
                        time_signature_denominator,
                    );

                let mut use_sample_audio = true;

                let mut routed_audio_left_buffer: [f32; BLOCK_SIZE_MAX as usize] = [0.0; BLOCK_SIZE_MAX as usize];
                let mut routed_audio_right_buffer: [f32; BLOCK_SIZE_MAX as usize] = [0.0; BLOCK_SIZE_MAX as usize];

                let mut priority_reconciled = false;

                let audio_block = AudioBlock::default();
                let mut audio_block_buffer = vec![audio_block];

                let render_audio_block = AudioBlock::default();
                let render_audio_block_buffer = vec![render_audio_block];

                track_background_processor_helper.send_render_audio_consumer_details_to_app(track_render_audio_consumer_details);
                track_background_processor_helper.send_audio_consumer_details_to_jack(audio_consumer_details);
                // track_background_processor_helper.send_midi_consumer_details_to_jack(midi_consumer_details);

                loop {
                    // this thread must never outrank the jack callback thread (see helper)
                    if !priority_reconciled {
                        priority_reconciled = crate::domain::reconcile_track_thread_priority();
                    }

                    // the Kill event handler stops the plugins and clears keep_alive:
                    // leave the loop so no thread can be inside process() when the
                    // application main thread destroys the plugin instances at exit
                    if !track_background_processor_helper.keep_alive {
                        info!("Track background processor thread for track {} got the Kill and is leaving its loop.", track_uuid.as_str());
                        break;
                    }

                    // read the audio configuration from the helper each iteration so an
                    // AudioConfigurationChange event takes effect without restarting the thread
                    let current_block_size = track_background_processor_helper.block_size as i32;
                    // plugin lifecycle work (see VST24_PLUGIN_LIFECYCLE_LOCK) holds the
                    // write lock against every thread's plugin dispatch section.
                    {
                        let _vst24_lifecycle_guard = crate::domain::VST24_PLUGIN_LIFECYCLE_LOCK.write();
                        track_background_processor_helper.handle_incoming_events();
                    }

                    // the Kill may have arrived in THIS pass: its handler just
                    // stopped/suspended/closed the plugins - falling through
                    // would dispatch process() onto those closed instances
                    // (which is what segfaulted inside the JUCE VST2 wrapper),
                    // so leave the loop right now instead of at the next check.
                    if !track_background_processor_helper.keep_alive {
                        info!("Track background processor thread for track {} got the Kill and is leaving its loop.", track_uuid.as_str());
                        break;
                    }
                    {
                        let _vst24_process_guard = crate::domain::VST24_PLUGIN_LIFECYCLE_LOCK.read();
                        track_background_processor_helper.process_audio_events();
                        track_background_processor_helper.refresh_effect_plugin_editors();
                        track_background_processor_helper.handle_request_plugin_preset_data();
                        track_background_processor_helper.handle_request_effect_plugins_parameters();
                    }

                    // zero the live output channels for the block about to be produced.
                    // the sample reader accumulates into these buffers with += and only
                    // an effect plugin would overwrite them - without this every block
                    // added on top of the running sum of all previous blocks, so the track
                    // ramped up over a couple of minutes into full-scale garbage (noise).
                    {
                        let (_, mut channels) = audio_buffer.split();
                        channels.get_mut(0).fill(0.0);
                        channels.get_mut(1).fill(0.0);
                    }
                    {
                        let (_, mut channels) = audio_buffer_swapped.split();
                        channels.get_mut(0).fill(0.0);
                        channels.get_mut(1).fill(0.0);
                    }

                    let (left_pan, right_pan) = DAWUtils::constant_power_stereo_pan(track_background_processor_helper.pan);

                    use_sample_audio = true;

                    // handle audio data routed to this track
                    for audio_route_uuid in track_background_processor_helper.audio_inward_routings.iter().find(|(_, audio_route)| match &audio_route.destination {
                        AudioRoutingNodeType::Track(_) => true,
                        AudioRoutingNodeType::Instrument(_, _, _, _) => false,
                        AudioRoutingNodeType::Effect(_, _, _, _) => false,
                    }).map(|(_, audio_routing)| audio_routing.uuid()).iter() {
                        if let Some((consumer_left, consumer_right)) = track_background_processor_helper.audio_inward_consumers.get_mut(audio_route_uuid) {
                            let (_, mut outputs_32) = audio_buffer.split();

                            use_sample_audio = false;

                            for index in 0..routed_audio_left_buffer.len() {
                                routed_audio_left_buffer[index] = 0.0;
                                routed_audio_right_buffer[index] = 0.0;
                            }

                            if let Ok(read) = consumer_left.read(&mut routed_audio_left_buffer) {
                                let left_channel = outputs_32.get_mut(0);

                                for index in 0..read {
                                    left_channel[index] = routed_audio_left_buffer[index];
                                }
                            }

                            if let Ok(read) = consumer_right.read(&mut routed_audio_right_buffer) {
                                let right_channel = outputs_32.get_mut(1);

                                for index in 0..read {
                                    right_channel[index] = routed_audio_right_buffer[index];
                                }
                            }
                        }
                    }

                    if use_sample_audio {
                        track_background_processor_helper.process_sample(&mut audio_buffer, current_block_size, left_pan, right_pan);
                    }

                    // the arrangement's override effects chain processes the track audio in
                    // place of the track's own effects while the override is switched on
                    let use_override_effects = track_background_processor_helper.override_effects_active && !track_background_processor_helper.override_effect_plugin_instances.is_empty();
                    let active_effect_plugin_instances = if use_override_effects {
                        &mut track_background_processor_helper.override_effect_plugin_instances
                    }
                    else {
                        &mut track_background_processor_helper.effect_plugin_instances
                    };
                    let mut swap = true;
                    {
                        let _vst24_process_guard = crate::domain::VST24_PLUGIN_LIFECYCLE_LOCK.read();
                        for effect in active_effect_plugin_instances.iter_mut() {
                            let audio_buffer_in_use = if swap {
                                &mut audio_buffer_swapped
                            }
                            else {
                                &mut audio_buffer
                            };
                            swap = !swap;

                            match effect {
                                BackgroundProcessorAudioPluginType::Vst24(effect) => {
                                    effect.vst_plugin_instance_mut().process(audio_buffer_in_use, current_block_size);
                                }
                                BackgroundProcessorAudioPluginType::Vst3(_effect) => {

                                }
                                BackgroundProcessorAudioPluginType::Clap(_effect) => {

                                }
                            }
                        }
                    }

                    let mode = match track_thread_coast.lock() {
                        Ok(mode) => match *mode {
                            TrackBackgroundProcessorMode::AudioOut => TrackBackgroundProcessorMode::AudioOut,
                            TrackBackgroundProcessorMode::Coast => TrackBackgroundProcessorMode::Coast,
                            TrackBackgroundProcessorMode::Render => TrackBackgroundProcessorMode::Render,
                        }
                        Err(_) => TrackBackgroundProcessorMode::AudioOut,
                    };

                    // transfer to the ring buffer
                    let (left_pan, right_pan) = DAWUtils::constant_power_stereo_pan(track_background_processor_helper.pan);
                    let mut left_channel_level: f32 = 0.0;
                    let mut right_channel_level: f32 = 0.0;
                    let audio_buffer_in_use = if !swap {
                        &mut audio_buffer_swapped
                    }
                    else {
                        &mut audio_buffer
                    };
                    if mode == TrackBackgroundProcessorMode::AudioOut {
                        let audio_block = audio_block_buffer.get_mut(0).unwrap();
                        audio_block.block = *track_background_processor_helper.event_processor.block_index();
                        audio_block.audio_data_left.fill(0.0);
                        audio_block.audio_data_right.fill(0.0);
                        let (_, mut outputs_32) = audio_buffer_in_use.split();
                        let left_channel = outputs_32.get_mut(0);
                        for (index, left_frame) in left_channel.iter_mut().enumerate() {
                            if index >= current_block_size as usize { break; }
                            *left_frame *= track_background_processor_helper.volume;
                            *left_frame *= left_pan;
                            if *left_frame > left_channel_level {
                                left_channel_level = *left_frame;
                            }
                            audio_block.audio_data_left[index] = *left_frame;
                        }
                        let right_channel = outputs_32.get_mut(1);
                        for (index, right_frame) in right_channel.iter_mut().enumerate() {
                            if index >= current_block_size as usize { break; }
                            *right_frame *= track_background_processor_helper.volume;
                            *right_frame *= right_pan;
                            if *right_frame > right_channel_level {
                                right_channel_level = *right_frame;
                            }
                            audio_block.audio_data_right[index] = *right_frame;
                        }

                        write_blocking_or_exit(&producer_ring_buffer_block, &audio_block_buffer);

                        let _ = tx_track_background_thread.send(TrackBackgroundProcessorOutwardEvent::ChannelLevels(track_uuid.clone(), left_channel_level, right_channel_level));
                    }
                    else if mode == TrackBackgroundProcessorMode::Coast {
                        thread::sleep(Duration::from_millis(100));
                    }
                    else if mode == TrackBackgroundProcessorMode::Render {
                        let (_, _outputs_32) = audio_buffer_in_use.split();
                        write_blocking_or_exit(&render_producer_block, &render_audio_block_buffer);
                    }
                    
                } // end loop

                // destroy the plugin instances of this track thread while the
                // lifecycle write lock is held, so their effClose dispatches
                // cannot overlap with any other track thread's process()
                {
                    let _vst24_lifecycle_guard = crate::domain::VST24_PLUGIN_LIFECYCLE_LOCK.write();
                    track_background_processor_helper.release_plugin_instances();
                }

                debug!("#####################Dropped out of Vst loop.")
            }) {
            Ok(_) => (),
            Err(error) => debug!("{:?}", error),
        }
    }
}

#[derive(Default)]
pub struct MidiTrackBackgroundProcessor{
}

impl MidiTrackBackgroundProcessor {

    pub fn new() -> Self {
        MidiTrackBackgroundProcessor {
        }
    }
}

impl TrackBackgroundProcessor for MidiTrackBackgroundProcessor {
    fn start_processing(&self,
                        track_uuid: String,
                        tx_audio: crossbeam_channel::Sender<AudioLayerInwardEvent>,
                        rx_track_background_thread: Receiver<TrackBackgroundProcessorInwardEvent>,
                        tx_track_background_thread: Sender<TrackBackgroundProcessorOutwardEvent>,
                        track_thread_coast: Arc<Mutex<TrackBackgroundProcessorMode>>,
                        volume: f32,
                        pan: f32,
                        vst_host_time_info: Arc<RwLock<TimeInfo>>,
                        sample_rate: f64,
                        block_size: f64,
                        tempo: f64,
                        time_signature_numerator: i32,
                        time_signature_denominator: i32,
    ) {
        match ThreadBuilder::default()
            .name(format!("MidiTrackBackgroundProcessor: {}", track_uuid.as_str()))
            .priority(ThreadPriority::Crossplatform(95.try_into().unwrap()))
            .spawn(move |result| {
                match result {
                    Ok(_) => debug!("Thread set to max priority: 95."),
                    Err(error) => debug!("Could not set thread to max priority: {:?}.", error),
                }

                // keep denormal FP arithmetic from silently eating this thread's cpu
                crate::domain::flush_denormals_to_zero();
                // counted live until this closure's locals (including the helper and
                // its plugin handles) are gone - main() waits for zero before letting
                // the CLAP registry destroy instances
                let _liveness_guard = crate::domain::TrackThreadLivenessGuard::new();


                let ring_buffer_midi: SpscRb<(u32, u8, u8, u8, bool)> = SpscRb::new(EVENT_BUFFER_SIZE);
                let (mut producer_midi, consumer_midi) = (ring_buffer_midi.producer(), ring_buffer_midi.consumer());
                let midi_consumer_details = MidiConsumerDetails::<(u32, u8, u8, u8, bool)>::new(track_uuid.clone(), consumer_midi);

                let mut track_background_processor_helper =
                    TrackBackgroundProcessorHelper::new(
                        track_uuid.clone(),
                        tx_audio.clone(),
                        rx_track_background_thread,
                        tx_track_background_thread,
                        track_thread_coast.clone(),
                        volume,
                        pan,
                        GeneralTrackType::MidiTrack,
                        vst_host_time_info,
                        Box::new(BlockBufferTrackEventProcessor::new()),
                        sample_rate,
                        block_size,
                        tempo,
                        time_signature_numerator,
                        time_signature_denominator,
                    );

                track_background_processor_helper.send_midi_consumer_details_to_jack(midi_consumer_details);

                let mut priority_reconciled = false;

                loop {
                    // this thread must never outrank the jack callback thread (see helper)
                    if !priority_reconciled {
                        priority_reconciled = crate::domain::reconcile_track_thread_priority();
                    }

                    // the Kill event handler stops the plugins and clears keep_alive:
                    // leave the loop so no thread can be inside process() when the
                    // application main thread destroys the plugin instances at exit
                    if !track_background_processor_helper.keep_alive {
                        info!("Track background processor thread for track {} got the Kill and is leaving its loop.", track_uuid.as_str());
                        break;
                    }

                    track_background_processor_helper.handle_incoming_events();

                    // the Kill may have arrived in THIS pass: its handler just
                    // stopped/suspended/closed the plugins - falling through
                    // would dispatch process() onto those closed instances
                    // (which is what segfaulted inside the JUCE VST2 wrapper),
                    // so leave the loop right now instead of at the next check.
                    if !track_background_processor_helper.keep_alive {
                        info!("Track background processor thread for track {} got the Kill and is leaving its loop.", track_uuid.as_str());
                        break;
                    }
                    // track_background_processor_helper.dump_play_info();
                    track_background_processor_helper.process_jack_midi_out_events(&mut producer_midi);
                } // end loop

                // destroy the plugin instances of this track thread while the
                // lifecycle write lock is held, so their effClose dispatches
                // cannot overlap with any other track thread's process()
                {
                    let _vst24_lifecycle_guard = crate::domain::VST24_PLUGIN_LIFECYCLE_LOCK.write();
                    track_background_processor_helper.release_plugin_instances();
                }

                debug!("#####################Dropped out of Midi track background processor loop.")
            }) {
            Ok(_) => (),
            Err(error) => debug!("{:?}", error),
        }
    }
}

pub trait Track {
    fn name(&self) -> &str;
    fn name_mut(&mut self) -> &str;
    fn set_name(&mut self, name: String);
    fn mute(&self) -> bool;
    fn set_mute(&mut self, mute: bool);
    fn solo(&self) -> bool;
    fn set_solo(&mut self, solo: bool);
    fn colour(&self) -> (f64, f64, f64, f64);
    fn colour_mut(&mut self) -> (f64, f64, f64, f64);
    fn set_colour(&mut self, red: f64, green: f64, blue: f64, alpha: f64);
    fn riffs_mut(&mut self) -> &mut Vec<Riff>;
    fn riff_refs_mut(&mut self) -> &mut Vec<RiffReference>;
    fn riffs(&self) -> &Vec<Riff>;
    fn riff_refs(&self) -> &Vec<RiffReference>;
    fn automation_mut(&mut self) -> &mut Automation;
    fn automation(&self) -> &Automation;
    fn uuid(&self) -> Uuid;
    fn uuid_mut(&mut self) -> &mut Uuid;
    fn uuid_string(&mut self) -> String;
    fn set_uuid(&mut self, uuid: Uuid);
    fn start_background_processing(&self,
                                   tx_audio: crossbeam_channel::Sender<AudioLayerInwardEvent>,
                                   rx_vst_thread: Receiver<TrackBackgroundProcessorInwardEvent>,
                                   tx_vst_thread: Sender<TrackBackgroundProcessorOutwardEvent>,
                                   track_thread_coast: Arc<Mutex<TrackBackgroundProcessorMode>>,
                                   volume: f32,
                                   pan: f32,
                                   vst_host_time_info: Arc<RwLock<TimeInfo>>,
                                   sample_rate: f64,
                                   block_size: f64,
                                   tempo: f64,
                                   time_signature_numerator: i32,
                                   time_signature_denominator: i32);
    fn volume(&self) -> f32;
    fn volume_mut(&mut self) -> f32;
    fn set_volume(&mut self, volume: f32); // 0.0 to 1.0
    fn pan(&self) -> f32;
    fn pan_mut(&mut self) -> f32;
    fn set_pan(&mut self, pan: f32); // -1.0 to 1.0
    fn midi_routings_mut(&mut self) -> &mut Vec<TrackEventRouting>;
    fn midi_routings(&self) -> &Vec<TrackEventRouting>;
    fn audio_routings_mut(&mut self) -> &mut Vec<AudioRouting>;
    fn audio_routings(&self) -> &Vec<AudioRouting>;
}

pub trait AudioEffectTrack {
    /// Get a reference to the instrument track's effects.
    fn effects(&self) -> &[AudioPlugin];

    /// Set the instrument track's effects.
    fn set_effects(&mut self, effects: Vec<AudioPlugin>);

    /// Get a mutable reference to the instrument track's effects.
     fn effects_mut(&mut self) -> &mut Vec<AudioPlugin>;
}

#[derive(Serialize, Deserialize)]
pub struct InstrumentTrack {
    uuid: Uuid,
	name: String,
	mute: bool,
	solo: bool,
	red: f64,
	green: f64,
	blue: f64,
    alpha: f64,
	instrument: AudioPlugin,
	pub effects: Vec<AudioPlugin>,
    riffs: Vec<Riff>,
    riff_refs: Vec<RiffReference>,
    automation: Automation,
    #[serde(skip_serializing, skip_deserializing)]
    track_background_processor: InstrumentTrackBackgroundProcessor,
    volume: f32,
    pan: f32,
    midi_routings: Vec<TrackEventRouting>,
    audio_routings: Vec<AudioRouting>,
}

impl InstrumentTrack {
	pub fn new() -> Self {
		let mut track = Self {
            uuid: Uuid::new_v4(),
			name: String::from("Unknown"),
			mute: false,
			solo: false,
			red: 1.0,
			green: 0.0,
			blue: 0.0,
			alpha: 0.5,
			instrument: AudioPlugin::new(),
            effects: vec![],
			riffs: vec![],
			riff_refs: vec![],
			automation: Automation::new(),
            track_background_processor: InstrumentTrackBackgroundProcessor::new(),
            volume: 1.0,
            pan: 0.0,
            midi_routings: vec!{},
            audio_routings: vec![],
		};

        track
	}

    pub fn instrument_mut(&mut self) -> &mut AudioPlugin {
        &mut self.instrument
    }

    pub fn set_instrument(&mut self, instrument: AudioPlugin) {
        self.instrument = instrument;
    }

    /// Get a reference to the instrument track's instrument.
    pub fn instrument(&self) -> &AudioPlugin {
        &self.instrument
    }

    pub fn track_background_processor(&self) -> &InstrumentTrackBackgroundProcessor {
        &self.track_background_processor
    }

    pub fn track_background_processor_mut(&mut self) -> &mut InstrumentTrackBackgroundProcessor {
        &mut self.track_background_processor
    }
}

impl LuaUserData for InstrumentTrack {
    fn add_fields<'lua, F: mlua::UserDataFields<'lua, Self>>(fields: &mut F) {
        fields.add_field_method_get("name", |_, this| Ok(this.name.clone()));
        fields.add_field_method_set("name", |_, this, val| {
            this.name = val;
            Ok(())
        });
    }
    fn add_methods<'lua, M: mlua::UserDataMethods<'lua, Self>>(_methods: &mut M) {
    }
}


impl Track for InstrumentTrack {
    fn name(&self) -> &str {
        self.name.as_ref()
    }

    fn name_mut(&mut self) -> &str {
        self.name.as_ref()
    }

    fn set_name(&mut self, name: String) {
        self.name = name;
    }

    fn mute(&self) -> bool {
        self.mute
    }

    fn set_mute(&mut self, mute: bool) {
        self.mute = mute;
    }

    fn solo(&self) -> bool {
        self.solo
    }

    fn set_solo(&mut self, solo: bool) {
        self.solo = solo;
    }

    fn colour(&self) -> (f64, f64, f64, f64) {
        (self.red, self.green, self.blue, self.alpha)
    }

    fn colour_mut(&mut self) -> (f64, f64, f64, f64) {
        (self.red, self.green, self.blue, self.alpha)
    }

    fn set_colour(&mut self, red: f64, green: f64, blue: f64, alpha: f64) {
        self.red = red;
        self.green = green;
        self.blue = blue;
        self.alpha = alpha;
    }

    fn riffs_mut(&mut self) -> &mut Vec<Riff> {
        &mut self.riffs
    }

    fn riff_refs_mut(&mut self) -> &mut Vec<RiffReference> {
        &mut self.riff_refs
    }

    fn automation_mut(&mut self) -> &mut Automation {
        &mut self.automation
    }

    fn automation(&self) -> &Automation {
        &self.automation
    }

    fn riffs(&self) -> &Vec<Riff> {
        &self.riffs
    }

    fn riff_refs(&self) -> &Vec<RiffReference> {
        &self.riff_refs
    }

    /// Get a reference to the instrument track's uuid.
    fn uuid(&self) -> Uuid {
        self.uuid
    }

    /// Get a mutable reference to the instrument track's uuid.
    fn uuid_mut(&mut self) -> &mut Uuid {
        &mut self.uuid
    }

    /// Set the instrument track's uuid.
    fn set_uuid(&mut self, uuid: Uuid) {
        self.uuid = uuid;
    }

    fn uuid_string(&mut self) -> String {
        self.uuid.to_string()
    }

    fn start_background_processing(&self,
                                   tx_audio: crossbeam_channel::Sender<AudioLayerInwardEvent>,
                                   rx_vst_thread: Receiver<TrackBackgroundProcessorInwardEvent>,
                                   tx_vst_thread: Sender<TrackBackgroundProcessorOutwardEvent>,
                                   track_thread_coast: Arc<Mutex<TrackBackgroundProcessorMode>>,
                                   volume: f32,
                                   pan: f32,
                                   vst_host_time_info: Arc<RwLock<TimeInfo>>,
                                   sample_rate: f64,
                                   block_size: f64,
                                   tempo: f64,
                                   time_signature_numerator: i32,
                                   time_signature_denominator: i32,
    ) {
        self.track_background_processor().start_processing(
            self.uuid().to_string(), tx_audio, rx_vst_thread, tx_vst_thread, track_thread_coast, volume, pan, vst_host_time_info,
            sample_rate,
            block_size,
            tempo,
            time_signature_numerator,
            time_signature_denominator,
        );
    }

    fn volume(&self) -> f32 {
        self.volume
    }

    fn volume_mut(&mut self) -> f32 {
        self.volume
    }

    fn pan(&self) -> f32 {
        self.pan
    }

    fn pan_mut(&mut self) -> f32 {
        self.pan
    }

    fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
    }

    fn set_pan(&mut self, pan: f32) {
        self.pan = pan;
    }

    fn midi_routings_mut(&mut self) -> &mut Vec<TrackEventRouting> {
        &mut self.midi_routings
    }

    fn midi_routings(&self) -> &Vec<TrackEventRouting> {
        &self.midi_routings
    }

    fn audio_routings_mut(&mut self) -> &mut Vec<AudioRouting> {
        &mut self.audio_routings
    }

    fn audio_routings(&self) -> &Vec<AudioRouting> {
        &self.audio_routings
    }
}

impl AudioEffectTrack for InstrumentTrack {
    /// Get a reference to the instrument track's effects.
    fn effects(&self) -> &[AudioPlugin] {
        self.effects.as_ref()
    }

    /// Set the instrument track's effects.
    fn set_effects(&mut self, effects: Vec<AudioPlugin>) {
        self.effects = effects;
    }

    /// Get a mutable reference to the instrument track's effects.
    fn effects_mut(&mut self) -> &mut Vec<AudioPlugin> {
        &mut self.effects
    }
}

#[derive(Serialize, Deserialize)]
pub enum MidiDeviceType {
    Jack,
    Alsa,
}

#[derive(Serialize, Deserialize)]
pub struct MidiDevice {
	name: String,
    midi_device_type: MidiDeviceType,
    midi_channel: i32,
}

impl MidiDevice {
    pub fn new() -> MidiDevice {
        MidiDevice {
            name: String::from("Unknown"),
            midi_device_type: MidiDeviceType::Jack,
            midi_channel: 0,
        }
    }

    pub fn name(&self) -> &str {
        self.name.as_ref()
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }

    pub fn set_midi_channel(&mut self, midi_channel: i32) {
        self.midi_channel = midi_channel;
    }

    pub fn midi_channel(&self) -> i32 {
        self.midi_channel
    }
}

#[derive(Serialize, Deserialize)]
pub struct MidiTrack {
    uuid: Uuid,
	name: String,
	mute: bool,
	solo: bool,
	red: f64,
	green: f64,
	blue: f64,
    alpha: f64,
	midi_device: MidiDevice,
    riffs: Vec<Riff>,
    riff_refs: Vec<RiffReference>,
    automation: Automation,
    #[serde(skip_serializing, skip_deserializing)]
    track_background_processor: MidiTrackBackgroundProcessor,
    volume: f32,
    pan: f32,
    midi_routings: Vec<TrackEventRouting>,
    audio_routings: Vec<AudioRouting>,
}

impl MidiTrack {
	pub fn new() -> Self {
		let mut track = Self {
            uuid: Uuid::new_v4(),
			name: String::from("Unknown"),
			mute: false,
			solo: false,
			red: 1.0,
			green: 0.0,
			blue: 0.0,
            alpha: 0.0,
			midi_device: MidiDevice::new(),
			riffs: vec![],
			riff_refs: vec![],
			automation: Automation::new(),
            track_background_processor: MidiTrackBackgroundProcessor::new(),
            volume: 1.0,
            pan: 0.0,
            midi_routings: vec![],
            audio_routings: vec![],
		};


        track
	}

    pub fn midi_device(&self) -> &MidiDevice {
        &self.midi_device
    }

    pub fn midi_device_mut(&mut self) -> &mut MidiDevice {
        &mut self.midi_device
    }

    pub fn track_background_processor(&self) -> &MidiTrackBackgroundProcessor {
        &self.track_background_processor
    }

    pub fn track_background_processor_mut(&mut self) -> &mut MidiTrackBackgroundProcessor {
        &mut self.track_background_processor
    }
}

impl Track for MidiTrack {
    fn name(&self) -> &str {
        self.name.as_ref()
    }

    fn name_mut(&mut self) -> &str {
        self.name.as_ref()
    }

    fn set_name(&mut self, name: String) {
        self.name = name;
    }

    fn mute(&self) -> bool {
        self.mute
    }

    fn set_mute(&mut self, mute: bool) {
        self.mute = mute;
    }

    fn solo(&self) -> bool {
        self.solo
    }

    fn set_solo(&mut self, solo: bool) {
        self.solo = solo;
    }

    fn colour(&self) -> (f64, f64, f64, f64) {
        (self.red, self.green, self.blue, self.alpha)
    }

    fn colour_mut(&mut self) -> (f64, f64, f64, f64) {
        (self.red, self.green, self.blue, self.alpha)
    }

    fn set_colour(&mut self, red: f64, green: f64, blue: f64, alpha: f64) {
        self.red = red;
        self.green = green;
        self.blue = blue;
        self.alpha = alpha;
    }

    fn riffs_mut(&mut self) -> &mut Vec<Riff> {
        &mut self.riffs
    }

    fn riff_refs_mut(&mut self) -> &mut Vec<RiffReference> {
        &mut self.riff_refs
    }

    fn automation_mut(&mut self) -> &mut Automation {
        &mut self.automation
    }

    fn automation(&self) -> &Automation {
        &self.automation
    }

    fn riffs(&self) -> &Vec<Riff> {
        &self.riffs
    }

    fn riff_refs(&self) -> &Vec<RiffReference> {
        &self.riff_refs
    }

    /// Get a reference to the track's uuid.
    fn uuid(&self) -> Uuid {
        self.uuid
    }

    /// Get a mutable reference to the track's uuid.
    fn uuid_mut(&mut self) -> &mut Uuid {
        &mut self.uuid
    }

    /// Set the track's uuid.
    fn set_uuid(&mut self, uuid: Uuid) {
        self.uuid = uuid;
    }

    fn uuid_string(&mut self) -> String {
        self.uuid.to_string()
    }

    fn start_background_processing(&self,
                                   tx_audio: crossbeam_channel::Sender<AudioLayerInwardEvent>,
                                   rx_vst_thread: Receiver<TrackBackgroundProcessorInwardEvent>,
                                   tx_vst_thread: Sender<TrackBackgroundProcessorOutwardEvent>,
                                   track_thread_coast: Arc<Mutex<TrackBackgroundProcessorMode>>,
                                   volume: f32,
                                   pan: f32,
                                   vst_host_time_info: Arc<RwLock<TimeInfo>>,
                                   sample_rate: f64,
                                   block_size: f64,
                                   tempo: f64,
                                   time_signature_numerator: i32,
                                   time_signature_denominator: i32,
    ) {
        self.track_background_processor().start_processing(
            self.uuid().to_string(), tx_audio, rx_vst_thread, tx_vst_thread, track_thread_coast, volume, pan, vst_host_time_info,
            sample_rate,
            block_size,
            tempo,
            time_signature_numerator,
            time_signature_denominator,
        );
    }

    fn volume(&self) -> f32 {
        self.volume
    }

    fn volume_mut(&mut self) -> f32 {
        self.volume
    }

    fn pan(&self) -> f32 {
        self.pan
    }

    fn pan_mut(&mut self) -> f32 {
        self.pan
    }

    fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
    }

    fn set_pan(&mut self, pan: f32) {
        self.pan = pan;
    }

    fn midi_routings_mut(&mut self) -> &mut Vec<TrackEventRouting> {
        &mut self.midi_routings
    }

    fn midi_routings(&self) -> &Vec<TrackEventRouting> {
        &self.midi_routings
    }

    fn audio_routings_mut(&mut self) -> &mut Vec<AudioRouting> {
        &mut self.audio_routings
    }

    fn audio_routings(&self) -> &Vec<AudioRouting> {
        &self.audio_routings
    }
}

#[derive(Serialize, Deserialize)]
pub struct AudioTrack {
    uuid: Uuid,
	name: String,
	mute: bool,
	solo: bool,
	red: f64,
	green: f64,
	blue: f64,
	alpha: f64,
    riffs: Vec<Riff>,
    riff_refs: Vec<RiffReference>,
    automation: Automation,
    pub effects: Vec<AudioPlugin>,
    volume: f32,
    pan: f32,
    #[serde(skip_serializing, skip_deserializing)]
    track_background_processor: AudioTrackBackgroundProcessor,
    midi_routings: Vec<TrackEventRouting>,
    audio_routings: Vec<AudioRouting>,
}

impl AudioTrack {
	pub fn new() -> Self {
		let mut track = Self {
            uuid: Uuid::new_v4(),
			name: String::from("Unknown"),
			mute: false,
			solo: false,
			red: 1.0,
			green: 0.0,
			blue: 0.0,
			alpha: 0.5,
			riffs: vec![],
			riff_refs: vec![],
			automation: Automation::new(),
            effects: vec![],
            volume: 1.0,
            pan: 0.0,
            track_background_processor: AudioTrackBackgroundProcessor::new(),
            midi_routings: vec![],
            audio_routings: vec![],
        };


        track
	}

    pub fn track_background_processor(&self) -> &AudioTrackBackgroundProcessor {
        &self.track_background_processor
    }

    pub fn track_background_processor_mut(&mut self) -> &mut AudioTrackBackgroundProcessor {
        &mut self.track_background_processor
    }
}

impl Track for AudioTrack {
    fn name(&self) -> &str {
        self.name.as_ref()
    }

    fn name_mut(&mut self) -> &str {
        self.name.as_ref()
    }

    fn set_name(&mut self, name: String) {
        self.name = name;
    }

    fn mute(&self) -> bool {
        self.mute
    }

    fn set_mute(&mut self, mute: bool) {
        self.mute = mute;
    }

    fn solo(&self) -> bool {
        self.solo
    }

    fn set_solo(&mut self, solo: bool) {
        self.solo = solo;
    }

    fn colour(&self) -> (f64, f64, f64, f64) {
        (self.red, self.green, self.blue, self.alpha)
    }

    fn colour_mut(&mut self) -> (f64, f64, f64, f64) {
        (self.red, self.green, self.blue, self.alpha)
    }

    fn set_colour(&mut self, red: f64, green: f64, blue: f64, alpha: f64) {
        self.red = red;
        self.green = green;
        self.blue = blue;
        self.alpha = alpha;
    }

    fn riffs_mut(&mut self) -> &mut Vec<Riff> {
        &mut self.riffs
    }

    fn riff_refs_mut(&mut self) -> &mut Vec<RiffReference> {
        &mut self.riff_refs
    }

    fn automation_mut(&mut self) -> &mut Automation {
        &mut self.automation
    }

    fn automation(&self) -> &Automation {
        &self.automation
    }

    fn riffs(&self) -> &Vec<Riff> {
        &self.riffs
    }

    fn riff_refs(&self) -> &Vec<RiffReference> {
        &self.riff_refs
    }

    /// Get a reference to the track's uuid.
    fn uuid(&self) -> Uuid {
        self.uuid
    }

    /// Get a mutable reference to the track's uuid.
    fn uuid_mut(&mut self) -> &mut Uuid {
        &mut self.uuid
    }

    /// Set the track's uuid.
    fn set_uuid(&mut self, uuid: Uuid) {
        self.uuid = uuid;
    }

    fn uuid_string(&mut self) -> String {
        self.uuid.to_string()
    }

    fn start_background_processing(
        &self,
        _tx_audio: crossbeam_channel::Sender<AudioLayerInwardEvent>,
        _rx_vst_thread: Receiver<TrackBackgroundProcessorInwardEvent>,
        _tx_vst_thread: Sender<TrackBackgroundProcessorOutwardEvent>,
        _track_thread_coast: Arc<Mutex<TrackBackgroundProcessorMode>>,
        _volume: f32,
        _pan: f32,
        _vst_host_time_info: Arc<RwLock<TimeInfo>>,
        _sample_rate: f64,
        _block_size: f64,
        _tempo: f64,
        _time_signature_numerator: i32,
        _time_signature_denominator: i32,
    ) {
        // TODO implement
    }

    fn volume(&self) -> f32 {
        self.volume
    }

    fn volume_mut(&mut self) -> f32 {
        self.volume
    }

    fn pan(&self) -> f32 {
        self.pan
    }

    fn pan_mut(&mut self) -> f32 {
        self.pan
    }

    fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
    }

    fn set_pan(&mut self, pan: f32) {
        self.pan = pan;
    }

    fn midi_routings_mut(&mut self) -> &mut Vec<TrackEventRouting> {
        &mut self.midi_routings
    }

    fn midi_routings(&self) -> &Vec<TrackEventRouting> {
        &self.midi_routings
    }

    fn audio_routings_mut(&mut self) -> &mut Vec<AudioRouting> {
        &mut self.audio_routings
    }

    fn audio_routings(&self) -> &Vec<AudioRouting> {
        &self.audio_routings
    }
}

impl AudioEffectTrack for AudioTrack {
    /// Get a reference to the instrument track's effects.
    fn effects(&self) -> &[AudioPlugin] {
        self.effects.as_ref()
    }

    /// Set the instrument track's effects.
    fn set_effects(&mut self, effects: Vec<AudioPlugin>) {
        self.effects = effects;
    }

    /// Get a mutable reference to the instrument track's effects.
    fn effects_mut(&mut self) -> &mut Vec<AudioPlugin> {
        &mut self.effects
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Loop {
    uuid: Uuid,
	name: String,
	start_position: f64,
	end_position: f64,
}

impl Loop {
	pub fn new() -> Loop {
		Loop {
            uuid: Uuid::new_v4(),
			name: String::from("unkown"),
			start_position: 0.0,
			end_position: 0.0
		}
	}

	pub fn new_with_uuid(uuid: Uuid) -> Loop {
		Loop {
            uuid,
			name: String::from("unkown"),
			start_position: 0.0,
			end_position: 0.0
		}
	}

	pub fn new_with_uuid_and_name(uuid: Uuid, name: String) -> Loop {
		Loop {
            uuid,
			name,
			start_position: 0.0,
			end_position: 0.0
		}
	}

    /// Get the loop's uuid.
    pub fn uuid(&self) -> Uuid {
        self.uuid
    }

    /// Get a reference to the loop's name.
    pub fn name(&self) -> &str {
        self.name.as_ref()
    }

    /// Get a mutable reference to the loop's name.
    pub fn name_mut(&mut self) -> &mut String {
        &mut self.name
    }

    /// Set the loop's name.
    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }

    /// Get the loop's start position.
    pub fn start_position(&self) -> f64 {
        self.start_position
    }

    /// Get a mutable reference to the loop's start position.
    pub fn start_position_mut(&mut self) -> &mut f64 {
        &mut self.start_position
    }

    /// Set the loop's start position.
    pub fn set_start_position(&mut self, start_position: f64) {
        self.start_position = start_position;
    }

    /// Get the loop's end position.
    pub fn end_position(&self) -> f64 {
        self.end_position
    }

    /// Get a mutable reference to the loop's end position.
    pub fn end_position_mut(&mut self) -> &mut f64 {
        &mut self.end_position
    }

    /// Set the loop's end position.
    pub fn set_end_position(&mut self, end_position: f64) {
        self.end_position = end_position;
    }
}

#[derive(Serialize, Deserialize)]
pub struct Song {
	name: String,
	tempo: f64,
	time_signature_numerator: f64,
	time_signature_denominator: f64,
    tracks: Vec<TrackType>,
	length_in_beats: u64,
	loops: Vec<Loop>,
    riff_sets: Vec<RiffSet>,
    riff_sequences: Vec<RiffSequence>,
    #[serde(default)]
    riff_grids: Vec<RiffGrid>,
    riff_arrangements: Vec<RiffArrangement>,
    #[serde(default)]
    tempo_automation: Automation,
    samples: HashMap<String, Sample>,
}

impl Song {
	pub fn new() -> Song {
		Song {
			name: String::from("unknown"),
			tempo: 140.0,
            time_signature_numerator: 4.0,
            time_signature_denominator: 4.0,
            tracks: vec![TrackType::InstrumentTrack(InstrumentTrack::new())],
			length_in_beats: 100,
			loops: vec![],
            riff_sets: vec![],
            riff_sequences: vec![],
            riff_grids: vec![],
            riff_arrangements: vec![],
            tempo_automation: Automation::new(),
            samples: HashMap::new(),
		}
	}

	pub fn add_loop(&mut self, a_loop: Loop) {
		self.loops.push(a_loop);
	}

	pub fn delete_loop(&mut self, uuid: Uuid) {
		self.loops.retain(|current_loop| current_loop.uuid() != uuid);
	}

    pub fn change_loop_name(&mut self, uuid: Uuid, name: String) {
        match self.loops.iter_mut().find(|current_loop| current_loop.uuid() == uuid) {
            Some(current_loop) => current_loop.set_name(name),
            None => debug!("Could not find loop with uuid: {}", uuid),
        }
    }

    /// Get a mutable reference to the song's tracks.
    pub fn tracks_mut(&mut self) -> &mut Vec<TrackType> {
        &mut self.tracks
    }

    /// Get a mutable reference to the song's name.
    pub fn name_mut(&mut self) -> &mut String {
        &mut self.name
    }

    pub fn delete_track(&mut self, track_uuid: String) {
        self.tracks_mut().retain(|track| track.uuid().to_string() != track_uuid);

        // update the riff sets
        self.riff_sets.iter_mut().for_each(|riff_set| riff_set.remove_track(track_uuid.clone()));

        // update the riff arrangements
        self.riff_arrangements.iter_mut().for_each(|riff_arrangement| {
            riff_arrangement.remove_track_automation(&track_uuid);
            riff_arrangement.remove_track_instrument_override(&track_uuid);
            riff_arrangement.remove_track_effects_override(&track_uuid);
        });
    }

    /// Get a reference to the song's tempo.
    pub fn tempo(&self) -> f64 {
        self.tempo
    }

    /// Set the song's tempo.
    pub fn set_tempo(&mut self, tempo: f64) {
        self.tempo = tempo;
    }

    /// Get a reference to the song's tempo automation. Tempo is a song level thing -
    /// the song's tempo automation is fed to every track when the song is played.
    pub fn tempo_automation(&self) -> &Automation {
        &self.tempo_automation
    }

    /// Get a mutable reference to the song's tempo automation.
    pub fn tempo_automation_mut(&mut self) -> &mut Automation {
        &mut self.tempo_automation
    }

    pub fn track_mut(&mut self, uuid: &Uuid) -> Option<&mut TrackType> {
        self.tracks_mut().iter_mut().find(|track| track.uuid().eq(uuid))
    }

    pub fn track(&self, uuid: String) -> Option<&TrackType> {
        self.tracks().iter().find(|track| track.uuid().to_string() == uuid)
    }

    /// Get a reference to the song's tracks.
    pub fn tracks(&self) -> &[TrackType] {
        self.tracks.as_ref()
    }

    pub fn add_track(&mut self, track: TrackType) {
        let track_uuid = track.uuid().to_string();
        self.tracks.push(track);

        // a new track simply has no riff references in the existing riff sets - its slots
        // are silent for the full length of each set, so no placeholder references are
        // needed to keep the sets' timelines aligned

        // update the riff arrangements
        self.riff_arrangements.iter_mut().for_each(|riff_arrangement| {
            riff_arrangement.add_track_automation(track_uuid.clone());
            riff_arrangement.add_track_instrument_override(track_uuid.clone());
            riff_arrangement.add_track_effects_override(track_uuid.clone());
        });
    }

    /// Get a reference to the song's loops.
    pub fn loops(&self) -> &[Loop] {
        self.loops.as_ref()
    }

    /// Get a mutable reference to the song's loops.
    pub fn loops_mut(&mut self) -> &mut Vec<Loop> {
        &mut self.loops
    }

    /// Get a reference to the song's name.
    pub fn name(&self) -> &str {
        self.name.as_ref()
    }

    /// Set the song's name.
    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }

    pub fn add_riff_set(&mut self, riff_set: RiffSet) {
        self.riff_sets.push(riff_set);
    }

    pub fn add_riff_set_at_position(&mut self, riff_set: RiffSet, position: usize) {
        self.riff_sets.insert(position, riff_set);
    }

    pub fn riff_set(&self, uuid: String) -> Option<&RiffSet> {
        self.riff_sets.iter().find(|riff_set| riff_set.uuid() == uuid)
    }

    pub fn riff_set_mut(&mut self, uuid: String) -> Option<&mut RiffSet> {
        self.riff_sets.iter_mut().find(|riff_set| riff_set.uuid() == uuid)
    }

    pub fn remove_riff_set(&mut self, uuid: String) {
        self.riff_sets.retain(|riff_set| riff_set.uuid() != uuid);
    }

    pub fn riff_set_copy(&mut self, uuid: String, new_copy_riff_set_uuid: Uuid) -> Option<&RiffSet> {
        let position = self.riff_sets.iter().position(|riff_set| riff_set.uuid() == uuid);
        if let Some(riff_set) = self.riff_sets.iter_mut().find(|riff_set| riff_set.uuid() == uuid) {
            let mut new_copy_riff_set = RiffSet::new_with_uuid(new_copy_riff_set_uuid);

            new_copy_riff_set.set_name(format!("Copy of {}", riff_set.name()));

            for (track_uuid, riff_ref) in riff_set.riff_refs().iter() {
                new_copy_riff_set.set_riff_ref_for_track(track_uuid.to_string(), RiffReference::new(riff_ref.linked_to(), riff_ref.position()));
            }

            if let Some(position) = position {
                let position = position + 1;
                self.riff_sets.insert(position, new_copy_riff_set);
                self.riff_sets.get(position)
            }
            else {
                None
            }
        }
        else {
            None
        }
    }

    pub fn riff_sets(&self) -> &Vec<RiffSet> {
        &self.riff_sets
    }

    pub fn riff_sets_mut(&mut self) -> &mut Vec<RiffSet> {
        &mut self.riff_sets
    }

    pub fn riff_set_move_to_position(&mut self, riff_set_uuid: String, to_position_index: usize) {
        // find the riff set
        if let Some(mut index) = self.riff_sets_mut().iter_mut().position(|riff_set| riff_set.uuid() == riff_set_uuid) {
            // move the riff set
            if index < to_position_index {
                while index < to_position_index {
                    self.riff_sets_mut().swap(index, index + 1);
                    index += 1;
                }
            }
            else if index > to_position_index {
                while index > to_position_index {
                    self.riff_sets_mut().swap(index, index - 1);
                    index -= 1;
                }
            }
        }
    }

    pub fn riff_sequence_riff_set_move_to_position(&mut self, riff_sequence_uuid: String, riff_set_uuid: String, to_position_index: usize) {
        // find the sequence
        if let Some(riff_sequence) = self.riff_sequence_mut(riff_sequence_uuid) {
            // find the riff set
            if let Some(mut index) = riff_sequence.riff_sets().iter().position(|riff_set| riff_set_uuid.starts_with(riff_set.uuid().as_str()) ) {
                let riff_sets = riff_sequence.riff_sets_mut();

                // move the riff set
                if index < to_position_index {
                    while index < to_position_index {
                        riff_sets.swap(index, index + 1);
                        index += 1;
                    }
                }
                else if index > to_position_index {
                    while index > to_position_index {
                        riff_sets.swap(index, index - 1);
                        index -= 1;
                    }
                }
            }
        }
    }

    pub fn track_move_to_position(&mut self, track_uuid: String, to_position_index: usize) {
        // find the track
        if let Some(mut index) = self.tracks_mut().iter_mut().position(|track| track.uuid().to_string() == track_uuid) {
            // move the track
            if index < to_position_index {
                while index < to_position_index {
                    self.tracks_mut().swap(index, index + 1);
                    index += 1;
                }
            }
            else if index > to_position_index {
                while index > to_position_index {
                    self.tracks_mut().swap(index, index - 1);
                    index -= 1;
                }
            }
        }
    }

    pub fn add_riff_sequence(&mut self, riff_sequence: RiffSequence) {
        self.riff_sequences.push(riff_sequence);
    }

    pub fn riff_sequence(&self, uuid: String) -> Option<&RiffSequence> {
        self.riff_sequences.iter().find(|riff_sequence| riff_sequence.uuid() == uuid)
    }

    pub fn riff_sequence_mut(&mut self, uuid: String) -> Option<&mut RiffSequence> {
        self.riff_sequences.iter_mut().find(|riff_sequence| riff_sequence.uuid() == uuid)
    }

    pub fn remove_riff_sequence(&mut self, uuid: String) {
        self.riff_sequences.retain(|riff_sequence| riff_sequence.uuid() != uuid);
    }

    pub fn riff_sequences(&self) -> &Vec<RiffSequence> {
        &self.riff_sequences
    }

    pub fn add_riff_grid(&mut self, riff_grid: RiffGrid) {
        self.riff_grids.push(riff_grid);
    }

    pub fn riff_grid(&self, uuid: String) -> Option<&RiffGrid> {
        self.riff_grids.iter().find(|riff_grid| riff_grid.uuid() == uuid)
    }

    pub fn riff_grid_mut(&mut self, uuid: String) -> Option<&mut RiffGrid> {
        self.riff_grids.iter_mut().find(|riff_grid| riff_grid.uuid() == uuid)
    }

    pub fn remove_riff_grid(&mut self, uuid: String) {
        self.riff_grids.retain(|riff_grid| riff_grid.uuid() != uuid);
    }

    pub fn riff_grids(&self) -> &Vec<RiffGrid> {
        &self.riff_grids
    }

    pub fn riff_grids_mut(&mut self) -> &mut Vec<RiffGrid> {
        &mut self.riff_grids
    }

    pub fn riff_sequences_mut(&mut self) -> &mut Vec<RiffSequence> {
        &mut self.riff_sequences
    }

    pub fn add_riff_arrangement(&mut self, riff_arrangement: RiffArrangement) {
        self.riff_arrangements.push(riff_arrangement);
    }

    pub fn riff_arrangement(&self, uuid: String) -> Option<&RiffArrangement> {
        self.riff_arrangements.iter().find(|riff_arrangement| riff_arrangement.uuid() == uuid)
    }

    pub fn riff_arrangement_mut(&mut self, uuid: String) -> Option<&mut RiffArrangement> {
        self.riff_arrangements.iter_mut().find(|riff_arrangement| riff_arrangement.uuid() == uuid)
    }

    pub fn remove_riff_arrangement(&mut self, uuid: String) {
        self.riff_arrangements.retain(|riff_arrangement| riff_arrangement.uuid() != uuid);
    }

    pub fn riff_arrangements(&self) -> &Vec<RiffArrangement> {
        &self.riff_arrangements
    }

    pub fn riff_arrangements_mut(&mut self) -> &mut Vec<RiffArrangement> {
        &mut self.riff_arrangements
    }

    pub fn riff_arrangement_move_riff_item_to_position(&mut self, riff_arrangement_uuid: String, riff_item_uuid: String, to_position_index: usize) {
        // find the sequence
        if let Some(riff_arrangement) = self.riff_arrangement_mut(riff_arrangement_uuid) {
            // find the riff set
            if let Some(mut index) = riff_arrangement.items().iter().position(|riff_item| riff_item_uuid.starts_with(riff_item.uuid().as_str()) ) {
                let riff_items = riff_arrangement.items_mut();

                // move the riff item
                if index < to_position_index {
                    while index < to_position_index {
                        riff_items.swap(index, index + 1);
                        index += 1;
                    }
                }
                else if index > to_position_index {
                    while index > to_position_index {
                        riff_items.swap(index, index - 1);
                        index -= 1;
                    }
                }
            }
        }
    }

    pub fn length_in_beats(&self) -> u64 {
        self.length_in_beats
    }

    pub fn length_in_beats_mut(&mut self) -> &mut u64 {
        &mut self.length_in_beats
    }

    pub fn set_length_in_beats(&mut self, length_in_beats: u64) {
        self.length_in_beats = length_in_beats;
    }

    pub fn recalculate_song_length(&mut self) {
        let mut song_length_in_beats: u64 = 0;
        for track in self.tracks().iter() {
            // get the track length
            for riff_ref in track.riff_refs().iter() {
                let linked_to_riff_uuid = riff_ref.linked_to();
                let found_riff = track.riffs().iter().find(|current_riff| current_riff.uuid().to_string() == linked_to_riff_uuid);
                if let Some(riff) = found_riff {
                    let riff_ref_end_position = (riff_ref.position() + riff.length()) as u64;
                    if riff_ref_end_position > song_length_in_beats {
                        song_length_in_beats = riff_ref_end_position;
                    }
                }
            }
        }
        self.set_length_in_beats(song_length_in_beats);
    }

    pub fn time_signature_numerator(&self) -> f64 {
        self.time_signature_numerator
    }

    pub fn time_signature_denominator(&self) -> f64 {
        self.time_signature_denominator
    }

    pub fn time_signature_numerator_mut(&mut self) -> &mut f64 {
        &mut self.time_signature_numerator
    }

    pub fn time_signature_denominator_mut(&mut self) -> &mut f64 {
        &mut self.time_signature_denominator
    }

    pub fn set_time_signature_numerator(&mut self, time_signature_numerator: f64) {
        self.time_signature_numerator = time_signature_numerator;
    }

    pub fn set_time_signature_denominator(&mut self, time_signature_denominator: f64) {
        self.time_signature_denominator = time_signature_denominator;
    }
    pub fn samples(&self) -> &HashMap<String, Sample> {
        &self.samples
    }
    pub fn samples_mut(&mut self) -> &mut HashMap<String, Sample> {
        &mut self.samples
    }
}

#[derive(Serialize, Deserialize)]
pub struct Project {
	song: Song,
}

impl Project {
	pub fn new() -> Project {
		Project {
			song: Song::new(),
		}
	}

    /// Set the project's song.
    pub fn set_song(&mut self, song: Song) {
        self.song = song;
    }

    /// Get a mutable reference to the project's song.
    pub fn song_mut(&mut self) -> &mut Song {
        &mut self.song
    }

    /// Get a reference to the project's song.
    pub fn song(&self) -> &Song {
        &self.song
    }
}

pub struct AudioConsumerDetails<T> {
    track_id: String,
    consumer: Consumer<T>,
    // repetition watchdog state (the jack layer uses this for the audio block
    // consumers): a content signature of the last delivered block and how many
    // consecutive deliveries have been bit identical
    last_signature: u64,
    repeat_run: i32,
    // health telemetry (updated by the jack layer once per cycle): delivered block
    // count, delivery gaps and the peak level of the most recent block, so a track
    // that starves the mix or starts shipping hot garbage can be named in the log
    delivered_blocks: u64,
    missing_run: u32,
    max_missing_run: u32,
    last_peak: f32,
    hot_run: u32,
    max_hot_run: u32,
    // inter-track audio routing latency compensation: a track whose chain is fed by
    // routed audio presents its output `presentation_inherent_blocks` late (its
    // plugin chain cannot be input-offset, see TrackBackgroundProcessorHelper::
    // update_plugin_lookahead). The master mix delays every consumer to the largest
    // inherent delay present so all tracks - routed or not - land in time together.
    pub presentation_inherent_blocks: i32,
    pub presentation_delay_blocks: usize,
    pub pending_blocks: VecDeque<(T, f32)>,
}

impl<T> AudioConsumerDetails<T> {
    pub fn new(track_id: String, consumer: Consumer<T>) -> Self {
        Self {
            track_id,
            consumer,
            last_signature: 0,
            repeat_run: 0,
            delivered_blocks: 0,
            missing_run: 0,
            max_missing_run: 0,
            last_peak: 0.0,
            hot_run: 0,
            max_hot_run: 0,
            presentation_inherent_blocks: 0,
            presentation_delay_blocks: 0,
            pending_blocks: VecDeque::new(),
        }
    }

    /// Get a reference to the consumer details track id.
    pub fn track_id(&self) -> &String {
        &self.track_id
    }

    /// Get a mutable reference to the consumer details track id.
    pub fn track_id_mut(&mut self) -> &mut String {
        &mut self.track_id
    }

    /// Set the consumer details track id.
    pub fn set_track_id(&mut self, track_id: String) {
        self.track_id = track_id;
    }

    pub fn consumer(&self) -> &Consumer<T> {
        &self.consumer
    }

    pub fn consumer_mut(&mut self) -> &mut Consumer<T> {
        &mut self.consumer
    }

    pub fn set_consumer(&mut self, consumer: Consumer<T>) {
        self.consumer = consumer;
    }

    /// True while the presentation queue is still filling up after a transport
    /// start/jump: blocks cannot be presented yet, which must not be counted as
    /// a missing delivery by the health telemetry.
    pub fn in_presentation_fill(&self) -> bool {
        self.presentation_delay_blocks > 0 && self.pending_blocks.len() <= self.presentation_delay_blocks
    }

    /// The jack layer feeds every delivered non-silent audio block through this. When a
    /// producer keeps handing over bit-identical content (a plugin that has died but is
    /// still being shipped, or a stuck read position) the block would be machine-gunned
    /// into the mix as noise - after a few identical deliveries the consumer is flagged
    /// stale so the mixer drops its blocks until the content changes again. Silence is
    /// never flagged. Returns true when this block should be dropped.
    pub fn observe_repetition(&mut self, signature: u64, non_silent: bool) -> bool {
        if !non_silent {
            self.last_signature = 0;
            self.repeat_run = 0;
            return false;
        }
        if signature == self.last_signature {
            self.repeat_run += 1;
            if self.repeat_run == 4 {
                debug!("Audio watchdog: track {} is repeating the same audio block - dropping its output until it changes (a dead plugin/producer?).", self.track_id);
            }
        }
        else {
            self.last_signature = signature;
            self.repeat_run = 0;
        }
        self.repeat_run >= 4
    }

    /// Reset the repetition watchdog - called on transport moves (play/seek/stop) so
    /// replaying the same passage from the top is never mistaken for a stuck producer.
    pub fn reset_watchdog(&mut self) {
        self.last_signature = 0;
        self.repeat_run = 0;
    }

    /// One health sample per jack cycle: whether this consumer delivered and how hot
    /// the delivered block is. `hot_run` counts consecutive blocks whose peak exceeds
    /// a sane musical ceiling - sustained hot blocks are how a diverging / garbage
    /// producing plugin announces itself.
    pub fn observe_cycle(&mut self, delivered: bool, peak: f32) {
        if delivered {
            self.delivered_blocks += 1;
            self.missing_run = 0;
            self.last_peak = peak;
            if peak > 1.05 {
                self.hot_run += 1;
                if self.hot_run > self.max_hot_run {
                    self.max_hot_run = self.hot_run;
                }
            }
            else {
                self.hot_run = 0;
            }
        }
        else {
            self.missing_run += 1;
            if self.missing_run > self.max_missing_run {
                self.max_missing_run = self.missing_run;
            }
        }
    }

    /// Start a fresh health reporting window.
    pub fn begin_health_window(&mut self) {
        self.max_missing_run = 0;
        self.max_hot_run = 0;
    }

    pub fn delivered_blocks(&self) -> u64 {
        self.delivered_blocks
    }

    pub fn max_missing_run(&self) -> u32 {
        self.max_missing_run
    }

    pub fn max_hot_run(&self) -> u32 {
        self.max_hot_run
    }

    pub fn last_peak(&self) -> f32 {
        self.last_peak
    }
}

pub struct MidiConsumerDetails<T> {
    track_uuid: String,
    consumer: Consumer<T>,
    midi_out_port: Option<Port<MidiOut>>,
}

impl<T> MidiConsumerDetails<T> {
    pub fn new(track_uuid: String, consumer: Consumer<T>) -> Self {
        Self {
            track_uuid,
            consumer,
            midi_out_port: None,
        }
    }

    /// Get a reference to the consumer details track id.
    pub fn track_uuid(&self) -> &String {
        &self.track_uuid
    }

    /// Get a mutable reference to the consumer details track id.
    pub fn track_uuid_mut(&mut self) -> &mut String {
        &mut self.track_uuid
    }

    /// Get a reference to the consumer details consumer.
    pub fn consumer(&self) -> &Consumer<T> {
        &self.consumer
    }

    /// Get a mutable reference to the consumer details consumer.
    pub fn consumer_mut(&mut self) -> &mut Consumer<T> {
        &mut self.consumer
    }

    pub fn midi_out_port_mut(&mut self) -> Option<&mut Port<MidiOut>> {
        self.midi_out_port.as_mut()
    }

    pub fn set_midi_out_port(&mut self, midi_out_port: Option<Port<MidiOut>>) {
        self.midi_out_port = midi_out_port;
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ScannedPlugin {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) id: String,
    pub(crate) sub_id: Option<String>,
    pub(crate) audio_plugin_stack: AudioPluginType
}

#[derive(Clone, Serialize, Deserialize)]
pub struct DAWConfiguration {
    pub audio: AudioConfiguration,
    pub scanned_instrument_plugins: ScannedPlugins,
    pub scanned_effect_plugins: ScannedPlugins,
    pub midi_input_connections: MidiInputConnections,
    pub midi_output_connections: MidiOutputConnections,
    #[serde(default = "Vec::new")]
    pub vst24_plugin_paths: Vec<String>,
    #[serde(default = "Vec::new")]
    pub clap_plugin_paths: Vec<String>,
    #[serde(default = "Vec::new")]
    pub vst3_plugin_paths: Vec<String>,
}

impl DAWConfiguration {
    pub fn new() -> Self {
        Self {
            audio: AudioConfiguration::new(),
            scanned_instrument_plugins: ScannedPlugins::new(),
            scanned_effect_plugins: ScannedPlugins::new(),
            midi_input_connections: MidiInputConnections::new(),
            midi_output_connections: MidiOutputConnections::new(),
            vst24_plugin_paths: vec![],
            clap_plugin_paths: vec![],
            vst3_plugin_paths: vec![],
        }
    }

    pub fn load_config() -> DAWConfiguration {
        if let Some(mut config_path) = dirs::config_dir() {
            config_path.push(CONFIGURATION_FILE_NAME);
            if let Ok(mut file) = std::fs::File::open(config_path) {
                let mut json_text = String::new();

                if let Ok(_) = file.read_to_string(&mut json_text) {
                    let result: std::result::Result<DAWConfiguration, serde_json::Error> = serde_json::from_str(&json_text);
                    if let Ok(config) = result {
                        return config
                    }
                }
            }
        }

        DAWConfiguration::new()
    }

    pub fn save(&self) {
        debug!("Entering save configuration...");
        if let Some(mut config_path) = dirs::config_dir() {
            config_path.push(CONFIGURATION_FILE_NAME);

            match serde_json::to_string_pretty(self) {
                Ok(json_text) => {
                    match std::fs::write(config_path.clone(), json_text) {
                        Err(error) => debug!("save failure writing to file: {}", error),
                        _ => debug!("config saved.")
                    }
                },
                Err(error) => {
                    debug!("can_serialise failure: {}", error);
                }
            }
        }

        debug!("Exited save configuration.");
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AudioConfiguration {
    pub block_size: i32,
    pub sample_rate: i32,
}

impl AudioConfiguration {
    pub fn new() -> Self {
        Self {
            block_size: 1024,
            sample_rate: 44100,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ScannedPlugins {
    pub successfully_scanned: HashMap<String, ScannedPlugin>, // name, ScannedPlugin struct
}

impl ScannedPlugins {
    pub fn new() -> Self {
        Self {
            successfully_scanned: HashMap::new(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct MidiInputConnections {
    pub midi_input_connections: HashMap<String, String>, // from=name, to=name (DAW input port)
}

impl MidiInputConnections {
    pub fn new() -> Self {
        Self {
            midi_input_connections: HashMap::new(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct MidiOutputConnections {
    pub midi_output_connections: HashMap<String, String>, // from=name (DAW audio output port), to=name
}

impl MidiOutputConnections {
    pub fn new() -> Self {
        Self {
            midi_output_connections: HashMap::new(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub enum TrackEventRoutingNodeType {
    Track(String), // track uuid
    Instrument(String, String), // track uuid, instrument uuid
    Effect(String, String), // track uuid, effect uuid
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TrackEventRouting{
    uuid: Uuid,
    pub description: String,
    pub channel: u8,
    pub note_range: (u8, u8), // start note, end no
    pub source: TrackEventRoutingNodeType,
    pub destination: TrackEventRoutingNodeType,
}

impl TrackEventRouting {
    pub fn new(
        description: String,
        source: TrackEventRoutingNodeType,
        destination: TrackEventRoutingNodeType,
        ) -> Self {
        Self {
            uuid: Uuid::new_v4(),
            description,
            channel: 1,
            note_range: (0, 127),
            source,
            destination,
        }
    }

    pub fn new_with_note_range(
        note_range: (u8, u8),
        description: String,
        source: TrackEventRoutingNodeType,
        destination: TrackEventRoutingNodeType,
    ) -> Self {
        Self {
            uuid: Uuid::new_v4(),
            description,
            channel: 1,
            note_range,
            source,
            destination,
        }
    }

    pub fn new_with_all(
        channel: u8,
        note_range: (u8, u8),
        description: String,
        source: TrackEventRoutingNodeType,
        destination: TrackEventRoutingNodeType,
    ) -> Self {
        Self {
            uuid: Uuid::new_v4(),
            description,
            channel,
            note_range,
            source,
            destination,
        }
    }

    pub fn uuid(&self) -> String {
        self.uuid.to_string()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub enum AudioRoutingNodeType {
    Track(String), // track uuid
    Instrument(String, String, i32, i32), // track uuid, instrument uuid, left audio input index, right audio input index 
    Effect(String, String, i32, i32), // track uuid, effect uuid, left audio input index, right audio input index
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AudioRouting{
    uuid: Uuid,
    pub description: String,
    pub source: AudioRoutingNodeType,
    pub destination: AudioRoutingNodeType,
}

impl AudioRouting {
    pub fn new(
        description: String,
        source: AudioRoutingNodeType,
        destination: AudioRoutingNodeType,
        ) -> Self {
        Self {
            uuid: Uuid::new_v4(),
            description,
            source,
            destination,
        }
    }

    pub fn new_with_note_range(
        description: String,
        source: AudioRoutingNodeType,
        destination: AudioRoutingNodeType,
    ) -> Self {
        Self {
            uuid: Uuid::new_v4(),
            description,
            source,
            destination,
        }
    }

    pub fn new_with_all(
        description: String,
        source: AudioRoutingNodeType,
        destination: AudioRoutingNodeType,
    ) -> Self {
        Self {
            uuid: Uuid::new_v4(),
            description,
            source,
            destination,
        }
    }

    pub fn uuid(&self) -> String {
        self.uuid.to_string()
    }
}

pub trait TrackEventProcessor {
    fn process_events(&mut self) -> (Vec<TrackEvent>, Vec<PluginParameter>);
    fn track_event_blocks(&self) -> &Option<Vec<Vec<TrackEvent>>>;
    fn set_track_event_blocks(&mut self, track_event_blocks: Option<Vec<Vec<TrackEvent>>>);
    fn track_event_blocks_transition_to(&self) -> &Option<Vec<Vec<TrackEvent>>>;
    fn set_track_event_blocks_transition_to(&mut self, track_event_blocks_transition_to: Option<Vec<Vec<TrackEvent>>>);
    fn param_event_blocks(&self) -> &Option<Vec<Vec<PluginParameter>>>;
    fn set_param_event_blocks(&mut self, param_event_blocks: Option<Vec<Vec<PluginParameter>>>);
    fn play(&self) -> &bool;
    fn set_play(&mut self, play: bool);
    fn play_loop_on(&self) -> &bool;
    fn set_play_loop_on(&mut self, play_loop_on: bool);
    fn block_index(&self) -> &i32;
    fn set_block_index(&mut self, block_index: i32);
    fn audio_plugin_immediate_events(&self) -> &Vec<TrackEvent>;
    fn audio_plugin_immediate_events_mut(&mut self) -> &mut Vec<TrackEvent>;
    fn set_audio_plugin_immediate_events(&mut self, audio_plugin_immediate_events: Vec<TrackEvent>);
    fn play_left_block_index(&self) -> &i32;
    fn set_play_left_block_index(&mut self, play_left_block_index: i32);
    fn play_right_block_index(&self) -> &i32;
    fn set_play_right_block_index(&mut self, play_right_block_index: i32);
    fn playing_notes(&self) -> &Vec<i32>;
    fn playing_notes_mut(&mut self) -> &mut Vec<i32>;
    fn set_playing_notes(&mut self, playing_notes: Vec<i32>);
    fn mute(&self) -> &bool;
    fn set_mute(&mut self, mute: bool);
    /// Change the block size the processor is scheduled against. Any pre-blocked
    /// events are invalidated - they get regenerated (against the new configuration)
    /// when playback (re)starts.
    fn set_block_size(&mut self, block_size: f64);
    /// How many blocks ahead of the wall clock the PLUGIN input stream is extracted
    /// to compensate the active plugin chain's reported group delay: the host must
    /// send a plugin its inputs `latency` frames early for the plugin's delayed
    /// output to land on the timeline like every other track. The transport cursor
    /// itself always stays on the wall clock.
    fn plugin_lookahead_blocks(&self) -> i32;
    fn set_plugin_lookahead_blocks(&mut self, blocks: i32);
}

pub struct BlockBufferTrackEventProcessor {
    pub track_event_blocks: Option<Vec<Vec<TrackEvent>>>,
    pub track_event_blocks_transition_to: Option<Vec<Vec<TrackEvent>>>,
    pub param_event_blocks: Option<Vec<Vec<PluginParameter>>>,
    pub audio_plugin_immediate_events: Vec<TrackEvent>,
    pub block_index: i32,
    pub play: bool,
    pub play_loop_on: bool,
    pub play_left_block_index: i32,
    pub play_right_block_index: i32,
    pub playing_notes: Vec<i32>,
    pub mute: bool,
    pub plugin_lookahead_blocks: i32,
}

impl BlockBufferTrackEventProcessor {
    pub fn new() -> Self {
        Self {
            track_event_blocks: None,
            track_event_blocks_transition_to: None,
            param_event_blocks: None,
            audio_plugin_immediate_events: vec![],
            block_index: 0,
            play: false,
            play_loop_on: false,
            play_left_block_index: -1,
            play_right_block_index: -1,
            playing_notes: vec![],
            mute: false,
            plugin_lookahead_blocks: 0,
        }
    }
}

impl TrackEventProcessor for BlockBufferTrackEventProcessor {
    fn process_events(&mut self) -> (Vec<TrackEvent>, Vec<PluginParameter>) {
        let mut events = vec![];
        let param_event_blocks_ref = &self.param_event_blocks;
        let mut param_events = vec![];
        let mut transition_happened = false;

        if self.play {
            match &self.track_event_blocks {
                Some(event_blocks) => {
                    if event_blocks.is_empty() {
                        self.block_index = -1;
                    } else if self.block_index > event_blocks.len() as i32 {
                        self.block_index = 0;
                    }

                    if self.block_index > -1 &&
                        self.play_loop_on &&
                        (self.block_index > self.play_right_block_index || self.block_index < self.play_left_block_index) {
                        self.block_index = self.play_left_block_index;
                    }

                    if self.block_index > -1 {
                        // plugin latency compensation: loop/end bookkeeping and the
                        // transport cursor stay on the wall clock, but the block that
                        // is actually extracted for the plugins is read ahead by the
                        // chain's reported group delay (in blocks) so the plugin's own
                        // delay puts its audio back on the timeline position every
                        // other track (and the jack transport) are playing.
                        let lookahead = self.plugin_lookahead_blocks.max(0);
                        let mut extract_block_index = self.block_index + lookahead;
                        if self.play_loop_on && lookahead > 0 {
                            extract_block_index = extract_block_index.min(self.play_right_block_index.max(0));
                        }
                        let extract_block_index = extract_block_index.max(0) as usize;
                        let wall_block_in_range = (self.block_index as usize) < event_blocks.len();
                        let param_block_index = extract_block_index;

                        if wall_block_in_range {
                            if let Some(event_block) = event_blocks.get(extract_block_index) {
                            for event in event_block {
                                // we transition on a measure???
                                // are we transitioning: do we have anything to transition to, have we hit an appropriate boundary event
                                match event {
                                    TrackEvent::NoteOn(note_on) => {
                                        // debug!("**************** Note on detected at: block={}, frame={}, note={}", self.block_index, note_on.position(), note_on.note());
                                        self.playing_notes.push(note_on.note());
                                        events.push(event.clone());
                                    }
                                    TrackEvent::NoteOff(note_off) => {
                                        // debug!("**************** Note off detected at: block={}, frame={}, note={}", self.block_index, note_off.position(), note_off.note());
                                        self.playing_notes.retain(|note| note_off.note() != *note);
                                        events.push(event.clone());
                                    }
                                    TrackEvent::Measure(_measure) => {
                                        // debug!("**************** Measure boundary detected at: block={}, frame={}", self.block_index, measure.position());
                                        if let Some(transistion_to_event_blocks) = &mut self.track_event_blocks_transition_to {
                                            // debug!("**************** Transition detected.");
                                            transition_happened = true;

                                            if let Some(transistion_to_event_block) = transistion_to_event_blocks.get(extract_block_index) {
                                                let mut start_processing = false;
                                                for transition_event in transistion_to_event_block.iter() {
                                                    // fast-forward within block to measure boundary
                                                    if let TrackEvent::Measure(measure) = transition_event {
                                                        start_processing = true;

                                                        // stop playing notes from pre-transition
                                                        for note in &self.playing_notes { // FIXME playing notes may need to hold a struct with note and midi channel data
                                                            events.push(TrackEvent::NoteOff(
                                                                NoteOff::new_with_params(MidiPolyphonicExpressionNoteId::ALL as i32, measure.position(), *note, 0)
                                                            ))
                                                        }
                                                        self.playing_notes.clear();
                                                    }
                                                    if start_processing {
                                                        events.push(transition_event.clone());
                                                        if let TrackEvent::NoteOn(note_on) = transition_event {
                                                            self.playing_notes.push(note_on.note());
                                                        } else if let TrackEvent::NoteOff(note_off) = transition_event {
                                                            self.playing_notes.retain(|note| note_off.note() != *note);
                                                        }
                                                    }
                                                }
                                            }

                                            break;
                                        }
                                    }
                                    _ => {
                                        events.push(event.clone());
                                    }
                                }
                            }
                            }

                            self.block_index += 1;
                        } else if self.play_loop_on {
                            self.block_index = self.play_left_block_index;
                        } else {
                            self.block_index = -1;
                        }
                        if let Some(param_event_blocks) = param_event_blocks_ref {
                            if let Some(param_event_block) = param_event_blocks.get(param_block_index as usize) {
                                for event in param_event_block {
                                    param_events.push(event.clone());
                                }
                            }
                        }
                    }
                },
                None => (),
            }

            // swap in the transition event blocks if required
            if transition_happened {
                self.track_event_blocks = self.track_event_blocks_transition_to.take();
            }
        }

        if !self.audio_plugin_immediate_events.is_empty() {
            for event in self.audio_plugin_immediate_events.iter() {
                events.push(*event);
            }
            self.audio_plugin_immediate_events.clear();
        }

        (events, param_events)
    }

    fn track_event_blocks(&self) -> &Option<Vec<Vec<TrackEvent>>> {
        &self.track_event_blocks
    }

    fn set_track_event_blocks(&mut self, track_event_blocks: Option<Vec<Vec<TrackEvent>>>) {
        self.track_event_blocks = track_event_blocks;
    }

    fn track_event_blocks_transition_to(&self) -> &Option<Vec<Vec<TrackEvent>>> {
        &self.track_event_blocks_transition_to
    }

    fn set_track_event_blocks_transition_to(&mut self, track_event_blocks_transition_to: Option<Vec<Vec<TrackEvent>>>) {
        self.track_event_blocks_transition_to = track_event_blocks_transition_to;
    }

    fn param_event_blocks(&self) -> &Option<Vec<Vec<PluginParameter>>> {
        &self.param_event_blocks
    }

    fn set_param_event_blocks(&mut self, param_event_blocks: Option<Vec<Vec<PluginParameter>>>) {
        self.param_event_blocks = param_event_blocks;
    }

    fn play(&self) -> &bool {
        &self.play
    }

    fn set_play(&mut self, play: bool) {
        self.play = play;
    }

    fn play_loop_on(&self) -> &bool {
        &self.play_loop_on
    }

    fn set_play_loop_on(&mut self, play_loop_on: bool) {
        self.play_loop_on = play_loop_on;
    }

    fn block_index(&self) -> &i32 {
        &self.block_index
    }

    fn set_block_index(&mut self, block_index: i32) {
        self.block_index = block_index;
    }

    fn audio_plugin_immediate_events(&self) -> &Vec<TrackEvent> {
        &self.audio_plugin_immediate_events
    }

    fn audio_plugin_immediate_events_mut(&mut self) -> &mut Vec<TrackEvent> {
        &mut self.audio_plugin_immediate_events
    }

    fn set_audio_plugin_immediate_events(&mut self, audio_plugin_immediate_events: Vec<TrackEvent>) {
        self.audio_plugin_immediate_events = audio_plugin_immediate_events;
    }

    fn play_left_block_index(&self) -> &i32 {
        &self.play_left_block_index
    }

    fn set_play_left_block_index(&mut self, play_left_block_index: i32) {
        self.play_left_block_index = play_left_block_index;
    }

    fn play_right_block_index(&self) -> &i32 {
        &self.play_right_block_index
    }

    fn set_play_right_block_index(&mut self, play_right_block_index: i32) {
        self.play_right_block_index = play_right_block_index;
    }

    fn playing_notes(&self) -> &Vec<i32> {
        &self.playing_notes
    }

    fn playing_notes_mut(&mut self) -> &mut Vec<i32> {
        &mut self.playing_notes
    }

    fn set_playing_notes(&mut self, playing_notes: Vec<i32>) {
        self.playing_notes = playing_notes;
    }

    fn mute(&self) -> &bool {
        &self.mute
    }

    fn set_mute(&mut self, mute: bool) {
        self.mute = mute;
    }

    fn set_block_size(&mut self, _block_size: f64) {
        // the blocks are scheduled per block size - drop them, they are regenerated when
        // playback (re)starts.
        self.track_event_blocks = None;
        self.track_event_blocks_transition_to = None;
        self.param_event_blocks = None;
    }

    fn plugin_lookahead_blocks(&self) -> i32 {
        self.plugin_lookahead_blocks
    }

    fn set_plugin_lookahead_blocks(&mut self, blocks: i32) {
        self.plugin_lookahead_blocks = blocks.max(0);
    }
}

pub struct RiffBufferTrackEventProcessor {
    pub track_event_blocks: Option<Vec<Vec<TrackEvent>>>,
    pub track_event_blocks_transition_to: Option<Vec<Vec<TrackEvent>>>,
    pub param_event_blocks: Option<Vec<Vec<PluginParameter>>>,
    pub audio_plugin_immediate_events: Vec<TrackEvent>,
    pub block_index: i32,
    pub play: bool,
    pub play_loop_on: bool,
    pub play_left_block_index: i32,
    pub play_right_block_index: i32,
    pub playing_notes: Vec<i32>,
    pub block_size: f64,
    pub mute: bool,
    pub plugin_lookahead_blocks: i32,
}

impl RiffBufferTrackEventProcessor {
    pub fn new(block_size: f64) -> Self {
        Self {
            track_event_blocks: None,
            track_event_blocks_transition_to: None,
            param_event_blocks: None,
            audio_plugin_immediate_events: vec![],
            block_index: 0,
            play: false,
            play_loop_on: false,
            play_left_block_index: -1,
            play_right_block_index: -1,
            playing_notes: vec![],
            block_size,
            mute: false,
            plugin_lookahead_blocks: 0,
        }
    }

    fn extract_events(
        &mut self,
        events: &mut Vec<TrackEvent>,
        param_events: &mut Vec<PluginParameter>,
        param_event_blocks_ref: &mut Option<Vec<Vec<PluginParameter>>>,
        transition: bool,
        param_block_index: i32,
        riff_track_events: &Vec<TrackEvent>,
        start_sample: &i32,
        end_sample: &i32,
        wrapped_block_sample_off_set: i32,
    ) {
        if transition {
            // stop playing notes from pre-transition
            debug!("{} - Transitioning...", std::thread::current().name().unwrap_or_else(|| "Unknown Track"));
            for note in &self.playing_notes { // FIXME playing notes may need to hold a struct with note and midi channel data
                debug!("{} - Transitioning - Stopping note from previous riff: block={}, start_sample={}, end_sample={}, frame={}, note={}", std::thread::current().name().unwrap_or_else(|| "Unknown Track"), self.block_index, start_sample, end_sample, 0, note);
                events.push(TrackEvent::NoteOff(
                    NoteOff::new_with_params(MidiPolyphonicExpressionNoteId::ALL as i32, 0.0, *note, 0)
                ))
            }
            self.playing_notes.clear();
            debug!("{} - Transitioned.", std::thread::current().name().unwrap_or_else(|| "Unknown Track"));
        }

        for riff_track_event in riff_track_events.iter() {
            let mut position = riff_track_event.position() as i32;
            // debug!("event position={}", position);

            if position > *end_sample {
                break;
            }

            // maybe this prevents some sort of coalescing of note off and on events so that a plugin voice gets turned off before a new voice is added for the same note - bizarre
            if position == *start_sample {
                position += 1;
            }

            if *start_sample <= position {
                let adjusted_position = (position - *start_sample + wrapped_block_sample_off_set) as f64;
                // we transition on a measure???
                // are we transitioning: do we have anything to transition to, have we hit an appropriate boundary event
                match riff_track_event {
                    TrackEvent::NoteOn(note_on) => {
                        // debug!("{} - **************** Note on detected at: block={}, start_sample={}, end_sample={}, original position={}, frame={}, note={}", std::thread::current().name().unwrap_or_else(|| "Unknown Track"), self.block_index, start_sample, end_sample, position, adjusted_position, note_on.note());
                        self.playing_notes.push(note_on.note());
                        let mut track_event = riff_track_event.clone();
                        track_event.set_position(adjusted_position);
                        events.push(track_event);
                    }
                    TrackEvent::NoteOff(note_off) => {
                        // debug!("{} - **************** Note off detected at: block={}, start_sample={}, end_sample={}, original position={}, frame={}, note={}", std::thread::current().name().unwrap_or_else(|| "Unknown Track"), self.block_index, start_sample, end_sample, position, adjusted_position, note_off.note());
                        self.playing_notes.retain(|note| note_off.note() != *note);
                        let mut track_event = riff_track_event.clone();
                        track_event.set_position(adjusted_position);
                        events.push(track_event);
                    }
                    _ => {
                        events.push(riff_track_event.clone());
                    }
                }
            }
        }

        if let Some(param_event_blocks) = param_event_blocks_ref {
            if let Some(param_event_block) = param_event_blocks.get(param_block_index as usize) {
                for event in param_event_block {
                    let position = event.position() as i32;
                    if *start_sample <= position && position <= *end_sample {
                        let mut cloned_event = event.clone();

                        cloned_event.set_position(cloned_event.position() - (*start_sample as f64));
                        param_events.push(cloned_event);
                    }
                }
            }
        }
    }
}

impl TrackEventProcessor for RiffBufferTrackEventProcessor {
    fn process_events(&mut self) -> (Vec<TrackEvent>, Vec<PluginParameter>) {
        let mut events_to_play = vec![];
        let mut param_events_to_play = vec![];
        let transition = if let Some(_riffs) = &self.track_event_blocks_transition_to {
            self.track_event_blocks = self.track_event_blocks_transition_to.take();
            true
        }
        else {
            false
        };

        if self.play {
            match &self.track_event_blocks {
                Some(riffs) => {
                    if riffs.is_empty() {
                        self.block_index = -1;
                    }
                    else if self.block_index == -1 {
                        // real blocks have just been applied to a processor that was
                        // parked silent (a track that had no reference in the previously
                        // playing riff set, transitioning to one that does) - start the
                        // new blocks from the beginning of the loop instead of staying
                        // parked at -1 forever
                        self.block_index = if self.play_left_block_index > -1 { self.play_left_block_index } else { 0 };
                    }

                    if self.block_index > -1 {
                        // plugin latency compensation: the riff window is read ahead
                        // of the wall clock by the chain's group delay (whole blocks)
                        // so the plugin's own delay puts its audio back on the
                        // timeline; the cursor and transport bookkeeping stay real-time.
                        let read_ahead_block_index = self.block_index + self.plugin_lookahead_blocks.max(0);
                        let param_block_index = read_ahead_block_index;

                        // when playing riff sets there should only be 1 riff
                        if let Some(riff_events) = riffs.get(0) {
                            if riff_events.len() > 0 { // there should always be at least 1 measure event - riffs multiple measures in size will get around this
                                // debug!("Have an event block with events: {}", event_block.len());
                                let riff_size_in_samples = {
                                    let last_event = &riff_events[riff_events.len() - 1];
                                    let riff_size_in_samples = if let TrackEvent::Measure(measure) = last_event {
                                        // debug!("Last event in the event block is a measure: position={}", measure.position() as i32);
                                        measure.position() as i32
                                    }
                                    else {
                                        // debug!("Last event in the event block is not a measure");
                                        0
                                    };
                                    riff_size_in_samples
                                };

                                if riff_size_in_samples > 0 {
                                    // debug!("{} - Riff size in samples: {}", std::thread::current().name().unwrap_or_else(|| "Unknown Track"), riff_size_in_samples);
                                    // debug!("{} - Processing block...: {}", std::thread::current().name().unwrap_or_else(|| "Unknown Track"), self.block_index);
                                    // debug!("riff_size_in_samples: {}", riff_size_in_samples);
                                    let start_sample = (read_ahead_block_index * (self.block_size as i32)) % riff_size_in_samples;
                                    let mut end_sample = start_sample + (self.block_size as i32);
                                    let wrap = end_sample > riff_size_in_samples;
                                    let overflow = end_sample - riff_size_in_samples;
                                    let cloned_events = riff_events.clone();
                                    let mut cloned_param_events = self.param_event_blocks.clone();

                                    if wrap {
                                        end_sample = riff_size_in_samples;
                                    }

                                    // debug!("start_sample={}, end_sample={}", start_sample, end_sample);
                                    self.extract_events(&mut events_to_play, &mut param_events_to_play, &mut cloned_param_events, transition, param_block_index, &cloned_events, &start_sample, &end_sample, 0);

                                    if wrap {
                                        let wrapped_start_sample = 0;
                                        let wrapped_end_sample = overflow;
                                        let wrapped_block_sample_off_set = self.block_size as i32 - overflow;
                                        // debug!("start_sample={}, end_sample={}", start_sample, end_sample);
                                        // debug!("{} - Wrapping...", std::thread::current().name().unwrap_or_else(|| "Unknown Track"));
                                        self.extract_events(&mut events_to_play, &mut param_events_to_play, &mut cloned_param_events, false, param_block_index, &cloned_events, &wrapped_start_sample, &wrapped_end_sample, wrapped_block_sample_off_set);
                                        // debug!("{} - Wrapped.", std::thread::current().name().unwrap_or_else(|| "Unknown Track"));
                                    }

                                    // debug!("{} - Processed block.", std::thread::current().name().unwrap_or_else(|| "Unknown Track"));
                                }
                            }
                        }

                        self.block_index += 1;
                    }
                }
                None => (),
            }
        }

        if !self.audio_plugin_immediate_events.is_empty() {
            for event in self.audio_plugin_immediate_events.iter() {
                events_to_play.push(*event);
            }
            self.audio_plugin_immediate_events.clear();
        }


        // debug!("playing total events: {}", events.len());

        (events_to_play, param_events_to_play)
    }

    fn track_event_blocks(&self) -> &Option<Vec<Vec<TrackEvent>>> {
        &self.track_event_blocks
    }

    fn set_track_event_blocks(&mut self, track_event_blocks: Option<Vec<Vec<TrackEvent>>>) {
        self.track_event_blocks = track_event_blocks;
    }

    fn track_event_blocks_transition_to(&self) -> &Option<Vec<Vec<TrackEvent>>> {
        &self.track_event_blocks_transition_to
    }

    fn set_track_event_blocks_transition_to(&mut self, track_event_blocks_transition_to: Option<Vec<Vec<TrackEvent>>>) {
        self.track_event_blocks_transition_to = track_event_blocks_transition_to;
    }

    fn param_event_blocks(&self) -> &Option<Vec<Vec<PluginParameter>>> {
        &self.param_event_blocks
    }

    fn set_param_event_blocks(&mut self, param_event_blocks: Option<Vec<Vec<PluginParameter>>>) {
        self.param_event_blocks = param_event_blocks;
    }

    fn play(&self) -> &bool {
        &self.play
    }

    fn set_play(&mut self, play: bool) {
        self.play = play;
    }

    fn play_loop_on(&self) -> &bool {
        &self.play_loop_on
    }

    fn set_play_loop_on(&mut self, play_loop_on: bool) {
        self.play_loop_on = play_loop_on;
    }

    fn block_index(&self) -> &i32 {
        &self.block_index
    }

    fn set_block_index(&mut self, block_index: i32) {
        self.block_index = block_index;
    }

    fn audio_plugin_immediate_events(&self) -> &Vec<TrackEvent> {
        &self.audio_plugin_immediate_events
    }

    fn audio_plugin_immediate_events_mut(&mut self) -> &mut Vec<TrackEvent> {
        &mut self.audio_plugin_immediate_events
    }

    fn set_audio_plugin_immediate_events(&mut self, audio_plugin_immediate_events: Vec<TrackEvent>) {
        self.audio_plugin_immediate_events = audio_plugin_immediate_events;
    }

    fn play_left_block_index(&self) -> &i32 {
        &self.play_left_block_index
    }

    fn set_play_left_block_index(&mut self, play_left_block_index: i32) {
        self.play_left_block_index = play_left_block_index;
    }

    fn play_right_block_index(&self) -> &i32 {
        &self.play_right_block_index
    }

    fn set_play_right_block_index(&mut self, play_right_block_index: i32) {
        self.play_right_block_index = play_right_block_index;
    }

    fn playing_notes(&self) -> &Vec<i32> {
        &self.playing_notes
    }

    fn playing_notes_mut(&mut self) -> &mut Vec<i32> {
        &mut self.playing_notes
    }

    fn set_playing_notes(&mut self, playing_notes: Vec<i32>) {
        self.playing_notes = playing_notes;
    }

    fn mute(&self) -> &bool {
        &self.mute
    }

    fn set_mute(&mut self, mute: bool) {
        self.mute = mute;
    }

    fn set_block_size(&mut self, block_size: f64) {
        self.block_size = block_size;
        // the blocks are scheduled per block size - drop them, they are regenerated when
        // playback (re)starts.
        self.track_event_blocks = None;
        self.track_event_blocks_transition_to = None;
        self.param_event_blocks = None;
    }

    fn plugin_lookahead_blocks(&self) -> i32 {
        self.plugin_lookahead_blocks
    }

    fn set_plugin_lookahead_blocks(&mut self, blocks: i32) {
        self.plugin_lookahead_blocks = blocks.max(0);
    }
}

#[cfg(test)]
mod tests {
    use crossbeam_channel::unbounded;
    use flexi_logger::Logger;

    use crate::domain::*;
    use crate::state::MidiPolyphonicExpressionNoteId;

    #[test]
    fn can_serialise() {
        let mut project = Project::new();
        let pattern = Riff::new_with_name_and_length(Uuid::new_v4(), "test".to_owned(), 1.0);
        let part = RiffReference::new(pattern.uuid().to_string(), 0.0);
        let _song = project.song_mut();
        let tracks = project.song_mut().tracks_mut();

        match tracks.get_mut(0) {
            Some(track) => {
                let patterns = track.riffs_mut();
                patterns.push(pattern);

                let parts = track.riff_refs_mut();
                parts.push(part);
                assert_eq!(1, project.song_mut().tracks_mut().len());
                match serde_json::to_string(&project) {
                    Ok(json_text) => debug!("can_serialise success: {}", json_text),
                    Err(error) => debug!("can_serialise failure: {}",error)
                }
            },
            None => (),
        }
    }

    #[test]
    fn can_deserialise() {
        let json_text = include_str!("../test_data/test.fdaw");
        let mut project: Project = serde_json::from_str(json_text).unwrap();
        debug!("can_deserialise_from_file success: {}", project.song_mut().name_mut());
    }

    #[test]
    fn generate_some_uuids() {
        debug!("{}", Uuid::new_v4());
        debug!("{}", Uuid::new_v4());
        debug!("{}", Uuid::new_v4());
        debug!("{}", Uuid::new_v4());
        debug!("{}", Uuid::new_v4());
        debug!("{}", Uuid::new_v4());
        debug!("{}", Uuid::new_v4());
        debug!("{}", Uuid::new_v4());
    }

    #[test]
    fn serialise_configuration() {
        match serde_json::to_string_pretty(&DAWConfiguration::new()) {
            Ok(json_text) => debug!("DAWConfiguration serialise success: {}", json_text),
            Err(error) => debug!("DAWConfiguration serialise failure: {}",error)
        }
    }

    #[test]
    fn test_transition_between_riff_sets() {
        let bpm = 140.0;
        let sample_rate = 44100.0;
        let block_size = 1024.0;
        let song_length_in_beats = 10.0;
        let (tx_to_audio, _rx_to_audio) = unbounded::<AudioLayerInwardEvent>();
        let (tx_to_vst, rx_to_vst) = channel::<TrackBackgroundProcessorInwardEvent>();
        let _tx_to_vst_ref = tx_to_vst;
        let (tx_from_vst, _rx_from_vst) = channel::<TrackBackgroundProcessorOutwardEvent>();
        let track_thread_coast: Arc<Mutex<TrackBackgroundProcessorMode>> = Arc::new(Mutex::new(TrackBackgroundProcessorMode::AudioOut));
        let _track_uuid = Uuid::new_v4();
        let automation = Automation::new();
        let mut riffs: Vec<Riff> = vec![];
        let mut riff_refs: Vec<RiffReference> = vec![];
        let transition_automation = Automation::new();
        let mut transition_riffs: Vec<Riff> = vec![];
        let mut transition_riff_refs: Vec<RiffReference> = vec![];
        let vst_host_time_info = Arc::new(RwLock::new(TimeInfo {
            sample_pos: 0.0,
            sample_rate: 44100.0,
            nanoseconds: 0.0,
            ppq_pos: 0.0,
            tempo: 140.0,
            bar_start_pos: 0.0,
            cycle_start_pos: 0.0,
            cycle_end_pos: 0.0,
            time_sig_numerator: 4,
            time_sig_denominator: 4,
            smpte_offset: 0,
            smpte_frame_rate: vst::api::SmpteFrameRate::Smpte24fps,
            samples_to_next_clock: 0,
            flags: 3,
        }));
        let time_signature_numerator = 4.0;
        let time_signature_denominator = 4.0;

        let mut track_helper = TrackBackgroundProcessorHelper::new(
            Uuid::new_v4().to_string(),
            tx_to_audio,
            rx_to_vst,
            tx_from_vst,
            track_thread_coast,
            1.0,
            1.0,
            GeneralTrackType::InstrumentTrack,
            vst_host_time_info,
            Box::new(RiffBufferTrackEventProcessor::new(1024.0)),
            44100.0,
            1924.0,
            140.0,
            4,
            4
        );

        // create a 1 bar riff with a long note
        let mut riff_one_bar_with_long_note = Riff::new_with_name_and_length(Uuid::new_v4(), "dark under current".to_string(), 4.0);
        let note = Note::new_with_params(MidiPolyphonicExpressionNoteId::ALL as i32, 0.0, 69, 127, 3.4285714285714284);
        riff_one_bar_with_long_note.events_mut().push(TrackEvent::Note(note));
        riffs.push(riff_one_bar_with_long_note.clone());

        // create a 1 bar empty riff to transition to
        let riff_one_bar_empty = Riff::new_with_name_and_length(Uuid::new_v4(), "dark under current".to_string(), 4.0);
        transition_riffs.push(riff_one_bar_empty.clone());

        // create a riff ref for the long note bar
        let mut riff_ref_riff_one_bar_with_long_note = RiffReference::new(Uuid::new_v4().to_string(), 0.0);
        riff_ref_riff_one_bar_with_long_note.set_linked_to(riff_one_bar_with_long_note.uuid().to_string());
        riff_refs.push(riff_ref_riff_one_bar_with_long_note.clone());

        // create a riff ref for the empty
        let mut riff_ref_riff_one_bar_empty = RiffReference::new(Uuid::new_v4().to_string(), 0.0);
        riff_ref_riff_one_bar_empty.set_linked_to(riff_one_bar_empty.uuid().to_string());
        transition_riff_refs.push(riff_ref_riff_one_bar_empty.clone());

        // do the conversion
        let (event_blocks, _param_event_blocks) =
            DAWUtils::convert_to_event_blocks(&automation, &riffs, &riff_refs, bpm, block_size, sample_rate, song_length_in_beats, 0, true, time_signature_numerator, time_signature_denominator, None);

        // do the transition conversion
        let (transition_event_blocks, _transition_param_event_blocks) =
            DAWUtils::convert_to_event_blocks(&transition_automation, &transition_riffs, &transition_riff_refs, bpm, block_size, sample_rate, song_length_in_beats, 0, true, time_signature_numerator, time_signature_denominator, None);

        track_helper.event_processor.set_track_event_blocks(Some(event_blocks.clone()));
        track_helper.event_processor.set_play(true);
        track_helper.event_processor.set_play_loop_on(true);
        track_helper.event_processor.set_block_index(0);
        track_helper.event_processor.set_play_left_block_index(0);
        track_helper.event_processor.set_play_right_block_index(event_blocks.len() as i32 - 1);

        // fas forward past the note events
        for _block_index in 0..64 {
            track_helper.process_plugin_events();
        }

        track_helper.event_processor.set_track_event_blocks_transition_to(Some(transition_event_blocks));

        for block_index in 64..event_blocks.len() {
            if let Some(events) = event_blocks.get(block_index) {
                if !events.is_empty() {
                    // do some checks
                    debug!("Doing some checks...");
                }

                // TODO need some assertions
                assert_eq!(0, track_helper.event_processor.playing_notes().len());

                track_helper.process_plugin_events();
            }
        }

        debug!("");
    }

    #[test]
    fn test_riff_buffer_track_event_processor() {
        // setup logging
        let logger_init_result = Logger::try_with_str("debug");
        let _logger = if let Ok(logger) = logger_init_result {
            let logger = logger
                // .log_to_file(FileSpec::default())
                // .write_mode(WriteMode::Async)
                .start();
            Some(logger)
        }
        else {
            None
        };

        let mut riff_buffer_track_event_processor = RiffBufferTrackEventProcessor::new(1024.0);

        let bpm = 140.0;
        let sample_rate = 44100.0;
        let block_size = 1024.0;
        let midi_channel = 0;
        let time_signature_numerator = 4.0;
        let time_signature_denominator = 4.0;

        let mut riffs: Vec<Riff> = vec![];
        let mut riff_refs: Vec<RiffReference> = vec![];
        let mut transition_riffs: Vec<Riff> = vec![];
        let mut transition_riff_refs: Vec<RiffReference> = vec![];

        let mut transition_happened = false;
        let mut start_sample = 0;
        let mut end_sample = block_size as i32;

        let riff_size_in_samples = 75600 * 4;

        // create a 4 bar riff with two long notes
        let mut riff_four_bar_with_two_long_notes = Riff::new_with_name_and_length(Uuid::new_v4(), "repro-1".to_string(), 4.0 * 4.0);
        let note1 = Note::new_with_params(MidiPolyphonicExpressionNoteId::ALL as i32, 0.0, 60, 127, 7.99);
        riff_four_bar_with_two_long_notes.events_mut().push(TrackEvent::Note(note1));
        let note2 = Note::new_with_params(MidiPolyphonicExpressionNoteId::ALL as i32, 8.0, 67, 127, 7.99);
        riff_four_bar_with_two_long_notes.events_mut().push(TrackEvent::Note(note2));
        riffs.push(riff_four_bar_with_two_long_notes.clone());

        // create a riff ref for 4 bar riff with two long notes
        let mut riff_ref_riff_four_bar_with_two_long_notes = RiffReference::new(Uuid::new_v4().to_string(), 0.0);
        riff_ref_riff_four_bar_with_two_long_notes.set_linked_to(riff_four_bar_with_two_long_notes.uuid().to_string());
        riff_refs.push(riff_ref_riff_four_bar_with_two_long_notes.clone());

        // create a 1 bar empty riff to transition to
        let riff_one_bar_empty = Riff::new_with_name_and_length(Uuid::new_v4(), "silence".to_string(), 4.0);
        transition_riffs.push(riff_one_bar_empty.clone());

        // create a riff ref for the one bar empty transition riff
        let mut riff_ref_riff_one_bar_empty = RiffReference::new(Uuid::new_v4().to_string(), 0.0);
        riff_ref_riff_one_bar_empty.set_linked_to(riff_one_bar_empty.uuid().to_string());
        transition_riff_refs.push(riff_ref_riff_one_bar_empty.clone());

        // convert the events
        let mut riff_converted_track_events: Vec<TrackEvent> = DAWUtils::extract_riff_ref_events(&riffs, &riff_refs, bpm, sample_rate, midi_channel, time_signature_numerator, time_signature_denominator);
        // let mut transistion_converted_track_events: Vec<TrackEvent> = DAWUtils::extract_riff_ref_events(&transition_riffs, &transition_riff_refs, bpm, sample_rate, midi_channel);

        let mut track_events: Vec<TrackEvent> = vec![];
        let mut param_events: Vec<PluginParameter> = vec![];
        let mut param_event_blocks_ref: Option<Vec<Vec<PluginParameter>>> = None;

        // riff_buffer_track_event_processor.set_track_event_blocks(Some(track_event_blocks));
        riff_buffer_track_event_processor.set_block_index(0);
        riff_buffer_track_event_processor.set_play(true);

        for x in 0..1000 {
            let mut has_tail = false;
            let mut tail_size = 0;

            {
                let start_sample = start_sample % riff_size_in_samples;
                let mut end_sample = start_sample + block_size as i32;

                if end_sample > riff_size_in_samples {
                    has_tail = true;
                    tail_size = end_sample - riff_size_in_samples;
                    end_sample = riff_size_in_samples;
                }

                debug!("Block: {}, start_sample: {}, end_sample: {}", x, start_sample, end_sample);
                riff_buffer_track_event_processor.extract_events(&mut track_events, &mut param_events, &mut param_event_blocks_ref, transition_happened, 0, &riff_converted_track_events, &start_sample, &end_sample, 0);
            }

            if has_tail {
                let start_sample = 0;
                let mut end_sample = tail_size;

                debug!("Block: {}, start_sample: {}, end_sample: {}", x, start_sample, end_sample);
                riff_buffer_track_event_processor.extract_events(&mut track_events, &mut param_events, &mut param_event_blocks_ref, transition_happened, 0, &riff_converted_track_events, &start_sample, &end_sample, 0);
            }

            start_sample = start_sample + block_size as i32;
            end_sample = start_sample + block_size as i32;
            riff_buffer_track_event_processor.set_block_index(riff_buffer_track_event_processor.block_index() + 1);
        }
    }

    #[test]
    fn test_riff_buffer_track_event_processor_process_events() {
        // setup logging
        let logger_init_result = Logger::try_with_str("debug");
        let _logger = if let Ok(logger) = logger_init_result {
            let logger = logger
                // .log_to_file(FileSpec::default())
                // .write_mode(WriteMode::Async)
                .start();
            Some(logger)
        }
        else {
            None
        };

        let mut riff_buffer_track_event_processor = RiffBufferTrackEventProcessor::new(1024.0);

        let bpm = 140.0;
        let sample_rate = 44100.0;
        let block_size = 1024.0;
        let midi_channel = 0;
        let time_signature_numerator = 4.0;
        let time_signature_denominator = 4.0;

        let mut riffs: Vec<Riff> = vec![];
        let mut riff_refs: Vec<RiffReference> = vec![];
        let mut transition_riffs: Vec<Riff> = vec![];
        let mut transition_riff_refs: Vec<RiffReference> = vec![];

        let mut transition_happened = false;
        let mut start_sample = 0;
        let mut end_sample = block_size as i32;

        let riff_size_in_samples = 75600 * 4;

        // create a 4 bar riff with two long notes
        let mut riff_four_bar_with_two_long_notes = Riff::new_with_name_and_length(Uuid::new_v4(), "repro-1".to_string(), 4.0 * 4.0);
        let note1 = Note::new_with_params(MidiPolyphonicExpressionNoteId::ALL as i32, 0.0, 60, 127, 7.99);
        riff_four_bar_with_two_long_notes.events_mut().push(TrackEvent::Note(note1));
        let note2 = Note::new_with_params(MidiPolyphonicExpressionNoteId::ALL as i32, 8.0, 67, 127, 7.99);
        riff_four_bar_with_two_long_notes.events_mut().push(TrackEvent::Note(note2));
        riffs.push(riff_four_bar_with_two_long_notes.clone());

        // create a riff ref for 4 bar riff with two long notes
        let mut riff_ref_riff_four_bar_with_two_long_notes = RiffReference::new(Uuid::new_v4().to_string(), 0.0);
        riff_ref_riff_four_bar_with_two_long_notes.set_linked_to(riff_four_bar_with_two_long_notes.uuid().to_string());
        riff_refs.push(riff_ref_riff_four_bar_with_two_long_notes.clone());

        // create a 1 bar empty riff to transition to
        let riff_one_bar_empty = Riff::new_with_name_and_length(Uuid::new_v4(), "silence".to_string(), 4.0);
        transition_riffs.push(riff_one_bar_empty.clone());

        // create a riff ref for the one bar empty transition riff
        let mut riff_ref_riff_one_bar_empty = RiffReference::new(Uuid::new_v4().to_string(), 0.0);
        riff_ref_riff_one_bar_empty.set_linked_to(riff_one_bar_empty.uuid().to_string());
        transition_riff_refs.push(riff_ref_riff_one_bar_empty.clone());

        // convert the events
        let mut riff_converted_track_events: Vec<TrackEvent> = DAWUtils::extract_riff_ref_events(&riffs, &riff_refs, bpm, sample_rate, midi_channel, time_signature_numerator, time_signature_denominator);
        let mut transistion_converted_track_events: Vec<TrackEvent> = DAWUtils::extract_riff_ref_events(&transition_riffs, &transition_riff_refs, bpm, sample_rate, midi_channel, time_signature_numerator, time_signature_denominator);

        // dump out the converted events
        for event in riff_converted_track_events.iter() {
            match event {
                TrackEvent::Note(note) => debug!("Note: position={}, note={}, length={}", note.position(), note.note(), note.length()),
                TrackEvent::NoteOn(note_on) => debug!("Note on: position={}, note={}", note_on.position(), note_on.note()),
                TrackEvent::NoteOff(note_off) => debug!("Note off: position={}, note={}", note_off.position(), note_off.note()),
                TrackEvent::Measure(_) => {}
                _ => {}
            }
        }

        let mut track_events: Vec<TrackEvent> = vec![];
        let mut param_events: Vec<PluginParameter> = vec![];
        let mut param_event_blocks_ref: Option<Vec<Vec<PluginParameter>>> = None;

        riff_buffer_track_event_processor.set_track_event_blocks(Some(vec![riff_converted_track_events]));
        riff_buffer_track_event_processor.set_block_index(0);
        riff_buffer_track_event_processor.set_play(true);

        for x in 0..1000 {
            riff_buffer_track_event_processor.process_events();

            // if x == 140 {
            if x == 220 {
            // if x == 221 {
                riff_buffer_track_event_processor.set_track_event_blocks_transition_to(Some(vec![
                    // DAWUtils::extract_riff_ref_events(&transition_riffs, &transition_riff_refs, bpm, sample_rate, midi_channel)
                    transistion_converted_track_events.clone()
                ]));
            }

            // depending on the block check for events.
        }
    }

    #[test]
    fn test_riff_buffer_track_event_processor2_process_events() {
        // setup logging
        let logger_init_result = Logger::try_with_str("debug");
        let _logger = if let Ok(logger) = logger_init_result {
            let logger = logger
                // .log_to_file(FileSpec::default())
                // .write_mode(WriteMode::Async)
                .start();
            Some(logger)
        }
        else {
            None
        };

        let mut riff_buffer_track_event_processor2 = RiffBufferTrackEventProcessor::new(1024.0);

        let bpm = 140.0;
        let sample_rate = 44100.0;
        let block_size = 1024.0;
        let midi_channel = 0;
        let time_signature_numerator = 4.0;
        let time_signature_denominator = 4.0;

        let mut riffs: Vec<Riff> = vec![];
        let mut riff_refs: Vec<RiffReference> = vec![];
        let mut transition_riffs: Vec<Riff> = vec![];
        let mut transition_riff_refs: Vec<RiffReference> = vec![];

        let mut transition_happened = false;
        let mut start_sample = 0;
        let mut end_sample = block_size as i32;

        let riff_size_in_samples = 75600 * 4;

        // create a 4 bar riff with two long notes
        let mut riff_four_bar_with_two_long_notes = Riff::new_with_name_and_length(Uuid::new_v4(), "repro-1".to_string(), 4.0 * 4.0);
        let note1 = Note::new_with_params(MidiPolyphonicExpressionNoteId::ALL as i32, 0.0, 60, 127, 7.99);
        riff_four_bar_with_two_long_notes.events_mut().push(TrackEvent::Note(note1));
        let note2 = Note::new_with_params(MidiPolyphonicExpressionNoteId::ALL as i32, 8.0, 67, 127, 7.99);
        riff_four_bar_with_two_long_notes.events_mut().push(TrackEvent::Note(note2));
        riffs.push(riff_four_bar_with_two_long_notes.clone());

        // create a riff ref for 4 bar riff with two long notes
        let mut riff_ref_riff_four_bar_with_two_long_notes = RiffReference::new(Uuid::new_v4().to_string(), 0.0);
        riff_ref_riff_four_bar_with_two_long_notes.set_linked_to(riff_four_bar_with_two_long_notes.uuid().to_string());
        riff_refs.push(riff_ref_riff_four_bar_with_two_long_notes.clone());

        // create a 1 bar empty riff to transition to
        let riff_one_bar_empty = Riff::new_with_name_and_length(Uuid::new_v4(), "silence".to_string(), 4.0);
        transition_riffs.push(riff_one_bar_empty.clone());

        // create a riff ref for the one bar empty transition riff
        let mut riff_ref_riff_one_bar_empty = RiffReference::new(Uuid::new_v4().to_string(), 0.0);
        riff_ref_riff_one_bar_empty.set_linked_to(riff_one_bar_empty.uuid().to_string());
        transition_riff_refs.push(riff_ref_riff_one_bar_empty.clone());

        // convert the events
        let mut riff_converted_track_events: Vec<TrackEvent> = DAWUtils::extract_riff_ref_events(&riffs, &riff_refs, bpm, sample_rate, midi_channel, time_signature_numerator, time_signature_denominator);
        let mut transistion_converted_track_events: Vec<TrackEvent> = DAWUtils::extract_riff_ref_events(&transition_riffs, &transition_riff_refs, bpm, sample_rate, midi_channel, time_signature_numerator, time_signature_denominator);

        // dump out the converted events
        for event in riff_converted_track_events.iter() {
            match event {
                TrackEvent::Note(note) => debug!("Note: position={}, note={}, length={}", note.position(), note.note(), note.length()),
                TrackEvent::NoteOn(note_on) => debug!("Note on: position={}, note={}", note_on.position(), note_on.note()),
                TrackEvent::NoteOff(note_off) => debug!("Note off: position={}, note={}", note_off.position(), note_off.note()),
                TrackEvent::Measure(_) => {}
                _ => {}
            }
        }

        let mut track_events: Vec<TrackEvent> = vec![];
        let mut param_events: Vec<PluginParameter> = vec![];
        let mut param_event_blocks_ref: Option<Vec<Vec<PluginParameter>>> = None;

        riff_buffer_track_event_processor2.set_track_event_blocks(Some(vec![riff_converted_track_events]));
        riff_buffer_track_event_processor2.set_block_index(0);
        riff_buffer_track_event_processor2.set_play(true);

        for x in 0..230000 {
            riff_buffer_track_event_processor2.process_events();

            // if x == 140 {
            // if x == 220 {
            if x == (220 * 1000) {
            // if x == 221 {
                riff_buffer_track_event_processor2.set_track_event_blocks_transition_to(Some(vec![
                    // DAWUtils::extract_riff_ref_events(&transition_riffs, &transition_riff_refs, bpm, sample_rate, midi_channel)
                    transistion_converted_track_events.clone()
                ]));
            }

            // depending on the block check for events.
        }
    }

    #[test]
    fn test_track_event_sort() -> Result<(), String> {
        let mut riff_four_bar_with_two_long_notes = Riff::new_with_name_and_length(Uuid::new_v4(), "repro-1".to_string(), 4.0 * 4.0);
        let note1_on = NoteOn::new_with_params(-1, 0.0, 60, 127);
        riff_four_bar_with_two_long_notes.events_mut().push(TrackEvent::NoteOn(note1_on));
        let note1_off = NoteOff::new_with_params(-1, 7.99, 60, 127);
        riff_four_bar_with_two_long_notes.events_mut().push(TrackEvent::NoteOff(note1_off));
        let note2_on = NoteOn::new_with_params(-1, 8.0, 67, 127);
        riff_four_bar_with_two_long_notes.events_mut().push(TrackEvent::NoteOn(note2_on));
        let note2_off = NoteOff::new_with_params(-1, 15.99, 67, 127);
        riff_four_bar_with_two_long_notes.events_mut().push(TrackEvent::NoteOff(note2_off));
        let note3_on = NoteOn::new_with_params(-1, 16.0, 60, 127);
        riff_four_bar_with_two_long_notes.events_mut().push(TrackEvent::NoteOn(note3_on));
        let measure = Measure::new(16.0);
        riff_four_bar_with_two_long_notes.events_mut().push(TrackEvent::Measure(measure));
        let note3_off = NoteOff::new_with_params(-1, 23.99, 60, 127);
        riff_four_bar_with_two_long_notes.events_mut().push(TrackEvent::NoteOff(note3_off));
        let note4_on = NoteOn::new_with_params(-1, 24.0, 67, 127);
        riff_four_bar_with_two_long_notes.events_mut().push(TrackEvent::NoteOn(note4_on));
        let note4_off = NoteOff::new_with_params(-1, 31.99, 67, 127);
        riff_four_bar_with_two_long_notes.events_mut().push(TrackEvent::NoteOff(note4_off));

        riff_four_bar_with_two_long_notes.events_mut().sort_by(&DAWUtils::sort_track_events);

        for track_event in riff_four_bar_with_two_long_notes.events() {
            match track_event {
                TrackEvent::NoteOn(note_on) => debug!("Note on: position={}", note_on.position()),
                TrackEvent::NoteOff(note_off) => debug!("Note off: position={}", note_off.position()),
                TrackEvent::Measure(measure) => debug!("Measure: position={}", measure.position()),
                _ => ()
            }
        }

        if let Some(track_event) = riff_four_bar_with_two_long_notes.events().get(5) {
            match track_event {
                TrackEvent::Measure(_) => Ok(()),
                _ => Err("The sixth element is not a measure.".to_string())
            }
        }
        else {
            Err("There is no fifth element.".to_string())
        }
    }
}