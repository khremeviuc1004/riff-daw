extern crate factor;

use std::{collections::HashMap, sync::{Arc, mpsc::{channel, Receiver, Sender}, Mutex}, time::Duration};
use std::collections::HashSet;
use std::fmt::{Display, Formatter};
use std::ops::DerefMut;
use std::path::PathBuf;
use std::thread;

use apres::MIDI;
use apres::MIDIEvent::{InstrumentName, TrackName};
use factor::factor_include::factor_include;
use itertools::Itertools;
use jack::{AsyncClient, Client, PortFlags};
use parking_lot::RwLock;
use rb::{RB, RbConsumer, SpscRb};
use simple_clap_host_helper_lib::plugin::library::PluginLibrary;
use strum_macros::EnumIter;
use uuid::Uuid;
use vst::api::TimeInfo;
use vst::host::PluginLoader;

use crate::{audio::Audio, AudioLayerOutwardEvent, utils::DAWUtils, domain::*, event::{AudioLayerInwardEvent, CurrentView, DAWEvents, TrackBackgroundProcessorInwardEvent, TrackBackgroundProcessorOutwardEvent, AutomationEditType}, GeneralTrackType, audio::JackNotificationHandler};
use crate::constants::{BLOCK_SIZE_MAX, EVENT_BUFFER_SIZE, MUSICAL_ITEM_LENGTH_OPTIONS};
use crate::event::{AudioLayerTimeCriticalOutwardEvent, EventProcessorType};
use crate::domain::TrackType;

use xilem::{AppState, WindowId};
use xilem::tokio::sync::mpsc::UnboundedSender;


use crate::domain::{AudioBlock, AudioConsumerDetails, Project, NoteExpressionType, PlayMode, PluginParameterDetail, RiffItem, RiffReference, Track, TrackEvent, DAWConfiguration};
use crate::event::{AudioLayerEvent, OperationModeType};
use crate::history::HistoryManager;
use crate::views::{Bookmarks, DialogState, DrawMode, SyncState};

pub enum RiffDAWMainView {
    Track,
    Riff
}

#[derive(Clone)]
pub enum RiffView {
    RiffSet,
    RiffSequence,
    RiffGrid,
    RiffArrangement
}

#[derive(Clone)]
pub enum EventEditView {
    PianoRoll,
    Automation,
    SampleRoll,
    SampleLibrary,
    Mixer,
    Scripting,
}


#[derive(Clone)]
pub enum AutomationViewMode {
    NoteVelocities,
    NoteExpression,
    Controllers,
    PitchBend,
    Instrument,
    Effect,
}

#[derive(Clone, PartialEq, Debug, EnumIter)]
pub enum MidiPolyphonicExpressionNoteId {
    ALL = -1,
    NoteId0,
    NoteId1,
    NoteId2,
    NoteId3,
    NoteId4,
    NoteId5,
    NoteId6,
    NoteId7,
    NoteId8,
    NoteId9,
    NoteId10,
}

impl Display for MidiPolyphonicExpressionNoteId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            MidiPolyphonicExpressionNoteId::ALL => write!(f, "ALL"),
            MidiPolyphonicExpressionNoteId::NoteId0 => write!(f, "0"),
            MidiPolyphonicExpressionNoteId::NoteId1 => write!(f, "1"),
            MidiPolyphonicExpressionNoteId::NoteId2 => write!(f, "2"),
            MidiPolyphonicExpressionNoteId::NoteId3 => write!(f, "3"),
            MidiPolyphonicExpressionNoteId::NoteId4 => write!(f, "4"),
            MidiPolyphonicExpressionNoteId::NoteId5 => write!(f, "5"),
            MidiPolyphonicExpressionNoteId::NoteId6 => write!(f, "6"),
            MidiPolyphonicExpressionNoteId::NoteId7 => write!(f, "7"),
            MidiPolyphonicExpressionNoteId::NoteId8 => write!(f, "8"),
            MidiPolyphonicExpressionNoteId::NoteId9 => write!(f, "9"),
            MidiPolyphonicExpressionNoteId::NoteId10 => write!(f, "10"),
        }
    }
}

#[derive(Clone, PartialEq, Debug, EnumIter)]
pub enum NoteExpressionPortIndex {
    Global = -1,
    PortIndex0,
    PortIndex1,
    PortIndex2,
    PortIndex3,
    PortIndex4,
    PortIndex5,
    PortIndex6,
    PortIndex7,
    PortIndex8,
    PortIndex9,
    PortIndex10,
}

impl Display for NoteExpressionPortIndex {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            NoteExpressionPortIndex::Global => write!(f, "Global"),
            NoteExpressionPortIndex::PortIndex0 => write!(f, "0"),
            NoteExpressionPortIndex::PortIndex1 => write!(f, "1"),
            NoteExpressionPortIndex::PortIndex2 => write!(f, "2"),
            NoteExpressionPortIndex::PortIndex3 => write!(f, "3"),
            NoteExpressionPortIndex::PortIndex4 => write!(f, "4"),
            NoteExpressionPortIndex::PortIndex5 => write!(f, "5"),
            NoteExpressionPortIndex::PortIndex6 => write!(f, "6"),
            NoteExpressionPortIndex::PortIndex7 => write!(f, "7"),
            NoteExpressionPortIndex::PortIndex8 => write!(f, "8"),
            NoteExpressionPortIndex::PortIndex9 => write!(f, "9"),
            NoteExpressionPortIndex::PortIndex10 => write!(f, "10"),
        }
    }
}

#[derive(Clone, PartialEq, Debug, EnumIter)]
pub enum NoteExpressionChannel {
    Global = -1,
    Channel0,
    Channel1,
    Channel2,
    Channel3,
    Channel4,
    Channel5,
    Channel6,
    Channel7,
    Channel8,
    Channel9,
    Channel10,
}

impl Display for NoteExpressionChannel {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            NoteExpressionChannel::Global => write!(f, "Global"),
            NoteExpressionChannel::Channel0 => write!(f, "0"),
            NoteExpressionChannel::Channel1 => write!(f, "1"),
            NoteExpressionChannel::Channel2 => write!(f, "2"),
            NoteExpressionChannel::Channel3 => write!(f, "3"),
            NoteExpressionChannel::Channel4 => write!(f, "4"),
            NoteExpressionChannel::Channel5 => write!(f, "5"),
            NoteExpressionChannel::Channel6 => write!(f, "6"),
            NoteExpressionChannel::Channel7 => write!(f, "7"),
            NoteExpressionChannel::Channel8 => write!(f, "8"),
            NoteExpressionChannel::Channel9 => write!(f, "9"),
            NoteExpressionChannel::Channel10 => write!(f, "10"),
        }
    }
}

#[derive(Clone, PartialEq, Debug, EnumIter)]
pub enum NoteExpressionKey {
    Global = -1,
    Cminus2,
    Csharp_Dbminus2,
    Dminus2,
    Dsharp_Ebminus2,
    Eminus2,
    Fminus2,
    Fsharp_Gbminus2,
    Gminus2,
    Gsharp_Abminus2,
    Aminus2,
    Asharp_Bbminus2,
    Bminus2,
    Cminus1,
    Csharp_Dbminus1,
    Dminus1,
    Dsharp_Ebminus1,
    Eminus1,
    Fminus1,
    Fsharp_Gbminus1,
    Gminus1,
    Gsharp_Abminus1,
    Aminus1,
    Asharp_Bbminus1,
    Bminus1,
    C0,
    Csharp_Db0,
    D0,
    Dsharp_Eb0,
    E0,
    F0,
    Fsharp_Gb0,
    G0,
    Gsharp_Ab0,
    A0,
    Asharp_Bb0,
    B0,
    C1,
    Csharp_Db1,
    D1,
    Dsharp_Eb1,
    E1,
    F1,
    Fsharp_Gb1,
    G1,
    Gsharp_Ab1,
    A1,
    Asharp_Bb1,
    B1,
    C2,
    Csharp_Db2,
    D2,
    Dsharp_Eb2,
    E2,
    F2,
    Fsharp_Gb2,
    G2,
    Gsharp_Ab2,
    A2,
    Asharp_Bb2,
    B2,
    C3,
    Csharp_Db3,
    D3,
    Dsharp_Eb3,
    E3,
    F3,
    Fsharp_Gb3,
    G3,
    Gsharp_Ab3,
    A3,
    Asharp_Bb3,
    B3,
    C4,
    Csharp_Db4,
    D4,
    Dsharp_Eb4,
    E4,
    F4,
    Fsharp_Gb4,
    G4,
    Gsharp_Ab4,
    A4,
    Asharp_Bb4,
    B4,
    C5,
    Csharp_Db5,
    D5,
    Dsharp_Eb5,
    E5,
    F5,
    Fsharp_Gb5,
    G5,
    Gsharp_Ab5,
    A5,
    Asharp_Bb5,
    B5,
    C6,
    Csharp_Db6,
    D6,
    Dsharp_Eb6,
    E6,
    F6,
    Fsharp_Gb6,
    G6,
    Gsharp_Ab6,
    A6,
    Asharp_Bb6,
    B6,
    C7,
    Csharp_Db7,
    D7,
    Dsharp_Eb7,
    E7,
    F7,
    Fsharp_Gb7,
    G7,
    Gsharp_Ab7,
    A7,
    Asharp_Bb7,
    B7,
    C8,
    Csharp_Db8,
    D8,
    Dsharp_Eb8,
    E8,
    F8,
    Fsharp_Gb8,
    G8,
}

impl Display for NoteExpressionKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            NoteExpressionKey::Global => write!(f, "Global"),
            NoteExpressionKey::Cminus2 => write!(f, "C-2"),
            NoteExpressionKey::Csharp_Dbminus2 => write!(f, "C#/Db-2"),
            NoteExpressionKey::Dminus2 => write!(f, "D-2"),
            NoteExpressionKey::Dsharp_Ebminus2 => write!(f, "D#/Eb-2"),
            NoteExpressionKey::Eminus2 => write!(f, "E-2"),
            NoteExpressionKey::Fminus2 => write!(f, "F-2"),
            NoteExpressionKey::Fsharp_Gbminus2 => write!(f, "F#/Gb-2"),
            NoteExpressionKey::Gminus2 => write!(f, "G-2"),
            NoteExpressionKey::Gsharp_Abminus2 => write!(f, "G#/Ab-2"),
            NoteExpressionKey::Aminus2 => write!(f, "A-2"),
            NoteExpressionKey::Asharp_Bbminus2 => write!(f, "A#/Bb-2"),
            NoteExpressionKey::Bminus2 => write!(f, "B-2"),
            NoteExpressionKey::Cminus1 => write!(f, "C-1"),
            NoteExpressionKey::Csharp_Dbminus1 => write!(f, "C#/Db-1"),
            NoteExpressionKey::Dminus1 => write!(f, "D-1"),
            NoteExpressionKey::Dsharp_Ebminus1 => write!(f, "D#/Eb-1"),
            NoteExpressionKey::Eminus1 => write!(f, "E-1"),
            NoteExpressionKey::Fminus1 => write!(f, "F-1"),
            NoteExpressionKey::Fsharp_Gbminus1 => write!(f, "F#/Gb-1"),
            NoteExpressionKey::Gminus1 => write!(f, "G-1"),
            NoteExpressionKey::Gsharp_Abminus1 => write!(f, "G#/Ab-1"),
            NoteExpressionKey::Aminus1 => write!(f, "A-1"),
            NoteExpressionKey::Asharp_Bbminus1 => write!(f, "A#/Bb-1"),
            NoteExpressionKey::Bminus1 => write!(f, "B-1"),
            NoteExpressionKey::C0 => write!(f, "C0"),
            NoteExpressionKey::Csharp_Db0 => write!(f, "C#/Db0"),
            NoteExpressionKey::D0 => write!(f, "D0"),
            NoteExpressionKey::Dsharp_Eb0 => write!(f, "D#/Eb0"),
            NoteExpressionKey::E0 => write!(f, "E0"),
            NoteExpressionKey::F0 => write!(f, "F0"),
            NoteExpressionKey::Fsharp_Gb0 => write!(f, "F#/Gb0"),
            NoteExpressionKey::G0 => write!(f, "G0"),
            NoteExpressionKey::Gsharp_Ab0 => write!(f, "G#/Ab0"),
            NoteExpressionKey::A0 => write!(f, "A0"),
            NoteExpressionKey::Asharp_Bb0 => write!(f, "A#/Bb0"),
            NoteExpressionKey::B0 => write!(f, "B0"),
            NoteExpressionKey::C1 => write!(f, "C1"),
            NoteExpressionKey::Csharp_Db1 => write!(f, "C#/Db1"),
            NoteExpressionKey::D1 => write!(f, "D1"),
            NoteExpressionKey::Dsharp_Eb1 => write!(f, "D#/Eb1"),
            NoteExpressionKey::E1 => write!(f, "E1"),
            NoteExpressionKey::F1 => write!(f, "F1"),
            NoteExpressionKey::Fsharp_Gb1 => write!(f, "F#/Gb1"),
            NoteExpressionKey::G1 => write!(f, "G1"),
            NoteExpressionKey::Gsharp_Ab1 => write!(f, "G#/Ab1"),
            NoteExpressionKey::A1 => write!(f, "A1"),
            NoteExpressionKey::Asharp_Bb1 => write!(f, "A#/Bb1"),
            NoteExpressionKey::B1 => write!(f, "B1"),
            NoteExpressionKey::C2 => write!(f, "C2"),
            NoteExpressionKey::Csharp_Db2 => write!(f, "C#/Db2"),
            NoteExpressionKey::D2 => write!(f, "D2"),
            NoteExpressionKey::Dsharp_Eb2 => write!(f, "D#/Eb2"),
            NoteExpressionKey::E2 => write!(f, "E2"),
            NoteExpressionKey::F2 => write!(f, "F2"),
            NoteExpressionKey::Fsharp_Gb2 => write!(f, "F#/Gb2"),
            NoteExpressionKey::G2 => write!(f, "G2"),
            NoteExpressionKey::Gsharp_Ab2 => write!(f, "G#/Ab2"),
            NoteExpressionKey::A2 => write!(f, "A2"),
            NoteExpressionKey::Asharp_Bb2 => write!(f, "A#/Bb2"),
            NoteExpressionKey::B2 => write!(f, "B2"),
            NoteExpressionKey::C3 => write!(f, "C3"),
            NoteExpressionKey::Csharp_Db3 => write!(f, "C#/Db3"),
            NoteExpressionKey::D3 => write!(f, "D3"),
            NoteExpressionKey::Dsharp_Eb3 => write!(f, "D#/Eb3"),
            NoteExpressionKey::E3 => write!(f, "E3"),
            NoteExpressionKey::F3 => write!(f, "F3"),
            NoteExpressionKey::Fsharp_Gb3 => write!(f, "F#/Gb3"),
            NoteExpressionKey::G3 => write!(f, "G3"),
            NoteExpressionKey::Gsharp_Ab3 => write!(f, "G#/Ab3"),
            NoteExpressionKey::A3 => write!(f, "A3"),
            NoteExpressionKey::Asharp_Bb3 => write!(f, "A#/Bb3"),
            NoteExpressionKey::B3 => write!(f, "B3"),
            NoteExpressionKey::C4 => write!(f, "C4"),
            NoteExpressionKey::Csharp_Db4 => write!(f, "C#/Db4"),
            NoteExpressionKey::D4 => write!(f, "D4"),
            NoteExpressionKey::Dsharp_Eb4 => write!(f, "D#/Eb4"),
            NoteExpressionKey::E4 => write!(f, "E4"),
            NoteExpressionKey::F4 => write!(f, "F4"),
            NoteExpressionKey::Fsharp_Gb4 => write!(f, "F#/Gb4"),
            NoteExpressionKey::G4 => write!(f, "G4"),
            NoteExpressionKey::Gsharp_Ab4 => write!(f, "G#/Ab4"),
            NoteExpressionKey::A4 => write!(f, "A4"),
            NoteExpressionKey::Asharp_Bb4 => write!(f, "A#/Bb4"),
            NoteExpressionKey::B4 => write!(f, "B4"),
            NoteExpressionKey::C5 => write!(f, "C5"),
            NoteExpressionKey::Csharp_Db5 => write!(f, "C#/Db5"),
            NoteExpressionKey::D5 => write!(f, "D5"),
            NoteExpressionKey::Dsharp_Eb5 => write!(f, "D#/Eb5"),
            NoteExpressionKey::E5 => write!(f, "E5"),
            NoteExpressionKey::F5 => write!(f, "F5"),
            NoteExpressionKey::Fsharp_Gb5 => write!(f, "F#/Gb5"),
            NoteExpressionKey::G5 => write!(f, "G5"),
            NoteExpressionKey::Gsharp_Ab5 => write!(f, "G#/Ab5"),
            NoteExpressionKey::A5 => write!(f, "A5"),
            NoteExpressionKey::Asharp_Bb5 => write!(f, "A#/Bb5"),
            NoteExpressionKey::B5 => write!(f, "B5"),
            NoteExpressionKey::C6 => write!(f, "C6"),
            NoteExpressionKey::Csharp_Db6 => write!(f, "C#/Db6"),
            NoteExpressionKey::D6 => write!(f, "D6"),
            NoteExpressionKey::Dsharp_Eb6 => write!(f, "D#/Eb6"),
            NoteExpressionKey::E6 => write!(f, "E6"),
            NoteExpressionKey::F6 => write!(f, "F6"),
            NoteExpressionKey::Fsharp_Gb6 => write!(f, "F#/Gb6"),
            NoteExpressionKey::G6 => write!(f, "G6"),
            NoteExpressionKey::Gsharp_Ab6 => write!(f, "G#/Ab6"),
            NoteExpressionKey::A6 => write!(f, "A6"),
            NoteExpressionKey::Asharp_Bb6 => write!(f, "A#/Bb6"),
            NoteExpressionKey::B6 => write!(f, "B6"),
            NoteExpressionKey::C7 => write!(f, "C7"),
            NoteExpressionKey::Csharp_Db7 => write!(f, "C#/Db7"),
            NoteExpressionKey::D7 => write!(f, "D7"),
            NoteExpressionKey::Dsharp_Eb7 => write!(f, "D#/Eb7"),
            NoteExpressionKey::E7 => write!(f, "E7"),
            NoteExpressionKey::F7 => write!(f, "F7"),
            NoteExpressionKey::Fsharp_Gb7 => write!(f, "F#/Gb7"),
            NoteExpressionKey::G7 => write!(f, "G7"),
            NoteExpressionKey::Gsharp_Ab7 => write!(f, "G#/Ab7"),
            NoteExpressionKey::A7 => write!(f, "A7"),
            NoteExpressionKey::Asharp_Bb7 => write!(f, "A#/Bb7"),
            NoteExpressionKey::B7 => write!(f, "B7"),
            NoteExpressionKey::C8 => write!(f, "C8"),
            NoteExpressionKey::Csharp_Db8 => write!(f, "C#/Db8"),
            NoteExpressionKey::D8 => write!(f, "D8"),
            NoteExpressionKey::Dsharp_Eb8 => write!(f, "D#/Eb8"),
            NoteExpressionKey::E8 => write!(f, "E8"),
            NoteExpressionKey::F8 => write!(f, "F8"),
            NoteExpressionKey::Fsharp_Gb8 => write!(f, "F#/Gb8"),
            NoteExpressionKey::G8 => write!(f, "G8"),
        }
    }
}

#[derive(Clone)]
pub struct TrackGridState {
    pub track_operation_mode_select: bool,
    pub track_grid_selected_snap: usize,
    pub track_grid_riff_references_copy_buffer: Vec<RiffReference>,
    pub track_grid_operation_mode: OperationModeType,
    pub track_grid_edit_cursor_time_in_beats: f64,
    pub track_grid_edit_cursor_position: f64,
    pub track_grid_cursor_follow: bool,
    pub selected_track_grid_riff_references: Vec<String>,
    pub show_automation: bool,
    pub show_notes: bool,
    pub show_note_velocities: bool,
    pub show_pan: bool,
}

