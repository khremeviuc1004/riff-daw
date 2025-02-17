use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use std::path::PathBuf;

use jack::{MidiOut, Port};
use rb::{Consumer, Producer, SpscRb};
use simple_clap_host_helper_lib::plugin::library::PluginLibrary;
use uuid::Uuid;
use vst::{event::MidiEvent, host::PluginLoader};

use crate::{MidiConsumerDetails, SampleData, domain::Riff};
use crate::domain::{AudioBlock, AudioConsumerDetails, AudioRouting, NoteExpressionType, PluginParameter, RiffItemType, RiffReference, TrackEvent, TrackEventRouting, VstHost};
use crate::state::{MidiPolyphonicExpressionNoteId};

#[derive(Clone)]
pub enum CurrentView {
    Track,
    RiffSet,
    RiffSequence,
    RiffGrid,
    RiffArrangement,
}

#[derive(Clone)]
pub enum NotificationType {
    Info,
    Warning,
    Question,
    Error,
    Other,
}

#[derive(Clone)]
pub enum ShowType {
    Velocity,
    NoteExpression,
    Controller,
    PitchBend,
    InstrumentParameter,
    EffectParameter,
}

#[derive(Clone)]
pub enum AutomationEditType {
    Track,
    Riff,
}

#[derive(Clone)]
pub enum LoopChangeType {
    LoopOn,
    LoopOff,
    ActiveLoopChanged(Option<Uuid>),
    LoopLimitLeftChanged(f64),
    LoopLimitRightChanged(f64),
    Added(String), // loop name
    Deleted,
    NameChanged(String), // new loop name
}

#[derive(Clone)]
pub enum ProjectEvent {
    Closed,
    Opened,
    Changed,
    SongMode,
    AuditionMode,
    RiffSetMode,
    Loading,
    BPM,
}

#[derive(Debug, Clone)]
pub enum OperationModeType {
    Add,
    Delete,
    Change,
    PointMode,
    LoopPointMode,
    DeleteSelected,
    CopySelected,
    PasteSelected,
    SelectAll,
    DeselectAll,
    Undo,
    Redo,
    SelectStartNote,
    SelectRiffReferenceMode,
}

#[derive(Clone)]
pub enum TranslateDirection {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone)]
pub enum TranslationEntityType {
    ActiveSense,
    AfterTouch,
    ProgramChange,
    Note,
    NoteOn,
    NoteOff,
    Controller,
    PitchBend,
    KeyPressure,
    AudioPluginParameter,
    Sample,
    RiffRef,
    Any,
}

#[derive(Clone)]
pub enum GeneralTrackType {
    InstrumentTrack,
    AudioTrack,
    MidiTrack,
    MasterTrack,
}

#[derive(Clone)]
pub enum NoteExpressionData {
    Type(NoteExpressionType),
    NoteId(i32), 
    PortIndex(i32), 
    Channel(i32), 
    Key(i32),
}

#[derive(Clone)]
pub enum AutomationChangeData {
    ParameterType(i32),
    NoteExpression(NoteExpressionData), 
}

#[derive(Clone)]
pub enum RiffGridChangeType {
    RiffReferenceAdd{ track_index: i32, position: f64 },
    RiffReferenceDelete{ track_index: i32, position: f64 },
    RiffReferenceCutSelected{ x1: f64, y1: i32, x2: f64, y2: i32 },
    RiffReferenceCopySelected{ x1: f64, y1: i32, x2: f64, y2: i32 },
    RiffReferencePaste,
    RiffReferenceChange{ orginal_riff_copy: Riff, changed_riff: Riff },
}

#[derive(Clone)]
pub enum TrackChangeType {
    Added(GeneralTrackType),
    Deleted,
    Modified,
    Selected,

    SoloOn,
    SoloOff,
    Mute,
    Unmute,

    Record(bool),

    Volume(Option<f64>, f32), // position, volume range 0.0 to 1.0
    Pan(Option<f64>, f32),    // position, pan range -1.0 to 1.0

    MidiOutputDeviceChanged(String), // jack port name
    MidiInputDeviceChanged,
    MidiOutputChannelChanged(i32), // midi channel 0-15
    MidiInputChannelChanged,

