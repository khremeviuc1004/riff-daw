extern crate factor;

use std::{collections::HashMap, sync::{Arc, mpsc::{channel, Receiver, Sender}, Mutex}, time::Duration};
use std::collections::HashSet;
use std::path::PathBuf;
use std::thread;

use apres::MIDI;
use apres::MIDIEvent::{InstrumentName, TrackName};
use factor::factor_include::factor_include;
use indexmap::IndexMap;
use itertools::Itertools;
use jack::{AsyncClient, Client, ClientOptions, PortFlags};
use log::*;
use rb::{RB, RbConsumer, SpscRb};
use simple_clap_host_helper_lib::plugin::library::PluginLibrary;
use uuid::Uuid;
use vst::api::TimeInfo;
use vst::host::PluginLoader;

use crate::{Audio, AudioLayerOutwardEvent, DAWUtils, domain::*, event::{AudioLayerInwardEvent, CurrentView, DAWEvents, TrackBackgroundProcessorInwardEvent, TrackBackgroundProcessorOutwardEvent, AutomationEditType}, GeneralTrackType, JackNotificationHandler};
use crate::event::EventProcessorType;
use crate::TrackType;

extern {
    fn gdk_x11_window_get_xid(window: gdk::Window) -> u32;
}

pub enum AutomationViewMode {
    NoteVelocities,
    Controllers,
    Instrument,
    Effect,
    NoteExpression,
}

pub struct DAWState {
    pub configuration: DAWConfiguration,
    project: Project,
    selected_track: Option<String>,
    selected_riff_uuid_map: HashMap<String, String>,
    selected_riff_ref_uuid: Option<String>,
    current_file_path: Option<String>,
    sender: crossbeam_channel::Sender<DAWEvents>,
    pub instrument_track_senders: HashMap<String, Sender<TrackBackgroundProcessorInwardEvent>>,
    pub instrument_track_receivers: HashMap<String, Receiver<TrackBackgroundProcessorOutwardEvent>>,
    pub audio_plugin_parameters: HashMap<String, HashMap<String, Vec<PluginParameterDetail>>>,
    active_loop: Option<Uuid>,
    looping: bool,
    recording: bool,
    playing: bool,
    play_mode: PlayMode,
    playing_riff_set: Option<String>,
    playing_riff_sequence: Option<String>,
    playing_riff_arrangement: Option<String>,
    // (f64 - sequence length, Vec<(f64 - riff set ref length, String - riff set ref id, String riff set id)>)
    playing_riff_sequence_summary_data: Option<(f64, Vec<(f64, String, String)>)>,
    // (f64 - arrangement length, Vec<(f64 - set/sequence length, RiffItem, Vec<(f64, RiffItem)>)>)
    playing_riff_arrangement_summary_data: Option<(f64, Vec<(f64, RiffItem, Vec<(f64, RiffItem)>)>)>,
    play_position_in_frames: u32,
    track_event_copy_buffer: Vec<TrackEvent>,
    riff_references_copy_buffer: Vec<RiffReference>,
    automation_view_mode: AutomationViewMode,
    automation_edit_type: AutomationEditType,
    automation_type: Option<i32>,
    note_expression_id: i32,
    note_expression_port_index: i32,
    note_expression_channel: i32,
    note_expression_key: i32,
    note_expression_type: NoteExpressionType,
    parameter_index: Option<i32>,
    selected_effect_plugin_uuid: Option<String>,
    selected_riff_arrangement_uuid: Option<String>,
    jack_client: Vec<AsyncClient<JackNotificationHandler, Audio>>,
    jack_connections: HashMap<String, String>,
    sample_data: HashMap<String, SampleData>,
    track_render_audio_consumers: Arc<Mutex<HashMap<String, AudioConsumerDetails<AudioBlock>>>>,
    centre_split_pane_position: i32,
    vst_instrument_plugins: IndexMap<String, String>,
    vst_effect_plugins: IndexMap<String, String>,
    track_grid_cursor_follow: bool,
    pub current_view: CurrentView,
    pub dirty: bool,
    selected_automation: Vec<String>,
    automation_event_copy_buffer: Vec<TrackEvent>,
    selected_riff_events: Vec<String>,
    riff_set_selected_uuid: Option<String>,
    riff_sequence_riff_set_reference_selected_uuid: Option<(String, String)>,
    riff_arrangement_riff_item_selected_uuid: Option<(String, String)>,
}

impl DAWState {
    pub fn new(sender: crossbeam_channel::Sender<DAWEvents>) -> Self {
        Self {
            configuration: DAWConfiguration::load_config(),
            project: Project::new(),
            current_file_path: None,
            sender,
            selected_track: None,
            selected_riff_uuid_map: HashMap::new(),
            selected_riff_ref_uuid: None,
            instrument_track_senders: HashMap::new(),
            instrument_track_receivers: HashMap::new(),
            audio_plugin_parameters: HashMap::new(),
            active_loop: None,
            looping: false,
            recording: false,
            playing: false,
            play_mode: PlayMode::Song,
            playing_riff_set: None,
            playing_riff_sequence: None,
            playing_riff_arrangement: None,
            playing_riff_sequence_summary_data: None,
            playing_riff_arrangement_summary_data: None,
            play_position_in_frames: 0,
            track_event_copy_buffer: vec![],
            riff_references_copy_buffer: vec![],
            automation_view_mode: AutomationViewMode::NoteVelocities,
            automation_edit_type: AutomationEditType::Track,
            automation_type: None,
            note_expression_id: -1,
            note_expression_port_index: -1,
            note_expression_channel: -1,
            note_expression_key: -1,
            note_expression_type: NoteExpressionType::Volume,
            parameter_index: None,
            selected_effect_plugin_uuid: None,
            jack_client: vec![],
            jack_connections: HashMap::new(),
            sample_data: HashMap::new(),
            track_render_audio_consumers: Arc::new(Mutex::new(HashMap::new())),
            centre_split_pane_position: 600,
            vst_instrument_plugins: IndexMap::new(),
            vst_effect_plugins: IndexMap::new(),
            track_grid_cursor_follow: true,
            current_view: CurrentView::Track,
            selected_riff_arrangement_uuid: None,
            dirty: false,
            selected_automation: Vec::new(),
            automation_event_copy_buffer: vec![],
            selected_riff_events: Vec::new(),
            riff_set_selected_uuid: None,
            riff_sequence_riff_set_reference_selected_uuid: None,
            riff_arrangement_riff_item_selected_uuid: None,
        }
    }