#[derive(Clone)]
pub struct PianoRollState {
    pub piano_roll_edit_cursor_position: f64,
    pub piano_roll_edit_cursor_time_in_beats: f64,
    pub piano_roll_grid_operation_mode: OperationModeType,
    pub piano_roll_mpe_note_id: MidiPolyphonicExpressionNoteId,
    pub piano_roll_mpe_voice_picklist_options: Vec<String>,
    pub piano_roll_quantise_end: bool,
    pub piano_roll_quantise_quantise_strength: u32,
    pub piano_roll_quantise_start: bool,
    pub piano_roll_scroll_y: f32,
    pub piano_roll_selected_snap: usize,
    pub piano_roll_subdivision_options: Vec<String>,
    pub piano_roll_triplet_options: Vec<String>,
    pub selected_piano_roll_note_length_option: usize,
    pub selected_piano_roll_note_adj: usize,
    pub selected_piano_roll_triplet: usize,
    pub selected_piano_roll_subdivision: usize,
    pub window_undock: bool,
}

#[derive(Clone)]
pub struct AutomationViewState {
    pub automation_discrete: bool,
    pub automation_edit_cursor_time_in_beats: f64,
    pub automation_edit_type: AutomationEditType,
    pub automation_event_copy_buffer: Vec<TrackEvent>,
    pub automation_grid_operation_mode: OperationModeType,
    pub controller_type_index: Option<i32>,
    pub instrument_parameter_type: Option<i32>,
    pub effect_parameter_type: Option<i32>,
    pub automation_view_mode: AutomationViewMode,
    pub draw_mode: DrawMode,
    pub note_expression_channel: NoteExpressionChannel,
    pub note_expression_id: MidiPolyphonicExpressionNoteId,
    pub note_expression_key: NoteExpressionKey,
    pub note_expression_port_index: NoteExpressionPortIndex,
    pub note_expression_type: NoteExpressionType,
    pub window_undock: bool,
}

pub struct TrackDetailViewState {
    pub add_riff_length: f64,
    pub add_riff_length_text: String,
    pub add_riff_name: String
}

pub struct RiffSetViewState {
    pub add_riff_set_name: String,
}

pub struct RiffSequenceViewState {
    pub add_riff_sequence_name: String,
    pub add_to_seq_riff_set_index: usize,
    pub riff_seq_to_select_index: usize,
    pub selected_riff_sequence_uuid: Option<String>,
}

impl RiffSequenceViewState {
    pub fn selected_riff_sequence_uuid(&self) -> &Option<String> {
        &self.selected_riff_sequence_uuid
    }

    pub fn set_selected_riff_sequence_uuid(&mut self, selected_riff_sequence_uuid: Option<String>) {
        self.selected_riff_sequence_uuid = selected_riff_sequence_uuid;
    }
}

pub struct RiffGridViewState {
    pub add_riff_grid_name: String,
    pub riff_grid_to_select_index: usize,
    pub selected_riff_grid_uuid: Option<String>,
}

impl RiffGridViewState {
    pub fn selected_riff_grid_uuid(&self) -> &Option<String> {
        &self.selected_riff_grid_uuid
    }

    pub fn set_selected_riff_grid_uuid(&mut self, selected_riff_grid_uuid: Option<String>) {
        self.selected_riff_grid_uuid = selected_riff_grid_uuid;
    }
}

pub struct RiffArrangementViewState {
    pub add_riff_arrangement_name: String,
    pub add_to_arr_riff_set_index: usize,
    pub add_to_arr_riff_seq_index: usize,
    pub add_to_arr_riff_grid_index: usize,
    pub riff_arr_to_select_index: usize,
    pub riff_arrangement_riff_item_selected_uuid: Option<(String, String)>,
    pub selected_riff_arrangement_uuid: Option<String>,
}

impl RiffArrangementViewState {

    pub fn riff_arrangement_riff_item_selected_uuid(&self) -> &Option<(String, String)> {
        &self.riff_arrangement_riff_item_selected_uuid
    }

    pub fn set_riff_arrangement_riff_item_selected_uuid(&mut self, riff_arrangement_riff_item_selected_uuid: Option<(String, String)>) {
        self.riff_arrangement_riff_item_selected_uuid = riff_arrangement_riff_item_selected_uuid;
    }
}

pub struct FileDialog {
    pub dialog_window_id: Option<WindowId>,
    pub dialog: DialogState,
    pub bookmarks: Bookmarks,
}

pub struct RiffDAWState {
    pub configuration: DAWConfiguration,
    pub history_manager: Arc<Mutex<HistoryManager>>,
    pub active_loop: Option<Uuid>,
    pub audio_plugin_parameters: HashMap<String, HashMap<String, Vec<PluginParameterDetail>>>,
    pub centre_split_pane_position: i32,
    pub current_file_path: Option<String>,
    pub current_view: CurrentView,
    pub dirty: bool,
    pub event_edit_view: EventEditView,
    pub height: f32,
    pub open_file_dialog_window_id: WindowId,
    pub open_file_dialogue: HashMap<WindowId, bool>,
    pub save_file_dialog_window_id: WindowId,
    pub save_file_dialogue: HashMap<WindowId, bool>,
    pub track_details_window_id: WindowId,
    pub track_details_window: HashMap<WindowId, bool>,
    pub riff_name: Option<String>,
    pub riff_name_window_id: WindowId,
    pub riff_name_window: HashMap<WindowId, bool>,
    pub settings_window_id: WindowId,
    pub settings_window: HashMap<WindowId, bool>,
    pub looping: bool,
    pub main_view: RiffDAWMainView,
    pub main_window_id: WindowId,
    pub note_expression_channel: i32,
    pub note_expression_id: i32,
    pub note_expression_key: i32,
    pub note_expression_port_index: i32,
    pub note_expression_type: NoteExpressionType,
    pub play_mode: PlayMode,
    pub play_position_in_frames: u32,
    pub playing: bool,
    pub playing_riff_set: Option<String>,
    pub playing_riff_sequence: Option<String>,
    pub playing_riff_grid: Option<String>,
    pub playing_riff_arrangement: Option<String>,
    pub playing_riff_sequence_summary_data: Option<(f64, Vec<(f64, String, String)>)>,
    pub playing_riff_grid_summary_data: Option<(f64, Vec<(f64, String, String)>)>,
    pub playing_riff_arrangement_summary_data: Option<(f64, Vec<(f64, RiffItem, Vec<(f64, RiffItem)>)>)>,
    pub parameter_index: Option<i32>,
    pub play_position: f64,
    pub project: Arc<Mutex<Project>>,
    pub riff_view: RiffView,
    pub recording: bool,
    pub riff_grid_cursor_follow: bool,
    pub riff_grid_riff_references_copy_buffer: Vec<RiffReference>,
    pub riff_sequence_riff_set_reference_selected_uuid: Option<(String, String)>,
    pub riff_set_selected_uuid: Option<String>,
    pub running: bool,
    pub selected_automation: Vec<String>,
    pub selected_effect_plugin_uuid: Option<String>,
    pub selected_track: Option<String>,
    pub sample_data: HashMap<String, SampleData>,
    pub selected_riff_grid_riff_references: Vec<String>,
    pub selected_riff_ref_uuid: Option<String>,
    pub selected_riff_uuid_map: HashMap<String, String>,
    pub selected_loop: usize,
    pub selected_trap_type: usize,
    pub selected_track_type: GeneralTrackType,
    pub selected_riff_uuid: String,
    pub selected_riff_events: Vec<String>,
    pub time_signature_denominator: i32,
    pub time_signature_numerator: i32,
    pub track_event_copy_buffer: Vec<TrackEvent>,
    pub track_render_audio_consumers: Arc<Mutex<HashMap<String, AudioConsumerDetails<AudioBlock>>>>,
    pub track_type_dropdown_toggle: bool,
    pub track_type_options: Vec<String>,
    pub track_view_scroll_x: f32,
    pub track_view_scroll_y: f32,
    pub width: f32,

    pub riff_seq_selected_riff_set_index: usize,

    pub audio_layer_sender: Option<UnboundedSender<AudioLayerEvent>>,

    pub vst24_plugin_loaders: Arc<Mutex<HashMap<String, PluginLoader<VstHost>>>>,
    pub clap_plugin_loaders: Arc<Mutex<HashMap<String, PluginLibrary>>>,

    pub automation_view_state: AutomationViewState,
    pub piano_roll_state: PianoRollState,
    pub track_detail_view_state: TrackDetailViewState,
    pub track_grid_state: TrackGridState,
    pub riff_set_view_state: RiffSetViewState,
    pub riff_sequence_view_state: RiffSequenceViewState,
    pub riff_grid_view_state: RiffGridViewState,
    pub riff_arrangement_view_state: RiffArrangementViewState,

    pub recorded_playing_notes: HashMap<i32, f64>,

    pub file_dialog: FileDialog,

    axis_values: HashMap<String, f64>,
}


impl SyncState for RiffDAWState {
    fn axis(&self, key: &str) -> f64 {
        self.axis_values.get(key).copied().unwrap_or(0.0)
    }

    fn set_axis(&mut self, key: &str, value: f64) {
        self.axis_values.insert(key.to_string(), value);
    }
}


impl RiffDAWState {
    pub fn new(sender: crossbeam_channel::Sender<DAWEvents>) -> Self {
        let configuration = DAWConfiguration::load_config();
        let bookmark_paths = configuration.bookmark_paths.iter().map(|bookmark_path| PathBuf::from(bookmark_path.as_str())).collect();
        Self {
            configuration,
            history_manager: Arc::new(Mutex::new(HistoryManager::new())),
            project: Arc::new(Mutex::new(Project::new())),
            current_file_path: None,
            selected_track: None,
            selected_riff_uuid_map: HashMap::new(),
            selected_riff_ref_uuid: None,
            audio_plugin_parameters: HashMap::new(),
            active_loop: None,
            looping: false,
            recording: false,
            playing: false,
            play_mode: PlayMode::Song,
            playing_riff_set: None,
            playing_riff_sequence: None,
            playing_riff_grid: None,
            playing_riff_arrangement: None,
            playing_riff_sequence_summary_data: None,
            playing_riff_grid_summary_data: None,
            playing_riff_arrangement_summary_data: None,
            play_position_in_frames: 0,
            track_event_copy_buffer: vec![],
            riff_grid_riff_references_copy_buffer: vec![],
            note_expression_id: -1,
            note_expression_port_index: -1,
            note_expression_channel: -1,
            note_expression_key: -1,
            note_expression_type: NoteExpressionType::Volume,
            parameter_index: None,
            selected_effect_plugin_uuid: None,
            sample_data: HashMap::new(),
            track_render_audio_consumers: Arc::new(Mutex::new(HashMap::new())),
            centre_split_pane_position: 600,
            riff_grid_cursor_follow: true,
            current_view: CurrentView::Track,
            dirty: false,
            selected_automation: Vec::new(),
            selected_riff_events: Vec::new(),
            riff_set_selected_uuid: None,
            riff_sequence_riff_set_reference_selected_uuid: None,
            selected_riff_grid_riff_references: vec![],
            vst24_plugin_loaders: Arc::new(Mutex::new(HashMap::new())),
            clap_plugin_loaders: Arc::new(Mutex::new(HashMap::new())),


            event_edit_view: EventEditView::PianoRoll,
            open_file_dialog_window_id: WindowId::next(),
            open_file_dialogue: HashMap::new(),
            save_file_dialog_window_id: WindowId::next(),
            save_file_dialogue: HashMap::new(),
            track_details_window: HashMap::new(),
            riff_name: None,
            riff_name_window_id: WindowId::next(),
            riff_name_window: HashMap::new(),
            settings_window_id: WindowId::next(),
            settings_window: HashMap::new(),
            height: 200.,
            main_view: RiffDAWMainView::Track,
            main_window_id: WindowId::next(),
            play_position: 0.0,
            riff_view: RiffView::RiffSet,
            running: true,
            selected_loop: usize::MAX,
            selected_track_type: GeneralTrackType::InstrumentTrack,
            selected_trap_type: 2,
            time_signature_denominator: 4,
            time_signature_numerator: 4,
            track_details_window_id: WindowId::next(),
            track_type_dropdown_toggle: false,
            track_type_options: vec!["Audio track".to_string(), "Midi track".to_string(), "Instrument track".to_string()],
            track_view_scroll_x: 0.0,
            track_view_scroll_y: 0.0,
            width: 200.,
            riff_seq_selected_riff_set_index: 0,

            audio_layer_sender: None,

            selected_riff_uuid: "".to_string(),

            automation_view_state: AutomationViewState {
                automation_view_mode: AutomationViewMode::NoteVelocities,
                automation_edit_type: AutomationEditType::Track,
                controller_type_index: None,
                instrument_parameter_type: None,
                effect_parameter_type: None,
                automation_event_copy_buffer: vec![],
                automation_discrete: true,
                automation_edit_cursor_time_in_beats: 0.0,
                automation_grid_operation_mode: OperationModeType::PointMode,
                draw_mode: DrawMode::Point,
                note_expression_id: MidiPolyphonicExpressionNoteId::ALL,
                note_expression_port_index: NoteExpressionPortIndex::Global,
                note_expression_channel: NoteExpressionChannel::Global,
                note_expression_key: NoteExpressionKey::Global,
                note_expression_type: NoteExpressionType::Volume,
                window_undock: false,
            },

            piano_roll_state: PianoRollState {
                piano_roll_mpe_note_id: MidiPolyphonicExpressionNoteId::ALL,
                piano_roll_edit_cursor_time_in_beats: 0.0,
                piano_roll_edit_cursor_position: 0.0,
                piano_roll_selected_snap: MUSICAL_ITEM_LENGTH_OPTIONS.iter().find_position(|snap| "1/4" == **snap).unwrap().0,
                piano_roll_grid_operation_mode: OperationModeType::PointMode,
                piano_roll_mpe_voice_picklist_options: vec![
                    "All".to_string(),
                    "0".to_string(),
                    "1".to_string(),
                    "2".to_string(),
                    "3".to_string(),
                    "4".to_string(),
                    "5".to_string(),
                    "6".to_string(),
                    "7".to_string(),
                    "8".to_string(),
                    "9".to_string(),
                    "10".to_string()
                ],
                piano_roll_quantise_end: false,
                piano_roll_quantise_quantise_strength: 100,
                piano_roll_quantise_start: true,
                piano_roll_scroll_y: 0.0,
                piano_roll_subdivision_options: vec!["Normal".to_string(), "Triplet".to_string()],
                piano_roll_triplet_options: vec!["1/4 triplet".to_string(), "1/8 triplet".to_string(), "1/16 triplet".to_string()],
                selected_piano_roll_note_adj: 10,
                selected_piano_roll_note_length_option: 15,
                selected_piano_roll_subdivision: 0,
                selected_piano_roll_triplet: 1,
                window_undock: false,
            },

            track_detail_view_state: TrackDetailViewState {
                add_riff_name: "".to_string(),
                add_riff_length: 4.0,
                add_riff_length_text: MUSICAL_ITEM_LENGTH_OPTIONS.get(10).unwrap().to_string(),
            },

            track_grid_state: TrackGridState {
                track_grid_riff_references_copy_buffer: vec![],
                track_grid_cursor_follow: true,
                selected_track_grid_riff_references: vec![],
                track_operation_mode_select: true,
                track_grid_edit_cursor_time_in_beats: 0.0,
                track_grid_operation_mode: OperationModeType::PointMode,
                track_grid_edit_cursor_position: 0.0,
                track_grid_selected_snap: MUSICAL_ITEM_LENGTH_OPTIONS.iter().find_position(|snap| "1" == **snap).unwrap().0,
                show_automation: false,
                show_notes: false,
                show_note_velocities: false,
                show_pan: false,
            },

            riff_set_view_state: RiffSetViewState {
                add_riff_set_name: "".to_string(),
            },

            riff_sequence_view_state: RiffSequenceViewState {
                add_riff_sequence_name: "".to_string(),
                add_to_seq_riff_set_index: 0,
                riff_seq_to_select_index: 0,
                selected_riff_sequence_uuid: None,
            },

            riff_grid_view_state: RiffGridViewState {
                add_riff_grid_name: "".to_string(),
                riff_grid_to_select_index: 0,
                selected_riff_grid_uuid: None,
            },

            riff_arrangement_view_state: RiffArrangementViewState {
                add_riff_arrangement_name: "".to_string(),
                riff_arrangement_riff_item_selected_uuid: None,
                add_to_arr_riff_set_index: 0,
                add_to_arr_riff_seq_index: 0,
                add_to_arr_riff_grid_index: 0,
                riff_arr_to_select_index: 0,
                selected_riff_arrangement_uuid: None,
            },

            recorded_playing_notes: HashMap::new(),

            file_dialog: FileDialog {
                dialog_window_id: None,
                dialog: DialogState::new("fdaw".to_string()),
                bookmarks: Bookmarks {
                    items: bookmark_paths,
                    selected: None,
                },
            },

            axis_values: HashMap::new(),
        }
    }

    pub fn load_from_file(&mut self,
                          // vst24_plugin_loaders: Arc<Mutex<HashMap<String, PluginLoader<VstHost>>>>,
                          // clap_plugin_loaders: Arc<Mutex<HashMap<String, PluginLibrary>>>,
                          path: &str,
                          // tx_audio: crossbeam_channel::Sender<AudioLayerInwardEvent>,
                          // track_audio_coast: Arc<Mutex<TrackBackgroundProcessorMode>>,
                          // vst_host_time_info: Arc<RwLock<TimeInfo>>,
    ) {
        self.current_file_path = Some(path.to_string());
        let json_text = std::fs::read_to_string(path).unwrap();
        let mut project: Project = serde_json::from_str(&json_text).unwrap();
        // let mut instrument_track_senders2 = HashMap::new();
        // let mut instrument_track_receivers2 = HashMap::new();


        // let mut song_length_in_beats: u64 = 0;

        // load all the samples - create the sample data objects
        let sample_rate = self.configuration.audio.sample_rate;
        let mut sample_references = HashMap::new();
        let mut samples_data = HashMap::new();
        for (_sample_uuid, sample) in project.song_mut().samples_mut().iter_mut() {
            let sample_data_uuid = sample.sample_data_uuid();
            let sample_file_name = sample.file_name();

            let sample_data = SampleData::new_with_uuid(sample_data_uuid.to_string(), sample_file_name.to_string(), sample_rate as i32);
            samples_data.insert(sample_data_uuid.to_string(), sample_data);
            sample_references.insert(sample.uuid().to_string(), sample_data_uuid.to_string());
        }
        for (sample_data_uuid, sample_data) in samples_data.iter() {
            self.sample_data_mut().insert(sample_data_uuid.to_string(), sample_data.clone());
        }

        println!("state.load_from_file() - number of riff sequences={}", project.song().riff_sequences().len());

        {
            let sample_rate = self.configuration.audio.sample_rate as f64;
            let block_size = self.configuration.audio.block_size as f64;
            let tempo = project.song().tempo();
            let time_signature_numerator = project.song().time_signature_numerator();
            let time_signature_denominator = project.song().time_signature_denominator();
            for track in project.song_mut().tracks_mut().iter_mut() {
                self.init_track(
                    // vst24_plugin_loaders.clone(),
                    // clap_plugin_loaders.clone(),
                    // tx_audio.clone(),
                    // track_audio_coast.clone(),
                    // &mut instrument_track_senders2,
                    // &mut instrument_track_receivers2,
                    track,
                    Some(&sample_references),
                    Some(&samples_data),
                    // vst_host_time_info.clone(),
                    sample_rate,
                    block_size,
                    tempo,
                    time_signature_numerator as i32,
                    time_signature_denominator as i32,
                );
            }
        }

        // self.update_track_senders_and_receivers(instrument_track_senders2, instrument_track_receivers2);

        {
            for track in project.song().tracks().iter() {
                let track_from_uuid = track.uuid().to_string();

                for routing in track.midi_routings().iter() {
                    self.send_midi_routing_to_track_background_processors(track_from_uuid.clone(), routing.clone());
                }
            }
        }

        // set the transient event ids (don't need to be persisted)
        {
            let mut track_uuids = vec![];

            // set the event ids in the track automation events
            for track in project.song_mut().tracks_mut().iter_mut() {
                track_uuids.push(track.uuid_string());

                for event in track.automation_mut().events_mut().iter_mut() {
                    event.set_id(Uuid::new_v4().to_string());
                }
            }

            // set the event ids in the riff arrangement automation events
            for riff_arrangement in project.song_mut().riff_arrangements_mut().iter_mut() {
                for track_uuid in track_uuids.iter() {
                    for automation in riff_arrangement.automation_mut(track_uuid) {
                        for event in automation.events_mut().iter_mut() {
                            event.set_id(Uuid::new_v4().to_string());
                        }
                    }
                }
            }
        }

        if let Ok(old_project) = self.project.lock().as_mut() {
            *(old_project.deref_mut()) = project;
        }
    }