    InstrumentChanged(String),
    ShowInstrument,

    TrackNameChanged(String), // new track name
    CopyTrack,

    EffectAdded(Uuid, String, String),    // uuid, name, path
    EffectDeleted(String),                // vst effect plugin uuid
    EffectSelected(String),               // vst effect plugin uuid
    EffectToggleWindowVisibility(String), // effect uuid

    TrackColourChanged(f64, f64, f64, f64), // red, green, blue, alpha
    RiffColourChanged(String, f64, f64, f64, f64), // uuid, red, green, blue, alpha

    RiffAdd(Uuid, String, f64),     // uuid, name, length
    RiffCopy(String, Uuid, String), // uuid to copy, uuid, name
    RiffDelete(String),             // riff uuid
    RiffNameChange(String, String), // riff uuid, new riff name
    RiffLengthChange(String, f64),  // riff uuid, new riff length
    RiffSelect(String),             // riff uuid

    RiffReferenceAdd(i32, f64),                    // track index, position
    RiffReferenceDelete(i32, f64),                 // track index, position
    RiffReferenceCutSelected(f64, i32, f64, i32),  // window - x1, y1, x2, y2
    RiffReferenceCopySelected(f64, i32, f64, i32), // window - x1, y1, x2, y2
    RiffReferencePaste,

    RiffAddNote(i32, f64, f64),    // note_number, position, duration
    RiffDeleteNote(i32, f64),      // note_number, position
    RiffAddSample(String, f64),    // sample_reference_uuid, position
    RiffDeleteSample(String, f64), // sample_reference_uuid, position
    RiffTranslateSelected(
        TranslationEntityType,
        TranslateDirection,
        f64,
        i32,
        f64,
        i32,
    ), // window - x1, y1, x2, y2
    RiffEventChange(
        TrackEvent,
        TrackEvent,
    ), // original event copy, changed event
    RiffReferenceChange(
        Riff,
        Riff,
    ), // original riff copy, changed riff
    RiffQuantiseSelected(f64, i32, f64, i32), // window - x1, y1, x2, y2
    RiffCutSelected(f64, i32, f64, i32), // window - x1, y1, x2, y2
    RiffCopySelected(f64, i32, f64, i32), // window - x1, y1, x2, y2
    RiffPasteSelected,
    RiffChangeLengthOfSelected(bool, f64, i32, f64, i32), // true to lengthen, false to shorten, window - x1, y1, x2, y2
    RiffEventsSelected(f64, i32, f64, i32, bool),
    RiffSetStartNote(i32, f64),
    RiffReferencePlayMode(i32, f64),

    AutomationAdd(f64, i32),
    AutomationDelete(f64),
    AutomationTranslateSelected(
        TranslationEntityType,
        TranslateDirection,
        f64,
        i32,
        f64,
        i32,
    ), // window - x1, y1, x2, y2
    AutomationQuantiseSelected,
    AutomationCut,
    AutomationCopy,
    AutomationPaste,
    AutomationTypeChange(AutomationChangeData),
    AutomationSelected(f64, i32, f64, i32, bool),

    RouteMidiTo(TrackEventRouting),
    RemoveMidiRouting(String), // route_uuid
    UpdateMidiRouting(String, i32, i32, i32), // route_uuid, midi channel, start note, end note

    RouteAudioTo(AudioRouting),
    RemoveAudioRouting(String), // route_uuid

    TrackMoveToPosition(usize),            // move to position

    TrackDetails(bool), // show: true/false
    UpdateTrackDetails,
}

#[derive(Clone)]
pub enum TransportChangeType {
    GotoStart,
    Rewind,
    Stop,
    Record,
    Play,
    FastForward,
    GotoEnd,
    Pause,
    SampleRate,
}

