use std::{collections::HashMap, default::Default, sync::{Arc, Mutex}, time::Duration};
use std::thread;

use apres::MIDI;
use constants::{TRACK_VIEW_TRACK_PANEL_HEIGHT, LUA_GLOBAL_STATE, VST_PATH_ENVIRONMENT_VARIABLE_NAME, CLAP_PATH_ENVIRONMENT_VARIABLE_NAME, DAW_AUTO_SAVE_THREAD_NAME};
use crossbeam_channel::{Receiver, Sender, unbounded};
use flexi_logger::{LogSpecification, Logger};
use gtk::{Adjustment, ButtonsType, ComboBoxText, DrawingArea, Frame, glib, MessageDialog, MessageType, prelude::{ActionMapExt, AdjustmentExt, ApplicationExt, Cast, ComboBoxExtManual, ComboBoxTextExt, ContainerExt, DialogExt, EntryExt, GtkWindowExt, LabelExt, ProgressBarExt, ScrolledWindowExt, SpinButtonExt, TextBufferExt, TextViewExt, ToggleToolButtonExt, WidgetExt}, SpinButton, Window, WindowType};
use indexmap::IndexMap;
use itertools::Itertools;
use jack::MidiOut;
use log::*;
use mlua::{Lua, MultiValue, Value};
use parking_lot::RwLock;
use simple_clap_host_helper_lib::plugin::library::PluginLibrary;
use thread_priority::{ThreadBuilder, ThreadPriority};
use uuid::Uuid;
use vst::host::PluginLoader;
use vst::api::TimeInfo;

use audio::JackNotificationHandler;
use audio_plugin_util::*;
use domain::*;
use event::*;
use history::*;
use lua_api::*;
use state::*;
use ui::*;

use crate::{grid::Grid, utils::DAWUtils};
use crate::audio::Audio;
use crate::constants::EVENT_DELETION_BEAT_TOLERANCE;

mod constants;
mod domain;
mod ui;
mod state;
mod event;
mod audio;
mod grid;
mod utils;
mod audio_plugin_util;
mod history;
mod lua_api;

#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

extern {
    fn gdk_x11_window_get_xid(window: gdk::Window) -> u32;
}