    pub fn update_track_senders_and_receivers(&mut self, instrument_track_senders2: HashMap<Option<String>, Sender<TrackBackgroundProcessorInwardEvent>>, instrument_track_receivers2: HashMap<Option<String>, Receiver<TrackBackgroundProcessorOutwardEvent>>) {
        // for (uuid, sender) in instrument_track_senders2 {
        //     match uuid {
        //         Some(uuid) => {
        //             self.instrument_track_senders_mut().insert(uuid, sender);
        //         },
        //         None => println!("Entry did not contain a uuid."),
        //     }
        // }
        //
        // for (uuid, receiver) in instrument_track_receivers2 {
        //     match uuid {
        //         Some(uuid) => {
        //             self.instrument_track_receivers_mut().insert(uuid, receiver);
        //         },
        //         None => println!("Entry did not contain a uuid."),
        //     }
        // }
    }

    pub fn init_track(
        &self,
        // vst24_plugin_loaders: Arc<Mutex<HashMap<String, PluginLoader<VstHost>>>>,
        // clap_plugin_loaders: Arc<Mutex<HashMap<String, PluginLibrary>>>,
        // tx_audio: crossbeam_channel::Sender<AudioLayerInwardEvent>,
        // track_audio_coast: Arc<Mutex<TrackBackgroundProcessorMode>>,
        // instrument_track_senders2: &mut HashMap<Option<String>, Sender<TrackBackgroundProcessorInwardEvent>>,
        // instrument_track_receivers2: &mut HashMap<Option<String>, Receiver<TrackBackgroundProcessorOutwardEvent>>,
        track_type: &mut TrackType,
        sample_references: Option<&HashMap<String, String>>,
        samples_data: Option<&HashMap<String, SampleData>>,
        // vst_host_time_info: Arc<RwLock<TimeInfo>>,
        sample_rate: f64,
        block_size: f64,
        tempo: f64,
        time_signature_numerator: i32,
        time_signature_denominator: i32,
    ) {
        // let (tx_to_vst, rx_to_vst) = channel::<TrackBackgroundProcessorInwardEvent>();
        // let tx_to_vst_ref = tx_to_vst.clone();
        // let (tx_from_vst, rx_from_vst) = channel::<TrackBackgroundProcessorOutwardEvent>();
        let track_uuid_string = track_type.uuid();
        // let volume = track_type.volume_mut();
        // let pan = track_type.pan_mut();

        match track_type {
            TrackType::InstrumentTrack(track) => {
                let effect_presets = {

                    if let Some(sender) = self.audio_layer_sender.as_ref() {
                        sender.send(AudioLayerEvent::AddTrackBackgroundProcessor(GeneralTrackType::InstrumentTrack, track_uuid_string.clone()));
                    }

                    let mut effect_presets = vec![];
                    for effect in track.effects_mut() {
                        effect_presets.push(String::from(effect.preset_data()));
                        let mut effect_details = String::from(effect.file());

                        effect_details.push(':');
                        match effect.sub_plugin_id() {
                            Some(sub_plugin_id) => {
                                effect_details.push_str(sub_plugin_id.to_string().as_str());
                            },
                            None => (),
                        }

                        effect_details.push(':');
                        effect_details.push_str(effect.plugin_type());

                        if let Some(sender) = self.audio_layer_sender.as_ref() {
                            sender.send(AudioLayerEvent::TrackBackgroundProcessorInward(TrackBackgroundProcessorInwardEvent::AddEffect(Arc::new(Mutex::new(HashMap::new())), Arc::new(Mutex::new(HashMap::new())), Uuid::parse_str(effect.uuid().as_str()).unwrap(), effect_details), track_uuid_string.clone()));
                        }
                    }
                    effect_presets
                };
                let preset = {
                    let instrument = track.instrument_mut();
                    let mut instrument_details = String::from(instrument.file());
                    let instrument_uuid = instrument.uuid();

                    instrument_details.push(':');
                    match instrument.sub_plugin_id() {
                        Some(sub_plugin_id) => {
                            instrument_details.push_str(sub_plugin_id.to_string().as_str());
                        },
                        None => (),
                    }

                    instrument_details.push(':');
                    instrument_details.push_str(instrument.plugin_type());

                    if instrument_details.contains(".so") || instrument_details.contains(".clap") || instrument_details.contains(".vst3") {
                        if let Some(sender) = self.audio_layer_sender.as_ref() {
                            sender.send(AudioLayerEvent::TrackBackgroundProcessorInward(
                                TrackBackgroundProcessorInwardEvent::ChangeInstrument(
                                    Arc::new(Mutex::new(HashMap::new())),
                                    Arc::new(Mutex::new(HashMap::new())),
                                    Uuid::parse_str(instrument_uuid.as_str()).unwrap(),
                                    instrument_details
                                ), track_uuid_string.clone()));
                        }
                        let preset_data = instrument.preset_data();
                        if !preset_data.is_empty() {
                            Some(preset_data)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };
                if let Some(preset_data) = preset {
                    if let Some(sender) = self.audio_layer_sender.as_ref() {
                        let _ = sender.send(AudioLayerEvent::TrackBackgroundProcessorInward(TrackBackgroundProcessorInwardEvent::SetPresetData(String::from(preset_data), effect_presets), track_uuid_string.clone()));
                    }
                }
            },
            TrackType::AudioTrack(track) => {

                // send all sample data referenced in riffs to the track background processor
                for riff in track.riffs().iter() {
                    let riff: &Riff = riff;
                    for event in riff.events().iter() {
                        if let TrackEvent::Sample(sample_reference) = event {
                            if let Some(sample_references) = sample_references {
                                if let Some(sample_data_uuid) = sample_references.get(&sample_reference.sample_ref_uuid().to_string()) {
                                    if let Some(samples_data) = samples_data {
                                        if let Some(sample_data) = samples_data.get(sample_data_uuid) {
                                            if let Some(sender) = self.audio_layer_sender.as_ref() {
                                                let _ = sender.send(AudioLayerEvent::TrackBackgroundProcessorInward(TrackBackgroundProcessorInwardEvent::SetSample(sample_data.clone()), track_uuid_string.clone()));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                let track_uuid_string = track.uuid().to_string();
                if let Some(sender) = self.audio_layer_sender.as_ref() {
                    sender.send(AudioLayerEvent::AddTrackBackgroundProcessor(GeneralTrackType::AudioTrack, track_uuid_string.clone()));
                }
            },
            TrackType::MidiTrack(track) => {
                if let Some(sender) = self.audio_layer_sender.as_ref() {
                    sender.send(AudioLayerEvent::AddTrackBackgroundProcessor(GeneralTrackType::MidiTrack, track_uuid_string));
                }
            },
        }
    }

    pub fn start_default_track_background_processing(&mut self,
                                                     tx_audio: crossbeam_channel::Sender<AudioLayerInwardEvent>,
                                                     track_audio_coast: Arc<Mutex<AudioMode>>,
                                                     track_uuid: String,
                                                     vst_host_time_info: Arc<RwLock<TimeInfo>>,
    ) {
        if let Ok(mut project) = self.project.lock() {
            let (tx_to_vst, rx_to_vst) = channel::<TrackBackgroundProcessorInwardEvent>();
            let (tx_from_vst, rx_from_vst) = channel::<TrackBackgroundProcessorOutwardEvent>();
            let mut instrument_track_senders2 = HashMap::new();
            let mut instrument_track_receivers2 = HashMap::new();
            let sample_rate = self.configuration.audio.sample_rate as f64;
            let block_size = self.configuration.audio.block_size as f64;
            let tempo = project.song().tempo();
            let time_signature_numerator = project.song().time_signature_numerator();
            let time_signature_denominator = project.song().time_signature_denominator();

            match project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                Some(track) => {
                    let track_uuid_string = track.uuid().to_string();
                    instrument_track_senders2.insert(track_uuid_string.clone(), tx_to_vst);
                    instrument_track_receivers2.insert(track_uuid_string, rx_from_vst);
                    // track.start_background_processing(tx_audio, rx_to_vst, tx_from_vst, track_audio_coast, track.volume(), track.pan(), vst_host_time_info,
                    //                                   sample_rate,
                    //                                   block_size,
                    //                                   tempo,
                    //                                   time_signature_numerator as i32,
                    //                                   time_signature_denominator as i32,
                    // );
                },
                None => {}
            }

            // for (uuid, sender) in instrument_track_senders2 {
            //     self.instrument_track_senders_mut().insert(uuid, sender);
            // }
            //
            // for (uuid, receiver) in instrument_track_receivers2 {
            //     self.instrument_track_receivers_mut().insert(uuid, receiver);
            // }
        }
    }

    pub fn load_instrument(&mut self,
                           instrument_details: String,
                           track_uuid: String,
    ) {
        if let Ok(mut project) = self.project.lock() {
            let mut index = 0;
            for track_type in project.song_mut().tracks_mut() {
                match track_type {
                    TrackType::InstrumentTrack(track) => if track.uuid().to_string() == track_uuid {
                        let (sub_plugin_id, library_path, plugin_type) = get_plugin_details(instrument_details.clone());
                        let instrument = track.instrument_mut();
                        let instrument_uuid = Uuid::new_v4();
                        instrument.set_uuid(instrument_uuid.clone());
                        instrument.is_instrument = true;
                        instrument.set_file(library_path);
                        instrument.set_sub_plugin_id(sub_plugin_id);
                        instrument.set_plugin_type(plugin_type);

                        if instrument_details.contains(".so") || instrument_details.contains(".clap") || instrument_details.contains(".vst3") {
                            match self.audio_layer_sender.as_ref() {
                                Some(sender) => {
                                    let fake1 = Arc::new(Mutex::new(HashMap::new()));
                                    let fake2 = Arc::new(Mutex::new(HashMap::new()));
                                    match sender.send(AudioLayerEvent::TrackBackgroundProcessorInward(TrackBackgroundProcessorInwardEvent::ChangeInstrument(
                                        fake1, fake2, instrument_uuid, instrument_details), track.uuid())) {
                                        Ok(_) => (),
                                        Err(error) => println!("{:?}", error),
                                    }
                                    let _ = sender.send(AudioLayerEvent::TrackBackgroundProcessorInward(TrackBackgroundProcessorInwardEvent::RequestInstrumentParameters, track_uuid.clone()));
                                },
                                None => println!("Couldn't send message to track!"),
                            };
                        }
                        break;
                    },
                    TrackType::AudioTrack(track) => if track.uuid().to_string() == track_uuid {
                        break;
                    },
                    TrackType::MidiTrack(track) => if track.uuid().to_string() == track_uuid {
                        break;
                    },
                };
                index += 1;
            }

            // self.sender.send(DAWEvents::UpdateUI);
        }
    }

    pub fn send_to_track_background_processor(&self, track_uuid_string: String, message: TrackBackgroundProcessorInwardEvent) {
        if let Some(sender) = self.audio_layer_sender.as_ref() {
            let _ = sender.send(AudioLayerEvent::TrackBackgroundProcessorInward(message, track_uuid_string.clone()));
        }
    }

    pub fn send_midi_routing_to_track_background_processors(&self, track_from_uuid: String, routing: TrackEventRouting) {
        // create the consumer producer pair
        let track_event_ring_buffer: SpscRb<TrackEvent> = SpscRb::new(EVENT_BUFFER_SIZE);
        let track_event_producer = track_event_ring_buffer.producer();
        let track_event_consumer = track_event_ring_buffer.consumer();

        // send the producer to the originating track
        self.send_to_track_background_processor(
            track_from_uuid.clone(),
            TrackBackgroundProcessorInwardEvent::AddTrackEventSendRouting(routing.clone(), track_event_ring_buffer, track_event_producer)
        );

        // send the consumer to the destination track
        let destination_track_uuid = match &routing.destination {
            TrackEventRoutingNodeType::Track(track_uuid) => track_uuid.clone(),
            TrackEventRoutingNodeType::Instrument(track_uuid, _) => track_uuid.clone(),
            TrackEventRoutingNodeType::Effect(track_uuid, _) => track_uuid.clone(),
        };

        self.send_to_track_background_processor(
            destination_track_uuid,
            TrackBackgroundProcessorInwardEvent::AddTrackEventReceiveRouting(routing.clone(), track_event_consumer)
        );
    }

    pub fn send_audio_routing_to_track_background_processors(&self, track_from_uuid: String, routing: AudioRouting) {
        // create the consumer producer pair
        let audio_ring_buffer_left: SpscRb<f32> = SpscRb::new(self.configuration.audio.block_size as usize);
        let audio_producer_left = audio_ring_buffer_left.producer();
        let audio_consumer_left = audio_ring_buffer_left.consumer();
        let audio_ring_buffer_right: SpscRb<f32> = SpscRb::new(self.configuration.audio.block_size as usize);
        let audio_producer_right = audio_ring_buffer_right.producer();
        let audio_consumer_right = audio_ring_buffer_right.consumer();

        // send the producer to the originating track
        self.send_to_track_background_processor(
            track_from_uuid.clone(),
            TrackBackgroundProcessorInwardEvent::AddAudioSendRouting(
                routing.clone(), (audio_ring_buffer_left, audio_ring_buffer_right), (audio_producer_left, audio_producer_right))
        );

        // send the consumer to the destination track
        let destination_track_uuid = match &routing.destination {
            AudioRoutingNodeType::Track(track_uuid) => track_uuid.clone(),
            AudioRoutingNodeType::Instrument(track_uuid, _, _, _) => track_uuid.clone(),
            AudioRoutingNodeType::Effect(track_uuid, _, _, _) => track_uuid.clone(),
        };

        self.send_to_track_background_processor(
            destination_track_uuid,
            TrackBackgroundProcessorInwardEvent::AddAudioReceiveRouting(routing.clone(), (audio_consumer_left, audio_consumer_right))
        );
    }

    fn request_presets_from_all_tracks(&mut self) {
        println!("Entering request_presets_from_all_tracks...");

        if let Ok(mut project) = self.project.lock() {
            let mut uuids = vec![];
            {
                for track_type in project.song_mut().tracks_mut() {
                    println!("Found track");
                    match track_type {
                        TrackType::InstrumentTrack(track) => {
                            println!("Adding instrument track uuid to vector: {}", track.uuid());
                            uuids.push(track.uuid().to_string());
                        }
                        TrackType::AudioTrack(track) => {
                            println!("Adding audio track uuid to vector: {}", track.uuid());
                            uuids.push(track.uuid().to_string());
                        }
                        TrackType::MidiTrack(_) => (),
                    }
                }
            }

            {
                for uuid in uuids {
                    println!("Found uuid in vector: {}", &uuid);
                    match self.audio_layer_sender.as_ref() {
                        Some(sender) => {
                            println!("State: requesting preset data from track with uuid: {}", uuid.clone());
                            match sender.send(AudioLayerEvent::TrackBackgroundProcessorInward(TrackBackgroundProcessorInwardEvent::RequestPresetData, uuid.clone())) {
                                Ok(_) => (),
                                Err(error) => println!("Problem requesting vst preset data for track: {}", error),
                            }
                        }
                        None => println!("Could not find tx_to_vst thread for track."),
                    }
                }
            }
        }
        println!("Exiting request_presets_from_all_tracks.");
    }

    fn save_presets_for_all_tracks(&mut self) {
        println!("Entering save_presets_for_all_tracks...");

        if let Ok(mut project) = self.project.lock() {
            // let mut presets = HashMap::new();

            {
                let track_data = project.song_mut().tracks_mut().iter_mut().map(|track| (track.uuid().to_string(), match track {
                    TrackType::InstrumentTrack(_) => GeneralTrackType::InstrumentTrack,
                    TrackType::AudioTrack(_) => GeneralTrackType::AudioTrack,
                    TrackType::MidiTrack(_) => GeneralTrackType::MidiTrack,
                })).collect_vec();
                for (track_uuid, track_type) in track_data.iter() {
                    match track_type {
                        GeneralTrackType::InstrumentTrack => {
                            // if let Some((uuid, track_outward_receiver)) = self.instrument_track_receivers_mut().iter_mut().find(|(uuid, _)| *track_uuid == **uuid) {
                            //     match track_outward_receiver.recv() {
                            //         Ok(preset_data) => {
                            //             println!("Instrument track preset data received: {}", uuid.clone());
                            //             presets.insert(String::from(uuid.as_str()), preset_data);
                            //         },
                            //         Err(error) => println!("Problem receiving instrument track thread plugin preset data for track uuid: {} {}", uuid.clone(), error),
                            //     }
                            // }
                        },
                        GeneralTrackType::AudioTrack => {
                            // if let Some((uuid, track_outward_receiver)) = self.instrument_track_receivers_mut().iter_mut().find(|(uuid, _)| *track_uuid == **uuid) {
                            //     match track_outward_receiver.recv() {
                            //         Ok(preset_data) => {
                            //             println!("Audio track preset data received: {}", uuid.clone());
                            //             presets.insert(String::from(uuid.as_str()), preset_data);
                            //         },
                            //         Err(error) => println!("Problem receiving audio track thread plugin preset data for track uuid: {} {}", uuid.clone(), error),
                            //     }
                            // }
                        },
                        _ => (),
                    }
                }
            }

            {
                // for (uuid, preset_data) in presets {
                //     for track_type in project.song_mut().tracks_mut() {
                //         match track_type {
                //             TrackType::InstrumentTrack(mut track) => {
                //                 if track.uuid().to_string().as_str() == uuid.as_str() {
                //                     if let TrackBackgroundProcessorOutwardEvent::GetPresetData(instrument_preset, effect_presets) = preset_data {
                //                         track.instrument_mut().set_preset_data(instrument_preset);
                //                         let mut index = 0;
                //                         for effect_preset in effect_presets {
                //                             match track.effects_mut().get_mut(index) {
                //                                 Some(effect) => effect.set_preset_data(effect_preset),
                //                                 None => println!("Effect could not be found for effect preset data at index: {}", index),
                //                             }
                //                             index += 1;
                //                         }
                //                     }
                //                     break;
                //                 }
                //             },
                //             TrackType::AudioTrack(track) => {
                //                 if track.uuid().to_string().as_str() == uuid.as_str() {
                //                     if let TrackBackgroundProcessorOutwardEvent::GetPresetData(_instrument_preset, effect_presets) = preset_data {
                //                         let mut index = 0;
                //                         for effect_preset in effect_presets {
                //                             match track.effects_mut().get_mut(index) {
                //                                 Some(effect) => effect.set_preset_data(effect_preset),
                //                                 None => println!("Effect could not be found for effect preset data at index: {}", index),
                //                             }
                //                             index += 1;
                //                         }
                //                     }
                //                     break;
                //                 }
                //             },
                //             TrackType::MidiTrack(_) => (),
                //         }
                //     }
                // }
            }
        }
        println!("Exiting save_presets_for_all_tracks...");
    }

    pub fn save(&mut self) {
        println!("Entering save...");
        self.request_presets_from_all_tracks();
        self.save_presets_for_all_tracks();

        if let Ok(mut project) = self.project.lock() {
            project.song_mut().recalculate_song_length();

            println!("state.save() - number of riff sequences={}", project.song().riff_sequences().len());

            match serde_json::to_string_pretty(project.deref_mut()) {
                Ok(json_text) => {
                    match self.get_current_file_path() {
                        Some(path) => {
                            match std::fs::write(path.clone(), json_text) {
                                Err(error) => println!("save failure writing to file: {}", error),
                                _ => {
                                    println!("saved to file: {}", path);
                                    self.dirty = false;
                                }
                            };
                        },
                        None => println!("No file path."),
                    }
                },
                Err(error) => {
                    println!("can_serialise failure: {}",error);
                }
            };
        }
        println!("Exited save.");
    }

    pub fn autosave(&mut self) {
        println!("Entering autosave...");
        self.request_presets_from_all_tracks();
        self.save_presets_for_all_tracks();

        if let Ok(mut project) = self.project.lock() {
            project.song_mut().recalculate_song_length();

            match serde_json::to_string_pretty(project.deref_mut()) {
                Ok(json_text) => {
                    match self.get_current_file_path() {
                        Some(path) => {
                            let autosave_path = format!("{}_{}.fdaw.xz", path, chrono::offset::Local::now().to_string());
                            if let Ok(compressed) = lzma::compress(json_text.as_bytes(), 6) {
                                match std::fs::write(autosave_path.clone(), compressed) {
                                    Err(error) => println!("save failure writing to file: {}", error),
                                    _ => println!("saved to file: {}", autosave_path)
                                };
                            }
                        }
                        None => {
                            let path = format!("/tmp/unknown_{}.fdaw.xz", chrono::offset::Local::now().to_string());
                            if let Ok(compressed) = lzma::compress(json_text.as_bytes(), 6) {
                                match std::fs::write(path.clone(), compressed) {
                                    Err(error) => println!("save failure writing to file: {}", error),
                                    _ => println!("saved to file: {}", path)
                                }
                            }
                        }
                    }
                }
                Err(error) => {
                    println!("autosave can't serialise project to JSON failure: {}",error);
                }
            }
        }
        println!("Exited autosave.");
    }

    pub fn save_as(&mut self, path: &str) {
        self.request_presets_from_all_tracks();
        self.save_presets_for_all_tracks();

        if let Ok(mut project) = self.project.lock() {
            self.current_file_path = Some(path.to_string());
            match serde_json::to_string_pretty(project.deref_mut()) {
                Ok(json_text) => {
                    match std::fs::write(path, json_text) {
                        Err(error) => println!("save as failure writing to file: {}", error),
                        _ => {
                            self.dirty = false;
                        }
                    };
                },
                Err(error) => {
                    println!("can_serialise failure: {}",error);
                }
            }
        }
    }

    pub fn get_project(&mut self) -> Arc<Mutex<Project>> {
        self.project.clone()
    }

    pub fn get_current_file_path(&self) -> &Option<String> {
        // let boris = self.current_file_path.clone().unwrap();
        // let mick = String::from(&boris[0..boris.len()]);
        // mick
        &self.current_file_path
    }

    pub fn set_project(&mut self, project: Project) {
        self.project = Arc::new(Mutex::new(project));
    }

    pub fn set_current_file_path(&mut self, current_file_path: Option<String>) {
        self.current_file_path = current_file_path;
    }

    /// Set the freedom daw state's selected track.
    pub fn set_selected_track(&mut self, selected_track: Option<String>) {
        self.selected_track = selected_track;
    }

    /// Set the freedom daw state's selected riff number.
    pub fn set_selected_riff_uuid(&mut self, track_uuid: String, selected_riff_uuid: String) {
        self.selected_riff_uuid_map.insert(track_uuid, selected_riff_uuid);
    }

    /// Set the freedom daw state's selected riff ref index.
    pub fn set_selected_riff_ref_uuid(&mut self, selected_riff_ref_uuid: Option<String>) {
        self.selected_riff_ref_uuid = selected_riff_ref_uuid;
    }

    /// Get a reference to the freedom daw state's selected riff track number.
    pub fn selected_track(&self) -> Option<String> {
        self.selected_track.clone()
    }

    /// Get a reference to the freedom daw state's selected riff track number.
    pub fn selected_track_mut(&mut self) -> &mut Option<String> {
        &mut self.selected_track
    }

    /// Get a reference to the freedom daw state's selected riff index.
    pub fn selected_riff_uuid(&self, track_uuid: String) -> Option<String> {
        self.selected_riff_uuid_map.get(&track_uuid).cloned()
    }

    /// Get a reference to the freedom daw state's selected riff ref index.
    pub fn selected_riff_ref_uuid(&self) -> Option<String> {
        self.selected_riff_ref_uuid.clone()
    }

    /// Get a mutable reference to the freedom daw state's selected riff index.
    pub fn selected_riff_uuid_mut(&mut self, track_uuid: String) -> Option<&mut String> {
        self.selected_riff_uuid_map.get_mut(&track_uuid)
    }

    /// Get a reference to the freedom daw state's project.
    pub fn project(&self) -> &Arc<Mutex<Project>> {
        &self.project
    }

    /// Get the freedom daw state's active loop.
    pub fn active_loop(&self) -> Option<Uuid> {
        self.active_loop
    }

    /// Set the freedom daw state's active loop.
    pub fn set_active_loop(&mut self, active_loop: Option<Uuid>) {
        self.active_loop = active_loop;
    }

    /// Get a mutable reference to the freedom daw state's active loop.
    pub fn active_loop_mut(&mut self) -> &mut Option<Uuid> {
        &mut self.active_loop
    }

    /// Get the freedom daw state's looping.
    pub fn looping(&self) -> bool {
        self.looping
    }

    /// Get the freedom daw state's looping.
    pub fn looping_mut(&mut self) -> &mut bool {
        &mut self.looping
    }

    /// Set the freedom daw state's looping.
    pub fn set_looping(&mut self, looping: bool) {
        self.looping = looping;
    }

    /// Get the freedom daw state's recording.
    pub fn recording(&self) -> bool {
        self.recording
    }

    /// Get a mutable reference to the freedom daw state's recording.
    pub fn recording_mut(&mut self) -> &mut bool {
        &mut self.recording
    }

    /// Set the freedom daw state's recording.
    pub fn set_recording(&mut self, recording: bool) {
        self.recording = recording;
    }

    /// Get the freedom daw state's playing.
    pub fn playing(&self) -> bool {
        self.playing
    }

    /// Get a mutable reference to the freedom daw state's playing.
    pub fn playing_mut(&mut self) -> &mut bool {
        &mut self.playing
    }

    /// Set the freedom daw state's playing.
    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }

    /// Get the freedom daw state's play position in frames.
    pub fn play_position_in_frames(&self) -> u32 {
        self.play_position_in_frames
    }

    /// Set the freedom daw state's play position in frames.
    pub fn set_play_position_in_frames(&mut self, play_position_in_frames: u32) {
        self.play_position_in_frames = play_position_in_frames;
    }

    /// Get a reference to the freedom daw state's track event copy buffer.
    pub fn track_event_copy_buffer(&self) -> &[TrackEvent] {
        self.track_event_copy_buffer.as_ref()
    }

    /// Get a mutable reference to the freedom daw state's track event copy buffer.
    pub fn track_event_copy_buffer_mut(&mut self) -> &mut Vec<TrackEvent> {
        &mut self.track_event_copy_buffer
    }

    /// Get a reference to the freedom daw state's track grid riff references copy buffer.
    pub fn track_grid_riff_references_copy_buffer(&self) -> &[RiffReference] {
        self.track_grid_state.track_grid_riff_references_copy_buffer.as_ref()
    }

    /// Get a mutable reference to the freedom daw state's track grid riff references copy buffer.
    pub fn track_grid_riff_references_copy_buffer_mut(&mut self) -> &mut Vec<RiffReference> {
        &mut self.track_grid_state.track_grid_riff_references_copy_buffer
    }

    /// Get a reference to the freedom daw state's riff grid riff references copy buffer.
    pub fn riff_grid_riff_references_copy_buffer(&self) -> &[RiffReference] {
        self.riff_grid_riff_references_copy_buffer.as_ref()
    }

    /// Get a mutable reference to the freedom daw state's riff grid riff references copy buffer.
    pub fn riff_grid_riff_references_copy_buffer_mut(&mut self) -> &mut Vec<RiffReference> {
        &mut self.riff_grid_riff_references_copy_buffer
    }

    /// Get a reference to the freedom daw state's automation view mode.
    #[must_use]
    pub fn automation_view_mode(&self) -> &AutomationViewMode {
        &self.automation_view_state.automation_view_mode
    }

    /// Set the freedom daw state's automation view mode.
    pub fn set_automation_view_mode(&mut self, automation_view_mode: AutomationViewMode) {
        self.automation_view_state.automation_view_mode = automation_view_mode;
    }

    /// Get a mutable reference to the freedom daw state's automation view mode.
    #[must_use]
    pub fn automation_view_mode_mut(&mut self) -> &mut AutomationViewMode {
        &mut self.automation_view_state.automation_view_mode
    }

    /// Get a mutable reference to the freedom daw state's vst plugin parameters.
    #[must_use]
    pub fn audio_plugin_parameters_mut(&mut self) -> &mut HashMap<String, HashMap<String, Vec<PluginParameterDetail>>> {
        &mut self.audio_plugin_parameters
    }

    /// Get a reference to the freedom daw state's vst plugin parameters.
    #[must_use]
    pub fn audio_plugin_parameters(&self) -> &HashMap<String, HashMap<String, Vec<PluginParameterDetail>>> {
        &self.audio_plugin_parameters
    }

    /// Get the freedom daw state's parameter index.
    #[must_use]
    pub fn parameter_index(&self) -> Option<i32> {
        self.parameter_index
    }

    /// Set the freedom daw state's parameter index.
    pub fn set_parameter_index(&mut self, parameter_index: Option<i32>) {
        self.parameter_index = parameter_index;
    }

    /// Get a mutable reference to the freedom daw state's parameter index.
    #[must_use]
    pub fn parameter_index_mut(&mut self) -> &mut Option<i32> {
        &mut self.parameter_index
    }

    /// Get a reference to the freedom daw state's selected effect plugin uuid.
    #[must_use]
    pub fn selected_effect_plugin_uuid(&self) -> Option<&String> {
        self.selected_effect_plugin_uuid.as_ref()
    }

    /// Set the freedom daw state's selected effect plugin uuid.
    pub fn set_selected_effect_plugin_uuid(&mut self, selected_effect_plugin_uuid: Option<String>) {
        self.selected_effect_plugin_uuid = selected_effect_plugin_uuid;
    }

    /// Get a mutable reference to the freedom daw state's selected effect plugin uuid.
    #[must_use]
    pub fn selected_effect_plugin_uuid_mut(&mut self) -> &mut Option<String> {
        &mut self.selected_effect_plugin_uuid
    }

    pub fn play_song(&mut self) -> i32 {
        std::thread::sleep(std::time::Duration::from_secs(5));
        let mut number_of_blocks = 0;

        self.set_playing(true);
        self.set_play_mode(PlayMode::Song);

        if let Ok(mut project) = self.project.lock() {
            let bpm = project.song().tempo();
            let time_signature_numerator = project.song().time_signature_numerator();
            let time_signature_denominator = project.song().time_signature_denominator();
            let sample_rate = self.configuration.audio.sample_rate as f64;
            let block_size = self.configuration.audio.block_size as f64;
            let mut song_length_in_beats = 400.0;
            let mut start_block = 0;
            let mut end_block = 0;
            let mut found_active_loop = false;

            // make sure everything is sorted
            project.song_mut().tracks_mut().iter_mut().for_each(|track| {
                track.riffs_mut().iter_mut().for_each(|riff| {
                    riff.events_mut().sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
                })
            });

            song_length_in_beats = *project.song_mut().length_in_beats_mut() as f64;

            let play_position_in_frames = self.play_position_in_frames();
            start_block = (play_position_in_frames as f64 / block_size) as i32;


            if self.looping {
                if let Some(loop_uuid) = &self.active_loop {
                    if let Some(active_loop) = project.song().loops().iter().find(|current_loop| current_loop.uuid().to_string() == loop_uuid.to_string()) {
                        let start_position_in_beats = active_loop.start_position();
                        let end_position_in_beats = active_loop.end_position();

                        found_active_loop = true;

                        start_block = (start_position_in_beats * sample_rate * 60.0 / bpm / block_size) as i32;
                        end_block = (end_position_in_beats * sample_rate * 60.0 / bpm / block_size) as i32;
                    }
                }
            }

            for track in project.song().tracks() {
                let midi_channel = if let TrackType::MidiTrack(midi_track) = track {
                    midi_track.midi_device().midi_channel()
                } else {
                    0
                };
                let vst_event_blocks = DAWUtils::convert_to_event_blocks(
                    track.automation(),
                    track.riffs(),
                    track.riff_refs(),
                    bpm,
                    block_size,
                    sample_rate,
                    song_length_in_beats,
                    midi_channel,
                    self.automation_discrete(),
                    time_signature_numerator,
                    time_signature_denominator,
                );
                self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEventProcessorType(EventProcessorType::BlockEventProcessor));
                self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEvents(vst_event_blocks, false));

                if found_active_loop {
                    self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Loop(true));
                    self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::LoopExtents(start_block, end_block));
                }
            }

            // thread::sleep(Duration::from_millis(2000));

            for track in project.song().tracks() {
                self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Play(start_block));
            }

            number_of_blocks = (song_length_in_beats / bpm * 60.0 * sample_rate / block_size) as i32;
            if let Some(sender) = self.audio_layer_sender.as_ref() {
                let _ = sender.send(AudioLayerEvent::AudioLayerInward(AudioLayerInwardEvent::Play(true, number_of_blocks, start_block)));
            }
        }

        number_of_blocks
    }

    pub fn play_riff_set(&mut self, riff_set_uuid: String) {
        println!("Playing riff set={}", riff_set_uuid.as_str());
        self.play_riff_set_as_riff(riff_set_uuid);
    }

    pub fn play_riff_set_in_blocks(&mut self, riff_set_uuid: String) {
        println!("Playing riff set in blocks={}", riff_set_uuid.as_str());

        let already_playing = self.playing();

        self.set_playing(true);
        self.set_play_mode(PlayMode::RiffSet);
        self.set_playing_riff_set(Some(riff_set_uuid.clone()));

        if let Ok(mut project) = self.project.lock() {
            let song = project.song();
            let play_position_in_frames = 0;
            let tracks = song.tracks();
            let bpm = song.tempo();
            let time_signature_numerator = song.time_signature_numerator();
            let time_signature_denominator = song.time_signature_denominator();
            let sample_rate = self.configuration.audio.sample_rate as f64;
            let block_size = self.configuration.audio.block_size as f64;
            let start_block = (play_position_in_frames as f64 / block_size) as i32;
            let mut lowest_common_factor_in_beats = 400;

            if let Some(riff_set) = project.song().riff_set(riff_set_uuid.clone()) {
                let mut riff_lengths = vec![];
                println!("Found riff set: uuid={}, name={}", riff_set_uuid.as_str(), riff_set.name());

                // get the number of repeats
                for track in project.song().tracks().iter() {
                    // get the riff_ref
                    if let Some(riff_ref) = riff_set.get_riff_ref_for_track(track.uuid().to_string()) {
                        // get the riff
                        if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_ref.linked_to()) {
                            riff_lengths.push(riff.length() as i32);
                        }
                    }
                }

                let (product, unique_riff_lengths) = RiffDAWState::get_length_product(riff_lengths);

                lowest_common_factor_in_beats = RiffDAWState::get_lowest_common_factor(unique_riff_lengths, product);

                for track in project.song().tracks().iter() {
                    let mut riff_refs = vec![];
                    let midi_channel = if let TrackType::MidiTrack(midi_track) = track {
                        midi_track.midi_device().midi_channel()
                    } else {
                        0
                    };

                    // get the riff_ref
                    if let Some(riff_ref) = riff_set.get_riff_ref_for_track(track.uuid().to_string()) {
                        // get the riff
                        if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_ref.linked_to()) {
                            for repeat in 0..(lowest_common_factor_in_beats / (riff.length() as i32)) {
                                // clone the riff set and set its position
                                let mut riff_reference = riff_ref.clone();
                                riff_reference.set_position(riff.length() * repeat as f64);
                                riff_refs.push(riff_reference);
                            }
                            let mut riffs = vec![];
                            riffs.push(riff.clone());
                            let automation = Automation::new();
                            let track_event_blocks = DAWUtils::convert_to_event_blocks(
                                &automation,
                                &riffs,
                                &riff_refs,
                                bpm,
                                block_size,
                                sample_rate,
                                lowest_common_factor_in_beats as f64,
                                midi_channel,
                                self.automation_discrete(),
                                time_signature_numerator,
                                time_signature_denominator
                            );
                            println!("Riff set # of blocks: {}", track_event_blocks.0.len());
                            self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::LoopExtents(0, track_event_blocks.0.len() as i32));
                            self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEvents(track_event_blocks, true));
                            self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Loop(true));
                        } else {
                            let track_event_blocks = (vec![], vec![]);
                            self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEvents(track_event_blocks, true));
                        }
                    } else {
                        let track_event_blocks = (vec![], vec![]);
                        self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEvents(track_event_blocks, true));
                    }
                }
            }

            if !already_playing {
                for track in tracks {
                    self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Play(start_block));
                }
            }

            let number_of_blocks = (lowest_common_factor_in_beats as f64 / bpm * 60.0 * sample_rate / block_size) as i32;
            if let Some(sender) = self.audio_layer_sender.as_ref() {
                match sender.send(AudioLayerEvent::AudioLayerInward(AudioLayerInwardEvent::Play(true, number_of_blocks, start_block))) {
                    Ok(_) => (),
                    Err(error) => println!("Problem using audio_layer_sender to send message to jack layer when turning play riff set on: {}", error),
                }
            }
        }
    }

    pub fn play_riff_set_as_riff(&mut self, riff_set_uuid: String) {
        println!("Playing riff set as riff={}", riff_set_uuid.as_str());

        let already_playing = self.playing();

        self.set_playing(true);
        self.set_play_mode(PlayMode::RiffSet);
        self.set_playing_riff_set(Some(riff_set_uuid.clone()));

        if let Ok(mut project) = self.project.lock() {
            let song = project.song();
            let play_position_in_frames = 0;
            let tracks = song.tracks();
            let bpm = song.tempo();
            let time_signature_numerator = song.time_signature_numerator();
            let time_signature_denominator = song.time_signature_denominator();
            let sample_rate = self.configuration.audio.sample_rate as f64;
            let block_size = self.configuration.audio.block_size as f64;
            let start_block = (play_position_in_frames as f64 / block_size) as i32;
            let number_of_blocks = i32::MAX;

            if let Some(riff_set) = project.song().riff_set(riff_set_uuid.clone()) {
                println!("Found riff set: uuid={}, name={}", riff_set_uuid.as_str(), riff_set.name());

                for track in project.song().tracks().iter() {
                    let mut riff_refs = vec![];
                    let midi_channel = if let TrackType::MidiTrack(midi_track) = track {
                        midi_track.midi_device().midi_channel()
                    } else {
                        0
                    };

                    if !already_playing {
                        self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEventProcessorType(EventProcessorType::RiffBufferEventProcessor));
                    }

                    // get the riff_ref
                    if let Some(riff_ref) = riff_set.get_riff_ref_for_track(track.uuid().to_string()) {
                        println!("Found riff_ref: track={}, riff_ref_linked_to={}", track.uuid(), riff_ref.linked_to());
                        let mut riff_reference = riff_ref.clone();
                        riff_refs.push(riff_reference);

                        // get the riff
                        if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_ref.linked_to()) {
                            let mut riffs = vec![];
                            riffs.push(riff.clone());

                            let mut track_events: Vec<TrackEvent> = DAWUtils::extract_riff_ref_events(&riffs, &riff_refs, bpm, sample_rate, midi_channel, time_signature_numerator, time_signature_denominator);

                            for track_event in track_events.iter() {
                                match track_event {
                                    TrackEvent::ActiveSense => println!("After sense: position={}", track_event.position()),
                                    TrackEvent::AfterTouch => println!("After touch: position={}", track_event.position()),
                                    TrackEvent::ProgramChange => println!("Program change: position={}", track_event.position()),
                                    TrackEvent::Note(_) => println!("Note: position={}", track_event.position()),
                                    TrackEvent::NoteOn(_) => println!("Note on: position={}", track_event.position()),
                                    TrackEvent::NoteOff(_) => println!("Note off: position={}", track_event.position()),
                                    TrackEvent::NoteExpression(_) => println!("Note expression: position={}", track_event.position()),
                                    TrackEvent::Controller(_) => println!("Controller: position={}", track_event.position()),
                                    TrackEvent::PitchBend(_) => println!("Pitch bend: position={}", track_event.position()),
                                    TrackEvent::KeyPressure => println!("Key pressure: position={}", track_event.position()),
                                    TrackEvent::AudioPluginParameter(_) => println!("Audio plugin parameter: position={}", track_event.position()),
                                    TrackEvent::Sample(_) => println!("Sample: position={}", track_event.position()),
                                    TrackEvent::Measure(_) => println!("Measure: position={}", track_event.position()),
                                }
                            }

                            let track_event_blocks = vec![track_events];

                            // TODO this needs to be patched in
                            let automation: Vec<PluginParameter> = vec![];
                            let automation_event_blocks = vec![automation];

                            // let track_event_blocks = DAWUtils::convert_to_event_blocks(&automation, &riffs, &riff_refs, bpm, block_size, sample_rate, lowest_common_factor_in_beats as f64, midi_channel);
                            println!("Riff set # of blocks: {}", track_event_blocks.len());
                            self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::LoopExtents(0, number_of_blocks));
                            self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEvents((track_event_blocks, automation_event_blocks), true));
                            self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Loop(true));
                        } else {
                            let track_event_blocks = (vec![], vec![]);
                            self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEvents(track_event_blocks, true));
                        }
                    } else {
                        let track_event_blocks = (vec![], vec![]);
                        self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEvents(track_event_blocks, true));
                    }
                }
            }

            if !already_playing {
                for track in tracks {
                    self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Play(start_block));
                }

                if let Some(sender) = self.audio_layer_sender.as_ref() {
                    match sender.send(AudioLayerEvent::AudioLayerInward(AudioLayerInwardEvent::Play(true, number_of_blocks, start_block))) {
                        Ok(_) => (),
                        Err(error) => println!("Problem using audio_layer_sender to send message to jack layer when turning play riff set as riff on: {}", error),
                    }
                }
            }
        }
    }



    pub fn play_riff_set_update_track_as_riff(&self, riff_set_uuid: String, track_uuid: String) {
        if let Ok(mut project) = self.project.lock() {
            let song = project.song();
            let bpm = song.tempo();
            let time_signature_numerator = song.time_signature_numerator();
            let time_signature_denominator = song.time_signature_denominator();
            let sample_rate = self.configuration.audio.sample_rate as f64;
            let number_of_blocks = i32::MAX;


            if let Some(riff_set) = project.song().riff_set(riff_set_uuid) {
                println!("state.play_riff_set_update_track: found riff set");
                for track in project.song().tracks().iter() {
                    if track.uuid().to_string() == track_uuid {
                        println!("state.play_riff_set_update_track_as_riff: found track");
                        let mut riff_refs = vec![];
                        let midi_channel = if let TrackType::MidiTrack(midi_track) = track {
                            midi_track.midi_device().midi_channel()
                        } else {
                            0
                        };

                        // get the riff_ref
                        if let Some(riff_ref) = riff_set.get_riff_ref_for_track(track.uuid().to_string()) {
                            let mut riff_reference = riff_ref.clone();
                            riff_refs.push(riff_reference);

                            // get the riff
                            if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_ref.linked_to()) {
                                let mut riffs = vec![];
                                riffs.push(riff.clone());

                                let mut track_events: Vec<TrackEvent> = DAWUtils::extract_riff_ref_events(&riffs, &riff_refs, bpm, sample_rate, midi_channel, time_signature_numerator, time_signature_denominator);

                                for track_event in track_events.iter() {
                                    match track_event {
                                        TrackEvent::ActiveSense => println!("After sense: position={}", track_event.position()),
                                        TrackEvent::AfterTouch => println!("After touch: position={}", track_event.position()),
                                        TrackEvent::ProgramChange => println!("Program change: position={}", track_event.position()),
                                        TrackEvent::Note(_) => println!("Note: position={}", track_event.position()),
                                        TrackEvent::NoteOn(_) => println!("Note on: position={}", track_event.position()),
                                        TrackEvent::NoteOff(_) => println!("Note off: position={}", track_event.position()),
                                        TrackEvent::NoteExpression(_) => println!("Note expression: position={}", track_event.position()),
                                        TrackEvent::Controller(_) => println!("Controller: position={}", track_event.position()),
                                        TrackEvent::PitchBend(_) => println!("Pitch bend: position={}", track_event.position()),
                                        TrackEvent::KeyPressure => println!("Key pressure: position={}", track_event.position()),
                                        TrackEvent::AudioPluginParameter(_) => println!("Audio plugin parameter: position={}", track_event.position()),
                                        TrackEvent::Sample(_) => println!("Sample: position={}", track_event.position()),
                                        TrackEvent::Measure(_) => println!("Measure: position={}", track_event.position()),
                                    }
                                }

                                let track_event_blocks = vec![track_events];

                                // TODO this needs to be patched in
                                let automation: Vec<PluginParameter> = vec![];
                                let automation_event_blocks = vec![automation];

                                println!("Riff set # of blocks: {}", track_event_blocks.len());
                                self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::LoopExtents(0, number_of_blocks));
                                self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEvents((track_event_blocks, automation_event_blocks), true));
                                self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Loop(true));
                            } else {
                                let vst_event_blocks = (vec![], vec![]);
                                println!("state.play_riff_set_update_track_as_riff: sending message to vst - set events without data");
                                self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEvents(vst_event_blocks, true));
                            }
                        } else {
                            let vst_event_blocks = (vec![], vec![]);
                            println!("state.play_riff_set_update_track_as_riff: sending message to vst - set events without data");
                            self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEvents(vst_event_blocks, true));
                        }
                        break;
                    }
                }
            }
        }
    }



    pub fn play_riff_set_update_track_in_blocks(&self, riff_set_uuid: String, track_uuid: String) {
        if let Ok(mut project) = self.project.lock() {
            let song = project.song();
            let bpm = song.tempo();
            let time_signature_numerator = song.time_signature_numerator();
            let time_signature_denominator = song.time_signature_denominator();
            let sample_rate = self.configuration.audio.sample_rate as f64;
            let block_size = self.configuration.audio.block_size as f64;
            let mut lowest_common_factor_in_beats = 400;


            if let Some(riff_set) = project.song().riff_set(riff_set_uuid) {
                println!("state.play_riff_set_update_track_in_blocks: found riff set");
                let mut riff_lengths = vec![];

                // get the number of repeats
                for track in project.song().tracks().iter() {
                    // get the riff_ref
                    if let Some(riff_ref) = riff_set.get_riff_ref_for_track(track.uuid().to_string()) {
                        // get the riff
                        if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_ref.linked_to()) {
                            riff_lengths.push(riff.length() as i32);
                        }
                    }
                }

                let (product, unique_riff_lengths) = RiffDAWState::get_length_product(riff_lengths);

                lowest_common_factor_in_beats = RiffDAWState::get_lowest_common_factor(unique_riff_lengths, product);

                for track in project.song().tracks().iter() {
                    if track.uuid().to_string() == track_uuid {
                        println!("state.play_riff_set_update_track_in_blocks: found track");
                        let mut riff_refs = vec![];
                        let midi_channel = if let TrackType::MidiTrack(midi_track) = track {
                            midi_track.midi_device().midi_channel()
                        } else {
                            0
                        };

                        // get the riff_ref
                        if let Some(riff_ref) = riff_set.get_riff_ref_for_track(track.uuid().to_string()) {
                            // get the riff
                            if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_ref.linked_to()) {
                                for repeat in 0..(lowest_common_factor_in_beats / (riff.length() as i32)) {
                                    // clone the riff set and set its position
                                    let mut riff_reference = riff_ref.clone();
                                    riff_reference.set_position(riff.length() * repeat as f64);
                                    riff_refs.push(riff_reference);
                                }
                                let mut riffs = vec![];
                                riffs.push(riff.clone());
                                let automation = Automation::new();
                                let vst_event_blocks = DAWUtils::convert_to_event_blocks(
                                    &automation,
                                    &riffs,
                                    &riff_refs,
                                    bpm,
                                    block_size,
                                    sample_rate,
                                    lowest_common_factor_in_beats as f64,
                                    midi_channel,
                                    self.automation_discrete(),
                                    time_signature_numerator,
                                    time_signature_denominator,
                                );
                                println!("state.play_riff_set_update_track_in_blocks: sending message to vst - set events with data");
                                self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEvents(vst_event_blocks, true));
                            } else {
                                let vst_event_blocks = (vec![], vec![]);
                                println!("state.play_riff_set_update_track_in_blocks: sending message to vst - set events without data");
                                self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEvents(vst_event_blocks, true));
                            }
                        } else {
                            let vst_event_blocks = (vec![], vec![]);
                            println!("state.play_riff_set_update_track_in_blocks: sending message to vst - set events without data");
                            self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEvents(vst_event_blocks, true));
                        }
                        break;
                    }
                }
            }
        }
    }



    pub fn get_length_product(riff_lengths: Vec<i32>) -> (i32, Vec<i32>) {
        let mut lengths = HashSet::new();
        for riff_length in riff_lengths.iter() {
            lengths.insert(riff_length);
        }

        let mut product = 0;
        let mut first = true;
        for length in lengths.iter() {
            if first {
                product = **length;
                first = false;
            }
            else {
                product *= **length;
            }
        }

        (product, lengths.iter().map(|value| **value).collect())
    }



    pub fn get_lowest_common_factor(unique_riff_lengths: Vec<i32>, product: i32) -> i32 {
        // get the factors of the product
        let product_factors = factor_include(product as i64);
        let mut list_of_lists_of_divisible_lengths = vec![];
        let mut unique_divisible_lengths = HashSet::new();

        for riff_length in unique_riff_lengths {
            let mut divisible_lengths = vec![];
            for product_factor in product_factors.iter() {
                if *product_factor as i32 % riff_length == 0 {
                    divisible_lengths.push(*product_factor as i32);
                    unique_divisible_lengths.insert(*product_factor as i32);
                }
            }
            list_of_lists_of_divisible_lengths.push(divisible_lengths);
        }

        // somehow find the intersection between all the divisible sets and get the lowest value
        let mut found_length = 0;
        let mut data = unique_divisible_lengths.iter().copied().collect::<Vec<i32>>();

        data.sort();

        for unique_divisible_length in data.iter() {
            let mut count = 0;
            for list_of_divisible_lengths in list_of_lists_of_divisible_lengths.iter() {
                for divisible_length in list_of_divisible_lengths.iter() {
                    if *unique_divisible_length == *divisible_length {
                        count += 1;
                    }
                }
            }
            if count == list_of_lists_of_divisible_lengths.len() as i32 {
                found_length = *unique_divisible_length;
                break;
            }
        }

        found_length
    }



    pub fn play_riff_sequence(&mut self, riff_sequence_uuid: String) {
        let song_length_in_beats = 400.0;

        let already_playing = self.playing();

        self.set_playing(true);
        self.set_play_mode(PlayMode::RiffSequence);

        if let Ok(mut project) = self.project.lock() {
            let song = project.song();
            let play_position_in_frames = 0;
            let bpm = song.tempo();
            let time_signature_numerator = song.time_signature_numerator();
            let time_signature_denominator = song.time_signature_denominator();
            let sample_rate = self.configuration.audio.sample_rate as f64;
            let block_size = self.configuration.audio.block_size as f64;
            let start_block = (play_position_in_frames as f64 / block_size) as i32;

            // get the riff sequence
            if let Some(riff_sequence) = song.riff_sequence(riff_sequence_uuid) {
                let mut track_riff_refs_map = HashMap::new();
                let mut track_running_position = HashMap::new();

                // setup
                for track in project.song().tracks().iter() {
                    let track_riff_refs: Vec<RiffReference> = vec![];
                    track_riff_refs_map.insert(track.uuid().to_string(), track_riff_refs);
                    track_running_position.insert(track.uuid().to_string(), 0.0);

                    if !already_playing {
                        self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEventProcessorType(EventProcessorType::BlockEventProcessor));
                    }
                }

                self.playing_riff_sequence_summary_data = Some(self.get_riff_sequence_play_events(riff_sequence, &mut track_riff_refs_map, &mut track_running_position, &project));

                // convert and send events
                for track in project.song().tracks().iter() {
                    println!("Track: uuid={} - ", track.uuid().to_string());

                    let midi_channel = if let TrackType::MidiTrack(midi_track) = track {
                        midi_track.midi_device().midi_channel()
                    } else {
                        0
                    };
                    // get the riff refs
                    let riff_refs = match track_riff_refs_map.remove(track.uuid().to_string().as_str()) {
                        None => Vec::<RiffReference>::new(),
                        Some(riff_refs) => riff_refs,
                    };
                    for riff_ref in riff_refs.iter() {
                        println!("Riff ref: uuid={}, position={}, length={} - ", riff_ref.uuid().to_string(), riff_ref.position(), riff_ref.linked_to());
                    }
                    println!("");
                    let automation = Automation::new();
                    let vst_event_blocks = DAWUtils::convert_to_event_blocks(
                        &automation,
                        track.riffs(),
                        &riff_refs,
                        bpm,
                        block_size,
                        sample_rate,
                        song_length_in_beats,
                        midi_channel,
                        self.automation_discrete(),
                        time_signature_numerator,
                        time_signature_denominator,
                    );
                    self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEvents(vst_event_blocks, false));
                }
            }

            let number_of_blocks = (song_length_in_beats / bpm * 60.0 * sample_rate / block_size) as i32;

            // tell each track audio to play
            for track in project.song().tracks() {
                self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Play(start_block));
                self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::LoopExtents(start_block, number_of_blocks));
            }

            // set the start block and the number of blocks in the jack audio layer
            if let Some(sender) = self.audio_layer_sender.as_ref() {
                match sender.send(AudioLayerEvent::AudioLayerInward(AudioLayerInwardEvent::Play(true, number_of_blocks, start_block))) {
                    Ok(_) => (),
                    Err(error) => println!("Problem using audio_layer_sender to send message to jack layer when turning play riff grid on: {}", error),
                }
            }
        }
    }




    pub fn play_riff_grid(&mut self, riff_grid_uuid: String) -> i32 {
        let mut number_of_blocks = 0;

        self.set_playing(true);
        self.set_play_mode(PlayMode::Song);

        if let Ok(mut project) = self.project.lock() {
            let bpm = project.song().tempo();
            let sample_rate = self.configuration.audio.sample_rate as f64;
            let block_size = self.configuration.audio.block_size as f64;
            let mut song_length_in_beats = project.song().length_in_beats() as f64;
            let mut start_block = 0;
            let mut end_block = 0;
            let already_playing = self.playing();

            let song = project.song();
            let time_signature_numerator = song.time_signature_numerator();
            let time_signature_denominator = song.time_signature_denominator();

            let play_position_in_frames = self.play_position_in_frames();
            start_block = (play_position_in_frames as f64 / block_size) as i32;

            let tracks = song.tracks();
            for track in tracks {
                let midi_channel = if let TrackType::MidiTrack(midi_track) = track {
                    midi_track.midi_device().midi_channel()
                } else {
                    0
                };
                let automation = Automation::new();
                let vst_event_blocks = if let Some(riff_grid) = song.riff_grid(riff_grid_uuid.clone()) {
                    if let Some(track_riff_refs) = riff_grid.track_riff_references(track.uuid().to_string()) {
                        DAWUtils::convert_to_event_blocks(&automation, track.riffs(), track_riff_refs, bpm, block_size, sample_rate, song_length_in_beats, midi_channel, self.automation_discrete(), time_signature_numerator, time_signature_denominator)
                    } else {
                        DAWUtils::convert_to_event_blocks(&automation, track.riffs(), &vec![], bpm, block_size, sample_rate, song_length_in_beats, midi_channel, self.automation_discrete(), time_signature_numerator, time_signature_denominator)
                    }
                } else {
                    DAWUtils::convert_to_event_blocks(&automation, track.riffs(), &vec![], bpm, block_size, sample_rate, song_length_in_beats, midi_channel, self.automation_discrete(), time_signature_numerator, time_signature_denominator)
                };

                if !already_playing {
                    self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEventProcessorType(EventProcessorType::BlockEventProcessor));
                }

                self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEvents(vst_event_blocks, false));
            }

            number_of_blocks = (song_length_in_beats / bpm * 60.0 * sample_rate / block_size) as i32;
            for track in tracks {
                self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Play(start_block));
                self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::LoopExtents(start_block, number_of_blocks));
            }

            if let Some(sender) = self.audio_layer_sender.as_ref() {
                match sender.send(AudioLayerEvent::AudioLayerInward(AudioLayerInwardEvent::Play(true, number_of_blocks, start_block))) {
                    Ok(_) => (),
                    Err(error) => println!("Problem using audio_layer_sender to send message to jack layer when turning play grid on: {}", error),
                }
            }
        }

        number_of_blocks
    }



    pub fn get_riff_arrangement_play_events(
        &self,
        riff_items: &Vec<RiffItem>,
        track_riff_refs_map: &mut HashMap<String, Vec<RiffReference>>,
        track_running_position: &mut HashMap<String, f64>) -> (f64, Vec<(f64, RiffItem, Vec<(f64, RiffItem)>)>) {
        let mut riff_arrangement_actual_play_length = 0.0;
        let mut riff_item_actual_play_lengths = vec![];

        if let Ok(mut project) = self.project.lock() {
            for riff_item in riff_items.iter() {
                match riff_item.item_type() {
                    RiffItemType::RiffSet => {
                        if let Some(riff_set) = project.song().riff_set(riff_item.item_uuid().to_string()) {
                            println!("state.play_arrangement: riff set name={}", riff_set.name());
                            let riff_set_actual_play_length = self.get_riff_set_play_events(riff_set, track_riff_refs_map, track_running_position, &project);
                            riff_arrangement_actual_play_length += riff_set_actual_play_length;
                            riff_item_actual_play_lengths.push(
                                (
                                    riff_set_actual_play_length,
                                    riff_item.clone(),
                                    vec![]
                                )
                            );
                        }
                    }
                    RiffItemType::RiffSequence => {
                        if let Some(riff_sequence) = project.song().riff_sequence(riff_item.item_uuid().to_string()) {
                            println!("state.play_arrangement: riff sequence name={}", riff_sequence.name());
                            let riff_sequence_actual_details = self.get_riff_sequence_play_events(riff_sequence, track_riff_refs_map, track_running_position, &project);
                            riff_arrangement_actual_play_length += riff_sequence_actual_details.0;
                            riff_item_actual_play_lengths.push(
                                (
                                    riff_sequence_actual_details.0,
                                    riff_item.clone(),
                                    riff_sequence_actual_details.1.iter().map(|data| {
                                        (data.0, RiffItem::new_with_uuid_string(data.1.clone(), RiffItemType::RiffSet, data.2.clone()))
                                    }).collect_vec()
                                )
                            );
                        }
                    }
                    RiffItemType::RiffGrid => {
                        if let Some(riff_grid) = project.song().riff_grid(riff_item.item_uuid().to_string()) {
                            println!("state.play_arrangement: riff grid name={}", riff_grid.name());
                            let riff_grid_actual_details = self.get_riff_grid_play_events(riff_grid, track_riff_refs_map, track_running_position);
                            riff_arrangement_actual_play_length += riff_grid_actual_details.0;
                            riff_item_actual_play_lengths.push(
                                (
                                    riff_grid_actual_details.0,
                                    riff_item.clone(),
                                    riff_grid_actual_details.1.iter().map(|data| {
                                        (data.0, RiffItem::new_with_uuid_string(data.1.clone(), RiffItemType::RiffSet, data.2.clone()))
                                    }).collect_vec()
                                )
                            );
                        }
                    }
                }
            }
        }

        (riff_arrangement_actual_play_length, riff_item_actual_play_lengths)
    }



    fn get_riff_sequence_play_events(
        &self,
        riff_sequence: &RiffSequence,
        track_riff_refs_map: &mut HashMap<String, Vec<RiffReference>>,
        track_running_position: &mut HashMap<String, f64>,
        project: &Project,
    ) -> (f64, Vec<(f64, String, String)>) {
        let mut riff_sequence_actual_play_length = 0.0;
        let mut riff_set_actual_play_lengths = vec![];

        for riff_set_reference in riff_sequence.riff_sets().iter() {
            if let Some(riff_set) = project.song().riff_set(riff_set_reference.item_uuid().to_string()) {
                println!("state.play_sequence: riff set name={}", riff_set.name());
                let riff_set_actual_play_length = self.get_riff_set_play_events(riff_set, track_riff_refs_map, track_running_position, &project);
                riff_sequence_actual_play_length += riff_set_actual_play_length;
                riff_set_actual_play_lengths.push((riff_set_actual_play_length, riff_set_reference.uuid(), riff_set_reference.item_uuid().to_string()));
            }
        }

        (riff_sequence_actual_play_length, riff_set_actual_play_lengths)
    }



    pub fn get_riff_grid_play_events(
        &self,
        riff_grid: &RiffGrid,
        track_riff_refs_map: &mut HashMap<String, Vec<RiffReference>>,
        track_running_position: &mut HashMap<String, f64>) -> (f64, Vec<(f64, String, String)>) {
        // get the largest end position - will be the riff grid length
        let mut riff_grid_actual_play_length = 0.0;

        if let Ok(mut project) = self.project.lock() {
            for track_uuid in riff_grid.tracks() {
                if let Some(track) = project.song().track(track_uuid.clone()) {
                    for track_riff_references in riff_grid.track_riff_references(track_uuid.clone()).iter() {
                        for riff_ref in track_riff_references.iter() {
                            if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_ref.linked_to()) {
                                let length = riff_ref.position() + riff.length();
                                if length > riff_grid_actual_play_length {
                                    riff_grid_actual_play_length = length;
                                }
                            }
                        }
                    }
                }
            }

            for (track_uuid, track_riff_references) in track_riff_refs_map.iter_mut() {
                if let Some(running_position) = track_running_position.get(track_uuid) {
                    if let Some(grid_track_riff_references) = riff_grid.track_riff_references(track_uuid.clone()) {
                        for grid_track_riff_reference in grid_track_riff_references.iter() {
                            let mut grid_track_riff_reference_clone = grid_track_riff_reference.clone();
                            grid_track_riff_reference_clone.set_position(grid_track_riff_reference_clone.position() + running_position);
                            track_riff_references.push(grid_track_riff_reference_clone);
                        }
                    }
                    track_running_position.insert(track_uuid.clone(), running_position + riff_grid_actual_play_length);
                }
            }
        }

        (riff_grid_actual_play_length, vec![])
    }



    pub fn get_riff_set_play_events(
        &self,
        riff_set: &RiffSet,
        track_riff_refs_map: &mut HashMap<String, Vec<RiffReference>>,
        track_running_position: &mut HashMap<String, f64>,
        project: &Project,
    ) -> f64 {
        let mut lowest_common_factor_in_beats = 1;
        let mut riff_lengths = vec![];

        // get the track riff_lengths
        for track in project.song().tracks().iter() {
            // get the riff_ref
            if let Some(riff_ref) = riff_set.get_riff_ref_for_track(track.uuid().to_string()) {
                // get the riff
                if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_ref.linked_to()) {
                    riff_lengths.push(riff.length() as i32);
                }
            }
        }

        let (product, unique_riff_lengths) = RiffDAWState::get_length_product(riff_lengths);

        lowest_common_factor_in_beats = RiffDAWState::get_lowest_common_factor(unique_riff_lengths, product);

        for (track_uuid, riff_ref) in riff_set.riff_refs() {
            if let None = track_running_position.get(&track_uuid.clone()) {
                track_running_position.insert(track_uuid.clone(), 0.0);
            }

            if let Some(&mut position) = track_running_position.get_mut(&track_uuid.clone()) {
                // get the riff refs
                if let Some(riff_refs) = track_riff_refs_map.get_mut(track_uuid) {
                    // get the track
                    let track_option = project.song().tracks().iter().find(|track| {
                        track.uuid().to_string() == *track_uuid
                    });

                    if let Some(track) = track_option {
                        // get the riff
                        let riff_option = track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_ref.linked_to());

                        // clone the riff set and set its position
                        if let Some(riff) = riff_option {
                            for repeat in 0..(lowest_common_factor_in_beats / (riff.length() as i32)) {
                                // clone the riff set and set its position
                                let mut riff_reference = riff_ref.clone();
                                riff_reference.set_position(position + riff.length() * repeat as f64);
                                riff_refs.push(riff_reference);
                                track_running_position.insert(track_uuid.clone(), position + riff.length() * repeat as f64 + riff.length());
                            }
                        }
                    }
                }
            }
        }

        lowest_common_factor_in_beats as f64
    }



    pub fn play_riff_arrangement(&mut self, riff_arrangement_uuid: String, play_position_in_beats: f64) -> i32 {
        let mut song_length_in_beats = 400.0;
        let already_playing = self.playing();
        let mut number_of_blocks = 0;

        self.set_playing(true);
        self.set_play_mode(PlayMode::RiffArrangement);

        if let Ok(mut project) = self.project.lock() {
            let bpm = project.song().tempo();
            let time_signature_numerator = project.song().time_signature_numerator();
            let time_signature_denominator = project.song().time_signature_denominator();
            let sample_rate = self.configuration.audio.sample_rate as f64;
            let play_position_in_frames = play_position_in_beats / bpm * 60.0 * sample_rate;
            let block_size = self.configuration.audio.block_size as f64;
            let start_block = (play_position_in_frames / block_size) as i32;

            // get the riff arrangement
            let mut playing_riff_arrangement_summary_data: (f64, Vec<(f64, RiffItem, Vec<(f64, RiffItem)>)>) = (0.0, vec![]);
            if let Some(riff_arrangement) = project.song().riff_arrangement(riff_arrangement_uuid) {
                let mut track_riff_refs_map = HashMap::new();
                let mut track_running_position = HashMap::new();

                // setup
                for track in project.song().tracks().iter() {
                    let track_riff_refs: Vec<RiffReference> = vec![];
                    track_riff_refs_map.insert(track.uuid().to_string(), track_riff_refs);
                    track_running_position.insert(track.uuid().to_string(), 0.0);

                    if !already_playing {
                        self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEventProcessorType(EventProcessorType::BlockEventProcessor));
                    }
                }

                playing_riff_arrangement_summary_data = self.get_riff_arrangement_play_events(riff_arrangement.items(), &mut track_riff_refs_map, &mut track_running_position);

                // find the longest track running position and set the length to be played to that
                if let Some(largest_track_length) = track_running_position.values().into_iter().max_by(|a, b| a.partial_cmp(b).unwrap()) {
                    song_length_in_beats = *largest_track_length;
                }

                // convert and send events
                for track in project.song().tracks().iter() {
                    let midi_channel = if let TrackType::MidiTrack(midi_track) = track {
                        midi_track.midi_device().midi_channel()
                    } else {
                        0
                    };
                    // get the riff refs
                    let riff_refs = match track_riff_refs_map.remove(track.uuid().to_string().as_str()) {
                        None => Vec::<RiffReference>::new(),
                        Some(riff_refs) => riff_refs,
                    };
                    let vst_event_blocks = if let Some(track_automation) = riff_arrangement.automation(&track.uuid().to_string()) {
                        DAWUtils::convert_to_event_blocks(track_automation, track.riffs(), &riff_refs, bpm, block_size, sample_rate, song_length_in_beats, midi_channel, self.automation_discrete(), time_signature_numerator, time_signature_denominator)
                    } else {
                        let automation = Automation::new();
                        DAWUtils::convert_to_event_blocks(&automation, track.riffs(), &riff_refs, bpm, block_size, sample_rate, song_length_in_beats, midi_channel, self.automation_discrete(), time_signature_numerator, time_signature_denominator)
                    };
                    self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEvents(vst_event_blocks, false));
                    self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Loop(false));
                }
            }
            self.playing_riff_arrangement_summary_data = Some(playing_riff_arrangement_summary_data);

            number_of_blocks = (song_length_in_beats / bpm * 60.0 * sample_rate / block_size) as i32;

            // tell each track audio to play
            for track in project.song().tracks() {
                self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Play(start_block));
                self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::LoopExtents(-1, -1));
            }

            // set the start block and the number of blocks in the jack audio layer
            if let Some(sender) = self.audio_layer_sender.as_ref() {
                match sender.send(AudioLayerEvent::AudioLayerInward(AudioLayerInwardEvent::Play(true, number_of_blocks, start_block))) {
                    Ok(_) => (),
                    Err(error) => println!("Problem using audio_layer_sender to send message to jack layer when turning play riff arrangement on: {}", error),
                }
            }
        }

        number_of_blocks
    }



    pub fn calculate_riff_arrangement_length(
        &mut self,
        riff_arrangement_uuid: String,
    ) -> f64 {
        let mut riff_arrangement_length = 0.0;

        if let Ok(project) = self.project().lock() {
            // get the riff arrangement
            if let Some(riff_arrangement) = project.song().riff_arrangement(riff_arrangement_uuid) {
                // process all the items in the arrangement
                for item in riff_arrangement.items().iter() {
                    match item.item_type() {
                        RiffItemType::RiffSet => {
                            // find the riff set and process its events
                            if let Some(riff_set) = project.song().riff_sets().iter().find(|current_riff_set| current_riff_set.uuid() == item.item_uuid()) {
                                let mut riff_lengths = vec![];

                                // get the track riff_lengths
                                for track in project.song().tracks().iter() {
                                    // get the riff_ref
                                    if let Some(riff_ref) = riff_set.get_riff_ref_for_track(track.uuid().to_string()) {
                                        // get the riff
                                        if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_ref.linked_to()) {
                                            riff_lengths.push(riff.length() as i32);
                                        }
                                    }
                                }

                                let (product, unique_riff_lengths) = RiffDAWState::get_length_product(riff_lengths);
                                riff_arrangement_length += RiffDAWState::get_lowest_common_factor(unique_riff_lengths, product) as f64;
                            }
                        }
                        RiffItemType::RiffSequence => {
                            // find the riff sequence and process its events
                            if let Some(riff_sequence) = project.song().riff_sequences().iter().find(|current_riff_sequence| current_riff_sequence.uuid() == item.item_uuid()) {
                                for riff_set_reference in riff_sequence.riff_sets().iter() {
                                    if let Some(riff_set) = project.song().riff_set(riff_set_reference.item_uuid().to_string()) {
                                        let mut riff_lengths = vec![];

                                        // get the track riff_lengths
                                        for track in project.song().tracks().iter() {
                                            // get the riff_ref
                                            if let Some(riff_ref) = riff_set.get_riff_ref_for_track(track.uuid().to_string()) {
                                                // get the riff
                                                if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_ref.linked_to()) {
                                                    riff_lengths.push(riff.length() as i32);
                                                }
                                            }
                                        }

                                        let (product, unique_riff_lengths) = RiffDAWState::get_length_product(riff_lengths);
                                        riff_arrangement_length += RiffDAWState::get_lowest_common_factor(unique_riff_lengths, product) as f64;
                                    }
                                }
                            }
                        }
                        RiffItemType::RiffGrid => {
                            if let Some(riff_grid) = project.song().riff_grid(item.item_uuid().to_string()) {
                                riff_arrangement_length += DAWUtils::get_riff_grid_length(&riff_grid, self);
                            }
                        }
                    }
                }
            }
        }

        riff_arrangement_length
    }



    pub fn riff_set_increment_riff_for_track(&mut self, riff_set_uuid: String, track_uuid: String) -> String {
        println!("state.riff_set_increment_riff_for_track: {}, {}", riff_set_uuid.as_str(), track_uuid.as_str());
        // declare a riff set name string for appending
        let mut new_riff_set_name = "".to_string();

        if let Ok(mut project) = self.project.lock() {
            // get the track
            let riff_uuids: Vec<(String, bool)> = match project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                Some(track) => {
                    track.riffs_mut().iter().map(|riff| (riff.uuid().to_string(), riff.events().iter().any(|event| {
                        if let TrackEvent::Note(note) = event {
                            note.riff_start_note()
                        } else {
                            false
                        }
                    }))).collect_vec()
                },
                None => vec![],
            };
            if let Some(riff_set) = project.song_mut().riff_set_mut(riff_set_uuid.clone()) {
                if !riff_uuids.is_empty() {
                    // get the current riff_ref for the track
                    if let Some(riff_ref) = riff_set.get_riff_ref_for_track_mut(track_uuid.clone()) {
                        let mut count = 0;
                        let current_riff_ref_mode = riff_ref.mode().clone();
                        let mut index_to_get = 0;
                        let mut need_to_index = true;
                        for (riff_uuid, has_start_note) in riff_uuids.iter() {
                            if riff_uuid.to_string() == *riff_ref.linked_to_mut() {
                                if *has_start_note {
                                    if let RiffReferenceMode::Normal = current_riff_ref_mode {
                                        riff_ref.set_mode(RiffReferenceMode::Start);
                                        need_to_index = false;
                                    } else if let RiffReferenceMode::Start = current_riff_ref_mode {
                                        riff_ref.set_mode(RiffReferenceMode::End);
                                        need_to_index = false;
                                    } else {
                                        riff_ref.set_mode(RiffReferenceMode::Normal);
                                        index_to_get = count + 1;
                                    }
                                } else {
                                    riff_ref.set_mode(RiffReferenceMode::Normal);
                                    index_to_get = count + 1;
                                }
                                break;
                            }
                            count += 1;
                        }
                        if need_to_index {
                            if index_to_get >= riff_uuids.len() {
                                index_to_get = 0;
                            }
                            if let Some((riff_uuid, _)) = riff_uuids.get(index_to_get) {
                                riff_ref.set_linked_to(riff_uuid.to_owned());
                            }
                        }
                    } else {
                        // get the first riff uuid
                        if let Some((riff_uuid, _)) = riff_uuids.get(0) {
                            // create a new riff_ref and add to the riff set
                            riff_set.set_riff_ref_for_track(track_uuid, RiffReference::new(riff_uuid.to_owned(), 0.0));
                        }
                    }
                }
            }

            // setup to update the riff set name

            // loop through all the tracks in order
            if let Some(riff_set) = project.song().riff_set(riff_set_uuid.clone()) {
                for (track_number, track_type) in project.song().tracks().iter().enumerate() {
                    // find the riff ref in the riff set for the track and get its linked to value
                    if let Some(linked_to_riff_uuid) = riff_set.riff_refs().iter().find(|(track_uuid, riff_ref)| track_type.uuid().to_string() == track_uuid.to_string()).map(|(track_uuid, riff_ref)| riff_ref.linked_to()) {
                        // find the matching riff in the track and get its name
                        if let Some(riff) = track_type.riffs().iter().find(|riff| riff.uuid().to_string() == linked_to_riff_uuid) {
                            // handle the empty riff
                            let riff_name = if riff.name() == "empty" {
                                "-"
                            } else {
                                riff.name()
                            };
                            // concatenate the name to the riff set name
                            if track_number == 0 {
                                new_riff_set_name = format!("{}: {}", track_number + 1, riff_name);
                            } else {
                                new_riff_set_name = format!("{}, {}: {}", new_riff_set_name.as_str(), track_number + 1, riff_name);
                            }
                        }
                    }
                }
            }

            // update the riff set name
            if let Some(riff_set) = project.song_mut().riff_set_mut(riff_set_uuid.clone()) {
                riff_set.set_name(new_riff_set_name.clone());
            }
        }

        new_riff_set_name
    }

    pub fn riff_set_riff_for_track(&mut self, riff_set_uuid: String, track_uuid: String, riff_uuid: String) {
        println!("state.riff_set_riff_for_track: {}, {}", riff_set_uuid.as_str(), track_uuid.as_str());
        if let Ok(mut project) = self.project.lock() {
            if let Some(riff_set) = project.song_mut().riff_set_mut(riff_set_uuid) {
                // get the current riff_ref for the track
                if let Some(riff_ref) = riff_set.get_riff_ref_for_track_mut(track_uuid.clone()) {
                    riff_ref.set_linked_to(riff_uuid);
                } else {
                    riff_set.set_riff_ref_for_track(track_uuid, RiffReference::new(riff_uuid, 0.0));
                }
            }
        }
    }

    pub fn set_jack_client(&mut self, jack_client: AsyncClient<JackNotificationHandler, Audio>) {
        // self.jack_client.clear();
        // self.jack_client.push(jack_client);
    }

    pub fn jack_client(&self) -> Option<&Client> {
        // if let Some(async_jack_client) = self.jack_client.get(0) {
        //     Some(async_jack_client.as_client())
        // }
        // else {
        None
        // }
    }

    pub fn start_jack(
        &mut self,
        rx_to_audio: crossbeam_channel::Receiver<AudioLayerInwardEvent>,
        jack_midi_sender: crossbeam_channel::Sender<AudioLayerOutwardEvent>,
        jack_midi_sender_ui: crossbeam_channel::Sender<AudioLayerOutwardEvent>,
        jack_time_critical_midi_sender: crossbeam_channel::Sender<AudioLayerTimeCriticalOutwardEvent>,
        coast: Arc<Mutex<AudioMode>>,
        vst_host_time_info: Arc<RwLock<TimeInfo>>,
    ) {
        // let (jack_client, _status) =
        //     Client::new("DAW", ClientOptions::NO_START_SERVER).unwrap();
        // let _ = jack_client.set_buffer_size(self.configuration.audio.block_size as Frames);
        // let audio = Audio::new(
        //     &jack_client,
        //     rx_to_audio,
        //     jack_midi_sender,
        //     jack_midi_sender_ui.clone(),
        //     jack_time_critical_midi_sender,
        //     coast,
        //     vst_host_time_info,
        //     self.configuration.audio.sample_rate,
        //     self.configuration.audio.block_size,
        //     project.song().tempo(),
        // );
        // let notifications = JackNotificationHandler::new(jack_midi_sender_ui);
        // let jack_async_client = jack_client.activate_async(notifications, audio).unwrap();
        //
        // // these should come from configuration and be selected from a menu and dialogue
        // let _ = jack_async_client.as_client().connect_ports_by_name("DAW:out_l", "system:playback_1");
        // let _ = jack_async_client.as_client().connect_ports_by_name("DAW:out_r", "system:playback_2");
        // let _ = jack_async_client.as_client().connect_ports_by_name("a2j:Akai MPD24 [16] (capture): Akai MPD24 MIDI 1", "DAW:midi_control_in");
        // let _ = jack_async_client.as_client().connect_ports_by_name("a2j:nanoPAD2 [20] (capture): nanoPAD2 MIDI 1", "DAW:midi_in");
        //
        // self.set_jack_client(jack_async_client);
    }

    pub fn stop_jack(&mut self) {
        // if self.jack_client.len() == 1 {
        //     let async_client = self.jack_client.remove(0_usize);
        //     if let Err(err) =  async_client.deactivate() {
        //         println!("Problem stopping jack; {}", err);
        //     }
        // }
    }

    pub fn restart_jack(&mut self,
                        rx_to_audio: crossbeam_channel::Receiver<AudioLayerInwardEvent>,
                        jack_midi_sender: crossbeam_channel::Sender<AudioLayerOutwardEvent>,
                        jack_midi_sender_ui: crossbeam_channel::Sender<AudioLayerOutwardEvent>,
                        jack_time_critical_midi_sender: crossbeam_channel::Sender<AudioLayerTimeCriticalOutwardEvent>,
                        coast: Arc<Mutex<AudioMode>>,
                        vst_host_time_info: Arc<RwLock<TimeInfo>>,
    ) {
        // if self.jack_client.len() == 1 {
        //     let async_client = self.jack_client.remove(0_usize);
        //     match async_client.deactivate() {
        //         Ok((_client, _notification_handler, mut process_handler)) => {
        //             let consumers = process_handler.get_all_audio_consumers();
        //             let (jack_client, _status) =
        //                 Client::new("DAW", ClientOptions::NO_START_SERVER).unwrap();
        //             let audio = Audio::new_with_consumers(
        //                 &jack_client,
        //                 rx_to_audio,
        //                 jack_midi_sender,
        //                 jack_midi_sender_ui.clone(),
        //                 jack_time_critical_midi_sender.clone(),
        //                 coast,
        //                 consumers,
        //                 vec![],
        //                 vst_host_time_info,
        //                 self.configuration.audio.sample_rate,
        //                 self.configuration.audio.block_size,
        //                 project.song().tempo(),
        //             );
        //             let notifications = JackNotificationHandler::new(jack_midi_sender_ui);
        //             let jack_async_client = jack_client.activate_async(notifications, audio).unwrap();
        //             for (from_name, to_name) in self.jack_connections.iter() {
        //                 let _ = jack_async_client.as_client().connect_ports_by_name(from_name.as_str(), to_name.as_str());
        //             }
        //             self.set_jack_client(jack_async_client);
        //         }
        //         Err(_) => {
        //             self.start_jack(rx_to_audio, jack_midi_sender, jack_midi_sender_ui, jack_time_critical_midi_sender, coast, vst_host_time_info);
        //         }
        //     }
        // }
        // else {
        //     self.start_jack(rx_to_audio, jack_midi_sender, jack_midi_sender_ui, jack_time_critical_midi_sender, coast, vst_host_time_info);
        // }
    }

    pub fn jack_connection_add(&mut self, from_name: String, to_name: String) {
        // println!("Jack connection added: from={}, to={}", from_name.as_str(), to_name.as_str());
        // if let Some(jack_client) = self.jack_client.get(0) {
        //     let _ = jack_client.as_client().connect_ports_by_name(from_name.as_str(), to_name.as_str());
        // }
        // self.jack_connections.insert(from_name, to_name);
    }

    pub fn jack_midi_connection_add(&mut self, track_uuid: String, to_name: String) {
        // println!("Jack midi connection added: track={}, to={}", track_uuid.as_str(), to_name.as_str());
        // if let Some(jack_client) = self.jack_client.get(0) {
        //     let _ = jack_client.as_client().connect_ports_by_name(format!("DAW:{}", track_uuid.as_str()).as_str(), to_name.as_str());
        // }
        // self.jack_connections.insert(track_uuid, to_name);
    }

    pub fn jack_midi_connection_remove(&mut self, track_uuid: String, to_name: String) {
        // println!("Jack midi connection removed: track={}, to={}", track_uuid.as_str(), to_name.as_str());
        // if let Some(jack_client) = self.jack_client.get(0) {
        //     let _ = jack_client.as_client().disconnect_ports_by_name(format!("DAW:{}", track_uuid.as_str()).as_str(), to_name.as_str());
        // }
        // self.jack_connections.remove(&track_uuid);
    }

    pub fn sample_data(&self) -> &HashMap<String, SampleData> {
        &self.sample_data
    }

    pub fn sample_data_mut(&mut self) -> &mut HashMap<String, SampleData> {
        &mut self.sample_data
    }

    pub fn midi_devices(&mut self) -> Vec<String> {
        if let Some(client) = self.jack_client() {
            client.ports(None, Some("8 bit raw midi"), PortFlags::IS_INPUT).iter().filter(|port_name| !port_name.starts_with("DAW")).map(|port_name| port_name.to_string()).collect()
        } else {
            vec![]
        }
    }

    pub fn export_to_wave_file(&mut self,
                               path: std::path::PathBuf,
    ) {
        let number_of_blocks = match self.current_view() {
            CurrentView::Track => Some(self.play_song() + 1000 /* silence at the end */),
            CurrentView::RiffArrangement => {
                if let Some(selected_riff_arrangement_uuid) = self.selected_riff_arrangement_uuid() {
                    Some(self.play_riff_arrangement(selected_riff_arrangement_uuid.clone(), 0.0) + 1000 /* silence at the end */)
                }
                else {
                    None
                }
            }
            _ => None
        };

        let track_render_audio_consumers = self.track_render_audio_consumers.clone();

        if let Some(number_of_blocks) = number_of_blocks {
            let sample_rate = self.configuration.audio.sample_rate as u32;
            let block_size = self.configuration.audio.block_size as usize;

            let _ = thread::Builder::new().name("Export wave file".into()).spawn(move || {
                match track_render_audio_consumers.lock() {
                    Ok(track_render_audio_consumers) => if let Ok(mut export_wave_file) = std::fs::File::create(path) {
                        let number_of_audio_type_tracks = track_render_audio_consumers.len() as f32;
                        let mut master_left_channel_data: [f32; BLOCK_SIZE_MAX as usize] = [0.0; BLOCK_SIZE_MAX as usize];
                        let mut master_right_channel_data: [f32; BLOCK_SIZE_MAX as usize] = [0.0; BLOCK_SIZE_MAX as usize];
                        let mut sample_data = vec![];
                        let mut audio_blocks = vec![AudioBlock::default()];

                        for _block_number in 0..number_of_blocks {
                            // reset the master block
                            for index in 0..block_size {
                                master_left_channel_data[index] = 0.0;
                                master_right_channel_data[index] = 0.0;
                            }

                            for (_track_uuid, track_audio_consumer_details) in track_render_audio_consumers.iter() {
                                if let Some(blocks_read) = track_audio_consumer_details.consumer().read_blocking(&mut audio_blocks) {
                                    // println!("State.export_to_wave_file: track_uuid={}, channel=left, byes_read={}", track_uuid.as_str(), left_bytes_read);
                                    // copy the track channel data to the master channels
                                    if blocks_read == 1 {
                                        let audio_block = audio_blocks.get(0).unwrap();
                                        for index in 0..block_size {
                                            master_left_channel_data[index] += audio_block.audio_data_left[index] / number_of_audio_type_tracks;
                                        }
                                        for index in 0..block_size {
                                            master_right_channel_data[index] += audio_block.audio_data_right[index] / number_of_audio_type_tracks;
                                        }
                                    }
                                }
                            }

                            // write the master block out
                            for index in 0..block_size {
                                sample_data.push(master_left_channel_data[index]);
                                sample_data.push(master_right_channel_data[index]);
                            }
                        }

                        // write the file
                        let wav_header = wav_io::new_header(sample_rate, 32, true, false);
                        let _ = wav_io::write_to_file(&mut export_wave_file, &wav_header, &sample_data);
                    }
                    Err(_) => {}
                }
            });
        }
    }

    pub fn export_to_midi_file(&self, path: std::path::PathBuf) -> bool {
        let mut success = true;

        if let Ok(mut project) = self.project.lock() {
            if let Some(absolute_path) = path.to_str() {
                let mut midi = MIDI::new();
                let parts_per_quarter_note = midi.get_ppqn();
                let bpm = project.song().tempo();
                let microseconds_per_beat = (1.0 / bpm * 60.0 * 1000000.0) as u32;

                // set the tempo
                midi.insert_event(0, 0, apres::MIDIEvent::SetTempo(microseconds_per_beat));

                let mut track_index: usize = 1;
                for track in project.song().tracks().iter() {
                    let mut single_track_events: Vec<TrackEvent> = vec![];

                    midi.insert_event(track_index, 0, TrackName(track.name().to_string()));

                    if let TrackType::InstrumentTrack(instrument_track) = track {
                        midi.insert_event(track_index, 0, InstrumentName(instrument_track.instrument().name().to_string()));
                    }

                    // map all track events to single midi convertible events - Note becomes NoteOn and NoteOff
                    for riff_ref in track.riff_refs().iter() {
                        if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_ref.linked_to()) {
                            for event in riff.events().iter() {
                                let start_position_in_beats = riff_ref.position() + event.position();

                                match event {
                                    TrackEvent::Note(note) => {
                                        let end_position_in_beats = start_position_in_beats + note.length();

                                        single_track_events.push(TrackEvent::NoteOn(NoteOn::new_with_params(note.note_id(), start_position_in_beats, note.note(), note.velocity())));
                                        single_track_events.push(TrackEvent::NoteOff(NoteOff::new_with_params(note.note_id(), end_position_in_beats, note.note(), 0)));
                                    }
                                    _ => (),
                                }
                            }
                        }
                    }
                    for event in track.automation().events().iter() {
                        match event {
                            TrackEvent::Controller(controller) => {
                                single_track_events.push(TrackEvent::Controller(controller.clone()));
                            }
                            TrackEvent::PitchBend(pitch_bend) => {
                                single_track_events.push(TrackEvent::PitchBend(pitch_bend.clone()));
                            }
                            _ => (),
                        }
                    }

                    // convert the events to midi
                    for event in single_track_events.iter() {
                        let position = (event.position() * (parts_per_quarter_note as f64)) as usize;

                        match event {
                            TrackEvent::NoteOn(note_on) => {
                                midi.insert_event(track_index, position, apres::MIDIEvent::NoteOn(0, note_on.note() as u8, note_on.velocity() as u8));
                            }
                            TrackEvent::NoteOff(note_off) => {
                                midi.insert_event(track_index, position, apres::MIDIEvent::NoteOff(0, note_off.note() as u8, 0));
                            }
                            TrackEvent::Controller(controller) => {
                                match controller.controller() {
                                    7 => { midi.insert_event(track_index, position, apres::MIDIEvent::Volume(0, controller.value() as u8)); }
                                    10 => { midi.insert_event(track_index, position, apres::MIDIEvent::Pan(0, controller.value() as u8)); }
                                    _ => {}
                                }
                            }
                            TrackEvent::PitchBend(pitch_bend) => {
                                midi.insert_event(track_index, position, apres::MIDIEvent::PitchWheelChange(0, pitch_bend.value() as f64));
                            }
                            _ => {}
                        }
                    }

                    track_index += 1;
                }

                midi.save(absolute_path);
                success = true;
            }
            else {
                success = false;
            }
        }

        success
    }

    pub fn export_riffs_to_midi_file(&self, path: std::path::PathBuf) -> bool {
        let mut success = true;

        if let Ok(mut project) = self.project.lock() {
            if let Some(absolute_path) = path.to_str() {
                let bpm = project.song().tempo();
                let mut midi = MIDI::new();
                let parts_per_quarter_note = midi.get_ppqn();
                let microseconds_per_beat = (1.0 / bpm * 60.0 * 1000000.0) as u32;
                let mut track_number: usize = 0;

                midi.insert_event(track_number, 0, apres::MIDIEvent::SetTempo(microseconds_per_beat));
                midi.insert_event(track_number, 1, apres::MIDIEvent::EndOfTrack);
                track_number += 1;

                for track in project.song().tracks() {
                    let mut absolute_position: f64 = 0.0;

                    midi.insert_event(track_number, 0, TrackName(track.name().to_string()));
                    if let TrackType::InstrumentTrack(instrument_track) = track {
                        midi.insert_event(track_number, 0, InstrumentName(instrument_track.instrument().name().to_string()));
                    }

                    for riff in track.riffs() {
                        let mut single_track_events: Vec<TrackEvent> = vec![];
                        let riff_length = (riff.length() * (parts_per_quarter_note as f64)) as usize;

                        // convert notes to note on and note offs
                        for event in riff.events().iter() {
                            let start_position_in_beats = absolute_position + event.position();

                            match event {
                                TrackEvent::Note(note) => {
                                    let end_position_in_beats = start_position_in_beats + note.length();

                                    single_track_events.push(TrackEvent::NoteOn(NoteOn::new_with_params(note.note_id(), start_position_in_beats, note.note(), note.velocity())));
                                    single_track_events.push(TrackEvent::NoteOff(NoteOff::new_with_params(note.note_id(), end_position_in_beats, note.note(), 0)));
                                }
                                _ => (),
                            }
                        }

                        // sort the note ons and offs
                        single_track_events.sort_by(|a, b| a.position().partial_cmp(&b.position()).unwrap());

                        // convert to midi events
                        for event in single_track_events.iter() {
                            let mut position = (event.position() * (parts_per_quarter_note as f64)) as usize;

                            if position >= riff_length {
                                position = riff_length - 1;
                            }

                            match event {
                                TrackEvent::NoteOn(note_on) => {
                                    midi.insert_event(track_number, position, apres::MIDIEvent::NoteOn(0, note_on.note() as u8, note_on.velocity() as u8));
                                }
                                TrackEvent::NoteOff(note_off) => {
                                    midi.insert_event(track_number, position, apres::MIDIEvent::NoteOff(0, note_off.note() as u8, 0));
                                }
                                TrackEvent::Controller(controller) => {
                                    match controller.controller() {
                                        7 => { midi.insert_event(track_number, position, apres::MIDIEvent::Volume(0, controller.value() as u8)); }
                                        10 => { midi.insert_event(track_number, position, apres::MIDIEvent::Pan(0, controller.value() as u8)); }
                                        _ => {}
                                    }
                                }
                                TrackEvent::PitchBend(pitch_bend) => {
                                    midi.insert_event(track_number, position, apres::MIDIEvent::PitchWheelChange(0, pitch_bend.value() as f64));
                                }
                                _ => {}
                            }
                        }

                        // end the midi track
                        midi.insert_event(track_number, riff_length, apres::MIDIEvent::EndOfTrack);

                        // increment the absolute position
                        absolute_position += riff.length();
                    }

                    track_number += 1;
                }

                midi.save(absolute_path);

                success = true;
            }
            else {
                success = false;
            }
        }

        success
    }

    pub fn export_riffs_to_separate_midi_files(&self, path: std::path::PathBuf) -> bool {
        let mut success = false;

        if let Ok(mut project) = self.project.lock() {
            if let Some(dir_path) = path.to_str() {
                let bpm = project.song().tempo();

                let mut track_number: usize = 1;
                for track in project.song().tracks() {
                    for riff in track.riffs() {
                        let mut single_track_events: Vec<TrackEvent> = vec![];
                        let mut absolute_path_buffer = PathBuf::from(dir_path);
                        let mut midi_file_name = if track_number < 10 {
                            format!("0{}", track_number)
                        } else {
                            format!("{}", track_number)
                        };

                        midi_file_name.push('_');
                        midi_file_name.push_str(project.song().name());
                        midi_file_name.push('_');
                        midi_file_name.push_str(track.name());
                        midi_file_name.push('_');
                        midi_file_name.push_str(riff.name());

                        let midi_track_name = midi_file_name.to_string();

                        midi_file_name.push_str(".mid");

                        absolute_path_buffer.push(midi_file_name);

                        let mut midi = MIDI::new();
                        let parts_per_quarter_note = midi.get_ppqn();
                        let microseconds_per_beat = (1.0 / bpm * 60.0 * 1000000.0) as u32;
                        let riff_length = (riff.length() * (parts_per_quarter_note as f64)) as usize;

                        midi.insert_event(0, 0, apres::MIDIEvent::SetTempo(microseconds_per_beat));
                        midi.insert_event(0, 0, TrackName(midi_track_name));
                        if let TrackType::InstrumentTrack(instrument_track) = track {
                            midi.insert_event(0, 0, InstrumentName(instrument_track.instrument().name().to_string()));
                        }

                        for event in riff.events().iter() {
                            let start_position_in_beats = event.position();

                            match event {
                                TrackEvent::Note(note) => {
                                    let end_position_in_beats = start_position_in_beats + note.length();

                                    single_track_events.push(TrackEvent::NoteOn(NoteOn::new_with_params(note.note_id(), start_position_in_beats, note.note(), note.velocity())));
                                    single_track_events.push(TrackEvent::NoteOff(NoteOff::new_with_params(note.note_id(), end_position_in_beats, note.note(), 0)));
                                }
                                _ => (),
                            }
                        }

                        // sort the note ons and offs
                        single_track_events.sort_by(|a, b| a.position().partial_cmp(&b.position()).unwrap());

                        for event in single_track_events.iter() {
                            let mut position = (event.position() * (parts_per_quarter_note as f64)) as usize;

                            if position >= riff_length {
                                position = riff_length - 1;
                            }

                            match event {
                                TrackEvent::NoteOn(note_on) => {
                                    midi.insert_event(0, position, apres::MIDIEvent::NoteOn(0, note_on.note() as u8, note_on.velocity() as u8));
                                }
                                TrackEvent::NoteOff(note_off) => {
                                    midi.insert_event(0, position, apres::MIDIEvent::NoteOff(0, note_off.note() as u8, 0));
                                }
                                TrackEvent::Controller(controller) => {
                                    match controller.controller() {
                                        7 => { midi.insert_event(0, position, apres::MIDIEvent::Volume(0, controller.value() as u8)); }
                                        10 => { midi.insert_event(0, position, apres::MIDIEvent::Pan(0, controller.value() as u8)); }
                                        _ => {}
                                    }
                                }
                                TrackEvent::PitchBend(pitch_bend) => {
                                    midi.insert_event(0, position, apres::MIDIEvent::PitchWheelChange(0, pitch_bend.value() as f64));
                                }
                                _ => {}
                            }
                        }

                        midi.insert_event(0, riff_length, apres::MIDIEvent::EndOfTrack);

                        if let Some(os_path) = absolute_path_buffer.to_str() {
                            midi.save(os_path);
                        }
                    }

                    track_number += 1;
                }
                success = true;
            }
            else {
                success = false;
            }
        }

        success
    }

    pub fn track_render_audio_consumers(&self) -> &Arc<Mutex<HashMap<String, AudioConsumerDetails<AudioBlock>>>> {
        &self.track_render_audio_consumers
    }

    pub fn track_render_audio_consumers_mut(&mut self) -> &mut Arc<Mutex<HashMap<String, AudioConsumerDetails<AudioBlock>>>> {
        &mut self.track_render_audio_consumers
    }

    pub fn play_mode(&self) -> PlayMode {
        self.play_mode.clone()
    }

    pub fn play_mode_mut(&mut self) -> PlayMode {
        self.play_mode.clone()
    }

    pub fn set_play_mode(&mut self, play_mode: PlayMode) {
        self.play_mode = play_mode;
    }

    pub fn playing_riff_set(&self) -> &Option<String> {
        &self.playing_riff_set
    }

    pub fn playing_riff_set_mut(&mut self) -> &Option<String> {
        &self.playing_riff_set
    }

    pub fn set_playing_riff_set(&mut self, playing_riff_set: Option<String>) {
        self.playing_riff_set = playing_riff_set;
    }

    pub fn playing_riff_sequence(&self) -> &Option<String> {
        &self.playing_riff_sequence
    }

    pub fn playing_riff_sequence_mut(&mut self) -> &Option<String> {
        &self.playing_riff_sequence
    }

    pub fn set_playing_riff_sequence(&mut self, playing_riff_sequence: Option<String>) {
        self.playing_riff_sequence = playing_riff_sequence;
    }

    pub fn playing_riff_grid(&self) -> &Option<String> {
        &self.playing_riff_grid
    }

    pub fn playing_riff_grid_mut(&mut self) -> &Option<String> {
        &self.playing_riff_grid
    }

    pub fn set_playing_riff_grid(&mut self, playing_riff_grid: Option<String>) {
        self.playing_riff_grid = playing_riff_grid;
    }

    pub fn playing_riff_arrangement(&self) -> &Option<String> {
        &self.playing_riff_arrangement
    }

    pub fn playing_riff_arrangement_mut(&mut self) -> &Option<String> {
        &self.playing_riff_arrangement
    }

    pub fn set_playing_riff_arrangement(&mut self, playing_riff_arrangement: Option<String>) {
        self.playing_riff_arrangement = playing_riff_arrangement;
    }
    pub fn centre_split_pane_position(&self) -> i32 {
        self.centre_split_pane_position
    }
    pub fn set_centre_split_pane_position(&mut self, centre_split_pane_position: i32) {
        self.centre_split_pane_position = centre_split_pane_position;
    }
    pub fn track_grid_cursor_follow(&self) -> bool {
        self.track_grid_state.track_grid_cursor_follow
    }
    pub fn track_grid_cursor_follow_mut(&mut self) -> bool {
        self.track_grid_state.track_grid_cursor_follow
    }
    pub fn set_track_grid_cursor_follow(&mut self, track_grid_cursor_follow: bool) {
        self.track_grid_state.track_grid_cursor_follow = track_grid_cursor_follow;
    }

    pub fn riff_grid_cursor_follow(&self) -> bool {
        self.riff_grid_cursor_follow
    }

    pub fn riff_grid_cursor_follow_mut(&mut self) -> bool {
        self.riff_grid_cursor_follow
    }

    pub fn set_riff_grid_cursor_follow(&mut self, riff_grid_cursor_follow: bool) {
        self.riff_grid_cursor_follow = riff_grid_cursor_follow;
    }

    pub fn current_view(&self) -> &CurrentView {
        &self.current_view
    }

    pub fn current_view_mut(&mut self) -> &mut CurrentView {
        &mut self.current_view
    }

    pub fn set_current_view(&mut self, current_view: CurrentView) {
        self.current_view = current_view;
    }

    pub fn selected_riff_arrangement_uuid(&self) -> Option<&String> {
        self.riff_arrangement_view_state.selected_riff_arrangement_uuid.as_ref()
    }

    pub fn selected_riff_arrangement_uuid_mut(&mut self) -> &mut Option<String> {
        &mut self.riff_arrangement_view_state.selected_riff_arrangement_uuid
    }

    pub fn set_selected_riff_arrangement_uuid(&mut self, selected_riff_arrangement_uuid: Option<String>) {
        self.riff_arrangement_view_state.selected_riff_arrangement_uuid = selected_riff_arrangement_uuid;
    }
    //
    // pub fn get_automation_for_current_view(&self, project: &mut Project) -> Option<&Automation> {
    //     let mut automation = None;
    //
    //     if let Some(track_uuid) = self.selected_track() {
    //         if let Some(track) = project.song().tracks().iter().find(|track| track.uuid().to_string() == track_uuid) {
    //             match self.current_view() {
    //                 crate::event::CurrentView::RiffArrangement => {
    //                     if let Some(riff_arrangement_uuid) = self.selected_riff_arrangement_uuid() {
    //                         if let Some(riff_arrangement) = project.song().riff_arrangement(riff_arrangement_uuid.clone()) {
    //                             automation = riff_arrangement.automation(&track_uuid);
    //                         } else {
    //                             automation = Some(track.automation());
    //                         }
    //                     } else {
    //                         automation = Some(track.automation());
    //                     }
    //                 },
    //                 _ => automation = Some(track.automation()),
    //             }
    //         }
    //     }
    //
    //     automation
    // }
    //
    // pub fn get_automation_for_current_view_mut(&mut self) -> Option<&mut Automation> {
    //     let current_view = self.current_view().clone();
    //     let selected_riff_arrangement_uuid = if let Some(selected_riff_arrangement_uuid) = self.selected_riff_arrangement_uuid() {
    //         selected_riff_arrangement_uuid.clone()
    //     }
    //     else {
    //         "".to_string()
    //     };
    //     let selected_track_uuid = self.selected_track().clone();
    //
    //     if let Some(track_uuid) = selected_track_uuid {
    //         match current_view {
    //             crate::event::CurrentView::RiffArrangement => {
    //                 if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_riff_arrangement_uuid) {
    //                     riff_arrangement.automation_mut(&track_uuid)
    //                 }
    //                 else {
    //                     None
    //                 }
    //             },
    //             _ => if let Some(track) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
    //                 Some(track.automation_mut())
    //             }
    //             else {
    //                 None
    //             }
    //         }
    //     }
    //     else {
    //         None
    //     }
    // }

    pub fn dirty(&self) -> bool {
        self.dirty
    }

    pub fn dirty_mut(&mut self) -> &mut bool {
        &mut self.dirty
    }

    pub fn set_dirty(&mut self, dirty: bool) {
        self.dirty = dirty;
    }

    pub fn note_expression_id(&self) -> i32 {
        self.note_expression_id
    }

    pub fn note_expression_id_mut(&mut self) -> &mut i32 {
        &mut self.note_expression_id
    }

    pub fn set_note_expression_id(&mut self, note_expression_id: i32) {
        self.note_expression_id = note_expression_id;
    }

    pub fn note_expression_port_index(&self) -> i32 {
        self.note_expression_port_index
    }

    pub fn note_expression_port_index_mut(&mut self) -> &mut i32 {
        &mut self.note_expression_port_index
    }

    pub fn set_note_expression_port_index(&mut self, note_expression_port_index: i32) {
        self.note_expression_port_index = note_expression_port_index;
    }

    pub fn note_expression_channel(&self) -> i32 {
        self.note_expression_channel
    }

    pub fn note_expression_channel_mut(&mut self) -> &mut i32 {
        &mut self.note_expression_channel
    }

    pub fn set_note_expression_channel(&mut self, note_expression_channel: i32) {
        self.note_expression_channel = note_expression_channel;
    }

    pub fn note_expression_key(&self) -> i32 {
        self.note_expression_key
    }

    pub fn note_expression_key_mut(&mut self) -> &mut i32 {
        &mut self.note_expression_key
    }

    pub fn set_note_expression_key(&mut self, note_expression_key: i32) {
        self.note_expression_key = note_expression_key;
    }

    pub fn note_expression_type(&self) -> NoteExpressionType {
        self.note_expression_type
    }

    pub fn note_expression_type_mut(&mut self) -> &mut NoteExpressionType {
        &mut self.note_expression_type
    }

    pub fn set_note_expression_type(&mut self, note_expression_type: NoteExpressionType) {
        self.note_expression_type = note_expression_type;
    }

    pub fn automation_edit_type(&self) -> AutomationEditType {
        self.automation_view_state.automation_edit_type.clone()
    }

    pub fn automation_edit_type_mut(&mut self) -> &mut AutomationEditType {
        &mut self.automation_view_state.automation_edit_type
    }

    pub fn set_automation_edit_type(&mut self, automation_edit_type: AutomationEditType) {
        self.automation_view_state.automation_edit_type = automation_edit_type;
    }

    pub fn selected_automation(&self) -> &[String] {
        self.selected_automation.as_ref()
    }

    pub fn selected_automation_mut(&mut self) -> &mut Vec<String> {
        &mut self.selected_automation
    }

    pub fn automation_event_copy_buffer(&self) -> &[TrackEvent] {
        self.automation_view_state.automation_event_copy_buffer.as_ref()
    }

    pub fn automation_event_copy_buffer_mut(&mut self) -> &mut Vec<TrackEvent> {
        &mut self.automation_view_state.automation_event_copy_buffer
    }

    pub fn selected_riff_events(&self) -> &[String] {
        self.selected_riff_events.as_ref()
    }

    pub fn selected_riff_events_mut(&mut self) -> &mut Vec<String> {
        &mut self.selected_riff_events
    }

    pub fn playing_riff_sequence_summary_data(&self) -> &Option<(f64, Vec<(f64, String, String)>)> {
        &self.playing_riff_sequence_summary_data
    }

    pub fn playing_riff_grid_summary_data(&self) -> &Option<(f64, Vec<(f64, String, String)>)> {
        &self.playing_riff_grid_summary_data
    }

    pub fn playing_riff_arrangement_summary_data(&self) -> &Option<(f64, Vec<(f64, RiffItem, Vec<(f64, RiffItem)>)>)> {
        &self.playing_riff_arrangement_summary_data
    }

    pub fn riff_sequence_riff_set_reference_selected_uuid(&self) -> &Option<(String, String)> {
        &self.riff_sequence_riff_set_reference_selected_uuid
    }

    pub fn riff_set_selected_uuid(&self) -> &Option<String> {
        &self.riff_set_selected_uuid
    }

    pub fn set_riff_sequence_riff_set_reference_selected_uuid(&mut self, riff_sequence_riff_set_reference_selected_uuid: Option<(String, String)>) {
        self.riff_sequence_riff_set_reference_selected_uuid = riff_sequence_riff_set_reference_selected_uuid;
    }

    pub fn set_riff_set_selected_uuid(&mut self, riff_set_selected_uuid: Option<String>) {
        self.riff_set_selected_uuid = riff_set_selected_uuid;
    }

    pub fn close_all_tracks(&mut self) {
        if let Ok(mut track_render_audio_consumers) = self.track_render_audio_consumers_mut().lock() {
            track_render_audio_consumers.clear();
        }
        // need to kill audio threads for tracks in the current file
        if let Ok(mut project) = self.project.lock() {
            let current_track_uuids = project.song_mut().tracks_mut().iter_mut().map(|track| {
                track.uuid().to_string()
            }).collect::<Vec<String>>();

            for current_track_uuid in current_track_uuids.iter() {
                // kill the vst thread
                self.send_to_track_background_processor(current_track_uuid.to_string(), TrackBackgroundProcessorInwardEvent::Kill);

                // remove the consumer from the audio layer
                if let Some(sender) = self.audio_layer_sender.as_ref() {
                    match sender.send(AudioLayerEvent::AudioLayerInward(AudioLayerInwardEvent::RemoveTrack(current_track_uuid.to_string()))) {
                        Ok(_) => (),
                        Err(error) => println!("Problem using audio_layer_sender to send remove track consumer message to jack layer: {}", error),
                    }
                }
            }
        }
    }

    pub fn reset_state(&mut self) {
        self.selected_track = None;
        self.selected_riff_uuid_map.clear();
        self.selected_riff_ref_uuid = None;
        self.current_file_path = None;
        // self.instrument_track_senders.clear();
        // self.instrument_track_receivers.clear();
        self.audio_plugin_parameters.clear();
        self.active_loop = None;
        self.playing_riff_set = None;
        self.playing_riff_sequence = None;
        self.playing_riff_arrangement = None;
        self.playing_riff_sequence_summary_data = None;
        self.playing_riff_arrangement_summary_data = None;
        self.play_position_in_frames = 0;
        self.track_event_copy_buffer.clear();
        self.track_grid_state.track_grid_riff_references_copy_buffer.clear();
        self.selected_effect_plugin_uuid = None;
        self.riff_arrangement_view_state.selected_riff_arrangement_uuid = None;
        self.sample_data.clear();
        if let Ok(mut track_render_audio_consumers) = self.track_render_audio_consumers.lock() {
            track_render_audio_consumers.clear();
        }
        self.dirty = false;
        self.selected_automation.clear();
        self.automation_view_state.automation_event_copy_buffer.clear();
        self.selected_riff_events.clear();
        self.playing_riff_set = None;
        self.playing_riff_sequence = None;
        self.playing_riff_arrangement = None;
    }

    pub fn set_playing_riff_arrangement_summary_data(&mut self, playing_riff_arrangement_summary_data: Option<(f64, Vec<(f64, RiffItem, Vec<(f64, RiffItem)>)>)>) {
        self.playing_riff_arrangement_summary_data = playing_riff_arrangement_summary_data;
    }

    pub fn piano_roll_mpe_note_id(&self) -> &MidiPolyphonicExpressionNoteId {
        &self.piano_roll_state.piano_roll_mpe_note_id
    }

    pub fn set_piano_roll_mpe_note_id(&mut self, piano_roll_mpe_note_id: MidiPolyphonicExpressionNoteId) {
        self.piano_roll_state.piano_roll_mpe_note_id = piano_roll_mpe_note_id;
    }

    pub fn selected_riff_grid_riff_references(&self) -> &Vec<String> {
        &self.selected_riff_grid_riff_references
    }

    pub fn selected_riff_grid_riff_references_mut(&mut self) -> &mut Vec<String> {
        &mut self.selected_riff_grid_riff_references
    }

    pub fn set_selected_riff_grid_riff_references(&mut self, selected_riff_grid_riff_references: Vec<String>) {
        self.selected_riff_grid_riff_references = selected_riff_grid_riff_references;
    }

    pub fn selected_track_grid_riff_references(&self) -> &Vec<String> {
        &self.track_grid_state.selected_track_grid_riff_references
    }

    pub fn selected_track_grid_riff_references_mut(&mut self) -> &mut Vec<String> {
        &mut self.track_grid_state.selected_track_grid_riff_references
    }

    pub fn set_selected_track_grid_riff_references(&mut self, selected_track_grid_riff_references: Vec<String>) {
        self.track_grid_state.selected_track_grid_riff_references = selected_track_grid_riff_references;
    }

    pub fn automation_discrete(&self) -> bool {
        self.automation_view_state.automation_discrete
    }

    pub fn set_automation_discrete(&mut self, automation_discrete: bool) {
        self.automation_view_state.automation_discrete = automation_discrete;
    }

    pub fn history_manager_mut(&mut self) -> &mut Arc<Mutex<HistoryManager>> {
        &mut self.history_manager
    }
}