pub enum AudioLayerInwardEvent {
    NewAudioConsumer(AudioConsumerDetails<AudioBlock>),
    NewMidiConsumer(MidiConsumerDetails<(u32, u8, u8, u8, bool)>), // frame, midi byte 1, midi byte 2, midi byte 3
    Play(bool, i32, i32), // play - true/false, number of blocks, start at block
    ExtentsChange(i32),
    Stop,
    Tempo(f64),
    SampleRate(f64),
    BlockSize(f64),
    Volume(f32), // volume
    Pan(f32),    // pan
    Shutdown,
    RemoveTrack(String),                           // track uuid
    NewMidiOutPortForTrack(String, Port<MidiOut>), // track uuid, jack midi port

    PreviewSample(String), // absolute path sample file name
}

pub enum EventProcessorType {
    RiffBufferEventProcessor,
    BlockEventProcessor,
}

#[derive(Clone)]
pub enum DAWEvents {
    NewFile,
    Notification(NotificationType, String),
    OpenFile(PathBuf),
    SaveAs(PathBuf),
    Save,
    ImportMidiFile(PathBuf),
    ExportMidiFile(PathBuf),
    ExportRiffsToMidiFile(PathBuf),
    ExportRiffsToSeparateMidiFiles(PathBuf),
    ExportWaveFile(PathBuf),
    UpdateUI,
    UpdateState,
    HideProgressDialogue,

    Undo,
    Redo,

    AutomationViewShowTypeChange(ShowType),
    AutomationEditTypeChange(AutomationEditType),

    LoopChange(LoopChangeType, Uuid), // type, loop number
    ProjectChange(ProjectEvent),
    TrackChange(TrackChangeType, Option<String>), // change type, track uuid
    TrackEffectParameterChange(i32, i32),         // effect number, effect param number
    TrackInstrumentParameterChange(i32),          // instr param num
    TrackSelectedPatternChange(i32, i32),         // track num, pattern index
    TranslateHorizontalChange(i32),               // +/- value
    TranslateVerticalChange(i32),                 // +/- value
    TransportChange(TransportChangeType, f64, f64), // type, value1, value2
    ViewAutomationChange(bool),                   // show automation events
    ViewNoteChange(bool),                         // show note events
    ViewPanChange(bool),                          // show pan events
    ViewVolumeChange(bool),                       // show volume change events

    TrackGridOperationModeChange(OperationModeType),
    PianoRollOperationModeChange(OperationModeType),
    ControllerOperationModeChange(OperationModeType),

    TransportGotoStart,
    TransportMoveBack,
    TransportStop,
    TransportPlay,
    TransportRecordOn,
    TransportRecordOff,
    TransportPause,
    TransportMoveForward,
    TransportGotoEnd,

    PlayNoteImmediate(i32),
    StopNoteImmediate(i32),

    PlayPositionInBeats(f64),

    TempoChange(f64),

    Panic,
    TrimAllNoteDurations,

    RiffSetPlay(String), // riff set uuid
    RiffSetTrackIncrementRiff(
        String, /* riff set uuid */
        String, /* track uuid */
    ),
    RiffSetTrackSetRiff(
        String, /* riff set uuid */
        String, /* track uuid */
        String, /* riff uuid */
    ),
    RiffSetAdd(Uuid, String),                                     // new riff set uuid, name
    RiffSetDelete(String),                                // riff set uuid
    RiffSetCopy(String, Uuid),                            // riff set uuid, new copy riff set uuid
    RiffSetCopySelectedToTrackViewCursorPosition(String), // riff set uuid
    RiffSetNameChange(String, String),                    // riff set uuid, new name
    RiffSetMoveToPosition(String, usize),                   // riff set uuid, position
    RiffSetSelect(String, bool),                                  // riff set uuid, bool selected

    RiffSequencePlay(String),                     // riff sequence uuid
    RiffSequenceAdd(Uuid),                        // new riff sequence uuid
    RiffSequenceCopy(String),                        // riff sequence uuid to copy
    RiffSequenceDelete(String),                   // riff sequence uuid
    RiffSequenceNameChange(String, String),       // riff sequence uuid, new name
    RiffSequenceSelected(String),                 // riff sequence uuid
    RiffSequenceRiffSetAdd(String, String, Uuid), // riff sequence uuid, riff set uuid, riff set reference uuid
    RiffSequenceRiffSetDelete(String, String),    // riff sequence uuid, riff set uuid
    RiffSequenceRiffSetMoveToPosition(String, String, usize), // riff sequence uuid, riff set uuid, position
    RiffSequenceRiffSetMoveLeft(String, String),  // riff sequence uuid, riff set uuid
    RiffSequenceRiffSetMoveRight(String, String), // riff sequence uuid, riff set uuid
    RiffSequenceCopySelectedToTrackViewCursorPosition(String), // riff sequence uuid
    RiffSequenceRiffSetSelect(String, String, bool), // riff sequence uuid, riff set reference uuid, bool selected