    pub fn load_from_file(&mut self,
                            vst24_plugin_loaders: Arc<Mutex<HashMap<String, PluginLoader<VstHost>>>>,
                            clap_plugin_loaders: Arc<Mutex<HashMap<String, PluginLibrary>>>,
                            path: &str,
                            tx_audio: crossbeam_channel::Sender<AudioLayerInwardEvent>,
                            track_audio_coast: Arc<Mutex<TrackBackgroundProcessorMode>>,
                            vst_host_time_info: Arc<parking_lot::RwLock<TimeInfo>>,
    ) {
        self.current_file_path = Some(path.to_string());
        let json_text = std::fs::read_to_string(path).unwrap();
        let project: Project = serde_json::from_str(&json_text).unwrap();
        let mut instrument_track_senders2 = HashMap::new();
        let mut instrument_track_receivers2 = HashMap::new();

        self.project = project;

        // let mut song_length_in_beats: u64 = 0;

        // load all the samples - create the sample data objects
        let sample_rate = self.get_project().song_mut().sample_rate();
        let mut sample_references = HashMap::new();
        let mut samples_data = HashMap::new();
        for (_sample_uuid, sample) in self.get_project().song_mut().samples_mut().iter_mut() {
            let sample_data_uuid = sample.sample_data_uuid();
            let sample_file_name = sample.file_name();

            let sample_data = SampleData::new_with_uuid(sample_data_uuid.to_string(), sample_file_name.to_string(), sample_rate as i32);
            samples_data.insert(sample_data_uuid.to_string(), sample_data);
            sample_references.insert(sample.uuid().to_string(), sample_data_uuid.to_string());
        }
        for (sample_data_uuid, sample_data) in samples_data.iter() {
            self.sample_data_mut().insert(sample_data_uuid.to_string(), sample_data.clone());
        }

        debug!("state.load_from_file() - number of riff sequences={}", self.project().song().riff_sequences().len());

        {
            for track in self.get_project().song_mut().tracks_mut().iter_mut() {
                // Self::add_track(vst_plugin_loaders.clone(), tx_audio.clone(), track_audio_coast.clone(), &mut instrument_track_senders2, &mut instrument_track_receivers2, track_type)
                Self::init_track(
                    vst24_plugin_loaders.clone(),
                    clap_plugin_loaders.clone(),
                    tx_audio.clone(),
                    track_audio_coast.clone(),
                    &mut instrument_track_senders2,
                    &mut instrument_track_receivers2,
                    track,
                    Some(&sample_references),
                    Some(&samples_data),
                    vst_host_time_info.clone(),
                );
            }
        }

        self.update_track_senders_and_receivers(instrument_track_senders2, instrument_track_receivers2);

        {
            for track in self.project().song().tracks().iter() {
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
            for track in self.get_project().song_mut().tracks_mut().iter_mut() {
                track_uuids.push(track.uuid_string());

                for event in track.automation_mut().events_mut().iter_mut() {
                    event.set_id(Uuid::new_v4().to_string());
                }
            }

            // set the event ids in the riff arrangement automation events
            for riff_arrangement in self.get_project().song_mut().riff_arrangements_mut().iter_mut() {
                for track_uuid in track_uuids.iter() {
                    for automation in riff_arrangement.automation_mut(track_uuid) {
                        for event in automation.events_mut().iter_mut() {
                            event.set_id(Uuid::new_v4().to_string());
                        }
                    }
                }
            }
        }
    }

    pub fn update_track_senders_and_receivers(&mut self, instrument_track_senders2: HashMap<Option<String>, Sender<TrackBackgroundProcessorInwardEvent>>, instrument_track_receivers2: HashMap<Option<String>, Receiver<TrackBackgroundProcessorOutwardEvent>>) {
        for (uuid, sender) in instrument_track_senders2 {
            match uuid {
                Some(uuid) => {
                    self.instrument_track_senders_mut().insert(uuid, sender);
                },
                None => debug!("Entry did not contain a uuid."),
            }
        }

        for (uuid, receiver) in instrument_track_receivers2 {
            match uuid {
                Some(uuid) => {
                    self.instrument_track_receivers_mut().insert(uuid, receiver);
                },
                None => debug!("Entry did not contain a uuid."),
            }
        }
    }

    pub fn init_track(
        vst24_plugin_loaders: Arc<Mutex<HashMap<String, PluginLoader<VstHost>>>>,
        clap_plugin_loaders: Arc<Mutex<HashMap<String, PluginLibrary>>>,
        tx_audio: crossbeam_channel::Sender<AudioLayerInwardEvent>,
        track_audio_coast: Arc<Mutex<TrackBackgroundProcessorMode>>,
        instrument_track_senders2: &mut HashMap<Option<String>, Sender<TrackBackgroundProcessorInwardEvent>>,
        instrument_track_receivers2: &mut HashMap<Option<String>, Receiver<TrackBackgroundProcessorOutwardEvent>>,
        track_type: &mut TrackType,
        sample_references: Option<&HashMap<String, String>>,
        samples_data: Option<&HashMap<String, SampleData>>,
        vst_host_time_info: Arc<parking_lot::RwLock<TimeInfo>>,
    ) {
        let (tx_to_vst, rx_to_vst) = channel::<TrackBackgroundProcessorInwardEvent>();
        let tx_to_vst_ref = tx_to_vst.clone();
        let (tx_from_vst, rx_from_vst) = channel::<TrackBackgroundProcessorOutwardEvent>();
        let mut track_uuid = None;
        let volume = track_type.volume_mut();
        let pan = track_type.pan_mut();

        match track_type {
            TrackType::InstrumentTrack(track) => {
                let effect_presets = {
                    track_uuid = Some(track.uuid_mut().to_string());
                    instrument_track_senders2.insert(track_uuid.clone(), tx_to_vst);
                    instrument_track_receivers2.insert(track_uuid.clone(), rx_from_vst);

                    let track_uuid_string = track.uuid().to_string();
                    track.track_background_processor_mut().start_processing(
                        track_uuid_string,
                        tx_audio,
                        rx_to_vst,
                        tx_from_vst,
                        track_audio_coast,
                        volume,
                        pan,
                        vst_host_time_info,
                    );

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
    
                        match tx_to_vst_ref.send(TrackBackgroundProcessorInwardEvent::AddEffect(vst24_plugin_loaders.clone(), clap_plugin_loaders.clone(), effect.uuid(), effect_details)) {
                            Ok(_) => (),
                            Err(error) => debug!("Problem sending add effect: {}", error),
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

                    if instrument_details.contains(".so") || instrument_details.contains(".clap") {
                        match track_uuid {
                            Some(_) => {
                                match tx_to_vst_ref.send(TrackBackgroundProcessorInwardEvent::ChangeInstrument(
                                    vst24_plugin_loaders, clap_plugin_loaders, instrument_uuid, instrument_details)) {
                                    Ok(_) => {}
                                    Err(error) => debug!("Couldn't send instrument change event: {:?}", error)
                                }
                                let preset_data = instrument.preset_data();
                                if !preset_data.is_empty() {
                                    Some(preset_data)
                                } else {
                                    None
                                }
                            },
                            None => None,
                        }
                    } else {
                        None
                    }
                };
                if let Some(preset_data) = preset {
                    match tx_to_vst_ref.send(TrackBackgroundProcessorInwardEvent::SetPresetData(String::from(preset_data), effect_presets)) {
                        Ok(_) => (),
                        Err(error) => debug!("Couldn't send instrument preset data: {:?}", error),
                    }
                }
            },
            TrackType::AudioTrack(track) => {
                track_uuid = Some(track.uuid_mut().to_string());
                instrument_track_senders2.insert(track_uuid.clone(), tx_to_vst.clone());
                instrument_track_receivers2.insert(track_uuid, rx_from_vst);

                // send all sample data referenced in riffs to the track background processor
                for riff in track.riffs().iter() {
                    let riff: &Riff = riff;
                    for event in riff.events().iter() {
                        if let TrackEvent::Sample(sample_reference) = event {
                            if let Some(sample_references) = sample_references {
                                if let Some(sample_data_uuid) = sample_references.get(&sample_reference.sample_ref_uuid().to_string()) {
                                    if let Some(samples_data) = samples_data {
                                        if let Some(sample_data) = samples_data.get(sample_data_uuid) {
                                            match tx_to_vst.send(TrackBackgroundProcessorInwardEvent::SetSample(sample_data.clone())) {
                                                Ok(_) => {}
                                                Err(_) => {}
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                let track_uuid_string = track.uuid().to_string();
                track.track_background_processor_mut().start_processing(
                    track_uuid_string,
                    tx_audio,
                    rx_to_vst,
                    tx_from_vst,
                    track_audio_coast,
                    volume,
                    pan,
                    vst_host_time_info,
                );
            },
            TrackType::MidiTrack(track) => {
                track_uuid = Some(track.uuid_mut().to_string());
                instrument_track_senders2.insert(track_uuid.clone(), tx_to_vst);
                instrument_track_receivers2.insert(track_uuid, rx_from_vst);

                let track_uuid_string = track.uuid().to_string();
                track.track_background_processor_mut().start_processing(
                    track_uuid_string,
                    tx_audio,
                    rx_to_vst,
                    tx_from_vst,
                    track_audio_coast,
                    volume,
                    pan,
                    vst_host_time_info,
                );
            },
        }
    }

    pub fn start_default_track_background_processing(&mut self,
                                                     tx_audio: crossbeam_channel::Sender<AudioLayerInwardEvent>,
                                                     track_audio_coast: Arc<Mutex<TrackBackgroundProcessorMode>>,
                                                     track_uuid: String,
                                                     vst_host_time_info: Arc<parking_lot::RwLock<TimeInfo>>,
    ) {
        let (tx_to_vst, rx_to_vst) = channel::<TrackBackgroundProcessorInwardEvent>();
        let (tx_from_vst, rx_from_vst) = channel::<TrackBackgroundProcessorOutwardEvent>();
        let mut instrument_track_senders2 = HashMap::new();
        let mut instrument_track_receivers2 = HashMap::new();

        match self.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
            Some(track) => {
                let track_uuid_string = track.uuid().to_string();
                instrument_track_senders2.insert(track_uuid_string.clone(), tx_to_vst);
                instrument_track_receivers2.insert(track_uuid_string, rx_from_vst);
                track.start_background_processing(tx_audio, rx_to_vst, tx_from_vst, track_audio_coast, track.volume(), track.pan(), vst_host_time_info);
            },
            None => {}
        }

        for (uuid, sender) in instrument_track_senders2 {
            self.instrument_track_senders_mut().insert(uuid, sender);
        }

        for (uuid, receiver) in instrument_track_receivers2 {
            self.instrument_track_receivers_mut().insert(uuid, receiver);
        }
    }

    pub fn load_instrument(&mut self,
                            vst24_plugin_loaders: Arc<Mutex<HashMap<String, PluginLoader<VstHost>>>>,
                            clap_plugin_loaders: Arc<Mutex<HashMap<String, PluginLibrary>>>,
                            instrument_details: String,
                            track_uuid: String,
    ) {
        let mut index = 0;
        for track_type in self.get_project().song_mut().tracks_mut() {
            match track_type {
                TrackType::InstrumentTrack(track) => if track.uuid().to_string() == track_uuid {
                    let (sub_plugin_id, library_path, plugin_type) = get_plugin_details(instrument_details.clone());
                    let instrument = track.instrument_mut();
                    let instrument_uuid = instrument.uuid();

                    instrument.set_file(library_path);
                    instrument.set_sub_plugin_id(sub_plugin_id);
                    instrument.set_plugin_type(plugin_type);

                    if instrument_details.contains(".so") || instrument_details.contains(".clap") {
                        // instrument.load(vst_plugin_loaders, track_uuid.clone(), instrument_details, tx_audio.clone(), rx_vst, tx_from_vst, track_audio_coast);
                        match self.instrument_track_senders_mut().get_mut(&track_uuid) {
                            Some(sender) => {
                                match sender.send(TrackBackgroundProcessorInwardEvent::ChangeInstrument(
                                    vst24_plugin_loaders, clap_plugin_loaders, instrument_uuid, instrument_details)) {
                                    Ok(_) => (),
                                    Err(error) => debug!("{:?}", error),
                                }
                            },
                            None => debug!("Couldn't send message to track!"),
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

    pub fn send_to_track_background_processor(&self, track_hash: String, message: TrackBackgroundProcessorInwardEvent) {
        match self.instrument_track_senders().get(&track_hash) {
            Some(sender) => {
                match sender.send(message) {
                    Ok(_) => (),
                    Err(error) => debug!("{:?}", error),
                }
            },
            None => debug!("Couldn't send message to track!"),
        };
    }

    pub fn send_midi_routing_to_track_background_processors(&self, track_from_uuid: String, routing: TrackEventRouting) {
        // create the consumer producer pair
        let track_event_ring_buffer: SpscRb<TrackEvent> = SpscRb::new(1024);
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
        let audio_ring_buffer_left: SpscRb<f32> = SpscRb::new(1024);
        let audio_producer_left = audio_ring_buffer_left.producer();
        let audio_consumer_left = audio_ring_buffer_left.consumer();    
        let audio_ring_buffer_right: SpscRb<f32> = SpscRb::new(1024);
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
        debug!("Entering request_presets_from_all_tracks...");
        let mut uuids = vec![];
        {
            for track_type in self.get_project().song_mut().tracks_mut() {
                debug!("Found track");
                match track_type {
                    TrackType::InstrumentTrack(track) => {
                        debug!("Adding instrument track uuid to vector: {}", track.uuid());
                        uuids.push(track.uuid().to_string());
                    },
                    TrackType::AudioTrack(track) => {
                        debug!("Adding audio track uuid to vector: {}", track.uuid());
                        uuids.push(track.uuid().to_string());
                    },
                    TrackType::MidiTrack(_) => (),
                }
            }
        }

        {
            for uuid in uuids {
                debug!("Found uuid in vector: {}", &uuid);
                match self.instrument_track_senders_mut().get(&uuid) {
                    Some(sender) => {
                        debug!("State: requesting preset data from track with uuid: {}", uuid.clone());
                        match sender.send(TrackBackgroundProcessorInwardEvent::RequestPresetData) {
                            Ok(_) => (),
                            Err(error) => debug!("Problem requesting vst preset data for track: {}", error),
                        }
                    },
                    None => debug!("Could not find tx_to_vst thread for track."),
                }
            }
        }
        debug!("Exiting request_presets_from_all_tracks.");
    }

    fn save_presets_for_all_tracks(&mut self) {
        debug!("Entering save_presets_for_all_tracks...");
        let mut presets = HashMap::new();

        {
            let track_data = self.get_project().song_mut().tracks_mut().iter_mut().map(|track| (track.uuid().to_string(), match track {
                TrackType::InstrumentTrack(_) => GeneralTrackType::InstrumentTrack,
                TrackType::AudioTrack(_) => GeneralTrackType::AudioTrack,
                TrackType::MidiTrack(_) => GeneralTrackType::MidiTrack,
            })).collect_vec();
            for (track_uuid, track_type) in track_data.iter() {
                match track_type {
                    GeneralTrackType::InstrumentTrack => {
                        if let Some((uuid, vst_outward_receiver)) = self.instrument_track_receivers_mut().iter_mut().find(|(uuid, _)| *track_uuid == **uuid) {
                            match vst_outward_receiver.recv_timeout(Duration::from_secs(1)) {
                                Ok(preset_data) => {
                                    debug!("Instrument track preset data received: {}", uuid.clone());
                                    presets.insert(String::from(uuid.as_str()), preset_data);
                                },
                                Err(error) => debug!("Problem receiving instrument track vst thread preset data for track uuid: {} {}", uuid.clone(), error),
                            }
                        }
                    },
                    GeneralTrackType::AudioTrack => {
                        if let Some((uuid, vst_outward_receiver)) = self.instrument_track_receivers_mut().iter_mut().find(|(uuid, _)| *track_uuid == **uuid) {
                            match vst_outward_receiver.recv_timeout(Duration::from_secs(1)) {
                                Ok(preset_data) => {
                                    debug!("Audio track preset data received: {}", uuid.clone());
                                    presets.insert(String::from(uuid.as_str()), preset_data);
                                },
                                Err(error) => debug!("Problem receiving audio track vst thread preset data for track uuid: {} {}", uuid.clone(), error),
                            }
                        }
                    },
                    _ => (),
                }
            }
        }

        {
            for (uuid, preset_data) in presets {
                for track_type in self.get_project().song_mut().tracks_mut() {
                    match track_type {
                        TrackType::InstrumentTrack(track) => {
                            if track.uuid().to_string().as_str() == uuid.as_str() {
                                if let TrackBackgroundProcessorOutwardEvent::GetPresetData(instrument_preset, effect_presets)  = preset_data {
                                    track.instrument_mut().set_preset_data(instrument_preset);
                                    let mut index = 0;
                                    for effect_preset in effect_presets {
                                        match track.effects_mut().get_mut(index) {
                                            Some(effect) => effect.set_preset_data(effect_preset),
                                            None => debug!("Effect could not be found for effect preset data at index: {}", index),
                                        }
                                        index += 1;
                                    }
                                }
                                break;
                            }
                        },
                        TrackType::AudioTrack(track) => {
                            if track.uuid().to_string().as_str() == uuid.as_str() {
                                if let TrackBackgroundProcessorOutwardEvent::GetPresetData(_instrument_preset, effect_presets)  = preset_data {
                                    let mut index = 0;
                                    for effect_preset in effect_presets {
                                        match track.effects_mut().get_mut(index) {
                                            Some(effect) => effect.set_preset_data(effect_preset),
                                            None => debug!("Effect could not be found for effect preset data at index: {}", index),
                                        }
                                        index += 1;
                                    }
                                }
                                break;
                            }
                        },
                        TrackType::MidiTrack(_) => (),
                    }
                }
            }
        }
        debug!("Exiting save_presets_for_all_tracks...");
    }

    pub fn save(&mut self) {
        debug!("Entering save...");
        self.request_presets_from_all_tracks();
        self.save_presets_for_all_tracks();

        self.get_project().song_mut().recalculate_song_length();

        debug!("state.save() - number of riff sequences={}", self.project().song().riff_sequences().len());

        match serde_json::to_string_pretty(self.get_project()) {
            Ok(json_text) => {
                match self.get_current_file_path() {
                    Some(path) => {
                        match std::fs::write(path.clone(), json_text) {
                            Err(error) => debug!("save failure writing to file: {}", error),
                            _ => {
                                debug!("saved to file: {}", path);
                                self.dirty = false;
                            }
                        };
                    },
                    None => debug!("No file path."),
                }
            },
            Err(error) => {
                debug!("can_serialise failure: {}",error);
            }
        };
        debug!("Exited save.");
    }

    pub fn autosave(&mut self) {
        debug!("Entering autosave...");
        self.request_presets_from_all_tracks();
        self.save_presets_for_all_tracks();

        self.get_project().song_mut().recalculate_song_length();

        match serde_json::to_string_pretty(self.get_project()) {
            Ok(json_text) => {
                match self.get_current_file_path() {
                    Some(path) => {
                        let autosave_path = format!("{}_{}.fdaw.xz", path, chrono::offset::Local::now().to_string());
                        if let Ok(compressed) = lzma::compress(json_text.as_bytes(), 6) {
                            match std::fs::write(autosave_path.clone(), compressed) {
                                Err(error) => debug!("save failure writing to file: {}", error),
                                _ => debug!("saved to file: {}", autosave_path)
                            };
                        }
                    }
                    None => {
                        let path = format!("/tmp/unknown_{}.fdaw.xz", chrono::offset::Local::now().to_string());
                        if let Ok(compressed) = lzma::compress(json_text.as_bytes(), 6) {
                            match std::fs::write(path.clone(), compressed) {
                                Err(error) => debug!("save failure writing to file: {}", error),
                                _ => debug!("saved to file: {}", path)
                            }
                        }
                    }
                }
            }
            Err(error) => {
                debug!("autosave can't serialise project to JSON failure: {}",error);
            }
        };
        debug!("Exited autosave.");
    }

    pub fn save_as(&mut self, path: &str) {
        self.request_presets_from_all_tracks();
        self.save_presets_for_all_tracks();

        self.current_file_path = Some(path.to_string());
        match serde_json::to_string_pretty(self.get_project()) {
            Ok(json_text) => {
                match std::fs::write(path, json_text) {
                    Err(error) => debug!("save as failure writing to file: {}", error),
                    _ => {
                        self.dirty = false;
                    }
                };
            },
            Err(error) => {
                debug!("can_serialise failure: {}",error);
            }
        };
    }

    pub fn get_project(&mut self) -> &mut Project {
        &mut self.project
    }

    pub fn get_current_file_path(&self) -> &Option<String> {
        // let boris = self.current_file_path.clone().unwrap();
        // let mick = String::from(&boris[0..boris.len()]);
        // mick
        &self.current_file_path
    }

    pub fn set_project(&mut self, project: Project) {
        self.project = project;
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

    /// Get a mutable reference to the freedom daw state's instrument track senders.
    pub fn instrument_track_senders_mut(&mut self) -> &mut HashMap<String, Sender<TrackBackgroundProcessorInwardEvent>> {
        &mut self.instrument_track_senders
    }

    /// Get a mutable reference to the freedom daw state's instrument track receivers.
    pub fn instrument_track_receivers_mut(&mut self) -> &mut HashMap<String, Receiver<TrackBackgroundProcessorOutwardEvent>> {
        &mut self.instrument_track_receivers
    }

    /// Get a reference to the freedom daw state's instrument track senders.
    pub fn instrument_track_senders(&self) -> &HashMap<String, Sender<TrackBackgroundProcessorInwardEvent>> {
        &self.instrument_track_senders
    }

    /// Get a reference to the freedom daw state's instrument track receivers.
    pub fn instrument_track_receivers(&self) -> &HashMap<String, Receiver<TrackBackgroundProcessorOutwardEvent>> {
        &self.instrument_track_receivers
    }

    /// Get a reference to the freedom daw state's project.
    pub fn project(&self) -> &Project {
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

    /// Get a reference to the freedom daw state's riff references copy buffer.
    pub fn riff_references_copy_buffer(&self) -> &[RiffReference] {
        self.riff_references_copy_buffer.as_ref()
    }

    /// Get a mutable reference to the freedom daw state's riff references copy buffer.
    pub fn riff_references_copy_buffer_mut(&mut self) -> &mut Vec<RiffReference> {
        &mut self.riff_references_copy_buffer
    }

    /// Get a reference to the freedom daw state's automation view mode.
    #[must_use]
    pub fn automation_view_mode(&self) -> &AutomationViewMode {
        &self.automation_view_mode
    }

    /// Set the freedom daw state's automation view mode.
    pub fn set_automation_view_mode(&mut self, automation_view_mode: AutomationViewMode) {
        self.automation_view_mode = automation_view_mode;
    }

    /// Get a mutable reference to the freedom daw state's automation view mode.
    #[must_use]
    pub fn automation_view_mode_mut(&mut self) -> &mut AutomationViewMode {
        &mut self.automation_view_mode
    }

    /// Get the freedom daw state's automation type.
    #[must_use]
    pub fn automation_type(&self) -> Option<i32> {
        self.automation_type
    }

    /// Get a mutable reference to the freedom daw state's automation type.
    #[must_use]
    pub fn automation_type_mut(&mut self) -> &mut Option<i32> {
        &mut self.automation_type
    }

    /// Set the freedom daw state's automation type.
    pub fn set_automation_type(&mut self, automation_type: Option<i32>) {
        self.automation_type = automation_type;
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

    pub fn play_song(&mut self, tx_to_audio: crossbeam_channel::Sender<AudioLayerInwardEvent>) -> i32 {
        let mut bpm = 140.0;
        let mut sample_rate = 44100.0;
        let mut block_size = 1024.0;
        let mut song_length_in_beats = 400.0;
        let mut start_block = 0;
        let mut end_block = 0;
        let mut found_active_loop = false;

        song_length_in_beats = *self.get_project().song_mut().length_in_beats_mut() as f64;
        self.set_playing(true);
        self.set_play_mode(PlayMode::Song);

        let song = self.project().song();
        bpm = song.tempo();
        sample_rate = song.sample_rate();
        block_size = song.block_size();
        let play_position_in_frames = self.play_position_in_frames();
        start_block = (play_position_in_frames as f64 / block_size) as i32;


        if self.looping {
            if  let Some(loop_uuid) = &self.active_loop {
                let song: &Song = self.project().song();
                if let Some(active_loop) = song.loops().iter().find(|current_loop| current_loop.uuid().to_string() == loop_uuid.to_string()) {
                    let start_position_in_beats = active_loop.start_position();
                    let end_position_in_beats = active_loop.end_position();

                    found_active_loop = true;

                    start_block = (start_position_in_beats * sample_rate * 60.0 / bpm / block_size) as i32;
                    end_block = (end_position_in_beats * sample_rate * 60.0 / bpm / block_size) as i32;
                }
            }
        }

        let tracks = song.tracks();
        for track in tracks {
            let midi_channel = if let TrackType::MidiTrack(midi_track) = track {
                midi_track.midi_device().midi_channel()
            }
            else {
                0
            };
            let vst_event_blocks = DAWUtils::convert_to_event_blocks(track.automation().events(), track.riffs(), track.riff_refs(), bpm, block_size, sample_rate, song_length_in_beats, midi_channel);
            self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEventProcessorType(EventProcessorType::BlockEventProcessor));
            self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEvents(vst_event_blocks, false));

            if found_active_loop {
                self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Loop(true));
                self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::LoopExtents(start_block, end_block));
            }
        }

        // thread::sleep(Duration::from_millis(2000));

        for track in tracks {
            self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Play(start_block));
        }

        let number_of_blocks = (song_length_in_beats / bpm * 60.0 * sample_rate / block_size) as i32;
        match tx_to_audio.send(AudioLayerInwardEvent::Play(true, number_of_blocks, start_block)) {
                Ok(_) => (),
                Err(error) => debug!("Problem using tx_to_audio to send message to jack layer when turning play on: {}", error),
        }

        number_of_blocks
    }

    pub fn play_riff_set(&mut self, tx_to_audio: crossbeam_channel::Sender<AudioLayerInwardEvent>, riff_set_uuid: String) {
        debug!("Playing riff set={}", riff_set_uuid.as_str());

        // self.play_riff_set_in_blocks(tx_to_audio, riff_set_uuid);
        self.play_riff_set_as_riff(tx_to_audio, riff_set_uuid);
    }

    pub fn play_riff_set_in_blocks(&mut self, tx_to_audio: crossbeam_channel::Sender<AudioLayerInwardEvent>, riff_set_uuid: String) {
        debug!("Playing riff set in blocks={}", riff_set_uuid.as_str());

        let already_playing = self.playing();

        self.set_playing(true);
        self.set_play_mode(PlayMode::RiffSet);
        self.set_playing_riff_set(Some(riff_set_uuid.clone()));

        let song = self.project().song();
        let play_position_in_frames = 0;
        let tracks = song.tracks();
        let bpm = song.tempo();
        let sample_rate = song.sample_rate();
        let block_size = song.block_size();
        let start_block = (play_position_in_frames as f64 / block_size) as i32;
        let mut lowest_common_factor_in_beats = 400;

        if let Some(riff_set) = self.project().song().riff_set(riff_set_uuid.clone()) {
            let mut riff_lengths = vec![];
            debug!("Found riff set: uuid={}, name={}", riff_set_uuid.as_str(), riff_set.name());

            // get the number of repeats
            for track in self.project().song().tracks().iter() {
                // get the riff_ref
                if let Some(riff_ref) = riff_set.get_riff_ref_for_track(track.uuid().to_string()) {
                    // get the riff
                    if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_ref.linked_to()) {
                        riff_lengths.push(riff.length() as i32);
                    }
                }
            }

            let (product, unique_riff_lengths) = DAWState::get_length_product(riff_lengths);

            lowest_common_factor_in_beats = DAWState::get_lowest_common_factor(unique_riff_lengths, product);

            for track in self.project().song().tracks().iter() {
                let mut riff_refs = vec![];
                let midi_channel = if let TrackType::MidiTrack(midi_track) = track {
                    midi_track.midi_device().midi_channel()
                }
                else {
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
                        let automation: Vec<TrackEvent> = vec![];
                        let track_event_blocks = DAWUtils::convert_to_event_blocks(&automation, &riffs, &riff_refs, bpm, block_size, sample_rate, lowest_common_factor_in_beats as f64, midi_channel);
                        debug!("Riff set # of blocks: {}", track_event_blocks.0.len());
                        self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::LoopExtents(0, track_event_blocks.0.len() as i32));
                        self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEvents(track_event_blocks, true));
                        self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Loop(true));
                    }
                    else {
                        let track_event_blocks = (vec![], vec![]);
                        self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEvents(track_event_blocks, true));
                    }
                }
                else {
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
        match tx_to_audio.send(AudioLayerInwardEvent::Play(true, number_of_blocks, start_block)) {
                Ok(_) => (),
                Err(error) => debug!("Problem using tx_to_audio to send message to jack layer when turning play riff set on: {}", error),
        }
    }

    pub fn play_riff_set_as_riff(&mut self, tx_to_audio: crossbeam_channel::Sender<AudioLayerInwardEvent>, riff_set_uuid: String) {
        debug!("Playing riff set as riff={}", riff_set_uuid.as_str());

        let already_playing = self.playing();

        self.set_playing(true);
        self.set_play_mode(PlayMode::RiffSet);
        self.set_playing_riff_set(Some(riff_set_uuid.clone()));

        let song = self.project().song();
        let play_position_in_frames = 0;
        let tracks = song.tracks();
        let bpm = song.tempo();
        let sample_rate = song.sample_rate();
        let block_size = song.block_size();
        let start_block = (play_position_in_frames as f64 / block_size) as i32;
        let number_of_blocks = i32::MAX;

        if let Some(riff_set) = self.project().song().riff_set(riff_set_uuid.clone()) {
            debug!("Found riff set: uuid={}, name={}", riff_set_uuid.as_str(), riff_set.name());

            for track in self.project().song().tracks().iter() {
                let mut riff_refs = vec![];
                let midi_channel = if let TrackType::MidiTrack(midi_track) = track {
                    midi_track.midi_device().midi_channel()
                }
                else {
                    0
                };

                if !already_playing {
                    self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEventProcessorType(EventProcessorType::RiffBufferEventProcessor));
                }

                // get the riff_ref
                if let Some(riff_ref) = riff_set.get_riff_ref_for_track(track.uuid().to_string()) {
                    let mut riff_reference = riff_ref.clone();
                    riff_refs.push(riff_reference);

                    // get the riff
                    if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_ref.linked_to()) {
                        let mut riffs = vec![];
                        riffs.push(riff.clone());

                        let mut track_events: Vec<TrackEvent> = DAWUtils::extract_riff_ref_events(&riffs, &riff_refs, bpm, sample_rate, midi_channel);

                        for track_event in track_events.iter() {
                            match track_event {
                                TrackEvent::ActiveSense => debug!("After sense: position={}", track_event.position()),
                                TrackEvent::AfterTouch => debug!("After touch: position={}", track_event.position()),
                                TrackEvent::ProgramChange => debug!("Program change: position={}", track_event.position()),
                                TrackEvent::Note(_) => debug!("Note: position={}", track_event.position()),
                                TrackEvent::NoteOn(_) => debug!("Note on: position={}", track_event.position()),
                                TrackEvent::NoteOff(_) => debug!("Note off: position={}", track_event.position()),
                                TrackEvent::NoteExpression(_) => debug!("Note expression: position={}", track_event.position()),
                                TrackEvent::Controller(_) => debug!("Controller: position={}", track_event.position()),
                                TrackEvent::PitchBend(_) => debug!("Pitch bend: position={}", track_event.position()),
                                TrackEvent::KeyPressure => debug!("Key pressure: position={}", track_event.position()),
                                TrackEvent::AudioPluginParameter(_) => debug!("Audio plugin parameter: position={}", track_event.position()),
                                TrackEvent::Sample(_) => debug!("Sample: position={}", track_event.position()),
                                TrackEvent::Measure(_) => debug!("Measure: position={}", track_event.position()),
                            }
                        }

                        let track_event_blocks = vec![track_events];

                        // TODO this needs to be patched in
                        let automation: Vec<PluginParameter> = vec![];
                        let automation_event_blocks = vec![automation];

                        // let track_event_blocks = DAWUtils::convert_to_event_blocks(&automation, &riffs, &riff_refs, bpm, block_size, sample_rate, lowest_common_factor_in_beats as f64, midi_channel);
                        debug!("Riff set # of blocks: {}", track_event_blocks.len());
                        self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::LoopExtents(0, number_of_blocks));
                        self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEvents((track_event_blocks, automation_event_blocks), true));
                        self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Loop(true));
                    }
                    else {
                        let track_event_blocks = (vec![], vec![]);
                        self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEvents(track_event_blocks, true));
                    }
                }
                else {
                    let track_event_blocks = (vec![], vec![]);
                    self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEvents(track_event_blocks, true));
                }
            }
        }

        if !already_playing {
            for track in tracks {
                self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Play(start_block));
            }

            match tx_to_audio.send(AudioLayerInwardEvent::Play(true, number_of_blocks, start_block)) {
                Ok(_) => (),
                Err(error) => debug!("Problem using tx_to_audio to send message to jack layer when turning play riff set as riff on: {}", error),
            }
        }
    }



    pub fn play_riff_set_update_track_as_riff(&self, riff_set_uuid: String, track_uuid: String) {
        let song = self.project().song();
        let bpm = song.tempo();
        let sample_rate = song.sample_rate();
        let number_of_blocks = i32::MAX;


        if let Some(riff_set) = self.project().song().riff_set(riff_set_uuid) {
            debug!("state.play_riff_set_update_track: found riff set");
            for track in self.project().song().tracks().iter() {
                if track.uuid().to_string() == track_uuid {
                    debug!("state.play_riff_set_update_track_as_riff: found track");
                    let mut riff_refs = vec![];
                    let midi_channel = if let TrackType::MidiTrack(midi_track) = track {
                        midi_track.midi_device().midi_channel()
                    }
                    else {
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

                            let mut track_events: Vec<TrackEvent> = DAWUtils::extract_riff_ref_events(&riffs, &riff_refs, bpm, sample_rate, midi_channel);

                            for track_event in track_events.iter() {
                                match track_event {
                                    TrackEvent::ActiveSense => debug!("After sense: position={}", track_event.position()),
                                    TrackEvent::AfterTouch => debug!("After touch: position={}", track_event.position()),
                                    TrackEvent::ProgramChange => debug!("Program change: position={}", track_event.position()),
                                    TrackEvent::Note(_) => debug!("Note: position={}", track_event.position()),
                                    TrackEvent::NoteOn(_) => debug!("Note on: position={}", track_event.position()),
                                    TrackEvent::NoteOff(_) => debug!("Note off: position={}", track_event.position()),
                                    TrackEvent::NoteExpression(_) => debug!("Note expression: position={}", track_event.position()),
                                    TrackEvent::Controller(_) => debug!("Controller: position={}", track_event.position()),
                                    TrackEvent::PitchBend(_) => debug!("Pitch bend: position={}", track_event.position()),
                                    TrackEvent::KeyPressure => debug!("Key pressure: position={}", track_event.position()),
                                    TrackEvent::AudioPluginParameter(_) => debug!("Audio plugin parameter: position={}", track_event.position()),
                                    TrackEvent::Sample(_) => debug!("Sample: position={}", track_event.position()),
                                    TrackEvent::Measure(_) => debug!("Measure: position={}", track_event.position()),
                                }
                            }

                            let track_event_blocks = vec![track_events];

                            // TODO this needs to be patched in
                            let automation: Vec<PluginParameter> = vec![];
                            let automation_event_blocks = vec![automation];

                            debug!("Riff set # of blocks: {}", track_event_blocks.len());
                            self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::LoopExtents(0, number_of_blocks));
                            self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEvents((track_event_blocks, automation_event_blocks), true));
                            self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Loop(true));
                        }
                        else {
                            let vst_event_blocks = (vec![], vec![]);
                            debug!("state.play_riff_set_update_track_as_riff: sending message to vst - set events without data");
                            self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEvents(vst_event_blocks, true));
                        }
                    }
                    else {
                        let vst_event_blocks = (vec![], vec![]);
                        debug!("state.play_riff_set_update_track_as_riff: sending message to vst - set events without data");
                        self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEvents(vst_event_blocks, true));
                    }
                    break;
                }
            }
        }
    }



    pub fn play_riff_set_update_track_in_blocks(&self, riff_set_uuid: String, track_uuid: String) {
        let song = self.project().song();
        let bpm = song.tempo();
        let sample_rate = song.sample_rate();
        let block_size = song.block_size();
        let mut lowest_common_factor_in_beats = 400;


        if let Some(riff_set) = self.project().song().riff_set(riff_set_uuid) {
            debug!("state.play_riff_set_update_track_in_blocks: found riff set");
            let mut riff_lengths = vec![];

            // get the number of repeats
            for track in self.project().song().tracks().iter() {
                // get the riff_ref
                if let Some(riff_ref) = riff_set.get_riff_ref_for_track(track.uuid().to_string()) {
                    // get the riff
                    if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_ref.linked_to()) {
                        riff_lengths.push(riff.length() as i32);
                    }
                }
            }

            let (product, unique_riff_lengths) = DAWState::get_length_product(riff_lengths);

            lowest_common_factor_in_beats = DAWState::get_lowest_common_factor(unique_riff_lengths, product);

            for track in self.project().song().tracks().iter() {
                if track.uuid().to_string() == track_uuid {
                    debug!("state.play_riff_set_update_track_in_blocks: found track");
                    let mut riff_refs = vec![];
                    let midi_channel = if let TrackType::MidiTrack(midi_track) = track {
                        midi_track.midi_device().midi_channel()
                    }
                    else {
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
                            let automation: Vec<TrackEvent> = vec![];
                            let vst_event_blocks = DAWUtils::convert_to_event_blocks(&automation, &riffs, &riff_refs, bpm, block_size, sample_rate, lowest_common_factor_in_beats as f64, midi_channel);
                            debug!("state.play_riff_set_update_track_in_blocks: sending message to vst - set events with data");
                            self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEvents(vst_event_blocks, true));
                        }
                        else {
                            let vst_event_blocks = (vec![], vec![]);
                            debug!("state.play_riff_set_update_track_in_blocks: sending message to vst - set events without data");
                            self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEvents(vst_event_blocks, true));
                        }
                    }
                    else {
                        let vst_event_blocks = (vec![], vec![]);
                        debug!("state.play_riff_set_update_track_in_blocks: sending message to vst - set events without data");
                        self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEvents(vst_event_blocks, true));
                    }
                    break;
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



    pub fn play_riff_sequence(&mut self, tx_to_audio: crossbeam_channel::Sender<AudioLayerInwardEvent>, riff_sequence_uuid: String) {
        let song_length_in_beats = 400.0;

        let already_playing = self.playing();

        self.set_playing(true);
        self.set_play_mode(PlayMode::RiffSequence);
        let song = self.project().song();
        let play_position_in_frames = 0;
        let bpm = song.tempo();
        let sample_rate = song.sample_rate();
        let block_size = song.block_size();
        let start_block = (play_position_in_frames as f64 / block_size) as i32;

        // get the riff sequence
        if let Some(riff_sequence) = song.riff_sequence(riff_sequence_uuid) {
            let mut track_riff_refs_map = HashMap::new();
            let mut track_running_position = HashMap::new();

            // setup
            for track in self.project().song().tracks().iter() {
                let track_riff_refs: Vec<RiffReference> = vec![];
                track_riff_refs_map.insert(track.uuid().to_string(), track_riff_refs);
                track_running_position.insert(track.uuid().to_string(), 0.0);

                if !already_playing {
                    self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEventProcessorType(EventProcessorType::BlockEventProcessor));
                }
            }

            self.playing_riff_sequence_summary_data = Some(self.get_riff_sequence_play_events(riff_sequence, &mut track_riff_refs_map, &mut track_running_position));

            // convert and send events
            for track in self.project().song().tracks().iter() {
                debug!("Track: uuid={} - ", track.uuid().to_string());

                let midi_channel = if let TrackType::MidiTrack(midi_track) = track {
                    midi_track.midi_device().midi_channel()
                }
                else {
                    0
                };
                // get the riff refs
                let riff_refs= match track_riff_refs_map.remove(track.uuid().to_string().as_str()) {
                    None => Vec::<RiffReference>::new(),
                    Some(riff_refs) => riff_refs,
                };
                for riff_ref in riff_refs.iter() {
                    debug!("Riff ref: uuid={}, position={}, length={} - ", riff_ref.uuid().to_string(), riff_ref.position(), riff_ref.linked_to());
                }
                debug!("");
                let automation: Vec<TrackEvent> = vec![];
                let vst_event_blocks = DAWUtils::convert_to_event_blocks(&automation, track.riffs(), &riff_refs, bpm, block_size, sample_rate, song_length_in_beats, midi_channel);
                                self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEvents(vst_event_blocks, false));
            }
        }

        let number_of_blocks = (song_length_in_beats / bpm * 60.0 * sample_rate / block_size) as i32;

        // tell each track audio to play
        for track in self.project().song().tracks() {
            self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Play(start_block));
            self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::LoopExtents(start_block, number_of_blocks));
        }

        // set the start block and the number of blocks in the jack audio layer
        match tx_to_audio.send(AudioLayerInwardEvent::Play(true, number_of_blocks, start_block)) {
                Ok(_) => (),
                Err(error) => debug!("Problem using tx_to_audio to send message to jack layer when turning play riff sequence on: {}", error),
        }
    }



    pub fn get_riff_arrangement_play_events(
        &self,
        riff_arrangement: &RiffArrangement,
        track_riff_refs_map: &mut HashMap<String, Vec<RiffReference>>,
        track_running_position: &mut HashMap<String, f64>) -> (f64, Vec<(f64, RiffItem, Vec<(f64, RiffItem)>)>) {
        let mut riff_arrangement_actual_play_length = 0.0;
        let mut riff_item_actual_play_lengths = vec![];
        for riff_item in riff_arrangement.items().iter() {
            match riff_item.item_type() {
                RiffItemType::RiffSet => {
                    if let Some(riff_set) = self.project().song().riff_set(riff_item.item_uuid().to_string()) {
                        debug!("state.play_arrangement: riff set name={}", riff_set.name());
                        let riff_set_actual_play_length = self.get_riff_set_play_events(riff_set, track_riff_refs_map, track_running_position);
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
                    if let Some(riff_sequence) = self.project().song().riff_sequence(riff_item.item_uuid().to_string()) {
                        debug!("state.play_arrangement: riff sequence name={}", riff_sequence.name());
                        let riff_sequence_actual_details = self.get_riff_sequence_play_events(riff_sequence, track_riff_refs_map, track_running_position);
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
            }
        }

        (riff_arrangement_actual_play_length, riff_item_actual_play_lengths)
    }



    pub fn get_riff_sequence_play_events(
        &self,
        riff_sequence: &RiffSequence,
        track_riff_refs_map: &mut HashMap<String, Vec<RiffReference>>,
        track_running_position: &mut HashMap<String, f64>) -> (f64, Vec<(f64, String, String)>) {
        let mut riff_sequence_actual_play_length = 0.0;
        let mut riff_set_actual_play_lengths = vec![];
        for riff_set_reference in riff_sequence.riff_sets().iter() {
            if let Some(riff_set) = self.project().song().riff_set(riff_set_reference.item_uuid().to_string()) {
                debug!("state.play_sequence: riff set name={}", riff_set.name());
                let riff_set_actual_play_length = self.get_riff_set_play_events(riff_set, track_riff_refs_map, track_running_position);
                riff_sequence_actual_play_length += riff_set_actual_play_length;
                riff_set_actual_play_lengths.push((riff_set_actual_play_length, riff_set_reference.uuid(), riff_set_reference.item_uuid().to_string()));
            }
        }

        (riff_sequence_actual_play_length, riff_set_actual_play_lengths)
    }



    pub fn get_riff_set_play_events(
        &self,
        riff_set: &RiffSet,
        track_riff_refs_map: &mut HashMap<String, Vec<RiffReference>>,
        track_running_position: &mut HashMap<String, f64>) -> f64 {
        let mut riff_lengths = vec![];

        // get the track riff_lengths
        for track in self.project().song().tracks().iter() {
            // get the riff_ref
            if let Some(riff_ref) = riff_set.get_riff_ref_for_track(track.uuid().to_string()) {
                // get the riff
                if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_ref.linked_to()) {
                    riff_lengths.push(riff.length() as i32);
                }
            }
        }

        let (product, unique_riff_lengths) = DAWState::get_length_product(riff_lengths);

        let lowest_common_factor_in_beats = DAWState::get_lowest_common_factor(unique_riff_lengths, product);

        for (track_uuid, riff_ref) in riff_set.riff_refs() {
            if let None = track_running_position.get(&track_uuid.clone()) {
                track_running_position.insert(track_uuid.clone(), 0.0);
            }

            if let Some(&mut position) = track_running_position.get_mut(&track_uuid.clone()) {
                // get the riff refs
                if let Some(riff_refs)= track_riff_refs_map.get_mut(track_uuid) {
                    // get the track
                    let track_option = self.project().song().tracks().iter().find(|track| {
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



    pub fn play_riff_arrangement(&mut self, tx_to_audio: crossbeam_channel::Sender<AudioLayerInwardEvent>, riff_arrangement_uuid: String, play_position_in_beats: f64) {
        let mut song_length_in_beats = 400.0;

        let already_playing = self.playing();

        self.set_playing(true);
        self.set_play_mode(PlayMode::RiffArrangement);
        let song = self.project().song();
        let bpm = song.tempo();
        let sample_rate = song.sample_rate();
        let play_position_in_frames = play_position_in_beats / bpm * 60.0 * sample_rate;
        let block_size = song.block_size();
        let start_block = (play_position_in_frames / block_size) as i32;

        // get the riff arrangement
        if let Some(riff_arrangement) = song.riff_arrangement(riff_arrangement_uuid) {
            let mut track_riff_refs_map = HashMap::new();
            let mut track_running_position = HashMap::new();

            // setup
            for track in self.project().song().tracks().iter() {
                let track_riff_refs: Vec<RiffReference> = vec![];
                track_riff_refs_map.insert(track.uuid().to_string(), track_riff_refs);
                track_running_position.insert(track.uuid().to_string(), 0.0);

                if !already_playing {
                    self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEventProcessorType(EventProcessorType::BlockEventProcessor));
                }
            }

            // process all the items in the arrangement
            for item in riff_arrangement.items().iter() {
                match item.item_type() {
                    RiffItemType::RiffSet => {
                        // find the riff set and process its events
                        if let Some(riff_set) = self.project().song().riff_sets().iter().find(|current_riff_set| current_riff_set.uuid() == item.item_uuid()) {
                            self.get_riff_set_play_events(riff_set, &mut track_riff_refs_map, &mut track_running_position);
                        }
                    }
                    RiffItemType::RiffSequence => {
                        // find the riff sequence and process its events
                        if let Some(riff_sequence) = self.project().song().riff_sequences().iter().find(|current_riff_sequence| current_riff_sequence.uuid() == item.item_uuid()) {
                            self.get_riff_sequence_play_events(riff_sequence, &mut track_riff_refs_map, &mut track_running_position);
                        }
                    }
                }
            }

            self.playing_riff_arrangement_summary_data = Some(self.get_riff_arrangement_play_events(riff_arrangement, &mut track_riff_refs_map, &mut track_running_position));

            // find the longest track running position and set the length to be played to that
            if let Some(largest_track_length) = track_running_position.values().into_iter().max_by(|a, b| a.partial_cmp(b).unwrap()) {
                song_length_in_beats = *largest_track_length;
            }

            // convert and send events
            for track in self.project().song().tracks().iter() {
                let midi_channel = if let TrackType::MidiTrack(midi_track) = track {
                    midi_track.midi_device().midi_channel()
                }
                else {
                    0
                };
                // get the riff refs
                let riff_refs= match track_riff_refs_map.remove(track.uuid().to_string().as_str()) {
                    None => Vec::<RiffReference>::new(),
                    Some(riff_refs) => riff_refs,
                };
                let automation: Vec<TrackEvent> = vec![];
                let vst_event_blocks = DAWUtils::convert_to_event_blocks(&automation, track.riffs(), &riff_refs, bpm, block_size, sample_rate, song_length_in_beats, midi_channel);
                self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetEvents(vst_event_blocks, false));
                self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Loop(false));
            }
        }

        let number_of_blocks = (song_length_in_beats / bpm * 60.0 * sample_rate / block_size) as i32;

        // tell each track audio to play
        for track in self.project().song().tracks() {
            self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Play(start_block));
            self.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::LoopExtents(-1, -1));
        }

        // set the start block and the number of blocks in the jack audio layer
        match tx_to_audio.send(AudioLayerInwardEvent::Play(true, number_of_blocks, start_block)) {
                Ok(_) => (),
                Err(error) => debug!("Problem using tx_to_audio to send message to jack layer when turning play riff arrangement on: {}", error),
        }
    }



    pub fn calculate_riff_arrangement_length(
        &mut self, tx_to_audio: crossbeam_channel::Sender<AudioLayerInwardEvent>,
        riff_arrangement_uuid: String,
    ) -> f64 {
        let song = self.project().song();
        let mut riff_arrangement_length = 0.0;

        // get the riff arrangement
        if let Some(riff_arrangement) = song.riff_arrangement(riff_arrangement_uuid) {
            // process all the items in the arrangement
            for item in riff_arrangement.items().iter() {
                match item.item_type() {
                    RiffItemType::RiffSet => {
                        // find the riff set and process its events
                        if let Some(riff_set) = self.project().song().riff_sets().iter().find(|current_riff_set| current_riff_set.uuid() == item.item_uuid()) {
                            let mut riff_lengths = vec![];

                            // get the track riff_lengths
                            for track in self.project().song().tracks().iter() {
                                // get the riff_ref
                                if let Some(riff_ref) = riff_set.get_riff_ref_for_track(track.uuid().to_string()) {
                                    // get the riff
                                    if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_ref.linked_to()) {
                                        riff_lengths.push(riff.length() as i32);
                                    }
                                }
                            }

                            let (product, unique_riff_lengths) = DAWState::get_length_product(riff_lengths);
                            riff_arrangement_length += DAWState::get_lowest_common_factor(unique_riff_lengths, product) as f64;
                        }
                    }
                    RiffItemType::RiffSequence => {
                        // find the riff sequence and process its events
                        if let Some(riff_sequence) = self.project().song().riff_sequences().iter().find(|current_riff_sequence| current_riff_sequence.uuid() == item.item_uuid()) {
                            for riff_set_reference in riff_sequence.riff_sets().iter() {
                                if let Some(riff_set) = self.project().song().riff_set(riff_set_reference.item_uuid().to_string()) {
                                    let mut riff_lengths = vec![];

                                    // get the track riff_lengths
                                    for track in self.project().song().tracks().iter() {
                                        // get the riff_ref
                                        if let Some(riff_ref) = riff_set.get_riff_ref_for_track(track.uuid().to_string()) {
                                            // get the riff
                                            if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_ref.linked_to()) {
                                                riff_lengths.push(riff.length() as i32);
                                            }
                                        }
                                    }

                                    let (product, unique_riff_lengths) = DAWState::get_length_product(riff_lengths);
                                    riff_arrangement_length += DAWState::get_lowest_common_factor(unique_riff_lengths, product) as f64;
                                }
                            }
                        }
                    }
                }
            }
        }

        riff_arrangement_length
    }



    pub fn riff_set_increment_riff_for_track(&mut self, riff_set_uuid: String, track_uuid: String) {
        debug!("state.riff_set_increment_riff_for_track: {}, {}", riff_set_uuid.as_str(), track_uuid.as_str());
        // get the track
        let riff_uuids: Vec<String> = match self.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
            Some(track) => {
                track.riffs_mut().iter().map(|riff| riff.uuid().to_string()).collect_vec()
            },
            None => vec![],
        };
        if let Some(riff_set) = self.get_project().song_mut().riff_set_mut(riff_set_uuid) {

            if !riff_uuids.is_empty() {
                // get the current riff_ref for the track
                if let Some(riff_ref) = riff_set.get_riff_ref_for_track_mut(track_uuid.clone()) {
                    let mut count = 0;
                    let mut index_to_get = 0;
                    for riff_uuid in riff_uuids.iter() {
                        if riff_uuid.to_string() == *riff_ref.linked_to_mut() {
                            index_to_get = count + 1;
                            break;
                        }
                        count += 1;
                    }
                    if index_to_get >= riff_uuids.len() {
                        index_to_get = 0;
                    }
                    if let Some(riff_uuid) = riff_uuids.get(index_to_get) {
                        riff_ref.set_linked_to(riff_uuid.to_owned());
                    }
                }
                else {
                    // get the first riff uuid
                    if let Some(riff_uuid) = riff_uuids.get(0) {
                        // create a new riff_ref and add to the riff set
                        riff_set.set_riff_ref_for_track(track_uuid, RiffReference::new(riff_uuid.to_owned(), 0.0));
                    }
                }
            }
        }
    }

    pub fn riff_set_riff_for_track(&mut self, riff_set_uuid: String, track_uuid: String, riff_uuid: String) {
        debug!("state.riff_set_riff_for_track: {}, {}", riff_set_uuid.as_str(), track_uuid.as_str());
        if let Some(riff_set) = self.get_project().song_mut().riff_set_mut(riff_set_uuid) {
            // get the current riff_ref for the track
            if let Some(riff_ref) = riff_set.get_riff_ref_for_track_mut(track_uuid.clone()) {
                riff_ref.set_linked_to(riff_uuid);
            }
            else {
                riff_set.set_riff_ref_for_track(track_uuid, RiffReference::new(riff_uuid, 0.0));
            }
        }
    }

    pub fn set_jack_client(&mut self, jack_client: AsyncClient<JackNotificationHandler, Audio>) {
        self.jack_client.clear();
        self.jack_client.push(jack_client);
    }

    pub fn jack_client(&self) -> Option<&Client> {
        if let Some(async_jack_client) = self.jack_client.get(0) {
            Some(async_jack_client.as_client())
        }
        else {
            None
        }
    }

    pub fn start_jack(
        &mut self,
        rx_to_audio: crossbeam_channel::Receiver<AudioLayerInwardEvent>,
        jack_midi_sender: crossbeam_channel::Sender<AudioLayerOutwardEvent>,
        jack_midi_sender_ui: crossbeam_channel::Sender<AudioLayerOutwardEvent>,
        coast: Arc<Mutex<TrackBackgroundProcessorMode>>,
        vst_host_time_info: Arc<parking_lot::RwLock<TimeInfo>>,
    ) {
        let (jack_client, _status) =
            Client::new("DAW", ClientOptions::NO_START_SERVER).unwrap();
        let audio = Audio::new(&jack_client, rx_to_audio, jack_midi_sender.clone(), jack_midi_sender_ui.clone(), coast, vst_host_time_info);
        let notifications = JackNotificationHandler::new(jack_midi_sender_ui);
        let jack_async_client = jack_client.activate_async(notifications, audio).unwrap();

        // these should come from configuration and be selected from a menu and dialogue
        let _ = jack_async_client.as_client().connect_ports_by_name("DAW:out_l", "system:playback_1");
        let _ = jack_async_client.as_client().connect_ports_by_name("DAW:out_r", "system:playback_2");
        let _ = jack_async_client.as_client().connect_ports_by_name("a2j:Akai MPD24 [16] (capture): Akai MPD24 MIDI 1", "DAW:midi_control_in");
        let _ = jack_async_client.as_client().connect_ports_by_name("a2j:nanoPAD2 [20] (capture): nanoPAD2 MIDI 1", "DAW:midi_in");

        self.set_jack_client(jack_async_client);
    }

    pub fn restart_jack(&mut self,
                        rx_to_audio: crossbeam_channel::Receiver<AudioLayerInwardEvent>,
                        jack_midi_sender: crossbeam_channel::Sender<AudioLayerOutwardEvent>,
                        jack_midi_sender_ui: crossbeam_channel::Sender<AudioLayerOutwardEvent>,
                        coast: Arc<Mutex<TrackBackgroundProcessorMode>>,
                        vst_host_time_info: Arc<parking_lot::RwLock<TimeInfo>>,
    ) {
        if self.jack_client.len() == 1 {
            let async_client = self.jack_client.remove(0_usize);
            match async_client.deactivate() {
                Ok((_client, _notification_handler, mut process_handler)) => {
                    let consumers = process_handler.get_all_audio_consumers();
                    let (jack_client, _status) =
                        Client::new("DAW", ClientOptions::NO_START_SERVER).unwrap();
                    let audio = Audio::new_with_consumers(
                        &jack_client,
                        rx_to_audio,
                        jack_midi_sender.clone(),
                        jack_midi_sender_ui.clone(),
                        coast,
                        consumers,
                        vec![],
                        vst_host_time_info,
                    );
                    let notifications = JackNotificationHandler::new(jack_midi_sender_ui);
                    let jack_async_client = jack_client.activate_async(notifications, audio).unwrap();
                    for (from_name, to_name) in self.jack_connections.iter() {
                        let _ = jack_async_client.as_client().connect_ports_by_name(from_name.as_str(), to_name.as_str());
                    }
                    self.set_jack_client(jack_async_client);
                }
                Err(_) => {
                    self.start_jack(rx_to_audio, jack_midi_sender, jack_midi_sender_ui, coast, vst_host_time_info);
                }
            }
        }
        else {
            self.start_jack(rx_to_audio, jack_midi_sender, jack_midi_sender_ui, coast, vst_host_time_info);
        }
    }

    pub fn jack_connection_add(&mut self, from_name: String, to_name: String) {
        debug!("Jack connection added: from={}, to={}", from_name.as_str(), to_name.as_str());
        if let Some(jack_client) = self.jack_client.get(0) {
            let _ = jack_client.as_client().connect_ports_by_name(from_name.as_str(), to_name.as_str());
        }
        self.jack_connections.insert(from_name, to_name);
    }

    pub fn jack_midi_connection_add(&mut self, track_uuid: String, to_name: String) {
        debug!("Jack midi connection added: track={}, to={}", track_uuid.as_str(), to_name.as_str());
        if let Some(jack_client) = self.jack_client.get(0) {
            let _ = jack_client.as_client().connect_ports_by_name(format!("DAW:{}", track_uuid.as_str()).as_str(), to_name.as_str());
        }
        self.jack_connections.insert(track_uuid, to_name);
    }

    pub fn jack_midi_connection_remove(&mut self, track_uuid: String, to_name: String) {
        debug!("Jack midi connection removed: track={}, to={}", track_uuid.as_str(), to_name.as_str());
        if let Some(jack_client) = self.jack_client.get(0) {
            let _ = jack_client.as_client().disconnect_ports_by_name(format!("DAW:{}", track_uuid.as_str()).as_str(), to_name.as_str());
        }
        self.jack_connections.remove(&track_uuid);
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
                               tx_to_audio: crossbeam_channel::Sender<AudioLayerInwardEvent>,
                               track_audio_coast: Arc<Mutex<TrackBackgroundProcessorMode>>,
                               tx_from_ui: crossbeam_channel::Sender<DAWEvents>
    ) {
        let number_of_blocks = self.play_song(tx_to_audio);
        let track_render_audio_consumers = self.track_render_audio_consumers.clone();

        let _ = thread::Builder::new().name("Export wave file".into()).spawn(move || {
            match track_render_audio_consumers.lock() {
                Ok(track_render_audio_consumers) => if let Ok(mut export_wave_file) = std::fs::File::create(path) {
                    let number_of_audio_type_tracks = track_render_audio_consumers.len() as f32;
                    let mut master_left_channel_data: [f32; 1024] = [0.0; 1024];
                    let mut master_right_channel_data: [f32; 1024] = [0.0; 1024];
                    let mut sample_data = vec![];
                    let mut audio_blocks = vec![AudioBlock::default()];

                    for _block_number in 0..number_of_blocks {
                        // reset the master block
                        for index in 0..1024_usize {
                            master_left_channel_data[index] = 0.0;
                            master_right_channel_data[index] = 0.0;
                        }

                        for (_track_uuid, track_audio_consumer_details) in track_render_audio_consumers.iter() {
                            if let Some(blocks_read) = track_audio_consumer_details.consumer().read_blocking(&mut audio_blocks) {
                                // debug!("State.export_to_wave_file: track_uuid={}, channel=left, byes_read={}", track_uuid.as_str(), left_bytes_read);
                                // copy the track channel data to the the master channels
                                if blocks_read == 1 {
                                    let audio_block = audio_blocks.get(0).unwrap();
                                    for index in 0..1024_usize as usize {
                                        master_left_channel_data[index] += audio_block.audio_data_left[index] / number_of_audio_type_tracks;
                                    }
                                    for index in 0..1024_usize as usize {
                                        master_right_channel_data[index] += audio_block.audio_data_right[index] / number_of_audio_type_tracks;
                                    }
                                }
                            }
                        }

                        // write the master block out
                        for index in 0..1024_usize {
                            sample_data.push(master_left_channel_data[index]);
                            sample_data.push(master_right_channel_data[index]);
                        }
                    }

                    // write the file
                    let wav_header = wav_io::new_header(44100, 32, true, false);
                    let _ = wav_io::write_to_file(&mut export_wave_file, &wav_header, &sample_data);
                }
                Err(_) => {}
            }

            if let Ok(mut coast) = track_audio_coast.lock() {
                *coast = TrackBackgroundProcessorMode::AudioOut;
            }

            let _ = tx_from_ui.send(DAWEvents::HideProgressDialogue);
        });
    }

    pub fn export_to_midi_file(&self, path: std::path::PathBuf) -> bool {
        if let Some(absolute_path) = path.to_str() {
            let mut midi = MIDI::new();
            let parts_per_quarter_note = midi.get_ppqn();
            let bpm = self.project().song().tempo();
            let microseconds_per_beat = (1.0 / bpm * 60.0 * 1000000.0) as u32;

            // set the tempo
            midi.insert_event(0, 0, apres::MIDIEvent::SetTempo(microseconds_per_beat));

            let mut track_index: usize = 1;
            for track in self.project().song().tracks().iter() {
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

                                    single_track_events.push(TrackEvent::NoteOn(NoteOn::new_with_params(start_position_in_beats, note.note(), note.velocity())));
                                    single_track_events.push(TrackEvent::NoteOff(NoteOff::new_with_params(end_position_in_beats, note.note(), 0)));
                                }
                                _ => (),
                            }
                        }
                    }
                }
                for event in track.automation().events().iter() {
                    match event {
                        TrackEvent::Controller(controller) => {
                            single_track_events.push(TrackEvent::Controller(controller.clone())) ;
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
            true
        }
        else {
            false
        }
    }

    pub fn export_riffs_to_midi_file(&self, path: std::path::PathBuf) -> bool {
        if let Some(absolute_path) = path.to_str() {
            let bpm = self.project().song().tempo();
            let mut midi = MIDI::new();
            let parts_per_quarter_note = midi.get_ppqn();
            let microseconds_per_beat = (1.0 / bpm * 60.0 * 1000000.0) as u32;
            let mut track_number: usize = 0;

            midi.insert_event(track_number, 0, apres::MIDIEvent::SetTempo(microseconds_per_beat));
            midi.insert_event(track_number, 1, apres::MIDIEvent::EndOfTrack);
            track_number += 1;

            for track in self.project().song().tracks() {
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

                                single_track_events.push(TrackEvent::NoteOn(NoteOn::new_with_params(start_position_in_beats, note.note(), note.velocity())));
                                single_track_events.push(TrackEvent::NoteOff(NoteOff::new_with_params(end_position_in_beats, note.note(), 0)));
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

            true
        }
        else {
            false
        }
    }

    pub fn export_riffs_to_separate_midi_files(&self, path: std::path::PathBuf) -> bool {
        if let Some(dir_path) = path.to_str() {
            let bpm = self.project().song().tempo();

            let mut track_number: usize = 1;
            for track in self.project().song().tracks() {
                for riff in track.riffs() {
                    let mut single_track_events: Vec<TrackEvent> = vec![];
                    let mut absolute_path_buffer = PathBuf::from(dir_path);
                    let mut midi_file_name = if track_number < 10 {
                        format!("0{}", track_number)
                    }
                    else {
                        format!("{}", track_number)
                    };

                    midi_file_name.push('_');
                    midi_file_name.push_str(self.project().song().name());
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

                                single_track_events.push(TrackEvent::NoteOn(NoteOn::new_with_params(start_position_in_beats, note.note(), note.velocity())));
                                single_track_events.push(TrackEvent::NoteOff(NoteOff::new_with_params(end_position_in_beats, note.note(), 0)));
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
            true
        }
        else {
            false
        }
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
    pub fn vst_instrument_plugins(&self) -> &IndexMap<String, String> {
        &self.vst_instrument_plugins
    }
    pub fn vst_instrument_plugins_mut(&mut self) -> &mut IndexMap<String, String> {
        &mut self.vst_instrument_plugins
    }
    pub fn vst_effect_plugins(&self) -> &IndexMap<String, String> {
        &self.vst_effect_plugins
    }
    pub fn vst_effect_plugins_mut(&mut self) -> &mut IndexMap<String, String> {
        &mut self.vst_effect_plugins
    }
    pub fn track_grid_cursor_follow(&self) -> bool {
        self.track_grid_cursor_follow
    }
    pub fn track_grid_cursor_follow_mut(&mut self) -> bool {
        self.track_grid_cursor_follow
    }
    pub fn set_track_grid_cursor_follow(&mut self, track_grid_cursor_follow: bool) {
        self.track_grid_cursor_follow = track_grid_cursor_follow;
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
        self.selected_riff_arrangement_uuid.as_ref()
    }

    pub fn selected_riff_arrangement_uuid_mut(&mut self) -> &mut Option<String> {
        &mut self.selected_riff_arrangement_uuid
    }

    pub fn set_selected_riff_arrangement_uuid(&mut self, selected_riff_arrangement_uuid: Option<String>) {
        self.selected_riff_arrangement_uuid = selected_riff_arrangement_uuid;
    }

    pub fn get_automation_for_current_view(&self) -> Option<&Automation> {
        if let Some(track_uuid) = self.selected_track() {
            if let Some(track) = self.project().song().tracks().iter().find(|track| track.uuid().to_string() == track_uuid) {
                match self.current_view() {
                    crate::event::CurrentView::RiffArrangement => {
                        if let Some(riff_arrangement_uuid) = self.selected_riff_arrangement_uuid() {
                            if let Some(riff_arrangement) = self.project().song().riff_arrangement(riff_arrangement_uuid.clone()) {
                                riff_arrangement.automation(&track_uuid)
                            }
                            else {
                                Some(track.automation())
                            }
                        }
                        else {
                            Some(track.automation())
                        }
                    },
                    _ => Some(track.automation()),
                }
            }
            else {
                None
            }
        }
        else {
            None
        }
    }

    pub fn get_automation_for_current_view_mut(&mut self) -> Option<&mut Automation> {
        let current_view = self.current_view().clone();
        let selected_riff_arrangement_uuid = if let Some(selected_riff_arrangement_uuid) = self.selected_riff_arrangement_uuid() {
            selected_riff_arrangement_uuid.clone()
        }
        else {
            "".to_string()
        };
        let selected_track_uuid = self.selected_track().clone();

        if let Some(track_uuid) = selected_track_uuid {
            match current_view {
                crate::event::CurrentView::RiffArrangement => {
                    if let Some(riff_arrangement) = self.get_project().song_mut().riff_arrangement_mut(selected_riff_arrangement_uuid) {
                        riff_arrangement.automation_mut(&track_uuid)
                    }
                    else {
                        None
                    }
                },
                _ => if let Some(track) = self.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                    Some(track.automation_mut())
                }
                else {
                    None
                }
            }
        }
        else {
            None
        }
    }

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
        self.automation_edit_type.clone()
    }

    pub fn automation_edit_type_mut(&mut self) -> &mut AutomationEditType {
        &mut self.automation_edit_type
    }

    pub fn set_automation_edit_type(&mut self, automation_edit_type: AutomationEditType) {
        self.automation_edit_type = automation_edit_type;
    }

    pub fn selected_automation(&self) -> &[String] {
        self.selected_automation.as_ref()
    }

    pub fn selected_automation_mut(&mut self) -> &mut Vec<String> {
        &mut self.selected_automation
    }

    pub fn automation_event_copy_buffer(&self) -> &[TrackEvent] {
        self.automation_event_copy_buffer.as_ref()
    }

    pub fn automation_event_copy_buffer_mut(&mut self) -> &mut Vec<TrackEvent> {
        &mut self.automation_event_copy_buffer
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

    pub fn playing_riff_arrangement_summary_data(&self) -> &Option<(f64, Vec<(f64, RiffItem, Vec<(f64, RiffItem)>)>)> {
        &self.playing_riff_arrangement_summary_data
    }

    pub fn riff_arrangement_riff_item_selected_uuid(&self) -> &Option<(String, String)> {
        &self.riff_arrangement_riff_item_selected_uuid
    }

    pub fn riff_sequence_riff_set_reference_selected_uuid(&self) -> &Option<(String, String)> {
        &self.riff_sequence_riff_set_reference_selected_uuid
    }

    pub fn riff_set_selected_uuid(&self) -> &Option<String> {
        &self.riff_set_selected_uuid
    }

    pub fn set_riff_arrangement_riff_item_selected_uuid(&mut self, riff_arrangement_riff_item_selected_uuid: Option<(String, String)>) {
        self.riff_arrangement_riff_item_selected_uuid = riff_arrangement_riff_item_selected_uuid;
    }

    pub fn set_riff_sequence_riff_set_reference_selected_uuid(&mut self, riff_sequence_riff_set_reference_selected_uuid: Option<(String, String)>) {
        self.riff_sequence_riff_set_reference_selected_uuid = riff_sequence_riff_set_reference_selected_uuid;
    }

    pub fn set_riff_set_selected_uuid(&mut self, riff_set_selected_uuid: Option<String>) {
        self.riff_set_selected_uuid = riff_set_selected_uuid;
    }
}



#[cfg(test)]
mod tests {
    use crate::DAWState;

    extern crate factor;

    #[test]
    fn get_length_product() {
        let riff_lengths = vec![1, 1, 2, 2, 3, 3, 5, 5];
        let (product, unique_lengths) = DAWState::get_length_product(riff_lengths);

        assert_eq!(30, product);

        let found_length = DAWState::get_lowest_common_factor(unique_lengths, product);

        assert_eq!(30, found_length);
    }

    #[test]
    fn get_length_product_2() {
        let riff_lengths = vec![4, 8, 12, 16, 24];
        let (product, unique_lengths) = DAWState::get_length_product(riff_lengths);

        assert_eq!(147456, product);

        let found_length = DAWState::get_lowest_common_factor(unique_lengths, product);

        assert_eq!(48, found_length);
    }
}