impl Default for RiffDAWState {
    fn default() -> Self {
        let project: Project = Project::new();
        let configuration = DAWConfiguration::load_config();
        let bookmark_paths = configuration.bookmark_paths.iter().map(|bookmark_path| PathBuf::from(bookmark_path.as_str())).collect();
        let state = Self {
            configuration,
            history_manager: Arc::new(Mutex::new(HistoryManager::new())),
            active_loop: None,
            audio_plugin_parameters: HashMap::new(),
            centre_split_pane_position: 100,
            current_file_path: None,
            current_view: crate::event::CurrentView::Track,
            dirty: false,
            event_edit_view: EventEditView::PianoRoll,
            open_file_dialog_window_id: WindowId::next(),
            open_file_dialogue: HashMap::new(),
            save_file_dialog_window_id: WindowId::next(),
            save_file_dialogue: HashMap::new(),
            track_details_window: HashMap::new(),
            riff_name: None,
            riff_name_window_id: WindowId::next(),
            riff_name_window: HashMap::new(),
            settings_window_id: WindowId::next(),
            settings_window: HashMap::new(),
            sample_data: HashMap::new(),
            height: 200.,
            looping: false,
            main_view: RiffDAWMainView::Track,
            main_window_id: WindowId::next(),
            note_expression_channel: 0,
            note_expression_id: -1,
            note_expression_key: 0,
            note_expression_port_index: 0,
            note_expression_type: NoteExpressionType::Expression,
            parameter_index: None,
            play_mode: PlayMode::Song,
            play_position: 0.0,
            play_position_in_frames: 0,
            playing: false,
            playing_riff_set: None,
            playing_riff_sequence: None,
            playing_riff_grid: None,
            playing_riff_arrangement: None,
            playing_riff_sequence_summary_data: None,
            playing_riff_grid_summary_data: None,
            playing_riff_arrangement_summary_data: None,
            project: Arc::new(Mutex::new(project)),
            riff_view: RiffView::RiffSet,
            recording: false,
            riff_grid_cursor_follow: true,
            riff_grid_riff_references_copy_buffer: vec![],
            riff_sequence_riff_set_reference_selected_uuid: None,
            riff_set_selected_uuid: None,
            running: true,
            selected_automation: vec![],
            selected_effect_plugin_uuid: None,
            selected_loop: usize::MAX,
            selected_riff_events: vec![],
            selected_riff_grid_riff_references: vec![],
            selected_riff_ref_uuid: None,
            selected_riff_uuid_map: HashMap::new(),
            selected_track: None,
            selected_track_type: GeneralTrackType::InstrumentTrack,
            selected_trap_type: 2,
            selected_riff_uuid: "".to_string(),
            time_signature_denominator: 4,
            time_signature_numerator: 4,
            track_details_window_id: WindowId::next(),
            track_event_copy_buffer: vec![],
            track_render_audio_consumers: Arc::new(Mutex::new(HashMap::new())),
            track_type_dropdown_toggle: false,
            track_type_options: vec!["Audio track".to_string(), "Midi track".to_string(), "Instrument track".to_string()],
            track_view_scroll_x: 0.0,
            track_view_scroll_y: 0.0,
            width: 200.,
            riff_seq_selected_riff_set_index: 0,

            audio_layer_sender: None,

            vst24_plugin_loaders: Arc::new(Mutex::new(HashMap::new())),
            clap_plugin_loaders: Arc::new(Mutex::new(HashMap::new())),


            automation_view_state: AutomationViewState {
                automation_view_mode: AutomationViewMode::NoteVelocities,
                automation_edit_type: AutomationEditType::Track,
                controller_type_index: None,
                instrument_parameter_type: None,
                effect_parameter_type: None,
                automation_event_copy_buffer: vec![],
                automation_discrete: true,
                automation_edit_cursor_time_in_beats: 0.0,
                automation_grid_operation_mode: OperationModeType::PointMode,
                draw_mode: DrawMode::Point,
                note_expression_id: MidiPolyphonicExpressionNoteId::ALL,
                note_expression_port_index: NoteExpressionPortIndex::Global,
                note_expression_channel: NoteExpressionChannel::Global,
                note_expression_key: NoteExpressionKey::Global,
                note_expression_type: NoteExpressionType::Volume,
                window_undock: false,
            },

            piano_roll_state: PianoRollState {
                piano_roll_mpe_note_id: MidiPolyphonicExpressionNoteId::ALL,
                piano_roll_edit_cursor_time_in_beats: 0.0,
                piano_roll_edit_cursor_position: 0.0,
                piano_roll_selected_snap: MUSICAL_ITEM_LENGTH_OPTIONS.iter().find_position(|snap| "1/4" == **snap).unwrap().0,
                piano_roll_grid_operation_mode: OperationModeType::PointMode,
                piano_roll_mpe_voice_picklist_options: vec![
                    "All".to_string(),
                    "0".to_string(),
                    "1".to_string(),
                    "2".to_string(),
                    "3".to_string(),
                    "4".to_string(),
                    "5".to_string(),
                    "6".to_string(),
                    "7".to_string(),
                    "8".to_string(),
                    "9".to_string(),
                    "10".to_string()
                ],
                piano_roll_quantise_end: false,
                piano_roll_quantise_quantise_strength: 100,
                piano_roll_quantise_start: true,
                piano_roll_scroll_y: 0.0,
                piano_roll_subdivision_options: vec!["Normal".to_string(), "Triplet".to_string()],
                piano_roll_triplet_options: vec!["1/4 triplet".to_string(), "1/8 triplet".to_string(), "1/16 triplet".to_string()],
                selected_piano_roll_note_adj: 10,
                selected_piano_roll_note_length_option: 15,
                selected_piano_roll_subdivision: 0,
                selected_piano_roll_triplet: 1,
                window_undock: false,
            },

            track_detail_view_state: TrackDetailViewState {
                add_riff_name: "".to_string(),
                add_riff_length: 4.0,
                add_riff_length_text: MUSICAL_ITEM_LENGTH_OPTIONS.get(10).unwrap().to_string(),
            },

            track_grid_state: TrackGridState {
                track_grid_riff_references_copy_buffer: vec![],
                track_grid_cursor_follow: true,
                selected_track_grid_riff_references: vec![],
                track_operation_mode_select: true,
                track_grid_edit_cursor_time_in_beats: 0.0,
                track_grid_operation_mode: OperationModeType::PointMode,
                track_grid_edit_cursor_position: 0.0,
                track_grid_selected_snap: MUSICAL_ITEM_LENGTH_OPTIONS.iter().find_position(|snap| "1" == **snap).unwrap().0,
                show_automation: false,
                show_notes: false,
                show_note_velocities: false,
                show_pan: false,
            },

            riff_set_view_state: RiffSetViewState {
                add_riff_set_name: "".to_string(),
            },

            riff_sequence_view_state: RiffSequenceViewState {
                add_riff_sequence_name: "".to_string(),
                add_to_seq_riff_set_index: 0,
                riff_seq_to_select_index: 0,
                selected_riff_sequence_uuid: None,
            },

            riff_grid_view_state: RiffGridViewState {
                add_riff_grid_name: "".to_string(),
                riff_grid_to_select_index: 0,
                selected_riff_grid_uuid: None,
            },

            riff_arrangement_view_state: RiffArrangementViewState {
                add_riff_arrangement_name: "".to_string(),
                riff_arrangement_riff_item_selected_uuid: None,
                add_to_arr_riff_set_index: 0,
                add_to_arr_riff_seq_index: 0,
                add_to_arr_riff_grid_index: 0,
                riff_arr_to_select_index: 0,
                selected_riff_arrangement_uuid: None,
            },

            recorded_playing_notes: HashMap::new(),

            file_dialog: FileDialog {
                dialog_window_id: None,
                dialog: DialogState::new("fdaw".to_string()),
                bookmarks: Bookmarks {
                    items: bookmark_paths,
                    selected: None,
                },
            },

            axis_values: HashMap::new(),
        };

        state
    }
}

impl AppState for RiffDAWState {
    fn keep_running(&self) -> bool {
        self.running
    }
}