    RiffGridPlay(String),                     // riff grid uuid
    RiffGridAdd(String, String),               // riff grid uuid
    RiffGridCopy(String),               // riff grid uuid to copy
    RiffGridDelete(String),                   // riff grid uuid
    RiffGridSelected(String),                                  // riff grid uuid
    RiffGridChange(RiffGridChangeType, Option<String>), // change type, track uuid
    RiffGridNameChange(String),       // new name
    RiffGridCopySelectedToTrackViewCursorPosition(String), // riff grid uuid

    RiffArrangementPlay(String),               // riff arrangement uuid
    RiffArrangementAdd(Uuid),                  // new riff arrangement uuid
    RiffArrangementDelete(String),             // riff arrangement uuid
    RiffArrangementSelected(String),                 // riff arrangement uuid
    RiffArrangementCopy(String),               // riff arrangement uuid to copy
    RiffArrangementNameChange(String, String), // riff arrangement uuid, new name
    RiffArrangementMoveRiffItemToPosition(String, String, usize), // riff arrangement uuid, riff item compound uuid, position
    RiffArrangementRiffItemAdd(String, String, RiffItemType), // riff arrangement uuid, riff seq/set uuid, riff item tpe - riff set or riff sequence
    RiffArrangementRiffItemDelete(String, String),      // riff arrangement uuid, item uuid
    RiffArrangementCopySelectedToTrackViewCursorPosition(String), // riff arrangement uuid
    RiffArrangementRiffItemSelect(String, String, bool), // riff_arrangement uuid, riff item uuid (riff set reference uuid), bool selected

    MasterChannelChange(MasterChannelChangeType), // channel change type

    PianoRollSetTrackName(String),
    PianoRollSetRiffName(String),
    PianoRollMPENoteIdChange(MidiPolyphonicExpressionNoteId),

    SampleRollSetTrackName(String),
    SampleRollSetRiffName(String),

    PreviewSample(String), // absolute path sample file name

    SampleAdd(String),    // absolute path sample file name
    SampleDelete(String), // uuid

    RunLuaScript(String), // Lua script text

    TrackGridVerticalScaleChanged(f64), // scale
    RiffGridVerticalScaleChanged(f64), // scale

    Shutdown,

    RepaintAutomationView,
    RepaintTrackGridView,
    RepaintPianoRollView,
    RepaintSampleRollDrawingArea,
    RepaintRiffArrangementBox,
    RepaintRiffSetsBox,
    RepaintRiffSequencesBox,
}

pub enum TrackBackgroundProcessorInwardEvent {
    SetSample(SampleData),
    SetEvents(
        (
            Vec<Vec<TrackEvent>>,
            Vec<Vec<PluginParameter>>,
        ),
        bool,
    ), // instrument plugin events, instrument and effect plugin parameters, transition_to
    SetEventProcessorType(EventProcessorType),
    GotoStart,
    MoveBack,
    Play(i32), // start at block number
    Stop,
    Loop(bool),
    LoopExtents(i32, i32),
    Pause,
    MoveForward,
    GotoEnd,
    Mute,
    Unmute,
    Kill,

    AddEffect(
        Arc<Mutex<HashMap<String, PluginLoader<VstHost>>>>,
        Arc<Mutex<HashMap<String, PluginLibrary>>>,
        Uuid,
        String,
    ), // vst24 plugin loaders map, clap plugin loaders map, window id, effect uuid, absolute path to shared library (details - includes shell plugin id if exists)
    DeleteEffect(String),           // effect uuid,
    SetEffectWindowId(String, u32), // effect uuid, window id