fn main() {
    // transport
    let transport = Transport {
        playing: false,
        bpm: 140.0,
        sample_rate: 44100.0,
        block_size: 1024.0,
        position_in_beats: 0.0,
        position_in_frames: 0,
    };
    TRANSPORT.set(RwLock::new(transport));

    // recorded notes
    let mut recorded_playing_notes: HashMap<i32, f64> = HashMap::new();

    // setup history
    let mut history_manager = Arc::new(Mutex::new(HistoryManager::new()));

    // setup logging
    let logger_init_result = Logger::try_with_env();
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

    // VST timing
    let vst_host_time_info = Arc::new(parking_lot::RwLock::new(TimeInfo {
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

    let (tx_from_ui, rx_from_ui) = unbounded::<DAWEvents>();
    let (tx_to_audio, rx_to_audio) = unbounded::<AudioLayerInwardEvent>();
    let (jack_midi_sender_ui, jack_midi_receiver_ui) = unbounded::<AudioLayerOutwardEvent>();
    let (jack_midi_sender, jack_midi_receiver) = unbounded::<AudioLayerOutwardEvent>();

    let state = {
        let tx_from_ui = tx_from_ui.clone();
        Arc::new(Mutex::new (DAWState::new(tx_from_ui)))
    };

    let mut audio_plugin_windows: HashMap<String, Window> = HashMap::new();

    let lua = Lua::new();
    let _ = lua.globals().set(LUA_GLOBAL_STATE, LuaState {state: state.clone(), tx_from_ui: tx_from_ui.clone()});

    gtk::init().expect("Problem starting up GTK3.");

    let mut gui = {
        let tx_from_ui = tx_from_ui.clone();
        let state = state.clone();
        MainWindow::new(tx_from_ui, state)
    };

    if let Some(application) = gui.ui.wnd_main.application() {
        application.connect_startup(build_ui);
    }

    {
        let tx_from_ui = tx_from_ui.clone();
        gui.start(tx_from_ui);
    }

    let vst24_plugin_loaders: Arc<Mutex<HashMap<String, PluginLoader<VstHost>>>> = Arc::new(Mutex::new(HashMap::new()));
    let clap_plugin_loaders: Arc<Mutex<HashMap<String, PluginLibrary>>> = Arc::new(Mutex::new(HashMap::new()));
    let track_audio_coast = Arc::new(Mutex::new(TrackBackgroundProcessorMode::AudioOut));
    let jack_audio_coast = track_audio_coast.clone();

    let autosave_keep_alive = Arc::new(Mutex::new(true));

    set_up_initial_project_in_ui(&tx_to_audio, &track_audio_coast, &mut gui, tx_from_ui.clone(), state.clone(), vst_host_time_info.clone());

    // scan for audio plugins
    {
        if let Ok(vst_path) = std::env::var(VST_PATH_ENVIRONMENT_VARIABLE_NAME) {
            if let Ok(clap_path) = std::env::var(CLAP_PATH_ENVIRONMENT_VARIABLE_NAME) {
                match state.lock() {
                    Ok(mut state) => {
                        if state.configuration.scanned_vst_instrument_plugins.successfully_scanned.is_empty() && state.configuration.scanned_vst_effect_plugins.successfully_scanned.is_empty() {
                            let (instruments, effects) = scan_for_audio_plugins(vst_path.clone(), clap_path.clone());
                            for (key, value) in instruments.iter() {
                                state.vst_instrument_plugins_mut().insert(key.to_string(), value.to_string());
                                state.configuration.scanned_vst_instrument_plugins.successfully_scanned.insert(key.to_string(), value.to_string());
                            }
                            state.vst_instrument_plugins_mut().sort_by(|_key1, value1: &String, _key2, value2: &String| value1.cmp(value2));

                            for (key, value) in effects.iter() {
                                state.vst_effect_plugins_mut().insert(key.to_string(), value.to_string());
                                state.configuration.scanned_vst_effect_plugins.successfully_scanned.insert(key.to_string(), value.to_string());
                            }
                            state.vst_effect_plugins_mut().sort_by(|_key1, value1: &String, _key2, value2: &String| value1.cmp(value2));

                            state.configuration.save();
                        }
                        else {
                            let mut intermediate_map = HashMap::new();
                            for (key, value) in state.configuration.scanned_vst_instrument_plugins.successfully_scanned.iter() {
                                intermediate_map.insert(key.to_string(), value.to_string());
                            }
                            for (key, value) in intermediate_map.iter() {
                                state.vst_instrument_plugins_mut().insert(key.to_string(), value.to_string());
                            }
                            state.vst_instrument_plugins_mut().sort_by(|_key1, value1: &String, _key2, value2: &String| value1.cmp(value2));

                            intermediate_map.clear();
                            for (key, value) in state.configuration.scanned_vst_effect_plugins.successfully_scanned.iter() {
                                intermediate_map.insert(key.to_string(), value.to_string());
                            }
                            for (key, value) in intermediate_map.iter() {
                                state.vst_effect_plugins_mut().insert(key.to_string(), value.to_string());
                            }
                            state.vst_effect_plugins_mut().sort_by(|_key1, value1: &String, _key2, value2: &String| value1.cmp(value2));
                        }

                        gui.update_available_audio_plugins_in_ui(state.vst_instrument_plugins(), state.vst_effect_plugins());
                    }
                    Err(_) => {}
                }
            }
        }
    }

    {
        let state = state.clone();
        let autosave_keep_alive = autosave_keep_alive.clone();
        let _ = std::thread::Builder::new().name(DAW_AUTO_SAVE_THREAD_NAME.to_string()).spawn(move || {
            loop {
                if let Ok(mut state) = state.lock() {
                    if !state.playing() {
                        state.autosave();
                    }
                }
                std::thread::sleep(Duration::from_secs(300));
                if let Ok(keep_alive) = autosave_keep_alive.lock() {
                    if !*keep_alive {
                        break;
                    }
                }
            }
        });
    }

    // handle incoming events in the gui thread - lots of ui interaction
    {
        let mut state = state.clone();
        let mut delay_count = 0;
        let mut progress_bar_pulse_delay_count = 0;
        let rx_to_audio = rx_to_audio.clone();
        let jack_midi_sender = jack_midi_sender.clone();
        let jack_midi_sender_ui = jack_midi_sender_ui.clone();
        let vst_host_time_info = vst_host_time_info.clone();
        let tx_from_ui = tx_from_ui.clone();
        let jack_midi_receiver = jack_midi_receiver_ui.clone();
        let tx_to_audio = tx_to_audio.clone();


        glib::idle_add_local(move || {
            process_jack_events(
                &tx_from_ui,
                &jack_midi_receiver,
                &mut state,
                &tx_to_audio,
                &rx_to_audio,
                &jack_midi_sender,
                &jack_midi_sender_ui,
                &track_audio_coast,
                &mut gui,
                &vst_host_time_info,
                &mut recorded_playing_notes,
            );
            process_track_background_processor_events(
                &mut audio_plugin_windows,
                &mut state,
                &mut gui,
            );
            do_progress_dialogue_pulse(&mut gui, &mut progress_bar_pulse_delay_count);

            if delay_count > 1000 {
                delay_count = 0;
                process_application_events(
                    &mut history_manager, 
                    tx_from_ui.clone(),
                    &mut audio_plugin_windows,
                    &lua,
                    &mut gui,
                    vst24_plugin_loaders.clone(),
                    clap_plugin_loaders.clone(),
                    track_audio_coast.clone(),
                    rx_from_ui.clone(),
                    &mut state,
                    tx_to_audio.clone(),
                    vst_host_time_info.clone(),
                );
            }
            else {
                delay_count += 1;
            }

            glib::Continue(true)
        });
    }

    create_jack_event_processing_thread(
        tx_from_ui.clone(),
        jack_midi_receiver.clone(), 
        state.clone(), 
    );

    // kick off the audio layer
    {
        let rx_to_audio = rx_to_audio;
        let jack_midi_sender_ = jack_midi_sender.clone();
        let jack_midi_sender_ui = jack_midi_sender_ui;
        let jack_audio_coast = jack_audio_coast;

        match state.lock() {
            Ok(mut state) => {
                state.start_jack(rx_to_audio, jack_midi_sender, jack_midi_sender_ui, jack_audio_coast, vst_host_time_info);
            }
            Err(_) => {}
        }
    }

    gtk::main();


    match state.lock() {
        Ok(state) => {
            state.configuration.save();
        }
        Err(_) => {}
    };
}

pub fn build_ui(application: &gtk::Application) {
    let test_action = gio::SimpleAction::new("test", None);
    test_action.connect_activate(move |_, _| {
        debug!("%%%%%%%%%%%%%%%%%%%%%% Test action executed!");
    });
    application.add_action(&test_action);
    let quit_action = gio::SimpleAction::new("quit", None);
    quit_action.connect_activate(move |_, _| {
        debug!("%%%%%%%%%%%%%%%%%%%%%% Quit action executed!");
    });
    application.add_action(&quit_action);
}

fn set_up_initial_project_in_ui(tx_to_audio: &Sender<AudioLayerInwardEvent>,
                                track_audio_coast: &Arc<Mutex<TrackBackgroundProcessorMode>>,
                                gui_ref: &mut MainWindow,
                                tx_from_ui: Sender<DAWEvents>,
                                state_arc: Arc<Mutex<DAWState>>,
                                vst_host_time_info: Arc<parking_lot::RwLock<TimeInfo>>,
) {
    match state_arc.lock() {
        Ok(state) => {
            let mut state = state;
            let mut track_uuid = None;
            match state.get_project().song_mut().tracks_mut().last().unwrap() {
                TrackType::InstrumentTrack(track) => {
                    debug!("Adding a track to GUI...");
                    track_uuid = Some(track.uuid().to_string());
                    gui_ref.add_track(track.name(),
                                      track.uuid(),
                                      tx_from_ui,
                                      state_arc.clone(),
                                      GeneralTrackType::InstrumentTrack,
                                      None,
                                      1.0,
                                      0.0,
                                      false,
                                      false);
                    debug!("Added a track to the GUI.");
                },
                TrackType::AudioTrack(_) => (),
                TrackType::MidiTrack(_) => (),
            }
            if let Some(uuid) = track_uuid {
                state.start_default_track_background_processing(tx_to_audio.clone(), track_audio_coast.clone(), uuid, vst_host_time_info);
            }
            else {
                error!("Main - rx_ui run once - Initial track added to GUI - could not start track background processing.")
            }
        },
        Err(_) => error!("Main - rx_ui run once - Track Added to GUI - could not get lock on state"),
    };
}

fn process_application_events(history_manager: &mut Arc<Mutex<HistoryManager>>,
                              tx_from_ui: Sender<DAWEvents>,
                              audio_plugin_windows: &mut HashMap<String, Window>,
                              lua: &Lua,
                              gui: &mut MainWindow,
                              vst24_plugin_loaders: Arc<Mutex<HashMap<String, PluginLoader<VstHost>>>>,
                              clap_plugin_loaders: Arc<Mutex<HashMap<String, PluginLibrary>>>,
                              track_audio_coast: Arc<Mutex<TrackBackgroundProcessorMode>>,
                              rx_from_ui: Receiver<DAWEvents>,
                              state: &mut Arc<Mutex<DAWState>>,
                              tx_to_audio: Sender<AudioLayerInwardEvent>,
                              vst_host_time_info: Arc<parking_lot::RwLock<TimeInfo>>,
) {
    match rx_from_ui.try_recv() {
        Ok(event) => match event {
            DAWEvents::NewFile => {
                gui.clear_ui();
                // history.clear();
                let state_arc = state.clone();
                match state.lock() {
                    Ok(state) => {
                        let mut state = state;
                        if let Ok(mut track_render_audio_consumers) = state.track_render_audio_consumers_mut().lock() {
                            track_render_audio_consumers.clear();
                        }
                        // need to kill audio threads for tracks in the current file
                        let current_track_uuids = state.get_project().song_mut().tracks_mut().iter_mut().map(|track| {
                            track.uuid().to_string()
                        }).collect::<Vec<String>>();

                        for current_track_uuid in current_track_uuids.iter() {
                            // kill the vst thread
                            state.send_to_track_background_processor(current_track_uuid.to_string(), TrackBackgroundProcessorInwardEvent::Kill);

                            // remove the consumer from the audio layer
                            match tx_to_audio.send(AudioLayerInwardEvent::RemoveTrack(current_track_uuid.to_string())) {
                                Ok(_) => (),
                                Err(error) => debug!("Problem using tx_to_audio to send remove track consumer message to jack layer: {}", error),
                            }
                        }

                        let project = Project::new();

                        {
                            let mut time_info =  vst_host_time_info.write();
                            time_info.tempo = project.song().tempo();
                            time_info.sample_rate = project.song().sample_rate();
                            time_info.sample_pos = 0.0;
                        }

                        state.set_project(project);
                        state.set_current_file_path(None);
                        let mut instrument_track_senders2 = HashMap::new();
                        let mut instrument_track_receivers2 = HashMap::new();
                        let mut sample_references = HashMap::new();
                        let mut samples_data = HashMap::new();
                        for track in state.get_project().song_mut().tracks_mut().iter_mut() {
                            DAWState::init_track(
                                vst24_plugin_loaders.clone(),
                                clap_plugin_loaders.clone(),
                                tx_to_audio.clone(),
                                track_audio_coast.clone(),
                                &mut instrument_track_senders2,
                                &mut instrument_track_receivers2,
                                track,
                                Some(&sample_references),
                                Some(&samples_data),
                                vst_host_time_info.clone(),
                            );
                        }
                        state.update_track_senders_and_receivers(instrument_track_senders2, instrument_track_receivers2);

                        gui.update_ui_from_state(tx_from_ui, &mut state, state_arc);
                        match tx_to_audio.send(AudioLayerInwardEvent::BlockSize(state.project().song().block_size())) {
                            Ok(_) => (),
                            Err(error) => debug!("Problem using tx_to_audio to send block size message to jack layer: {}", error),
                        }
                        match tx_to_audio.send(AudioLayerInwardEvent::Tempo(state.project().song().tempo())) {
                            Ok(_) => (),
                            Err(error) => debug!("Problem using tx_to_audio to send tempo message to jack layer: {}", error),
                        }
                        match tx_to_audio.send(AudioLayerInwardEvent::SampleRate(state.project().song().sample_rate())) {
                            Ok(_) => (),
                            Err(error) => debug!("Problem using tx_to_audio to send sample rate message to jack layer: {}", error),
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - New File - could not get lock on state"),
                }
            },
            DAWEvents::OpenFile(path) => {
                gui.clear_ui();
                gui.ui.dialogue_progress_bar.set_text(Some(format!("Loading {}...", path.to_str().unwrap()).as_str()));
                gui.ui.progress_dialogue.set_title("Open");
                gui.ui.progress_dialogue.show_all();

                let state_arc = state.clone();
                let state = state_arc;
                let track_audio_coast = track_audio_coast;
                let tx_to_audio = tx_to_audio;
                let vst24_plugin_loaders = vst24_plugin_loaders;
                let tx_from_ui = tx_from_ui;
                let _ = thread::Builder::new().name("Open file".into()).spawn(move || {
                    if let Ok(mut coast) = track_audio_coast.lock() {
                        *coast = TrackBackgroundProcessorMode::Coast;
                    }
                    thread::sleep(Duration::from_millis(1000));
                    // history.clear();
                    let mut midi_tracks = HashMap::new();
                    let state_arc2 = state.clone();
                    match state.lock() {
                        Ok(mut state) => {
                            if let Ok(mut track_render_audio_consumers) = state.track_render_audio_consumers_mut().lock() {
                                track_render_audio_consumers.clear();
                            }
                            // need to kill audio threads for tracks in the current file
                            let current_track_uuids = state.get_project().song_mut().tracks_mut().iter_mut().map(|track| {
                                track.uuid().to_string()
                            }).collect::<Vec<String>>();

                            for current_track_uuid in current_track_uuids.iter() {
                                // kill the vst thread
                                state.send_to_track_background_processor(current_track_uuid.to_string(), TrackBackgroundProcessorInwardEvent::Kill);

                                // remove the consumer from the audio layer
                                match tx_to_audio.send(AudioLayerInwardEvent::RemoveTrack(current_track_uuid.to_string())) {
                                    Ok(_) => (),
                                    Err(error) => debug!("Problem using tx_to_audio to send remove track consumer message to jack layer: {}", error),
                                }
                            }

                            state.load_from_file(
                                vst24_plugin_loaders.clone(), clap_plugin_loaders.clone(), path.to_str().unwrap(), tx_to_audio.clone(), track_audio_coast.clone(), vst_host_time_info.clone());

                            let tempo = state.project().song().tempo();

                            {
                                let mut time_info = vst_host_time_info.write();
                                time_info.tempo = tempo;
                                time_info.sample_rate = state.project().song().sample_rate();
                                time_info.sample_pos = 0.0;
                            }

                            let mut audio_track_ids = vec![];
                            for track in state.get_project().song_mut().tracks_mut() {
                                match track {
                                    TrackType::MidiTrack(track) => {
                                        midi_tracks.insert(track.uuid().to_string(), track.name().to_string());
                                    }
                                    _ => {
                                        audio_track_ids.push(track.uuid().to_string());
                                    }
                                }
                            }
                            for track_id in audio_track_ids.iter() {
                                state.send_to_track_background_processor(track_id.clone(), TrackBackgroundProcessorInwardEvent::Tempo(tempo));
                            }

                            match tx_to_audio.send(AudioLayerInwardEvent::BlockSize(state.project().song().block_size())) {
                                Ok(_) => (),
                                Err(error) => debug!("Problem using tx_to_audio to send block size message to jack layer: {}", error),
                            }
                            match tx_to_audio.send(AudioLayerInwardEvent::Tempo(state.project().song().tempo())) {
                                Ok(_) => (),
                                Err(error) => debug!("Problem using tx_to_audio to send block size message to jack layer: {}", error),
                            }
                            match tx_to_audio.send(AudioLayerInwardEvent::SampleRate(state.project().song().sample_rate())) {
                                Ok(_) => (),
                                Err(error) => debug!("Problem using tx_to_audio to send block size message to jack layer: {}", error),
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - Open File - could not get lock on state"),
                    }

                    match state_arc2.lock() {
                        Ok(state) => {
                            // add midi track ports
                            for (track_uuid, _) in midi_tracks {
                                if let Some(jack_client) = state.jack_client() {
                                    if let Ok(midi_out_port) = jack_client.register_port(track_uuid.as_str(), MidiOut::default()) {
                                        match tx_to_audio.send(AudioLayerInwardEvent::NewMidiOutPortForTrack(track_uuid.clone(), midi_out_port)) {
                                            Ok(_) => (),
                                            Err(error) => debug!("Problem using tx_to_audio to send new midi out port message to jack layer: {}", error),
                                        }
                                    }
                                }
                            }
                        }
                        Err(_) => {}
                    }

                    if let Ok(mut coast) = track_audio_coast.lock() {
                        *coast = TrackBackgroundProcessorMode::AudioOut;
                    }

                    let _ = tx_from_ui.send(DAWEvents::UpdateUI);
                    let _ = tx_from_ui.send(DAWEvents::HideProgressDialogue);
                });
            },
            DAWEvents::Save => {
                gui.ui.dialogue_progress_bar.set_text(Some("Saving..."));
                gui.ui.progress_dialogue.set_title("Save");
                gui.ui.progress_dialogue.show_all();

                {
                    let state = state.clone();
                    let track_audio_coast = track_audio_coast;
                    let tx_from_ui = tx_from_ui;
                    let _ = thread::Builder::new().name("Save".into()).spawn(move || {
                        if let Ok(mut coast) = track_audio_coast.lock() {
                            *coast = TrackBackgroundProcessorMode::Coast;
                        }
                        thread::sleep(Duration::from_millis(1000));
                        match state.lock() {
                            Ok(state) => {
                                debug!("main - DAWEvents::Save - number of riff sequences={}", state.project().song().riff_sequences().len());
                                let mut state = state;
                                state.save();
                            },
                            Err(_) => debug!("Main - rx_ui processing loop - Save File - could not get lock on state"),
                        }
                        if let Ok(mut coast) = track_audio_coast.lock() {
                            *coast = TrackBackgroundProcessorMode::AudioOut;
                        }

                        let _ = tx_from_ui.send(DAWEvents::HideProgressDialogue);
                    });
                }
            },
            DAWEvents::SaveAs(path) => {
                gui.ui.dialogue_progress_bar.set_text(Some(format!("Saving as {}...", path.to_str().unwrap()).as_str()));
                gui.ui.progress_dialogue.set_title("Save As");
                gui.ui.progress_dialogue.show_all();

                {
                    let state = state.clone();
                    let track_audio_coast = track_audio_coast;
                    let tx_from_ui = tx_from_ui;
                    let _ = thread::Builder::new().name("Save as".into()).spawn(move || {
                        if let Ok(mut coast) = track_audio_coast.lock() {
                            *coast = TrackBackgroundProcessorMode::Coast;
                        }
                        match state.lock() {
                            Ok(state) => {
                                let mut state = state;
                                state.save_as(path.to_str().unwrap());
                            },
                            Err(_) => debug!("Main - rx_ui processing loop - Save As File - could not get lock on state"),
                        }
                        if let Ok(mut coast) = track_audio_coast.lock() {
                            *coast = TrackBackgroundProcessorMode::AudioOut;
                        }

                        let _ = tx_from_ui.send(DAWEvents::HideProgressDialogue);
                    });
                }
            },
            DAWEvents::ImportMidiFile(path) => {
                gui.clear_ui();
                gui.ui.dialogue_progress_bar.set_text(Some(format!("Importing midi file {}...", path.to_str().unwrap()).as_str()));
                gui.ui.progress_dialogue.set_title("Import Midi File");
                gui.ui.progress_dialogue.show_all();

                {
                    let state = state.clone();
                    let track_audio_coast = track_audio_coast;
                    let tx_from_ui = tx_from_ui;
                    let _ = thread::Builder::new().name("Import midi file".into()).spawn(move || {
                        if let Ok(mut coast) = track_audio_coast.lock() {
                            *coast = TrackBackgroundProcessorMode::Coast;
                        }
                        match state.lock() {
                            Ok(state) => {
                                let mut state = state;
                                let tracks = state.get_project().song_mut().tracks_mut();

                                if let Some(file) = path.to_str() {
                                    match MIDI::from_path(file) {
                                        Ok(midi) => {
                                            let mut track_number = 1;
                                            let mut tempo: u32 = 0;
                                            let ppq = midi.get_ppqn();
                                            let mut instrument_track_senders2 = HashMap::new();
                                            let mut instrument_track_receivers2 = HashMap::new();

                                            for track in midi.get_tracks().iter() {
                                                debug!("Track: {}", track_number);
                                                let mut freedom_daw_track = InstrumentTrack::new();
                                                let mut current_notes = HashMap::new();
                                                let riff = Riff::new_with_name_and_length(Uuid::new_v4(), "unknown".to_owned(), 4.0);
                                                let riff_ref = RiffReference::new(riff.uuid().to_string(), 0.0);

                                                freedom_daw_track.riffs_mut().push(riff);
                                                freedom_daw_track.riff_refs_mut().push(riff_ref);

                                                let riff = freedom_daw_track.riffs_mut().get_mut(1).unwrap();
                                                let mut track_name = "".to_owned();

                                                for (_, event_id) in track.iter() {
                                                    let position = midi.get_event_position(*event_id);
                                                    match midi.get_event(*event_id) {
                                                        Some(event) => {
                                                            debug!("Found event: {:?}", event);
                                                            match event {
                                                                apres::MIDIEvent::SequenceNumber(_) => (),
                                                                apres::MIDIEvent::Text(_) => (),
                                                                apres::MIDIEvent::CopyRightNotice(_) => (),
                                                                apres::MIDIEvent::TrackName(name) => track_name.push_str(name.as_str().trim_matches(char::from(0))),
                                                                apres::MIDIEvent::InstrumentName(_) => (),
                                                                apres::MIDIEvent::Lyric(_) => (),
                                                                apres::MIDIEvent::Marker(_) => (),
                                                                apres::MIDIEvent::CuePoint(_) => (),
                                                                apres::MIDIEvent::ChannelPrefix(_) => (),
                                                                apres::MIDIEvent::SetTempo(tempo_value) => {
                                                                    tempo = tempo_value;
                                                                    debug!("Tempo: {}", tempo);
                                                                },
                                                                apres::MIDIEvent::SMPTEOffset(_, _, _, _, _) => (),
                                                                apres::MIDIEvent::TimeSignature(_, _, _, _) => (),
                                                                apres::MIDIEvent::KeySignature(_) => (),
                                                                apres::MIDIEvent::SequencerSpecific(_) => (),
                                                                apres::MIDIEvent::NoteOn(_, note, velocity) => {
                                                                    if let Some((_, ticks)) = position {
                                                                        let position_in_beats = *ticks as f64 / ppq as f64;
                                                                        let new_note = Note::new_with_params(position_in_beats, note as i32, velocity as i32, 0.0);
                                                                        current_notes.insert(note, new_note);
                                                                    }
                                                                },
                                                                apres::MIDIEvent::NoteOff(_, note, _) => {
                                                                    if let Some((_track, ticks)) = position {
                                                                        let position_in_beats = *ticks as f64 / ppq as f64;
                                                                        if let Some(current_note) = current_notes.get_mut(&note) {
                                                                            current_note.set_length(position_in_beats - current_note.position());
                                                                            riff.events_mut().push(TrackEvent::Note(current_note.clone()));
                                                                            current_notes.retain(|current_note, _| *current_note != note);
                                                                        }
                                                                    }
                                                                },
                                                                apres::MIDIEvent::AfterTouch(_, _, _) => (),
                                                                apres::MIDIEvent::BankSelect(_, _) => (),
                                                                apres::MIDIEvent::BankSelectLSB(_, _) => (),
                                                                apres::MIDIEvent::ModulationWheel(_, _) => (),
                                                                apres::MIDIEvent::ModulationWheelLSB(_, _) => (),
                                                                apres::MIDIEvent::BreathController(_, _) => (),
                                                                apres::MIDIEvent::BreathControllerLSB(_, _) => (),
                                                                apres::MIDIEvent::FootPedal(_, _) => (),
                                                                apres::MIDIEvent::FootPedalLSB(_, _) => (),
                                                                apres::MIDIEvent::PortamentoTime(_, _) => (),
                                                                apres::MIDIEvent::PortamentoTimeLSB(_, _) => (),
                                                                apres::MIDIEvent::DataEntry(_, _) => (),
                                                                apres::MIDIEvent::DataEntryLSB(_, _) => (),
                                                                apres::MIDIEvent::Volume(_, _) => (),
                                                                apres::MIDIEvent::VolumeLSB(_, _) => (),
                                                                apres::MIDIEvent::Balance(_, _) => (),
                                                                apres::MIDIEvent::BalanceLSB(_, _) => (),
                                                                apres::MIDIEvent::Pan(_, _) => (),
                                                                apres::MIDIEvent::PanLSB(_, _) => (),
                                                                apres::MIDIEvent::Expression(_, _) => (),
                                                                apres::MIDIEvent::ExpressionLSB(_, _) => (),
                                                                apres::MIDIEvent::EffectControl1(_, _) => (),
                                                                apres::MIDIEvent::EffectControl1LSB(_, _) => (),
                                                                apres::MIDIEvent::EffectControl2(_, _) => (),
                                                                apres::MIDIEvent::EffectControl2LSB(_, _) => (),
                                                                apres::MIDIEvent::GeneralPurpose1(_, _) => (),
                                                                apres::MIDIEvent::GeneralPurpose1LSB(_, _) => (),
                                                                apres::MIDIEvent::GeneralPurpose2(_, _) => (),
                                                                apres::MIDIEvent::GeneralPurpose2LSB(_, _) => (),
                                                                apres::MIDIEvent::GeneralPurpose3(_, _) => (),
                                                                apres::MIDIEvent::GeneralPurpose3LSB(_, _) => (),
                                                                apres::MIDIEvent::GeneralPurpose4(_, _) => (),
                                                                apres::MIDIEvent::GeneralPurpose4LSB(_, _) => (),
                                                                apres::MIDIEvent::HoldPedal(_, _) => (),
                                                                apres::MIDIEvent::Portamento(_, _) => (),
                                                                apres::MIDIEvent::Sustenuto(_, _) => (),
                                                                apres::MIDIEvent::SoftPedal(_, _) => (),
                                                                apres::MIDIEvent::Legato(_, _) => (),
                                                                apres::MIDIEvent::Hold2Pedal(_, _) => (),
                                                                apres::MIDIEvent::SoundVariation(_, _) => (),
                                                                apres::MIDIEvent::SoundTimbre(_, _) => (),
                                                                apres::MIDIEvent::SoundReleaseTime(_, _) => (),
                                                                apres::MIDIEvent::SoundAttack(_, _) => (),
                                                                apres::MIDIEvent::SoundBrightness(_, _) => (),
                                                                apres::MIDIEvent::SoundControl1(_, _) => (),
                                                                apres::MIDIEvent::SoundControl2(_, _) => (),
                                                                apres::MIDIEvent::SoundControl3(_, _) => (),
                                                                apres::MIDIEvent::SoundControl4(_, _) => (),
                                                                apres::MIDIEvent::SoundControl5(_, _) => (),
                                                                apres::MIDIEvent::GeneralPurpose5(_, _) => (),
                                                                apres::MIDIEvent::GeneralPurpose6(_, _) => (),
                                                                apres::MIDIEvent::GeneralPurpose7(_, _) => (),
                                                                apres::MIDIEvent::GeneralPurpose8(_, _) => (),
                                                                apres::MIDIEvent::EffectsLevel(_, _) => (),
                                                                apres::MIDIEvent::TremuloLevel(_, _) => (),
                                                                apres::MIDIEvent::ChorusLevel(_, _) => (),
                                                                apres::MIDIEvent::CelesteLevel(_, _) => (),
                                                                apres::MIDIEvent::PhaserLevel(_, _) => (),
                                                                apres::MIDIEvent::DataIncrement(_) => (),
                                                                apres::MIDIEvent::DataDecrement(_) => (),
                                                                apres::MIDIEvent::RegisteredParameterNumber(_, _) => (),
                                                                apres::MIDIEvent::RegisteredParameterNumberLSB(_, _) => (),
                                                                apres::MIDIEvent::NonRegisteredParameterNumber(_, _) => (),
                                                                apres::MIDIEvent::NonRegisteredParameterNumberLSB(_, _) => (),
                                                                apres::MIDIEvent::AllControllersOff(_) => (),
                                                                apres::MIDIEvent::LocalControl(_, _) => (),
                                                                apres::MIDIEvent::AllNotesOff(_) => (),
                                                                apres::MIDIEvent::AllSoundOff(_) => (),
                                                                apres::MIDIEvent::OmniOff(_) => (),
                                                                apres::MIDIEvent::OmniOn(_) => (),
                                                                apres::MIDIEvent::MonophonicOperation(_, _) => (),
                                                                apres::MIDIEvent::PolyphonicOperation(_) => (),
                                                                apres::MIDIEvent::ControlChange(_, _, _) => (),
                                                                apres::MIDIEvent::ProgramChange(_, _) => (),
                                                                apres::MIDIEvent::ChannelPressure(_, _) => (),
                                                                apres::MIDIEvent::PitchWheelChange(_, _) => (),
                                                                apres::MIDIEvent::SystemExclusive(_) => (),
                                                                apres::MIDIEvent::MTCQuarterFrame(_, _) => (),
                                                                apres::MIDIEvent::SongPositionPointer(_) => (),
                                                                apres::MIDIEvent::SongSelect(_) => (),
                                                                apres::MIDIEvent::TimeCode(_, _, _, _, _) => (),
                                                                apres::MIDIEvent::EndOfTrack => {
                                                                    if let Some((_, ticks)) = position {
                                                                        let position_in_beats = *ticks as f64 / ppq as f64;
                                                                        riff.set_length(position_in_beats);
                                                                    }
                                                                },
                                                                apres::MIDIEvent::TuneRequest => (),
                                                                apres::MIDIEvent::MIDIClock => (),
                                                                apres::MIDIEvent::MIDIStart => (),
                                                                apres::MIDIEvent::MIDIContinue => (),
                                                                apres::MIDIEvent::MIDIStop => (),
                                                                apres::MIDIEvent::ActiveSense => (),
                                                                apres::MIDIEvent::Reset => (),
                                                            }
                                                        },
                                                        None => debug!("Could not find event."),
                                                    }
                                                }

                                                track_number += 1;
                                                freedom_daw_track.set_name(track_name);
                                                tracks.push(TrackType::InstrumentTrack(freedom_daw_track));

                                                if let Some(track_type) = tracks.last_mut() {
                                                    DAWState::init_track(
                                                        vst24_plugin_loaders.clone(),
                                                        clap_plugin_loaders.clone(),
                                                        tx_to_audio.clone(),
                                                        track_audio_coast.clone(),
                                                        &mut instrument_track_senders2,
                                                        &mut instrument_track_receivers2,
                                                        track_type,
                                                        None,
                                                        None,
                                                        vst_host_time_info.clone(),
                                                    );
                                                }
                                            }

                                            state.update_track_senders_and_receivers(instrument_track_senders2, instrument_track_receivers2);
                                        },
                                        Err(error) => debug!("Couldn't read midi file: {:?}", error),
                                    }
                                }
                            },
                            Err(_) => debug!("Main - rx_ui processing loop - Import Midi File - could not get lock on state"),
                        }
                        if let Ok(mut coast) = track_audio_coast.lock() {
                            *coast = TrackBackgroundProcessorMode::AudioOut;
                        }

                        let _ = tx_from_ui.send(DAWEvents::UpdateUI);
                        let _ = tx_from_ui.send(DAWEvents::HideProgressDialogue);
                    });
                }
            }
            DAWEvents::ExportMidiFile(path) => {
                gui.ui.dialogue_progress_bar.set_text(Some(format!("Exporting midi file as {}...", path.to_str().unwrap()).as_str()));
                gui.ui.progress_dialogue.set_title("Export Midi File");
                gui.ui.progress_dialogue.show_all();

                if let Ok(mut coast) = track_audio_coast.lock() {
                    *coast = TrackBackgroundProcessorMode::Render;
                }
                {
                    let state = state.clone();
                    let tx_from_ui = tx_from_ui;
                    let track_audio_coast = track_audio_coast;
                    let _ = thread::Builder::new().name("Export midi file".into()).spawn(move || {
                        match state.lock() {
                            Ok(state) => {
                                debug!("Main - rx_ui processing loop - Export Midi File - attempting to export.");
                                if !state.export_to_midi_file(path) {
                                    let _ = tx_from_ui.send(DAWEvents::HideProgressDialogue);
                                    let _ = tx_from_ui.send(DAWEvents::Notification(NotificationType::Error, "Could not export midi file.".to_string()));
                                }
                            }
                            Err(_) => debug!("Main - rx_ui processing loop - Export Midi File - could not get lock on state"),
                        }
                        if let Ok(mut coast) = track_audio_coast.lock() {
                            *coast = TrackBackgroundProcessorMode::AudioOut;
                        }
                        let _ = tx_from_ui.send(DAWEvents::HideProgressDialogue);
                    });
                }
            }
            DAWEvents::ExportRiffsToMidiFile(path) => {
                gui.ui.dialogue_progress_bar.set_text(Some(format!("Exporting riffs to midi file as {}...", path.to_str().unwrap()).as_str()));
                gui.ui.progress_dialogue.set_title("Export riffs to midi file");
                gui.ui.progress_dialogue.show_all();

                if let Ok(mut coast) = track_audio_coast.lock() {
                    *coast = TrackBackgroundProcessorMode::Render;
                }
                {
                    let state = state.clone();
                    let tx_from_ui = tx_from_ui;
                    let track_audio_coast = track_audio_coast;
                    let _ = thread::Builder::new().name("Export riffs to midi file".into()).spawn(move || {
                        match state.lock() {
                            Ok(state) => {
                                debug!("Main - rx_ui processing loop - Export riffs to midi file - attempting to export.");
                                if !state.export_riffs_to_midi_file(path) {
                                    let _ = tx_from_ui.send(DAWEvents::HideProgressDialogue);
                                    let _ = tx_from_ui.send(DAWEvents::Notification(NotificationType::Error, "Could not export riffs to midi file.".to_string()));
                                }
                            }
                            Err(_) => debug!("Main - rx_ui processing loop - Export riffs to midi file - could not get lock on state"),
                        }
                        if let Ok(mut coast) = track_audio_coast.lock() {
                            *coast = TrackBackgroundProcessorMode::AudioOut;
                        }
                        let _ = tx_from_ui.send(DAWEvents::HideProgressDialogue);
                    });
                }
            }
            DAWEvents::ExportRiffsToSeparateMidiFiles(path) => {
                gui.ui.dialogue_progress_bar.set_text(Some(format!("Exporting riffs to separate midi files to directory {}...", path.to_str().unwrap()).as_str()));
                gui.ui.progress_dialogue.set_title("Export riffs to separate midi files");
                gui.ui.progress_dialogue.show_all();

                if let Ok(mut coast) = track_audio_coast.lock() {
                    *coast = TrackBackgroundProcessorMode::Render;
                }
                {
                    let state = state.clone();
                    let tx_from_ui = tx_from_ui;
                    let track_audio_coast = track_audio_coast;
                    let _ = thread::Builder::new().name("Export riffs to separate midi files".into()).spawn(move || {
                        match state.lock() {
                            Ok(state) => {
                                debug!("Main - rx_ui processing loop - Export riffs to separate midi files - attempting to export.");
                                if !state.export_riffs_to_separate_midi_files(path) {
                                    let _ = tx_from_ui.send(DAWEvents::HideProgressDialogue);
                                    let _ = tx_from_ui.send(DAWEvents::Notification(NotificationType::Error, "Could not export riffs to separate midi files.".to_string()));
                                }
                            }
                            Err(_) => debug!("Main - rx_ui processing loop - Export riffs to separate midi files - could not get lock on state"),
                        }
                        if let Ok(mut coast) = track_audio_coast.lock() {
                            *coast = TrackBackgroundProcessorMode::AudioOut;
                        }
                        let _ = tx_from_ui.send(DAWEvents::HideProgressDialogue);
                    });
                }
            }
            DAWEvents::ExportWaveFile(path) => {
                gui.ui.dialogue_progress_bar.set_text(Some(format!("Exporting wave file as {}...", path.to_str().unwrap()).as_str()));
                gui.ui.progress_dialogue.set_title("Export Wav File");
                gui.ui.progress_dialogue.show_all();

                if let Ok(mut coast) = track_audio_coast.lock() {
                    *coast = TrackBackgroundProcessorMode::Render;
                }
                match state.lock() {
                    Ok(mut state) => {
                        debug!("Main - rx_ui processing loop - Export Wave File - attempting to export.");
                        state.export_to_wave_file(path, tx_to_audio, track_audio_coast, tx_from_ui);
                    }
                    Err(_) => debug!("Main - rx_ui processing loop - Export Wave File - could not get lock on state"),
                }
            }
            DAWEvents::UpdateUI => {
                let state_arc = state.clone();
                match state.lock() {
                    Ok(mut state) => {
                        gui.update_ui_from_state(tx_from_ui, &mut state, state_arc);
                    }
                    Err(_) => debug!("Main - rx_ui processing loop - Export Wave File - could not get lock on state"),
                }

                gui.ui.track_drawing_area.queue_draw();
                gui.ui.piano_roll_drawing_area.queue_draw();
                gui.ui.sample_roll_drawing_area.queue_draw();
                gui.ui.automation_drawing_area.queue_draw();
            }
            DAWEvents::UpdateState => debug!("Event: update state"),
            DAWEvents::Notification(notification_type, message) => {
                let message_type = match notification_type {
                    NotificationType::Info => { MessageType::Info }
                    NotificationType::Warning => { MessageType::Warning }
                    NotificationType::Question => { MessageType::Question }
                    NotificationType::Error => { MessageType::Error }
                    NotificationType::Other => { MessageType::Other }
                };
                let message_dialogue = MessageDialog::builder()
                    .modal(true)
                    .title("Message")
                    .message_type(message_type)
                    .text(message.as_str())
                    .buttons(ButtonsType::Close)
                    .build();

                message_dialogue.run();
                message_dialogue.close();
                message_dialogue.hide();
            }
            DAWEvents::AutomationViewShowTypeChange(show_type) => {
                let type_to_show = match show_type {
                    ShowType::Velocity => AutomationViewMode::NoteVelocities,
                    ShowType::Controller => AutomationViewMode::Controllers,
                    ShowType::InstrumentParameter => AutomationViewMode::Instrument,
                    ShowType::EffectParameter => AutomationViewMode::Effect,
                    ShowType::NoteExpression => AutomationViewMode::NoteExpression,
                };
                match state.lock() {
                    Ok(mut state) => state.set_automation_view_mode(type_to_show),
                    Err(error) => debug!("Error: {}", error),
                }
                gui.ui.automation_drawing_area.queue_draw();
            },
            DAWEvents::LoopChange(change_type, uuid) => {
                debug!("Event: LoopChange");
                match change_type {
                    LoopChangeType::LoopOn => {
                        match state.lock() {
                            Ok(state) => {
                                let mut start_block = 0;
                                let mut end_block = 44;
                                let song = state.project().song();
                                let tracks = song.tracks();

                                match state.active_loop() {
                                    Some(active_loop_uuid) => {
                                        match song.loops().iter().find(|current_loop| current_loop.uuid().to_string() == active_loop_uuid.to_string()) {
                                            Some(active_loop) => {
                                                start_block = (active_loop.start_position() * 44100.0 / 1024.0) as i32;
                                                end_block = (active_loop.end_position() * 44100.0 / 1024.0) as i32;
                                            },
                                            None => debug!("Could not find the active loop."),
                                        }
                                    },
                                    None => debug!("No active loop found to set left position."),
                                }
                                match tx_to_audio.send(AudioLayerInwardEvent::ExtentsChange(end_block - start_block)) {
                                    Ok(_) => (),
                                    Err(error) => debug!("Problem using tx_to_audio to send message to jack layer when turning looping on: {}", error),
                                }
                                for track in tracks {
                                    state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::LoopExtents(start_block, end_block));
                                    state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Loop(true));
                                }

                                {
                                    let mut state = state;
                                    state.set_looping(true);
                                }
                            },
                            Err(_) => debug!("Main - rx_ui processing loop - set active loop - could not get lock on state"),
                        }
                        gui.ui.track_drawing_area.queue_draw();
                    }
                    LoopChangeType::LoopOff => {
                        match state.lock() {
                            Ok(state) => {
                                let mut state = state;
                                state.set_looping(false);
                                let song = state.project().song();
                                let tracks = song.tracks();
                                for track in tracks {
                                    state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Loop(false));
                                }
                            },
                            Err(_) => debug!("Main - rx_ui processing loop - set active loop - could not get lock on state"),
                        }
                        gui.ui.track_drawing_area.queue_draw();
                    }
                    LoopChangeType::ActiveLoopChanged(uuid) => {
                        match state.lock() {
                            Ok(state) => {
                                let mut start_block = 0;
                                let mut end_block = 44;
                                let song = state.project().song();
                                let tracks = song.tracks();

                                match uuid {
                                    Some(active_loop_uuid) => {
                                        match song.loops().iter().find(|current_loop| current_loop.uuid().to_string() == active_loop_uuid.to_string()) {
                                            Some(active_loop) => {
                                                start_block = (active_loop.start_position() * 44100.0 / 1024.0) as i32;
                                                end_block = (active_loop.end_position() * 44100.0 / 1024.0) as i32;
                                            },
                                            None => debug!("Could not find the active loop."),
                                        }
                                    },
                                    None => debug!("No loop found to mark as active."),
                                }
                                for track in tracks {
                                    state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::LoopExtents(start_block, end_block));
                                }
                                {
                                    let mut state = state;
                                    state.set_active_loop(uuid);
                                }
                            },
                            Err(_) => debug!("Main - rx_ui processing loop - set active loop - could not get lock on state"),
                        }
                        gui.ui.track_drawing_area.queue_draw();
                    },
                    LoopChangeType::LoopLimitLeftChanged(start_position) => {
                        match state.lock() {
                            Ok(state) => {
                                match state.active_loop() {
                                    Some(active_loop_uuid) => {
                                        let song = state.project().song();
                                        let tracks = song.tracks();
                                        match song.loops().iter().find(|current_loop| current_loop.uuid().to_string() == active_loop_uuid.to_string()) {
                                            Some(active_loop) => {
                                                let start_block = (start_position * 44100.0 / 1024.0) as i32;
                                                let end_block = (active_loop.end_position() * 44100.0 / 1024.0) as i32;
                                                for track in tracks {
                                                    state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::LoopExtents(start_block, end_block));
                                                }
                                            },
                                            None => debug!("Could not find the active loop."),
                                        }
                                        let mut state = state;
                                        match state.get_project().song_mut().loops_mut().iter_mut().find(|current_loop| current_loop.uuid().to_string() == active_loop_uuid.to_string()) {
                                            Some(active_loop) => {
                                                active_loop.set_start_position(start_position);
                                            },
                                            None => debug!("Could not find the active loop."),
                                        }
                                    },
                                    None => debug!("No active loop found to set left position."),
                                }
                            },
                            Err(_) => debug!("Main - rx_ui processing loop - loop add - could not get lock on state"),
                        }
                        gui.ui.track_drawing_area.queue_draw();
                    },
                    LoopChangeType::LoopLimitRightChanged(end_position) => {
                        match state.lock() {
                            Ok(state) => {
                                match state.active_loop() {
                                    Some(active_loop_uuid) => {
                                        let song = state.project().song();
                                        let tracks = song.tracks();
                                        match song.loops().iter().find(|current_loop| current_loop.uuid().to_string() == active_loop_uuid.to_string()) {
                                            Some(active_loop) => {
                                                let start_block = (active_loop.start_position() * 44100.0 / 1024.0) as i32;
                                                let end_block = (end_position * 44100.0 / 1024.0) as i32;
                                                for track in tracks {
                                                    state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::LoopExtents(start_block, end_block));
                                                }
                                            },
                                            None => debug!("Could not find the active loop."),
                                        }
                                        let mut state = state;
                                        match state.get_project().song_mut().loops_mut().iter_mut().find(|current_loop| current_loop.uuid().to_string() == active_loop_uuid.to_string()) {
                                            Some(active_loop) => {
                                                active_loop.set_end_position(end_position);
                                            },
                                            None => debug!("Could not find the active loop."),
                                        }
                                    },
                                    None => debug!("No active loop found to set right position."),
                                }
                            },
                            Err(_) => debug!("Main - rx_ui processing loop - loop add - could not get lock on state"),
                        }
                        gui.ui.track_drawing_area.queue_draw();
                    },
                    LoopChangeType::Added(loop_name) => {
                        match state.lock() {
                            Ok(state) => {
                                let mut state = state;
                                state.get_project().song_mut().add_loop(Loop::new_with_uuid_and_name(uuid, loop_name));
                            },
                            Err(_) => debug!("Main - rx_ui processing loop - loop add - could not get lock on state"),
                        }
                        gui.ui.track_drawing_area.queue_draw();
                    }
                    LoopChangeType::Deleted => {
                        match state.lock() {
                            Ok(state) => {
                                let mut state = state;
                                state.get_project().song_mut().delete_loop(uuid);
                            },
                            Err(_) => debug!("Main - rx_ui processing loop - loop delete - could not get lock on state"),
                        }
                        gui.ui.track_drawing_area.queue_draw();
                    }
                    LoopChangeType::NameChanged(name) => {
                        match state.lock() {
                            Ok(state) => {
                                let mut state = state;
                                state.get_project().song_mut().change_loop_name(uuid, name);
                            },
                            Err(_) => debug!("Main - rx_ui processing loop - loop name change - could not get lock on state"),
                        }
                    },
                }
            },
            DAWEvents::ProjectChange(_) => debug!("Event: ProjectChange"),
            DAWEvents::PianoRollSetTrackName(name) => {
                gui.set_piano_roll_selected_track_name_label(name.as_str());
                gui.ui.piano_roll_drawing_area.queue_draw();
            }
            DAWEvents::PianoRollSetRiffName(name) => {
                gui.set_piano_roll_selected_riff_name_label(name.as_str());
                gui.ui.piano_roll_drawing_area.queue_draw();
            }
            DAWEvents::SampleRollSetTrackName(name) => {
                gui.set_sample_roll_selected_track_name_label(name.as_str());
                gui.ui.sample_roll_drawing_area.queue_draw();
            }
            DAWEvents::SampleRollSetRiffName(name) => {
                gui.set_sample_roll_selected_riff_name_label(name.as_str());
                gui.ui.sample_roll_drawing_area.queue_draw();
            }
            DAWEvents::TrackChange(track_change_type, track_uuid) => match track_change_type {
                TrackChangeType::Added(track_change_track_type) => {
                    let state_arc = state.clone();
                    let mut track_uuid = None;
                    match state.lock() {
                        Ok(mut state) => {
                            let tx_ui = tx_from_ui.clone();
                            let mut instrument_track_senders_local = HashMap::new();
                            let mut instrument_track_receivers_local = HashMap::new();

                            match track_change_track_type {
                                GeneralTrackType::InstrumentTrack => {
                                    debug!("Adding an instrument track to the state...");
                                    let track = InstrumentTrack::new();
                                    track_uuid = Some(track.uuid().to_string());
                                    // gui.add_track(track.name(), track.uuid(), tx_ui, state_arc, track_change_track_type, None, track.volume(), track.pan(), false, false);
                                    state.get_project().song_mut().add_track(TrackType::InstrumentTrack(track));
                                    if let Some(track_type) = state.get_project().song_mut().tracks_mut().last_mut() {
                                        DAWState::init_track(
                                            vst24_plugin_loaders,
                                            clap_plugin_loaders,
                                            tx_to_audio,
                                            track_audio_coast,
                                            &mut instrument_track_senders_local,
                                            &mut instrument_track_receivers_local,
                                            track_type,
                                            None,
                                            None,
                                            vst_host_time_info,
                                        );
                                    }
                                    debug!("Added an instrument track to the state.");
                                }
                                GeneralTrackType::AudioTrack => {
                                    debug!("Adding an audio track to the state...");
                                    let track = AudioTrack::new();
                                    track_uuid = Some(track.uuid().to_string());
                                    // gui.add_track(track.name(), track.uuid(), tx_ui, state_arc, track_change_track_type, None, track.volume(), track.pan(), false, false);
                                    state.get_project().song_mut().add_track(TrackType::AudioTrack(track));
                                    debug!("Added an audio track to the state.");
                                    if let Some(track_type) = state.get_project().song_mut().tracks_mut().last_mut() {
                                        DAWState::init_track(
                                            vst24_plugin_loaders,
                                            clap_plugin_loaders,
                                            tx_to_audio,
                                            track_audio_coast,
                                            &mut instrument_track_senders_local,
                                            &mut instrument_track_receivers_local,
                                            track_type,
                                            None,
                                            None,
                                            vst_host_time_info,
                                        );
                                    }
                                }
                                GeneralTrackType::MidiTrack => {
                                    debug!("Adding a midi track to the state...");
                                    let track = MidiTrack::new();
                                    let uuid = track.uuid().to_string();

                                    track_uuid = Some(track.uuid().to_string());
                                    // gui.add_track(track.name(), track.uuid(), tx_ui, state_arc, track_change_track_type, Some(state.midi_devices()), track.volume(), track.pan(), false, false);
                                    state.get_project().song_mut().add_track(TrackType::MidiTrack(track));
                                    if let Some(track_type) = state.get_project().song_mut().tracks_mut().last_mut() {
                                        DAWState::init_track(
                                            vst24_plugin_loaders,
                                            clap_plugin_loaders,
                                            tx_to_audio.clone(),
                                            track_audio_coast,
                                            &mut instrument_track_senders_local,
                                            &mut instrument_track_receivers_local,
                                            track_type,
                                            None,
                                            None,
                                            vst_host_time_info,
                                        );
                                    }
                                    thread::sleep(Duration::from_secs(1));
                                    if let Some(jack_client) = state.jack_client() {
                                        if let Ok(midi_out_port) = jack_client.register_port(uuid.as_str(), MidiOut::default()) {
                                            match tx_to_audio.send(AudioLayerInwardEvent::NewMidiOutPortForTrack(uuid, midi_out_port)) {
                                                Ok(_) => (),
                                                Err(error) => debug!("Problem using tx_to_audio to send new midi out port message to jack layer: {}", error),
                                            }
                                        }
                                    }
                                    debug!("Added a midi track to the state.");
                                }
                                _ => {}
                            }

                            gui.clear_ui();
                            gui.update_ui_from_state(tx_from_ui, &mut state, state_arc);

                            state.update_track_senders_and_receivers(instrument_track_senders_local, instrument_track_receivers_local);
                            gui.update_available_audio_plugins_in_ui(state.vst_instrument_plugins(), state.vst_effect_plugins());
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - Track Added - could not get lock on state"),
                    }
                }
                TrackChangeType::Deleted => {
                    let state_arc = state.clone();
                    match state.lock() {
                        Ok(state) => {
                            let mut state = state;

                            match track_uuid {
                                Some(track_uuid) => {
                                    // gui.delete_track_from_ui(track_uuid.clone());
                                    state.get_project().song_mut().delete_track(track_uuid);
                                    gui.clear_ui();
                                    gui.update_ui_from_state(tx_from_ui, &mut state, state_arc);
                                },
                                None => debug!("Main - rx_ui processing loop - Track Deleted - could not find track"),
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - Track Deleted - could not get lock on state"),
                    }
                    gui.ui.track_drawing_area.queue_draw();
                }
                TrackChangeType::Modified => debug!("TrackChangeType::Modified not yet implemented!"),
                TrackChangeType::SoloOn => {
                    match state.lock() {
                        Ok(mut state) => {
                            let mut tracks_to_mute = vec![];
                            let mut tracks_to_unmute = vec![];
                            {
                                let state = &mut state;
                                let track_uuid = track_uuid.unwrap();
                                for track in state.get_project().song_mut().tracks_mut() {
                                    if track.uuid().to_string() == track_uuid {
                                        track.set_solo(true);
                                        // track.set_mute(false);
                                        tracks_to_unmute.push(track.uuid().to_string());
                                    } else if !track.solo() {
                                        // track.set_mute(true);
                                        tracks_to_mute.push(track.uuid().to_string());
                                    }
                                }
                            }
                            for uuid in tracks_to_mute {
                                state.send_to_track_background_processor(uuid, TrackBackgroundProcessorInwardEvent::Mute);
                            }
                            for uuid in tracks_to_unmute {
                                state.send_to_track_background_processor(uuid, TrackBackgroundProcessorInwardEvent::Unmute);
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - SoloOn - could not get lock on state"),
                    }
                }
                TrackChangeType::SoloOff => {
                    debug!("Main - rx_ui processing loop - turn solo off - received event from the UI.");
                    match state.lock() {
                        Ok(mut state) => {
                            let mut tracks_to_mute = vec![];
                            let mut tracks_to_unmute = vec![];
                            {
                                let track_uuid = track_uuid.unwrap();
                                let mut found_solo_track = false;
                                for track in state.get_project().song_mut().tracks_mut() {
                                    if track.uuid().to_string() == track_uuid {
                                        track.set_solo(false);
                                    } else if track.solo() {
                                        found_solo_track = true;
                                    }
                                }
                                for track in state.get_project().song_mut().tracks_mut() {
                                    if found_solo_track && !track.solo() {
                                        tracks_to_mute.push(track.uuid().to_string());
                                    } else if !found_solo_track && !track.mute() {
                                        tracks_to_unmute.push(track.uuid().to_string());
                                    }
                                }
                            }
                            for uuid in tracks_to_unmute {
                                state.send_to_track_background_processor(uuid, TrackBackgroundProcessorInwardEvent::Unmute);
                            }
                            for uuid in tracks_to_mute {
                                state.send_to_track_background_processor(uuid, TrackBackgroundProcessorInwardEvent::Mute);
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - SoloOff - could not get lock on state"),
                    }
                }
                TrackChangeType::Mute => {
                    match state.lock() {
                        Ok(state) => {
                            let mut state = state;
                            let track_uuid = track_uuid.unwrap();
                            match state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                                Some(track) => track.set_mute(true),
                                None => (),
                            };
                            state.send_to_track_background_processor(track_uuid, TrackBackgroundProcessorInwardEvent::Mute);
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - Save As File - could not get lock on state"),
                    }
                }
                TrackChangeType::Unmute => {
                    match state.lock() {
                        Ok(state) => {
                            let mut state = state;
                            let track_uuid = track_uuid.unwrap();
                            match state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                                Some(track) => track.set_mute(false),
                                None => (),
                            };
                            state.send_to_track_background_processor(track_uuid, TrackBackgroundProcessorInwardEvent::Unmute);
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - Save As File - could not get lock on state"),
                    }
                }
                TrackChangeType::MidiOutputDeviceChanged(midi_device_name) => {
                    let track_uuid = track_uuid.unwrap();
                    match state.lock() {
                        Ok(mut state) => {
                            let previous_midi_device_name = match state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                                Some(track_type) => match track_type {
                                    TrackType::InstrumentTrack(_) => "".to_string(),
                                    TrackType::AudioTrack(_) => "".to_string(),
                                    TrackType::MidiTrack(track) => {
                                        let previous_midi_device_name = track.midi_device_mut().name().to_string();
                                        track.midi_device_mut().set_name(midi_device_name.clone());
                                        previous_midi_device_name
                                    },
                                },
                                None => "".to_string(),
                            };
                            if !previous_midi_device_name.is_empty() {
                                state.jack_midi_connection_remove(track_uuid.clone(), previous_midi_device_name);
                            }
                            state.jack_midi_connection_add(track_uuid, midi_device_name);
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - track instrument changed - could not get lock on state"),
                    }
                },
                TrackChangeType::MidiInputDeviceChanged => debug!("TrackChangeType::MidiInputDeviceChanged not yet implemented!"),
                TrackChangeType::MidiOutputChannelChanged(midi_channel) => {
                    let track_uuid = track_uuid.unwrap();
                    match state.lock() {
                        Ok(mut state) => {
                            match state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                                Some(track_type) => match track_type {
                                    TrackType::InstrumentTrack(_) => (),
                                    TrackType::AudioTrack(_) => (),
                                    TrackType::MidiTrack(track) => {
                                        track.midi_device_mut().set_midi_channel(midi_channel);
                                    },
                                },
                                None => (),
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - track instrument changed - could not get lock on state"),
                    }
                },
                TrackChangeType::MidiInputChannelChanged => debug!("TrackChangeType::MidiInputChannelChanged not yet implemented!"),
                TrackChangeType::InstrumentChanged(instrument_details) => {
                    match state.lock() {
                        Ok(state) => {
                            let mut state = state;
                            if let Some(track_uuid) = track_uuid {
                                // remove the old window
                                match state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                                    Some(track_type) => match track_type {
                                        TrackType::InstrumentTrack(track) => {
                                            let _track_name = track.name().to_string();
                                            let instrument = track.instrument_mut();
                                            if let Some(window) = audio_plugin_windows.get(&instrument.uuid().to_string()) {
                                                if window.is_visible() {
                                                    window.hide();
                                                }
                                            }
                                            audio_plugin_windows.remove_entry(&instrument.uuid().to_string());
                                        }
                                        _ => {}
                                    }
                                    None => {}
                                }
                                state.load_instrument(vst24_plugin_loaders, clap_plugin_loaders, instrument_details, track_uuid);
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - track instrument changed - could not get lock on state"),
                    };
                },
                TrackChangeType::ShowInstrument => {
                    let mut xid = 0;
                    let mut track_uuid = track_uuid.unwrap();
                    match state.lock() {
                        Ok(mut state) => {
                            match state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                                Some(track_type) => match track_type {
                                    TrackType::InstrumentTrack(track) => {
                                        let track_name = track.name().to_string();
                                        let instrument = track.instrument_mut();
                                        if let Some(window) = audio_plugin_windows.get(&instrument.uuid().to_string()) {
                                            if window.is_visible() {
                                                window.hide();
                                            } else {
                                                window.show_all();
                                            }
                                        } else {
                                            let win = Window::new(WindowType::Toplevel);
                                            win.set_title(format!("Track: {} - Instrument: {}", track_name, instrument.name()).as_str());
                                            win.connect_delete_event(|window, _| {
                                                window.hide();
                                                gtk::Inhibit(true)
                                            });
                                            win.set_height_request(200);
                                            win.set_width_request(200);
                                            win.set_resizable(true);
                                            win.show_all();
                                            audio_plugin_windows.insert(instrument.uuid().to_string(), win.clone());

                                            let window = win.clone();
                                            {
                                                glib::idle_add_local(move || {
                                                    if window.is_visible() {
                                                        window.queue_draw();
                                                    }
                                                    glib::Continue(true)
                                                });
                                            }

                                            unsafe {
                                                match win.window() {
                                                    Some(gdk_window) => {
                                                        xid = gdk_x11_window_get_xid(gdk_window);
                                                        debug!("xid: {}", xid);
                                                    },
                                                    None => debug!("Couldn't get gdk window."),
                                                }
                                            }

                                            track_uuid = track.uuid().to_string();
                                        }
                                    },
                                    TrackType::AudioTrack(_) => (),
                                    TrackType::MidiTrack(_) => (),
                                },
                                None => (),
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - show track instrument - could not get lock on state"),
                    };
                    if xid != 0 {
                        match state.lock() {
                            Ok(state) => {
                                state.send_to_track_background_processor(track_uuid, TrackBackgroundProcessorInwardEvent::SetInstrumentWindowId(xid));
                            },
                            Err(_) => debug!("Could not get read only lock on state."),
                        }
                    }
                }
                TrackChangeType::TrackNameChanged(track_name) => {
                    match state.lock() {
                        Ok(state) => {
                            let mut state = state;
                            let track_uuid = track_uuid.unwrap();
                            debug!("Track name changed: \"{}\", name=\"{}\"", track_name.as_str(), &track_uuid);
                            match state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                                Some(track) => {
                                    track.set_name(track_name.clone());
                                    gui.change_track_name(track_uuid.clone(), track_name);
                                },
                                None => (),
                            };
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - Save As File - could not get lock on state"),
                    };
                },
                TrackChangeType::EffectAdded(uuid, name, effect_details) => {
                    match state.lock() {
                        Ok(mut state) => {
                            match track_uuid {
                                Some(track_uuid) => {
                                    let track_uuid2 = track_uuid.clone();
                                    state.send_to_track_background_processor(track_uuid, TrackBackgroundProcessorInwardEvent::AddEffect(vst24_plugin_loaders, clap_plugin_loaders, uuid, effect_details.clone()));
                                    match state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid2) {
                                        Some(track_type) => match track_type {
                                            TrackType::InstrumentTrack(track) => {
                                                let (sub_plugin_id, library_path, plugin_type) = get_plugin_details(effect_details.clone());
                                                let effect = AudioPlugin::new_with_uuid(uuid, name, library_path, sub_plugin_id, plugin_type);
                                                track.effects_mut().push(effect);
                                            },
                                            TrackType::AudioTrack(_) => (),
                                            TrackType::MidiTrack(_) => (),
                                        },
                                        None => debug!("Main - rx_ui processing loop - track effect add - could not find track."),
                                    }
                                },
                                None => (),
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - track effect add - could not get lock on state"),
                    };
                },
                TrackChangeType::EffectDeleted(effect_uuid) => {
                    match state.lock() {
                        Ok(mut state) => {
                            match track_uuid.clone() {
                                Some(track_hash) => state.send_to_track_background_processor(track_hash, TrackBackgroundProcessorInwardEvent::DeleteEffect(effect_uuid.clone())),
                                None => (),
                            }
                            if let Some(track_uuid) = track_uuid {
                                let track_uuid2 = track_uuid;
                                if let Some(track_type) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid2) {
                                    match track_type {
                                        TrackType::InstrumentTrack(track) => {
                                            track.effects_mut().retain(|effect| {
                                                effect.uuid().to_string() != effect_uuid
                                            });
                                        },
                                        TrackType::AudioTrack(_) => (),
                                        TrackType::MidiTrack(_) => (),
                                    }
                                }
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - track effect delete - could not get lock on state"),
                    };
                }
                TrackChangeType::RiffAdd(uuid, mut name, length) => {
                    debug!("Main - rx_ui processing loop - riff add");
                    while name.is_empty() {
                        if gui.ui.riff_name_dialogue.run() == gtk::ResponseType::Ok && gui.ui.riff_name_entry.text().len() > 0 {
                            name = gui.ui.riff_name_entry.text().to_string();
                            gui.ui.riff_name_entry.set_text("");
                        }
                    }
                    gui.ui.riff_name_dialogue.hide();

                    let mut state = state.clone();
                    match history_manager.lock() {
                        Ok(mut history) => {
                            let action = RiffAdd::new(uuid, name, length, &mut state.clone());
                            match history.apply(&mut state, Box::new(action)) {
                                Ok(mut daw_events_to_propagate) => {
                                    // refresh UI
                                    gui.ui.track_drawing_area.queue_draw();
                                    gui.ui.piano_roll_drawing_area.queue_draw();
                                    gui.ui.automation_drawing_area.queue_draw();
                                    for _ in 0..(daw_events_to_propagate.len()) {
                                        let event = daw_events_to_propagate.remove(0);
                                        let _ = tx_from_ui.send(event);
                                    }
                                }
                                Err(error) => {
                                    error!("Main - rx_ui processing loop - riff add - error: {}", error);
                                }
                            }
                        }
                        Err(error) => {
                            error!("Main - rx_ui processing loop - riff add - error getting lock for history manager: {}", error);
                        }
                    }
                }
                TrackChangeType::RiffCopy(uuid_to_copy, uuid, mut name) => {
                    debug!("Main - rx_ui processing loop - riff copy");
                    match state.lock() {
                        Ok(mut state) => {
                            state.set_selected_riff_ref_uuid(None);

                            match track_uuid {
                                Some(track_uuid) => {
                                    state.set_selected_track(Some(track_uuid.clone()));
                                    state.set_selected_riff_uuid(track_uuid.clone(), uuid.to_string());

                                    while name.is_empty() {
                                        if gui.ui.riff_name_dialogue.run() == gtk::ResponseType::Ok && gui.ui.riff_name_entry.text().len() > 0 {
                                            name = gui.ui.riff_name_entry.text().to_string();
                                            gui.ui.riff_name_entry.set_text("");
                                        }
                                    }
                                    gui.ui.riff_name_dialogue.hide();

                                    // get the riff to copy and clone it

                                    match state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                                        Some(track) => {
                                            if let Some(riff) = track.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == uuid_to_copy) {
                                                let mut new_riff = riff.clone();
                                                new_riff.set_uuid(uuid);
                                                new_riff.set_name(name);
                                                track.riffs_mut().push(new_riff);
                                            }
                                        }
                                        None => {}
                                    }
                                },
                                None => debug!("Main - rx_ui processing loop - riff add  - problem getting selected riff track uuid"),
                            };
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - riff add - could not get lock on state"),
                    };
                    gui.ui.piano_roll_drawing_area.queue_draw();
                }
                TrackChangeType::RiffDelete(riff_uuid) => {
                    debug!("Need to handle track riff deleted.");

                    // check if any riff references are using this riff - if so then show a warning dialog
                    let found_info = match state.lock() {
                        Ok(state) => {
                            let mut found_info = vec![];
                            let mut riff_name = String::from("Unknown");
                            
                            // process the track
                            if let Some(uuid) = track_uuid.clone() {
                                if let Some(track) = state.project().song().tracks().iter().find(|track| track.uuid().to_string() == uuid) {
                                    // get the riff name
                                    if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_uuid) {
                                        riff_name = riff.name().to_string();
                                    }

                                    // check track riff refs
                                    for riff_ref in track.riff_refs().iter() {
                                        if riff_ref.linked_to() == riff_uuid {
                                            let message = format!("Track: \"{}\" has references to riff: \"{}\".", track.name(), riff_name.as_str());
                                            
                                            if !found_info.iter().any(|entry| *entry == message) {
                                                found_info.push(message);
                                            }
                                        }
                                    }
                                }
                            }

                            // check riff sets
                            for riff_set in state.project().song().riff_sets().iter() {
                                for (_, riff_ref) in riff_set.riff_refs().iter() {
                                    if riff_ref.linked_to() == riff_uuid {
                                        let message = format!("Riff set: \"{}\" has a reference to riff: \"{}\".", riff_set.name(), riff_name.as_str());

                                        if !found_info.iter().any(|entry| *entry == message) {
                                            found_info.push(message);
                                        }
                                    }
                                }
                            }

                            // check riff sequences
                            for riff_sequence in state.project().song().riff_sequences().iter() {
                                for riff_set_item in riff_sequence.riff_sets().iter() {
                                    if let Some(riff_set) = state.project().song().riff_set(riff_set_item.item_uuid().to_string()) {
                                        for (_, riff_ref) in riff_set.riff_refs().iter() {
                                            if riff_ref.linked_to() == riff_uuid {
                                                let message = format!("Riff sequence: \"{}\" has references to riff: \"{}\".", riff_sequence.name(), riff_name.as_str());

                                                if !found_info.iter().any(|entry| *entry == message) {
                                                    found_info.push(message);
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            // check riff arrangements
                            for riff_arrangement in state.project().song().riff_arrangements().iter() {
                                for riff_item in riff_arrangement.items().iter() {
                                    if *(riff_item.item_type()) == RiffItemType::RiffSet {
                                        if let Some(riff_set) = state.project().song().riff_set(riff_item.item_uuid().to_string()) {
                                            for (_, riff_ref) in riff_set.riff_refs().iter() {
                                                if riff_ref.linked_to() == riff_uuid {
                                                    let message = format!("Riff arrangement: \"{}\" has references to riff: \"{}\".", riff_arrangement.name(), riff_name.as_str());

                                                    if !found_info.iter().any(|entry| *entry == message) {
                                                        found_info.push(message);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    else {
                                        if let Some(riff_sequence) = state.project().song().riff_sequence(riff_item.uuid()) {
                                            for riff_set_item in riff_sequence.riff_sets().iter() {
                                                if let Some(riff_set) = state.project().song().riff_set(riff_set_item.item_uuid().to_string()) {
                                                    for (_, riff_ref) in riff_set.riff_refs().iter() {
                                                        if riff_ref.linked_to() == riff_uuid {
                                                            let message = format!("Riff arrangement: \"{}\" has references to riff: \"{}\".", riff_arrangement.name(), riff_name.as_str());
                                                    
                                                            if !found_info.iter().any(|entry| *entry == message) {
                                                                found_info.push(message);
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            found_info
                        }
                        Err(_) => {
                            debug!("Main - rx_ui processing loop - track_riff_delete - could not get lock on state");
                            vec![]
                        }
                    };

                    // if no riff refs are using this riff then delete it from the track
                    if found_info.len() == 0 {
                        let mut state = state.clone();
                        match history_manager.lock() {
                            Ok(mut history) => {
                                let action = RiffDelete::new(riff_uuid, track_uuid);
                                match history.apply(&mut state, Box::new(action)) {
                                    Ok(mut daw_events_to_propagate) => {
                                        for _ in 0..(daw_events_to_propagate.len()) {
                                            let event = daw_events_to_propagate.remove(0);
                                            let _ = tx_from_ui.send(event);
                                        }
                                    }
                                    Err(error) => {
                                        error!("Main - rx_ui processing loop - riff delete - error: {}", error);
                                    }
                                }
                            }
                            Err(error) => {
                                error!("Main - rx_ui processing loop - riff delete - error getting lock for history manager: {}", error);
                            }
                        }
                    }
                    else {
                        let mut error_message = String::from("Could not delete riff:\n");

                        for message in found_info.iter() {
                            error_message.push_str(message.as_str());
                            error_message.push_str("\n");
                        }

                        let _ = tx_from_ui.send(DAWEvents::Notification(NotificationType::Error, error_message));
                    }
                }
                TrackChangeType::RiffLengthChange(riff_uuid, riff_length) => {
                    match state.lock() {
                        Ok(state) => {
                            let mut state = state;
                            match track_uuid {
                                Some(track_uuid) => {
                                    state.set_selected_track(Some(track_uuid.clone()));
                                    state.set_selected_riff_uuid(track_uuid.clone(), riff_uuid.clone());
                                    state.set_selected_riff_ref_uuid(None);

                                    match state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                                        Some(track) => for riff in track.riffs_mut().iter_mut() {
                                            if riff.uuid().to_string() == riff_uuid {
                                                riff.set_length(riff_length);
                                                break;
                                            }
                                        },
                                        None => ()
                                    }
                                },
                                None => debug!("Main - rx_ui processing loop - track_riff_edit - no track number specified."),
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - track_riff_edit - could not get lock on state"),
                    };
                    gui.ui.piano_roll_drawing_area.queue_draw();
                    gui.ui.track_drawing_area.queue_draw();
                },
                TrackChangeType::RiffReferenceAdd(track_index, position) => {
                    let mut selected_riff_uuid = None;
                    match state.lock() {
                        Ok(state) => {
                            let song = state.project().song();
                            let tracks = song.tracks();

                            match tracks.get(track_index as usize) {
                                Some(track) => selected_riff_uuid = state.selected_riff_uuid(track.uuid().to_string()),
                                None => debug!("Main - rx_ui processing loop - track riff reference added - no track at index."),
                            };
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - track_riff_edit - could not get lock on state"),
                    };
                    match state.lock() {
                        Ok(mut state) => {
                            let song = state.get_project().song_mut();
                            let tracks = song.tracks_mut();

                            match tracks.get_mut(track_index as usize) {
                                Some(track) => match selected_riff_uuid {
                                    Some(riff_uuid) => {
                                        for riff in track.riffs().iter() {
                                            if riff.uuid().to_string() == riff_uuid {
                                                let riff_ref = RiffReference::new(riff_uuid, position);
                                                track.riff_refs_mut().push(riff_ref);
                                                break;
                                            }
                                        }
                                    },
                                    None => debug!("Main - rx_ui processing loop - track riff reference added - no selected riff index."),
                                },
                                None => debug!("Main - rx_ui processing loop - track riff reference added - no track at index."),
                            };

                            // re-calculate the song length
                            song.recalculate_song_length();
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - track_riff_edit - could not get lock on state"),
                    };
                    // need to calculate exactly where the riff reference has been added and only paint that area
                    // gui.ui.track_drawing_area.queue_draw_area(500, 338, 100, 100);
                    gui.ui.track_drawing_area.queue_draw();
                },
                TrackChangeType::RiffReferenceDelete(track_index, position) => {
                    match state.lock() {
                        Ok(state) => {
                            {
                                let mut state = state;
                                let song = state.get_project().song_mut();
                                let tempo = song.tempo();
                                let tracks = song.tracks_mut();

                                match tracks.get_mut(track_index as usize) {
                                    Some(track) => {
                                        //debug!("Selected track riff ref count: {}", track.riff_refs().len());
                                        let riffs = {
                                            let mut riffs = vec![];
                                            track.riffs_mut().iter_mut().for_each(|riff| { riffs.push(riff.clone()) });
                                            riffs
                                        };
                                        track.riff_refs_mut().retain(|riff_ref| {
                                            let riff_uuid = riff_ref.linked_to();
                                            let mut retain = true;
                                            for riff in riffs.iter() {
                                                if riff.uuid().to_string() == riff_uuid {
                                                    let riff_length = riff.length();
                                                    if riff_ref.position() <= position &&
                                                        position <= (riff_ref.position() + riff_length / tempo * 60.0) {
                                                        retain = false;
                                                    } else {
                                                        retain = true;
                                                    }
                                                    break;
                                                }
                                            }
                                            retain
                                        });
                                    },
                                    None => (),
                                }

                                // re-calculate the song length
                                song.recalculate_song_length();
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - riff reference delete - could not get lock on state"),
                    };
                    // need to calculate exactly where the riff reference has been added and only paint that area
                    // gui.ui.track_drawing_area.queue_draw_area(500, 338, 100, 100);
                    gui.ui.track_drawing_area.queue_draw();
                },
                TrackChangeType::RiffAddNote(note, position, duration) => {
                    {
                        let mut state = state.clone();
                        match history_manager.lock() {
                            Ok(mut history) => {
                                let action = RiffAddNoteAction::new(position, note, 127, duration, &mut state.clone());
                                if let Err(error) = history.apply(&mut state, Box::new(action)) {
                                    error!("Main - rx_ui processing loop - riff add note - error: {}", error);
                                } else {
                                    // refresh UI
                                    gui.ui.track_drawing_area.queue_draw();
                                    gui.ui.piano_roll_drawing_area.queue_draw();
                                }
                            }
                            Err(error) => {
                                error!("Main - rx_ui processing loop - riff add note - error getting lock for history manager: {}", error);
                            }
                        }
                    }
                    let mut midi_channel = 0;
                    let mut tempo = 140.0;
                    match state.lock() {
                        Ok(state) => {
                            tempo = state.project().song().tempo();
                            let track_uuid = state.selected_track();
                            match track_uuid {
                                Some(track_uuid) => {
                                    match state.project().song().tracks().iter().find(|track| track.uuid().to_string() == track_uuid) {
                                        Some(track) => {
                                            midi_channel = if let TrackType::MidiTrack(midi_track) = track {
                                                midi_track.midi_device().midi_channel()
                                            } else {
                                                0
                                            };
                                            state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::PlayNoteImmediate(note, midi_channel));

                                        },
                                        None => debug!("Play note immediate: Could not find track number."),
                                    }
                                },
                                None => debug!("Play note immediate: no track number given."),
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - play note immediate - could not get lock on state"),
                    };
                    {
                        let state_arc = state.clone();
                        let _ = thread::Builder::new().name("Riff add note".into()).spawn(move || {
                            thread::sleep(Duration::from_millis((duration * 60.0 / tempo * 1000.0) as u64));
                            match state_arc.lock() {
                                Ok(state) => {
                                    match state.selected_track() {
                                        Some(track_uuid) => {
                                            state.send_to_track_background_processor(track_uuid, TrackBackgroundProcessorInwardEvent::StopNoteImmediate(note, midi_channel));
                                        }
                                        None => {}
                                    }
                                }
                                Err(_) => {}
                            }
                        });
                    }
                }
                TrackChangeType::RiffDeleteNote(note_number, position) => {
                    {
                        let mut state = state.clone();
                        match history_manager.lock() {
                            Ok(mut history) => {
                                let action = RiffDeleteNoteAction::new(position, note_number, &mut state.clone());
                                if let Err(error) = history.apply(&mut state, Box::new(action)) {
                                    error!("Main - rx_ui processing loop - riff delete note - error: {}", error);
                                } else {
                                    // refresh UI
                                    gui.ui.track_drawing_area.queue_draw();
                                    gui.ui.piano_roll_drawing_area.queue_draw();
                                }
                            }
                            Err(error) => {
                                error!("Main - rx_ui processing loop - riff delete note - error getting lock for history manager: {}", error);
                            }
                        }
                    }
                }
                TrackChangeType::RiffAddSample(sample_reference_uuid, position) => {
                    let mut selected_riff_uuid = None;
                    let mut selected_riff_track_uuid = None;
                    match state.lock() {
                        Ok(state) => {
                            selected_riff_track_uuid = state.selected_track();

                            match selected_riff_track_uuid {
                                Some(track_uuid) => {
                                    selected_riff_uuid = state.selected_riff_uuid(track_uuid.clone());
                                    selected_riff_track_uuid = Some(track_uuid);
                                },
                                None => (),
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - riff add sample - could not get lock on state"),
                    };
                    match state.lock() {
                        Ok(state) => {
                            let mut state = state;

                            match selected_riff_track_uuid {
                                Some(track_uuid) => {
                                    for track in state.get_project().song_mut().tracks_mut().iter_mut() {
                                        match selected_riff_uuid.clone() {
                                            Some(riff_uuid) => {
                                                for riff in track.riffs_mut().iter_mut() {
                                                    if riff.uuid().to_string() == *riff_uuid {
                                                        riff.events_mut().push(TrackEvent::Sample(SampleReference::new(position, sample_reference_uuid.clone())));
                                                        break;
                                                    }
                                                }
                                            }
                                            None => debug!("Main - rx_ui processing loop - riff add sample - problem getting selected riff index"),
                                        }
                                    }

                                    // FIXME - this only needs to happen once per sample_data not every time it is added to a riff reference
                                    // find the sample and then the sample data
                                    if let Some(sample) = state.project().song().samples().get(&sample_reference_uuid) {
                                        if let Some(sample_data) = state.sample_data().get(&sample.sample_data_uuid().to_string()) {
                                            // send the sample data to the track background processor
                                            state.send_to_track_background_processor(track_uuid, TrackBackgroundProcessorInwardEvent::SetSample(sample_data.clone()));
                                        }
                                    }
                                },
                                None => debug!("Main - rx_ui processing loop - riff add sample  - problem getting selected riff track number"),
                            };
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - riff add sample - could not get lock on state"),
                    };
                    gui.ui.sample_roll_drawing_area.queue_draw();
                    gui.ui.track_drawing_area.queue_draw();
                }
                TrackChangeType::RiffDeleteSample(sample_reference_uuid, position) => {
                    debug!("Main - rx_ui processing loop - riff delete sample: sample_reference_uuid={}, position={}", sample_reference_uuid, position);
                    let mut selected_riff_uuid = None;
                    let mut selected_riff_track_uuid = None;
                    match state.lock() {
                        Ok(state) => {
                            selected_riff_track_uuid = state.selected_track();

                            match selected_riff_track_uuid {
                                Some(track_uuid) => {
                                    selected_riff_uuid = state.selected_riff_uuid(track_uuid.clone());
                                    selected_riff_track_uuid = Some(track_uuid);
                                },
                                None => (),
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - riff delete sample - could not get lock on state"),
                    };
                    match state.lock() {
                        Ok(state) => {
                            let mut state = state;

                            match selected_riff_track_uuid {
                                Some(_track_uuid) => {
                                    for track in state.get_project().song_mut().tracks_mut().iter_mut() {
                                        match selected_riff_uuid.clone() {
                                            Some(riff_uuid) => {
                                                for riff in track.riffs_mut().iter_mut() {
                                                    if riff.uuid().to_string() == *riff_uuid {
                                                        debug!("Main - rx_ui processing loop - riff delete sample - found the riff");
                                                        riff.events_mut().retain(|event| match event {
                                                            TrackEvent::Sample(sample) => !((sample.position() - 0.01) <= position && position <= (sample.position() + 0.25)),
                                                            _ => true,
                                                        });
                                                    }
                                                    break;
                                                }
                                            }
                                            None => debug!("Main - rx_ui processing loop - riff delete sample - problem getting selected riff index"),
                                        }
                                    }
                                },
                                None => debug!("Main - rx_ui processing loop - riff delete sample  - problem getting selected riff track number"),
                            };
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - riff delete sample - could not get lock on state"),
                    };
                    gui.ui.sample_roll_drawing_area.queue_draw();
                    gui.ui.track_drawing_area.queue_draw();
                }
                TrackChangeType::RiffSelect(riff_id) => {
                    // let state = state.clone();
                    // let tx_from_ui = tx_from_ui.clone();
                    // std::thread::run(move || {
                    let mut riff_id_final = riff_id.clone();
                    match state.lock() {
                        Ok(state) => {
                            let mut state = state;
                            match track_uuid {
                                Some(track_uuid) => {
                                    state.set_selected_track(Some(track_uuid.clone()));
                                    state.set_selected_riff_ref_uuid(None);
                                    //find the riff id via a riff set - alternate path
                                    let riff_id_option = if let Some(riff_set) = state.project().song().riff_sets().iter().find(|riff_set| riff_set.uuid() == riff_id) {
                                        if let Some((_, riff_ref)) = riff_set.riff_refs().iter().find(|(current_track_uuid, _riff_ref)| current_track_uuid.to_string() == track_uuid) {
                                            Some(riff_ref.linked_to())
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    };
                                    // map(|riff_set| riff_set.riff_refs().get(&track_uuid))
                                    for track in state.get_project().song_mut().tracks_mut().iter_mut() {
                                        if track.uuid().to_string() == track_uuid.clone() {
                                            if let TrackType::AudioTrack(_) = track {
                                                match tx_from_ui.send(DAWEvents::SampleRollSetTrackName(track.name().to_string())) {
                                                    Ok(_) => (),
                                                    Err(_) => (),
                                                }
                                            } else {
                                                match tx_from_ui.send(DAWEvents::PianoRollSetTrackName(track.name().to_string())) {
                                                    Ok(_) => (),
                                                    Err(_) => (),
                                                }
                                            }
                                            let riff_option = if let Some(riff_id) = riff_id_option {
                                                riff_id_final = riff_id.clone();
                                                track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_id)
                                            } else {
                                                track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_id)
                                            };
                                            if let Some(riff) = riff_option {
                                                if let TrackType::AudioTrack(_) = track {
                                                    match tx_from_ui.send(DAWEvents::SampleRollSetRiffName(riff.name().to_string())) {
                                                        Ok(_) => (),
                                                        Err(_) => (),
                                                    }
                                                } else {
                                                    match tx_from_ui.send(DAWEvents::PianoRollSetRiffName(riff.name().to_string())) {
                                                        Ok(_) => (),
                                                        Err(_) => (),
                                                    }
                                                }
                                                break;
                                            }
                                            break;
                                        }
                                    }

                                    state.set_selected_riff_uuid(track_uuid, riff_id_final);
                                },
                                None => debug!("Main - rx_ui processing loop - track_riff_selected - no track number specified."),
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - track_riff_selected - could not get lock on state"),
                    };
                    // });
                    gui.ui.piano_roll_drawing_area.queue_draw();
                },
                TrackChangeType::RiffEventsSelected(x, y, x2, y2, add_to_select) => {
                    let mut selected = Vec::new();
                    let mut selected_riff_uuid = None;
                    let mut selected_riff_track_uuid = None;
                    match state.lock() {
                        Ok(state) => {
                            selected_riff_track_uuid = state.selected_track();

                            match selected_riff_track_uuid {
                                Some(track_uuid) => {
                                    selected_riff_uuid = state.selected_riff_uuid(track_uuid.clone());
                                    selected_riff_track_uuid = Some(track_uuid);
                                },
                                None => (),
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - riff events selected - could not get lock on state"),
                    };
                    match state.lock() {
                        Ok(state) => {
                            let mut state = state;
            
                            match selected_riff_track_uuid {
                                Some(track_uuid) => {
                                    match state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                                        Some(track) => {
                                            match selected_riff_uuid {
                                                Some(riff_uuid) => {
                                                    for riff in track.riffs_mut().iter_mut() {
                                                        if riff.uuid().to_string() == *riff_uuid {
                                                            // store the notes for an undo
                                                            for track_event in riff.events_mut().iter_mut() {
                                                                if let TrackEvent::Note(note) = track_event {
                                                                    if y <= note.note() && note.note() <= y2 && x <= note.position() && (note.position() + note.length()) <= x2 {
                                                                        debug!("Note selected: x={}, y={}, x2={}, y2={}, note position={}, note={}, note duration={}", x, y, x2, y2, note.position(), note.note(), note.length());
                                                                        selected.push(note.id());
                                                                    }
                                                                }
                                                            }
                                                            break;
                                                        }
                                                    }
                                                },
                                                None => debug!("Main - rx_ui processing loop - riff events selected - problem getting selected riff index"),
                                            }
                                        },
                                        None => ()
                                    }
                                },
                                None => debug!("Main - rx_ui processing loop - riff events selected  - problem getting selected riff track number"),
                            };

                            if !selected.is_empty() {
                                let mut state = state;
                                if !add_to_select {
                                    state.selected_riff_events_mut().clear();
                                }
                                state.selected_riff_events_mut().append(&mut selected);
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - riff events selected - could not get lock on state"),
                    };
                }
                TrackChangeType::RiffCutSelected(x, y, x2, y2) => {
                    let mut selected_riff_uuid = None;
                    let mut selected_riff_track_uuid = None;
                    let mut selected_riff_events = vec![];

                    match state.lock() {
                        Ok(mut state) => {
                            selected_riff_track_uuid = state.selected_track();

                            match selected_riff_track_uuid {
                                Some(track_uuid) => {
                                    selected_riff_uuid = state.selected_riff_uuid(track_uuid.clone());
                                    selected_riff_track_uuid = Some(track_uuid);
                                    selected_riff_events = state.selected_riff_events_mut().clone();
                                },
                                None => (),
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - riff cut selected notes - could not get lock on state"),
                    };
                    {
                        let mut state = state.clone();
                        match history_manager.lock() {
                            Ok(mut history) => {
                                let action = RiffCutSelectedAction::new(selected_riff_track_uuid, selected_riff_uuid, selected_riff_events);
                                if let Err(error) = history.apply(&mut state, Box::new(action)) {
                                    error!("Main - rx_ui processing loop - riff cut selected notes - error: {}", error);
                                } else {
                                    // refresh UI
                                    gui.ui.track_drawing_area.queue_draw();
                                    gui.ui.piano_roll_drawing_area.queue_draw();
                                }
                            }
                            Err(error) => {
                                error!("Main - rx_ui processing loop - riff cut selected notes - error getting lock for history manager: {}", error);
                            }
                        }
                    }
                },
                TrackChangeType::RiffTranslateSelected(translation_entity_type, translate_direction, x, y, x2, y2) => {
                    let mut selected_riff_uuid = None;
                    let mut selected_riff_track_uuid = None;
                    let mut selected_riff_events = vec![];

                    match state.lock() {
                        Ok(mut state) => {
                            selected_riff_track_uuid = state.selected_track();

                            match selected_riff_track_uuid {
                                Some(track_uuid) => {
                                    selected_riff_uuid = state.selected_riff_uuid(track_uuid.clone());
                                    selected_riff_track_uuid = Some(track_uuid);
                                    selected_riff_events = state.selected_riff_events_mut().clone();
                                },
                                None => (),
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - riff translate selected - could not get lock on state"),
                    };
                    {
                        let mut state = state.clone();
                        match history_manager.lock() {
                            Ok(mut history) => {
                                let mut snap_in_beats = 1.0;
                                match gui.piano_roll_grid() {
                                    Some(piano_roll_grid) => match piano_roll_grid.lock() {
                                        Ok(piano_roll) => snap_in_beats = piano_roll.snap_position_in_beats(),
                                        Err(_) => (),
                                    },
                                    None => (),
                                }
                                let action = RiffTranslateSelectedAction::new(
                                    selected_riff_track_uuid,
                                    selected_riff_uuid,
                                    selected_riff_events,
                                    translation_entity_type,
                                    translate_direction,
                                    snap_in_beats
                                );
                                if let Err(error) = history.apply(&mut state, Box::new(action)) {
                                    error!("Main - rx_ui processing loop - riff translate selected - error: {}", error);
                                } else {
                                    // refresh UI
                                    gui.ui.track_drawing_area.queue_draw();
                                    gui.ui.piano_roll_drawing_area.queue_draw();
                                }
                            }
                            Err(error) => {
                                error!("Main - rx_ui processing loop - riff translate selected - error getting lock for history manager: {}", error);
                            }
                        }
                    }
                }
                TrackChangeType::Record(_record) => {
                    // TODO implement arming of tracks for recording into
                    // match state.lock() {
                    //     Ok(mut state) => {
                    //         // state.set_recording(record);
                    //     },
                    //     Err(_) => debug!("Main - rx_ui processing loop - transport goto start - could not get lock on state"),
                    // }
                }
                TrackChangeType::RiffQuantiseSelected(x, y, x2, y2) => {
                    let mut selected_riff_uuid = None;
                    let mut selected_riff_track_uuid = None;
                    let mut selected_riff_events = vec![];
                    match state.lock() {
                        Ok(mut state) => {
                            selected_riff_track_uuid = state.selected_track();

                            match selected_riff_track_uuid {
                                Some(track_uuid) => {
                                    selected_riff_uuid = state.selected_riff_uuid(track_uuid.clone());
                                    selected_riff_track_uuid = Some(track_uuid);
                                    selected_riff_events = state.selected_riff_events_mut().clone();
                                },
                                None => (),
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - riff quantise - could not get lock on state"),
                    }
                    {
                        let mut state = state.clone();
                        match history_manager.lock() {
                            Ok(mut history) => {
                                let mut snap_in_beats = 1.0;
                                match gui.piano_roll_grid() {
                                    Some(piano_roll_grid) => match piano_roll_grid.lock() {
                                        Ok(piano_roll) => snap_in_beats = piano_roll.snap_position_in_beats(),
                                        Err(_) => (),
                                    },
                                    None => (),
                                }
                                let action = RiffQuantiseSelectedAction::new(
                                    selected_riff_events,
                                    selected_riff_track_uuid,
                                    selected_riff_uuid,
                                    snap_in_beats
                                );
                                if let Err(error) = history.apply(&mut state, Box::new(action)) {
                                    error!("Main - rx_ui processing loop - riff translate selected - error: {}", error);
                                } else {
                                    // refresh UI
                                    gui.ui.track_drawing_area.queue_draw();
                                    gui.ui.piano_roll_drawing_area.queue_draw();
                                }
                            }
                            Err(error) => {
                                error!("Main - rx_ui processing loop - riff translate selected - error getting lock for history manager: {}", error);
                            }
                        }
                    }
                    gui.ui.piano_roll_drawing_area.queue_draw();
                    gui.ui.track_drawing_area.queue_draw();
                },
                TrackChangeType::RiffCopySelected(x, y, x2, y2) => {
                    let mut selected_riff_uuid = None;
                    let mut selected_riff_track_uuid = None;
                    match state.lock() {
                        Ok(state) => {
                            selected_riff_track_uuid = state.selected_track();

                            match selected_riff_track_uuid {
                                Some(track_uuid) => {
                                    selected_riff_uuid = state.selected_riff_uuid(track_uuid.clone());
                                    selected_riff_track_uuid = Some(track_uuid);
                                },
                                None => (),
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - riff delete note - could not get lock on state"),
                    };
                    let mut copy_buffer: Vec<TrackEvent> = vec![];
                    match state.lock() {
                        Ok(state) => {
                            {
                                let state = state;
                                let selected = state.selected_riff_events().to_vec();

                                match selected_riff_track_uuid {
                                    Some(track_uuid) => {
                                        match state.project().song().tracks().iter().find(|track| track.uuid().to_string() == track_uuid) {
                                            Some(track) => {
                                                match selected_riff_uuid {
                                                    Some(riff_uuid) => {
                                                        for riff in track.riffs().iter() {
                                                            if riff.uuid().to_string() == *riff_uuid {
                                                                riff.events().iter().for_each(|event| match event {
                                                                    TrackEvent::ActiveSense => {},
                                                                    TrackEvent::AfterTouch => {},
                                                                    TrackEvent::ProgramChange => {},
                                                                    TrackEvent::Note(note) => if selected.contains(&note.id()) {
                                                                        copy_buffer.push(event.clone());
                                                                    },
                                                                    TrackEvent::NoteOn(_) => {}
                                                                    TrackEvent::NoteOff(_) => {}
                                                                    TrackEvent::Controller(_) => {}
                                                                    TrackEvent::PitchBend(_pitch_bend) => {}
                                                                    TrackEvent::KeyPressure => {}
                                                                    TrackEvent::AudioPluginParameter(_) => {}
                                                                    TrackEvent::Sample(_sample) => {}
                                                                    TrackEvent::Measure(_) => {}
                                                                    TrackEvent::NoteExpression(_) => {},
                                                                });
                                                                break;
                                                            }
                                                        }
                                                    },
                                                    None => debug!("Main - rx_ui processing loop - riff paste selected - problem getting selected riff index"),
                                                }
                                            },
                                            None => ()
                                        }
                                    },
                                    None => debug!("Main - rx_ui processing loop - riff references paste selected  - problem getting selected riff track number"),
                                };
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - riff copy selected - could not get lock on state"),
                    };

                    match state.lock() {
                        Ok(state) => {
                            let mut edit_cursor_position_in_beats = 0.0;
                            match gui.piano_roll_grid() {
                                Some(piano_roll_grid) => match piano_roll_grid.lock() {
                                    Ok(piano_roll) => {
                                        edit_cursor_position_in_beats = piano_roll.edit_cursor_time_in_beats();
                                    },
                                    Err(_) => (),
                                },
                                None => (),
                            }
                            if !copy_buffer.is_empty() {
                                let mut state = state;
                                state.track_event_copy_buffer_mut().clear();
                                copy_buffer.iter().for_each(|event| {
                                    let value = event.clone();
                                    match value {
                                        TrackEvent::ActiveSense => debug!("TrackChangeType::RiffCopySelectedNotes ActiveSense not yet implemented!"),
                                        TrackEvent::AfterTouch => debug!("TrackChangeType::RiffCopySelectedNotes AfterTouch not yet implemented!"),
                                        TrackEvent::ProgramChange => debug!("TrackChangeType::RiffCopySelectedNotes ProgramChange not yet implemented!"),
                                        TrackEvent::Note(note) => {
                                            let mut note_value = note;
                                            note_value.set_position(note_value.position() - edit_cursor_position_in_beats);
                                            state.track_event_copy_buffer_mut().push(TrackEvent::Note(note_value));
                                        },
                                        TrackEvent::NoteOn(_) => debug!("TrackChangeType::RiffCopySelectedNotes NoteOn not yet implemented!"),
                                        TrackEvent::NoteOff(_) => debug!("TrackChangeType::RiffCopySelectedNotes NoteOff not yet implemented!"),
                                        TrackEvent::Controller(_) => debug!("TrackChangeType::RiffCopySelectedNotes Controller not yet implemented!"),
                                        TrackEvent::PitchBend(_pitch_bend) => debug!("TrackChangeType::RiffCopySelectedNotes PitchBend not yet implemented!"),
                                        TrackEvent::KeyPressure => debug!("TrackChangeType::RiffCopySelectedNotes KeyPressure not yet implemented!"),
                                        TrackEvent::AudioPluginParameter(_) => debug!("TrackChangeType::RiffCopySelectedNotes AudioPluginParameter not yet implemented!"),
                                        TrackEvent::Sample(_sample) => debug!("TrackChangeType::RiffCopySelectedNotes Sample not yet implemented!"),
                                        TrackEvent::Measure(_) => {}
                                        TrackEvent::NoteExpression(_) => {}
                                    }
                                });
                            }
                        },
                        Err(_) => (),
                    }
                },
                TrackChangeType::RiffPasteSelected => {
                    let mut selected_riff_uuid = None;
                    let mut selected_riff_track_uuid = None;
                    match state.lock() {
                        Ok(state) => {
                            selected_riff_track_uuid = state.selected_track();

                            match selected_riff_track_uuid {
                                Some(track_uuid) => {
                                    selected_riff_uuid = state.selected_riff_uuid(track_uuid.clone());
                                    selected_riff_track_uuid = Some(track_uuid);
                                },
                                None => (),
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - riff paste selected notes - could not get lock on state"),
                    };
                    let edit_cursor_position_in_secs = if let Some(piano_roll_grid) = gui.piano_roll_grid() {
                        match piano_roll_grid.lock() {
                            Ok(piano_roll) => {
                                piano_roll.edit_cursor_time_in_beats()
                            },
                            Err(_) => 0.0,
                        }
                    } else {
                        0.0
                    };
                    {
                        let mut state = state.clone();
                        match history_manager.lock() {
                            Ok(mut history) => {
                                let action = RiffPasteSelectedAction::new(selected_riff_track_uuid, selected_riff_uuid, edit_cursor_position_in_secs);
                                if let Err(error) = history.apply(&mut state, Box::new(action)) {
                                    error!("Main - rx_ui processing loop - riff paste selected notes - error: {}", error);
                                } else {
                                    // refresh UI
                                    gui.ui.track_drawing_area.queue_draw();
                                    gui.ui.piano_roll_drawing_area.queue_draw();
                                }
                            }
                            Err(error) => {
                                error!("Main - rx_ui processing loop - riff paste selected notes - error getting lock for history manager: {}", error);
                            }
                        }
                    }
                }
                TrackChangeType::RiffReferenceCutSelected(x, y, x2, y2) => {
                    let mut selected_riff_uuid = None;
                    let mut selected_riff_track_uuid = None;
                    match state.lock() {
                        Ok(state) => {
                            selected_riff_track_uuid = state.selected_track();

                            match selected_riff_track_uuid {
                                Some(track_uuid) => {
                                    selected_riff_uuid = state.selected_riff_uuid(track_uuid.clone());
                                    selected_riff_track_uuid = Some(track_uuid);
                                },
                                None => (),
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - riff delete note - could not get lock on state"),
                    };
                    let edit_cursor_position_in_secs = if let Some(grid) = gui.track_grid() {
                        match grid.lock() {
                            Ok(track_beat_grid) => {
                                track_beat_grid.edit_cursor_time_in_beats()
                            },
                            Err(_) => 0.0,
                        }
                    } else {
                        0.0
                    };
                    let mut copy_buffer: Vec<RiffReference> = vec![];
                    match state.lock() {
                        Ok(state) => {
                            {
                                let mut state = state;

                                match selected_riff_track_uuid {
                                    Some(track_uuid) => {
                                        match state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                                            Some(track) => {
                                                //debug!("Selected track riff ref count: {}", track.riff_refs().len());
                                                let riffs = {
                                                    let mut riffs = vec![];
                                                    track.riffs_mut().iter_mut().for_each(|riff| { riffs.push(riff.clone()) });
                                                    riffs
                                                };
                                                track.riff_refs_mut().retain(|riff_ref| {
                                                    let riff_uuid = riff_ref.linked_to();
                                                    let mut retain = true;
                                                    for riff in riffs.iter() {
                                                        if riff.uuid().to_string() == riff_uuid {
                                                            debug!("x={}, y={}, x2={}, y2={}, riff ref position={}, riff len={}", x, y, x2, y2, riff_ref.position(), riff.length());
                                                            if riff_ref.position() >= x &&
                                                                // (riff_ref.position() + riff.length()) <= x2
                                                                riff_ref.position() <= x2
                                                            {
                                                                debug!("Added a riff ref to the copy buffer.");
                                                                let mut value = riff_ref.clone();
                                                                value.set_position(value.position() - edit_cursor_position_in_secs);
                                                                copy_buffer.push(value);
                                                                retain = false;
                                                            } else {
                                                                retain = true;
                                                            }
                                                            break;
                                                        }
                                                    }
                                                    retain
                                                });
                                            },
                                            None => ()
                                        }
                                    },
                                    None => debug!("Main - rx_ui processing loop - riff references cut selected  - problem getting selected riff track number"),
                                };
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - riff references cut selected - could not get lock on state"),
                    };

                    match state.lock() {
                        Ok(state) => {
                            if !copy_buffer.is_empty() {
                                debug!("Riff references copy buffer length: {}", copy_buffer.len());
                                let mut state = state;
                                state.riff_references_copy_buffer_mut().clear();
                                copy_buffer.iter().for_each(|event| state.riff_references_copy_buffer_mut().push(event.clone()));
                            }
                        },
                        Err(_) => (),
                    }
                    gui.ui.piano_roll_drawing_area.queue_draw();
                    gui.ui.track_drawing_area.queue_draw();
                },
                TrackChangeType::RiffReferenceCopySelected(x, y, x2, y2) => {
                    let edit_cursor_position_in_secs = if let Some(grid) = gui.track_grid() {
                        match grid.lock() {
                            Ok(track_beat_grid) => {
                                track_beat_grid.edit_cursor_time_in_beats()
                            },
                            Err(_) => 0.0,
                        }
                    } else {
                        0.0
                    };
                    let mut copy_buffer: Vec<RiffReference> = vec![];
                    match state.lock() {
                        Ok(state) => {
                            {
                                let state = state;
                                let selected_track_uuid = state.selected_track();

                                match selected_track_uuid {
                                    Some(track_uuid) => {
                                        match state.project().song().tracks().iter().find(|track| track.uuid().to_string() == track_uuid) {
                                            Some(track) => {
                                                debug!("Selected track riff ref count: {}", track.riff_refs().len());
                                                track.riff_refs().iter().for_each(|riff_ref| {
                                                    let riff_uuid = riff_ref.linked_to();
                                                    for riff in track.riffs().iter() {
                                                        if riff.uuid().to_string() == riff_uuid {
                                                            debug!("x={}, y={}, x2={}, y2={}, riff ref position={}, riff len={}", x, y, x2, y2, riff_ref.position(), riff.length());
                                                            if riff_ref.position() >= x &&
                                                                // (riff_ref.position() + riff.length()) <= x2
                                                                riff_ref.position() <= x2
                                                            {
                                                                debug!("Added a riff ref to the copy buffer.");
                                                                let mut value = riff_ref.clone();
                                                                value.set_position(value.position() - edit_cursor_position_in_secs);
                                                                copy_buffer.push(value);
                                                            }
                                                            break;
                                                        }
                                                    }
                                                });
                                            },
                                            None => ()
                                        }
                                    },
                                    None => debug!("Main - rx_ui processing loop - riff references copy selected  - problem getting selected riff track number"),
                                };
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - riff references copy selected - could not get lock on state"),
                    };

                    match state.lock() {
                        Ok(state) => {
                            if !copy_buffer.is_empty() {
                                debug!("Riff references copy buffer length: {}", copy_buffer.len());
                                let mut state = state;
                                state.riff_references_copy_buffer_mut().clear();
                                copy_buffer.iter().for_each(|event| state.riff_references_copy_buffer_mut().push(event.clone()));
                            }
                        },
                        Err(_) => (),
                    }
                },
                TrackChangeType::RiffReferencePaste => {
                    let mut selected_riff_uuid = None;
                    let mut selected_riff_track_uuid = None;
                    match state.lock() {
                        Ok(state) => {
                            selected_riff_track_uuid = state.selected_track();

                            match selected_riff_track_uuid {
                                Some(track_uuid) => {
                                    selected_riff_uuid = state.selected_riff_uuid(track_uuid.clone());
                                    selected_riff_track_uuid = Some(track_uuid);
                                },
                                None => (),
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - riff delete note - could not get lock on state"),
                    };

                    match state.lock() {
                        Ok(state) => {
                            let edit_cursor_position_in_secs = if let Some(track_grid) = gui.track_grid() {
                                match track_grid.lock() {
                                    Ok(grid) => {
                                        grid.edit_cursor_time_in_beats()
                                    },
                                    Err(_) => 0.0,
                                }
                            } else {
                                0.0
                            };
                            let mut copy_buffer: Vec<RiffReference> = vec![];
                            state.riff_references_copy_buffer().iter().for_each(|riff_ref| copy_buffer.push(riff_ref.clone()));
                            let mut state = state;

                            match selected_riff_track_uuid {
                                Some(track_uuid) => {
                                    match state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                                        Some(track) => {
                                            copy_buffer.iter_mut().for_each(|riff_ref| {
                                                let mut riff_ref_copy = riff_ref.clone();
                                                riff_ref_copy.set_position(riff_ref_copy.position() + edit_cursor_position_in_secs);
                                                track.riff_refs_mut().push(riff_ref_copy);
                                            });
                                        },
                                        None => ()
                                    }
                                },
                                None => debug!("Main - rx_ui processing loop - riff references paste selected  - problem getting selected riff track number"),
                            };

                            // re-calculate the song length
                            state.get_project().song_mut().recalculate_song_length();
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - riff references paste selected - could not get lock on state"),
                    };
                    gui.ui.track_drawing_area.queue_draw();
                },
                TrackChangeType::Selected => {
                    match state.lock() {
                        Ok(mut state) => {
                            state.set_selected_track(track_uuid.clone());
                            gui.update_automation_ui_from_state(&mut state);
                            match track_uuid {
                                Some(track_uuid) => {
                                    let riff_uuid = if let Some(riff_uuid) = state.selected_riff_uuid_mut(track_uuid.clone()) {
                                        riff_uuid.clone()
                                    } else {
                                        "".to_string()
                                    };

                                    fn scroll_notes_into_view(gui: &MainWindow, riff: &Riff) {
                                        let events: &Vec<TrackEvent> = riff.events_vec();

                                        // find the lowest and highest note
                                        let mut lowest_note = 0;
                                        let mut highest_note = 127;
                                        for event in events.iter() {
                                            match event {
                                                TrackEvent::Note(note) => {
                                                    if note.note() < lowest_note {
                                                        lowest_note = note.note();
                                                    }
                                                    if note.note() > highest_note {
                                                        highest_note = note.note();
                                                    }
                                                }
                                                _ => {}
                                            }
                                        }

                                        let mid_note = (highest_note - lowest_note) as f64 / 2.0;

                                        let piano_roll_drawing_area_height = gui.ui.piano_roll_drawing_area.height_request() as f64;
                                        let piano_roll_vertical_adj: Adjustment = gui.ui.piano_roll_scrolled_window.vadjustment();
                                        let range_max = piano_roll_vertical_adj.upper();
                                        let entity_height_in_pixels = piano_roll_drawing_area_height / 127.0;
                                        let note_vertical_position_on_grid = piano_roll_drawing_area_height - mid_note * entity_height_in_pixels;
                                        let piano_roll_grid_vertical_scroll_position =  range_max - (note_vertical_position_on_grid/ piano_roll_drawing_area_height * range_max);

                                        piano_roll_vertical_adj.set_value(piano_roll_grid_vertical_scroll_position);
                                    }

                                    for track in state.get_project().song_mut().tracks_mut().iter_mut() {
                                        if track.uuid().to_string() == track_uuid.clone() {
                                            if !riff_uuid.is_empty() {
                                                if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == *riff_uuid) {
                                                    let riff_name = riff.name();

                                                    scroll_notes_into_view(gui, riff);

                                                    if let TrackType::AudioTrack(_) = track {
                                                        gui.set_sample_roll_selected_riff_name_label(riff_name);
                                                    } else {
                                                        gui.set_piano_roll_selected_riff_name_label(riff_name);
                                                    }
                                                } else if let TrackType::AudioTrack(_) = track {
                                                    gui.set_sample_roll_selected_riff_name_label("");
                                                } else {
                                                    gui.set_piano_roll_selected_riff_name_label("");
                                                }
                                            } else if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == *riff_uuid) {
                                                let riff_name = riff.name();

                                                scroll_notes_into_view(gui, riff);

                                                if let TrackType::AudioTrack(_) = track {
                                                    gui.set_sample_roll_selected_riff_name_label(riff_name);
                                                } else {
                                                    gui.set_piano_roll_selected_riff_name_label(riff_name);
                                                }
                                            } else if let TrackType::AudioTrack(_) = track {
                                                gui.set_sample_roll_selected_riff_name_label("");
                                            } else {
                                                gui.set_piano_roll_selected_riff_name_label("");
                                            }
                                            if let TrackType::AudioTrack(_) = track {
                                                gui.set_sample_roll_selected_track_name_label(track.name());
                                            } else {
                                                gui.set_piano_roll_selected_track_name_label(track.name());
                                            }
                                            break;
                                        }
                                    }
                                },
                                None => debug!("Main - rx_ui processing loop - track_riff_selected - no track number specified."),
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - track selected - could not get lock on state"),
                    }
                    gui.ui.piano_roll_drawing_area.queue_draw();
                    gui.ui.automation_drawing_area.queue_draw();
                }
                TrackChangeType::RiffChangeLengthOfSelected(lengthen, x, y, x2, y2) => {
                    let mut selected_riff_uuid = None;
                    let mut selected_riff_track_uuid = None;
                    let mut selected_riff_events = vec![];
                    match state.lock() {
                        Ok(mut state) => {
                            selected_riff_track_uuid = state.selected_track();

                            match selected_riff_track_uuid {
                                Some(track_uuid) => {
                                    selected_riff_uuid = state.selected_riff_uuid(track_uuid.clone());
                                    selected_riff_track_uuid = Some(track_uuid);
                                    selected_riff_events = state.selected_riff_events_mut().clone();
                                },
                                None => (),
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - riff change selected notes length - could not get lock on state"),
                    };
                    {
                        let mut state = state.clone();
                        match history_manager.lock() {
                            Ok(mut history) => {
                                let mut length_increment_in_beats = 1.0;
                                match gui.piano_roll_grid() {
                                    Some(piano_roll_grid) => match piano_roll_grid.lock() {
                                        Ok(piano_roll) => length_increment_in_beats = piano_roll.entity_length_increment_in_beats(),
                                        Err(_) => (),
                                    },
                                    None => (),
                                }
                                let action = RiffChangeLengthOfSelectedAction::new(
                                    selected_riff_track_uuid,
                                    selected_riff_uuid,
                                    selected_riff_events,
                                    length_increment_in_beats,
                                    lengthen,
                                );
                                if let Err(error) = history.apply(&mut state, Box::new(action)) {
                                    error!("Main - rx_ui processing loop - riff change selected notes length - error: {}", error);
                                } else {
                                    // refresh UI
                                    gui.ui.track_drawing_area.queue_draw();
                                    gui.ui.piano_roll_drawing_area.queue_draw();
                                }
                            }
                            Err(error) => {
                                error!("Main - rx_ui processing loop - riff change selected notes length - error getting lock for history manager: {}", error);
                            }
                        }
                    }
                },
                TrackChangeType::RiffNameChange(riff_uuid, name) => {
                    match state.lock() {
                        Ok(state) => {
                            let mut state = state;

                            match track_uuid {
                                Some(track_uuid) => {
                                    match state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                                        Some(track) => {
                                            for riff in track.riffs_mut().iter_mut() {
                                                if riff.uuid().to_string() == *riff_uuid {
                                                    riff.set_name(name);
                                                    break;
                                                }
                                            }
                                        },
                                        None => ()
                                    }
                                },
                                None => debug!("Main - rx_ui processing loop - riff name change - problem getting selected riff track number"),
                            };
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - riff name change - could not get lock on state"),
                    };
                    gui.ui.piano_roll_drawing_area.queue_draw();
                    gui.ui.track_drawing_area.queue_draw();
                },
                TrackChangeType::AutomationSelected(time_lower, value_lower, time_higher, value_higher, add_to_select) => {
                    match state.lock() {
                        Ok(state) => {
                            let automation_view_mode = {
                                match state.automation_view_mode() {
                                    AutomationViewMode::NoteVelocities => AutomationViewMode::NoteVelocities,
                                    AutomationViewMode::Controllers => AutomationViewMode::Controllers,
                                    AutomationViewMode::Instrument => AutomationViewMode::Instrument,
                                    AutomationViewMode::Effect => AutomationViewMode::Effect,
                                    AutomationViewMode::NoteExpression => AutomationViewMode::NoteExpression,
                                }
                            };
                            let automation_type = state.automation_type();
                            let mut state = state;
                            let track_uuid = state.selected_track();
                            let selected_riff_uuid = if let Some(track_uuid) = track_uuid.clone() {
                                state.selected_riff_uuid(track_uuid)
                            }
                            else {
                                None
                            };
                            let selected_effect_plugin_uuid = if let Some(uuid) = state.selected_effect_plugin_uuid() {
                                uuid.clone()
                            }
                            else {
                                "".to_string()
                            };
                            let current_view = state.current_view().clone();
                            let automation_edit_type = state.automation_edit_type();
                            let song = state.project().song();
                            let tracks = song.tracks();

                            let mut selected = Vec::new();

                            match track_uuid {
                                Some(track_uuid) =>
                                    {
                                        match tracks.iter().find(|track| track.uuid().to_string() == track_uuid) {
                                            Some(track_type) => {
                                                let events = if let CurrentView::RiffArrangement = current_view {
                                                    let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                                                        Some(selected_arrangement_uuid.clone())
                                                    } else {
                                                        None
                                                    };

                                                    // get the arrangement
                                                    if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                                                        if let Some(riff_arrangement) = state.project().song().riff_arrangement(selected_arrangement_uuid.clone()) {
                                                            if let Some(riff_arr_automation) = riff_arrangement.automation(&track_uuid) {
                                                                Some(riff_arr_automation.events())
                                                            } else {
                                                                None
                                                            }
                                                        } else {
                                                            None
                                                        }
                                                    } else {
                                                        None
                                                    }
                                                } else {
                                                    match automation_edit_type {
                                                        AutomationEditType::Track => {
                                                            Some(track_type.automation().events())
                                                        }
                                                        AutomationEditType::Riff => {
                                                            if let Some(selected_riff_uuid) = selected_riff_uuid {
                                                                if let Some(riff) = track_type.riffs().iter().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
                                                                    Some(riff.events_vec())
                                                                } else {
                                                                    None
                                                                }
                                                            } else {
                                                                None
                                                            }
                                                        }
                                                    }
                                                };

                                                if let Some(events) = events {
                                                    match automation_view_mode {
                                                        AutomationViewMode::NoteVelocities => {
                                                            for event in events.iter() {
                                                                match event {
                                                                    TrackEvent::Note(note) => {
                                                                        let position = note.position();
                                                                        if time_lower <= position && (position + note.length()) <= time_higher
                                                                            && value_lower <= note.velocity() && note.velocity() <= value_higher {
                                                                            selected.push(note.id());
                                                                        }
                                                                    },
                                                                    _ => {},
                                                                }
                                                            }
                                                        }
                                                        AutomationViewMode::Controllers => {
                                                            if let Some(automation_type_value) = automation_type {
                                                                events.iter().for_each(|event| {
                                                                    match event {
                                                                        TrackEvent::Controller(controller) => {
                                                                            let position = controller.position();
                                                                            if controller.controller() == automation_type_value &&
                                                                                time_lower <= position && position <= time_higher
                                                                                && value_lower <= controller.value() && controller.value() <= value_higher {
                                                                                selected.push(controller.id());
                                                                            }
                                                                        },
                                                                        _ => (),
                                                                    }
                                                                })
                                                            }
                                                        }
                                                        AutomationViewMode::Instrument => {
                                                            if let Some(automation_type_value) = automation_type {
                                                                // get the instrument plugin uuid
                                                                let instrument_plugin_id = if let TrackType::InstrumentTrack(instrument_track) = track_type {
                                                                    instrument_track.instrument().uuid()
                                                                } else {
                                                                    return;
                                                                };

                                                                events.iter().for_each(|event| {
                                                                    match event {
                                                                        TrackEvent::AudioPluginParameter(plugin_param) => {
                                                                            let position = plugin_param.position();
                                                                            if plugin_param.index == automation_type_value &&
                                                                                plugin_param.plugin_uuid.to_string() == instrument_plugin_id.to_string() &&
                                                                                time_lower <= position &&
                                                                                position <= time_higher {
                                                                                selected.push(plugin_param.id());
                                                                            }
                                                                        },
                                                                        _ => (),
                                                                    }
                                                                })
                                                            }
                                                        }
                                                        AutomationViewMode::Effect => {
                                                            if let Some(automation_type_value) = automation_type {
                                                                events.iter().for_each(|event| {
                                                                    match event {
                                                                        TrackEvent::AudioPluginParameter(plugin_param) => {
                                                                            let position = plugin_param.position();
                                                                            if plugin_param.index == automation_type_value &&
                                                                                plugin_param.plugin_uuid.to_string() == selected_effect_plugin_uuid &&
                                                                                time_lower <= position &&
                                                                                position <= time_higher {
                                                                                selected.push(plugin_param.id());
                                                                            }
                                                                        },
                                                                        _ => (),
                                                                    }
                                                                })
                                                            }
                                                        }
                                                        AutomationViewMode::NoteExpression => {}
                                                    }
                                                }
                                            },
                                            None => ()
                                        }
                                    },
                                None => debug!("Main - rx_ui processing loop - automation select - problem getting selected track number"),
                            };

                            if !selected.is_empty() {
                                let mut state = state;
                                if !add_to_select {
                                    state.selected_automation_mut().clear();
                                }
                                state.selected_automation_mut().append(&mut selected);
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - automation select - could not get lock on state"),
                    };
                    gui.ui.track_drawing_area.queue_draw();
                    gui.ui.automation_drawing_area.queue_draw();
                },
                TrackChangeType::AutomationAdd(time, value) => {
                    handle_automation_add(time, value, &state);
                    gui.ui.track_drawing_area.queue_draw();
                    gui.ui.automation_drawing_area.queue_draw();
                },
                TrackChangeType::AutomationDelete(time) => {
                    handle_automation_delete(time, &state);
                    gui.ui.track_drawing_area.queue_draw();
                    gui.ui.automation_drawing_area.queue_draw();
                },
                TrackChangeType::AutomationTranslateSelected(_translation_entity_type, translate_direction, time_lower, _value_lower, time_higher, _value_higher) => {
                    match state.lock() {
                        Ok(state) => {
                            let selected = state.selected_automation().to_vec();
                            let tempo = {
                                state.project().song().tempo()
                            };

                            let controller_view_mode = {
                                match state.automation_view_mode() {
                                    AutomationViewMode::NoteVelocities => AutomationViewMode::NoteVelocities,
                                    AutomationViewMode::Controllers => AutomationViewMode::Controllers,
                                    AutomationViewMode::Instrument => AutomationViewMode::Instrument,
                                    AutomationViewMode::Effect => AutomationViewMode::Effect,
                                    AutomationViewMode::NoteExpression => AutomationViewMode::NoteExpression,
                                }
                            };
                            let automation_type = state.automation_type();
                            let mut state = state;
                            let track_uuid = state.selected_track();
                            let selected_riff_uuid = if let Some(track_uuid) = track_uuid.clone() {
                                state.selected_riff_uuid(track_uuid)
                            }
                            else {
                                None
                            };
                            let song = state.get_project().song_mut();
                            let tracks = song.tracks_mut();

                            let mut snap_in_beats = 1.0;
                            match gui.automation_grid() {
                                Some(controller_grid) => match controller_grid.lock() {
                                    Ok(grid) => snap_in_beats = grid.snap_position_in_beats(),
                                    Err(_) => (),
                                },
                                None => (),
                            }
                            let snap_position_in_secs = snap_in_beats / tempo * 60.0;

                            match track_uuid {
                                Some(track_uuid) =>
                                    {
                                        match tracks.iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                                            Some(track_type) => {
                                                let automation = track_type.automation_mut();
                                                match controller_view_mode {
                                                    AutomationViewMode::NoteVelocities => {
                                                        let mut snap_in_beats = 1.0;
                                                        match &gui.automation_grid {
                                                            Some(automation_grid) => match automation_grid.lock() {
                                                                Ok(grid) => snap_in_beats = grid.snap_position_in_beats(),
                                                                Err(_) => (),
                                                            },
                                                            None => (),
                                                        }

                                                        match selected_riff_uuid {
                                                            Some(riff_uuid) => {
                                                                for riff in track_type.riffs_mut().iter_mut() {
                                                                    if riff.uuid().to_string() == *riff_uuid {
                                                                        for event in riff.events_mut().iter_mut() {
                                                                            match event {
                                                                                TrackEvent::Note(note) => if selected.contains(&note.id()) {
                                                                                    let mut note_velocity = note.velocity();

                                                                                    match translate_direction {
                                                                                        TranslateDirection::Up => {
                                                                                            note_velocity += 1;
                                                                                            if note_velocity > 127 {
                                                                                                note_velocity = 127;
                                                                                            }
                                                                                            note.set_velocity(note_velocity);
                                                                                        }
                                                                                        TranslateDirection::Down => {
                                                                                            note_velocity -= 1;
                                                                                            if note_velocity < 0 {
                                                                                                note_velocity = 0;
                                                                                            }
                                                                                            note.set_velocity(note_velocity);
                                                                                        }
                                                                                        _ => {}
                                                                                    }
                                                                                },
                                                                                _ => {},
                                                                            }
                                                                        }
                                                                        break;
                                                                    }
                                                                }
                                                            },
                                                            None => debug!("Main - rx_ui processing loop - riff changing note velocity - problem getting selected riff index"),
                                                        }
                                                    }
                                                    AutomationViewMode::Controllers => {
                                                        if let Some(automation_type_value) = automation_type {
                                                            automation.events_mut().iter_mut().for_each(|event| {
                                                                match event {
                                                                    TrackEvent::Controller(controller) => {
                                                                        let position = controller.position();
                                                                        if controller.controller() == automation_type_value && selected.contains(&controller.id()) {
                                                                            match translate_direction {
                                                                                TranslateDirection::Up => {
                                                                                    if controller.value() < 127 {
                                                                                        controller.set_value(controller.value() + 1);
                                                                                    }
                                                                                },
                                                                                TranslateDirection::Down => {
                                                                                    if controller.value() > 0 {
                                                                                        controller.set_value(controller.value() - 1);
                                                                                    }
                                                                                },
                                                                                TranslateDirection::Left => {
                                                                                    if position > 0.0 && (position - snap_position_in_secs) >= 0.0 {
                                                                                        controller.set_position(position - snap_position_in_secs);
                                                                                    }
                                                                                },
                                                                                TranslateDirection::Right => {
                                                                                    controller.set_position(position + snap_position_in_secs);
                                                                                },
                                                                            }
                                                                        }
                                                                    },
                                                                    _ => (),
                                                                }
                                                            })
                                                        }
                                                    }
                                                    AutomationViewMode::Instrument => {
                                                        if let Some(automation_type_value) = automation_type {
                                                            automation.events_mut().iter_mut().for_each(|event| {
                                                                match event {
                                                                    TrackEvent::AudioPluginParameter(plugin_param) => {
                                                                        let position = plugin_param.position();
                                                                        if plugin_param.index == automation_type_value && selected.contains(&plugin_param.id()) {
                                                                            match translate_direction {
                                                                                TranslateDirection::Up => {
                                                                                    if plugin_param.value() <= 0.99 {
                                                                                        plugin_param.set_value(plugin_param.value() + 0.01);
                                                                                    }
                                                                                },
                                                                                TranslateDirection::Down => {
                                                                                    if plugin_param.value() >= 0.01 {
                                                                                        plugin_param.set_value(plugin_param.value() - 0.01);
                                                                                    }
                                                                                },
                                                                                TranslateDirection::Left => {
                                                                                    if position > 0.0 && (position - snap_position_in_secs) >= 0.0 {
                                                                                        plugin_param.set_position(position - snap_position_in_secs);
                                                                                    }
                                                                                },
                                                                                TranslateDirection::Right => {
                                                                                    plugin_param.set_position(position + snap_position_in_secs);
                                                                                },
                                                                            }
                                                                        }
                                                                    },
                                                                    _ => (),
                                                                }
                                                            })
                                                        }
                                                    }
                                                    AutomationViewMode::Effect => {}
                                                    AutomationViewMode::NoteExpression => {}
                                                }
                                            },
                                            None => ()
                                        }
                                    },
                                None => debug!("Main - rx_ui processing loop - automation add - problem getting selected track number"),
                            };
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - automation add - could not get lock on state"),
                    };
                    gui.ui.track_drawing_area.queue_draw();
                    gui.ui.automation_drawing_area.queue_draw();
                },
                TrackChangeType::AutomationQuantiseSelected => {
                    match state.lock() {
                        Ok(state) => {
                            let selected = state.selected_automation().to_vec();
                            let controller_view_mode = {
                                match state.automation_view_mode() {
                                    AutomationViewMode::NoteVelocities => AutomationViewMode::NoteVelocities,
                                    AutomationViewMode::Controllers => AutomationViewMode::Controllers,
                                    AutomationViewMode::Instrument => AutomationViewMode::Instrument,
                                    AutomationViewMode::Effect => AutomationViewMode::Effect,
                                    AutomationViewMode::NoteExpression => AutomationViewMode::NoteExpression,
                                }
                            };
                            let automation_type = state.automation_type();
                            let mut state = state;
                            let track_uuid = state.selected_track();

                            let mut snap_in_beats = 1.0;
                            match gui.automation_grid() {
                                Some(controller_grid) => match controller_grid.lock() {
                                    Ok(grid) => snap_in_beats = grid.snap_position_in_beats(),
                                    Err(_) => (),
                                },
                                None => (),
                            }

                            match track_uuid {
                                Some(track_uuid) =>
                                    {
                                        match state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                                            Some(track_type) => {
                                                match track_type {
                                                    TrackType::InstrumentTrack(track) => {
                                                        let automation = track.automation_mut();
                                                        match controller_view_mode {
                                                            AutomationViewMode::NoteVelocities => (),
                                                            AutomationViewMode::Controllers => {
                                                                if let Some(automation_type_value) = automation_type {
                                                                    automation.events_mut().iter_mut().for_each(|event| {
                                                                        match event {
                                                                            TrackEvent::Controller(controller) => {
                                                                                if controller.controller() == automation_type_value && selected.contains(&controller.id()) {
                                                                                    let snap_delta = controller.position() % snap_in_beats;
                                                                                    if (controller.position() - snap_delta) >= 0.0 {
                                                                                        controller.set_position(controller.position() - snap_delta);
                                                                                    }
                                                                                }
                                                                            },
                                                                            _ => (),
                                                                        }
                                                                    })
                                                                }
                                                            }
                                                            AutomationViewMode::Instrument => {
                                                                if let Some(automation_type_value) = automation_type {
                                                                    automation.events_mut().iter_mut().for_each(|event| {
                                                                        match event {
                                                                            TrackEvent::AudioPluginParameter(plugin_param) => {
                                                                                if plugin_param.index == automation_type_value && selected.contains(&plugin_param.id()) {
                                                                                    let snap_delta = plugin_param.position() % snap_in_beats;
                                                                                    if (plugin_param.position() - snap_delta) >= 0.0 {
                                                                                        plugin_param.set_position(plugin_param.position() - snap_delta);
                                                                                    }
                                                                                }
                                                                            },
                                                                            _ => (),
                                                                        }
                                                                    })
                                                                }
                                                            }
                                                            AutomationViewMode::Effect => {}
                                                            AutomationViewMode::NoteExpression => {}
                                                        }
                                                    },
                                                    TrackType::AudioTrack(_) => (),
                                                    TrackType::MidiTrack(_) => (),
                                                }
                                            },
                                            None => ()
                                        }
                                    },
                                None => debug!("Main - rx_ui processing loop - automation add - problem getting selected track number"),
                            };
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - automation add - could not get lock on state"),
                    };
                    gui.ui.track_drawing_area.queue_draw();
                    gui.ui.automation_drawing_area.queue_draw();
                },
                TrackChangeType::AutomationCut => {
                    handle_automation_cut(&state);
                    gui.ui.track_drawing_area.queue_draw();
                    gui.ui.automation_drawing_area.queue_draw();
                },
                TrackChangeType::AutomationCopy => {
                    let edit_cursor_time_in_beats = if let Some(grid) = gui.automation_grid() {
                        match grid.lock() {
                            Ok(grid) => grid.edit_cursor_time_in_beats(),
                            Err(_) => 0.0,
                        }
                    }
                    else { 0.0 };
                    handle_automation_copy(&state, edit_cursor_time_in_beats);
                    gui.ui.track_drawing_area.queue_draw();
                    gui.ui.automation_drawing_area.queue_draw();
                }
                TrackChangeType::AutomationPaste => {
                    let edit_cursor_time_in_beats = if let Some(grid) = gui.automation_grid() {
                        match grid.lock() {
                            Ok(grid) => grid.edit_cursor_time_in_beats(),
                            Err(_) => 0.0,
                        }
                    }
                    else { 0.0 };
                    handle_automation_paste(&state, edit_cursor_time_in_beats);
                    gui.ui.track_drawing_area.queue_draw();
                    gui.ui.automation_drawing_area.queue_draw();
                }
                TrackChangeType::AutomationTypeChange(automation_type) => {
                    match state.lock() {
                        Ok(mut state) => {
                            match automation_type {
                                AutomationChangeData::ParameterType(automation_type) => state.set_automation_type(Some(automation_type)),
                                AutomationChangeData::NoteExpression(note_expression_data) => {
                                    match note_expression_data {
                                        NoteExpressionData::NoteId(id) => state.set_note_expression_id(id),
                                        NoteExpressionData::PortIndex(port_index) => state.set_note_expression_port_index(port_index),
                                        NoteExpressionData::Channel(channel) => state.set_note_expression_channel(channel),
                                        NoteExpressionData::Key(key) => state.set_note_expression_key(key),
                                        NoteExpressionData::Type(exp_type) => state.set_note_expression_type(exp_type),
                                    }
                                }
                            }
                        }
                        Err(_) => debug!("Main - rx_ui processing loop - automation type change - could not get lock on state"),
                    };
                    gui.ui.track_drawing_area.queue_draw();
                    gui.ui.automation_drawing_area.queue_draw();
                },
                TrackChangeType::EffectSelected(effect_uuid) => {
                    match state.lock() {
                        Ok(mut state) => {
                            state.set_selected_effect_plugin_uuid(Some(effect_uuid.clone()));
                            if let Some(uuid) = state.selected_track() {
                                gui.update_automation_effect_parameters_combo(&mut state, uuid, effect_uuid);
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - automation view  effect change - could not get lock on state"),
                    };
                    gui.ui.track_drawing_area.queue_draw();
                    gui.ui.automation_drawing_area.queue_draw();
                },
                TrackChangeType::EffectToggleWindowVisibility(effect_uuid) => {
                    match track_uuid {
                        Some(track_uuid) => {
                            let mut xid = 0;
                            match state.lock() {
                                Ok(mut state) => {
                                    match state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                                        Some(track_type) => {
                                            let track_name = track_type.name().to_string();
                                            match track_type {
                                                TrackType::InstrumentTrack(track) => {
                                                    debug!("track name={}, # of effects={}", track.name(), track.effects().len());
                                                    for effect in track.effects_mut().iter_mut() {
                                                        debug!("effect name={}, effect uuid={}, search for effect uuid={}", effect.name(), effect.uuid(), effect_uuid.as_str());
                                                        if effect.uuid().to_string() == effect_uuid {
                                                            if let Some(window) = audio_plugin_windows.get(&effect_uuid) {
                                                                if window.is_visible() {
                                                                    window.hide();
                                                                } else {
                                                                    window.show_all();
                                                                }
                                                            } else {
                                                                let win = Window::new(WindowType::Toplevel);
                                                                win.set_title(format!("Track: {} - Effect: {}", track_name, effect.name()).as_str());
                                                                win.connect_delete_event(|window, _| {
                                                                    window.hide();
                                                                    gtk::Inhibit(true)
                                                                });
                                                                win.set_height_request(800);
                                                                win.set_width_request(900);
                                                                win.set_resizable(true);
                                                                win.show_all();
                                                                audio_plugin_windows.insert(effect_uuid.clone(), win.clone());

                                                                let window = win.clone();
                                                                {
                                                                    glib::idle_add_local(move || {
                                                                        if window.is_visible() {
                                                                            window.queue_draw();
                                                                        }
                                                                        glib::Continue(true)
                                                                    });
                                                                }

                                                                unsafe {
                                                                    match win.window() {
                                                                        Some(gdk_window) => {
                                                                            xid = gdk_x11_window_get_xid(gdk_window);
                                                                            debug!("xid: {}", xid);
                                                                        },
                                                                        None => debug!("Couldn't get gdk window."),
                                                                    }
                                                                }
                                                            }

                                                            break;
                                                        }
                                                    }
                                                },
                                                TrackType::AudioTrack(_) => (),
                                                TrackType::MidiTrack(_) => (),
                                            }
                                        },
                                        None => ()
                                    }
                                },
                                Err(_) => debug!("Main - rx_ui processing loop - track effect toggle window visibility - could not get lock on state"),
                            };
                            if xid != 0 {
                                match state.lock() {
                                    Ok(state) => {
                                        state.send_to_track_background_processor(track_uuid, TrackBackgroundProcessorInwardEvent::SetEffectWindowId(effect_uuid, xid));
                                    },
                                    Err(_) => debug!("Could not get read only lock on state."),
                                }
                            }
                        },
                        None => (),
                    }
                },
                TrackChangeType::Volume(position, volume) => {
                    debug!("Received volume change: track={}, volume={}", track_uuid.clone().unwrap(), volume);
                    if let Some(track_uuid) = track_uuid {
                        match state.lock() {
                            Ok(mut state) => {
                                let recording = *state.recording_mut();
                                let playing = *state.playing_mut();
                                let play_position_in_frames = state.play_position_in_frames() as f64;
                                let sample_rate = state.get_project().song_mut().sample_rate();
                                let bpm = state.get_project().song_mut().tempo();
                                let play_position_in_beats = play_position_in_frames / sample_rate * bpm / 60.0;
                                let mut midi_channel = 0;

                                for track in state.get_project().song_mut().tracks_mut().iter_mut() {
                                    if track.uuid().to_string() == track_uuid {
                                        if let TrackType::MidiTrack(midi_track) = track {
                                            midi_channel = midi_track.midi_device().midi_channel();
                                        }

                                        if !recording {
                                            track.set_volume(volume);
                                        } else if recording && playing {
                                            if let Some(position) = position {
                                                track.automation_mut().events_mut().push(TrackEvent::Controller(Controller::new(position, 7, (volume * 127.0) as i32)));
                                            } else {
                                                track.automation_mut().events_mut().push(TrackEvent::Controller(Controller::new(play_position_in_beats, 7, (volume * 127.0) as i32)));
                                            }
                                        }
                                        break;
                                    }
                                }
                                state.send_to_track_background_processor(track_uuid.clone(), TrackBackgroundProcessorInwardEvent::Volume(volume));
                                state.send_to_track_background_processor(track_uuid, TrackBackgroundProcessorInwardEvent::PlayControllerImmediate(7, (volume * 127.0) as i32, midi_channel));
                            },
                            Err(_) => debug!("Could not get read only lock on state."),
                        }
                    }
                }
                TrackChangeType::Pan(position, pan) => {
                    debug!("Received pan change: track={}, pan={}", track_uuid.clone().unwrap(), pan);
                    if let Some(track_uuid) = track_uuid {
                        match state.lock() {
                            Ok(mut state) => {
                                let recording = *state.recording_mut();
                                let playing = *state.playing_mut();
                                let play_position_in_frames = state.play_position_in_frames() as f64;
                                let sample_rate = state.get_project().song_mut().sample_rate();
                                let bpm = state.get_project().song_mut().tempo();
                                let play_position_in_beats = play_position_in_frames / sample_rate * bpm / 60.0;
                                let mut midi_channel = 0;

                                for track in state.get_project().song_mut().tracks_mut().iter_mut() {
                                    if let TrackType::MidiTrack(midi_track) = track {
                                        midi_channel = midi_track.midi_device().midi_channel();
                                    }

                                    if track.uuid().to_string() == track_uuid {
                                        if !recording {
                                            track.set_pan(pan);
                                        } else if recording && playing {
                                            if let Some(position) = position {
                                                track.automation_mut().events_mut().push(TrackEvent::Controller(Controller::new(position, 14, (pan * 63.5 + 63.5) as i32)));
                                            } else {
                                                track.automation_mut().events_mut().push(TrackEvent::Controller(Controller::new(play_position_in_beats, 14, (pan * 63.5 + 63.5) as i32)));
                                            }
                                        }
                                        break;
                                    }
                                }
                                state.send_to_track_background_processor(track_uuid.clone(), TrackBackgroundProcessorInwardEvent::Pan(pan));
                                state.send_to_track_background_processor(track_uuid, TrackBackgroundProcessorInwardEvent::PlayControllerImmediate(14, (pan * 63.5 + 63.5) as i32, midi_channel));
                            },
                            Err(_) => debug!("Could not get read only lock on state."),
                        }
                    }
                }
                TrackChangeType::TrackColourChanged(red, green, blue, alpha) => {
                    if let Some(track_uuid) = track_uuid {
                        match state.lock() {
                            Ok(mut state) => {
                                for track in state.get_project().song_mut().tracks_mut().iter_mut() {
                                    if track.uuid().to_string() == track_uuid {
                                        track.set_colour(red, green, blue, alpha);
                                        gui.ui.track_drawing_area.queue_draw();
                                        gui.ui.piano_roll_drawing_area.queue_draw();
                                        gui.ui.sample_roll_drawing_area.queue_draw();
                                        gui.ui.automation_drawing_area.queue_draw();
                                        break;
                                    }
                                }
                            },
                            Err(_) => debug!("Could not get read only lock on state."),
                        }
                    }
                }
                TrackChangeType::RiffColourChanged(uuid, red, green, blue, alpha) => {
                    if let Some(track_uuid) = track_uuid {
                        match state.lock() {
                            Ok(mut state) => {
                                for track in state.get_project().song_mut().tracks_mut().iter_mut() {
                                    if track.uuid().to_string() == track_uuid {
                                        // find the riff and update it
                                        for riff in track.riffs_mut().iter_mut() {
                                            if riff.uuid().to_string() == uuid {
                                                riff.set_colour(Some((red, green, blue, alpha)));
                                                break;
                                            }
                                        }
                                        gui.ui.track_drawing_area.queue_draw();
                                        gui.ui.piano_roll_drawing_area.queue_draw();
                                        gui.ui.sample_roll_drawing_area.queue_draw();
                                        gui.ui.automation_drawing_area.queue_draw();
                                        break;
                                    }
                                }
                            },
                            Err(_) => debug!("Could not get read only lock on state."),
                        }
                    }
                }
                TrackChangeType::CopyTrack => {
                    let state_arc = state.clone();
                    match state.lock() {
                        Ok(mut state) => {
                            // find the track to copy
                            if let Some(track_uuid) = track_uuid {
                                if let Some(track_type) = state.project().song().tracks().iter().find(|track_type| track_type.uuid().to_string() == track_uuid) {
                                    let mut new_track = InstrumentTrack::new();
                                    let mut instrument_track_senders2 = HashMap::new();
                                    let mut instrument_track_receivers2 = HashMap::new();


                                    // copy what is needed from the originating track - a bit difficult to make this clone-able
                                    let mut new_name = "Copy of ".to_string();
                                    new_name.push_str(track_type.name());
                                    new_track.set_name(new_name);
                                    for riff in track_type.riffs().iter() {
                                        if riff.name() != "empty" {
                                            let mut new_riff = riff.clone();
                                            new_riff.set_uuid(Uuid::new_v4());
                                            new_track.riffs_mut().push(new_riff);
                                        }
                                    }

                                    // need to copy the instrument and effect details

                                    let mut new_track_type = TrackType::InstrumentTrack(new_track);


                                    DAWState::init_track(
                                        vst24_plugin_loaders.clone(),
                                        clap_plugin_loaders.clone(),
                                        tx_to_audio.clone(),
                                        track_audio_coast.clone(),
                                        &mut instrument_track_senders2,
                                        &mut instrument_track_receivers2,
                                        &mut  new_track_type,
                                        None,
                                        None,
                                        vst_host_time_info.clone(),
                                    );

                                    state.get_project().song_mut().tracks_mut().push(new_track_type);
                                    state.update_track_senders_and_receivers(instrument_track_senders2, instrument_track_receivers2);

                                    gui.clear_ui();
                                    gui.update_ui_from_state(tx_from_ui.clone(), &mut state, state_arc.clone());
                                }
                            }
                        },
                        Err(_) => todo!(),
                    }
                }
                TrackChangeType::RouteMidiTo(routing) => {
                    match state.lock() {
                        Ok(mut state) => {
                            if let Some(track_from_uuid) = track_uuid {
                                state.send_midi_routing_to_track_background_processors(track_from_uuid.clone(), routing.clone());

                                // add the new routing to the track
                                if let Some(track) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_from_uuid) {
                                    track.midi_routings_mut().push(routing);
                                }
                            }
                        }
                        Err(error) => {
                            debug!("Problem locking state when routing midi to a track: {}", error);
                        }
                    }
                }
                TrackChangeType::RemoveMidiRouting(route_uuid) => {
                    match state.lock() {
                        Ok(mut state) => {
                            if let Some(track_from_uuid) = track_uuid {
                                // get the destination track uuid
                                let destination_track_uuid = if let Some(track) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_from_uuid.clone()) {
                                    'splashdown: {
                                        for index in 0..track.midi_routings().len() {
                                            if let Some(route) = track.midi_routings().get(index) {
                                                if route.uuid() == route_uuid {
                                                    // extract the track uuid from the destination part of the route
                                                    let destination_track_uuid = match &route.destination {
                                                        TrackEventRoutingNodeType::Track(track_uuid) => track_uuid.clone(),
                                                        TrackEventRoutingNodeType::Instrument(track_uuid, _) => track_uuid.clone(),
                                                        TrackEventRoutingNodeType::Effect(track_uuid, _) => track_uuid.clone(),
                                                    };
                                                    
                                                    track.midi_routings_mut().remove(index);
                                                    break 'splashdown Some(destination_track_uuid);
                                                }
                                            }
                                        }
                                        None
                                    }
                                }
                                else {
                                    None
                                };
                                
                                // delete the routing from the source track background processor
                                state.send_to_track_background_processor(track_from_uuid.clone(), TrackBackgroundProcessorInwardEvent::RemoveTrackEventSendRouting(route_uuid.clone()));
                                
                                // delete the routing from the destination track
                                if let Some(destination_track_uuid) = destination_track_uuid {
                                    state.send_to_track_background_processor(destination_track_uuid, TrackBackgroundProcessorInwardEvent::RemoveTrackEventReceiveRouting(route_uuid.clone()));
                                }
                            }
                        }
                        Err(error) => {
                            debug!("Problem locking state when routing midi to a track: {}", error);
                        }
                    }
                }
                TrackChangeType::UpdateMidiRouting(route_uuid, midi_channel, start_note, end_note) => {
                    match state.lock() {
                        Ok(mut state) => {
                            if let Some(track_from_uuid) = track_uuid {
                                // get the destination track uuid
                                let details = if let Some(track) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_from_uuid.clone()) {
                                    'splashdown: {
                                        for index in 0..track.midi_routings().len() {
                                            if let Some(route) = track.midi_routings_mut().get_mut(index) {
                                                if route.uuid() == route_uuid {
                                                    // extract the track uuid from the destination part of the route
                                                    let destination_track_uuid = match &route.destination {
                                                        TrackEventRoutingNodeType::Track(track_uuid) => track_uuid.clone(),
                                                        TrackEventRoutingNodeType::Instrument(track_uuid, _) => track_uuid.clone(),
                                                        TrackEventRoutingNodeType::Effect(track_uuid, _) => track_uuid.clone(),
                                                    };

                                                    route.channel = midi_channel as u8;
                                                    route.note_range = (start_note as u8, end_note as u8);

                                                    break 'splashdown Some((route.clone(), destination_track_uuid));
                                                }
                                            }
                                        }
                                        None
                                    }
                                }
                                else {
                                    None
                                };
                                
                                if let Some((route, destination_track_uuid)) = details {
                                    // delete the routing from the source track background processor
                                    state.send_to_track_background_processor(track_from_uuid.clone(), TrackBackgroundProcessorInwardEvent::UpdateTrackEventSendRouting(route_uuid.clone(), route.clone()));
                                    
                                    // delete the routing from the destination track
                                    state.send_to_track_background_processor(destination_track_uuid, TrackBackgroundProcessorInwardEvent::UpdateTrackEventReceiveRouting(route_uuid.clone(), route));
                                }
                            }
                        }
                        Err(error) => {
                            debug!("Problem locking state when routing midi to a track: {}", error);
                        }
                    }
                }
                TrackChangeType::RouteAudioTo(routing) => {
                    match state.lock() {
                        Ok(mut state) => {
                            if let Some(track_from_uuid) = track_uuid {
                                state.send_audio_routing_to_track_background_processors(track_from_uuid.clone(), routing.clone());

                                // add the new routing to the track
                                if let Some(track) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_from_uuid) {
                                    track.audio_routings_mut().push(routing);
                                }
                            }
                        }
                        Err(error) => {
                            debug!("Problem locking state when routing audio to a track: {}", error);
                        }
                    }
                }
                TrackChangeType::RemoveAudioRouting(route_uuid) => {
                    match state.lock() {
                        Ok(mut state) => {
                            if let Some(track_from_uuid) = track_uuid {
                                // get the destination track uuid
                                let destination_track_uuid = if let Some(track) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_from_uuid.clone()) {
                                    'splashdown: {
                                        for index in 0..track.audio_routings().len() {
                                            if let Some(route) = track.audio_routings().get(index) {
                                                if route.uuid() == route_uuid {
                                                    // extract the track uuid from the destination part of the route
                                                    let destination_track_uuid = match &route.destination {
                                                        AudioRoutingNodeType::Track(track_uuid) => track_uuid.clone(),
                                                        AudioRoutingNodeType::Instrument(track_uuid, _, _, _) => track_uuid.clone(),
                                                        AudioRoutingNodeType::Effect(track_uuid, _, _, _) => track_uuid.clone(),
                                                    };
                                                    
                                                    track.audio_routings_mut().remove(index);
                                                    break 'splashdown Some(destination_track_uuid);
                                                }
                                            }
                                        }
                                        None
                                    }
                                }
                                else {
                                    None
                                };
                                
                                // delete the routing from the source track background processor
                                state.send_to_track_background_processor(track_from_uuid.clone(), TrackBackgroundProcessorInwardEvent::RemoveAudioSendRouting(route_uuid.clone()));
                                
                                // delete the routing from the destination track
                                if let Some(destination_track_uuid) = destination_track_uuid {
                                    state.send_to_track_background_processor(destination_track_uuid, TrackBackgroundProcessorInwardEvent::RemoveAudioReceiveRouting(route_uuid.clone()));
                                }
                            }
                        }
                        Err(error) => {
                            debug!("Problem locking state when routing audio to a track: {}", error);
                        }
                    }
                }
                TrackChangeType::TrackMoveToPosition(move_to_position) => {
                    debug!("Main - rx_ui processing loop - track move to position");
                    if let Some(track_uuid) = track_uuid {
                        let state_arc = state.clone();
                        match state.lock() {
                            Ok(mut state) => {
                                state.get_project().song_mut().track_move_to_position(track_uuid, move_to_position);
                                gui.clear_ui();
                                gui.update_ui_from_state(tx_from_ui.clone(), &mut state, state_arc);
                            },
                            Err(_) => debug!("Main - rx_ui processing loop - track move to position - could not get lock on state"),
                        };
                        gui.ui.riff_sets_box.queue_draw();
                    }
                }
                TrackChangeType::RiffEventChange(original_event_copy, changed_event) => {
                    let mut selected_riff_uuid = None;
                    let mut selected_riff_track_uuid = None;
                    match state.lock() {
                        Ok(state) => {
                            selected_riff_track_uuid = state.selected_track();

                            match selected_riff_track_uuid {
                                Some(track_uuid) => {
                                    selected_riff_uuid = state.selected_riff_uuid(track_uuid.clone());
                                    selected_riff_track_uuid = Some(track_uuid);
                                },
                                None => (),
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - riff translate event - could not get lock on state"),
                    }
                    if let Some(selected_riff_uuid) = selected_riff_uuid {
                        if let Some(selected_riff_track_uuid) = selected_riff_track_uuid {
                            match state.lock() {
                                Ok(mut state) => {
                                    match original_event_copy {
                                        TrackEvent::Note(original_note_copy) => {
                                            if let Some(track) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == selected_riff_track_uuid) {
                                                if let Some(riff) = track.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == selected_riff_uuid) {
                                                    for event in riff.events_mut().iter_mut() {
                                                        if let TrackEvent::Note(note) = event {
                                                            if *note == original_note_copy {
                                                                if let TrackEvent::Note(translated_event_copy) = changed_event {
                                                                    note.set_position(translated_event_copy.position());
                                                                    note.set_note(translated_event_copy.note());
                                                                    note.set_length(translated_event_copy.length());
                                                                    break;
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                                Err(_error) => debug!("Main - rx_ui processing loop - riff translate event - could not get lock on state"),
                            }
                        }
                    }
                    gui.ui.piano_roll_drawing_area.queue_draw();
                }
                TrackChangeType::RiffReferenceChange(original_riff_copy, changed_riff) => {
                    if let Some(track_uuid) = track_uuid {
                        match state.lock() {
                            Ok(mut state) => {
                                if let Some(track) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                                    if let Some(riff) = track.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == original_riff_copy.uuid().to_string()) {
                                        riff.set_length(changed_riff.length());
                                    }
                                    if let Some(riff_ref) = track.riff_refs_mut().iter_mut().find(|riff_ref| riff_ref.uuid().to_string() == changed_riff.uuid().to_string()) {
                                        riff_ref.set_position(changed_riff.position());
                                    }
                                }
                            }
                            Err(_error) => debug!("Main - rx_ui processing loop - riff reference change - could not get lock on state"),
                        }
                    }
                }
                TrackChangeType::TrackDetails(show) => {
                    if let Some(track_uuid) = track_uuid {
                        match state.lock() {
                            Ok(mut state) => {
                                state.set_selected_track(Some(track_uuid.clone()));

                                // TODO this needs to be shown in the UI
                            }
                            Err(_error) => debug!("Main - rx_ui processing loop - track details - could not get lock on state"),
                        }
                        if let Some((_, dialogue)) = gui.track_details_dialogues.iter().find(|(dialogue_track_uuid, _dialogue)| dialogue_track_uuid.to_string() == track_uuid) {
                            if show {
                                dialogue.track_details_dialogue.show_all();
                            } else {
                                dialogue.track_details_dialogue.hide();
                            }
                        }
                    }
                }
                TrackChangeType::UpdateTrackDetails => {
                    if let Some(track_uuid) = track_uuid {
                        match state.lock() {
                            Ok(mut state) => {
                                let midi_input_devices: Vec<String> = state.midi_devices();
                                let mut instrument_plugins: IndexMap<String, String> = IndexMap::new();

                                for (key, value) in state.vst_instrument_plugins().iter() {
                                    instrument_plugins.insert(key.clone(), value.clone());
                                }

                                for (mut track_number, track) in state.get_project().song_mut().tracks_mut().iter_mut().enumerate() {
                                    if track.uuid().to_string() == track_uuid {
                                        let mut track_number = track_number as i32;
                                        gui.update_track_details_dialogue(&midi_input_devices, &mut instrument_plugins, &mut track_number, &track);
                                        break;
                                    }
                                }
                            }
                            Err(_error) => debug!("Main - rx_ui processing loop - update track details - could not get lock on state"),
                        }
                    }
                }
            },
            DAWEvents::TrackEffectParameterChange(_, _) => debug!("Event: TrackEffectParameterChange"),
            DAWEvents::TrackInstrumentParameterChange(_) => debug!("Event: TrackInstrumentParameterChange"),
            DAWEvents::TrackSelectedPatternChange(_, _) => debug!("Event: TrackSelectedPatternChange"),
            DAWEvents::TranslateHorizontalChange(_) => debug!("Event: TranslateHorizontalChange"),
            DAWEvents::TranslateVerticalChange(_) => debug!("Event: TranslateVerticalChange"),
            DAWEvents::TransportChange(_, _, _) => debug!("Event: TransportChange"),
            DAWEvents::ViewAutomationChange(show_automation_events) => debug!("Event: ViewAutomationChange: {}", show_automation_events),
            DAWEvents::ViewNoteChange(show_note_events) => debug!("Event: ViewNoteChange: {}", show_note_events),
            DAWEvents::ViewPanChange(show_pan_events) => debug!("Event: ViewPanChange: {}", show_pan_events),
            DAWEvents::ViewVolumeChange(show_volume_events) => debug!("Event: ViewVolumeChange: {}", show_volume_events),
            DAWEvents::TrackGridOperationModeChange(_) => debug!("Event: TrackGridOperationModeChange"),
            DAWEvents::PianoRollOperationModeChange(_) => debug!("Event: PianoRollOperationModeChange"),
            DAWEvents::ControllerOperationModeChange(_) => debug!("Event: ControllerOperationModeChange"),
            DAWEvents::TransportGotoStart => {
                match state.lock() {
                    Ok(mut state) => {
                        let bpm = state.get_project().song().tempo();
                        let time_signature_numerator = state.get_project().song().time_signature_numerator();
                        let sample_rate = state.get_project().song().sample_rate();
                        let play_position_in_frames = 0.0;
                        let play_position_in_beats = play_position_in_frames / sample_rate * bpm / 60.0;
                        let current_bar = play_position_in_beats as i32 / time_signature_numerator as i32 + 1;
                        let current_beat_in_bar = play_position_in_beats as i32 % time_signature_numerator as i32 + 1;

                        state.set_play_position_in_frames(play_position_in_frames as u32);

                        gui.ui.song_position_txt_ctrl.set_label(format!("{:03}:{:03}:000", current_bar, current_beat_in_bar).as_str());
                        if let Some(piano_roll_grid) = gui.piano_roll_grid() {
                            match piano_roll_grid.lock() {
                                Ok(mut grid) => grid.set_track_cursor_time_in_beats(play_position_in_beats),
                                Err(_) => (),
                            }
                        }
                        if let Some(track_grid) = gui.track_grid() {
                            match track_grid.lock() {
                                Ok(mut grid) => grid.set_track_cursor_time_in_beats(play_position_in_beats),
                                Err(_) => (),
                            }
                        }
                        if let Some(sample_roll_grid) = gui.sample_roll_grid() {
                            match sample_roll_grid.lock() {
                                Ok(mut grid) => grid.set_track_cursor_time_in_beats(play_position_in_beats),
                                Err(_) => (),
                            }
                        }
                        if let Some(automation_grid) = gui.automation_grid() {
                            match automation_grid.lock() {
                                Ok(mut grid) => grid.set_track_cursor_time_in_beats(play_position_in_beats),
                                Err(_) => (),
                            }
                        }

                        let song = state.project().song();
                        let tracks = song.tracks();
                        for track in tracks {
                            state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::GotoStart);
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - transport goto start - could not get lock on state"),
                }
                gui.ui.piano_roll_drawing_area.queue_draw();
                gui.ui.sample_roll_drawing_area.queue_draw();
                gui.ui.track_drawing_area.queue_draw();
                gui.ui.automation_drawing_area.queue_draw();
            }
            DAWEvents::TransportMoveBack => {
                debug!("Main - rx_ui processing loop - transport move back - received");
                match state.lock() {
                    Ok(mut state) => {
                        let bpm = state.get_project().song().tempo();
                        let sample_rate = state.get_project().song().sample_rate();
                        let block_size = state.get_project().song().block_size();
                        let time_signature_numerator = state.project().song().time_signature_numerator();
                        let beats_per_bar = time_signature_numerator;
                        let mut play_position_in_frames = state.play_position_in_frames();
                        let frames_per_beat = sample_rate * 60.0 /bpm;
                        let frames_in_measure = (frames_per_beat * beats_per_bar) as u32;

                        if play_position_in_frames >= frames_in_measure {
                            play_position_in_frames -= frames_in_measure;
                        }
                        else {
                            play_position_in_frames = 0;
                        }
                        state.set_play_position_in_frames(play_position_in_frames);

                        {
                            let state = state;
                            for track in state.project().song().tracks().iter() {
                                state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetBlockPosition((play_position_in_frames / (block_size as u32)) as i32));
                            }
                        }

                        let play_position_in_beats = play_position_in_frames as f64 / sample_rate * bpm / 60.0;
                        let current_bar = play_position_in_beats as i32 / time_signature_numerator as i32 + 1;
                        let current_beat_in_bar = play_position_in_beats as i32 % time_signature_numerator as i32 + 1;

                        gui.ui.song_position_txt_ctrl.set_label(format!("{:03}:{:03}:000", current_bar, current_beat_in_bar).as_str());
                        if let Some(piano_roll_grid) = gui.piano_roll_grid() {
                            match piano_roll_grid.lock() {
                                Ok(mut grid) => grid.set_track_cursor_time_in_beats(play_position_in_beats),
                                Err(_) => (),
                            }
                        }
                        if let Some(track_grid) = gui.track_grid() {
                            match track_grid.lock() {
                                Ok(mut grid) => grid.set_track_cursor_time_in_beats(play_position_in_beats),
                                Err(_) => (),
                            }
                        }
                        if let Some(sample_roll_grid) = gui.sample_roll_grid() {
                            match sample_roll_grid.lock() {
                                Ok(mut grid) => grid.set_track_cursor_time_in_beats(play_position_in_beats),
                                Err(_) => (),
                            }
                        }
                        if let Some(automation_grid) = gui.automation_grid() {
                            match automation_grid.lock() {
                                Ok(mut grid) => grid.set_track_cursor_time_in_beats(play_position_in_beats),
                                Err(_) => (),
                            }
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - play position in beats - could not get lock on state"),
                }

                gui.ui.piano_roll_drawing_area.queue_draw();
                gui.ui.sample_roll_drawing_area.queue_draw();
                gui.ui.track_drawing_area.queue_draw();
                gui.ui.automation_drawing_area.queue_draw();
            }
            DAWEvents::TransportStop => {
                // set some sensible defaults
                let mut bpm = 140.0;
                let mut sample_rate = 44100.0;
                let mut block_size = 1024.0;
                let mut song_length_in_beats = 400.0;

                match state.lock() {
                    Ok(mut state) => {
                        song_length_in_beats = *state.get_project().song_mut().length_in_beats_mut() as f64;
                        state.set_playing(false);
                        if let Some(playing_riff_set_uuid) = state.playing_riff_set() {
                            gui.repaint_riff_set_view_riff_set_active_drawing_areas(playing_riff_set_uuid, 0.0);
                            state.set_playing_riff_set(None);
                        }
                        if let Some(playing_riff_sequence_uuid) = state.playing_riff_sequence() {
                            let playing_riff_sequence_summary_data = (0.0, vec![]);
                            gui.repaint_riff_sequence_view_riff_sequence_active_drawing_areas(playing_riff_sequence_uuid, 0.0, &playing_riff_sequence_summary_data);
                            state.set_playing_riff_sequence(None);
                        }
                        if let Some(playing_riff_arrangement_uuid) = state.playing_riff_arrangement() {
                            let playing_riff_arrangement_summary_data = (0.0, vec![]);
                            gui.repaint_riff_arrangement_view_riff_arrangement_active_drawing_areas(playing_riff_arrangement_uuid, 0.0, &playing_riff_arrangement_summary_data);
                            state.set_playing_riff_arrangement(None);
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - transport stop - could not get lock on state"),
                };
                match state.lock() {
                    Ok(state) => {
                        let song = state.project().song();
                        let tracks = song.tracks();
                        bpm = song.tempo();
                        sample_rate = song.sample_rate();
                        block_size = song.block_size();
                        for track in tracks {
                            state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Stop);
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - transport stop - could not get lock on state"),
                };
                let number_of_blocks = (song_length_in_beats / bpm * 60.0 * sample_rate / block_size) as i32;
                match tx_to_audio.send(AudioLayerInwardEvent::Play(false, number_of_blocks, 0)) {
                    Ok(_) => (),
                    Err(error) => debug!("Problem using tx_to_audio to send message to jack layer when stopping play: {}", error),
                }
            }
            DAWEvents::TransportPlay => {
                match state.lock() {
                    Ok(mut state) => {

                        {
                            let mut time_info = vst_host_time_info.write();
                            time_info.sample_pos = 0.0;
                        }

                        let track_riffs_stack_visible_name = gui.get_track_riffs_stack_visible_name();
                        if track_riffs_stack_visible_name == "Track Grid" {
                            state.play_song(tx_to_audio);
                        } else if track_riffs_stack_visible_name == "Riffs" {
                            let riffs_stack_visible_name = gui.get_riffs_stack_visible_name();
                            if riffs_stack_visible_name == "riff_sets" {
                                let riff_set_uuid = if let Some(riff_set) = state.get_project().song_mut().riff_sets_mut().get_mut(0) {
                                    riff_set.uuid()
                                } else {
                                    "".to_string()
                                };
                                state.play_riff_set(tx_to_audio, riff_set_uuid);
                            } else if riffs_stack_visible_name == "riff_sequences" {
                                let riff_sequence_uuid = if let Some(riff_sequence) = state.get_project().song_mut().riff_sequences_mut().get_mut(0) {
                                    riff_sequence.uuid()
                                } else {
                                    "".to_string()
                                };
                                state.play_riff_sequence(tx_to_audio, riff_sequence_uuid);
                            } else if riffs_stack_visible_name == "riff_arrangement" {
                                let riff_arrangement_uuid = if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangements_mut().get_mut(0) {
                                    riff_arrangement.uuid()
                                } else {
                                    "".to_string()
                                };
                                state.play_riff_arrangement(tx_to_audio, riff_arrangement_uuid, 0.0);
                            }
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - transport play - could not get lock on state"),
                };
            }
            DAWEvents::TransportRecordOn => {
                match state.lock() {
                    Ok(mut state) => {
                        state.set_recording(true);
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - transport record on - could not get lock on state"),
                };
            }
            DAWEvents::TransportRecordOff => {
                match state.lock() {
                    Ok(mut state) => {
                        state.set_recording(false);
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - transport record off - could not get lock on state"),
                };
            }
            DAWEvents::TransportPause => {
                match state.lock() {
                    Ok(state) => {
                        let song = state.project().song();
                        let tracks = song.tracks();
                        for track in tracks {
                            state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Pause);
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - transport pause - could not get lock on state"),
                };
            }
            DAWEvents::TransportMoveForward => {
                debug!("Main - rx_ui processing loop - transport move forward - received");
                match state.lock() {
                    Ok(mut state) => {
                        let bpm = state.get_project().song().tempo();
                        let sample_rate = state.get_project().song().sample_rate();
                        let block_size = state.get_project().song().block_size();
                        let time_signature_numerator = state.project().song().time_signature_numerator();
                        let beats_per_bar = time_signature_numerator;
                        let mut play_position_in_frames = state.play_position_in_frames();
                        let frames_per_beat = sample_rate * 60.0 /bpm;
                        let frames_in_measure = (frames_per_beat * beats_per_bar) as u32;

                        play_position_in_frames += frames_in_measure;
                        state.set_play_position_in_frames(play_position_in_frames);

                        {
                            let state = state;
                            for track in state.project().song().tracks().iter() {
                                state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetBlockPosition((play_position_in_frames / (block_size as u32)) as i32));
                            }
                        }
                        let play_position_in_beats = play_position_in_frames as f64 / sample_rate * bpm / 60.0;
                        let current_bar = play_position_in_beats as i32 / time_signature_numerator as i32 + 1;
                        let current_beat_in_bar = play_position_in_beats as i32 % time_signature_numerator as i32 + 1;

                        gui.ui.song_position_txt_ctrl.set_label(format!("{:03}:{:03}:000", current_bar, current_beat_in_bar).as_str());
                        if let Some(piano_roll_grid) = gui.piano_roll_grid() {
                            match piano_roll_grid.lock() {
                                Ok(mut grid) => grid.set_track_cursor_time_in_beats(play_position_in_beats),
                                Err(_) => (),
                            }
                        }
                        if let Some(track_grid) = gui.track_grid() {
                            match track_grid.lock() {
                                Ok(mut grid) => grid.set_track_cursor_time_in_beats(play_position_in_beats),
                                Err(_) => (),
                            }
                        }
                        if let Some(sample_roll_grid) = gui.sample_roll_grid() {
                            match sample_roll_grid.lock() {
                                Ok(mut grid) => grid.set_track_cursor_time_in_beats(play_position_in_beats),
                                Err(_) => (),
                            }
                        }
                        if let Some(automation_grid) = gui.automation_grid() {
                            match automation_grid.lock() {
                                Ok(mut grid) => grid.set_track_cursor_time_in_beats(play_position_in_beats),
                                Err(_) => (),
                            }
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - play position in beats - could not get lock on state"),
                }

                gui.ui.piano_roll_drawing_area.queue_draw();
                gui.ui.sample_roll_drawing_area.queue_draw();
                gui.ui.track_drawing_area.queue_draw();
                gui.ui.automation_drawing_area.queue_draw();
            }
            DAWEvents::TransportGotoEnd => {
                match state.lock() {
                    Ok(state) => {
                        let song = state.project().song();
                        let tracks = song.tracks();
                        for track in tracks {
                            state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::GotoEnd);
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - transport goto end - could not get lock on state"),
                };
            }
            DAWEvents::PlayNoteImmediate(note) => {
                match state.lock() {
                    Ok(state) => {
                        let track_uuid = state.selected_track();
                        match track_uuid {
                            Some(track_uuid) => {
                                match state.project().song().tracks().iter().find(|track| track.uuid().to_string() == track_uuid) {
                                    Some(track) => {
                                        let midi_channel = if let TrackType::MidiTrack(midi_track) = track {
                                            midi_track.midi_device().midi_channel()
                                        } else {
                                            0
                                        };
                                        state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::PlayNoteImmediate(note, midi_channel));
                                    },
                                    None => debug!("Play note immediate: Could not find track number."),
                                }
                            },
                            None => debug!("Play note immediate: no track number given."),
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - play note immediate - could not get lock on state"),
                };
            },
            DAWEvents::StopNoteImmediate(note) => {
                match state.lock() {
                    Ok(state) => {
                        let track_uuid = state.selected_track();
                        match track_uuid {
                            Some(track_uuid) => {
                                match state.project().song().tracks().iter().find(|track| track.uuid().to_string() == track_uuid) {
                                    Some(track) => {
                                        let midi_channel = if let TrackType::MidiTrack(midi_track) = track {
                                            midi_track.midi_device().midi_channel()
                                        } else {
                                            0
                                        };
                                        state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::StopNoteImmediate(note, midi_channel));
                                    },
                                    None => debug!("Stop note immediate: Could not find track number."),
                                }
                            },
                            None => debug!("Stop note immediate: no track number given."),
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - stop note immediate - could not get lock on state"),
                };
            },
            DAWEvents::TempoChange(tempo) => {
                match state.lock() {
                    Ok(mut state) => {
                        state.get_project().song_mut().set_tempo(tempo);
                        if let Some(track_grid) = gui.track_grid() {
                            if let Ok(track) = track_grid.lock() {
                                let mut grid = track;
                                grid.set_tempo(state.project().song().tempo());
                            }
                        }
                        if let Some(piano_roll_grid) = gui.piano_roll_grid() {
                            if let Ok(piano_roll) = piano_roll_grid.lock() {
                                let mut grid = piano_roll;
                                grid.set_tempo(state.project().song().tempo());
                            }
                        }
                        if let Some(controller_grid) = gui.automation_grid() {
                            if let Ok(controllers) = controller_grid.lock() {
                                let mut grid = controllers;
                                grid.set_tempo(state.project().song().tempo());
                            }
                        }
                        match tx_to_audio.send(AudioLayerInwardEvent::Tempo(state.project().song().tempo())) {
                            Ok(_) => (),
                            Err(error) => debug!("Problem using tx_to_audio to send tempo message to jack layer: {}", error),
                        }
                        for track in state.project().song().tracks().iter() {
                            state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Tempo(tempo));
                        }
                },
                    Err(_) => debug!("Main - rx_ui processing loop - tempo change - could not get lock on state"),
                };
            },
            DAWEvents::Panic => {
                debug!("Sending note off messages to everything...");
                match state.lock() {
                    Ok(state) => for track in state.project().song().tracks().iter() {
                        let midi_channel = if let TrackType::MidiTrack(midi_track) = track {
                            midi_track.midi_device().midi_channel()
                        } else {
                            0
                        };
                        for note_number in 0..128 {
                            state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::StopNoteImmediate(note_number, midi_channel));
                        }
                    },
                    Err(_) => (),
                }
            },
            DAWEvents::MasterChannelChange(channel_change_type) => {
                match channel_change_type {
                    MasterChannelChangeType::VolumeChange(volume) => {
                        debug!("Master channel volume change: {}", volume);
                        match tx_to_audio.send(AudioLayerInwardEvent::Volume(volume as f32)) {
                            Ok(_) => (),
                            Err(error) => debug!("Problem using tx_to_audio to send master volume message to jack layer: {}", error),
                        }
                    },
                    MasterChannelChangeType::PanChange(pan) => {
                        debug!("Master channel pan change: {}", pan);
                        match tx_to_audio.send(AudioLayerInwardEvent::Pan(pan as f32)) {
                            Ok(_) => (),
                            Err(error) => debug!("Problem using tx_to_audio to send master pan message to jack layer: {}", error),
                        }
                    },
                }
            },
            DAWEvents::PlayPositionInBeats(play_position_in_beats) => {
                debug!("Received DAWEvents::PlayPositionInBeats");
                match state.lock() {
                    Ok(mut state) => {
                        let bpm = state.get_project().song().tempo();
                        let sample_rate = state.get_project().song().sample_rate();
                        let block_size = state.get_project().song().block_size();
                        let play_position_in_frames = 60.0 * play_position_in_beats / bpm * sample_rate;

                        state.set_play_position_in_frames(play_position_in_frames as u32);

                        {
                            let state = state;
                            for track in state.project().song().tracks().iter() {
                                state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetBlockPosition((play_position_in_frames / block_size) as i32));
                            }
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - play position in beats - could not get lock on state"),
                };
            },
            DAWEvents::TrimAllNoteDurations => {
                match state.lock() {
                    Ok(mut state) => {
                        {
                            for track_type in state.get_project().song_mut().tracks_mut().iter_mut() {
                                match track_type {
                                    TrackType::InstrumentTrack(track) => {
                                        for riff in track.riffs_mut().iter_mut() {
                                            for event in riff.events_mut().iter_mut() {
                                                if let TrackEvent::Note(note_on) = event {
                                                    note_on.set_length(note_on.length() - 0.01);
                                                }
                                            }
                                        }
                                    },
                                    TrackType::AudioTrack(_) => (),
                                    TrackType::MidiTrack(_) => (),
                                }
                            }
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - trim all note durations - could not get lock on state"),
                };
            }
            DAWEvents::RiffSetAdd(uuid, name) => {
                match state.lock() {
                    Ok(mut state) => {
                        let selected_riff_set_uuid = if let Some(selected_riff_set_uuid) = state.riff_set_selected_uuid() {
                            Some(selected_riff_set_uuid.to_string())
                        }
                        else { None};
                        let song = state.get_project().song_mut();
                        let mut riff_set = RiffSet::new_with_uuid(uuid);
                        riff_set.set_name(name);
                        for track in song.tracks().iter() {
                            let empty_riff_uuid = if let Some(riff) = track.riffs().iter().find(|riff| riff.name() == "empty") {
                                riff.uuid().to_string()
                            }
                            else {
                                "".to_string()
                            };
                            riff_set.set_riff_ref_for_track(track.uuid().to_string(), RiffReference::new(empty_riff_uuid, 0.0));
                        }
                        let selected_riff_set_position = if let Some(selected_riff_set_uuid) = selected_riff_set_uuid {
                            if let Some(selected_riff_set_position) = song.riff_sets().iter().position(|riff_set| riff_set.uuid() == *selected_riff_set_uuid) {
                                Some(selected_riff_set_position)
                            }
                            else { None }
                        }
                        else { None };

                        if let Some(selected_riff_set_position) = selected_riff_set_position {
                            song.add_riff_set_at_position(riff_set, selected_riff_set_position + 1);
                        }
                        else {
                            song.add_riff_set(riff_set);
                        }
                        gui.update_available_riff_sets(&state);
                    },
                    Err(_) => (),
                }
                gui.ui.riff_sets_box.queue_draw();
            },
            DAWEvents::RiffSetDelete(uuid) => {
                // check if any riff sequences or arrangements are using this riff - if so then show a warning dialog
                let found_info = match state.lock() {
                    Ok(state) => {
                        let mut found_info = vec![];

                        // check riff sequences
                        for riff_sequence in state.project().song().riff_sequences().iter() {
                            for riff_set_item in riff_sequence.riff_sets().iter() {
                                if let Some(riff_set) = state.project().song().riff_set(riff_set_item.item_uuid().to_string()) {
                                    if riff_set.uuid() == uuid {
                                        let message = format!("Riff sequence: \"{}\" has references to riff set: \"{}\".", riff_sequence.name(), riff_set.name());

                                        if !found_info.iter().any(|entry| *entry == message) {
                                            found_info.push(message);
                                        }
                                    }
                                }
                            }
                        }

                        // check riff arrangements
                        for riff_arrangement in state.project().song().riff_arrangements().iter() {
                            for riff_item in riff_arrangement.items().iter() {
                                if *(riff_item.item_type()) == RiffItemType::RiffSet {
                                    if let Some(riff_set) = state.project().song().riff_set(riff_item.item_uuid().to_string()) {
                                        if riff_set.uuid() == uuid {
                                            let message = format!("Riff arrangement: \"{}\" has references to riff set: \"{}\".", riff_arrangement.name(), riff_set.name());

                                            if !found_info.iter().any(|entry| *entry == message) {
                                                found_info.push(message);
                                            }
                                        }
                                    }
                                }
                                else {
                                    if let Some(riff_sequence) = state.project().song().riff_sequence(riff_item.uuid()) {
                                        for riff_set_item in riff_sequence.riff_sets().iter() {
                                            if let Some(riff_set) = state.project().song().riff_set(riff_set_item.item_uuid().to_string()) {
                                                if riff_set.uuid() == uuid {
                                                    let message = format!("Riff arrangement: \"{}\" (via riff sequence) has references to riff set: \"{}\".", riff_arrangement.name(), riff_set.name());

                                                    if !found_info.iter().any(|entry| *entry == message) {
                                                        found_info.push(message);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        found_info
                    }
                    Err(_) => {
                        debug!("Main - rx_ui processing loop - riff set delete - could not get lock on state");
                        vec![]
                    }
                };

                // if the riff arrangement is not using this riff set then delete it from the project/song
                if found_info.len() == 0 {
                    match state.lock() {
                        Ok(mut state) => {
                            let song = state.get_project().song_mut();
                            // remove the riff set from the song
                            song.remove_riff_set(uuid.clone());
                            // remove the riff set from the add riff set picklists (riff sequence and riff arrangement views)
                            gui.update_available_riff_sets(&state);
                            // remove the riff set head and blade from the UI (riff set view)
                            gui.delete_riff_set_blade(uuid);
                        },
                        Err(_) => (),
                    }
                    gui.ui.riff_sets_box.queue_draw();
                } else {
                    let mut error_message = String::from("Could not delete riff set:\n");

                    for message in found_info.iter() {
                        error_message.push_str(message.as_str());
                        error_message.push_str("\n");
                    }

                    let _ = tx_from_ui.send(DAWEvents::Notification(NotificationType::Error, error_message));
                }
            },
            DAWEvents::RiffSetCopy(uuid, new_copy_riff_set_uuid) => {
                match state.lock() {
                    Ok(mut state) => {
                        if let Some(copy_of_riff_set) = state.get_project().song_mut().riff_set_copy(uuid, new_copy_riff_set_uuid.clone()) {                            
                            gui.update_riff_set_name_in_riff_views(new_copy_riff_set_uuid.to_string(), copy_of_riff_set.name().to_string());
                        }
                        gui.update_available_riff_sets(&state);
                    },
                    Err(_) => (),
                }
                gui.ui.riff_sets_box.queue_draw();
            },
            DAWEvents::RiffSetNameChange(uuid, name) => {
                match state.lock() {
                    Ok(mut state) => {
                        let song = state.get_project().song_mut();
                        match song.riff_set_mut(uuid.clone()) {
                            Some(riff_set) => riff_set.set_name(name.clone()),
                            None => debug!("Could not find the riff set to change the name of."),
                        }
                        gui.update_available_riff_sets(&state);
                        gui.update_riff_set_name_in_riff_views(uuid, name);
                    },
                    Err(error) => debug!("Could not lock the state when trying to change a riff set name: {}", error),
                }
            },
            DAWEvents::RiffSetPlay(uuid) => {
                debug!("Main - rx_ui processing loop - riff set play: {}", uuid);
                match state.lock() {
                    Ok(mut state) => {
                        state.play_riff_set(tx_to_audio, uuid);
                        if let Some(playing_riff_set_uuid) = state.playing_riff_set() {
                            gui.repaint_riff_set_view_riff_set_active_drawing_areas(playing_riff_set_uuid, 0.0);
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - riff set play - could not get lock on state"),
                };
            }
            DAWEvents::RiffSetTrackIncrementRiff(riff_set_uuid, track_uuid) => {
                debug!("Main - rx_ui processing loop - riff set track incr riff: {}, {}", riff_set_uuid.as_str(), track_uuid.as_str());
                match state.lock() {
                    Ok(mut state) => {
                        state.riff_set_increment_riff_for_track(riff_set_uuid.clone(), track_uuid.clone());
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - riff set track incr riff - could not get lock on state"),
                };
                let drawing_area_widget_name = format!("{}_{}", riff_set_uuid.as_str(), track_uuid.as_str());
                if let Some(riff_set_blade) = gui.ui.riff_sets_box.children().iter().find(|child| child.widget_name().to_string().contains(riff_set_uuid.as_str())) {
                    if let Some(riff_set_box) = riff_set_blade.dynamic_cast_ref::<gtk::Box>() {
                        for child in riff_set_box.children().iter() {
                            if child.widget_name().contains(drawing_area_widget_name.as_str()) {
                                child.queue_draw();
                            }
                        }
                    }
                }
            }
            DAWEvents::RiffSetTrackSetRiff(riff_set_uuid, track_uuid, riff_uuid) => {
                debug!("Main - rx_ui processing loop - riff set track set riff: riff set={}, track={}, riff={}", riff_set_uuid.as_str(), track_uuid.as_str(), riff_uuid.as_str());
                match state.lock() {
                    Ok(mut state) => {
                        state.riff_set_riff_for_track(riff_set_uuid, track_uuid, riff_uuid);
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - riff set track set riff - could not get lock on state"),
                };
                gui.ui.riff_sets_box.queue_draw();
            }
            DAWEvents::RiffSequencePlay(riff_sequence_uuid) => {
                match state.lock() {
                    Ok(mut state) => {
                        state.play_riff_sequence(tx_to_audio, riff_sequence_uuid.clone());
                        state.set_playing_riff_sequence(Some(riff_sequence_uuid.clone()));
                        if let Some(playing_riff_sequence_summary_data) = state.playing_riff_sequence_summary_data() {
                            gui.repaint_riff_sequence_view_riff_sequence_active_drawing_areas(&riff_sequence_uuid, 0.0, playing_riff_sequence_summary_data);
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - riff sequence play - could not get lock on state"),
                };
                gui.ui.riff_sequences_box.queue_draw();
            }
            DAWEvents::RiffSequenceAdd(riff_sequence_uuid) => {
                match state.lock() {
                    Ok(mut state) => {
                        state.get_project().song_mut().add_riff_sequence(RiffSequence::new_with_uuid(riff_sequence_uuid));
                        gui.update_available_riff_sequences_in_riff_arrangement_blades(&state);
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - riff sequence add - could not get lock on state"),
                };
                gui.ui.riff_sequences_box.queue_draw();
            }
            DAWEvents::RiffSequenceDelete(riff_sequence_uuid) => {
                // check if any riff sequences or arrangements are using this riff - if so then show a warning dialog
                let found_info = match state.lock() {
                    Ok(state) => {
                        let mut found_info = vec![];

                        // check riff arrangements
                        for riff_arrangement in state.project().song().riff_arrangements().iter() {
                            for riff_item in riff_arrangement.items().iter() {
                                if let Some(riff_sequence) = state.project().song().riff_sequence(riff_item.item_uuid().to_string()) {
                                    if riff_sequence.uuid() == riff_sequence_uuid {
                                        let message = format!("Riff arrangement: \"{}\" has references to riff sequence: \"{}\".", riff_arrangement.name(), riff_sequence.name());

                                        if !found_info.iter().any(|entry| *entry == message) {
                                            found_info.push(message);
                                        }
                                    }
                                }
                            }
                        }

                        found_info
                    }
                    Err(_) => {
                        debug!("Main - rx_ui processing loop - riff sequence delete - could not get lock on state");
                        vec![]
                    }
                };

                // if the riff is not used then delete it from the project/song
                if found_info.len() == 0 {
                    match state.lock() {
                        Ok(mut state) => {
                            // remove the riff sequence from the song
                            state.get_project().song_mut().remove_riff_sequence(riff_sequence_uuid.clone());
                            // remove the riff sequence from arrangement riff sequence pick list
                            gui.update_available_riff_sequences_in_riff_arrangement_blades(&state);
                            // remove the riff sequence from the sequence combobox in the riff sequence view
                            gui.update_riff_sequences_combobox_in_riff_sequence_view(&state);
                            // remove the riff sequence blade from riff sequence view
                            gui.delete_riff_sequence_blade(riff_sequence_uuid);
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - riff sequence delete - could not get lock on state"),
                    };
                    gui.ui.riff_sequences_box.queue_draw();
                } else {
                    let mut error_message = String::from("Could not delete riff sequence:\n");

                    for message in found_info.iter() {
                        error_message.push_str(message.as_str());
                        error_message.push_str("\n");
                    }

                    let _ = tx_from_ui.send(DAWEvents::Notification(NotificationType::Error, error_message));
                }
            }
            DAWEvents::RiffSequenceNameChange(riff_sequence_uuid, name) => {
                match state.lock() {
                    Ok(mut state) => {
                        if let Some(riff_sequence) = state.get_project().song_mut().riff_sequence_mut(riff_sequence_uuid) {
                            riff_sequence.set_name(name);
                        }
                        gui.update_available_riff_sequences_in_riff_arrangement_blades(&state);
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - riff sequence name change - could not get lock on state"),
                };
                gui.ui.riff_sequences_box.queue_draw();
            }
            DAWEvents::RiffSequenceRiffSetAdd(riff_sequence_uuid, riff_set_uuid, riff_set_reference_uuid) => {
                debug!("Main - rx_ui processing loop - riff sequence - riff set add: {}, {}", riff_sequence_uuid.as_str(), riff_set_uuid.as_str());
                match state.lock() {
                    Ok(mut state) => {
                        let selected_riff_set_instance_details = if let Some(selected_riff_set_uuid) = state.riff_sequence_riff_set_reference_selected_uuid() {
                            Some(selected_riff_set_uuid.clone())
                        }
                        else { None};
                        let selected_riff_set_position = if let Some(riff_sequence) = state.project().song().riff_sequences().iter().find(|riff_sequence| riff_sequence.uuid() == riff_sequence_uuid) {
                            if let Some(selected_riff_set_instance_details) = selected_riff_set_instance_details {
                                if let Some(selected_riff_set_position) = riff_sequence.riff_sets().iter().position(|riff_set| riff_set.uuid() == selected_riff_set_instance_details.1) {
                                    Some(selected_riff_set_position)
                                }
                                else { None }
                            }
                            else { None }
                        }
                        else { None };

                        if let Some(selected_riff_set_position) = selected_riff_set_position {
                            if let Some(riff_sequence) = state.get_project().song_mut().riff_sequence_mut(riff_sequence_uuid) {
                                riff_sequence.add_riff_set_at_position(riff_set_reference_uuid, riff_set_uuid, selected_riff_set_position + 1);
                            }
                        }
                        else {
                            if let Some(riff_sequence) = state.get_project().song_mut().riff_sequence_mut(riff_sequence_uuid) {
                                riff_sequence.add_riff_set(riff_set_reference_uuid, riff_set_uuid);
                            }
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - riff sequence - riff set add - could not get lock on state"),
                }
                gui.ui.riff_sequences_box.queue_draw();
            }
            DAWEvents::RiffSequenceRiffSetDelete(riff_sequence_uuid, riff_set_reference_uuid) => {
                debug!("Main - rx_ui processing loop - riff sequence - riff sequence delete: {}, {}", riff_sequence_uuid.as_str(), riff_set_reference_uuid.as_str());
                let state_arc = state.clone();
                match state.lock() {
                    Ok(mut state) => {
                        if let Some(riff_sequence) = state.get_project().song_mut().riff_sequence_mut(riff_sequence_uuid.clone()) {
                            // remove the riff item referencing a riff set from the riff sequence
                            riff_sequence.remove_riff_set(riff_set_reference_uuid);
                            let mut track_uuids = MainWindow::collect_track_uuids(&mut state);
                            // update any references to the riff sequence in the riff sequence view
                            gui.update_riff_sequences(&tx_from_ui, &mut state, &state_arc, &mut track_uuids, true);
                            // update any references to the riff sequence in the riff arrangement view
                            gui.update_riff_arrangements(tx_from_ui, &mut state, state_arc, track_uuids, true);
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - riff sequence - riff set delete - could not get lock on state"),
                };
                gui.ui.riff_sequences_box.queue_draw();
            }
            DAWEvents::RiffSequenceRiffSetMoveLeft(riff_sequence_uuid, riff_set_reference_uuid) => {
                debug!("Main - rx_ui processing loop - riff sequence - riff set reference move left: {}, {}", riff_sequence_uuid.as_str(), riff_set_reference_uuid.as_str());
                match state.lock() {
                    Ok(mut state) => {
                        if let Some(riff_sequence) = state.get_project().song_mut().riff_sequence_mut(riff_sequence_uuid) {
                            riff_sequence.riff_set_move_left(riff_set_reference_uuid);
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - riff sequence - riff set reference move left - could not get lock on state"),
                };
                gui.ui.riff_sequences_box.queue_draw();
            }
            DAWEvents::RiffSequenceRiffSetMoveRight(riff_sequence_uuid, riff_set_uuid) => {
                debug!("Main - rx_ui processing loop - riff sequence - riff set reference move right: {}, {}", riff_sequence_uuid.as_str(), riff_set_uuid.as_str());
                match state.lock() {
                    Ok(mut state) => {
                        if let Some(riff_sequence) = state.get_project().song_mut().riff_sequence_mut(riff_sequence_uuid) {
                            riff_sequence.riff_set_move_right(riff_set_uuid);
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - riff sequence - riff set reference move right - could not get lock on state"),
                };
                gui.ui.riff_sequences_box.queue_draw();
            }
            DAWEvents::RiffArrangementPlay(riff_arrangement_uuid) => {
                match state.lock() {
                    Ok(mut state) => {
                        let selected_riff_arrangement_play_position = if let Some(riff_arrangement) = state.project().song().riff_arrangement(riff_arrangement_uuid.clone()) {
                            let selected_riff_item_index = gui.get_selected_riff_arrangement_play_position();
                            let mut play_position_in_beats = 0.0;
                            for (index, riff_item) in riff_arrangement.items().iter().enumerate() {
                                if index >= selected_riff_item_index {
                                    break;
                                }
                                // FIXME riff set lengths need to be determined using the lowest common factor not straight up find the largest (the largest may not be the actual lowest common factor)
                                if let RiffItemType::RiffSet = riff_item.item_type() {
                                    // grab the item
                                    if let Some(riff_set) = state.project().song().riff_set(riff_item.item_uuid().to_string()) {
                                        let mut riff_lengths = vec![];
                                        for (track_uuid, riff_set_uuid) in riff_set.riff_refs().iter().map(|(track_uuid, value)| (track_uuid.to_string(), value.linked_to().to_string())).collect::<Vec<(String, String)>>().iter() {
                                            if let Some(track) = state.project().song().track(track_uuid.to_string()) {
                                                if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == *riff_set_uuid) {
                                                    riff_lengths.push(riff.length() as i32);
                                                }
                                            }
                                        }
                                        let (product, unique_riff_lengths) = DAWState::get_length_product(riff_lengths);
                                        play_position_in_beats += DAWState::get_lowest_common_factor(unique_riff_lengths, product) as f64;
                                    }
                                }
                                else if let Some(riff_sequence) = state.project().song().riff_sequence(riff_item.item_uuid().to_string()) {
                                    for riff_item in riff_sequence.riff_sets().iter() {
                                        if let Some(riff_set) = state.project().song().riff_set(riff_item.item_uuid().to_string()) {
                                            let mut riff_lengths = vec![];
                                            for (track_uuid, riff_set_uuid) in riff_set.riff_refs().iter().map(|(track_uuid, value)| (track_uuid.to_string(), value.linked_to().to_string())).collect::<Vec<(String, String)>>().iter() {
                                                if let Some(track) = state.project().song().track(track_uuid.to_string()) {
                                                    if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == *riff_set_uuid) {
                                                        riff_lengths.push(riff.length() as i32);
                                                    }
                                                }
                                            }
                                            let (product, unique_riff_lengths) = DAWState::get_length_product(riff_lengths);
                                            play_position_in_beats += DAWState::get_lowest_common_factor(unique_riff_lengths, product) as f64;
                                        }
                                    }
                                }
                            }
                            play_position_in_beats
                        }
                        else {
                            0.0
                        };
                        state.play_riff_arrangement(tx_to_audio, riff_arrangement_uuid.clone(), selected_riff_arrangement_play_position);
                        state.set_playing_riff_arrangement(Some(riff_arrangement_uuid.clone()));
                        if let Some(playing_riff_arrangement_summary_data) = state.playing_riff_arrangement_summary_data() {
                            gui.repaint_riff_arrangement_view_riff_arrangement_active_drawing_areas(&riff_arrangement_uuid, 0.0, playing_riff_arrangement_summary_data);
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - riff arrangement play - could not get lock on state"),
                };
                gui.ui.riff_arrangement_box.queue_draw();
            }
            DAWEvents::RiffArrangementAdd(riff_arrangement_uuid) => {
                match state.lock() {
                    Ok(mut state) => {
                        let mut arrangement = RiffArrangement::new_with_uuid(riff_arrangement_uuid);

                        // add an automation object for each track
                        for track in state.project().song().tracks().iter() {
                            arrangement.add_track_automation(track.uuid().to_string());
                        }

                        state.get_project().song_mut().add_riff_arrangement(arrangement);
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - riff arrangement add - could not get lock on state"),
                };
                gui.ui.riff_arrangement_box.queue_draw();
            }
            DAWEvents::RiffArrangementDelete(riff_arrangement_uuid) => {
                match state.lock() {
                    Ok(mut state) => {
                        // remove the riff arrangement from the song
                        state.get_project().song_mut().remove_riff_arrangement(riff_arrangement_uuid);
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - riff arrangement delete - could not get lock on state"),
                };
                gui.ui.riff_arrangement_box.queue_draw();
            }
            DAWEvents::RiffArrangementNameChange(riff_arrangement_uuid, name) => {
                match state.lock() {
                    Ok(mut state) => {
                        if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangement_mut(riff_arrangement_uuid) {
                            riff_arrangement.set_name(name);
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - riff arrangement name change - could not get lock on state"),
                };
                gui.ui.riff_arrangement_box.queue_draw();
            }
            DAWEvents::RiffArrangementMoveRiffItemToPosition(riff_arrangement_uuid, riff_item_compound_uuid, position) => {
                debug!("Main - rx_ui processing loop - riff arrangement={} move riff set={} to position={}", riff_arrangement_uuid.as_str(), riff_item_compound_uuid.as_str(), position);
                match state.lock() {
                    Ok(mut state) => {
                        state.get_project().song_mut().riff_arrangement_move_riff_item_to_position(riff_arrangement_uuid, riff_item_compound_uuid, position);
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - riff arrangement move riff set to position - could not get lock on state"),
                };
                gui.ui.riff_arrangement_box.queue_draw();
            }
            DAWEvents::RiffArrangementRiffItemAdd(riff_arrangement_uuid, item_uuid, riff_item_type) => {
                debug!("Main - rx_ui processing loop - riff arrangement={} - riff item add: {}, {}, {}", riff_arrangement_uuid.as_str(), riff_arrangement_uuid.as_str(), item_uuid.as_str(), if let RiffItemType::RiffSet = riff_item_type.clone() { "RiffSet" } else {"RiffSequence"});
                let state_arc = state.clone();
                match state.lock() {
                    Ok(mut state) => {
                        let item_reference_uuid = Uuid::new_v4();
                        let selected_riff_item_details = if let Some(selected_riff_item_uuid) = state.riff_arrangement_riff_item_selected_uuid() {
                            Some(selected_riff_item_uuid.clone())
                        }
                        else { None};
                        let selected_riff_item_position = if let Some(riff_arrangement) = state.project().song().riff_arrangements().iter().find(|riff_arrangement| riff_arrangement.uuid() == riff_arrangement_uuid) {
                            if let Some(selected_riff_item_details) = selected_riff_item_details {
                                if let Some(selected_riff_item_position) = riff_arrangement.items().iter().position(|riff_item| riff_item.uuid() == selected_riff_item_details.1) {
                                    Some(selected_riff_item_position)
                                }
                                else { None }
                            }
                            else { None }
                        }
                        else { None };

                        if let Some(selected_riff_item_position) = selected_riff_item_position {
                            if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangement_mut(riff_arrangement_uuid.clone()) {
                                riff_arrangement.add_item_at_position(RiffItem::new_with_uuid_string(item_reference_uuid.to_string(), riff_item_type.clone(), item_uuid.clone()), selected_riff_item_position + 1);
                            }
                        }
                        else {
                            if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangement_mut(riff_arrangement_uuid.clone()) {
                                riff_arrangement.add_item(RiffItem::new_with_uuid_string(item_reference_uuid.to_string(), riff_item_type.clone(), item_uuid.clone()));
                            }
                        }

                        let track_uuids: Vec<String> = state.project().song().tracks().iter().map(|track| track.uuid().to_string()).collect();
                        if let RiffItemType::RiffSet = riff_item_type {
                            let riff_set_name = if let Some(riff_set) = state.project().song().riff_sets().iter().find(|riff_set| riff_set.uuid() == item_uuid.clone()) {
                                riff_set.name().to_string()
                            }
                            else {
                                "".to_string()
                            };
                            gui.add_riff_arrangement_riff_set_blade(
                                tx_from_ui,
                                riff_arrangement_uuid,
                                item_reference_uuid.to_string(),
                                item_uuid,
                                track_uuids,
                                gui.selected_style_provider.clone(),
                                gui.ui.riff_arrangement_vertical_adjustment.clone(),
                                riff_set_name,
                                state_arc,
                            );
                        }
                        else {
                            gui.add_riff_arrangement_riff_sequence_blade(
                                tx_from_ui,
                                riff_arrangement_uuid,
                                item_reference_uuid.to_string(),
                                item_uuid,
                                track_uuids,
                                gui.selected_style_provider.clone(),
                                gui.ui.riff_arrangement_vertical_adjustment.clone(),
                                "".to_string(),
                                state_arc,
                                &state,
                            );
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - riff arrangement - riff item add - could not get lock on state"),
                }
                gui.ui.riff_arrangement_box.queue_draw();
            }
            DAWEvents::RiffArrangementRiffItemDelete(riff_arrangement_uuid, item_uuid) => {
                debug!("Main - rx_ui processing loop - riff arrangement={} - riff item delete: {}", riff_arrangement_uuid.as_str(), item_uuid.as_str());
                match state.lock() {
                    Ok(mut state) => {
                        if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangement_mut(riff_arrangement_uuid) {
                            riff_arrangement.remove_item(item_uuid);
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - riff arrangement - riff item delete - could not get lock on state"),
                };
                gui.ui.riff_arrangement_box.queue_draw();
            }
            DAWEvents::Shutdown => {
                if let Ok(mut coast) = track_audio_coast.lock() {
                    *coast = TrackBackgroundProcessorMode::Coast;
                }
                match state.lock() {
                    Ok(state) => {
                        let state = state;
                        for track in state.project().song().tracks() {
                            state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Kill);
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - Open File - could not get lock on state"),
                }
                match tx_to_audio.send(AudioLayerInwardEvent::Shutdown) {
                    Ok(_) => {}
                    Err(_) => {}
                }
            }
            DAWEvents::Undo => {
                match history_manager.lock() {
                    Ok(mut history_manager) => {
                        if let Err(error) = history_manager.undo(&mut state.clone()) {
                            debug!("{}", error);
                        }
                    }
                    Err(_) => {
                        debug!("Couldn't lock the history manager!");
                    }
                }
                gui.ui.piano_roll_drawing_area.queue_draw();
                gui.ui.sample_roll_drawing_area.queue_draw();
                gui.ui.automation_drawing_area.queue_draw();
                gui.ui.track_drawing_area.queue_draw();
                gui.ui.riff_arrangement_box.queue_draw();
                gui.ui.riff_sets_box.queue_draw();
                gui.ui.riff_sequences_box.queue_draw();
            }
            DAWEvents::Redo => {
                match history_manager.lock() {
                    Ok(mut history_manager) => {
                        if let Err(error) = history_manager.redo(&mut state.clone()) {
                            debug!("{}", error);
                        }
                    }
                    Err(_) => {
                        debug!("Couldn't lock the history manager.");
                    }
                }
                gui.ui.piano_roll_drawing_area.queue_draw();
                gui.ui.sample_roll_drawing_area.queue_draw();
                gui.ui.automation_drawing_area.queue_draw();
                gui.ui.track_drawing_area.queue_draw();
                gui.ui.riff_arrangement_box.queue_draw();
                gui.ui.riff_sets_box.queue_draw();
                gui.ui.riff_sequences_box.queue_draw();
            }
            DAWEvents::PreviewSample(file_name) => {
                match tx_to_audio.send(AudioLayerInwardEvent::PreviewSample(file_name)) {
                    Ok(_) => {}
                    Err(_) => {}
                }
            }
            DAWEvents::SampleAdd(file_name) => {
                match state.lock() {
                    Ok(mut state) => {
                        // create the sample object and store it
                        let sample_data = SampleData::new(
                            file_name.clone(),
                            *state.get_project().song_mut().sample_rate_mut() as i32,
                        );
                        let sample = Sample::new(
                            file_name.clone(),
                            file_name,
                            sample_data.uuid().to_string(),
                        );

                        state.get_project().song_mut().samples_mut().insert(sample.uuid().to_string(), sample.clone());
                        state.sample_data_mut().insert(sample_data.uuid().to_string(), sample_data);
                        debug!("Added sample: id={}, text={}, uuid={}", sample.file_name(), sample.name(), sample.uuid());

                        // update the sample roll browser list store
                        gui.update_sample_roll_sample_browser(sample.uuid().to_string(), sample.name().to_string());
                    }
                    Err(_) => {}
                }
            }
            DAWEvents::SampleDelete(_uuid) => {}
            DAWEvents::RunLuaScript(script) => {
                match lua.load(script.as_str()).eval::<MultiValue>() {
                    Ok(values) => {
                        if let Some(console_output_text_buffer) = gui.ui.scripting_console_output_text_view.buffer() {
                            let console_output_text = format!("{}\n>> ",
                                                              values
                                                                  .iter()
                                                                  .map(|value| {
                                                                      match value {
                                                                          Value::Nil => "Nil".to_string(),
                                                                          Value::Boolean(data) => format!("{}", data),
                                                                          Value::LightUserData(_data) => "LightUserData".to_string(),
                                                                          Value::Integer(data) => format!("{}", data),
                                                                          Value::Number(data) => format!("{}", data),
                                                                          Value::String(data) => data.to_str().unwrap().to_string(),
                                                                          Value::Table(_data) => "Table".to_string(),
                                                                          Value::Function(_data) => "Function".to_string(),
                                                                          Value::Thread(_data) => "Thread".to_string(),
                                                                          Value::UserData(_data) => "AnyUserData".to_string(),
                                                                          Value::Error(data) => format!("{:?}", data),
                                                                          _ => "".to_string(),
                                                                      }
                                                                  })
                                                                  .collect::<Vec<_>>()
                                                                  .join("\t")
                            );
                            console_output_text_buffer.insert(&mut console_output_text_buffer.end_iter(), console_output_text.as_str());
                        }
                    }
                    Err(error) => {
                        if let Some(console_output_text_buffer) = gui.ui.scripting_console_output_text_view.buffer() {
                            let console_output_text = format!("{}\n>> ", error);
                            console_output_text_buffer.insert(&mut console_output_text_buffer.end_iter(), console_output_text.as_str());
                        }
                    }
                }
            }
            DAWEvents::HideProgressDialogue => {
                gui.ui.progress_dialogue.hide();
            }
            DAWEvents::RiffSetCopySelectedToTrackViewCursorPosition(uuid) => {
                // get the current track cursor position and convert it to beats
                let edit_cursor_position_in_beats = match &gui.track_grid {
                    Some(track_grid) => match track_grid.lock() {
                        Ok(grid) => grid.edit_cursor_time_in_beats(),
                        Err(_) => 0.0
                    },
                    None => 0.0
                };

                DAWUtils::copy_riff_set_to_position(uuid, edit_cursor_position_in_beats, state.clone());
            }
            DAWEvents::RiffSequenceCopySelectedToTrackViewCursorPosition(uuid) => {
                // get the current track cursor position and convert it to beats
                let edit_cursor_position_in_beats = match &gui.track_grid {
                    Some(track_grid) => match track_grid.lock() {
                        Ok(grid) => grid.edit_cursor_time_in_beats(),
                        Err(_) => 0.0
                    },
                    None => 0.0
                };

                DAWUtils::copy_riff_sequence_to_position(uuid, edit_cursor_position_in_beats, state.clone());
            }
            DAWEvents::RiffSequenceRiffSetSelect(riff_sequence_uuid, riff_set_reference_uuid, selected) => {
                debug!("Main - rx_ui processing loop - riff sequence={} riff set reference selected uuid={}, selected={}", riff_sequence_uuid.as_str(), riff_set_reference_uuid.as_str(), selected);
                match state.lock() {
                    Ok(mut state) => {
                        let mut set_selection_to_none = false;
                        if selected {
                            state.set_riff_sequence_riff_set_reference_selected_uuid(Some((riff_sequence_uuid, riff_set_reference_uuid)));
                        }
                        else if let Some((riff_sequence_uuid_selected, riff_set_reference_uuid_selected)) = state.riff_sequence_riff_set_reference_selected_uuid() {
                            if riff_sequence_uuid == *riff_sequence_uuid_selected && riff_set_reference_uuid == *riff_set_reference_uuid_selected {
                                set_selection_to_none = true;
                            }
                        }
                        if set_selection_to_none {
                            state.set_riff_sequence_riff_set_reference_selected_uuid(None);
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - riff sequence riff set reference selected uuid - could not get lock on state"),
                }
            }
            DAWEvents::RiffArrangementCopySelectedToTrackViewCursorPosition(uuid) => {
                // get the current track cursor position and convert it to beats
                let edit_cursor_position_in_beats = match &gui.track_grid {
                    Some(track_grid) => match track_grid.lock() {
                        Ok(grid) => grid.edit_cursor_time_in_beats(),
                        Err(_) => 0.0
                    },
                    None => 0.0
                };

                DAWUtils::copy_riff_arrangement_to_position(uuid, edit_cursor_position_in_beats, state.clone());
            }
            DAWEvents::RiffArrangementRiffItemSelect(riff_arrangement_uuid, riff_item_uuid, selected) => {
                debug!("Main - rx_ui processing loop - riff arrangement={} riff item reference selected uuid={}, selected={}", riff_arrangement_uuid.as_str(), riff_item_uuid.as_str(), selected);
                match state.lock() {
                    Ok(mut state) => {
                        let mut set_selection_to_none = false;
                        if selected {
                            state.set_riff_arrangement_riff_item_selected_uuid(Some((riff_arrangement_uuid, riff_item_uuid)));
                        }
                        else if let Some((riff_arrangement_uuid_selected, riff_item_uuid_selected)) = state.riff_arrangement_riff_item_selected_uuid() {
                            if riff_arrangement_uuid == *riff_arrangement_uuid_selected && riff_item_uuid == *riff_item_uuid_selected {
                                set_selection_to_none = true;
                            }
                        }
                        if set_selection_to_none {
                            state.set_riff_arrangement_riff_item_selected_uuid(None);
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - riff arrangement riff item reference selected uuid - could not get lock on state"),
                }
            }
            DAWEvents::RiffArrangementCopy(uuid) => {
                if let Ok(mut state) = state.lock() {
                    if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangement(uuid) {
                        let mut new_riff_arrangement = riff_arrangement.clone();
                        let mut new_name = "Copy of ".to_string();

                        new_name.push_str(new_riff_arrangement.name());
                        new_riff_arrangement.set_name(new_name);
                        new_riff_arrangement.set_uuid(Uuid::new_v4());

                        state.get_project().song_mut().add_riff_arrangement(new_riff_arrangement);
                    }
                }
                let _ = tx_from_ui.send(DAWEvents::UpdateUI);
            }
            DAWEvents::AutomationEditTypeChange(automation_edit_type) => {
                if let Ok(mut state) = state.lock() {
                    state.set_automation_edit_type(automation_edit_type);
                    gui.ui.automation_drawing_area.queue_draw();
                }
            }
            DAWEvents::RiffSetMoveToPosition(riff_set_uuid, to_position_in_container) => {
                debug!("Main - rx_ui processing loop - riff set move to position: {}", riff_set_uuid.as_str());
                match state.lock() {
                    Ok(mut state) => {
                        state.get_project().song_mut().riff_set_move_to_position(riff_set_uuid, to_position_in_container);
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - riff set move to position - could not get lock on state"),
                };
                gui.ui.riff_sets_box.queue_draw();
            }
            DAWEvents::RiffSetSelect(riff_set_uuid, selected) => {
                debug!("Main - rx_ui processing loop - riff set selected uuid={}, selected={}", riff_set_uuid.as_str(), selected);
                match state.lock() {
                    Ok(mut state) => {
                        let mut set_selection_to_none = false;
                        if selected {
                            state.set_riff_set_selected_uuid(Some(riff_set_uuid));
                        }
                        else if let Some(riff_set_selected_uuid) = state.riff_set_selected_uuid() {
                            if riff_set_uuid == *riff_set_selected_uuid {
                                set_selection_to_none = true;
                            }
                        }
                        if set_selection_to_none {
                            state.set_riff_set_selected_uuid(None);
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - riff set selected uuid - could not get lock on state"),
                }
            }
            DAWEvents::RiffSequenceRiffSetMoveToPosition(riff_sequence_uuid, riff_set_uuid, to_position_in_container) => {
                debug!("Main - rx_ui processing loop - riff sequence riff set move to position: {}", riff_set_uuid.as_str());
                match state.lock() {
                    Ok(mut state) => {
                        state.get_project().song_mut().riff_sequence_riff_set_move_to_position(riff_sequence_uuid, riff_set_uuid, to_position_in_container);
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - riff sequence riff set move to position - could not get lock on state"),
                };
                gui.ui.riff_sequences_box.queue_draw();
            }
            DAWEvents::TrackGridVerticalScaleChanged(vertical_scale) => {
                
                let widget_height = (TRACK_VIEW_TRACK_PANEL_HEIGHT as f64 * vertical_scale) as i32;
                for track_panel in gui.ui.top_level_vbox.children().iter_mut() {
                    debug!("$$$$$$$$$$$$$$$$$$$$$$$$$$$$ Track panel height: {}", track_panel.allocation().height);
                    track_panel.set_height_request(widget_height);
                }
                // gui.ui.track_panel_scrolled_window.queue_draw();
                gui.ui.top_level_vbox.queue_draw();
                gui.ui.track_drawing_area.queue_draw();
            }
            DAWEvents::RepaintAutomationView => {
                gui.ui.automation_drawing_area.queue_draw();
            }
            DAWEvents::RepaintTrackGridView => {
                gui.ui.track_drawing_area.queue_draw();
            }
            DAWEvents::RepaintPianoRollView => {
                gui.ui.piano_roll_drawing_area.queue_draw();
            }
            DAWEvents::RepaintSampleRollDrawingArea => {
                gui.ui.sample_roll_drawing_area.queue_draw();
            }
            DAWEvents::RepaintRiffArrangementBox => {
                gui.ui.riff_arrangement_box.queue_draw();
            }
            DAWEvents::RepaintRiffSetsBox => {
                gui.ui.riff_sets_box.queue_draw();
            }
            DAWEvents::RepaintRiffSequencesBox => {
                gui.ui.riff_sequences_box.queue_draw();
            }
        },
        Err(_) => (),
    }
}

fn handle_automation_add(time: f64, value: i32, state: &Arc<Mutex<DAWState>>) {
    match state.lock() {
        Ok(mut state) => {
            match state.automation_view_mode() {
                AutomationViewMode::Controllers => handle_automation_controller_add(time, value, &mut state),
                AutomationViewMode::Instrument => handle_automation_instrument_add(time, value, &mut state),
                AutomationViewMode::Effect => handle_automation_effect_add(time, value, &mut state),
                AutomationViewMode::NoteExpression => handle_automation_note_expression_add(time, value, &mut state),
                _ => (),
            }            
        }
        Err(_) => {

        }
    }
}

fn handle_automation_instrument_add(time: f64, value: i32, state: &mut DAWState) {
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_type();
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();

    if let Some(track_type) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
        if let TrackType::InstrumentTrack(instrument_track) = track_type {
            let plugin_uuid = instrument_track.instrument().uuid();
            let events = if let CurrentView::RiffArrangement = current_view {
                let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                    Some(selected_arrangement_uuid.clone())
                } else {
                    None
                };

                // get the arrangement
                if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                    if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                        if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            Some(riff_arr_automation.events_mut())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                match automation_edit_type {
                    AutomationEditType::Track => {
                        Some(track_type.automation_mut().events_mut())
                    }
                    AutomationEditType::Riff => {
                        if let Some(selected_riff_uuid) = selected_riff_uuid {
                            if let Some(riff) = track_type.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
                                Some(riff.events_mut())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                }
            };

            if let Some(automation_type_value) = automation_type {
                let parameter = PluginParameter {
                    id: Uuid::new_v4(),
                    plugin_uuid: plugin_uuid,
                    instrument: true,
                    position: time,
                    index: automation_type_value,
                    value: value as f32 / 127.0,
                };
                if let Some(events) = events {
                    events.push(TrackEvent::AudioPluginParameter(parameter));
                }
            }
        }
    }
}

fn handle_automation_note_expression_add(time: f64, value: i32, state: &mut DAWState) {
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();
    let note_expression_type = state.note_expression_type().clone();
    let note_expression_id = state.note_expression_id();
    let note_expression_port_index = state.note_expression_port_index() as i16;
    let note_expression_channel = state.note_expression_channel() as i16;
    let note_expression_key = state.note_expression_key();

    if let Some(track_type) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
        if let TrackType::InstrumentTrack(_instrument_track) = track_type {
            let events = if let CurrentView::RiffArrangement = current_view {
                let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                    Some(selected_arrangement_uuid.clone())
                } else {
                    None
                };

                // get the arrangement
                if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                    if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                        if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            Some(riff_arr_automation.events_mut())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                match automation_edit_type {
                    AutomationEditType::Track => {
                        Some(track_type.automation_mut().events_mut())
                    }
                    AutomationEditType::Riff => {
                        if let Some(selected_riff_uuid) = selected_riff_uuid {
                            if let Some(riff) = track_type.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
                                Some(riff.events_mut())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                }
            };

            let note_expression = NoteExpression::new_with_params(
                note_expression_type, 
                note_expression_port_index, 
                note_expression_channel, 
                time, 
                note_expression_id,
                note_expression_key, 
                value as f64 / 127.0
            );
            if let Some(events) = events {
                events.push(TrackEvent::NoteExpression(note_expression));
            }
        }
    }
}

fn handle_automation_effect_add(time: f64, value: i32, state: &mut DAWState) {
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_type();
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();
    let selected_effect_uuid = if let Some(selected_effect_uuid) = state.selected_effect_plugin_uuid() {
        Some(selected_effect_uuid.clone())
    }
    else {
        None
    };
    
    if let Some(track_type) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
        let appropriate_track_type = match track_type {
            TrackType::InstrumentTrack(_) => true,
            TrackType::AudioTrack(_) => true,
            TrackType::MidiTrack(_) => false,
        };
        if appropriate_track_type {
            if let Some(selected_effect_uuid) = selected_effect_uuid {
                let events = if let CurrentView::RiffArrangement = current_view {
                    let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                        Some(selected_arrangement_uuid.clone())
                    } else {
                        None
                    };

                    // get the arrangement
                    if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                        if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                            if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                Some(riff_arr_automation.events_mut())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    match automation_edit_type {
                        AutomationEditType::Track => {
                            Some(track_type.automation_mut().events_mut())
                        }
                        AutomationEditType::Riff => {
                            if let Some(selected_riff_uuid) = selected_riff_uuid {
                                if let Some(riff) = track_type.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
                                    Some(riff.events_mut())
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        }
                    }
                };
    
                if let Some(automation_type_value) = automation_type {
                    let parameter = PluginParameter {
                        id: Uuid::new_v4(),
                        plugin_uuid: Uuid::parse_str(selected_effect_uuid.as_str()).unwrap_or(Uuid::nil()),
                        instrument: false,
                        position: time,
                        index: automation_type_value,
                        value: value as f32 / 127.0,
                    };
                    if let Some(events) = events {
                        events.push(TrackEvent::AudioPluginParameter(parameter));
                    }
                }
            }
        }
    }
}

fn handle_automation_controller_add(time: f64, value: i32, state: &mut DAWState) {
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_type();
    let automation_edit_type = state.automation_edit_type();
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();

    if let Some(track_type) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
        let events = if let CurrentView::RiffArrangement = current_view {
            let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                Some(selected_arrangement_uuid.clone())
            } else {
                None
            };

            // get the arrangement
            if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                    if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                        Some(riff_arr_automation.events_mut())
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            match automation_edit_type {
                AutomationEditType::Track => {
                    Some(track_type.automation_mut().events_mut())
                }
                AutomationEditType::Riff => {
                    if let Some(selected_riff_uuid) = selected_riff_uuid {
                        if let Some(riff) = track_type.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
                            Some(riff.events_mut())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
            }
        };

        if let Some(automation_type_value) = automation_type {
            if let Some(events) = events {
                let controller = Controller::new(time, automation_type_value, value);
                events.push(TrackEvent::Controller(controller));
            }
        }
    }
}

fn handle_automation_delete(time: f64, state: &Arc<Mutex<DAWState>>) {
    match state.lock() {
        Ok(mut state) => {
            match state.automation_view_mode() {
                AutomationViewMode::Controllers => handle_automation_controller_delete(time, &mut state),
                AutomationViewMode::Instrument => handle_automation_instrument_delete(time, &mut state),
                AutomationViewMode::Effect => handle_automation_effect_delete(time, &mut state),
                AutomationViewMode::NoteExpression => handle_automation_note_expression_delete(time, &mut state),
                _ => (),
            }            
        }
        Err(_) => {

        }
    }
}

fn handle_automation_instrument_delete(time: f64, state: &mut DAWState) {
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_type();
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();
    
    if let Some(track_type) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
        if let TrackType::InstrumentTrack(instrument_track) = track_type {
            let plugin_uuid = instrument_track.instrument().uuid();
            let events = if let CurrentView::RiffArrangement = current_view {
                let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                    Some(selected_arrangement_uuid.clone())
                } else {
                    None
                };

                // get the arrangement
                if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                    if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                        if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            Some(riff_arr_automation.events_mut())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                match automation_edit_type {
                    AutomationEditType::Track => {
                        Some(track_type.automation_mut().events_mut())
                    }
                    AutomationEditType::Riff => {
                        if let Some(selected_riff_uuid) = selected_riff_uuid {
                            if let Some(riff) = track_type.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
                                Some(riff.events_mut())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                }
            };

            if let Some(automation_type_value) = automation_type {
                if let Some(events) = events {
                    events.retain(|event| {
                        match event {
                            TrackEvent::AudioPluginParameter(plugin_parameter) => {
                                !(plugin_parameter.index == automation_type_value &&
                                    (time - EVENT_DELETION_BEAT_TOLERANCE) <= plugin_parameter.position() &&
                                    plugin_parameter.position() <= (time + EVENT_DELETION_BEAT_TOLERANCE) &&
                                    plugin_parameter.plugin_uuid() == plugin_uuid.to_string() &&
                                    plugin_parameter.instrument()
                                )
                            },
                            _ => true,
                        }
                    });
                }
            }
        }
    }
}

fn handle_automation_note_expression_delete(time: f64, state: &mut DAWState) {
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();

    if let Some(track_type) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
        if let TrackType::InstrumentTrack(_instrument_track) = track_type {
            let events = if let CurrentView::RiffArrangement = current_view {
                let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                    Some(selected_arrangement_uuid.clone())
                } else {
                    None
                };

                // get the arrangement
                if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                    if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                        if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            Some(riff_arr_automation.events_mut())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                match automation_edit_type {
                    AutomationEditType::Track => {
                        Some(track_type.automation_mut().events_mut())
                    }
                    AutomationEditType::Riff => {
                        if let Some(selected_riff_uuid) = selected_riff_uuid {
                            if let Some(riff) = track_type.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
                                Some(riff.events_mut())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                }
            };

            if let Some(events) = events {
                events.retain(|event| {
                    match event {
                        TrackEvent::NoteExpression(note_expression) => {
                            !((time - EVENT_DELETION_BEAT_TOLERANCE) <= note_expression.position() && note_expression.position() <= (time + EVENT_DELETION_BEAT_TOLERANCE))
                        },
                        _ => true,
                    }
                });
            }
        }
    }
}

fn handle_automation_effect_delete(time: f64, state: &mut DAWState) {
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_type();
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();
    let selected_effect_uuid = if let Some(selected_effect_uuid) = state.selected_effect_plugin_uuid() {
        Some(selected_effect_uuid.clone())
    }
    else {
        None
    };
    
    if let Some(track_type) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
        let appropriate_track_type = match track_type {
            TrackType::InstrumentTrack(_) => true,
            TrackType::AudioTrack(_) => true,
            TrackType::MidiTrack(_) => false,
        };
        if appropriate_track_type {
            if let Some(selected_effect_uuid) = selected_effect_uuid {
                let events = if let CurrentView::RiffArrangement = current_view {
                    let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                        Some(selected_arrangement_uuid.clone())
                    } else {
                        None
                    };

                    // get the arrangement
                    if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                        if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                            if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                Some(riff_arr_automation.events_mut())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    match automation_edit_type {
                        AutomationEditType::Track => {
                            Some(track_type.automation_mut().events_mut())
                        }
                        AutomationEditType::Riff => {
                            if let Some(selected_riff_uuid) = selected_riff_uuid {
                                if let Some(riff) = track_type.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
                                    Some(riff.events_mut())
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        }
                    }
                };
    
                if let Some(automation_type_value) = automation_type {
                    if let Some(events) = events {
                        events.retain(|event| {
                            match event {
                                TrackEvent::AudioPluginParameter(plugin_parameter) => {
                                    !(plugin_parameter.index == automation_type_value &&
                                        (time - EVENT_DELETION_BEAT_TOLERANCE) <= plugin_parameter.position() &&
                                        plugin_parameter.position() <= (time + EVENT_DELETION_BEAT_TOLERANCE) &&
                                        plugin_parameter.plugin_uuid() == selected_effect_uuid &&
                                        !plugin_parameter.instrument()
                                    )
                                },
                                _ => true,
                            }
                        });
                    }
                }
            }
        }
    }
}

fn handle_automation_controller_delete(time: f64, state: &mut DAWState) {
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_type();
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();
    
    if let Some(track_type) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
        let events = if let CurrentView::RiffArrangement = current_view {
            let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                Some(selected_arrangement_uuid.clone())
            } else {
                None
            };

            // get the arrangement
            if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                    if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                        Some(riff_arr_automation.events_mut())
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            match automation_edit_type {
                AutomationEditType::Track => {
                    Some(track_type.automation_mut().events_mut())
                }
                AutomationEditType::Riff => {
                    if let Some(selected_riff_uuid) = selected_riff_uuid {
                        if let Some(riff) = track_type.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
                            Some(riff.events_mut())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
            }
        };

        if let Some(automation_type_value) = automation_type {
            if let Some(events) = events {
                events.retain(|event| {
                    match event {
                        TrackEvent::Controller(controller) => {
                            !(controller.controller() == automation_type_value && (time - EVENT_DELETION_BEAT_TOLERANCE) <= controller.position() && controller.position() <= (time + EVENT_DELETION_BEAT_TOLERANCE))
                        },
                        _ => true,
                    }
                });
            }
        }
    }
}

fn handle_automation_cut(state: &Arc<Mutex<DAWState>>) {
    match state.lock() {
        Ok(mut state) => {
            match state.automation_view_mode() {
                AutomationViewMode::Controllers => handle_automation_controller_cut(&mut state),
                AutomationViewMode::Instrument => handle_automation_instrument_cut(&mut state),
                AutomationViewMode::Effect => handle_automation_effect_cut(&mut state),
                AutomationViewMode::NoteExpression => handle_automation_note_expression_cut(&mut state),
                _ => (),
            }            
        }
        Err(_) => {

        }
    }
}

fn handle_automation_instrument_cut(state: &mut DAWState) {
    let selected = state.selected_automation().to_vec();
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_type();
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();
    
    if let Some(track_type) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
        if let TrackType::InstrumentTrack(instrument_track) = track_type {
            let plugin_uuid = instrument_track.instrument().uuid();
            let events = if let CurrentView::RiffArrangement = current_view {
                let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                    Some(selected_arrangement_uuid.clone())
                } else {
                    None
                };

                // get the arrangement
                if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                    if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                        if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            Some(riff_arr_automation.events_mut())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                match automation_edit_type {
                    AutomationEditType::Track => {
                        Some(track_type.automation_mut().events_mut())
                    }
                    AutomationEditType::Riff => {
                        if let Some(selected_riff_uuid) = selected_riff_uuid {
                            if let Some(riff) = track_type.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
                                Some(riff.events_mut())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                }
            };

            if let Some(automation_type_value) = automation_type {
                if let Some(events) = events {
                    events.retain(|event| {
                        match event {
                            TrackEvent::AudioPluginParameter(plugin_param) => {
                                !(plugin_param.index == automation_type_value && selected.contains(&event.id()))
                            },
                            _ => true,
                        }
                    });
                }
            }
        }
    }
}

fn handle_automation_note_expression_cut(state: &mut DAWState) {
    let selected = state.selected_automation().to_vec();
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();

    if let Some(track_type) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
        if let TrackType::InstrumentTrack(_instrument_track) = track_type {
            let events = if let CurrentView::RiffArrangement = current_view {
                let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                    Some(selected_arrangement_uuid.clone())
                } else {
                    None
                };

                // get the arrangement
                if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                    if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                        if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            Some(riff_arr_automation.events_mut())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                match automation_edit_type {
                    AutomationEditType::Track => {
                        Some(track_type.automation_mut().events_mut())
                    }
                    AutomationEditType::Riff => {
                        if let Some(selected_riff_uuid) = selected_riff_uuid {
                            if let Some(riff) = track_type.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
                                Some(riff.events_mut())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                }
            };

            if let Some(events) = events {
                events.retain(|event| {
                    match event {
                        TrackEvent::NoteExpression(note_expression) => {
                            !(selected.contains(&note_expression.id()))
                        },
                        _ => true,
                    }
                });
            }
        }
    }
}

fn handle_automation_effect_cut(state: &mut DAWState) {
    let selected = state.selected_automation().to_vec();
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_type();
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();
    let selected_effect_uuid = if let Some(selected_effect_uuid) = state.selected_effect_plugin_uuid() {
        Some(selected_effect_uuid.clone())
    }
    else {
        None
    };
    
    if let Some(track_type) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
        let appropriate_track_type = match track_type {
            TrackType::InstrumentTrack(_) => true,
            TrackType::AudioTrack(_) => true,
            TrackType::MidiTrack(_) => false,
        };
        if appropriate_track_type {
            if let Some(selected_effect_uuid) = selected_effect_uuid {
                let events = if let CurrentView::RiffArrangement = current_view {
                    let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                        Some(selected_arrangement_uuid.clone())
                    } else {
                        None
                    };

                    // get the arrangement
                    if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                        if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                            if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                Some(riff_arr_automation.events_mut())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    match automation_edit_type {
                        AutomationEditType::Track => {
                            Some(track_type.automation_mut().events_mut())
                        }
                        AutomationEditType::Riff => {
                            if let Some(selected_riff_uuid) = selected_riff_uuid {
                                if let Some(riff) = track_type.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
                                    Some(riff.events_mut())
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        }
                    }
                };
    
                if let Some(automation_type_value) = automation_type {
                    if let Some(events) = events {
                        events.retain(|event| {
                            match event {
                                TrackEvent::AudioPluginParameter(plugin_param) => {
                                    !(plugin_param.index == automation_type_value && selected.contains(&plugin_param.id()))
                                },
                                _ => true,
                            }
                        });
                    }
                }
            }
        }
    }
}

fn handle_automation_controller_cut(state: &mut DAWState) {
    let selected = state.selected_automation().to_vec();
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_type();
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();
    
    if let Some(track_type) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
        let events = if let CurrentView::RiffArrangement = current_view {
            let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                Some(selected_arrangement_uuid.clone())
            } else {
                None
            };

            // get the arrangement
            if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                    if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                        Some(riff_arr_automation.events_mut())
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            match automation_edit_type {
                AutomationEditType::Track => {
                    Some(track_type.automation_mut().events_mut())
                }
                AutomationEditType::Riff => {
                    if let Some(selected_riff_uuid) = selected_riff_uuid {
                        if let Some(riff) = track_type.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
                            Some(riff.events_mut())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
            }
        };

        if let Some(automation_type_value) = automation_type {
            if let Some(events) = events {
                events.retain(|event| {
                    match event {
                        TrackEvent::Controller(controller) => {
                            !(controller.controller() == automation_type_value && selected.contains(&controller.id())
                            )
                        },
                        _ => true,
                    }
                });
            }
        }
    }
}

fn handle_automation_copy(state: &Arc<Mutex<DAWState>>, edit_cursor_time_in_beats: f64) {
    match state.lock() {
        Ok(mut state) => {
            match state.automation_view_mode() {
                AutomationViewMode::Controllers => handle_automation_controller_copy(&mut state, edit_cursor_time_in_beats),
                AutomationViewMode::Instrument => handle_automation_instrument_copy(&mut state, edit_cursor_time_in_beats),
                AutomationViewMode::Effect => handle_automation_effect_copy(&mut state, edit_cursor_time_in_beats),
                AutomationViewMode::NoteExpression => handle_automation_note_expression_copy(&mut state, edit_cursor_time_in_beats),
                _ => (),
            }
        }
        Err(_) => {

        }
    }
}

fn handle_automation_instrument_copy(state: &mut DAWState, edit_cursor_time_in_beats: f64) {
    let selected = state.selected_automation().to_vec();
    let mut events_to_copy = vec![];
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_type();
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();

    if let Some(track_type) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
        if let TrackType::InstrumentTrack(instrument_track) = track_type {
            let plugin_uuid = instrument_track.instrument().uuid();
            let events = if let CurrentView::RiffArrangement = current_view {
                let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                    Some(selected_arrangement_uuid.clone())
                } else {
                    None
                };

                // get the arrangement
                if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                    if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                        if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            Some(riff_arr_automation.events_mut())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                match automation_edit_type {
                    AutomationEditType::Track => {
                        Some(track_type.automation_mut().events_mut())
                    }
                    AutomationEditType::Riff => {
                        if let Some(selected_riff_uuid) = selected_riff_uuid {
                            if let Some(riff) = track_type.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
                                Some(riff.events_mut())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                }
            };

            if let Some(automation_type_value) = automation_type {
                if let Some(events) = events {
                    for event in events.iter().filter(|event| selected.contains(&event.id())) {
                        if let TrackEvent::AudioPluginParameter(plugin_param) = event {
                            if plugin_param.index == automation_type_value {
                                let mut track_event = event.clone();
                                // adjust the position to be relative to the edit cursor
                                track_event.set_position(track_event.position() - edit_cursor_time_in_beats);
                                events_to_copy.push(track_event);
                            }
                        }
                    }
                }
            }
        }
    }

    if !events_to_copy.is_empty() {
        state.automation_event_copy_buffer_mut().clear();
        for event in events_to_copy.iter() {
            state.automation_event_copy_buffer_mut().push(event.clone());
        }
    }
}

fn handle_automation_note_expression_copy(state: &mut DAWState, edit_cursor_time_in_beats: f64) {
    let selected = state.selected_automation().to_vec();
    let mut events_to_copy = vec![];
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();

    if let Some(track_type) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
        if let TrackType::InstrumentTrack(_instrument_track) = track_type {
            let events = if let CurrentView::RiffArrangement = current_view {
                let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                    Some(selected_arrangement_uuid.clone())
                } else {
                    None
                };

                // get the arrangement
                if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                    if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                        if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            Some(riff_arr_automation.events_mut())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                match automation_edit_type {
                    AutomationEditType::Track => {
                        Some(track_type.automation_mut().events_mut())
                    }
                    AutomationEditType::Riff => {
                        if let Some(selected_riff_uuid) = selected_riff_uuid {
                            if let Some(riff) = track_type.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
                                Some(riff.events_mut())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                }
            };

            if let Some(events) = events {
                for event in events.iter().filter(|event| selected.contains(&event.id())) {
                    if let TrackEvent::NoteExpression(note_expression) = event {
                        let mut track_event = event.clone();
                        // adjust the position to be relative to the edit cursor
                        track_event.set_position(track_event.position() - edit_cursor_time_in_beats);
                        events_to_copy.push(track_event);
                    }
                }
            }
        }
    }

    if !events_to_copy.is_empty() {
        state.automation_event_copy_buffer_mut().clear();
        for event in events_to_copy.iter() {
            state.automation_event_copy_buffer_mut().push(event.clone());
        }
    }
}

fn handle_automation_effect_copy(state: &mut DAWState, edit_cursor_time_in_beats: f64) {
    let selected = state.selected_automation().to_vec();
    let mut events_to_copy = vec![];
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_type();
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();
    let selected_effect_uuid = if let Some(selected_effect_uuid) = state.selected_effect_plugin_uuid() {
        Some(selected_effect_uuid.clone())
    }
    else {
        None
    };

    if let Some(track_type) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
        let appropriate_track_type = match track_type {
            TrackType::InstrumentTrack(_) => true,
            TrackType::AudioTrack(_) => true,
            TrackType::MidiTrack(_) => false,
        };
        if appropriate_track_type {
            if let Some(selected_effect_uuid) = selected_effect_uuid {
                let events = if let CurrentView::RiffArrangement = current_view {
                    let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                        Some(selected_arrangement_uuid.clone())
                    } else {
                        None
                    };

                    // get the arrangement
                    if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                        if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                            if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                Some(riff_arr_automation.events_mut())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    match automation_edit_type {
                        AutomationEditType::Track => {
                            Some(track_type.automation_mut().events_mut())
                        }
                        AutomationEditType::Riff => {
                            if let Some(selected_riff_uuid) = selected_riff_uuid {
                                if let Some(riff) = track_type.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
                                    Some(riff.events_mut())
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        }
                    }
                };

                if let Some(automation_type_value) = automation_type {
                    if let Some(events) = events {
                        for event in events.iter().filter(|event| selected.contains(&event.id())) {
                            if let TrackEvent::AudioPluginParameter(plugin_param) = event {
                                if plugin_param.index == automation_type_value {
                                    let mut track_event = event.clone();
                                    // adjust the position to be relative to the edit cursor
                                    track_event.set_position(track_event.position() - edit_cursor_time_in_beats);
                                    events_to_copy.push(track_event);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if !events_to_copy.is_empty() {
        state.automation_event_copy_buffer_mut().clear();
        for event in events_to_copy.iter() {
            state.automation_event_copy_buffer_mut().push(event.clone());
        }
    }
}

fn handle_automation_controller_copy(state: &mut DAWState, edit_cursor_time_in_beats: f64) {
    let selected = state.selected_automation().to_vec();
    let mut events_to_copy = vec![];
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_type();
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();

    if let Some(track_type) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
        let events = if let CurrentView::RiffArrangement = current_view {
            let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                Some(selected_arrangement_uuid.clone())
            } else {
                None
            };

            // get the arrangement
            if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                    if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                        Some(riff_arr_automation.events_mut())
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            match automation_edit_type {
                AutomationEditType::Track => {
                    Some(track_type.automation_mut().events_mut())
                }
                AutomationEditType::Riff => {
                    if let Some(selected_riff_uuid) = selected_riff_uuid {
                        if let Some(riff) = track_type.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
                            Some(riff.events_mut())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
            }
        };

        if let Some(automation_type_value) = automation_type {
            if let Some(events) = events {
                for event in events.iter().filter(|event| selected.contains(&event.id())) {
                    if let TrackEvent::Controller(controller) = event {
                        if controller.controller() == automation_type_value {
                            let mut track_event = event.clone();
                            // adjust the position to be relative to the edit cursor
                            track_event.set_position(track_event.position() - edit_cursor_time_in_beats);
                            events_to_copy.push(track_event);
                        }
                    }
                }
            }
        }
    }

    if !events_to_copy.is_empty() {
        state.automation_event_copy_buffer_mut().clear();
        for event in events_to_copy.iter() {
            state.automation_event_copy_buffer_mut().push(event.clone());
        }
    }
}

fn handle_automation_paste(state: &Arc<Mutex<DAWState>>, edit_cursor_time_in_beats: f64) {
    match state.lock() {
        Ok(mut state) => {
            match state.automation_view_mode() {
                AutomationViewMode::Controllers => handle_automation_controller_paste(&mut state, edit_cursor_time_in_beats),
                AutomationViewMode::Instrument => handle_automation_instrument_paste(&mut state, edit_cursor_time_in_beats),
                AutomationViewMode::Effect => handle_automation_effect_paste(&mut state, edit_cursor_time_in_beats),
                AutomationViewMode::NoteExpression => handle_automation_note_expression_paste(&mut state, edit_cursor_time_in_beats),
                _ => (),
            }
        }
        Err(_) => {

        }
    }
}

fn handle_automation_instrument_paste(state: &mut DAWState, edit_cursor_time_in_beats: f64) {
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_type();
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();
    let automation_event_copy_buffer = state.automation_event_copy_buffer().iter().map(|event| event.clone()).collect_vec();

    if let Some(track_type) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
        if let TrackType::InstrumentTrack(instrument_track) = track_type {
            let plugin_uuid = instrument_track.instrument().uuid();
            let events = if let CurrentView::RiffArrangement = current_view {
                let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                    Some(selected_arrangement_uuid.clone())
                } else {
                    None
                };

                // get the arrangement
                if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                    if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                        if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            Some(riff_arr_automation.events_mut())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                match automation_edit_type {
                    AutomationEditType::Track => {
                        Some(track_type.automation_mut().events_mut())
                    }
                    AutomationEditType::Riff => {
                        if let Some(selected_riff_uuid) = selected_riff_uuid {
                            if let Some(riff) = track_type.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
                                Some(riff.events_mut())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                }
            };

            if let Some(automation_type_value) = automation_type {
                if let Some(events) = events {
                    for event in automation_event_copy_buffer {
                        if let TrackEvent::AudioPluginParameter(plugin_param) = event {
                            if plugin_param.index == automation_type_value {
                                let mut track_event = event.clone();
                                // adjust the position to be relative to the edit cursor
                                track_event.set_position(edit_cursor_time_in_beats + track_event.position());
                                events.push(track_event);
                            }
                        }
                    }
                }
            }
        }
    }
}

fn handle_automation_note_expression_paste(state: &mut DAWState, edit_cursor_time_in_beats: f64) {
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();
    let automation_event_copy_buffer = state.automation_event_copy_buffer().iter().map(|event| event.clone()).collect_vec();

    if let Some(track_type) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
        if let TrackType::InstrumentTrack(_instrument_track) = track_type {
            let events = if let CurrentView::RiffArrangement = current_view {
                let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                    Some(selected_arrangement_uuid.clone())
                } else {
                    None
                };

                // get the arrangement
                if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                    if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                        if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            Some(riff_arr_automation.events_mut())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                match automation_edit_type {
                    AutomationEditType::Track => {
                        Some(track_type.automation_mut().events_mut())
                    }
                    AutomationEditType::Riff => {
                        if let Some(selected_riff_uuid) = selected_riff_uuid {
                            if let Some(riff) = track_type.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
                                Some(riff.events_mut())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                }
            };

            if let Some(events) = events {
                for event in automation_event_copy_buffer {
                    if let TrackEvent::NoteExpression(note_expression) = event {
                        let mut track_event = event.clone();
                        // adjust the position to be relative to the edit cursor
                        track_event.set_position(edit_cursor_time_in_beats + track_event.position());
                        events.push(track_event);
                    }
                }
            }
        }
    }
}

fn handle_automation_effect_paste(state: &mut DAWState, edit_cursor_time_in_beats: f64) {
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_type();
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();
    let selected_effect_uuid = if let Some(selected_effect_uuid) = state.selected_effect_plugin_uuid() {
        Some(selected_effect_uuid.clone())
    }
    else {
        None
    };
    let automation_event_copy_buffer = state.automation_event_copy_buffer().iter().map(|event| event.clone()).collect_vec();

    if let Some(track_type) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
        let appropriate_track_type = match track_type {
            TrackType::InstrumentTrack(_) => true,
            TrackType::AudioTrack(_) => true,
            TrackType::MidiTrack(_) => false,
        };
        if appropriate_track_type {
            if let Some(selected_effect_uuid) = selected_effect_uuid {
                let events = if let CurrentView::RiffArrangement = current_view {
                    let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                        Some(selected_arrangement_uuid.clone())
                    } else {
                        None
                    };

                    // get the arrangement
                    if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                        if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                            if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                Some(riff_arr_automation.events_mut())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    match automation_edit_type {
                        AutomationEditType::Track => {
                            Some(track_type.automation_mut().events_mut())
                        }
                        AutomationEditType::Riff => {
                            if let Some(selected_riff_uuid) = selected_riff_uuid {
                                if let Some(riff) = track_type.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
                                    Some(riff.events_mut())
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        }
                    }
                };

                if let Some(automation_type_value) = automation_type {
                    if let Some(events) = events {
                        for event in automation_event_copy_buffer {
                            if let TrackEvent::AudioPluginParameter(plugin_param) = event {
                                if plugin_param.index == automation_type_value {
                                    let mut track_event = event.clone();
                                    // adjust the position to be relative to the edit cursor
                                    track_event.set_position(edit_cursor_time_in_beats + track_event.position());
                                    events.push(track_event);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn handle_automation_controller_paste(state: &mut DAWState, edit_cursor_time_in_beats: f64) {
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_type();
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();
    let automation_event_copy_buffer = state.automation_event_copy_buffer().iter().map(|event| event.clone()).collect_vec();

    if let Some(track_type) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
        let events = if let CurrentView::RiffArrangement = current_view {
            let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                Some(selected_arrangement_uuid.clone())
            } else {
                None
            };

            // get the arrangement
            if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                    if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                        Some(riff_arr_automation.events_mut())
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            match automation_edit_type {
                AutomationEditType::Track => {
                    Some(track_type.automation_mut().events_mut())
                }
                AutomationEditType::Riff => {
                    if let Some(selected_riff_uuid) = selected_riff_uuid {
                        if let Some(riff) = track_type.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
                            Some(riff.events_mut())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
            }
        };

        if let Some(automation_type_value) = automation_type {
            if let Some(events) = events {
                for event in automation_event_copy_buffer {
                    if let TrackEvent::Controller(controller) = event {
                        if controller.controller() == automation_type_value {
                            let mut track_event = event.clone();
                            // adjust the position to be relative to the edit cursor
                            track_event.set_position(edit_cursor_time_in_beats + track_event.position());
                            events.push(track_event);
                        }
                    }
                }
            }
        }
    }
}

fn do_progress_dialogue_pulse(gui: &mut MainWindow, progress_bar_pulse_delay_count: &mut i32) {
    if gui.ui.progress_dialogue.is_visible() {
        if *progress_bar_pulse_delay_count > 10000 {
            *progress_bar_pulse_delay_count = 0;
            gui.ui.dialogue_progress_bar.pulse();
        } else {
            *progress_bar_pulse_delay_count += 1;
        }
    }
}

fn create_jack_event_processing_thread(
    tx_from_ui: Sender<DAWEvents>,
    jack_midi_receiver: Receiver<AudioLayerOutwardEvent>,
    state: Arc<Mutex<DAWState>>,
) {

    let _ = ThreadBuilder::default()
            .name("jack_event_proc")
            .priority(ThreadPriority::Crossplatform(95.try_into().unwrap()))
            .spawn(move |result| {
                match result {
                    Ok(_) => debug!("Thread set to max priority: 95."),
                    Err(error) => debug!("Could not set thread to max priority: {:?}.", error),
                }
                let mut recorded_playing_notes: HashMap<i32, f64> = HashMap::new() ;

                loop {
                    match jack_midi_receiver.try_recv() {
                        Ok(audio_layer_outward_event) => {
                            match audio_layer_outward_event {
                                AudioLayerOutwardEvent::MidiEvent(jack_midi_event) => {
                                let midi_msg_type = jack_midi_event.data[0] as i32;

                                match state.lock() {
                                    Ok(state) => {
                                        match state.selected_track() {
                                            Some(track_uuid) => {
                                                match state.project().song().tracks().iter().find(|track| track.uuid().to_string() == track_uuid) {
                                                    Some(track) => {
                                                        let midi_channel = if let TrackType::MidiTrack(midi_track) = track {
                                                            midi_track.midi_device().midi_channel()
                                                        } else {
                                                            0
                                                        };
                                                        if (144..=159).contains(&midi_msg_type) {
                                                            state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::PlayNoteImmediate(jack_midi_event.data[1] as i32, midi_channel));
                                                        }
                                                        else if (128..=143).contains(&midi_msg_type) {
                                                            state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::StopNoteImmediate(jack_midi_event.data[1] as i32, midi_channel));
                                                        }
                                                        else if (176..=191).contains(&midi_msg_type) {
                                                            state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::PlayControllerImmediate(jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32, midi_channel));
                                                        }
                                                        else if (224..=239).contains(&midi_msg_type) {
                                                            state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::PlayPitchBendImmediate(jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32, midi_channel));
                                                        }
                                                        else {
                                                            debug!("Unknown jack midi event: ");
                                                            for event_byte in jack_midi_event.data.iter() {
                                                                debug!(" {}", event_byte);
                                                            }
                                                            debug!("");
                                                        }
                                                    },
                                                    None => (),
                                                };
                                            },
                                            None => debug!("Play note immediate: no track number given."),
                                        }
                                    },
                                    Err(_) => debug!("Main - jack_event_prcessing_thread processing loop - play note immediate - could not get lock on state"),
                                };
                                let mut selected_riff_uuid = None;
                                let mut selected_riff_track_uuid = None;
                                match state.lock() {
                                    Ok(state) => {
                                        selected_riff_track_uuid = state.selected_track();

                                        match selected_riff_track_uuid {
                                            Some(track_uuid) => {
                                                selected_riff_uuid = state.selected_riff_uuid(track_uuid.clone());
                                                selected_riff_track_uuid = Some(track_uuid);
                                            },
                                            None => (),
                                        }
                                    },
                                    Err(_) => debug!("Main - jack_event_prcessing_thread processing loop - Record - could not get lock on state"),
                                };
                                match state.lock() {
                                    Ok(mut state) => {
                                        let tempo = state.project().song().tempo();
                                        let sample_rate = state.project().song().sample_rate();

                                        if *state.playing_mut() && *state.recording_mut() {
                                            let play_mode = state.play_mode();
                                            let playing_riff_set = state.playing_riff_set().clone();
                                            let mut riff_changed = false;

                                            match selected_riff_track_uuid {
                                                Some(track_uuid) => {
                                                    match state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == *track_uuid) {
                                                        Some(track_type) => match track_type {
                                                            TrackType::InstrumentTrack(track) => {
                                                                match selected_riff_uuid {
                                                                    Some(uuid) => {
                                                                        for riff in track.riffs_mut().iter_mut() {
                                                                            if riff.uuid().to_string() == *uuid {
                                                                                if (144..=159).contains(&midi_msg_type) { //note on
                                                                                    debug!("Adding note to riff: delta frames={}, note={}, velocity={}", jack_midi_event.delta_frames, jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32);
                                                                                    let note = Note::new_with_params(
                                                                                        tempo / 60.0 * jack_midi_event.delta_frames as f64 / sample_rate, jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32, 0.2);
                                                                                    recorded_playing_notes.insert(note.note(), note.position());
                                                                                    riff.events_mut().push(TrackEvent::Note(note));
                                                                                }
                                                                                else if (128..=143).contains(&midi_msg_type) { // note off
                                                                                    let note_number = jack_midi_event.data[1] as i32;
                                                                                    if let Some(note_position) = recorded_playing_notes.get_mut(&note_number) {
                                                                                        // find the note in the riff
                                                                                        for track_event in riff.events_mut().iter_mut() {
                                                                                            if track_event.position() == *note_position {
                                                                                                if let TrackEvent::Note(note) = track_event {
                                                                                                    if note.note() == note_number {
                                                                                                        note.set_length(tempo / 60.0 * jack_midi_event.delta_frames as f64 / sample_rate - note.position());
                                                                                                        riff_changed = true;
                                                                                                        break;
                                                                                                    }
                                                                                                }
                                                                                            }
                                                                                        }
                                                                                    }
                                                                                    recorded_playing_notes.remove(&note_number);
                                                                                }
                                                                                else if (176..=191).contains(&midi_msg_type) { // Controller - including modulation wheel
                                                                                    debug!("Adding controller to riff: delta frames={}, controller={}, value={}", jack_midi_event.delta_frames, jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32);
                                                                                    riff.events_mut().push(
                                                                                        TrackEvent::Controller(
                                                                                            Controller::new(
                                                                                                tempo / 60.0 * jack_midi_event.delta_frames as f64 / sample_rate, jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32)));
                                                                                }
                                                                                else if (224..=239).contains(&midi_msg_type) {
                                                                                    debug!("Adding pitch bend to riff: delta frames={}, lsb={}, msb={}", jack_midi_event.delta_frames, jack_midi_event.data[1], jack_midi_event.data[2]);
                                                                                    riff.events_mut().push(
                                                                                        TrackEvent::PitchBend(
                                                                                            PitchBend::new_from_midi_bytes(
                                                                                                tempo / 60.0 * jack_midi_event.delta_frames as f64 / sample_rate, jack_midi_event.data[1], jack_midi_event.data[2])));
                                                                                }

                                                                                break;
                                                                            }
                                                                        }
                                                                    },
                                                                    None => debug!("Jack midi receiver - no selected riff."),
                                                                }
                                                            },
                                                            TrackType::AudioTrack(_) => (),
                                                            TrackType::MidiTrack(_) => (),
                                                        },
                                                        None => (),
                                                    };

                                                    if play_mode == PlayMode::RiffSet && riff_changed {
                                                        if let Some(playing_riff_set) = playing_riff_set {
                                                            debug!("RiffSet riff updated - now calling state.play_riff_set_update_track");
                                                            state.play_riff_set_update_track_as_riff(playing_riff_set, track_uuid);
                                                        }
                                                    }
                                                },
                                                None => debug!("Record: no track number given."),
                                            }
                                        }
                                    },
                                    Err(_) => debug!("Main - jack_event_prcessing_thread processing loop - Record - could not get lock on state"),
                                };
                                }
                                AudioLayerOutwardEvent::PlayPositionInFrames(play_position_in_frames) => {
                                }
                                AudioLayerOutwardEvent::GeneralMMCEvent(mmc_sysex_bytes) => {
                                    debug!("Midi generic MMC event: ");
                                    let command_byte = mmc_sysex_bytes[4];
                                    match command_byte {
                                        1 => {
                                            match tx_from_ui.send(DAWEvents::TransportStop) {
                                                Ok(_) => {}
                                                Err(_) => {}
                                            }
                                        }
                                        2 => {
                                            match tx_from_ui.send(DAWEvents::TransportPlay) {
                                                Ok(_) => {}
                                                Err(_) => {}
                                            }
                                        }
                                        4 => {
                                            match tx_from_ui.send(DAWEvents::TransportMoveForward) {
                                                Ok(_) => {}
                                                Err(_) => {}
                                            }
                                        }
                                        5 => {
                                            match tx_from_ui.send(DAWEvents::TransportMoveBack) {
                                                Ok(_) => {}
                                                Err(_) => {}
                                            }
                                        }
                                        6 => {
                                            match state.lock() {
                                                Ok(state) => {
                                                    let recording = !state.recording();
                                                },
                                                Err(_) => debug!("Main - jack_event_prcessing_thread processing loop - record - could not get lock on state"),
                                            };
                                        }
                                        _ => {}
                                    }
                                }
                                AudioLayerOutwardEvent::MidiControlEvent(jack_midi_event) => {
                                    match state.lock() {
                                        Ok(mut state) => {
                                            if jack_midi_event.data[0] as i32 == 144 && jack_midi_event.data[1] as usize >= 36_usize {
                                                // let riff_thing_index = jack_midi_event.data[1] as usize - 36_usize;
                                                // let track_riffs_stack_visible_name = gui.get_track_riffs_stack_visible_name();
                                                // if track_riffs_stack_visible_name == "Track Grid" {
                                                //     state.play_song(tx_to_audio.clone());
                                                // } else if track_riffs_stack_visible_name == "Riffs" {
                                                //     let riffs_stack_visible_name = gui.get_riffs_stack_visible_name();
                                                //     if riffs_stack_visible_name == "riff_sets" {
                                                //         let riff_set_uuid = if let Some(riff_set) = state.get_project().song_mut().riff_sets_mut().get_mut(riff_thing_index) {
                                                //             riff_set.uuid()
                                                //         } else {
                                                //             "".to_string()
                                                //         };
                                                //         state.play_riff_set(tx_to_audio.clone(), riff_set_uuid);
                                                //     } else if riffs_stack_visible_name == "riff_sequences" {
                                                //         let riff_sequence_uuid = if let Some(riff_sequence) = state.get_project().song_mut().riff_sequences_mut().get_mut(riff_thing_index) {
                                                //             riff_sequence.uuid()
                                                //         } else {
                                                //             "".to_string()
                                                //         };
                                                //         state.play_riff_sequence(tx_to_audio.clone(), riff_sequence_uuid);
                                                //     } else if riffs_stack_visible_name == "riff_arrangement" {
                                                //         let riff_arrangement_uuid = if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangements_mut().get_mut(riff_thing_index) {
                                                //             riff_arrangement.uuid()
                                                //         } else {
                                                //             "".to_string()
                                                //         };
                                                //         state.play_riff_arrangement(tx_to_audio.clone(), riff_arrangement_uuid);
                                                //     }
                                                // }
                                            } else if jack_midi_event.data[0] as i32 >= 176 && (jack_midi_event.data[0] as i32 <= (176 + 15)) {
                                                debug!("Main - jack_event_prcessing_thread processing loop - jack AudioLayerOutwardEvent::MidiControlEvent - received a controller message: {} {} {}", jack_midi_event.data[0], jack_midi_event.data[1], jack_midi_event.data[2]);
                                                // need to send some track volume (176) or pan (177) messages
                                                let position_in_frames = jack_midi_event.delta_frames;
                                                let position_in_beats = (position_in_frames as f64) / state.project().song().sample_rate() * state.project().song().tempo() / 60.0;
                                                let track_index = jack_midi_event.data[1] as i32 - 1;
                                                let track_change_type = if jack_midi_event.data[0] as i32 == 176 {
                                                    TrackChangeType::Volume(Some(position_in_beats), jack_midi_event.data[2] as f32 / 127.0)
                                                } else {
                                                    TrackChangeType::Pan(Some(position_in_beats), (jack_midi_event.data[2] as f32 - 63.5) / 63.5)
                                                };

                                                if let Some(track) = state.project().song().tracks().get(track_index as usize) {
                                                    match tx_from_ui.send(DAWEvents::TrackChange(track_change_type, Some(track.uuid().to_string()))) {
                                                        Ok(_) => {}
                                                        Err(_) => {}
                                                    }
                                                }
                                            } else {
                                                debug!("Main - jack_event_prcessing_thread processing loop - jack AudioLayerOutwardEvent::MidiControlEvent - received a unknown message: {} {} {}", jack_midi_event.data[0], jack_midi_event.data[1], jack_midi_event.data[2]);
                                            }
                                        }
                                        Err(_) => {}
                                    }
                                }
                                AudioLayerOutwardEvent::JackRestartRequired => {
                                    // match state.lock() {
                                    //     Ok(mut state) => {
                                    //         state.restart_jack(rx_to_audio.clone(), jack_midi_sender.clone(), jack_audio_coast.clone(), vst_host_time_info.clone());
                                    //     }
                                    //     Err(_) => {}
                                    // }
                                }
                                AudioLayerOutwardEvent::JackConnect(jack_port_from_name, jack_port_to_name) => {
                                    // match state.lock() {
                                    //     Ok(mut state) => {
                                    //         state.jack_connection_add(jack_port_from_name, jack_port_to_name);
                                    //     }
                                    //     Err(_) => {}
                                    // }
                                }
                                AudioLayerOutwardEvent::MasterChannelLevels(left_channel_level, right_channel_level) => {},
                                }
                        },
                        Err(_) => (),
                    }

                    thread::sleep(Duration::from_millis(10));
                }
            });
}

fn process_jack_events(tx_from_ui: &Sender<DAWEvents>,
                       jack_midi_receiver: &Receiver<AudioLayerOutwardEvent>,
                       state: &mut Arc<Mutex<DAWState>>,
                       tx_to_audio: &Sender<AudioLayerInwardEvent>,
                       rx_to_audio: &Receiver<AudioLayerInwardEvent>,
                       jack_midi_sender: &Sender<AudioLayerOutwardEvent>,
                       jack_midi_sender_ui: &Sender<AudioLayerOutwardEvent>,
                       jack_audio_coast: &Arc<Mutex<TrackBackgroundProcessorMode>>,
                       gui: &mut MainWindow,
                       vst_host_time_info: &Arc<parking_lot::RwLock<TimeInfo>>,
                       recorded_playing_notes: &mut HashMap<i32, f64>,
) {
    match jack_midi_receiver.try_recv() {
        Ok(audio_layer_outward_event) => {
            match audio_layer_outward_event {
                AudioLayerOutwardEvent::MidiEvent(jack_midi_event) => {
                    // let midi_msg_type = jack_midi_event.data[0] as i32;

                    // match state.lock() {
                    //     Ok(state) => {
                    //         match state.selected_track() {
                    //             Some(track_uuid) => {
                    //                 match state.project().song().tracks().iter().find(|track| track.uuid().to_string() == track_uuid) {
                    //                     Some(track) => {
                    //                         let midi_channel = if let TrackType::MidiTrack(midi_track) = track {
                    //                             midi_track.midi_device().midi_channel()
                    //                         } else {
                    //                             0
                    //                         };
                    //                         if (144..=159).contains(&midi_msg_type) {
                    //                             state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::PlayNoteImmediate(jack_midi_event.data[1] as i32, midi_channel));
                    //                         }
                    //                         else if (128..=143).contains(&midi_msg_type) {
                    //                             state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::StopNoteImmediate(jack_midi_event.data[1] as i32, midi_channel));
                    //                         }
                    //                         else if (176..=191).contains(&midi_msg_type) {
                    //                             state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::PlayControllerImmediate(jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32, midi_channel));
                    //                         }
                    //                         else if (224..=239).contains(&midi_msg_type) {
                    //                             state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::PlayPitchBendImmediate(jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32, midi_channel));
                    //                         }
                    //                         else {
                    //                             debug!("Unknown jack midi event: ");
                    //                             for event_byte in jack_midi_event.data.iter() {
                    //                                 debug!(" {}", event_byte);
                    //                             }
                    //                             debug!("");
                    //                         }
                    //                     },
                    //                     None => (),
                    //                 };
                    //             },
                    //             None => debug!("Play note immediate: no track number given."),
                    //         }
                    //     },
                    //     Err(_) => debug!("Main - rx_ui processing loop - play note immediate - could not get lock on state"),
                    // };
                    // let mut selected_riff_uuid = None;
                    // let mut selected_riff_track_uuid = None;
                    // match state.lock() {
                    //     Ok(state) => {
                    //         selected_riff_track_uuid = state.selected_track();

                    //         match selected_riff_track_uuid {
                    //             Some(track_uuid) => {
                    //                 selected_riff_uuid = state.selected_riff_uuid(track_uuid.clone());
                    //                 selected_riff_track_uuid = Some(track_uuid);
                    //             },
                    //             None => (),
                    //         }
                    //     },
                    //     Err(_) => debug!("Main - rx_ui processing loop - Record - could not get lock on state"),
                    // };
                    // match state.lock() {
                    //     Ok(mut state) => {
                    //         let tempo = state.project().song().tempo();
                    //         let sample_rate = state.project().song().sample_rate();

                    //         if *state.playing_mut() && *state.recording_mut() {
                    //             let play_mode = state.play_mode();
                    //             let playing_riff_set = state.playing_riff_set().clone();
                    //             let mut riff_changed = false;

                    //             match selected_riff_track_uuid {
                    //                 Some(track_uuid) => {
                    //                     match state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == *track_uuid) {
                    //                         Some(track_type) => match track_type {
                    //                             TrackType::InstrumentTrack(track) => {
                    //                                 match selected_riff_uuid {
                    //                                     Some(uuid) => {
                    //                                         for riff in track.riffs_mut().iter_mut() {
                    //                                             if riff.uuid().to_string() == *uuid {
                    //                                                 if (144..=159).contains(&midi_msg_type) { //note on
                    //                                                     debug!("Adding note to riff: delta frames={}, note={}, velocity={}", jack_midi_event.delta_frames, jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32);
                    //                                                     let note = Note::new_with_params(
                    //                                                         tempo / 60.0 * jack_midi_event.delta_frames as f64 / sample_rate, jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32, 0.2);
                    //                                                     recorded_playing_notes.insert(note.note(), note.position());
                    //                                                     riff.events_mut().push(TrackEvent::Note(note));
                    //                                                 }
                    //                                                 else if (128..=143).contains(&midi_msg_type) { // note off
                    //                                                     let note_number = jack_midi_event.data[1] as i32;
                    //                                                     if let Some(note_position) = recorded_playing_notes.get_mut(&note_number) {
                    //                                                         // find the note in the riff
                    //                                                         for track_event in riff.events_mut().iter_mut() {
                    //                                                             if track_event.position() == *note_position {
                    //                                                                 if let TrackEvent::Note(note) = track_event {
                    //                                                                     if note.note() == note_number {
                    //                                                                         note.set_length(tempo / 60.0 * jack_midi_event.delta_frames as f64 / sample_rate - note.position());
                    //                                                                         riff_changed = true;
                    //                                                                         break;
                    //                                                                     }
                    //                                                                 }
                    //                                                             }
                    //                                                         }
                    //                                                     }
                    //                                                     recorded_playing_notes.remove(&note_number);
                    //                                                 }
                    //                                                 else if (176..=191).contains(&midi_msg_type) { // Controller - including modulation wheel
                    //                                                     debug!("Adding controller to riff: delta frames={}, controller={}, value={}", jack_midi_event.delta_frames, jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32);
                    //                                                     riff.events_mut().push(
                    //                                                         TrackEvent::Controller(
                    //                                                             Controller::new(
                    //                                                                 tempo / 60.0 * jack_midi_event.delta_frames as f64 / sample_rate, jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32)));
                    //                                                 }
                    //                                                 else if (224..=239).contains(&midi_msg_type) {
                    //                                                     debug!("Adding pitch bend to riff: delta frames={}, lsb={}, msb={}", jack_midi_event.delta_frames, jack_midi_event.data[1], jack_midi_event.data[2]);
                    //                                                     riff.events_mut().push(
                    //                                                         TrackEvent::PitchBend(
                    //                                                             PitchBend::new_from_midi_bytes(
                    //                                                                 tempo / 60.0 * jack_midi_event.delta_frames as f64 / sample_rate, jack_midi_event.data[1], jack_midi_event.data[2])));
                    //                                                 }

                    //                                                 break;
                    //                                             }
                    //                                         }
                    //                                     },
                    //                                     None => debug!("Jack midi receiver - no selected riff."),
                    //                                 }
                    //                             },
                    //                             TrackType::AudioTrack(_) => (),
                    //                             TrackType::MidiTrack(_) => (),
                    //                         },
                    //                         None => (),
                    //                     };

                    //                     if play_mode == PlayMode::RiffSet && riff_changed {
                    //                         if let Some(playing_riff_set) = playing_riff_set {
                    //                             debug!("RiffSet riff updated - now calling state.play_riff_set_update_track");
                    //                             state.play_riff_set_update_track(playing_riff_set, track_uuid);
                    //                         }
                    //                     }
                    //                 },
                    //                 None => debug!("Record: no track number given."),
                    //             }
                    //         }
                    //     },
                    //     Err(_) => debug!("Main - rx_ui processing loop - Record - could not get lock on state"),
                    // };
                    // // gui.ui.piano_roll_drawing_area.queue_draw();
                }
                AudioLayerOutwardEvent::PlayPositionInFrames(play_position_in_frames) => {
                    match state.lock() {
                        Ok(mut state) => {
                            let bpm = state.get_project().song().tempo();
                            let time_signature_numerator = state.get_project().song().time_signature_numerator();
                            let sample_rate = state.get_project().song().sample_rate();
                            let play_position_in_beats = play_position_in_frames as f64 / sample_rate * bpm / 60.0;

                            let current_bar = play_position_in_beats as i32 / time_signature_numerator as i32 + 1;
                            let current_beat_in_bar = play_position_in_beats as i32 % time_signature_numerator as i32 + 1;

                            gui.ui.song_position_txt_ctrl.set_label(format!("{:03}:{:03}:000", current_bar, current_beat_in_bar).as_str());

                            // debug!("Play position in frames: {}", play_position_in_frames);
                            state.set_play_position_in_frames(play_position_in_frames);
                            if let Some(piano_roll_grid) = gui.piano_roll_grid() {
                                match piano_roll_grid.lock() {
                                    Ok(mut grid) => grid.set_track_cursor_time_in_beats(play_position_in_beats),
                                    Err(_) => (),
                                }
                            }
                            if let Some(track_grid) = gui.track_grid() {
                                match track_grid.lock() {
                                    Ok(mut grid) => grid.set_track_cursor_time_in_beats(play_position_in_beats),
                                    Err(_) => (),
                                }
                            }
                            if let Some(sample_roll_grid) = gui.sample_roll_grid() {
                                match sample_roll_grid.lock() {
                                    Ok(mut grid) => grid.set_track_cursor_time_in_beats(play_position_in_beats),
                                    Err(_) => (),
                                }
                            }
                            if let Some(automation_grid) = gui.automation_grid() {
                                match automation_grid.lock() {
                                    Ok(mut grid) => grid.set_track_cursor_time_in_beats(play_position_in_beats),
                                    Err(_) => (),
                                }
                            }

                            if state.track_grid_cursor_follow() {
                                if let Some(track_grid_arc) = gui.track_grid() {
                                    if let Ok(track_grid) = track_grid_arc.lock() {
                                        let adjusted_beat_width_in_pixels = track_grid.beat_width_in_pixels() * track_grid.zoom_horizontal();
                                        let play_position_in_pixels = play_position_in_beats * adjusted_beat_width_in_pixels;
                                        let track_grid_width = gui.ui.track_drawing_area.width_request() as f64;
                                        let track_grid_horiz_adj: Adjustment = gui.ui.track_grid_scrolled_window.hadjustment();
                                        let range_max = track_grid_horiz_adj.upper();
                                        let track_grid_horiz_scroll_position = play_position_in_pixels / track_grid_width * range_max;

                                        track_grid_horiz_adj.set_value(track_grid_horiz_scroll_position - 300.0);
                                    }
                                    else {
                                        debug!("Couldn't lock the track_grid");
                                    }
                                }
                                else {
                                    debug!("Couldn't get the track_grid");
                                }
                            }

                            if let Some(riff_set_uuid) = state.playing_riff_set() {
                                gui.repaint_riff_set_view_riff_set_active_drawing_areas(riff_set_uuid.as_str(), play_position_in_beats);
                            }
                            else if let Some(riff_sequence_uuid) = state.playing_riff_sequence() {
                                if let Some(playing_riff_sequence_summary_data) = state.playing_riff_sequence_summary_data() {
                                    gui.repaint_riff_sequence_view_riff_sequence_active_drawing_areas(riff_sequence_uuid.as_str(), play_position_in_beats, playing_riff_sequence_summary_data);
                                }
                            }
                            else if let Some(riff_arrangement_uuid) = state.playing_riff_arrangement() {
                                if let Some(playing_riff_arrangement_summary_data) = state.playing_riff_arrangement_summary_data() {
                                    gui.repaint_riff_arrangement_view_riff_arrangement_active_drawing_areas(riff_arrangement_uuid.as_str(), play_position_in_beats, playing_riff_arrangement_summary_data);
                                }
                            }
                            // else {
                            //     // debug!("Not playing riff set");
                            // }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - play position - could not get lock on state"),
                    }

                    // should only repaint what is actively being viewed
                    gui.ui.piano_roll_drawing_area.queue_draw();
                    gui.ui.sample_roll_drawing_area.queue_draw();
                    gui.ui.track_drawing_area.queue_draw();
                    gui.ui.automation_drawing_area.queue_draw();
                }
                AudioLayerOutwardEvent::GeneralMMCEvent(mmc_sysex_bytes) => {
                    // debug!("Midi generic MMC event: ");
                    // let command_byte = mmc_sysex_bytes[4];
                    // match command_byte {
                    //     1 => {
                    //         match tx_from_ui.send(DAWEvents::TransportStop) {
                    //             Ok(_) => {}
                    //             Err(_) => {}
                    //         }
                    //     }
                    //     2 => {
                    //         match tx_from_ui.send(DAWEvents::TransportPlay) {
                    //             Ok(_) => {}
                    //             Err(_) => {}
                    //         }
                    //     }
                    //     4 => {
                    //         match tx_from_ui.send(DAWEvents::TransportMoveForward) {
                    //             Ok(_) => {}
                    //             Err(_) => {}
                    //         }
                    //     }
                    //     5 => {
                    //         match tx_from_ui.send(DAWEvents::TransportMoveBack) {
                    //             Ok(_) => {}
                    //             Err(_) => {}
                    //         }
                    //     }
                    //     6 => {
                    //         match state.lock() {
                    //             Ok(state) => {
                    //                 let recording = !state.recording();
                    //                 gui.ui.transport_record_button.set_active(recording);
                    //             },
                    //             Err(_) => debug!("Main - rx_ui processing loop - record - could not get lock on state"),
                    //         };
                    //     }
                    //     _ => {}
                    // }
                }
                AudioLayerOutwardEvent::MidiControlEvent(jack_midi_event) => {
                    match state.lock() {
                        Ok(mut state) => {
                            if jack_midi_event.data[0] as i32 == 144 && jack_midi_event.data[1] as usize >= 36_usize {
                                let riff_thing_index = jack_midi_event.data[1] as usize - 36_usize;
                                let track_riffs_stack_visible_name = gui.get_track_riffs_stack_visible_name();
                                if track_riffs_stack_visible_name == "Track Grid" {
                                    state.play_song(tx_to_audio.clone());
                                } else if track_riffs_stack_visible_name == "Riffs" {
                                    let riffs_stack_visible_name = gui.get_riffs_stack_visible_name();
                                    if riffs_stack_visible_name == "riff_sets" {
                                        let riff_set_uuid = if let Some(riff_set) = state.get_project().song_mut().riff_sets_mut().get_mut(riff_thing_index) {
                                            riff_set.uuid()
                                        } else {
                                            "".to_string()
                                        };
                                        state.play_riff_set(tx_to_audio.clone(), riff_set_uuid);
                                    } else if riffs_stack_visible_name == "riff_sequences" {
                                        let riff_sequence_uuid = if let Some(riff_sequence) = state.get_project().song_mut().riff_sequences_mut().get_mut(riff_thing_index) {
                                            riff_sequence.uuid()
                                        } else {
                                            "".to_string()
                                        };
                                        state.play_riff_sequence(tx_to_audio.clone(), riff_sequence_uuid);
                                    } else if riffs_stack_visible_name == "riff_arrangement" {
                                        let riff_arrangement_uuid = if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangements_mut().get_mut(riff_thing_index) {
                                            riff_arrangement.uuid()
                                        } else {
                                            "".to_string()
                                        };
                                        state.play_riff_arrangement(tx_to_audio.clone(), riff_arrangement_uuid, 0.0);
                                    }
                                }
                            } else if jack_midi_event.data[0] as i32 >= 176 && (jack_midi_event.data[0] as i32 <= (176 + 15)) {
                                // debug!("Main - rx_ui processing loop - jack AudioLayerOutwardEvent::MidiControlEvent - received a controller message: {} {} {}", jack_midi_event.data[0], jack_midi_event.data[1], jack_midi_event.data[2]);
                                // // need to send some track volume (176) or pan (177) messages
                                // let position_in_frames = jack_midi_event.delta_frames;
                                // let position_in_beats = (position_in_frames as f64) / state.project().song().sample_rate() * state.project().song().tempo() / 60.0;
                                // let track_index = jack_midi_event.data[1] as i32 - 1;
                                // let track_change_type = if jack_midi_event.data[0] as i32 == 176 {
                                //     TrackChangeType::Volume(Some(position_in_beats), jack_midi_event.data[2] as f32 / 127.0)
                                // } else {
                                //     TrackChangeType::Pan(Some(position_in_beats), (jack_midi_event.data[2] as f32 - 63.5) / 63.5)
                                // };

                                // if let Some(track) = state.project().song().tracks().get(track_index as usize) {
                                //     match tx_from_ui.send(DAWEvents::TrackChange(track_change_type, Some(track.uuid().to_string()))) {
                                //         Ok(_) => {}
                                //         Err(_) => {}
                                //     }
                                // }
                            } else {
                                debug!("Main - rx_ui processing loop - jack AudioLayerOutwardEvent::MidiControlEvent - received a unknown message: {} {} {}", jack_midi_event.data[0], jack_midi_event.data[1], jack_midi_event.data[2]);
                            }
                        }
                        Err(_) => {}
                    }
                }
                AudioLayerOutwardEvent::JackRestartRequired => {
                    match state.lock() {
                        Ok(mut state) => {
                            state.restart_jack(rx_to_audio.clone(), jack_midi_sender.clone(), jack_midi_sender_ui.clone(), jack_audio_coast.clone(), vst_host_time_info.clone());
                        }
                        Err(_) => {}
                    }
                }
                AudioLayerOutwardEvent::JackConnect(jack_port_from_name, jack_port_to_name) => {
                    match state.lock() {
                        Ok(mut state) => {
                            state.jack_connection_add(jack_port_from_name, jack_port_to_name);
                        }
                        Err(_) => {}
                    }
                }
                AudioLayerOutwardEvent::MasterChannelLevels(left_channel_level, right_channel_level) => {
                    if let Some(master_mixer_blade_widget) = gui.ui.mixer_box.children().first() {
                        if let Some(master_mixer_blade) = master_mixer_blade_widget.dynamic_cast_ref::<Frame>() {
                            if let Some(master_mixer_blade_box_widget) = master_mixer_blade.children().first() {
                                if let Some(master_mixer_blade_box) = master_mixer_blade_box_widget.dynamic_cast_ref::<gtk::Box>() {
                                        for child in master_mixer_blade_box.children().iter() {
                                            if child.widget_name() == "mixer_blade_volume_box" {
                                                if let Some(volume_box) = child.dynamic_cast_ref::<gtk::Box>() {
                                                    if let Some(channel_meter_box_widget) = volume_box.children().get(1) {
                                                        if let Some(channel_meter_box) = channel_meter_box_widget.dynamic_cast_ref::<gtk::Box>() {
                                                            if let Some(left_channel_spin_button_widget) = channel_meter_box.children().get_mut(1) {
                                                                if let Some(left_channel_spin_button) = left_channel_spin_button_widget.dynamic_cast_ref::<SpinButton>() {
                                                                    left_channel_spin_button.set_value((left_channel_level.abs().log10() * 20.0) as f64);
                                                                }
                                                            }
                                                            if let Some(right_channel_spin_button_widget) = channel_meter_box.children().get_mut(2) {
                                                                if let Some(right_channel_spin_button) = right_channel_spin_button_widget.dynamic_cast_ref::<SpinButton>() {
                                                                    right_channel_spin_button.set_value((right_channel_level.abs().log10() * 20.0) as f64);
                                                                }
                                                            }
                                                            if let Some(channel_meter_levels_drawing_area_widget) = channel_meter_box.children().get_mut(0) {
                                                                if let Some(channel_meter_levels_drawing_area) = channel_meter_levels_drawing_area_widget.dynamic_cast_ref::<DrawingArea>() {
                                                                    channel_meter_levels_drawing_area.queue_draw();
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                                break;
                                            }
                                        }
                                }
                            }
                        }
                    }
                },
            }
        },
        Err(_) => (),
    }
}

fn process_track_background_processor_events(
    vst_audio_plugin_windows: &mut HashMap<String, Window>,
    state: &mut Arc<Mutex<DAWState>>,
    gui: &mut MainWindow,
) {
    match state.lock() {
        Ok(mut state) => {
            let mut track_to_plugins_to_plugin_params_map = HashMap::new();
            let mut track_render_audio_consumers = HashMap::new();
            let mut track_instrument_names = HashMap::new();
            let mut automation_event = None;
            let mut automation_track_uuid = "".to_string();
            state.instrument_track_receivers().iter().for_each(|(track_uuid, receiver)| {
                let mut plugins_to_plugin_params_map = HashMap::new();
                match receiver.try_recv() {
                    Ok(event) => match event {
                        TrackBackgroundProcessorOutwardEvent::InstrumentParameters(instrument_parameters) => {
                            let mut parameter_details = vec![];
                            let mut plugin_uuid = String::new();
                            debug!("Received instrument plugin parameter details.");
                            instrument_parameters.iter().for_each(|(param_index, _track_uuid_orig, plugin_uuid_orig, param_name, param_label, _param_value, param_text)| {
                                // debug!("Received plugin parameter details for: track uuid={}, plugin uuid={},  param - index={},  param - name={}, label={}, value={}, text={}",
                                // track_uuid_orig, plugin_uuid_orig.clone(), param_index, param_name, param_label, param_value, param_text);
                                plugin_uuid.clear();
                                plugin_uuid.push_str(plugin_uuid_orig.to_string().as_str()); // plugin uuid
                                parameter_details.push(PluginParameterDetail {
                                    index: *param_index,
                                    name: param_name.clone(),
                                    label: param_label.clone(),
                                    text: param_text.clone(),
                                });
                            });
                            plugins_to_plugin_params_map.insert(plugin_uuid, parameter_details);
                        }
                        TrackBackgroundProcessorOutwardEvent::InstrumentName(name) => {
                            debug!("Main loop - received instrument name.");
                            track_instrument_names.insert(track_uuid.clone(), name);
                        }
                        TrackBackgroundProcessorOutwardEvent::EffectParameters(effect_params) => {
                            debug!("Received effects parameters: {:?}", effect_params);
                            let mut parameter_details = vec![];
                            let mut plugin_uuid = String::new();
                            effect_params.iter().for_each(|(plugin_id, param_index, param_name, param_label, _param_value, param_text)| {
                                plugin_uuid.clear();
                                plugin_uuid.push_str(plugin_id.as_str()); // plugin uuid
                                parameter_details.push(PluginParameterDetail {
                                    index: *param_index,
                                    name: param_name.clone(),
                                    label: param_label.clone(),
                                    text: param_text.clone(),
                                });
                            });
                            plugins_to_plugin_params_map.insert(plugin_uuid, parameter_details);
                        },
                        TrackBackgroundProcessorOutwardEvent::GetPresetData(_, _) => (),
                        TrackBackgroundProcessorOutwardEvent::InstrumentPluginWindowSize(track_uuid, plugin_window_width, plugin_window_height) => {
                            state.project().song().tracks().iter().for_each(|track_type| {
                                match track_type {
                                    TrackType::InstrumentTrack(track) => if track.uuid().to_string() == track_uuid {
                                        if let Some(window) = vst_audio_plugin_windows.get(&track.instrument().uuid().to_string()) {
                                            debug!("Instrument plugin window resize requested: width={}, height={}", plugin_window_width, plugin_window_height);
                                            window.resize(plugin_window_width, plugin_window_height);
                                        }
                                    },
                                    TrackType::AudioTrack(_) => (),
                                    TrackType::MidiTrack(_) => (),
                                }
                            });
                        },
                        TrackBackgroundProcessorOutwardEvent::Automation(track_uuid, plugin_uuid, is_instrument, param_index, param_value) => {
                            automation_track_uuid.push_str(track_uuid.as_str());
                            let play_position_in_beats = state.play_position_in_frames() as f64 / state.project().song().sample_rate() * state.project().song().tempo() / 60.0;
                            automation_event = Some(TrackEvent::AudioPluginParameter(PluginParameter {
                                id: Uuid::new_v4(),
                                index: param_index,
                                position: play_position_in_beats,
                                value: param_value,
                                instrument: is_instrument,
                                plugin_uuid: Uuid::parse_str(plugin_uuid.as_str()).unwrap(),
                            }));
                        },
                        TrackBackgroundProcessorOutwardEvent::EffectPluginWindowSize(track_uuid, plugin_uuid, plugin_window_width, plugin_window_height) => {
                            debug!("Effect plugin requested window resize: width={}, height={}", plugin_window_width, plugin_window_height);
                            state.project().song().tracks().iter().for_each(|track_type| {
                                match track_type {
                                    TrackType::InstrumentTrack(track) => if track.uuid().to_string() == track_uuid {
                                        for effect in track.effects() {
                                            if effect.uuid().to_string() == plugin_uuid {
                                                if let Some(window) = vst_audio_plugin_windows.get(&plugin_uuid) {
                                                    window.resize(plugin_window_width, plugin_window_height);
                                                    window.set_height_request(plugin_window_height);
                                                    window.set_width_request(plugin_window_width);
                                                    window.queue_resize();
                                                }
                                                break;
                                            }
                                        }
                                    },
                                    TrackType::AudioTrack(_) => (),
                                    TrackType::MidiTrack(_) => (),
                                }
                            });
                        },
                        TrackBackgroundProcessorOutwardEvent::TrackRenderAudioConsumer(track_render_audio_consumer) => {
                            track_render_audio_consumers.insert(track_render_audio_consumer.track_id().to_string(), track_render_audio_consumer);
                        }
                        TrackBackgroundProcessorOutwardEvent::ChannelLevels(track_uuid, left_channel_level, right_channel_level) => {
                            // debug!("Track: {}, left: {}, left in db: {}, right: {}, right in db: {}", track_uuid.as_str(), left_channel_level, left_channel_level.abs().log10() * 20.0, right_channel_level, right_channel_level.abs().log10() * 20.0);
                            for mixer_blade_widget in gui.ui.mixer_box.children().iter() {
                                if mixer_blade_widget.widget_name() == track_uuid.as_str() {
                                    if let Some(mixer_blade) = mixer_blade_widget.dynamic_cast_ref::<Frame>() {
                                        if let Some(mixer_blade_box_widget) = mixer_blade.children().first() {
                                            if let Some(mixer_blade_box) = mixer_blade_box_widget.dynamic_cast_ref::<gtk::Box>() {
                                                    for child in mixer_blade_box.children().iter() {
                                                        if child.widget_name() == "mixer_blade_volume_box" {
                                                            if let Some(volume_box) = child.dynamic_cast_ref::<gtk::Box>() {
                                                                if let Some(channel_meter_box_widget) = volume_box.children().get(1) {
                                                                    if let Some(channel_meter_box) = channel_meter_box_widget.dynamic_cast_ref::<gtk::Box>() {
                                                                        if let Some(left_channel_spin_button_widget) = channel_meter_box.children().get_mut(1) {
                                                                            if let Some(left_channel_spin_button) = left_channel_spin_button_widget.dynamic_cast_ref::<SpinButton>() {
                                                                                left_channel_spin_button.set_value((left_channel_level.abs().log10() * 20.0) as f64);
                                                                            }
                                                                        }
                                                                        if let Some(right_channel_spin_button_widget) = channel_meter_box.children().get_mut(2) {
                                                                            if let Some(right_channel_spin_button) = right_channel_spin_button_widget.dynamic_cast_ref::<SpinButton>() {
                                                                                right_channel_spin_button.set_value((right_channel_level.abs().log10() * 20.0) as f64);
                                                                            }
                                                                        }
                                                                        if let Some(channel_meter_levels_drawing_area_widget) = channel_meter_box.children().get_mut(0) {
                                                                            if let Some(channel_meter_levels_drawing_area) = channel_meter_levels_drawing_area_widget.dynamic_cast_ref::<DrawingArea>() {
                                                                                channel_meter_levels_drawing_area.queue_draw();
                                                                            }
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                            break;
                                                        }
                                                    }
                                            }
                                        }
                                    }
                                    break;
                                }
                            }
                        },
                    },
                    Err(_) => (),
                }

                if plugins_to_plugin_params_map.keys().count() > 0 {
                    track_to_plugins_to_plugin_params_map.insert(track_uuid.clone(), plugins_to_plugin_params_map);
                }
            });
            let state = &mut state;
            track_to_plugins_to_plugin_params_map.iter_mut().for_each(|(track_uuid, plugins_to_plugin_params_map)| {
                let mut plugins_to_plugin_params_map_copy = HashMap::new();
                plugins_to_plugin_params_map.iter().for_each(|(plugin_uuid, plugin_params_orig)| {
                    let mut plugin_params_copy = vec![];
                    plugin_params_orig.iter().for_each(|param| {
                        plugin_params_copy.push(param.clone());
                    });
                    plugins_to_plugin_params_map_copy.insert(String::from(plugin_uuid), plugin_params_copy);
                });

                match state.audio_plugin_parameters_mut().get_mut(track_uuid.as_str()) {
                    Some(audio_plugin_parameters) => {
                        // need to merge
                        for (key, value) in plugins_to_plugin_params_map_copy {
                            audio_plugin_parameters.insert(key, value);
                        }
                    }
                    None => {
                        state.audio_plugin_parameters_mut().insert(String::from(track_uuid.as_str()), plugins_to_plugin_params_map_copy);
                    },
                }
            });

            let track_render_audio_consumer_uuids: Vec<String> = track_render_audio_consumers.iter().map(|(track_uuid, _)| track_uuid.to_string()).collect();
            for track_uuid in track_render_audio_consumer_uuids.iter() {
                if let Some(track_render_audio_consumer) = track_render_audio_consumers.remove(&track_uuid.clone()) {
                    debug!("Received track_render_audio_consumer for track: {}", track_uuid.clone());
                    if let Ok(mut track_render_audio_consumers) = state.track_render_audio_consumers_mut().lock() {
                        track_render_audio_consumers.insert(track_uuid.clone(), track_render_audio_consumer);
                    }
                }
            }

            // find the track
            state.get_project().song_mut().tracks_mut().iter_mut().for_each(|track_type| {
                match track_type {
                    TrackType::InstrumentTrack(track) => {
                        if automation_track_uuid == track.uuid().to_string() {
                            if let Some(event) = automation_event.clone() {
                                track.automation_mut().events_mut().push(event);
                            }
                        }
                    }
                    TrackType::AudioTrack(track) => {
                        if automation_track_uuid == track.uuid().to_string() {
                            if let Some(event) = automation_event.clone() {
                                track.automation_mut().events_mut().push(event);
                            }
                        }
                    }
                    _ => {}
                }
            });

            // change the track instrument name
            for (track_uuid, name) in track_instrument_names.iter() {
                debug!("Trying to change instrument name: track={}, instrument name={}", track_uuid.clone(), name);
                if let Some(track_type) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid.clone()) {
                    match track_type {
                        TrackType::InstrumentTrack(track) => {
                            track.instrument_mut().set_name(name.clone());
                        }
                        _ => {}
                    }
                }
            }
        },
        Err(_) => (),
    }
}