    ChangeInstrument(
        Arc<Mutex<HashMap<String, PluginLoader<VstHost>>>>,
        Arc<Mutex<HashMap<String, PluginLibrary>>>,
        Uuid,
        String,
    ), // vst24 plugin loaders map, clap plugin loaders map, window id, instrument uuid, absolute path to shared library (details - includes shell plugin id if exists)
    SetInstrumentWindowId(u32),
    SetInstrumentParameter(i32, f32), // parameter index, value

    SetPresetData(String, Vec<String>), // instrument preset data, vector of effect preset data
    RequestPresetData,

    PlayNoteImmediate(i32, i32), // note number, midi channel number
    StopNoteImmediate(i32, i32), // note number, midi channel number

    PlayControllerImmediate(i32, i32, i32), // controller number, controller value, midi channel number

    PlayPitchBendImmediate(i32, i32, i32), // lsb (7bits), msb (7bits), midi channel number

    RequestInstrumentParameters,
    RequestEffectParameters(String), // uuid

    SetBlockPosition(i32), // block position

    Volume(f32), // volume
    Pan(f32),    // pan
    
    Tempo(f64),

    AddTrackEventSendRouting(TrackEventRouting, SpscRb<TrackEvent>, Producer<TrackEvent>), // track event routing, ring buffer, producer
    RemoveTrackEventSendRouting(String), // route uuid
    UpdateTrackEventSendRouting(String, TrackEventRouting), // route_uuid, route
    AddTrackEventReceiveRouting(TrackEventRouting, Consumer<TrackEvent>), // track event routing, consumer
    RemoveTrackEventReceiveRouting(String), // route uuid
    UpdateTrackEventReceiveRouting(String, TrackEventRouting), // route_uuid, route

    AddAudioSendRouting(AudioRouting, (SpscRb<f32>, SpscRb<f32>), (Producer<f32>, Producer<f32>)), // audio routing, (ring buffer left, ring buffer right), (left producer, right producer)
    RemoveAudioSendRouting(String), // route uuid
    AddAudioReceiveRouting(AudioRouting, (Consumer<f32>, Consumer<f32>)), // audio routing, (consumer left, consumer right)
    RemoveAudioReceiveRouting(String), // route uuid
}

pub enum TrackBackgroundProcessorOutwardEvent {
    InstrumentParameters(Vec<(i32, String, Uuid, String, String, f32, String)>), // param index, track uuid, instrument uuid, param name, param label, param value, param text
    InstrumentName(String),
    EffectParameters(Vec<(String, i32, String, String, f32, String)>), // vector of plugin uuid, param index, param name, param label, param value, param text
    GetPresetData(String, Vec<String>),
    InstrumentPluginWindowSize(String, i32, i32), // track uuid, width, height
    EffectPluginWindowSize(String, String, i32, i32), // track uuid, plugin uuid, width, height
    Automation(String, String, bool, i32, f32), // track uuid, vst plugin uuid, is instrument, param index, param value - 0.0 to 1.0
    TrackRenderAudioConsumer(AudioConsumerDetails<AudioBlock>),
    ChannelLevels(String, f32, f32), // track_uuid, left channel level, right channel_level
}

pub enum AudioLayerOutwardEvent {
    MidiControlEvent(MidiEvent),
    GeneralMMCEvent([u8; 6]),
    PlayPositionInFrames(u32),
    JackRestartRequired,
    JackConnect(String, String), // from, to
    MasterChannelLevels(f32, f32),
}

pub enum AudioLayerTimeCriticalOutwardEvent {
    MidiEvent(MidiEvent),
    TrackVolumePanLevel(MidiEvent),
}

pub enum AudioPluginHostOutwardEvent {
    Automation(String, String, bool, i32, f32), // track uuid, audio plugin uuid, is instrument, param index, param value - 0.0 to 1.0
    SizeWindow(String, String, bool, i32, i32), // track uuid, audio plugin uuid, is instrument, width, height
}

#[derive(Clone)]
pub enum MasterChannelChangeType {
    VolumeChange(f64), // volume
    PanChange(f64),    // pan
}
