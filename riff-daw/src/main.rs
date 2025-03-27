use std::{collections::HashMap, default::Default, sync::{Arc, Mutex}, time::Duration};
use std::cell::RefCell;
use std::thread;

use apres::MIDI;
use constants::{TRACK_VIEW_TRACK_PANEL_HEIGHT, LUA_GLOBAL_STATE, VST_PATH_ENVIRONMENT_VARIABLE_NAME, CLAP_PATH_ENVIRONMENT_VARIABLE_NAME, DAW_AUTO_SAVE_THREAD_NAME};
use crossbeam_channel::{Receiver, Sender, unbounded};
use flexi_logger::{LogSpecification, Logger};
use gtk::{Adjustment, ButtonsType, ComboBoxText, DrawingArea, Frame, glib, MessageDialog, MessageType, prelude::{ActionMapExt, AdjustmentExt, ApplicationExt, Cast, ComboBoxExtManual, ComboBoxTextExt, ContainerExt, DialogExt, EntryExt, GtkWindowExt, LabelExt, ProgressBarExt, ScrolledWindowExt, SpinButtonExt, TextBufferExt, TextViewExt, ToggleToolButtonExt, WidgetExt}, SpinButton, Window, WindowType, Viewport};
use gtk::prelude::BinExt;
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
use crate::constants::{EVENT_DELETION_BEAT_TOLERANCE, VST3_PATH_ENVIRONMENT_VARIABLE_NAME};
use crate::utils::CalculatedSnap;
use crate::vst3_cxx_bridge::ffi;

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
mod vst3_cxx_bridge;

#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

extern {
    fn gdk_x11_window_get_xid(window: gdk::Window) -> u32;
}

thread_local!(static THREAD_POOL: RefCell<rayon::ThreadPool> = RefCell::new(
    rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .thread_name(|index: usize| format!("daw_evt_thrd-{}", index))
        .build()
        .unwrap()));


fn main() {
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

    let (tx_from_ui, rx_from_ui) = unbounded::<DAWEvents>();
    let (tx_to_audio, rx_to_audio) = unbounded::<AudioLayerInwardEvent>();
    let (jack_midi_sender_ui, jack_midi_receiver_ui) = unbounded::<AudioLayerOutwardEvent>();
    let (jack_midi_sender, jack_midi_receiver) = unbounded::<AudioLayerOutwardEvent>();
    let (jack_time_critical_midi_sender, jack_time_critical_midi_receiver) = unbounded::<AudioLayerTimeCriticalOutwardEvent>();

    let state = {
        let tx_from_ui = tx_from_ui.clone();
        Arc::new(Mutex::new (DAWState::new(tx_from_ui)))
    };

    let sample_rate = if let Ok(state) = state.lock() {
        // transport
        let transport = Transport {
            playing: false,
            bpm: 140.0,
            sample_rate: state.configuration.audio.sample_rate as f64,
            block_size: state.configuration.audio.block_size as f64,
            position_in_beats: 0.0,
            position_in_frames: 0,
        };
        TRANSPORT.set(RwLock::new(transport));

        state.configuration.audio.sample_rate as f64
    } else { 44100.0 };


    // VST2 timing
    let vst_host_time_info = Arc::new(RwLock::new(TimeInfo {
        sample_pos: 0.0,
        sample_rate,
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

    let mut audio_plugin_windows: HashMap<String, Window> = HashMap::new();

    let lua = Lua::new();
    let _ = lua.globals().set(LUA_GLOBAL_STATE, LuaState {state: state.clone(), tx_from_ui: tx_from_ui.clone()});

    gtk::init().expect("Problem starting up GTK3.");

    let mut gui = {
        let tx_from_ui = tx_from_ui.clone();
        let state = state.clone();
        MainWindow::new(tx_from_ui, tx_to_audio.clone(), state)
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
    scan_audio_plugins(state.clone(), &gui);

    start_autosave(state.clone(), autosave_keep_alive.clone());

    // handle incoming events in the gui thread - lots of ui interaction
    {
        let mut state = state.clone();
        let mut delay_count = 0;
        let mut progress_bar_pulse_delay_count = 0;
        let rx_to_audio = rx_to_audio.clone();
        let jack_midi_sender = jack_midi_sender.clone();
        let jack_midi_sender_ui = jack_midi_sender_ui.clone();
        let jack_time_critical_midi_sender = jack_time_critical_midi_sender.clone();
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
                &jack_time_critical_midi_sender,
                &track_audio_coast,
                &mut gui,
                &vst_host_time_info,
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
                    &rx_to_audio,
                    &jack_midi_sender,
                    &jack_midi_sender_ui,
                    &jack_time_critical_midi_sender,
                    &track_audio_coast,
                    vst_host_time_info.clone(),
                );
            }
            else {
                delay_count += 1;
            }

            glib::Continue(true)
        });
    }

    create_jack_time_critical_event_processing_thread(
        tx_from_ui.clone(),
        jack_time_critical_midi_receiver.clone(),
        state.clone()
    );

    // kick off the audio layer
    {
        let rx_to_audio = rx_to_audio;
        let jack_midi_sender = jack_midi_sender.clone();
        let jack_midi_sender_ui = jack_midi_sender_ui;
        let jack_time_critical_midi_sender = jack_time_critical_midi_sender.clone();
        let jack_audio_coast = jack_audio_coast;

        match state.lock() {
            Ok(mut state) => {
                state.start_jack(rx_to_audio, jack_midi_sender, jack_midi_sender_ui, jack_time_critical_midi_sender, jack_audio_coast, vst_host_time_info);
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

pub fn start_autosave(state: Arc<Mutex<DAWState>>, autosave_keep_alive: Arc<Mutex<bool>>)     {
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


pub fn scan_audio_plugins(state: Arc<Mutex<DAWState>>, gui: &MainWindow)     {
    if let Ok(vst_path) = std::env::var(VST_PATH_ENVIRONMENT_VARIABLE_NAME) {
        if let Ok(clap_path) = std::env::var(CLAP_PATH_ENVIRONMENT_VARIABLE_NAME) {
            if let Ok(vst3_path) = std::env::var(VST3_PATH_ENVIRONMENT_VARIABLE_NAME) {
                match state.lock() {
                    Ok(mut state) => {
                        if state.configuration.scanned_instrument_plugins.successfully_scanned.is_empty() && state.configuration.scanned_effect_plugins.successfully_scanned.is_empty() {
                            let (instruments, effects) = scan_for_audio_plugins(vst_path.clone(), clap_path.clone(), vst3_path.clone());
                            for (key, value) in instruments.iter() {
                                state.instrument_plugins_mut().insert(key.to_string(), value.to_string());
                                state.configuration.scanned_instrument_plugins.successfully_scanned.insert(key.to_string(), value.to_string());
                            }
                            state.instrument_plugins_mut().sort_by(|_key1, value1: &String, _key2, value2: &String| value1.cmp(value2));

                            for (key, value) in effects.iter() {
                                state.effect_plugins_mut().insert(key.to_string(), value.to_string());
                                state.configuration.scanned_effect_plugins.successfully_scanned.insert(key.to_string(), value.to_string());
                            }
                            state.effect_plugins_mut().sort_by(|_key1, value1: &String, _key2, value2: &String| value1.cmp(value2));

                            state.configuration.save();
                        } else {
                            let mut intermediate_map = HashMap::new();
                            for (key, value) in state.configuration.scanned_instrument_plugins.successfully_scanned.iter() {
                                intermediate_map.insert(key.to_string(), value.to_string());
                            }
                            for (key, value) in intermediate_map.iter() {
                                state.instrument_plugins_mut().insert(key.to_string(), value.to_string());
                            }
                            state.instrument_plugins_mut().sort_by(|_key1, value1: &String, _key2, value2: &String| value1.cmp(value2));

                            intermediate_map.clear();
                            for (key, value) in state.configuration.scanned_effect_plugins.successfully_scanned.iter() {
                                intermediate_map.insert(key.to_string(), value.to_string());
                            }
                            for (key, value) in intermediate_map.iter() {
                                state.effect_plugins_mut().insert(key.to_string(), value.to_string());
                            }
                            state.effect_plugins_mut().sort_by(|_key1, value1: &String, _key2, value2: &String| value1.cmp(value2));
                        }

                        gui.update_available_audio_plugins_in_ui(state.instrument_plugins(), state.effect_plugins());
                    }
                    Err(_) => {}
                }
            }
        }
    }
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
                                vst_host_time_info: Arc<RwLock<TimeInfo>>,
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
                              rx_to_audio: &Receiver<AudioLayerInwardEvent>,
                              jack_midi_sender: &Sender<AudioLayerOutwardEvent>,
                              jack_midi_sender_ui: &Sender<AudioLayerOutwardEvent>,
                              jack_time_critical_midi_sender: &Sender<AudioLayerTimeCriticalOutwardEvent>,
                              jack_audio_coast: &Arc<Mutex<TrackBackgroundProcessorMode>>,
                              vst_host_time_info: Arc<RwLock<TimeInfo>>,
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
                        state.close_all_tracks(tx_to_audio.clone());
                        state.reset_state();

                        let mut project = Project::new();

                        project.song_mut().set_tempo(gui.ui.song_tempo_spinner.value());

                        {
                            let mut time_info =  vst_host_time_info.write();
                            time_info.sample_pos = 0.0;
                            time_info.sample_rate = state.configuration.audio.sample_rate as f64;
                            time_info.nanoseconds = 0.0;
                            time_info.ppq_pos = 0.0;
                            time_info.tempo = project.song().tempo();
                            time_info.bar_start_pos = 0.0;
                            time_info.cycle_start_pos = 0.0;
                            time_info.cycle_end_pos = 0.0;
                            time_info.time_sig_numerator = project.song().time_signature_numerator() as i32;
                            time_info.time_sig_denominator = project.song().time_signature_denominator() as i32;
                            time_info.smpte_offset = 0;
                            time_info.smpte_frame_rate = vst::api::SmpteFrameRate::Smpte24fps;
                            time_info.samples_to_next_clock = 0;
                            time_info.flags = 3;
                        }

                        state.set_project(project);
                        state.set_current_file_path(None);
                        let mut instrument_track_senders2 = HashMap::new();
                        let mut instrument_track_receivers2 = HashMap::new();
                        let mut sample_references = HashMap::new();
                        let mut samples_data = HashMap::new();
                        let sample_rate = state.configuration.audio.sample_rate as f64;
                        let block_size = state.configuration.audio.block_size as f64;
                        let tempo = state.project().song().tempo();
                        let time_signature_numerator = state.project().song().time_signature_numerator();
                        let time_signature_denominator = state.project().song().time_signature_denominator();
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
                                sample_rate,
                                block_size,
                                tempo,
                                time_signature_numerator as i32,
                                time_signature_denominator as i32,
                            );
                        }
                        state.update_track_senders_and_receivers(instrument_track_senders2, instrument_track_receivers2);

                        gui.update_ui_from_state(tx_from_ui, &mut state, state_arc);
                        match tx_to_audio.send(AudioLayerInwardEvent::BlockSize(state.configuration.audio.block_size as f64)) {
                            Ok(_) => (),
                            Err(error) => debug!("Problem using tx_to_audio to send block size message to jack layer: {}", error),
                        }
                        match tx_to_audio.send(AudioLayerInwardEvent::Tempo(state.project().song().tempo())) {
                            Ok(_) => (),
                            Err(error) => debug!("Problem using tx_to_audio to send tempo message to jack layer: {}", error),
                        }
                        match tx_to_audio.send(AudioLayerInwardEvent::SampleRate(state.configuration.audio.sample_rate as f64)) {
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
                THREAD_POOL.with_borrow(|thread_pool| thread_pool.spawn(move || {
                    if let Ok(mut coast) = track_audio_coast.lock() {
                        *coast = TrackBackgroundProcessorMode::Coast;
                    }
                    thread::sleep(Duration::from_millis(1000));
                    // history.clear();
                    let mut midi_tracks = HashMap::new();
                    let state_arc2 = state.clone();
                    match state.lock() {
                        Ok(mut state) => {
                            state.close_all_tracks(tx_to_audio.clone());
                            state.reset_state();

                            state.load_from_file(
                                vst24_plugin_loaders.clone(), clap_plugin_loaders.clone(), path.to_str().unwrap(), tx_to_audio.clone(), track_audio_coast.clone(), vst_host_time_info.clone());

                            let tempo = state.project().song().tempo();

                            {
                                let mut time_info = vst_host_time_info.write();
                                time_info.sample_pos = 0.0;
                                time_info.sample_rate = state.configuration.audio.sample_rate as f64;; // FIXME is sample rate and block size part of a song or should it be part of configuration???
                                time_info.nanoseconds = 0.0;
                                time_info.ppq_pos = 0.0;
                                time_info.tempo = tempo;
                                time_info.bar_start_pos = 0.0;
                                time_info.cycle_start_pos = 0.0;
                                time_info.cycle_end_pos = 0.0;
                                time_info.time_sig_numerator = state.project().song().time_signature_numerator() as i32;
                                time_info.time_sig_denominator = state.project().song().time_signature_denominator() as i32;
                                time_info.smpte_offset = 0;
                                time_info.smpte_frame_rate = vst::api::SmpteFrameRate::Smpte24fps;
                                time_info.samples_to_next_clock = 0;
                                time_info.flags = 3;
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

                            match tx_to_audio.send(AudioLayerInwardEvent::BlockSize(state.configuration.audio.block_size as f64)) {
                                Ok(_) => (),
                                Err(error) => debug!("Problem using tx_to_audio to send block size message to jack layer: {}", error),
                            }
                            match tx_to_audio.send(AudioLayerInwardEvent::Tempo(state.project().song().tempo())) {
                                Ok(_) => (),
                                Err(error) => debug!("Problem using tx_to_audio to send block size message to jack layer: {}", error),
                            }
                            match tx_to_audio.send(AudioLayerInwardEvent::SampleRate(state.configuration.audio.sample_rate as f64)) {
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
                }));
            },
            DAWEvents::Save => {
                gui.ui.dialogue_progress_bar.set_text(Some("Saving..."));
                gui.ui.progress_dialogue.set_title("Save");
                gui.ui.progress_dialogue.show_all();

                {
                    let state = state.clone();
                    let track_audio_coast = track_audio_coast;
                    let tx_from_ui = tx_from_ui;
                    let _ = THREAD_POOL.with_borrow(|thread_pool| thread_pool.spawn(move || {
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
                    }));
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
                    let _ = THREAD_POOL.with_borrow(|thread_pool| thread_pool.spawn(move || {
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
                    }));
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
                    let _ = THREAD_POOL.with_borrow(|thread_pool| thread_pool.spawn(move || {
                        if let Ok(mut coast) = track_audio_coast.lock() {
                            *coast = TrackBackgroundProcessorMode::Coast;
                        }
                        match state.lock() {
                            Ok(mut state) => {
                                let sample_rate = state.configuration.audio.sample_rate as f64;;
                                let block_size = state.configuration.audio.block_size as f64;;
                                let time_signature_numerator = state.project().song().time_signature_numerator();
                                let time_signature_denominator = state.project().song().time_signature_denominator();
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
                                                                        let new_note = Note::new_with_params(
                                                                            MidiPolyphonicExpressionNoteId::ALL as i32, position_in_beats, note as i32, velocity as i32, 0.0);
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
                                                        sample_rate,
                                                        block_size,
                                                        tempo as f64,
                                                        time_signature_numerator as i32,
                                                        time_signature_denominator as i32,
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
                    }));
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
                    let _ = THREAD_POOL.with_borrow(|thread_pool| thread_pool.spawn(move || {
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
                    }));
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
                    let _ = THREAD_POOL.with_borrow(|thread_pool| thread_pool.spawn(move || {
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
                    }));
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
                    let _ = THREAD_POOL.with_borrow(|thread_pool| thread_pool.spawn(move || {
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
                    }));
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
                    ShowType::PitchBend => AutomationViewMode::PitchBend,
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
                                let sample_rate = state.configuration.audio.sample_rate as f64;
                                let block_size = state.configuration.audio.block_size as f64;
                                let song = state.project().song();
                                let tracks = song.tracks();

                                match state.active_loop() {
                                    Some(active_loop_uuid) => {
                                        match song.loops().iter().find(|current_loop| current_loop.uuid().to_string() == active_loop_uuid.to_string()) {
                                            Some(active_loop) => {
                                                start_block = (active_loop.start_position() * sample_rate / block_size) as i32;
                                                end_block = (active_loop.end_position() * sample_rate / block_size) as i32;
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
                                let sample_rate = state.configuration.audio.sample_rate as f64;
                                let block_size = state.configuration.audio.block_size as f64;
                                let song = state.project().song();
                                let tracks = song.tracks();

                                match uuid {
                                    Some(active_loop_uuid) => {
                                        match song.loops().iter().find(|current_loop| current_loop.uuid().to_string() == active_loop_uuid.to_string()) {
                                            Some(active_loop) => {
                                                start_block = (active_loop.start_position() * sample_rate / block_size) as i32;
                                                end_block = (active_loop.end_position() * sample_rate / block_size) as i32;
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
                                let sample_rate = state.configuration.audio.sample_rate as f64;
                                let block_size = state.configuration.audio.block_size as f64;

                                match state.active_loop() {
                                    Some(active_loop_uuid) => {
                                        let song = state.project().song();
                                        let tracks = song.tracks();
                                        match song.loops().iter().find(|current_loop| current_loop.uuid().to_string() == active_loop_uuid.to_string()) {
                                            Some(active_loop) => {
                                                let start_block = (start_position * sample_rate / block_size) as i32;
                                                let end_block = (active_loop.end_position() * sample_rate / block_size) as i32;
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
                                let sample_rate = state.configuration.audio.sample_rate as f64;
                                let block_size = state.configuration.audio.block_size as f64;

                                match state.active_loop() {
                                    Some(active_loop_uuid) => {
                                        let song = state.project().song();
                                        let tracks = song.tracks();
                                        match song.loops().iter().find(|current_loop| current_loop.uuid().to_string() == active_loop_uuid.to_string()) {
                                            Some(active_loop) => {
                                                let start_block = (active_loop.start_position() * sample_rate / block_size) as i32;
                                                let end_block = (end_position * sample_rate / block_size) as i32;
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
            DAWEvents::PianoRollSetTrackName(name) => {
                gui.set_piano_roll_selected_track_name_label(name.as_str());
                gui.ui.piano_roll_drawing_area.queue_draw();
            }
            DAWEvents::PianoRollSetRiffName(name) => {
                gui.set_piano_roll_selected_riff_name_label(name.as_str());
                gui.ui.piano_roll_drawing_area.queue_draw();
            }
            DAWEvents::PianoRollMPENoteIdChange(mpe_note_id_change) => {
                match state.lock() {
                    Ok(mut state) => {
                        state.set_piano_roll_mpe_note_id(mpe_note_id_change);
                        gui.ui.piano_roll_drawing_area.queue_draw();
                    }
                    Err(_) => debug!("Main - rx_ui processing loop - PianoRollMPENoteIdChange - could not get lock on state"),
                }
            }
            DAWEvents::PianoRollWindowedZoom{x1, y1, x2, y2} => { // values are in pixels
                if let Some(widget) = gui.ui.piano_roll_scrolled_window.child() {
                    if let Some(view_port) = widget.dynamic_cast_ref::<Viewport>() {
                        let width = view_port.allocated_width();
                        let height = view_port.allocated_height();
                        let window_width = x2 - x1;
                        let window_height = y2 - y1;
                        let horizontal_scale_up = width as f64 / window_width;
                        let vertical_scale_up = height as f64 / window_height;

                        if let Some(grid_arc) = gui.piano_roll_grid.clone() {
                            if let Ok(mut grid) = grid_arc.lock() {
                                let zoom_horizontal = grid.zoom_horizontal();
                                let zoom_vertical = grid.zoom_vertical();
                                let adjusted_horizontal_zoom = zoom_horizontal * horizontal_scale_up;
                                let adjusted_vertical_zoom = zoom_vertical * vertical_scale_up;

                                grid.set_horizontal_zoom(zoom_horizontal * horizontal_scale_up);
                                grid.set_vertical_zoom(zoom_vertical * vertical_scale_up);


                                // need to adjust the gtk scale widget adjustments (ranges) - probably should do this rather than setting the zoom directly


                                // need to scroll the zoom window into view


                            }
                        }
                    }
                }
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
                            let sample_rate = state.configuration.audio.sample_rate as f64;
                            let block_size = state.configuration.audio.block_size as f64;;
                            let tempo = state.project().song().tempo();
                            let time_signature_numerator = state.project().song().time_signature_numerator();
                            let time_signature_denominator = state.project().song().time_signature_denominator();

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
                                            sample_rate,
                                            block_size,
                                            tempo,
                                            time_signature_numerator as i32,
                                            time_signature_denominator as i32,
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
                                            sample_rate,
                                            block_size,
                                            tempo,
                                            time_signature_numerator as i32,
                                            time_signature_denominator as i32,
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
                                            sample_rate,
                                            block_size,
                                            tempo,
                                            time_signature_numerator as i32,
                                            time_signature_denominator as i32,
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
                            gui.update_available_audio_plugins_in_ui(state.instrument_plugins(), state.effect_plugins());
                        }
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
                                    state.get_project().song_mut().delete_track(track_uuid.clone());
                                    if let Err(error) = tx_to_audio.send(AudioLayerInwardEvent::RemoveTrack(track_uuid.clone())) {
                                        debug!("Main - rx_ui processing loop - Track Deleted - could send delete track to audio layer: {}", error);
                                    }
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
                                    for _ in 0..daw_events_to_propagate.len() {
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
                TrackChangeType::RiffAddWithTrackIndex(uuid, length, track_index) => {
                    debug!("Main - rx_ui processing loop - TrackChangeType::RiffAddWithTrackIndex");

                    // display a dialogue and prompt to get the riff name
                    let mut name = "".to_string();
                    while name.is_empty() {
                        if gui.ui.riff_name_dialogue.run() == gtk::ResponseType::Ok && gui.ui.riff_name_entry.text().len() > 0 {
                            name = gui.ui.riff_name_entry.text().to_string();
                            gui.ui.riff_name_entry.set_text("");
                        }
                    }
                    gui.ui.riff_name_dialogue.hide();

                    // get the track id
                    let track_id = if let Ok(state) = state.lock() {
                        if let Some(track) = state.project().song().tracks().get(track_index as usize) {
                            Some(track.uuid().to_string())
                        }
                        else { None }
                    }
                    else { None };

                    let mut state_arc = state.clone();
                    let mut state = state.clone();
                    match history_manager.lock() {
                        Ok(mut history) => {
                            let action = RiffAdd::new_with_track_id(Uuid::parse_str(uuid.clone().as_str()).unwrap(), name, length, &mut state.clone(), track_id.clone());
                            match history.apply(&mut state, Box::new(action)) {
                                Ok(mut daw_events_to_propagate) => {
                                    // set the selected riff
                                    if let Ok(mut state) = state_arc.lock() {
                                        if let Some(track_id) = track_id {
                                            state.set_selected_riff_uuid(track_id, uuid);
                                        }
                                    }

                                    // TODO need to refresh the track details dialogue

                                    // refresh UI
                                    gui.ui.track_drawing_area.queue_draw();
                                    gui.ui.piano_roll_drawing_area.queue_draw();
                                    gui.ui.automation_drawing_area.queue_draw();
                                    for _ in 0..daw_events_to_propagate.len() {
                                        let event = daw_events_to_propagate.remove(0);
                                        let _ = tx_from_ui.send(event);
                                    }
                                }
                                Err(error) => {
                                    error!("Main - rx_ui processing loop - TrackChangeType::RiffAddWithTrackIndex - error: {}", error);
                                }
                            }
                        }
                        Err(error) => {
                            error!("Main - rx_ui processing loop - TrackChangeType::RiffAddWithTrackIndex - error getting lock for history manager: {}", error);
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

                            // check riff grids
                            if let Some(uuid) = track_uuid.clone() {
                                for riff_grid in state.project().song().riff_grids().iter() {
                                    if let Some(riff_references) = riff_grid.track_riff_references(uuid.clone()) {
                                        for riff_reference in riff_references.iter() {
                                            if riff_reference.linked_to() == riff_uuid {
                                                let message = format!("Riff grid: \"{}\" has references to riff: \"{}\".", riff_grid.name(), riff_name.as_str());

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
                                    match *(riff_item.item_type()) {
                                        RiffItemType::RiffSet => {
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
                                        RiffItemType::RiffSequence => {
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
                                        RiffItemType::RiffGrid => {
                                            if let Some(uuid) = track_uuid.clone() {
                                                if let Some(riff_grid) = state.project().song().riff_grid(riff_item.uuid()) {
                                                    if let Some(riff_references) = riff_grid.track_riff_references(uuid.clone()) {
                                                        for riff_reference in riff_references.iter() {
                                                            if riff_reference.linked_to() == riff_uuid {
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
                                        for _ in 0..daw_events_to_propagate.len() {
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
                    } else {
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
                TrackChangeType::RiffAddNote(new_notes) => {
                    {
                        let note_id = if let Ok(state) = state.lock() {
                            state.piano_roll_mpe_note_id().clone() as i32
                        } else {
                            MidiPolyphonicExpressionNoteId::ALL as i32
                        };
                        let mut state = state.clone();
                        match history_manager.lock() {
                            Ok(mut history) => {
                                for (note, position, duration) in new_notes.iter() {
                                    let action = RiffAddNoteAction::new(note_id, *position, *note, 127, *duration, &mut state.clone());
                                    if let Err(error) = history.apply(&mut state, Box::new(action)) {
                                        error!("Main - rx_ui processing loop - riff add note - error: {}", error);
                                    } else {
                                        // refresh UI
                                        gui.ui.track_drawing_area.queue_draw();
                                        gui.ui.piano_roll_drawing_area.queue_draw();
                                    }
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
                                            for (note, _position, _duration) in new_notes.iter() {
                                                state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::PlayNoteImmediate(*note, midi_channel));
                                            }
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
                        let _ = THREAD_POOL.with_borrow(|thread_pool| thread_pool.spawn(move || {
                            for (note, _position, duration) in new_notes {
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
                            }
                        }));
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
                }
                TrackChangeType::RiffSelectWithTrackIndex{ track_index, position } => {
                    debug!("Main - rx_ui processing loop - TrackChangeType::RiffSelectWithTrackIndex");
                    match state.lock() {
                        Ok(mut state) => {
                            // get the track
                            let track_riff = if let Some(track) = state.get_project().song_mut().tracks_mut().get_mut(track_index as usize) {
                                let track_uuid = track.uuid().to_string();
                                let track_name = track.name().to_string();
                                let riff_details = track.riffs_mut().iter_mut().map(|riff| (riff.id(), (riff.name().to_string(), riff.length()))).collect::<HashMap<String, (String, f64)>>();
                                let mut riff_name = None;
                                if let Some(riff_ref) = track.riff_refs_mut().iter_mut().find(|riff_ref| {
                                    if let Some((name, riff_length)) = riff_details.get(&riff_ref.linked_to()) {
                                        riff_name = Some(name.to_string());
                                        let riff_ref_end_position = riff_ref.position() + *riff_length;
                                        if riff_ref.position() <= position && position <= riff_ref_end_position {
                                            true
                                        }
                                        else { false }
                                    }
                                    else { false }
                                }) {
                                    if let Some(riff_name) = riff_name {
                                        if riff_name.as_str() != "empty" {
                                            Some((track_uuid, riff_ref.linked_to(), track_name.to_string(), riff_name))
                                        }
                                        else { None }
                                    }
                                    else { None }
                                }
                                else { None }
                            }
                            else { None };

                            if let Some((track_uuid, riff_uuid, track_name, riff_name)) = track_riff {
                                state.set_selected_riff_uuid(track_uuid.clone(), riff_uuid);
                                state.set_selected_track(Some(track_uuid));
                                gui.set_piano_roll_selected_track_name_label(track_name.as_str());
                                gui.set_piano_roll_selected_riff_name_label(riff_name.as_str());
                                gui.ui.piano_roll_drawing_area.queue_draw();
                            }
                        }
                        Err(_) => debug!("Main - rx_ui processing loop - TrackChangeType::RiffSelectWithTrackIndex - could not get lock on state"),
                    }
                }
                TrackChangeType::RiffEventsSelectMultiple(x, y, x2, y2, add_to_select) => {
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
                        }
                        Err(_) => debug!("Main - rx_ui processing loop - riff events selected - could not get lock on state"),
                    }
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
                            } else {
                                state.selected_riff_events_mut().clear();
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - riff events selected - could not get lock on state"),
                    }
                    gui.ui.piano_roll_drawing_area.queue_draw();
                }
                TrackChangeType::RiffEventsSelectSingle(x, y, add_to_select) => {
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
                        Err(_) => debug!("Main - rx_ui processing loop - RiffEventsSelectSingle - could not get lock on state"),
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
                                                    'outer_loop:
                                                    for riff in track.riffs_mut().iter_mut() {
                                                        if riff.uuid().to_string() == *riff_uuid {
                                                            // store the notes for an undo
                                                            for track_event in riff.events_mut().iter_mut() {
                                                                if let TrackEvent::Note(note) = track_event {
                                                                    if note.note() == y && note.position()<= x && x <= (note.position() + note.length()) {
                                                                        debug!("RiffEventsSelectSingle Note selected: x={}, y={}, note position={}, note={}, note duration={}", x, y, note.position(), note.note(), note.length());
                                                                        selected.push(note.id());
                                                                        break 'outer_loop;
                                                                    }
                                                                }
                                                            }
                                                            break;
                                                        }
                                                    }
                                                },
                                                None => debug!("Main - rx_ui processing loop - RiffEventsSelectSingle - problem getting selected riff index"),
                                            }
                                        },
                                        None => ()
                                    }
                                },
                                None => debug!("Main - rx_ui processing loop - RiffEventsSelectSingle  - problem getting selected riff track number"),
                            };

                            if !selected.is_empty() {
                                let mut state = state;
                                if !add_to_select {
                                    state.selected_riff_events_mut().clear();
                                }
                                state.selected_riff_events_mut().append(&mut selected);
                            } else {
                                state.selected_riff_events_mut().clear();
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - RiffEventsSelectSingle - could not get lock on state"),
                    }
                    gui.ui.piano_roll_drawing_area.queue_draw();
                }
                TrackChangeType::RiffEventsDeselectMultiple(x, y, x2, y2) => {
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
                        Err(_) => debug!("Main - rx_ui processing loop - RiffEventsDeselectMultiple - could not get lock on state"),
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
                                                None => debug!("Main - rx_ui processing loop - RiffEventsDeselectMultiple - problem getting selected riff index"),
                                            }
                                        },
                                        None => ()
                                    }
                                },
                                None => debug!("Main - rx_ui processing loop - RiffEventsDeselectMultiple  - problem getting selected riff track number"),
                            };

                            if !selected.is_empty() {
                                let mut state = state;
                                state.selected_riff_events_mut().retain(|riff_ref_id| !selected.contains(riff_ref_id));
                            } else {
                                state.selected_riff_events_mut().clear();
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - RiffEventsDeselectMultiple - could not get lock on state"),
                    }
                    gui.ui.piano_roll_drawing_area.queue_draw();
                }
                TrackChangeType::RiffEventsDeselectSingle(x, y) => {
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
                        Err(_) => debug!("Main - rx_ui processing loop - RiffEventsDeselectSingle - could not get lock on state"),
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
                                                    'outer_loop:
                                                    for riff in track.riffs_mut().iter_mut() {
                                                        if riff.uuid().to_string() == *riff_uuid {
                                                            // store the notes for an undo
                                                            for track_event in riff.events_mut().iter_mut() {
                                                                if let TrackEvent::Note(note) = track_event {
                                                                    if note.note() == y && note.position()<= x && x <= (note.position() + note.length()) {
                                                                        debug!("RiffEventsDeselectSingle Note selected: x={}, y={}, note position={}, note={}, note duration={}", x, y, note.position(), note.note(), note.length());
                                                                        selected.push(note.id());
                                                                        break 'outer_loop;
                                                                    }
                                                                }
                                                            }
                                                            break;
                                                        }
                                                    }
                                                },
                                                None => debug!("Main - rx_ui processing loop - RiffEventsDeselectSingle - problem getting selected riff index"),
                                            }
                                        },
                                        None => ()
                                    }
                                },
                                None => debug!("Main - rx_ui processing loop - RiffEventsDeselectSingle  - problem getting selected riff track number"),
                            };

                            if !selected.is_empty() {
                                let mut state = state;
                                state.selected_riff_events_mut().retain(|riff_ref_id| !selected.contains(riff_ref_id));
                            } else {
                                state.selected_riff_events_mut().clear();
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - RiffEventsDeselectSingle - could not get lock on state"),
                    }
                    gui.ui.piano_roll_drawing_area.queue_draw();
                }
                TrackChangeType::RiffEventsSelectAll => {
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
                        Err(_) => debug!("Main - rx_ui processing loop - RiffEventsSelectAll - could not get lock on state"),
                    }
                    match state.lock() {
                        Ok(mut state) => {
                            match selected_riff_track_uuid {
                                Some(track_uuid) => {
                                    match state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                                        Some(track) => {
                                            match selected_riff_uuid {
                                                Some(riff_uuid) => {
                                                    for riff in track.riffs_mut().iter_mut() {
                                                        if riff.uuid().to_string() == *riff_uuid {
                                                            for track_event in riff.events_mut().iter_mut() {
                                                                if let TrackEvent::Note(note) = track_event {
                                                                    selected.push(note.id());
                                                                }
                                                            }
                                                            break;
                                                        }
                                                    }
                                                },
                                                None => debug!("Main - rx_ui processing loop - RiffEventsSelectAll - problem getting selected riff index"),
                                            }
                                        },
                                        None => ()
                                    }
                                },
                                None => debug!("Main - rx_ui processing loop - RiffEventsSelectAll  - problem getting selected riff track number"),
                            }

                            if !selected.is_empty() {
                                state.selected_riff_events_mut().clear();
                                state.selected_riff_events_mut().append(&mut selected);
                            } else {
                                state.selected_riff_events_mut().clear();
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - RiffEventsSelectAll - could not get lock on state"),
                    }
                    gui.ui.piano_roll_drawing_area.queue_draw();
                }
                TrackChangeType::RiffEventsDeselectAll => {
                    match state.lock() {
                        Ok(mut state) => {
                                state.selected_riff_events_mut().clear();
                        }
                        Err(_) => debug!("Main - rx_ui processing loop - RiffEventsDeselectAll - could not get lock on state"),
                    }
                    gui.ui.piano_roll_drawing_area.queue_draw();
                }
                TrackChangeType::RiffCutSelected => {
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
                TrackChangeType::RiffTranslateSelected(translation_entity_type, translate_direction) => {
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
                    // TODO implement arming of tracks for recording into???
                    // match state.lock() {
                    //     Ok(mut state) => {
                    //         // state.set_recording(record);
                    //     },
                    //     Err(_) => debug!("Main - rx_ui processing loop - transport goto start - could not get lock on state"),
                    // }
                }
                TrackChangeType::RiffQuantiseSelected => {
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
                                let mut snap_strength = 1.0;
                                let mut snap_start = true;
                                let mut snap_end = false;
                                match gui.piano_roll_grid() {
                                    Some(piano_roll_grid) => match piano_roll_grid.lock() {
                                        Ok(piano_roll) => {
                                            snap_in_beats = piano_roll.snap_position_in_beats();
                                            snap_strength = piano_roll.snap_strength();
                                            snap_start = piano_roll.snap_start();
                                            snap_end = piano_roll.snap_end();
                                        }
                                        Err(_) => (),
                                    },
                                    None => (),
                                }
                                let action = RiffQuantiseSelectedAction::new(
                                    selected_riff_events,
                                    selected_riff_track_uuid,
                                    selected_riff_uuid,
                                    snap_in_beats,
                                    snap_strength,
                                    snap_start,
                                    snap_end,
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
                TrackChangeType::RiffCopySelected => {
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
                        Err(_) => debug!("Main - rx_ui processing loop - riff copy selected - could not get lock on state"),
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
                                                    None => debug!("Main - rx_ui processing loop - riff copy selected - problem getting selected riff index"),
                                                }
                                            },
                                            None => ()
                                        }
                                    },
                                    None => debug!("Main - rx_ui processing loop - riff copy selected  - problem getting selected riff track number"),
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
                TrackChangeType::RiffReferenceCutSelected => {
                    let mut copy_buffer: Vec<RiffReference> = vec![];

                    match state.lock() {
                        Ok(mut state) => {
                            let current_view = state.current_view();
                            if let CurrentView::RiffGrid = current_view {
                                let selected_riff_references = state.selected_riff_grid_riff_references().clone();
                                let edit_cursor_position_in_secs = if let Some(grid) = gui.riff_grid() {
                                    match grid.lock() {
                                        Ok(grid) => {
                                            grid.edit_cursor_time_in_beats()
                                        }
                                        Err(_) => 0.0,
                                    }
                                } else {
                                    0.0
                                };

                                // get the selected riff grid
                                if let Some(selected_riff_grid) = state.selected_riff_grid_uuid().clone() {
                                    if let Some(riff_grid) = state.get_project().song_mut().riff_grid_mut(selected_riff_grid.clone()) {
                                        for track in riff_grid.tracks_mut().map(|track_uuid| track_uuid.clone()).collect_vec().iter() {
                                            if let Some(riff_refs) = riff_grid.track_riff_references_mut(track.clone()) {
                                                riff_refs.retain(|riff_ref| {
                                                    if selected_riff_references.clone().contains(&riff_ref.uuid().to_string()) {
                                                        let mut value = riff_ref.clone();
                                                        value.set_position(value.position() - edit_cursor_position_in_secs);
                                                        value.set_track_id(track.clone());
                                                        copy_buffer.push(value);
                                                        false
                                                    } else { true }
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                            else if let CurrentView::Track = current_view {
                                let selected_riff_references = state.selected_track_grid_riff_references().clone();
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

                                for track in state.get_project().song_mut().tracks_mut().iter_mut() {
                                    debug!("Selected track riff ref count: {}", track.riff_refs().len());
                                    let track_uuid = track.uuid_mut().to_string();
                                    track.riff_refs_mut().retain(|riff_ref| {
                                        if selected_riff_references.clone().contains(&riff_ref.uuid().to_string()) {
                                            let mut value = riff_ref.clone();
                                            value.set_position(value.position() - edit_cursor_position_in_secs);
                                            value.set_track_id(track_uuid.clone());
                                            copy_buffer.push(value);
                                            false
                                        } else { true }
                                    });
                                }
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - riff references cut selected - could not get lock on state"),
                    }

                    match state.lock() {
                        Ok(state) => {
                            if !copy_buffer.is_empty() {
                                debug!("Riff references copy buffer length: {}", copy_buffer.len());
                                let mut state = state;
                                state.track_grid_riff_references_copy_buffer_mut().clear();
                                copy_buffer.iter().for_each(|event| state.track_grid_riff_references_copy_buffer_mut().push(event.clone()));
                            }
                        },
                        Err(_) => (),
                    }
                    gui.ui.piano_roll_drawing_area.queue_draw();
                    gui.ui.track_drawing_area.queue_draw();
                },
                TrackChangeType::RiffReferenceCopySelected => {
                    let mut copy_buffer: Vec<RiffReference> = vec![];
                    match state.lock() {
                        Ok(mut state) => {
                            let current_view = state.current_view();
                            if let CurrentView::RiffGrid = current_view {
                                let selected_riff_references = state.selected_riff_grid_riff_references().clone();
                                let edit_cursor_position_in_secs = if let Some(grid) = gui.riff_grid() {
                                    match grid.lock() {
                                        Ok(grid) => {
                                            grid.edit_cursor_time_in_beats()
                                        }
                                        Err(_) => 0.0,
                                    }
                                } else {
                                    0.0
                                };

                                // get the selected riff grid
                                if let Some(selected_riff_grid) = state.selected_riff_grid_uuid().clone() {
                                    if let Some(riff_grid) = state.get_project().song_mut().riff_grid_mut(selected_riff_grid.clone()) {
                                        for track in riff_grid.tracks_mut().map(|track_uuid| track_uuid.clone()).collect_vec().iter() {
                                            if let Some(riff_refs) = riff_grid.track_riff_references_mut(track.clone()) {
                                                riff_refs.iter().filter(|riff_ref| selected_riff_references.clone().contains(&riff_ref.uuid().to_string())).for_each(|riff_ref|  {
                                                    let mut value = riff_ref.clone();
                                                    value.set_position(value.position() - edit_cursor_position_in_secs);
                                                    value.set_track_id(track.clone());
                                                    copy_buffer.push(value);
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                            else if let CurrentView::Track = current_view {
                                let selected_riff_references = state.selected_track_grid_riff_references().clone();
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

                                for track in state.get_project().song_mut().tracks_mut().iter_mut() {
                                    debug!("Selected track riff ref count: {}", track.riff_refs().len());
                                    let track_uuid = track.uuid_mut().to_string();
                                    track.riff_refs_mut().iter().filter(|riff_ref| selected_riff_references.clone().contains(&riff_ref.uuid().to_string())).for_each(|riff_ref|  {
                                        let mut value = riff_ref.clone();
                                        value.set_position(value.position() - edit_cursor_position_in_secs);
                                        value.set_track_id(track_uuid.clone());
                                        copy_buffer.push(value);
                                    });
                                }
                            }
                        }
                        Err(_) => debug!("Main - rx_ui processing loop - riff references copy selected - could not get lock on state"),
                    };

                    match state.lock() {
                        Ok(state) => {
                            if !copy_buffer.is_empty() {
                                debug!("Riff references copy buffer length: {}", copy_buffer.len());
                                let mut state = state;
                                state.track_grid_riff_references_copy_buffer_mut().clear();
                                copy_buffer.iter().for_each(|event| state.track_grid_riff_references_copy_buffer_mut().push(event.clone()));
                            }
                        },
                        Err(_) => (),
                    }
                },
                TrackChangeType::RiffReferencePaste => {
                    match state.lock() {
                        Ok(mut state) => {
                            let current_view = state.current_view();
                            if let CurrentView::RiffGrid = current_view {
                                let edit_cursor_position_in_secs = if let Some(riff_grid) = gui.riff_grid() {
                                    match riff_grid.lock() {
                                        Ok(grid) => {
                                            grid.edit_cursor_time_in_beats()
                                        },
                                        Err(_) => 0.0,
                                    }
                                } else {
                                    0.0
                                };
                                let mut copy_buffer: Vec<RiffReference> = vec![];
                                let mut copy_buffer_riff_ref_ids: Vec<String> = vec![];

                                state.riff_grid_riff_references_copy_buffer().iter().for_each(|riff_ref| {
                                    copy_buffer.push(riff_ref.clone());
                                    copy_buffer_riff_ref_ids.push(riff_ref.uuid().to_string());
                                });

                                if let Some(selected_riff_grid) = state.selected_riff_grid_uuid().clone() {
                                    if let Some(riff_grid) = state.get_project().song_mut().riff_grid_mut(selected_riff_grid.clone()) {
                                        for track_uuid in riff_grid.tracks_mut().map(|track_uuid| track_uuid.clone()).collect_vec().iter() {
                                            for riff_ref in copy_buffer.iter() {
                                                if track_uuid == riff_ref.track_id() {
                                                    if let Some(riff_refs) = riff_grid.track_riff_references_mut(track_uuid.clone()) {
                                                        riff_refs.push(RiffReference::new(riff_ref.linked_to(), riff_ref.position() + edit_cursor_position_in_secs));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            else if let CurrentView::Track = current_view {
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
                                let mut copy_buffer_riff_ref_ids: Vec<String> = vec![];
                                state.track_grid_riff_references_copy_buffer().iter().for_each(|riff_ref| {
                                    copy_buffer.push(riff_ref.clone());
                                    copy_buffer_riff_ref_ids.push(riff_ref.uuid().to_string());
                                });
                                let mut state = state;

                                for track in state.get_project().song_mut().tracks_mut().iter_mut() {
                                    let track_uuid = track.uuid_mut().to_string();
                                    for riff_ref in copy_buffer.iter() {
                                        if track_uuid == riff_ref.track_id() {
                                            track.riff_refs_mut().push(RiffReference::new(riff_ref.linked_to(), riff_ref.position() + edit_cursor_position_in_secs));
                                        }
                                    }
                                }

                                // re-calculate the song length
                                state.get_project().song_mut().recalculate_song_length();
                            }
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
                TrackChangeType::RiffChangeLengthOfSelected(lengthen) => {
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
                }
                TrackChangeType::AutomationSelectMultiple(time_lower, value_lower, time_higher, value_higher, add_to_select) => {
                    match state.lock() {
                        Ok(state) => {
                            let note_expression_type = state.note_expression_type().clone();
                            let note_expression_note_id = state.note_expression_id();
                            let automation_view_mode = {
                                match state.automation_view_mode() {
                                    AutomationViewMode::NoteVelocities => AutomationViewMode::NoteVelocities,
                                    AutomationViewMode::Controllers => AutomationViewMode::Controllers,
                                    AutomationViewMode::PitchBend => AutomationViewMode::PitchBend,
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
                            } else {
                                None
                            };
                            let selected_effect_plugin_uuid = if let Some(uuid) = state.selected_effect_plugin_uuid() {
                                uuid.clone()
                            } else {
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
                                                let events = if let AutomationViewMode::NoteVelocities = automation_view_mode {
                                                    if let Some(selected_riff_uuid) = selected_riff_uuid {
                                                        if let Some(riff) = track_type.riffs().iter().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
                                                            Some(riff.events_vec())
                                                        } else { None }
                                                    } else { None }
                                                } else if let CurrentView::RiffArrangement = current_view {
                                                    let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                                                        Some(selected_arrangement_uuid.clone())
                                                    } else { None };

                                                    // get the arrangement
                                                    if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                                                        if let Some(riff_arrangement) = state.project().song().riff_arrangement(selected_arrangement_uuid.clone()) {
                                                            if let Some(automation) = riff_arrangement.automation(&track_uuid) {
                                                                if state.automation_discrete() {
                                                                    Some(automation.events())
                                                                }
                                                                else {
                                                                    if let Some(automation_type_value) = automation_type {
                                                                        if let Some(automation_envelope) = automation.envelopes().iter().find(|envelope| {
                                                                            let mut found = false;

                                                                            // need to know what kind of events we are looking for in order to get the appropriate envelope
                                                                            match automation_view_mode {
                                                                                AutomationViewMode::NoteVelocities => {}
                                                                                AutomationViewMode::Controllers => {
                                                                                    if let TrackEvent::Controller(controller) = envelope.event_details() {
                                                                                        if controller.controller() == automation_type_value {
                                                                                            found = true;
                                                                                        }
                                                                                    }
                                                                                }
                                                                                AutomationViewMode::PitchBend => {
                                                                                    if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                                                                        found = true;
                                                                                    }
                                                                                }
                                                                                AutomationViewMode::Instrument => {
                                                                                    let plugin_uuid = if let TrackType::InstrumentTrack(instrument_track) = track_type {
                                                                                        instrument_track.instrument().uuid().to_string()
                                                                                    } else {
                                                                                        "".to_string()
                                                                                    };
                                                                                    if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                                                        if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid {
                                                                                            found = true;
                                                                                        }
                                                                                    }
                                                                                }
                                                                                AutomationViewMode::Effect => {
                                                                                    if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                                                        if param.index == automation_type_value && param.plugin_uuid() == selected_effect_plugin_uuid {
                                                                                            found = true;
                                                                                        }
                                                                                    }
                                                                                }
                                                                                AutomationViewMode::NoteExpression => {
                                                                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                                                                        if *note_expression.expression_type() as i32 == automation_type_value {
                                                                                            found = true;
                                                                                        }
                                                                                    }
                                                                                }
                                                                            }
                                                                            return found;
                                                                        }) {
                                                                            Some(automation_envelope.events())
                                                                        } else { None }
                                                                    }
                                                                    else { None }
                                                                }
                                                            } else { None }
                                                        } else { None }
                                                    } else { None }
                                                } else {
                                                    match automation_edit_type {
                                                        AutomationEditType::Track => {
                                                            let automation = track_type.automation();
                                                            if state.automation_discrete() {
                                                                Some(automation.events())
                                                            }
                                                            else {
                                                                if let Some(automation_type_value) = automation_type {
                                                                    if let Some(automation_envelope) = automation.envelopes().iter().find(|envelope| {
                                                                        let mut found = false;

                                                                        // need to know what kind of events we are looking for in order to get the appropriate envelope
                                                                        match automation_view_mode {
                                                                            AutomationViewMode::NoteVelocities => {}
                                                                            AutomationViewMode::Controllers => {
                                                                                if let TrackEvent::Controller(controller) = envelope.event_details() {
                                                                                    if controller.controller() == automation_type_value {
                                                                                        found = true;
                                                                                    }
                                                                                }
                                                                            }
                                                                            AutomationViewMode::PitchBend => {
                                                                                if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                                                                    found = true;
                                                                                }
                                                                            }
                                                                            AutomationViewMode::Instrument => {
                                                                                let plugin_uuid = if let TrackType::InstrumentTrack(instrument_track) = track_type {
                                                                                    instrument_track.instrument().uuid().to_string()
                                                                                } else {
                                                                                    "".to_string()
                                                                                };
                                                                                if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                                                    if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid {
                                                                                        found = true;
                                                                                    }
                                                                                }
                                                                            }
                                                                            AutomationViewMode::Effect => {
                                                                                if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                                                    if param.index == automation_type_value && param.plugin_uuid() == selected_effect_plugin_uuid {
                                                                                        found = true;
                                                                                    }
                                                                                }
                                                                            }
                                                                            AutomationViewMode::NoteExpression => {
                                                                                if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                                                                    if *note_expression.expression_type() as i32 == automation_type_value {
                                                                                        found = true;
                                                                                    }
                                                                                }
                                                                            }
                                                                        }
                                                                        return found;
                                                                    }) {
                                                                        Some(automation_envelope.events())
                                                                    } else { None }
                                                                }
                                                                else { None }
                                                            }
                                                        }
                                                        AutomationEditType::Riff => {
                                                            if let Some(selected_riff_uuid) = selected_riff_uuid {
                                                                if let Some(riff) = track_type.riffs().iter().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
                                                                    Some(riff.events_vec())
                                                                } else { None }
                                                            } else { None }
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
                                                        AutomationViewMode::PitchBend => {
                                                            let value_lower = (value_lower as f32 / 127.0 * 16384.0 - 8192.0) as i32;
                                                            let value_higher = (value_higher as f32 / 127.0 * 16384.0 - 8192.0) as i32;
                                                            events.iter().for_each(|event| {
                                                                match event {
                                                                    TrackEvent::PitchBend(pitch_bend) => {
                                                                        let position = pitch_bend.position();
                                                                        if time_lower <= position && position <= time_higher
                                                                            && value_lower <= pitch_bend.value() && pitch_bend.value() <= value_higher {
                                                                            selected.push(pitch_bend.id());
                                                                        }
                                                                    }
                                                                    _ => (),
                                                                }
                                                            })
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
                                                        AutomationViewMode::NoteExpression => {
                                                            events.iter().for_each(|event| {
                                                                match event {
                                                                    TrackEvent::NoteExpression(note_expression) => {
                                                                        let position = note_expression.position();
                                                                        if time_lower <= position &&
                                                                            position <= time_higher &&
                                                                            note_expression_type as i32 == *(note_expression.expression_type()) as i32 &&
                                                                            note_expression_note_id == note_expression.note_id() {
                                                                            selected.push(note_expression.id());
                                                                        }
                                                                    }
                                                                    _ => (),
                                                                }
                                                            })
                                                        }
                                                    }
                                                }
                                            },
                                            None => ()
                                        }
                                    },
                                None => debug!("Main - rx_ui processing loop - AutomationSelectMultiple - problem getting selected track number"),
                            }

                            let mut state = state;
                            if !add_to_select {
                                state.selected_automation_mut().clear();
                            }

                            if !selected.is_empty() {
                                state.selected_automation_mut().append(&mut selected);
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - AutomationSelectMultiple - could not get lock on state"),
                    };
                    gui.ui.track_drawing_area.queue_draw();
                    gui.ui.automation_drawing_area.queue_draw();
                }
                TrackChangeType::AutomationDeselectMultiple(time_lower, value_lower, time_higher, value_higher) => {
                    match state.lock() {
                        Ok(state) => {
                            let note_expression_type = state.note_expression_type().clone();
                            let note_expression_note_id = state.note_expression_id();
                            let automation_view_mode = {
                                match state.automation_view_mode() {
                                    AutomationViewMode::NoteVelocities => AutomationViewMode::NoteVelocities,
                                    AutomationViewMode::Controllers => AutomationViewMode::Controllers,
                                    AutomationViewMode::PitchBend => AutomationViewMode::PitchBend,
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
                            } else {
                                None
                            };
                            let selected_effect_plugin_uuid = if let Some(uuid) = state.selected_effect_plugin_uuid() {
                                uuid.clone()
                            } else {
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
                                                let events = if let AutomationViewMode::NoteVelocities = automation_view_mode {
                                                    if let Some(selected_riff_uuid) = selected_riff_uuid {
                                                        if let Some(riff) = track_type.riffs().iter().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
                                                            Some(riff.events_vec())
                                                        } else {
                                                            None
                                                        }
                                                    } else {
                                                        None
                                                    }
                                                } else if let CurrentView::RiffArrangement = current_view {
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
                                                        AutomationViewMode::PitchBend => {
                                                            let value_lower = (value_lower as f32 / 127.0 * 16384.0 - 8192.0) as i32;
                                                            let value_higher = (value_higher as f32 / 127.0 * 16384.0 - 8192.0) as i32;
                                                            events.iter().for_each(|event| {
                                                                match event {
                                                                    TrackEvent::PitchBend(pitch_bend) => {
                                                                        let position = pitch_bend.position();
                                                                        if time_lower <= position && position <= time_higher
                                                                            && value_lower <= pitch_bend.value() && pitch_bend.value() <= value_higher {
                                                                            selected.push(pitch_bend.id());
                                                                        }
                                                                    }
                                                                    _ => (),
                                                                }
                                                            })
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
                                                        AutomationViewMode::NoteExpression => {
                                                            events.iter().for_each(|event| {
                                                                match event {
                                                                    TrackEvent::NoteExpression(note_expression) => {
                                                                        let position = note_expression.position();
                                                                        if time_lower <= position &&
                                                                            position <= time_higher &&
                                                                            note_expression_type as i32 == *(note_expression.expression_type()) as i32 &&
                                                                            note_expression_note_id == note_expression.note_id() {
                                                                            selected.push(note_expression.id());
                                                                        }
                                                                    }
                                                                    _ => (),
                                                                }
                                                            })
                                                        }
                                                    }
                                                }
                                            },
                                            None => ()
                                        }
                                    },
                                None => debug!("Main - rx_ui processing loop - AutomationSelectMultiple - problem getting selected track number"),
                            }

                            let mut state = state;
                            if !selected.is_empty() {
                                state.selected_automation_mut().retain(|automation_id| !selected.contains(automation_id));
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - AutomationSelectMultiple - could not get lock on state"),
                    };
                    gui.ui.track_drawing_area.queue_draw();
                    gui.ui.automation_drawing_area.queue_draw();
                }
                TrackChangeType::AutomationSelectAll => {
                    match state.lock() {
                        Ok(state) => {
                            let note_expression_type = state.note_expression_type().clone();
                            let note_expression_note_id = state.note_expression_id();
                            let note_expression_type = state.note_expression_type().clone();
                            let note_expression_port_index = state.note_expression_port_index() as i16;
                            let note_expression_channel = state.note_expression_channel() as i16;
                            let note_expression_key = state.note_expression_key();
                            let automation_view_mode = {
                                match state.automation_view_mode() {
                                    AutomationViewMode::NoteVelocities => AutomationViewMode::NoteVelocities,
                                    AutomationViewMode::Controllers => AutomationViewMode::Controllers,
                                    AutomationViewMode::PitchBend => AutomationViewMode::PitchBend,
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
                            } else {
                                None
                            };
                            let selected_effect_plugin_uuid = if let Some(uuid) = state.selected_effect_plugin_uuid() {
                                uuid.clone()
                            } else {
                                "".to_string()
                            };
                            let current_view = state.current_view().clone();
                            let automation_edit_type = state.automation_edit_type();
                            let song = state.project().song();
                            let tracks = song.tracks();
                            let automation_discrete = state.automation_discrete();
                            let mut selected = Vec::new();

                            match track_uuid {
                                Some(track_uuid) =>
                                    {
                                        match tracks.iter().find(|track| track.uuid().to_string() == track_uuid) {
                                            Some(track_type) => {
                                                let events = if let AutomationViewMode::NoteVelocities = automation_view_mode {
                                                    if let Some(selected_riff_uuid) = selected_riff_uuid {
                                                        if let Some(riff) = track_type.riffs().iter().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
                                                            Some(riff.events_vec())
                                                        } else {
                                                            None
                                                        }
                                                    } else {
                                                        None
                                                    }
                                                } else if let CurrentView::RiffArrangement = current_view {
                                                    let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                                                        Some(selected_arrangement_uuid.clone())
                                                    } else {
                                                        None
                                                    };

                                                    // get the arrangement
                                                    if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                                                        if let Some(riff_arrangement) = state.project().song().riff_arrangement(selected_arrangement_uuid.clone()) {
                                                            if let Some(automation) = riff_arrangement.automation(&track_uuid) {
                                                                if automation_discrete {
                                                                    Some(automation.events())
                                                                }
                                                                else {
                                                                    // find the relevant envelope
                                                                    if let Some(automation_type_value) = automation_type {
                                                                        let instrument_plugin_uuid = if let TrackType::InstrumentTrack(track) = track_type {
                                                                            track.instrument().uuid().to_string()
                                                                        }
                                                                        else {
                                                                            "".to_string()
                                                                        };

                                                                        match automation_view_mode {
                                                                            AutomationViewMode::NoteVelocities => {
                                                                                let find_fn = |envelope: &&AutomationEnvelope| {
                                                                                    let mut found = false;
                                                                                    if let TrackEvent::Note(_) = envelope.event_details() {
                                                                                        found = true;
                                                                                    }
                                                                                    return found;
                                                                                };
                                                                                if let Some(automation_envelope) = automation.envelopes().iter().find(find_fn) {
                                                                                    Some(automation_envelope.events())
                                                                                } else { None }
                                                                            }
                                                                            AutomationViewMode::Controllers => {
                                                                                let find_fn = |envelope: &&AutomationEnvelope| {
                                                                                    let mut found = false;
                                                                                    if let TrackEvent::Controller(controller) = envelope.event_details() {
                                                                                        if controller.controller() == automation_type_value {
                                                                                            found = true;
                                                                                        }
                                                                                    }
                                                                                    return found;
                                                                                };
                                                                                if let Some(automation_envelope) = automation.envelopes().iter().find(find_fn) {
                                                                                    Some(automation_envelope.events())
                                                                                } else { None }
                                                                            }
                                                                            AutomationViewMode::PitchBend => {
                                                                                let find_fn = |envelope: &&AutomationEnvelope| {
                                                                                    let mut found = false;
                                                                                    if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                                                                        found = true;
                                                                                    }
                                                                                    return found;
                                                                                };
                                                                                if let Some(automation_envelope) = automation.envelopes().iter().find(find_fn) {
                                                                                    Some(automation_envelope.events())
                                                                                } else { None }
                                                                            }
                                                                            AutomationViewMode::Instrument => {
                                                                                let find_fn = |envelope: &&AutomationEnvelope| {
                                                                                    let mut found = false;
                                                                                    if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                                                        if param.index == automation_type_value && param.plugin_uuid() == instrument_plugin_uuid {
                                                                                            found = true;
                                                                                        }
                                                                                    }
                                                                                    return found;
                                                                                };
                                                                                if let Some(automation_envelope) = automation.envelopes().iter().find(find_fn) {
                                                                                    Some(automation_envelope.events())
                                                                                } else { None }
                                                                            }
                                                                            AutomationViewMode::Effect => {
                                                                                let find_fn = |envelope: &&AutomationEnvelope| {
                                                                                    let mut found = false;
                                                                                    if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                                                        if param.index == automation_type_value && param.plugin_uuid() == selected_effect_plugin_uuid {
                                                                                            found = true;
                                                                                        }
                                                                                    }
                                                                                    return found;
                                                                                };
                                                                                if let Some(automation_envelope) = automation.envelopes().iter().find(find_fn) {
                                                                                    Some(automation_envelope.events())
                                                                                } else { None }
                                                                            }
                                                                            AutomationViewMode::NoteExpression => {
                                                                                let find_fn = |envelope: &&AutomationEnvelope| {
                                                                                    let mut found = false;
                                                                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                                                                        if
                                                                                        *(note_expression.expression_type()) == note_expression_type &&
                                                                                            note_expression.port() == note_expression_port_index &&
                                                                                            note_expression.channel() == note_expression_channel &&
                                                                                            note_expression.note_id() == note_expression_note_id &&
                                                                                            note_expression.key() == note_expression_key
                                                                                        {
                                                                                            found = true;
                                                                                        }
                                                                                    }
                                                                                    return found;
                                                                                };
                                                                                if let Some(automation_envelope) = automation.envelopes().iter().find(find_fn) {
                                                                                    Some(automation_envelope.events())
                                                                                } else { None }
                                                                            }
                                                                        }
                                                                    }
                                                                    else { None }
                                                                }
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
                                                                        selected.push(note.id());
                                                                    }
                                                                    _ => {}
                                                                }
                                                            }
                                                        }
                                                        AutomationViewMode::Controllers => {
                                                            if let Some(automation_type_value) = automation_type {
                                                                events.iter().for_each(|event| {
                                                                    match event {
                                                                        TrackEvent::Controller(controller) => {
                                                                            if controller.controller() == automation_type_value {
                                                                                selected.push(controller.id());
                                                                            }
                                                                        }
                                                                        _ => (),
                                                                    }
                                                                })
                                                            }
                                                        }
                                                        AutomationViewMode::PitchBend => {
                                                            events.iter().for_each(|event| {
                                                                match event {
                                                                    TrackEvent::PitchBend(pitch_bend) => {
                                                                        selected.push(pitch_bend.id());
                                                                    }
                                                                    _ => (),
                                                                }
                                                            })
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
                                                                            if plugin_param.index == automation_type_value &&
                                                                                plugin_param.plugin_uuid.to_string() == instrument_plugin_id.to_string() {
                                                                                selected.push(plugin_param.id());
                                                                            }
                                                                        }
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
                                                                            if plugin_param.index == automation_type_value &&
                                                                                plugin_param.plugin_uuid.to_string() == selected_effect_plugin_uuid {
                                                                                selected.push(plugin_param.id());
                                                                            }
                                                                        }
                                                                        _ => (),
                                                                    }
                                                                })
                                                            }
                                                        }
                                                        AutomationViewMode::NoteExpression => {
                                                            events.iter().for_each(|event| {
                                                                match event {
                                                                    TrackEvent::NoteExpression(note_expression) => {
                                                                        if note_expression_type as i32 == *(note_expression.expression_type()) as i32 &&
                                                                            note_expression_note_id == note_expression.note_id() {
                                                                            selected.push(note_expression.id());
                                                                        }
                                                                    }
                                                                    _ => (),
                                                                }
                                                            })
                                                        }
                                                    }
                                                }
                                            },
                                            None => ()
                                        }
                                    },
                                None => debug!("Main - rx_ui processing loop - AutomationSelectAll - problem getting selected track number"),
                            }

                            let mut state = state;
                            state.selected_automation_mut().clear();

                            if !selected.is_empty() {
                                state.selected_automation_mut().append(&mut selected);
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - AutomationSelectAll - could not get lock on state"),
                    };
                    gui.ui.track_drawing_area.queue_draw();
                    gui.ui.automation_drawing_area.queue_draw();
                }
                TrackChangeType::AutomationDeselectAll => {
                    match state.lock() {
                        Ok(mut state) => {
                            state.selected_automation_mut().clear();
                            gui.ui.track_drawing_area.queue_draw();
                            gui.ui.automation_drawing_area.queue_draw();
                        }
                        Err(_) => debug!("Main - rx_ui processing loop - AutomationSelectMultiple - could not get lock on state"),
                    }
                }
                TrackChangeType::AutomationAdd(automation) => {
                    for automation_item in automation.iter() {
                        handle_automation_add(automation_item.0, automation_item.1, &state);
                    }
                    gui.ui.track_drawing_area.queue_draw();
                    gui.ui.automation_drawing_area.queue_draw();
                }
                TrackChangeType::AutomationDelete(time) => {
                    handle_automation_delete(time, &state);
                    gui.ui.track_drawing_area.queue_draw();
                    gui.ui.automation_drawing_area.queue_draw();
                }
                TrackChangeType::AutomationTranslateSelected(_translation_entity_type, translate_direction) => {
                    let mut snap_in_beats = 1.0;
                    match gui.automation_grid() {
                        Some(controller_grid) => match controller_grid.lock() {
                            Ok(grid) => snap_in_beats = grid.snap_position_in_beats(),
                            Err(_) => (),
                        },
                        None => (),
                    }
                    handle_automation_translate_selected(state, translate_direction, snap_in_beats);
                    gui.ui.track_drawing_area.queue_draw();
                    gui.ui.automation_drawing_area.queue_draw();
                }
                TrackChangeType::AutomationChange(change) => {
                    debug!("TrackChangeType::AutomationChange");
                    handle_automation_change(&state, change);
                    gui.ui.track_drawing_area.queue_draw();
                    gui.ui.automation_drawing_area.queue_draw();
                }
                TrackChangeType::AutomationQuantiseSelected => {
                    let mut snap_in_beats = 1.0;
                    let mut quantise_strength = 1.0;
                    match gui.automation_grid() {
                        Some(grid) => match grid.lock() {
                            Ok(grid) => {
                                snap_in_beats = grid.snap_position_in_beats();
                                quantise_strength = grid.snap_strength();
                            }
                            Err(_) => (),
                        },
                        None => (),
                    }

                    handle_automation_quantise(&state, snap_in_beats, quantise_strength);
                    gui.ui.track_drawing_area.queue_draw();
                    gui.ui.automation_drawing_area.queue_draw();
                }
                TrackChangeType::AutomationCut => {
                    let edit_cursor_time_in_beats = if let Some(grid) = gui.automation_grid() {
                        match grid.lock() {
                            Ok(grid) => grid.edit_cursor_time_in_beats(),
                            Err(_) => 0.0,
                        }
                    } else { 0.0 };
                    handle_automation_cut(&state, edit_cursor_time_in_beats);
                    gui.ui.track_drawing_area.queue_draw();
                    gui.ui.automation_drawing_area.queue_draw();
                },
                TrackChangeType::AutomationCopy => {
                    let edit_cursor_time_in_beats = if let Some(grid) = gui.automation_grid() {
                        match grid.lock() {
                            Ok(grid) => grid.edit_cursor_time_in_beats(),
                            Err(_) => 0.0,
                        }
                    } else { 0.0 };
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
                    } else { 0.0 };
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
                                let sample_rate = state.configuration.audio.sample_rate as f64;
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
                                let sample_rate = state.configuration.audio.sample_rate as f64;
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
                                    let sample_rate = state.configuration.audio.sample_rate as f64;;
                                    let block_size = state.configuration.audio.block_size as f64;;
                                    let tempo = state.project().song().tempo();
                                    let time_signature_numerator = state.project().song().time_signature_numerator();
                                    let time_signature_denominator = state.project().song().time_signature_denominator();

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
                                        &mut new_track_type,
                                        None,
                                        None,
                                        vst_host_time_info.clone(),
                                        sample_rate,
                                        block_size,
                                        tempo,
                                        time_signature_numerator as i32,
                                        time_signature_denominator as i32,
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
                                } else {
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
                                } else {
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
                                } else {
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
                TrackChangeType::RiffEventChange(change) => {
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
                                    for (original_event_copy, changed_event) in change.iter() {
                                        match original_event_copy {
                                            TrackEvent::Note(original_note_copy) => {
                                                if let Some(track) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == selected_riff_track_uuid) {
                                                    if let Some(riff) = track.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == selected_riff_uuid) {
                                                        for event in riff.events_mut().iter_mut() {
                                                            if let TrackEvent::Note(note) = event {
                                                                if *note == *original_note_copy {
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
                                }
                                Err(_error) => debug!("Main - rx_ui processing loop - riff translate event - could not get lock on state"),
                            }
                        }
                    }
                    gui.ui.piano_roll_drawing_area.queue_draw();
                }
                TrackChangeType::RiffReferenceChange(mut change) => {
                    match state.lock() {
                        Ok(mut state) => {
                            let mut snap_position_in_beats = 1.0;
                            match gui.riff_grid() {
                                Some(riff_grid) => match riff_grid.lock() {
                                    Ok(grid) => {
                                        snap_position_in_beats = grid.snap_position_in_beats();
                                    }
                                    Err(_) => (),
                                },
                                None => (),
                            }

                            for track in state.get_project().song_mut().tracks_mut().iter_mut() {
                                let mut unused_changes = vec![];
                                for (original_riff, changed_riff) in change.iter() {
                                    let mut used = false;
                                    let mut riff_id = "".to_string();

                                    if let Some(riff_ref) = track.riff_refs_mut().iter_mut().find(|riff_ref| riff_ref.uuid().to_string() == changed_riff.uuid().to_string()) {
                                        let delta = riff_ref.position() - changed_riff.position();

                                        riff_id = riff_ref.linked_to();

                                        if delta < -0.000001 || delta > 0.000001 {
                                            let calculated_value = DAWUtils::quantise(changed_riff.position(), snap_position_in_beats, 1.0, false);
                                            if calculated_value.snapped {
                                                riff_ref.set_position(calculated_value.snapped_value);
                                            }
                                        }
                                        used = true;
                                    }

                                    if let Some(riff) = track.riffs_mut().iter_mut().find(|riff| riff.id() == riff_id) {
                                        let delta = riff.length() - changed_riff.length();
                                        if delta < -0.000001 || delta > 0.000001 {
                                            let calculated_value = DAWUtils::quantise(changed_riff.length(), snap_position_in_beats, 1.0, true);
                                            if calculated_value.snapped {
                                                riff.set_length(calculated_value.snapped_value);
                                            }
                                        }
                                        used = true;
                                    }

                                    if !used {
                                        unused_changes.push((original_riff.clone(), changed_riff.clone()));
                                    }
                                }
                                change.clear();
                                change.append(&mut unused_changes);
                            }
                            gui.ui.track_drawing_area.queue_draw();
                        }
                        Err(_error) => debug!("Main - rx_ui processing loop - riff reference change - could not get lock on state"),
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

                                for (key, value) in state.instrument_plugins().iter() {
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
                TrackChangeType::RiffSetStartNote(note_number, position) => {
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
                        Err(_) => debug!("Main - rx_ui processing loop - set riff start note - could not get lock on state"),
                    }

                    match state.lock() {
                        Ok(state) => {
                            let mut state = state;

                            match selected_riff_track_uuid {
                                Some(track_uuid) => {
                                    for track in state.get_project().song_mut().tracks_mut().iter_mut() {
                                        if track.uuid().to_string() == track_uuid {
                                            match selected_riff_uuid {
                                                Some(riff_uuid) => {
                                                    for riff in track.riffs_mut().iter_mut() {
                                                        if riff.uuid().to_string() == *riff_uuid {
                                                            // find the current start note
                                                            let current_start_note_details = if let Some(current_start_note) = riff.events_mut().iter_mut().find(|event| match event {
                                                                TrackEvent::Note(note) => note.note() == note_number && note.position() <= position && position <= (note.position() + note.length()) && note.riff_start_note(),
                                                                _ => false,
                                                            }) {
                                                                if let TrackEvent::Note(note) = current_start_note {
                                                                    Some((note.note(), note.position(), note.length()))
                                                                } else {
                                                                    None
                                                                }
                                                            } else {
                                                                None
                                                            };

                                                            // reset the previous start note
                                                            riff.events_mut().iter_mut().for_each(|event| {
                                                                if let TrackEvent::Note(note) = event {
                                                                    note.set_riff_start_note(false);
                                                                }
                                                            });
                                                            let note = riff.events_mut().iter_mut().find(|event| match event {
                                                                TrackEvent::Note(note) => note.note() == note_number && note.position() <= position && position <= (note.position() + note.length()),
                                                                _ => false,
                                                            });
                                                            if let Some(event) = note {
                                                                match event {
                                                                    TrackEvent::Note(note) => {
                                                                        debug!("Set riff start note: position={}, note={}, velocity={}, duration={}", note.position(), note.note(), note.velocity(), note.length());
                                                                        if let Some((current_start_note_number, current_start_note_position, current_start_note_length)) = current_start_note_details {
                                                                            if note.note() != current_start_note_number || note.position() != current_start_note_position || note.length() != current_start_note_length {
                                                                                note.set_riff_start_note(true);
                                                                            }
                                                                        } else {
                                                                            note.set_riff_start_note(true);
                                                                        }
                                                                    }
                                                                    _ => {}
                                                                }
                                                            }
                                                            break;
                                                        }
                                                    }
                                                }
                                                None => debug!("problem getting selected riff index"),
                                            }

                                            break;
                                        }
                                    }
                                },
                                None => debug!("problem getting selected riff track number"),
                            };
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - set riff start note - could not get lock on state"),
                    }
                }
                TrackChangeType::RiffReferencePlayMode(track_number, position) => {
                    // FIXME need to take into account the context - current view etc.
                    match state.lock() {
                        Ok(mut state) => {
                            let mut found = None;
                            match state.current_view().clone() {
                                CurrentView::Track => {
                                    if let Some(track) = state.get_project().song_mut().tracks_mut().get_mut(track_number as usize) {
                                        for riff_ref in track.riff_refs().iter().filter(|riff_ref| riff_ref.position() <= position) {
                                            if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_ref.linked_to()) {
                                                // position is inside the riff ref
                                                if riff_ref.position() <= position && position <= (riff_ref.position() + riff.length()) {
                                                    if position <= (riff_ref.position() + 1.0) {
                                                        found = Some((riff_ref.uuid(), RiffReferenceMode::Start));
                                                        break;
                                                    } else if position >= (riff_ref.position() + riff.length() - 1.0) {
                                                        found = Some((riff_ref.uuid(), RiffReferenceMode::End));
                                                        break;
                                                    } else {
                                                        found = Some((riff_ref.uuid(), RiffReferenceMode::Normal));
                                                        break;
                                                    }
                                                }
                                            }
                                        }

                                        if let Some((riff_ref_uuid, mode)) = found {
                                            if let Some(riff_ref) = track.riff_refs_mut().iter_mut().find(|riff_ref| riff_ref.uuid() == riff_ref_uuid) {
                                                riff_ref.set_mode(mode);
                                            }
                                        }
                                    }
                                }
                                CurrentView::RiffSet => {
                                    debug!("*****************************No idea what to do with a riff set when setting a riff ref mode.");
                                }
                                CurrentView::RiffGrid => {
                                    let track_details = if let Some(track) = state.project().song().tracks().get(track_number as usize) {
                                        let mut riff_lengths = HashMap::new();
                                        for riff in track.riffs().iter() {
                                            riff_lengths.insert(riff.uuid().to_string(), riff.length());
                                        }
                                        Some((track.uuid().to_string(), riff_lengths))
                                    } else {
                                        None
                                    };
                                    if let Some((track_uuid, riff_lengths)) = track_details {
                                        if let Some(selected_riff_grid_uuid) = state.selected_riff_grid_uuid().clone() {
                                            if let Some(riff_grid) = state.get_project().song_mut().riff_grid_mut(selected_riff_grid_uuid.clone()) {
                                                if let Some(track_riff_refs) = riff_grid.track_riff_references(track_uuid.clone()) {
                                                    for riff_ref in track_riff_refs.iter().filter(|riff_ref| riff_ref.position() <= position) {
                                                        if let Some(riff_length) = riff_lengths.get(&riff_ref.linked_to()) {
                                                            // position is inside the riff ref
                                                            if riff_ref.position() <= position && position <= (riff_ref.position() + riff_length) {
                                                                if position <= (riff_ref.position() + 1.0) {
                                                                    found = Some((riff_ref.uuid(), RiffReferenceMode::Start));
                                                                    break;
                                                                } else if position >= (riff_ref.position() + riff_length - 1.0) {
                                                                    found = Some((riff_ref.uuid(), RiffReferenceMode::End));
                                                                    break;
                                                                } else {
                                                                    found = Some((riff_ref.uuid(), RiffReferenceMode::Normal));
                                                                    break;
                                                                }
                                                            }
                                                        }
                                                    }

                                                    if let Some((riff_ref_uuid, mode)) = found {
                                                        if let Some(riff_refs) = riff_grid.track_riff_references_mut(track_uuid) {
                                                            if let Some(riff_ref) = riff_refs.iter_mut().find(|riff_ref| riff_ref.uuid() == riff_ref_uuid) {
                                                                riff_ref.set_mode(mode);
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - set riff reference play mode - could not get lock on state"),
                    }
                }
                TrackChangeType::RiffReferenceDragCopy(mut new_riff_references_details) => {
                    match state.lock() {
                        Ok(mut state) => {
                            let mut snap_position_in_beats = 1.0;
                            match gui.track_grid() {
                                Some(track_grid) => match track_grid.lock() {
                                    Ok(grid) => snap_position_in_beats = grid.snap_position_in_beats(),
                                    Err(_) => (),
                                },
                                None => (),
                            }

                            for track_type in state.get_project().song_mut().tracks_mut().iter_mut() {
                                let mut unused_changes = vec![];
                                for (position, original_riff_ref_uuid) in new_riff_references_details.iter() {
                                    // get the original riff ref linked to value
                                    let linked_to = if let Some(original_riff_ref) = track_type.riff_refs_mut().iter_mut().find(|riff_ref| riff_ref.id() == original_riff_ref_uuid.clone()) {
                                        Some(original_riff_ref.linked_to())
                                    } else {
                                        None
                                    };
                                    if let Some(linked_to) = linked_to {
                                        let snap_delta = position % snap_position_in_beats;
                                        let new_position = position - snap_delta;
                                        if new_position >= 0.0 {
                                            let riff_ref = RiffReference::new(linked_to, new_position);
                                            match track_type {
                                                TrackType::InstrumentTrack(track) => {
                                                    track.riff_refs_mut().push(riff_ref);
                                                }
                                                TrackType::MidiTrack(track) => {
                                                    track.riff_refs_mut().push(riff_ref);
                                                }
                                                _ => {}
                                            }
                                        }
                                    } else {
                                        unused_changes.push((*position, original_riff_ref_uuid.clone()));
                                    }
                                }

                                new_riff_references_details.clear();
                                new_riff_references_details.append(&mut unused_changes);
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - add new riff reference to track - could not get lock on state"),
                    }
                    gui.ui.track_drawing_area.queue_draw();
                }
                TrackChangeType::RiffReferencesSelectMultiple(x, y, x2, y2, add_to_select) => {
                    debug!("Main - rx_ui processing loop - TrackChangeType::RiffReferencesSelectMultiple: x={}, y={}, x2={}, y2={}, add_to_select={}", x, y, x2, y2, add_to_select);
                    match state.lock() {
                        Ok(state) => {
                            let mut selected = Vec::new();
                            let mut state = state;

                            for (index, track) in state.get_project().song_mut().tracks_mut().iter_mut().enumerate() {
                                let track_number = index as i32;
                                if y < track_number && track_number < y2 {
                                    let track_uuid = track.uuid().to_string();
                                    let riff_lengths = track.riffs().iter().map(|riff| (riff.uuid().to_string(), riff.length())).collect_vec();
                                    for riff_ref in track.riff_refs_mut().iter_mut() {
                                        let riff_length = riff_lengths.iter().find(|riff_length_details| riff_length_details.0 == riff_ref.linked_to());
                                        if let Some((_, length)) = riff_length {
                                            if x <= riff_ref.position() && (riff_ref.position() + length) <= x2 {
                                                debug!("Riff ref selected: uuid={}, x={}, y={}, x2={}, y2={}, position={}, track={}, length={}", riff_ref.uuid().to_string(), x, y, x2, y2, riff_ref.position(), track_uuid.clone(), length);
                                                selected.push(riff_ref.uuid().to_string());
                                            }
                                        }
                                    }
                                }
                            }

                            if !selected.is_empty() {
                                let mut state = state;
                                if !add_to_select {
                                    state.selected_track_grid_riff_references_mut().clear();
                                }
                                state.selected_track_grid_riff_references_mut().append(&mut selected);
                            } else {
                                state.selected_track_grid_riff_references_mut().clear();
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - riff references select multiple - could not get lock on state"),
                    }
                    gui.ui.track_drawing_area.queue_draw();
                }
                TrackChangeType::RiffReferencesSelectSingle(x, y, add_to_select) => {
                    debug!("Main - rx_ui processing loop - TrackChangeType::RiffReferencesSelectSingle: x={}, y={}, add_to_select={}", x, y, add_to_select);
                    match state.lock() {
                        Ok(state) => {
                            let mut selected = Vec::new();
                            let mut state = state;

                            if let Some(track) = state.get_project().song_mut().tracks_mut().get_mut(y as usize) {
                                let track_uuid = track.uuid().to_string();
                                let riff_lengths = track.riffs().iter().map(|riff| (riff.uuid().to_string(), riff.length())).collect_vec();
                                for riff_ref in track.riff_refs_mut().iter_mut() {
                                    let riff_length = riff_lengths.iter().find(|riff_length_details| riff_length_details.0 == riff_ref.linked_to());
                                    if let Some((_, length)) = riff_length {
                                        if riff_ref.position() <= x && x <= (riff_ref.position() + length) {
                                            debug!("Riff ref selected: uuid={}, x={}, y={}, position={}, track={}, length={}", riff_ref.uuid().to_string(), x, y, riff_ref.position(), track_uuid.clone(), length);
                                            selected.push(riff_ref.uuid().to_string());
                                            break;
                                        }
                                    }
                                }
                            }

                            if !selected.is_empty() {
                                let mut state = state;
                                if !add_to_select {
                                    state.selected_track_grid_riff_references_mut().clear();
                                }
                                state.selected_track_grid_riff_references_mut().append(&mut selected);
                            } else {
                                state.selected_track_grid_riff_references_mut().clear();
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - TrackChangeType::RiffReferencesSelectSingle - could not get lock on state"),
                    }
                    gui.ui.track_drawing_area.queue_draw();
                }
                TrackChangeType::RiffReferencesDeselectMultiple(x, y, x2, y2) => {
                    debug!("Main - rx_ui processing loop - TrackChangeType::RiffReferencesDeselectMultiple: x={}, y={}, x2={}, y2={}", x, y, x2, y2);
                    match state.lock() {
                        Ok(state) => {
                            let mut selected = Vec::new();
                            let mut state = state;

                            for (index, track) in state.get_project().song_mut().tracks_mut().iter_mut().enumerate() {
                                let track_number = index as i32;
                                if y < track_number && track_number < y2 {
                                    let track_uuid = track.uuid().to_string();
                                    let riff_lengths = track.riffs().iter().map(|riff| (riff.uuid().to_string(), riff.length())).collect_vec();
                                    for riff_ref in track.riff_refs_mut().iter_mut() {
                                        let riff_length = riff_lengths.iter().find(|riff_length_details| riff_length_details.0 == riff_ref.linked_to());
                                        if let Some((_, length)) = riff_length {
                                            if x <= riff_ref.position() && (riff_ref.position() + length) <= x2 {
                                                debug!("Riff ref deselected: uuid={}, x={}, y={}, x2={}, y2={}, position={}, track={}, length={}", riff_ref.uuid().to_string(), x, y, x2, y2, riff_ref.position(), track_uuid.clone(), length);
                                                selected.push(riff_ref.uuid().to_string());
                                            }
                                        }
                                    }
                                }
                            }

                            if !selected.is_empty() {
                                let mut state = state;
                                state.selected_track_grid_riff_references_mut().retain(|riff_ref_id| !selected.contains(riff_ref_id));
                            } else {
                                state.selected_track_grid_riff_references_mut().clear();
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - RiffReferencesDeselectMultiple - could not get lock on state"),
                    }
                    gui.ui.track_drawing_area.queue_draw();
                }
                TrackChangeType::RiffReferencesDeselectSingle(x, y) => {
                    debug!("Main - rx_ui processing loop - TrackChangeType::RiffReferencesDeselectSingle: x={}, y={},", x, y);
                    match state.lock() {
                        Ok(state) => {
                            let mut selected = Vec::new();
                            let mut state = state;

                            if let Some(track) = state.get_project().song_mut().tracks_mut().get_mut(y as usize) {
                                let track_uuid = track.uuid().to_string();
                                let riff_lengths = track.riffs().iter().map(|riff| (riff.uuid().to_string(), riff.length())).collect_vec();
                                for riff_ref in track.riff_refs_mut().iter_mut() {
                                    let riff_length = riff_lengths.iter().find(|riff_length_details| riff_length_details.0 == riff_ref.linked_to());
                                    if let Some((_, length)) = riff_length {
                                        if riff_ref.position() <= x && x <= (riff_ref.position() + length) {
                                            debug!("Riff ref deselected: uuid={}, x={}, y={}, position={}, track={}, length={}", riff_ref.uuid().to_string(), x, y, riff_ref.position(), track_uuid.clone(), length);
                                            selected.push(riff_ref.uuid().to_string());
                                            break;
                                        }
                                    }
                                }
                            }

                            if !selected.is_empty() {
                                let mut state = state;
                                state.selected_track_grid_riff_references_mut().retain(|riff_ref_id| !selected.contains(riff_ref_id));
                            } else {
                                state.selected_track_grid_riff_references_mut().clear();
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - TrackChangeType::RiffReferencesDeselectSingle - could not get lock on state"),
                    }
                    gui.ui.track_drawing_area.queue_draw();
                }
                TrackChangeType::RiffReferencesSelectAll => {
                    debug!("Main - rx_ui processing loop - TrackChangeType::RiffReferencesSelectAll");
                    match state.lock() {
                        Ok(mut state) => {
                            let mut selected = Vec::new();
                            for track in state.get_project().song_mut().tracks_mut().iter_mut() {
                                for riff_ref in track.riff_refs_mut().iter_mut() {
                                    selected.push(riff_ref.uuid().to_string());
                                }
                            }

                            if !selected.is_empty() {
                                let mut state = state;
                                state.selected_track_grid_riff_references_mut().clear();
                                state.selected_track_grid_riff_references_mut().append(&mut selected);
                            } else {
                                state.selected_track_grid_riff_references_mut().clear();
                            }
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - RiffReferencesSelectAll - could not get lock on state"),
                    }
                    gui.ui.track_drawing_area.queue_draw();
                }
                TrackChangeType::RiffReferencesDeselectAll => {
                    debug!("Main - rx_ui processing loop - TrackChangeType::RiffReferencesDeselectAll");
                    match state.lock() {
                        Ok(mut state) => {
                            state.selected_track_grid_riff_references_mut().clear();
                        }
                        Err(_) => debug!("Main - rx_ui processing loop - RiffReferencesDeselectAll - could not get lock on state"),
                    }
                    gui.ui.track_drawing_area.queue_draw();

                }
                TrackChangeType::RiffReferenceIncrementRiff{track_index, position} => {
                    debug!("Main - rx_ui processing loop - TrackChangeType::RiffReferenceIncrementRiff: track_index={}, position={}", track_index, position);
                    match state.lock() {
                        Ok(mut state) => {
                            // get the track
                            let track_riff = if let Some(track) = state.get_project().song_mut().tracks_mut().get_mut(track_index as usize) {
                                let track_uuid = track.uuid().to_string();
                                let track_name = track.name().to_string();
                                let riff_ids = track.riffs_mut().iter_mut().map(|riff| (riff.id(), riff.name().to_string())).collect_vec();
                                let riff_details = track.riffs_mut().iter_mut().map(|riff| (riff.id(), (riff.name().to_string(), riff.length()))).collect::<HashMap<String, (String, f64)>>();

                                if let Some(riff_ref) = track.riff_refs_mut().iter_mut().find(|riff_ref| {
                                    if let Some((name, riff_length)) = riff_details.get(&riff_ref.linked_to()) {
                                        let riff_ref_end_position = riff_ref.position() + *riff_length;
                                        if riff_ref.position() <= position && position <= riff_ref_end_position {
                                            true
                                        }
                                        else { false }
                                    }
                                    else { false }
                                }) {
                                    if let Some(index) = riff_ids.iter().position(|(id, _)| id.clone() == riff_ref.linked_to()) {
                                        let next_index = if (index + 1) < riff_ids.iter().count() {
                                            index + 1
                                        }
                                        else { 0 };

                                        if let Some((riff_id, name)) = riff_ids.get(next_index) {
                                            riff_ref.set_linked_to(riff_id.clone());
                                            gui.ui.track_drawing_area.queue_draw();

                                            if let Some(riff) = track.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == riff_id.clone()) {
                                                scroll_notes_into_view(gui, riff);
                                            }

                                            Some((track_uuid, riff_id.clone(), track_name.to_string(), name.clone()))
                                        } else { None }
                                    } else { None }
                                } else { None }
                            }
                            else { None };

                            if let Some((track_uuid, riff_uuid, track_name, riff_name)) = track_riff {
                                state.set_selected_riff_uuid(track_uuid.clone(), riff_uuid);
                                state.set_selected_track(Some(track_uuid));
                                gui.set_piano_roll_selected_track_name_label(track_name.as_str());
                                gui.set_piano_roll_selected_riff_name_label(riff_name.as_str());

                                gui.ui.piano_roll_drawing_area.queue_draw();
                            }
                        }
                        Err(_) => debug!("Main - rx_ui processing loop - TrackChangeType::RiffReferenceIncrementRiff - could not get lock on state"),
                    }
                }
            }
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
                        let sample_rate = state.configuration.audio.sample_rate as f64;;
                        let play_position_in_frames = 0.0;
                        let play_position_in_beats = play_position_in_frames / sample_rate * bpm / 60.0;
                        let current_bar = play_position_in_beats as i32 / time_signature_numerator as i32 + 1;
                        let current_beat_in_bar = play_position_in_beats as i32 % time_signature_numerator as i32 + 1;

                        state.set_play_position_in_frames(play_position_in_frames as u32);

                        gui.ui.song_position_txt_ctrl.set_label(format!("{:03}:{:03}:000", current_bar, current_beat_in_bar).as_str());

                        let time_in_secs = play_position_in_frames / sample_rate;
                        let minutes = time_in_secs as i32 / 60;
                        let seconds = time_in_secs as i32 % 60;
                        let milli_seconds = ((time_in_secs - (time_in_secs as u64) as f64) * 1000.0) as u64;
                        gui.ui.song_time_txt_ctrl.set_label(format!("{:03}:{:02}:{:03}", minutes, seconds, milli_seconds).as_str());

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
                        let sample_rate = state.configuration.audio.sample_rate as f64;;
                        let block_size = state.configuration.audio.block_size as f64;
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

                        let time_in_secs = play_position_in_frames as f64 / sample_rate;
                        let minutes = time_in_secs as i32 / 60;
                        let seconds = time_in_secs as i32 % 60;
                        let milli_seconds = ((time_in_secs - (time_in_secs as u64) as f64) * 1000.0) as u64;
                        gui.ui.song_time_txt_ctrl.set_label(format!("{:03}:{:02}:{:03}", minutes, seconds, milli_seconds).as_str());

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
                match state.lock() {
                    Ok(mut state) => {
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
                        if let Some(_) = state.playing_riff_grid() {
                            gui.repaint_riff_grid_view_drawing_area(0.0);
                            state.set_playing_riff_grid(None);
                        }
                        if let Some(playing_riff_arrangement_uuid) = state.playing_riff_arrangement() {
                            let playing_riff_arrangement_summary_data = (0.0, vec![]);
                            gui.repaint_riff_arrangement_view_active_drawing_areas(playing_riff_arrangement_uuid, 0.0, &playing_riff_arrangement_summary_data);
                            state.set_playing_riff_arrangement(None);
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - transport stop - could not get lock on state"),
                };
                match state.lock() {
                    Ok(state) => {
                        let song = state.project().song();
                        let song_length_in_beats = song.length_in_beats() as f64;
                        let tracks = song.tracks();
                        let bpm = song.tempo();
                        let sample_rate = state.configuration.audio.sample_rate as f64;
                        let block_size = state.configuration.audio.block_size as f64;
                        for track in tracks {
                            state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Stop);
                        }
                let number_of_blocks = (song_length_in_beats / bpm * 60.0 * sample_rate / block_size) as i32;
                match tx_to_audio.send(AudioLayerInwardEvent::Play(false, number_of_blocks, 0)) {
                    Ok(_) => (),
                    Err(error) => debug!("Problem using tx_to_audio to send message to jack layer when stopping play: {}", error),
                }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - transport stop - could not get lock on state"),
                };
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
                                let riff_set_uuid = if let Some(playing_riff_set_uuid) = state.playing_riff_set() {
                                    playing_riff_set_uuid.to_string()
                                }
                                else if let Some(riff_set) = state.get_project().song_mut().riff_sets_mut().get_mut(0) {
                                    riff_set.uuid()
                                } else {
                                    "".to_string()
                                };
                                state.set_playing_riff_set(Some(riff_set_uuid.clone()));
                                state.play_riff_set(tx_to_audio, riff_set_uuid);
                            } else if riffs_stack_visible_name == "riff_sequences" {
                                let riff_sequence_uuid = if let Some(selected_riff_sequence_uuid) = state.selected_riff_sequence_uuid() {
                                    selected_riff_sequence_uuid.to_string()
                                }
                                else if let Some(riff_sequence) = state.get_project().song_mut().riff_sequences_mut().get_mut(0) {
                                    riff_sequence.uuid()
                                } else {
                                    "".to_string()
                                };
                                state.set_playing_riff_sequence(Some(riff_sequence_uuid.clone()));
                                state.play_riff_sequence(tx_to_audio, riff_sequence_uuid);
                            } else if riffs_stack_visible_name == "riff_grids" {
                                let riff_grid_uuid = if let Some(riff_grid_uuid) = state.selected_riff_grid_uuid() {
                                    riff_grid_uuid.to_string()
                                }
                                else if let Some(riff_grid) = state.get_project().song_mut().riff_grids_mut().get_mut(0) {
                                    riff_grid.uuid()
                                } else {
                                    "".to_string()
                                };
                                state.set_playing_riff_grid(Some(riff_grid_uuid.clone()));
                                state.play_riff_grid(tx_to_audio, riff_grid_uuid);
                            } else if riffs_stack_visible_name == "riff_arrangement" {
                                let riff_arrangement_uuid = if let Some(selected_riff_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                                    selected_riff_arrangement_uuid.to_string()
                                }
                                else if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangements_mut().get_mut(0) {
                                    riff_arrangement.uuid()
                                } else {
                                    "".to_string()
                                };
                                state.set_playing_riff_arrangement(Some(riff_arrangement_uuid.clone()));
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
                        let sample_rate = state.configuration.audio.sample_rate as f64;;
                        let block_size = state.configuration.audio.block_size as f64;
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

                        let time_in_secs = play_position_in_frames as f64 / sample_rate;
                        let minutes = time_in_secs as i32 / 60;
                        let seconds = time_in_secs as i32 % 60;
                        let milli_seconds = ((time_in_secs - (time_in_secs as u64) as f64) * 1000.0) as u64;
                        gui.ui.song_time_txt_ctrl.set_label(format!("{:03}:{:02}:{:03}", minutes, seconds, milli_seconds).as_str());

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
                        if let Some(automation_grid) = gui.automation_grid() {
                            if let Ok(mut grid) = automation_grid.lock() {
                                grid.set_tempo(state.project().song().tempo());
                            }
                        }
                        if let Some(riff_grid) = gui.riff_grid() {
                            if let Ok(mut grid) = riff_grid.lock() {
                                grid.set_tempo(state.project().song().tempo());
                            }
                        }

                        {
                            let mut time_info = vst_host_time_info.write();
                            time_info.sample_pos = 0.0;
                            time_info.sample_rate = state.configuration.audio.sample_rate as f64;
                            time_info.nanoseconds = 0.0;
                            time_info.ppq_pos = 0.0;
                            time_info.tempo = tempo;
                            time_info.bar_start_pos = 0.0;
                            time_info.cycle_start_pos = 0.0;
                            time_info.cycle_end_pos = 0.0;
                            time_info.time_sig_numerator = state.project().song().time_signature_numerator() as i32;
                            time_info.time_sig_denominator = state.project().song().time_signature_denominator() as i32;
                            time_info.smpte_offset = 0;
                            time_info.smpte_frame_rate = vst::api::SmpteFrameRate::Smpte24fps;
                            time_info.samples_to_next_clock = 0;
                            time_info.flags = 3;
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
            DAWEvents::TimeSignatureNumeratorChange(time_signature_numerator) => {
                match state.lock() {
                    Ok(mut state) => {
                        let denominator = state.get_project().song_mut().time_signature_denominator();
                        state.get_project().song_mut().set_time_signature_numerator(time_signature_numerator);
                        if let Some(track_grid) = gui.track_grid() {
                            if let Ok(mut grid) = track_grid.lock() {
                                grid.set_beats_per_bar(time_signature_numerator as i32);
                            }
                        }
                        if let Some(track_grid_ruler) = gui.track_grid_ruler() {
                            if let Ok(mut grid) = track_grid_ruler.lock() {
                                grid.set_beats_per_bar(time_signature_numerator as i32);
                            }
                        }
                        if let Some(piano_roll_grid) = gui.piano_roll_grid() {
                            if let Ok(mut grid) = piano_roll_grid.lock() {
                                grid.set_beats_per_bar(time_signature_numerator as i32);
                            }
                        }
                        if let Some(piano_roll_grid_ruler) = gui.piano_roll_grid_ruler() {
                            if let Ok(mut grid) = piano_roll_grid_ruler.lock() {
                                grid.set_beats_per_bar(time_signature_numerator as i32);
                            }
                        }
                        if let Some(automation_grid) = gui.automation_grid() {
                            if let Ok(mut grid) = automation_grid.lock() {
                                grid.set_beats_per_bar(time_signature_numerator as i32);
                            }
                        }
                        if let Some(automation_grid_ruler) = gui.automation_grid_ruler() {
                            if let Ok(mut grid) = automation_grid_ruler.lock() {
                                grid.set_beats_per_bar(time_signature_numerator as i32);
                            }
                        }
                        if let Some(riff_grid) = gui.riff_grid() {
                            if let Ok(mut grid) = riff_grid.lock() {
                                grid.set_beats_per_bar(time_signature_numerator as i32);
                            }
                        }
                        if let Some(riff_grid_ruler) = gui.riff_grid_ruler() {
                            if let Ok(mut grid) = riff_grid_ruler.lock() {
                                grid.set_beats_per_bar(time_signature_numerator as i32);
                            }
                        }

                        {
                            let mut time_info = vst_host_time_info.write();
                            time_info.time_sig_numerator = time_signature_numerator as i32;
                        }

                        for track in state.project().song().tracks().iter() {
                            state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::TimeSignatureChange(time_signature_numerator as u32, denominator as u32));
                        }

                        gui.ui.piano_roll_drawing_area.queue_draw();
                        gui.ui.piano_roll_ruler_drawing_area.queue_draw();
                        gui.ui.track_drawing_area.queue_draw();
                        gui.ui.track_ruler_drawing_area.queue_draw();
                        gui.ui.automation_drawing_area.queue_draw();
                        gui.ui.automation_ruler_drawing_area.queue_draw();
                        gui.ui.riff_grid_drawing_area.queue_draw();
                        gui.ui.riff_grid_ruler_drawing_area.queue_draw();
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - time signature numerator change - could not get lock on state"),
                };
            }
            DAWEvents::TimeSignatureDenominatorChange(time_signature_denominator) => {
                match state.lock() {
                    Ok(mut state) => {
                        let numerator = state.get_project().song_mut().time_signature_numerator();
                        state.get_project().song_mut().set_time_signature_denominator(time_signature_denominator);
                        if let Some(track_grid) = gui.track_grid() {
                            if let Ok(mut track) = track_grid.lock() {
                                // grid.set_tempo(time_signature_denominator);
                            }
                        }
                        if let Some(mut piano_roll_grid) = gui.piano_roll_grid() {
                            if let Ok(piano_roll) = piano_roll_grid.lock() {
                                // grid.set_tempo(time_signature_denominator);
                            }
                        }
                        if let Some(automation_grid) = gui.automation_grid() {
                            if let Ok(mut grid) = automation_grid.lock() {
                                // grid.set_tempo(time_signature_denominator);
                            }
                        }
                        if let Some(riff_grid) = gui.riff_grid() {
                            if let Ok(mut grid) = riff_grid.lock() {
                                // grid.set_tempo(time_signature_denominator);
                            }
                        }

                        {
                            let mut time_info = vst_host_time_info.write();
                             time_info.time_sig_denominator = time_signature_denominator as i32;
                        }

                        for track in state.project().song().tracks().iter() {
                            state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::TimeSignatureChange(numerator as u32, time_signature_denominator as u32));
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - time signature denominator change - could not get lock on state"),
                };
            }
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
                        let sample_rate = state.configuration.audio.sample_rate as f64;
                        let block_size = state.configuration.audio.block_size as f64;
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
                                match *(riff_item.item_type()) {
                                    RiffItemType::RiffSet => {
                                        if let Some(riff_set) = state.project().song().riff_set(riff_item.item_uuid().to_string()) {
                                            if riff_set.uuid() == uuid {
                                                let message = format!("Riff arrangement: \"{}\" has references to riff set: \"{}\".", riff_arrangement.name(), riff_set.name());

                                                if !found_info.iter().any(|entry| *entry == message) {
                                                    found_info.push(message);
                                                }
                                            }
                                        }
                                    }
                                    RiffItemType::RiffSequence => {
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
                                    _ => {}
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
                        let new_riff_set_name = state.riff_set_increment_riff_for_track(riff_set_uuid.clone(), track_uuid.clone());
                        gui.update_available_riff_sets(&state);
                        gui.update_riff_set_name_in_riff_views(riff_set_uuid.clone(), new_riff_set_name);
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
            DAWEvents::RiffSequenceCopy(uuid) => {
                if let Ok(mut state) = state.lock() {
                    if let Some(riff_sequence) = state.get_project().song_mut().riff_sequence(uuid) {
                        let mut new_riff_sequence = riff_sequence.clone();
                        let new_name = format!("Copy of {}", new_riff_sequence.name());

                        new_riff_sequence.set_name(new_name);
                        new_riff_sequence.set_uuid(Uuid::new_v4());

                        state.set_selected_riff_sequence_uuid(Some(new_riff_sequence.uuid()));
                        state.get_project().song_mut().add_riff_sequence(new_riff_sequence);
                    }
                }
                let _ = tx_from_ui.send(DAWEvents::UpdateUI);
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
                            gui.update_riff_sequences_combobox_in_riff_sequence_view(&state, false);
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
                            gui.update_riff_sequences_combobox_in_riff_sequence_view(&mut state, true);
                            gui.update_available_riff_sequences_in_riff_arrangement_blades(&state);
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - riff sequence name change - could not get lock on state"),
                };
                gui.ui.riff_sequences_box.queue_draw();
            }
            DAWEvents::RiffSequenceSelected(riff_sequence_uuid) => {
                match state.lock() {
                    Ok(mut state) => {
                        state.set_selected_riff_sequence_uuid(Some(riff_sequence_uuid));
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - riff sequence selected - could not get lock on state"),
                };
            }
            DAWEvents::RiffSequenceRiffSetAdd(riff_sequence_uuid, riff_set_uuid, riff_set_reference_uuid) => {
                debug!("Main - rx_ui processing loop - riff sequence - riff set add: {}, {}", riff_sequence_uuid.as_str(), riff_set_uuid.as_str());
                let state_arc = state.clone();
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
                            if let Some(riff_sequence) = state.get_project().song_mut().riff_sequence_mut(riff_sequence_uuid.clone()) {
                                riff_sequence.add_riff_set_at_position(riff_set_reference_uuid, riff_set_uuid.clone(), selected_riff_set_position + 1);
                            }
                        }
                        else {
                            if let Some(riff_sequence) = state.get_project().song_mut().riff_sequence_mut(riff_sequence_uuid.clone()) {
                                riff_sequence.add_riff_set(riff_set_reference_uuid, riff_set_uuid.clone());
                            }
                        }
                        let riff_set_name = if let Some(riff_set) = state.project().song().riff_sets().iter().find(|riff_set| riff_set.uuid() == riff_set_uuid.clone()) {
                            riff_set.name().to_string()
                        }
                        else {
                            "".to_string()
                        };
                        let track_uuids: Vec<String> = state.project().song().tracks().iter().map(|track| track.uuid().to_string()).collect();
                        gui.add_riff_sequence_riff_set_blade(
                            tx_from_ui,
                            riff_sequence_uuid,
                            riff_set_reference_uuid.to_string(),
                            riff_set_uuid,
                            track_uuids,
                            gui.selected_style_provider.clone(),
                            riff_set_name,
                            state_arc,
                        );
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
            DAWEvents::RiffGridAdd(riff_grid_uuid, name) => {
                match state.lock() {
                    Ok(mut state) => {
                        let mut riff_grid = RiffGrid::new_with_uuid(Uuid::parse_str(riff_grid_uuid.as_str()).unwrap());
                        riff_grid.set_name(name);
                        state.get_project().song_mut().add_riff_grid(riff_grid);
                        gui.update_available_riff_grids_in_riff_arrangement_blades(&state);
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - riff grid add - could not get lock on state"),
                };
                gui.ui.riff_grid_box.queue_draw();
            }
            DAWEvents::RiffGridDelete(riff_grid_uuid) => {
                // check if any riff grids or arrangements are using this riff - if so then show a warning dialog
                let found_info = match state.lock() {
                    Ok(state) => {
                        let mut found_info = vec![];

                        // check riff arrangements
                        for riff_arrangement in state.project().song().riff_arrangements().iter() {
                            for riff_item in riff_arrangement.items().iter() {
                                if let Some(riff_grid) = state.project().song().riff_grid(riff_item.item_uuid().to_string()) {
                                    if riff_grid.uuid() == riff_grid_uuid {
                                        let message = format!("Riff arrangement: \"{}\" has references to riff grid: \"{}\".", riff_arrangement.name(), riff_grid.name());

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
                        debug!("Main - rx_ui processing loop - riff grid delete - could not get lock on state");
                        vec![]
                    }
                };

                // if the riff grid is not used then delete it from the project/song
                if found_info.len() == 0 {
                    match state.lock() {
                        Ok(mut state) => {
                            // remove the riff grid from the song
                            state.get_project().song_mut().remove_riff_grid(riff_grid_uuid.clone());
                            // remove the riff grid from arrangement riff grid pick list
                            gui.update_available_riff_grids_in_riff_arrangement_blades(&state);
                            // remove the riff grid from the grid combobox in the riff grid view
                            gui.update_riff_grids_combobox_in_riff_grid_view(&state, false);
                        },
                        Err(_) => debug!("Main - rx_ui processing loop - riff grid delete - could not get lock on state"),
                    };
                    gui.ui.riff_grid_box.queue_draw();
                } else {
                    let mut error_message = String::from("Could not delete riff grid:\n");

                    for message in found_info.iter() {
                        error_message.push_str(message.as_str());
                        error_message.push_str("\n");
                    }

                    let _ = tx_from_ui.send(DAWEvents::Notification(NotificationType::Error, error_message));
                }
            }
            DAWEvents::RiffGridSelected(riff_grid_uuid) => {
                match state.lock() {
                    Ok(mut state) => {
                        state.set_selected_riff_grid_uuid(Some(riff_grid_uuid.clone()));
                        if let Some(riff_grid) = state.project().song().riff_grid(riff_grid_uuid) {
                            gui.ui.selected_riff_grid_name_entry.set_text(riff_grid.name());
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - riff grid selected - could not get lock on state"),
                };
                gui.ui.riff_grid_drawing_area.queue_draw();
            }
            DAWEvents::RiffGridChange(riff_grid_change_type, track_uuid) => {
                match riff_grid_change_type {
                    RiffGridChangeType::RiffReferenceAdd{ track_index, position } => {
                        match state.lock() {
                            Ok(mut state) => {
                                let mut selected_riff_uuid = None;
                                let mut track_uuid = None;

                                match state.project().song().tracks().get(track_index as usize) {
                                    Some(track) => {
                                        selected_riff_uuid = state.selected_riff_uuid(track.uuid().to_string());
                                        track_uuid = Some(track.uuid().to_string());
                                    }
                                    None => debug!("Main - rx_ui processing loop - riff grid riff reference added - no track at index."),
                                }

                                let selected_riff_grid_uuid = state.selected_riff_grid_uuid().clone();
                                if let Some(selected_riff_grid_uuid) = selected_riff_grid_uuid {
                                    if let Some(track_uuid) = track_uuid {
                                        if let Some(selected_riff_uuid) = selected_riff_uuid {
                                            match state.get_project().song_mut().riff_grids_mut().iter_mut().find(|riff_grid| riff_grid.uuid().to_string() == selected_riff_grid_uuid.to_string()) {
                                                Some(riff_grid) => {
                                                    riff_grid.add_riff_reference_to_track(track_uuid, selected_riff_uuid.clone(), position);
                                                }
                                                None => debug!("Main - rx_ui processing loop - riff grid riff reference added - no riff grid with uuid."),
                                            }
                                        }
                                    }
                                }
                            },
                            Err(_) => debug!("Main - rx_ui processing loop - riff grid riff reference added - could not get lock on state"),
                        }
                        gui.ui.riff_grid_drawing_area.queue_draw();
                    }
                    RiffGridChangeType::RiffReferenceDelete{track_index, position} => {
                        match state.lock() {
                            Ok(mut state) => {
                                let mut track_uuid = None;
                                let mut track_riffs = vec![];

                                match state.project().song().tracks().get(track_index as usize) {
                                    Some(track) => {
                                        track_uuid = Some(track.uuid().to_string());
                                        track_riffs = track.riffs().iter().map(|riff| (riff.id(), riff.length())).collect_vec();
                                    }
                                    None => debug!("Main - rx_ui processing loop - riff grid riff reference deleted - no track at index."),
                                }

                                if let Some(selected_riff_grid_uuid) = state.selected_riff_grid_uuid().clone() {
                                    if let Some(track_uuid) = track_uuid {
                                        match state.get_project().song_mut().riff_grids_mut().iter_mut().find(|riff_grid| riff_grid.uuid().to_string() == selected_riff_grid_uuid.to_string()) {
                                            Some(riff_grid) => {
                                                if let Some(riff_references) = riff_grid.track_riff_references_mut(track_uuid.clone()) {
                                                    riff_references.retain(|riff_ref| {
                                                        let riff_uuid = riff_ref.linked_to();
                                                        let mut retain = true;
                                                        for riff in track_riffs.iter() {
                                                            if riff.0 == riff_uuid {
                                                                let riff_length = riff.1;
                                                                if riff_ref.position() <= position &&
                                                                    position <= (riff_ref.position() + riff_length) {
                                                                    retain = false;
                                                                } else {
                                                                    retain = true;
                                                                }
                                                                break;
                                                            }
                                                        }
                                                        retain
                                                    });
                                                }
                                            }
                                            None => debug!("Main - rx_ui processing loop - riff grid riff reference deleted - no riff grid with uuid."),
                                        }
                                    }
                                }
                            },
                            Err(_) => debug!("Main - rx_ui processing loop - riff grid riff reference deleted - could not get lock on state"),
                        }
                        gui.ui.riff_grid_drawing_area.queue_draw();
                    }
                    RiffGridChangeType::RiffReferenceCutSelected => {
                        match state.lock() {
                            Ok(mut state) => {
                                let selected_riff_references = state.selected_riff_grid_riff_references().clone();
                                let selected_riff_grid_uuid = if let Some(selected_riff_grid_uuid) = state.selected_riff_grid_uuid() {
                                    selected_riff_grid_uuid.clone()
                                }
                                else {
                                    "".to_string()
                                };
                                let edit_cursor_position_in_secs = if let Some(riff_grid_beat_grid) = gui.riff_grid() {
                                    match riff_grid_beat_grid.lock() {
                                        Ok(grid) => {
                                            grid.edit_cursor_time_in_beats()
                                        },
                                        Err(_) => 0.0,
                                    }
                                } else {
                                    0.0
                                };
                                let mut copy_buffer: Vec<RiffReference> = vec![];

                                if let Some(riff_grid) = state.get_project().song_mut().riff_grid_mut(selected_riff_grid_uuid) {
                                    let track_uuids = riff_grid.tracks().map(|key| key.clone()).collect_vec();
                                    for track_uuid in track_uuids {
                                        if let Some(track_riff_refs) = riff_grid.track_riff_references_mut(track_uuid.clone()) {
                                            track_riff_refs.retain(|riff_ref| {
                                                if selected_riff_references.clone().contains(&riff_ref.uuid().to_string()) {
                                                    let mut value = riff_ref.clone();
                                                    value.set_position(value.position() - edit_cursor_position_in_secs);
                                                    value.set_track_id(track_uuid.clone());
                                                    copy_buffer.push(value);
                                                    false
                                                } else { true }
                                            });
                                        }
                                    }

                                    gui.ui.riff_grid_drawing_area.queue_draw();
                                }

                                state.riff_grid_riff_references_copy_buffer_mut().clear();
                                for riff_ref in copy_buffer.iter() {
                                    state.riff_grid_riff_references_copy_buffer_mut().push(riff_ref.clone());
                                }
                            }
                            Err(_) => debug!("Main - rx_ui processing loop - riff grid riff reference cut - could not get lock on state"),
                        }
                    }
                    RiffGridChangeType::RiffReferenceCopySelected => {
                        match state.lock() {
                            Ok(mut state) => {
                                let selected_riff_references = state.selected_riff_grid_riff_references().clone();
                                let selected_riff_grid_uuid = if let Some(selected_riff_grid_uuid) = state.selected_riff_grid_uuid() {
                                    selected_riff_grid_uuid.clone()
                                }
                                else {
                                    "".to_string()
                                };
                                let edit_cursor_position_in_secs = if let Some(riff_grid_beat_grid) = gui.riff_grid() {
                                    match riff_grid_beat_grid.lock() {
                                        Ok(grid) => {
                                            grid.edit_cursor_time_in_beats()
                                        },
                                        Err(_) => 0.0,
                                    }
                                } else {
                                    0.0
                                };
                                let mut copy_buffer: Vec<RiffReference> = vec![];

                                if let Some(riff_grid) = state.get_project().song_mut().riff_grid_mut(selected_riff_grid_uuid) {
                                    let track_uuids = riff_grid.tracks().map(|key| key.clone()).collect_vec();
                                    for track_uuid in track_uuids {
                                        if let Some(track_riff_refs) = riff_grid.track_riff_references_mut(track_uuid.clone()) {
                                            track_riff_refs.iter().filter(|riff_ref| selected_riff_references.clone().contains(&riff_ref.uuid().to_string())).for_each(|riff_ref| {
                                                let mut value = riff_ref.clone();
                                                value.set_position(value.position() - edit_cursor_position_in_secs);
                                                value.set_track_id(track_uuid.clone());
                                                copy_buffer.push(value);
                                            });
                                        }
                                    }

                                    gui.ui.riff_grid_drawing_area.queue_draw();
                                }

                                state.riff_grid_riff_references_copy_buffer_mut().clear();
                                for riff_ref in copy_buffer.iter() {
                                    state.riff_grid_riff_references_copy_buffer_mut().push(riff_ref.clone());
                                }
                            }
                            Err(_) => debug!("Main - rx_ui processing loop - riff grid riff reference copy - could not get lock on state"),
                        }
                    }
                    RiffGridChangeType::RiffReferencePaste => {
                        match state.lock() {
                            Ok(mut state) => {
                                let selected_riff_grid_uuid = if let Some(selected_riff_grid_uuid) = state.selected_riff_grid_uuid() {
                                    selected_riff_grid_uuid.clone()
                                }
                                else {
                                    "".to_string()
                                };
                                let edit_cursor_position_in_secs = if let Some(riff_grid_beat_grid) = gui.riff_grid() {
                                    match riff_grid_beat_grid.lock() {
                                        Ok(grid) => {
                                            grid.edit_cursor_time_in_beats()
                                        },
                                        Err(_) => 0.0,
                                    }
                                } else {
                                    0.0
                                };
                                let mut copy_buffer: Vec<RiffReference> = vec![];
                                state.riff_grid_riff_references_copy_buffer().iter().for_each(|riff_ref| copy_buffer.push(riff_ref.clone()));
                                let mut state = state;

                                if let Some(riff_grid) = state.get_project().song_mut().riff_grid_mut(selected_riff_grid_uuid) {
                                    let track_uuids = riff_grid.tracks().map(|key| key.clone()).collect_vec();
                                    for track_uuid in track_uuids {
                                        if let Some(track_riff_refs) = riff_grid.track_riff_references_mut(track_uuid.clone()) {
                                            let mut copy_buffer_riff_refs_to_remove = vec![];
                                            for riff_ref in copy_buffer.iter() {
                                                if track_uuid == riff_ref.track_id() {
                                                    track_riff_refs.push(RiffReference::new(riff_ref.linked_to(), riff_ref.position() + edit_cursor_position_in_secs));
                                                    copy_buffer_riff_refs_to_remove.push(riff_ref.uuid().to_string());
                                                }
                                            }
                                            copy_buffer.retain(|riff_ref| !copy_buffer_riff_refs_to_remove.contains(&riff_ref.uuid().to_string()));
                                        }
                                    }

                                    gui.ui.riff_grid_drawing_area.queue_draw();
                                }
                            }
                            Err(_) => debug!("Main - rx_ui processing loop - riff grid riff reference paste - could not get lock on state"),
                        }
                    }
                    RiffGridChangeType::RiffReferenceChange(change) => {
                        debug!("Main - rx_ui processing loop - riff grid riff reference change.");
                        // just interested in position changes - the changed riff actually refers to riff reference by uuid
                        match state.lock() {
                            Ok(mut state) => {
                                let mut snap_position_in_beats = 1.0;
                                let selected_riff_grid_uuid = if let Some(selected_riff_grid_uuid) = state.selected_riff_grid_uuid() {
                                    selected_riff_grid_uuid.clone()
                                }
                                else {
                                    "".to_string()
                                };
                                match gui.riff_grid() {
                                    Some(riff_grid) => match riff_grid.lock() {
                                        Ok(grid) => snap_position_in_beats = grid.snap_position_in_beats(),
                                        Err(_) => (),
                                    },
                                    None => (),
                                }

                                let mut riff_id = "".to_string();
                                let mut track_id = "".to_string();
                                if let Some(riff_grid) = state.get_project().song_mut().riff_grid_mut(selected_riff_grid_uuid) {
                                    let track_uuids = { riff_grid.tracks().map(|key| key.to_string()).collect_vec() };
                                    for track_uuid in track_uuids {
                                        for (_, changed_riff) in change.iter() {
                                            for riff_refs in riff_grid.track_riff_references_mut(track_uuid.to_string()) {

                                                if let Some(riff_ref) = riff_refs.iter_mut().find(|riff_ref| riff_ref.uuid().to_string() == changed_riff.uuid().to_string()) {
                                                    let delta = riff_ref.position() - changed_riff.position();

                                                    track_id = track_uuid.clone();
                                                    riff_id = riff_ref.linked_to();

                                                    if delta < -0.000001 || delta > 0.000001 {
                                                        let calculated_value = DAWUtils::quantise(changed_riff.position(), snap_position_in_beats, 1.0, false);
                                                        if calculated_value.snapped {
                                                            riff_ref.set_position(calculated_value.snapped_value);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                gui.ui.riff_grid_drawing_area.queue_draw();
                            }
                            Err(_) => debug!("Main - rx_ui processing loop - riff grid - riff reference change - could not get lock on state"),
                        }
                    }
                    RiffGridChangeType::RiffReferenceDragCopy(mut new_riff_references_details) => {
                        match state.lock() {
                            Ok(mut state) => {
                                let mut snap_position_in_beats = 1.0;
                                match gui.riff_grid() {
                                    Some(riff_grid) => match riff_grid.lock() {
                                        Ok(grid) => snap_position_in_beats = grid.snap_position_in_beats(),
                                        Err(_) => (),
                                    },
                                    None => (),
                                }

                                // get the selected riff grid
                                if let Some(selected_riff_grid_uuid) = state.selected_riff_grid_uuid().clone() {
                                    if let Some(riff_grid) = state.get_project().song_mut().riff_grid_mut(selected_riff_grid_uuid) {
                                        let track_uuids = { riff_grid.tracks().map(|key| key.to_string()).collect_vec() };
                                        for track_uuid in track_uuids {
                                            // get the original riff ref linked to value
                                            if let Some(riff_references) = riff_grid.track_riff_references_mut(track_uuid.clone()) {
                                                let mut unused_changes = vec![];
                                                for (position, original_riff_ref_uuid) in new_riff_references_details.iter() {
                                                    let linked_to = if let Some(original_riff_ref) = riff_references.iter_mut().find(|riff_ref| riff_ref.id() == original_riff_ref_uuid.clone()) {
                                                        Some(original_riff_ref.linked_to())
                                                    } else {
                                                        None
                                                    };
                                                    if let Some(linked_to) = linked_to {
                                                        let snap_delta = position % snap_position_in_beats;
                                                        let new_position = position - snap_delta;
                                                        if new_position >= 0.0 {
                                                            let riff_ref = RiffReference::new(linked_to, new_position);
                                                            riff_references.push(riff_ref);
                                                        }
                                                    }
                                                    else {
                                                        unused_changes.push((*position, original_riff_ref_uuid.clone()));
                                                    }
                                                }

                                                new_riff_references_details.clear();
                                                new_riff_references_details.append(&mut unused_changes);
                                            }
                                        }
                                    }
                                }
                            }
                            Err(_) => debug!("Main - rx_ui processing loop - add new riff reference to riff grid track - could not get lock on state"),
                        }
                        gui.ui.riff_grid_drawing_area.queue_draw();
                    }
                    RiffGridChangeType::RiffReferencesSelectMultiple(x1, y1, x2, y2, add_to_select) => {
                        debug!("Main - rx_ui processing loop - RiffGridChangeType::RiffReferencesSelectMultiple: x1={}, y1={}, x2={}, y2={}, add_to_select={}", x1, y1, x2, y2, add_to_select);
                        let mut selected = Vec::new();
                        match state.lock() {
                            Ok(state) => {
                                let mut state = state;
                                // get the selected riff grid
                                if let Some(selected_riff_grid_uuid) = state.selected_riff_grid_uuid().clone() {
                                    let mut riff_lengths = HashMap::new();
                                    let mut track_uuids = vec![];
                                    for track in state.project().song().tracks().iter() {
                                        track_uuids.push(track.uuid().to_string());
                                        for riff in track.riffs().iter() {
                                            riff_lengths.insert(riff.uuid().to_string(), riff.length());
                                        }
                                    }

                                    if let Some(riff_grid) = state.get_project().song_mut().riff_grid_mut(selected_riff_grid_uuid) {
                                        for (index, track_uuid) in track_uuids.iter().enumerate() {
                                            let track_number = index as i32;
                                            if y1 < track_number && track_number < y2 {
                                                if let Some(riff_references) = riff_grid.track_riff_references_mut(track_uuid.clone()) {
                                                    for riff_ref in riff_references.iter_mut() {
                                                        if let Some(riff_length) = riff_lengths.get(&riff_ref.linked_to()) {
                                                            if x1 <= riff_ref.position() && (riff_ref.position() + riff_length) <= x2 {
                                                                debug!("Riff grid - Riff ref selected: x1={}, y1={}, x2={}, y2={}, position={}, track={}, length={}", x1, y1, x2, y2, riff_ref.position(), track_uuid.as_str(), riff_length);
                                                                selected.push(riff_ref.uuid().to_string());
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                if !selected.is_empty() {
                                    let mut state = state;
                                    if !add_to_select {
                                        state.selected_riff_grid_riff_references_mut().clear();
                                    }
                                    state.selected_riff_grid_riff_references_mut().append(&mut selected);
                                }
                                else {
                                    state.selected_riff_grid_riff_references_mut().clear();
                                }
                            },
                            Err(_) => debug!("Main - rx_ui processing loop - riff grid riff references select multiple - could not get lock on state"),
                        }
                        gui.ui.riff_grid_drawing_area.queue_draw();
                    }
                    RiffGridChangeType::RiffReferencesSelectSingle(x1, y1, add_to_select) => {
                        debug!("Main - rx_ui processing loop - RiffGridChangeType::RiffReferencesSelectSingle: x1={}, y1={}, add_to_select={}", x1, y1, add_to_select);
                        let mut selected = Vec::new();
                        match state.lock() {
                            Ok(state) => {
                                let mut state = state;
                                // get the selected riff grid
                                if let Some(selected_riff_grid_uuid) = state.selected_riff_grid_uuid().clone() {
                                    let mut riff_lengths = HashMap::new();
                                    let mut track_uuids = vec![];
                                    for track in state.project().song().tracks().iter() {
                                        track_uuids.push(track.uuid().to_string());
                                        for riff in track.riffs().iter() {
                                            riff_lengths.insert(riff.uuid().to_string(), riff.length());
                                        }
                                    }

                                    if let Some(riff_grid) = state.get_project().song_mut().riff_grid_mut(selected_riff_grid_uuid) {
                                        if let Some(track_uuid) = track_uuids.get(y1 as usize) {
                                            if let Some(riff_references) = riff_grid.track_riff_references_mut(track_uuid.clone()) {
                                                for riff_ref in riff_references.iter_mut() {
                                                    if let Some(riff_length) = riff_lengths.get(&riff_ref.linked_to()) {
                                                        if riff_ref.position() <= x1 && x1 <= (riff_ref.position() + riff_length) {
                                                            debug!("Riff grid - Riff ref select single: x1={}, y1={}, position={}, track={}, length={}", x1, y1, riff_ref.position(), track_uuid.as_str(), riff_length);
                                                            selected.push(riff_ref.uuid().to_string());
                                                            break;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                if !selected.is_empty() {
                                    let mut state = state;
                                    if !add_to_select {
                                        state.selected_riff_grid_riff_references_mut().clear();
                                    }
                                    state.selected_riff_grid_riff_references_mut().append(&mut selected);
                                }
                                else {
                                    state.selected_riff_grid_riff_references_mut().clear();
                                }
                            },
                            Err(_) => debug!("Main - rx_ui processing loop - riff grid riff references select single - could not get lock on state"),
                        }
                        gui.ui.riff_grid_drawing_area.queue_draw();
                    }
                    RiffGridChangeType::RiffReferencesDeselectMultiple(x1, y1, x2, y2) => {
                        debug!("Main - rx_ui processing loop - RiffGridChangeType::RiffReferencesDeselectMultiple: x1={}, y1={}, x2={}, y2={}", x1, y1, x2, y2);
                        let mut selected = Vec::new();
                        match state.lock() {
                            Ok(state) => {
                                let mut state = state;
                                // get the selected riff grid
                                if let Some(selected_riff_grid_uuid) = state.selected_riff_grid_uuid().clone() {
                                    let mut riff_lengths = HashMap::new();
                                    let mut track_uuids = vec![];
                                    for track in state.project().song().tracks().iter() {
                                        track_uuids.push(track.uuid().to_string());
                                        for riff in track.riffs().iter() {
                                            riff_lengths.insert(riff.uuid().to_string(), riff.length());
                                        }
                                    }

                                    if let Some(riff_grid) = state.get_project().song_mut().riff_grid_mut(selected_riff_grid_uuid) {
                                        for (index, track_uuid) in track_uuids.iter().enumerate() {
                                            let track_number = index as i32;
                                            if y1 < track_number && track_number < y2 {
                                                if let Some(riff_references) = riff_grid.track_riff_references_mut(track_uuid.clone()) {
                                                    for riff_ref in riff_references.iter_mut() {
                                                        if let Some(riff_length) = riff_lengths.get(&riff_ref.linked_to()) {
                                                            if x1 <= riff_ref.position() && (riff_ref.position() + riff_length) <= x2 {
                                                                debug!("Riff grid - Riff ref deselected: x1={}, y1={}, x2={}, y2={}, position={}, track={}, length={}", x1, y1, x2, y2, riff_ref.position(), track_uuid.as_str(), riff_length);
                                                                selected.push(riff_ref.uuid().to_string());
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                if !selected.is_empty() {
                                    let mut state = state;
                                    state.selected_riff_grid_riff_references_mut().retain(|riff_ref_id| !selected.contains(riff_ref_id));
                                }
                                else {
                                    state.selected_riff_grid_riff_references_mut().clear();
                                }
                            },
                            Err(_) => debug!("Main - rx_ui processing loop - riff grid riff references deselect multiple - could not get lock on state"),
                        }
                        gui.ui.riff_grid_drawing_area.queue_draw();
                    }
                    RiffGridChangeType::RiffReferencesDeselectSingle(x1, y1) => {
                        debug!("Main - rx_ui processing loop - RiffGridChangeType::RiffReferencesDeselectSingle: x1={}, y1={}", x1, y1);
                        let mut selected = Vec::new();
                        match state.lock() {
                            Ok(mut state) => {
                                // get the selected riff grid
                                if let Some(selected_riff_grid_uuid) = state.selected_riff_grid_uuid().clone() {
                                    let mut riff_lengths = HashMap::new();
                                    let mut track_uuids = vec![];
                                    for track in state.project().song().tracks().iter() {
                                        track_uuids.push(track.uuid().to_string());
                                        for riff in track.riffs().iter() {
                                            riff_lengths.insert(riff.uuid().to_string(), riff.length());
                                        }
                                    }

                                    if let Some(riff_grid) = state.get_project().song_mut().riff_grid_mut(selected_riff_grid_uuid) {
                                        if let Some(track_uuid) = track_uuids.get(y1 as usize) {
                                            if let Some(riff_references) = riff_grid.track_riff_references_mut(track_uuid.clone()) {
                                                for riff_ref in riff_references.iter_mut() {
                                                    if let Some(riff_length) = riff_lengths.get(&riff_ref.linked_to()) {
                                                        if riff_ref.position() <= x1 && x1 <= (riff_ref.position() + riff_length) {
                                                            debug!("Riff grid - Riff ref select single: x1={}, y1={}, position={}, track={}, length={}", x1, y1, riff_ref.position(), track_uuid.as_str(), riff_length);
                                                            selected.push(riff_ref.uuid().to_string());
                                                            break;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                if !selected.is_empty() {
                                    let mut state = state;
                                    state.selected_riff_grid_riff_references_mut().retain(|riff_ref_id| !selected.contains(riff_ref_id));
                                }
                                else {
                                    state.selected_riff_grid_riff_references_mut().clear();
                                }
                            }
                            Err(_) => debug!("Main - rx_ui processing loop - riff grid riff references deselect single - could not get lock on state"),
                        }
                        gui.ui.riff_grid_drawing_area.queue_draw();
                    }
                    RiffGridChangeType::RiffReferencesSelectAll => {
                        debug!("Main - rx_ui processing loop - RiffGridChangeType::RiffReferencesSelectAll");
                        let mut selected = Vec::new();
                        match state.lock() {
                            Ok(state) => {
                                let mut state = state;
                                // get the selected riff grid
                                if let Some(selected_riff_grid_uuid) = state.selected_riff_grid_uuid().clone() {
                                    let mut riff_lengths = HashMap::new();
                                    let mut track_uuids = vec![];
                                    for track in state.project().song().tracks().iter() {
                                        track_uuids.push(track.uuid().to_string());
                                        for riff in track.riffs().iter() {
                                            riff_lengths.insert(riff.uuid().to_string(), riff.length());
                                        }
                                    }

                                    if let Some(riff_grid) = state.get_project().song_mut().riff_grid_mut(selected_riff_grid_uuid) {
                                        for track_uuid in track_uuids.iter() {
                                            if let Some(riff_references) = riff_grid.track_riff_references_mut(track_uuid.clone()) {
                                                for riff_ref in riff_references.iter_mut() {
                                                    selected.push(riff_ref.uuid().to_string());
                                                }
                                            }
                                        }
                                    }
                                }

                                if !selected.is_empty() {
                                    let mut state = state;
                                    state.selected_riff_grid_riff_references_mut().clear();
                                    state.selected_riff_grid_riff_references_mut().append(&mut selected);
                                }
                                else {
                                    state.selected_riff_grid_riff_references_mut().clear();
                                }
                            },
                            Err(_) => debug!("Main - rx_ui processing loop - riff grid riff references select all - could not get lock on state"),
                        }
                        gui.ui.riff_grid_drawing_area.queue_draw();
                    }
                    RiffGridChangeType::RiffReferencesDeselectAll => {
                        debug!("Main - rx_ui processing loop - RiffGridChangeType::RiffReferencesDeselectAll");
                        match state.lock() {
                            Ok(mut state) => {
                                state.selected_riff_grid_riff_references_mut().clear();
                                gui.ui.riff_grid_drawing_area.queue_draw();
                            }
                            Err(_) => debug!("Main - rx_ui processing loop - riff grid riff references deselect all - could not get lock on state"),
                        }
                    }
                    RiffGridChangeType::RiffReferenceIncrementRiff { track_index, position } => {
                        debug!("Main - rx_ui processing loop - RiffGridChangeType::RiffReferenceIncrementRiff: track_index={}, position={}", track_index, position);
                        match state.lock() {
                            Ok(mut state) => {
                                let selected_riff_grid_uuid = state.selected_riff_grid_uuid().clone();

                                // get the track
                                let track_riff = if let Some(track) = state.get_project().song_mut().tracks_mut().get_mut(track_index as usize) {
                                    let track_uuid = track.uuid().to_string();
                                    let track_name = track.name().to_string();
                                    let riff_ids = track.riffs_mut().iter_mut().map(|riff| (riff.id(), riff.name().to_string())).collect_vec();
                                    let riff_details = track.riffs_mut().iter_mut().map(|riff| (riff.id(), (riff.name().to_string(), riff.length()))).collect::<HashMap<String, (String, f64)>>();
                                    let mut riff_name = None;

                                    // need to use the selected riff grid
                                    let track_riff = if let Some(selected_riff_grid_uuid) = selected_riff_grid_uuid {
                                        // find the riff grid
                                        if let Some(riff_grid) = state.get_project().song_mut().riff_grid_mut(selected_riff_grid_uuid) {
                                            if let Some(riff_grid_track_riff_refs) = riff_grid.track_riff_references_mut(track_uuid.clone()) {
                                                if let Some(riff_ref) = riff_grid_track_riff_refs.iter_mut().find(|riff_ref| {
                                                    if let Some((name, riff_length)) = riff_details.get(&riff_ref.linked_to()) {
                                                        riff_name = Some(name.to_string());
                                                        let riff_ref_end_position = riff_ref.position() + *riff_length;
                                                        if riff_ref.position() <= position && position <= riff_ref_end_position {
                                                            true
                                                        }
                                                        else { false }
                                                    }
                                                    else { false }
                                                }) {
                                                    if let Some(index) = riff_ids.iter().position(|(id, _)| id.clone() == riff_ref.linked_to()) {
                                                        let next_index = if (index + 1) < riff_ids.iter().count() {
                                                            index + 1
                                                        }
                                                        else { 0 };

                                                        if let Some((riff_id, riff_name)) = riff_ids.get(next_index) {
                                                            riff_ref.set_linked_to(riff_id.clone());
                                                            gui.ui.track_drawing_area.queue_draw();

                                                            Some((track_uuid, riff_ref.linked_to(), track_name.to_string(), riff_name.clone()))
                                                        } else { None }
                                                    } else { None }
                                                } else { None }
                                            } else { None }
                                        } else { None  }
                                    } else { None };

                                    if let Some((track_uuid, riff_uuid, track_name, riff_name)) = &track_riff {
                                        state.set_selected_riff_uuid(track_uuid.clone(), riff_uuid.clone());
                                        state.set_selected_track(Some(track_uuid.clone()));
                                        gui.set_piano_roll_selected_track_name_label(track_name.as_str());
                                        gui.set_piano_roll_selected_riff_name_label(riff_name.as_str());
                                        gui.ui.piano_roll_drawing_area.queue_draw();
                                    }

                                    track_riff
                                } else { None };

                                if let Some(track) = state.project().song().tracks().get(track_index as usize) {
                                    if let Some((track_uuid, riff_uuid, track_name, riff_name)) = track_riff {
                                        if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_uuid.clone()) {
                                            scroll_notes_into_view(gui, riff);
                                        }
                                    }
                                }
                            }
                            Err(_) => debug!("Main - rx_ui processing loop - RiffGridChangeType::RiffReferenceIncrementRiff - could not get lock on state"),
                        }
                    }
                    RiffGridChangeType::RiffSelectWithTrackIndex{ track_index, position } => {
                        debug!("Main - rx_ui processing loop - RiffGridChangeType::RiffSelectWithTrackIndex");
                        match state.lock() {
                            Ok(mut state) => {
                                let selected_riff_grid_uuid = state.selected_riff_grid_uuid().clone();

                                // get the track
                                let track_riff = if let Some(track) = state.get_project().song_mut().tracks_mut().get_mut(track_index as usize) {
                                    let track_uuid = track.uuid().to_string();
                                    let track_name = track.name().to_string();
                                    let riff_details = track.riffs_mut().iter_mut().map(|riff| (riff.id(), (riff.name().to_string(), riff.length()))).collect::<HashMap<String, (String, f64)>>();
                                    let mut riff_name = None;

                                    // need to use the selected riff grid
                                    if let Some(selected_riff_grid_uuid) = selected_riff_grid_uuid {
                                        // find the riff grid
                                        if let Some(riff_grid) = state.get_project().song_mut().riff_grid(selected_riff_grid_uuid) {
                                            if let Some(riff_grid_track_riff_refs) = riff_grid.track_riff_references(track_uuid.clone()) {
                                                if let Some(riff_ref) = riff_grid_track_riff_refs.iter().find(|riff_ref| {
                                                    if let Some((name, riff_length)) = riff_details.get(&riff_ref.linked_to()) {
                                                        riff_name = Some(name.to_string());
                                                        let riff_ref_end_position = riff_ref.position() + *riff_length;
                                                        if riff_ref.position() <= position && position <= riff_ref_end_position {
                                                            true
                                                        }
                                                        else { false }
                                                    }
                                                    else { false }
                                                }) {
                                                    if let Some(riff_name) = riff_name {
                                                        if riff_name.as_str() != "empty" {
                                                            Some((track_uuid, riff_ref.linked_to(), track_name.to_string(), riff_name))
                                                        } else { None }
                                                    } else { None }
                                                } else { None }
                                            } else { None }
                                        } else { None  }
                                    } else { None }
                                }
                                else { None };

                                if let Some((track_uuid, riff_uuid, track_name, riff_name)) = track_riff {
                                    state.set_selected_riff_uuid(track_uuid.clone(), riff_uuid);
                                    state.set_selected_track(Some(track_uuid));
                                    gui.set_piano_roll_selected_track_name_label(track_name.as_str());
                                    gui.set_piano_roll_selected_riff_name_label(riff_name.as_str());
                                    gui.ui.piano_roll_drawing_area.queue_draw();
                                }
                            }
                            Err(_) => debug!("Main - rx_ui processing loop - RiffGridChangeType::RiffSelectWithTrackIndex - could not get lock on state"),
                        }
                    }
                }
            }
            DAWEvents::RiffGridNameChange(name) => {
                match state.lock() {
                    Ok(mut state) => {
                        let selected_riff_grid_uuid = if let Some(selected_riff_grid_uuid) = state.selected_riff_grid_uuid() {
                            selected_riff_grid_uuid.to_string()
                        }
                        else {
                            "".to_string()
                        };
                        if let Some(riff_grid) = state.get_project().song_mut().riff_grid_mut(selected_riff_grid_uuid) {
                            riff_grid.set_name(name);
                            gui.update_riff_grids_combobox_in_riff_grid_view(&state, true);
                            gui.update_available_riff_grids_in_riff_arrangement_blades(&state);
                        }
                    }
                    Err(_) => debug!("Main - rx_ui processing loop - riff grid name change - could not get lock on state"),
                }
            }
            DAWEvents::RiffGridPlay(riff_grid_uuid) => {
                match state.lock() {
                    Ok(mut state) => {
                        state.play_riff_grid(tx_to_audio, riff_grid_uuid.clone());
                        state.set_playing_riff_grid(Some(riff_grid_uuid.clone()));
                        if let Some(playing_riff_grid_summary_data) = state.playing_riff_grid_summary_data() {
                            // gui.repaint_riff_sequence_view_riff_sequence_active_drawing_areas(&riff_grid_uuid, 0.0, playing_riff_sequence_summary_data);
                        }
                    }
                    Err(_) => debug!("Main - rx_ui processing loop - riff grid play - could not get lock on state"),
                }
            }
            DAWEvents::RiffGridCopy(riff_grid_uuid) => {
                match state.lock() {
                    Ok(mut state) => {
                        let mut copied_riff_grid = RiffGrid::new();
                        if let Some(riff_grid) = state.project().song().riff_grid(riff_grid_uuid) {
                            copied_riff_grid.set_name(format!("Copy of {}", riff_grid.name()));
                            for track_uuid in riff_grid.tracks() {
                                for track_riff_ref in riff_grid.track_riff_references(track_uuid.clone()).unwrap().iter() {
                                    copied_riff_grid.add_riff_reference_to_track(track_uuid.clone(), track_riff_ref.linked_to(), track_riff_ref.position());
                                }
                            }
                        }
                        state.set_selected_riff_grid_uuid(Some(copied_riff_grid.uuid()));
                        state.get_project().song_mut().add_riff_grid(copied_riff_grid);
                        gui.update_available_riff_grids_in_riff_arrangement_blades(&state);
                        gui.update_riff_grids_combobox_in_riff_grid_view(&state, false);
                    }
                    Err(_) => debug!("Main - rx_ui processing loop - riff grid copy - could not get lock on state"),
                }
            }
            DAWEvents::RiffGridCopySelectedToTrackViewCursorPosition(uuid) => {
                // get the current track cursor position and convert it to beats
                let edit_cursor_position_in_beats = match &gui.track_grid {
                    Some(track_grid) => match track_grid.lock() {
                        Ok(grid) => grid.edit_cursor_time_in_beats(),
                        Err(_) => 0.0
                    },
                    None => 0.0
                };

                DAWUtils::copy_riff_grid_to_position(uuid, edit_cursor_position_in_beats, state.clone());
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
                                else if let Some(riff_grid) = state.project().song().riff_grid(riff_item.item_uuid().to_string()) {
                                    play_position_in_beats += DAWUtils::get_riff_grid_length(&riff_grid, &state);
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
                            gui.repaint_riff_arrangement_view_active_drawing_areas(&riff_arrangement_uuid, 0.0, playing_riff_arrangement_summary_data);
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - riff arrangement play - could not get lock on state"),
                }
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
            DAWEvents::RiffArrangementSelected(riff_arrangement_uuid) => {
                match state.lock() {
                    Ok(mut state) => {
                        state.set_selected_riff_arrangement_uuid(Some(riff_arrangement_uuid));
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - riff arrangement selected - could not get lock on state"),
                };
                gui.ui.riff_arrangement_box.queue_draw();
            }
            DAWEvents::RiffArrangementNameChange(riff_arrangement_uuid, name) => {
                match state.lock() {
                    Ok(mut state) => {
                        if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangement_mut(riff_arrangement_uuid) {
                            riff_arrangement.set_name(name);
                            gui.update_riff_arrangements_combobox_in_riff_arrangement_view(&state, true);
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
            DAWEvents::RiffArrangementRiffItemAdd(riff_arrangement_uuid, item_referred_to_uuid, riff_item_type) => {
                debug!("Main - rx_ui processing loop - riff arrangement={} - riff item add: {}, {}, {}", riff_arrangement_uuid.as_str(), riff_arrangement_uuid.as_str(), item_referred_to_uuid.as_str(), match riff_item_type.clone() { RiffItemType::RiffSet => { "RiffSet" } RiffItemType::RiffSequence => {"RiffSequence"} RiffItemType::RiffGrid => {"RiffGrid"}} );
                let state_arc = state.clone();
                match state.lock() {
                    Ok(mut state) => {
                        let item_uuid = Uuid::new_v4();
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
                                riff_arrangement.add_item_at_position(RiffItem::new_with_uuid_string(item_uuid.to_string(), riff_item_type.clone(), item_referred_to_uuid.clone()), selected_riff_item_position + 1);
                            }
                        }
                        else {
                            if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangement_mut(riff_arrangement_uuid.clone()) {
                                riff_arrangement.add_item(RiffItem::new_with_uuid_string(item_uuid.to_string(), riff_item_type.clone(), item_referred_to_uuid.clone()));
                            }
                        }

                        let track_uuids: Vec<String> = state.project().song().tracks().iter().map(|track| track.uuid().to_string()).collect();
                        match riff_item_type {
                            RiffItemType::RiffSet => {
                                let riff_set_name = if let Some(riff_set) = state.project().song().riff_sets().iter().find(|riff_set| riff_set.uuid() == item_referred_to_uuid.clone()) {
                                    riff_set.name().to_string()
                                }
                                else {
                                    "".to_string()
                                };
                                gui.add_riff_arrangement_riff_set_blade(
                                    tx_from_ui,
                                    riff_arrangement_uuid,
                                    item_uuid.to_string(),
                                    item_referred_to_uuid,
                                    track_uuids,
                                    gui.selected_style_provider.clone(),
                                    gui.ui.riff_arrangement_vertical_adjustment.clone(),
                                    riff_set_name,
                                    state_arc,
                                );
                            }
                            RiffItemType::RiffSequence => {
                                gui.add_riff_arrangement_riff_sequence_blade(
                                    tx_from_ui,
                                    riff_arrangement_uuid,
                                    item_referred_to_uuid,
                                    item_uuid.to_string(),
                                    track_uuids,
                                    gui.selected_style_provider.clone(),
                                    gui.ui.riff_arrangement_vertical_adjustment.clone(),
                                    "".to_string(),
                                    state_arc,
                                    &state,
                                );
                            }
                            RiffItemType::RiffGrid => {
                                let riff_grid_name = if let Some(riff_grid) = state.project().song().riff_grids().iter().find(|riff_grid| riff_grid.uuid() == item_referred_to_uuid.clone()) {
                                    riff_grid.name().to_string()
                                }
                                else {
                                    "".to_string()
                                };
                                gui.add_riff_arrangement_riff_grid_blade(
                                    tx_from_ui,
                                    riff_arrangement_uuid,
                                    item_referred_to_uuid, // riff grid uuid
                                    item_uuid.to_string(),
                                    track_uuids,
                                    gui.selected_style_provider.clone(),
                                    gui.ui.riff_arrangement_vertical_adjustment.clone(),
                                    riff_grid_name,
                                    state_arc,
                                    &state,
                                );
                            }
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
                            state.configuration.audio.sample_rate,
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

                        state.set_selected_riff_arrangement_uuid(Some(new_riff_arrangement.uuid()));
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
                }
                gui.ui.riff_sequences_box.queue_draw();
            }
            DAWEvents::TrackGridVerticalScaleChanged(vertical_scale) => {
                
                let widget_height = (TRACK_VIEW_TRACK_PANEL_HEIGHT as f64 * vertical_scale) as i32;
                for track_panel in gui.ui.top_level_vbox.children().iter_mut() {
                    debug!("Track grid - Track panel height: {}", track_panel.allocation().height);
                    track_panel.set_height_request(widget_height);
                }
                // gui.ui.track_panel_scrolled_window.queue_draw();
                gui.ui.top_level_vbox.queue_draw();
                gui.ui.track_drawing_area.queue_draw();
            }
            DAWEvents::RiffGridVerticalScaleChanged(vertical_scale) => {

                let widget_height = (TRACK_VIEW_TRACK_PANEL_HEIGHT as f64 * vertical_scale) as i32;
                for track_panel in gui.ui.riff_grid_track_panel.children().iter_mut() {
                    debug!("Riff grid - Track panel height: {}", track_panel.allocation().height);
                    track_panel.set_height_request(widget_height);
                }
                // gui.ui.track_panel_scrolled_window.queue_draw();
                gui.ui.riff_grid_track_panel.queue_draw();
                gui.ui.riff_grid_drawing_area.queue_draw();
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
            DAWEvents::RiffReferenceRegenerateIds => {
                debug!("Main - rx_ui processing loop - DAWEvents::RiffReferenceRegenerateIds");
                let state_arc = state.clone();
                match state.lock() {
                    Ok(mut state) => {
                        for riff_grid in state.get_project().song_mut().riff_grids_mut().iter_mut() {
                            for track_uuid in riff_grid.tracks_mut().map(|track_uuid| track_uuid.clone()).collect_vec().iter() {
                                if let Some(track_riff_references) = riff_grid.track_riff_references_mut(track_uuid.clone()) {
                                    track_riff_references.iter_mut().for_each(|riff_ref| riff_ref.set_id(Uuid::new_v4().to_string()));
                                }
                            }
                        }

                        let track_uuids = state.get_project().song_mut().tracks_mut().iter_mut().map(|track| track.uuid().to_string()).collect_vec();
                        for riff_set in state.get_project().song_mut().riff_sets_mut().iter_mut() {
                            for track_uuid in track_uuids.iter() {
                                if let Some(riff_ref) = riff_set.get_riff_ref_for_track_mut(track_uuid.clone()) {
                                    riff_ref.set_id(Uuid::new_v4().to_string());
                                }
                            }
                        }

                        for track in state.get_project().song_mut().tracks_mut() {
                            track.riff_refs_mut().iter_mut().for_each(|riff_ref| riff_ref.set_id(Uuid::new_v4().to_string()));
                        }

                        gui.clear_ui();
                        gui.update_ui_from_state(tx_from_ui.clone(), &mut state, state_arc);
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - DAWEvents::RiffReferenceRegenerateIds - could not get lock on state"),
                }
            }
            DAWEvents::AudioConfigurationChanged(sample_rate, block_size) => {
                debug!("Main - rx_ui processing loop - DAWEvents::AudioConfigurationChanged");
                gui.clear_ui();
                let state_arc = state.clone();
                match state.lock() {
                    Ok(mut state) => {
                        state.close_all_tracks(tx_to_audio.clone());
                        state.reset_state();
                        state.configuration.audio.sample_rate = sample_rate;
                        state.configuration.audio.block_size = block_size;

                        {
                            let mut time_info =  vst_host_time_info.write();
                            time_info.sample_pos = 0.0;
                            time_info.sample_rate = state.configuration.audio.sample_rate as f64;
                            time_info.nanoseconds = 0.0;
                            time_info.ppq_pos = 0.0;
                            time_info.tempo = state.project().song().tempo();
                            time_info.bar_start_pos = 0.0;
                            time_info.cycle_start_pos = 0.0;
                            time_info.cycle_end_pos = 0.0;
                            time_info.time_sig_numerator = state.project().song().time_signature_numerator() as i32;
                            time_info.time_sig_denominator = state.project().song().time_signature_denominator() as i32;
                            time_info.smpte_offset = 0;
                            time_info.smpte_frame_rate = vst::api::SmpteFrameRate::Smpte24fps;
                            time_info.samples_to_next_clock = 0;
                            time_info.flags = 3;
                        }

                        // update the transport
                        TRANSPORT.get().write().sample_rate = sample_rate as f64;
                        TRANSPORT.get().write().block_size = block_size as f64;

                        state.stop_jack();
                        state.start_jack(rx_to_audio.clone(), jack_midi_sender.clone(), jack_midi_sender_ui.clone(), jack_time_critical_midi_sender.clone(), jack_audio_coast.clone(), vst_host_time_info.clone());

                        let mut instrument_track_senders2 = HashMap::new();
                        let mut instrument_track_receivers2 = HashMap::new();
                        let mut sample_references = HashMap::new();
                        let mut samples_data = HashMap::new();
                        let sample_rate = state.configuration.audio.sample_rate as f64;
                        let block_size = state.configuration.audio.block_size as f64;
                        let tempo = state.project().song().tempo();
                        let time_signature_numerator = state.project().song().time_signature_numerator();
                        let time_signature_denominator = state.project().song().time_signature_denominator();
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
                                sample_rate,
                                block_size,
                                tempo,
                                time_signature_numerator as i32,
                                time_signature_denominator as i32,
                            );
                        }
                        state.update_track_senders_and_receivers(instrument_track_senders2, instrument_track_receivers2);

                        gui.update_ui_from_state(tx_from_ui, &mut state, state_arc);
                        match tx_to_audio.send(AudioLayerInwardEvent::Tempo(state.project().song().tempo())) {
                            Ok(_) => (),
                            Err(error) => debug!("Problem using tx_to_audio to send tempo message to jack layer: {}", error),
                        }
                    },
                    Err(_) => debug!("Main - rx_ui processing loop - DAWEvents::AudioConfigurationChanged - could not get lock on state"),
                }
            }
        }
        Err(_) => (),
    }
}

// FIXME work in progress trying to figure out how to appease the mighty borrow checker
// fn get_current_context_automation_events(state: &mut DAWState) -> (String, Option<i32>, Option<String>, CurrentView, AutomationEditType, Option<&mut Vec<TrackEvent>>, Option<String>) {
//     let track_uuid = state.selected_track().unwrap_or("".to_string());
//     let automation_type = state.automation_type().clone();
//     let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
//         Some(selected_riff_uuid.clone())
//     }
//     else {
//         None
//     };
//     let current_view = state.current_view().clone();
//     let automation_edit_type = state.automation_edit_type().clone();
//     let selected_riff_arrangement_uuid = if let Some(selected_riff_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
//         Some(selected_riff_arrangement_uuid.clone())
//     }
//     else {
//         None
//     };
//     let plugin_uuid = if let Some(track_type) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
//         if let TrackType::InstrumentTrack(instrument_track) = track_type {
//             instrument_track.instrument().uuid().to_string()
//         }
//         else { "".to_string() }
//     }
//     else { "".to_string() };
//
//     let (events, plugin_uuid) = if let CurrentView::RiffArrangement = current_view {
//         // get the arrangement
//         if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
//             if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
//                 if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
//                     (Some(riff_arr_automation.events_mut()), Some(plugin_uuid))
//                 } else {
//                     riff_arrangement.add_track_automation(track_uuid.clone());
//                     (Some(riff_arrangement.automation_mut(&track_uuid).unwrap().events_mut()), Some(plugin_uuid))
//                 }
//             } else {
//                 (None, Some(plugin_uuid))
//             }
//         } else {
//             (None, Some(plugin_uuid))
//         }
//     } else if let Some(track_type) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
//         if let TrackType::InstrumentTrack(instrument_track) = track_type {
//             match automation_edit_type {
//                 AutomationEditType::Track => {
//                     (Some(track_type.automation_mut().events_mut()), Some(plugin_uuid))
//                 }
//                 AutomationEditType::Riff => {
//                     if let Some(selected_riff_uuid) = selected_riff_uuid.clone() {
//                         if let Some(riff) = track_type.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
//                             (Some(riff.events_mut()), Some(plugin_uuid))
//                         } else {
//                             (None, None)
//                         }
//                     } else {
//                         (None, None)
//                     }
//                 }
//             }
//         }
//         else {
//             (None, None)
//         }
//     }
//     else {
//         (None, None)
//     };
//
//     (track_uuid, automation_type, selected_riff_uuid, current_view, automation_edit_type, events, plugin_uuid)
// }

fn handle_automation_add(time: f64, value: i32, state: &Arc<Mutex<DAWState>>) {
    match state.lock() {
        Ok(mut state) => {
            match state.automation_view_mode() {
                AutomationViewMode::Controllers => handle_automation_controller_add(time, value, &mut state),
                AutomationViewMode::PitchBend => handle_automation_pitch_bend_add(time, value, &mut state),
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
    let automation_discrete = state.automation_discrete();

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
                        let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            riff_arr_automation
                        } else {
                            riff_arrangement.add_track_automation(track_uuid.clone());
                            riff_arrangement.automation_mut(&track_uuid).unwrap()
                        };
                        if automation_discrete {
                            Some(automation.events_mut())
                        }
                        else {
                            if let Some(automation_type_value) = automation_type {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                        if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(automation_envelope.events_mut())
                                } else {
                                    let event_details = PluginParameter {
                                        id: Uuid::new_v4(),
                                        plugin_uuid: plugin_uuid,
                                        instrument: true,
                                        position: 0.0,
                                        index: automation_type_value,
                                        value: 0.0,
                                    };
                                    let mut new_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
                                    automation.envelopes_mut().push(new_envelope);
                                    if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
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
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                match automation_edit_type {
                    AutomationEditType::Track => {
                        let automation = track_type.automation_mut();
                        if automation_discrete {
                            Some(automation.events_mut())
                        }
                        else {
                            if let Some(automation_type_value) = automation_type {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                        if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(automation_envelope.events_mut())
                                } else {
                                    let event_details = PluginParameter {
                                        id: Uuid::new_v4(),
                                        plugin_uuid: plugin_uuid,
                                        instrument: true,
                                        position: 0.0,
                                        index: automation_type_value,
                                        value: 0.0,
                                    };
                                    let mut new_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
                                    automation.envelopes_mut().push(new_envelope);
                                    if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
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
                    events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
                }
            }
        }
    }
}

fn handle_automation_note_expression_add(time: f64, value: i32, state: &mut DAWState) {
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
    let note_expression_type = state.note_expression_type().clone();
    let note_expression_id = state.note_expression_id();
    let note_expression_port_index = state.note_expression_port_index() as i16;
    let note_expression_channel = state.note_expression_channel() as i16;
    let note_expression_key = state.note_expression_key();
    let automation_discrete = state.automation_discrete();

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
                        let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            riff_arr_automation
                        } else {
                            riff_arrangement.add_track_automation(track_uuid.clone());
                            riff_arrangement.automation_mut(&track_uuid).unwrap()
                        };
                        if automation_discrete {
                            Some(automation.events_mut())
                        }
                        else {
                            if let Some(automation_type_value) = automation_type {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) as i32 == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(automation_envelope.events_mut())
                                } else {

                                    let event_details = NoteExpression::new_with_params(
                                        note_expression_type,
                                        note_expression_port_index,
                                        note_expression_channel,
                                        0.0,
                                        note_expression_id,
                                        note_expression_key,
                                        0.0
                                    );
                                    let new_envelope = AutomationEnvelope::new(TrackEvent::NoteExpression(event_details));
                                    automation.envelopes_mut().push(new_envelope);
                                    if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                            if
                                                *(note_expression.expression_type()) == note_expression_type &&
                                                note_expression.port() ==  note_expression_port_index &&
                                                note_expression.channel() ==  note_expression_channel &&
                                                note_expression.note_id() ==  note_expression_id &&
                                                note_expression.key() ==  note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
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
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                match automation_edit_type {
                    AutomationEditType::Track => {
                        let automation = track_type.automation_mut();
                        if automation_discrete {
                            Some(automation.events_mut())
                        }
                        else {
                            if let Some(automation_type_value) = automation_type {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) as i32 == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(automation_envelope.events_mut())
                                } else {

                                    let event_details = NoteExpression::new_with_params(
                                        note_expression_type,
                                        note_expression_port_index,
                                        note_expression_channel,
                                        0.0,
                                        note_expression_id,
                                        note_expression_key,
                                        0.0
                                    );
                                    let new_envelope = AutomationEnvelope::new(TrackEvent::NoteExpression(event_details));
                                    automation.envelopes_mut().push(new_envelope);
                                    if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                            if
                                            *(note_expression.expression_type()) == note_expression_type &&
                                                note_expression.port() ==  note_expression_port_index &&
                                                note_expression.channel() ==  note_expression_channel &&
                                                note_expression.note_id() ==  note_expression_id &&
                                                note_expression.key() ==  note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
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
                events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
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
    let automation_discrete = state.automation_discrete();

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
                            let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                riff_arr_automation
                            } else {
                                riff_arrangement.add_track_automation(track_uuid.clone());
                                riff_arrangement.automation_mut(&track_uuid).unwrap()
                            };
                            if automation_discrete {
                                Some(automation.events_mut())
                            }
                            else {
                                if let Some(automation_type_value) = automation_type {
                                    if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(automation_envelope.events_mut())
                                    } else {
                                        let event_details = PluginParameter {
                                            id: Uuid::new_v4(),
                                            plugin_uuid: Uuid::parse_str(selected_effect_uuid.as_str()).unwrap(),
                                            instrument: true,
                                            position: 0.0,
                                            index: automation_type_value,
                                            value: 0.0,
                                        };
                                        let mut new_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
                                        automation.envelopes_mut().push(new_envelope);
                                        if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                            let mut found = false;
                                            if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                    found = true;
                                                }
                                            }
                                            return found;
                                        }) {
                                            Some(envelope.events_mut())
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
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    match automation_edit_type {
                        AutomationEditType::Track => {
                            let automation = track_type.automation_mut();
                            if automation_discrete {
                                Some(automation.events_mut())
                            }
                            else {
                                if let Some(automation_type_value) = automation_type {
                                    if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(automation_envelope.events_mut())
                                    } else {
                                        let event_details = PluginParameter {
                                            id: Uuid::new_v4(),
                                            plugin_uuid: Uuid::parse_str(selected_effect_uuid.as_str()).unwrap(),
                                            instrument: true,
                                            position: 0.0,
                                            index: automation_type_value,
                                            value: 0.0,
                                        };
                                        let mut new_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
                                        automation.envelopes_mut().push(new_envelope);
                                        if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                            let mut found = false;
                                            if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                    found = true;
                                                }
                                            }
                                            return found;
                                        }) {
                                            Some(envelope.events_mut())
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
                        events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
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
    let automation_discrete = state.automation_discrete();

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
                    let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                        riff_arr_automation
                    } else {
                        riff_arrangement.add_track_automation(track_uuid.clone());
                        riff_arrangement.automation_mut(&track_uuid).unwrap()
                    };
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_type_value) = automation_type {
                            if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::Controller(controller) = envelope.event_details() {
                                    if controller.controller() == automation_type_value {
                                        found = true;
                                    }
                                }
                                return found;
                            }) {
                                Some(automation_envelope.events_mut())
                            } else {
                                let event_details = Controller::new(0.0, automation_type_value, 0);
                                let new_envelope = AutomationEnvelope::new(TrackEvent::Controller(event_details));
                                automation.envelopes_mut().push(new_envelope);
                                if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::Controller(controller) = envelope.event_details() {
                                        if controller.controller() == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(envelope.events_mut())
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
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            match automation_edit_type {
                AutomationEditType::Track => {
                    let automation = track_type.automation_mut();
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_type_value) = automation_type {
                            if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::Controller(controller) = envelope.event_details() {
                                    if controller.controller() == automation_type_value {
                                        found = true;
                                    }
                                }
                                return found;
                            }) {
                                Some(automation_envelope.events_mut())
                            } else {
                                let event_details = Controller::new(0.0, automation_type_value, 0);
                                let new_envelope = AutomationEnvelope::new(TrackEvent::Controller(event_details));
                                automation.envelopes_mut().push(new_envelope);
                                if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::Controller(controller) = envelope.event_details() {
                                        if controller.controller() == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(envelope.events_mut())
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
                events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
            }
        }
    }
}

fn handle_automation_pitch_bend_add(time: f64, value: i32, state: &mut DAWState) {
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_edit_type = state.automation_edit_type();
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_discrete = state.automation_discrete();

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
                    let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                        riff_arr_automation
                    } else {
                        riff_arrangement.add_track_automation(track_uuid.clone());
                        riff_arrangement.automation_mut(&track_uuid).unwrap()
                    };
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                            let mut found = false;
                            if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                found = true;
                            }
                            return found;
                        }) {
                            Some(automation_envelope.events_mut())
                        } else {
                            let event_details = PitchBend::new(0.0, 0);
                            let new_envelope = AutomationEnvelope::new(TrackEvent::PitchBend(event_details));
                            automation.envelopes_mut().push(new_envelope);
                            if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                    found = true;
                                }
                                return found;
                            }) {
                                Some(envelope.events_mut())
                            }
                            else {
                                None
                            }
                        }
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
                    let automation = track_type.automation_mut();
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                            let mut found = false;
                            if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                found = true;
                            }
                            return found;
                        }) {
                            Some(automation_envelope.events_mut())
                        } else {
                            let event_details = PitchBend::new(0.0, 0);
                            let new_envelope = AutomationEnvelope::new(TrackEvent::PitchBend(event_details));
                            automation.envelopes_mut().push(new_envelope);
                            if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                    found = true;
                                }
                                return found;
                            }) {
                                Some(envelope.events_mut())
                            }
                            else {
                                None
                            }
                        }
                    }
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
            let pitch_bend = PitchBend::new(time, (value as f32 / 127.0 * 16384.0 - 8192.0) as i32);
            events.push(TrackEvent::PitchBend(pitch_bend));
            events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
        }
    }
}

fn handle_automation_delete(time: f64, state: &Arc<Mutex<DAWState>>) {
    match state.lock() {
        Ok(mut state) => {
            match state.automation_view_mode() {
                AutomationViewMode::Controllers => handle_automation_controller_delete(time, &mut state),
                AutomationViewMode::PitchBend => handle_automation_pitch_bend_delete(time, &mut state),
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
    let automation_discrete = state.automation_discrete();

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
                        if let Some(automation) = riff_arrangement.automation_mut(&track_uuid) {
                            if automation_discrete {
                                Some(automation.events_mut())
                            }
                            else {
                                if let Some(automation_type_value) = automation_type {
                                    if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(automation_envelope.events_mut())
                                    } else { None }
                                }
                                else { None }
                            }
                        } else { None }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                match automation_edit_type {
                    AutomationEditType::Track => {
                        let automation = track_type.automation_mut();
                        if automation_discrete {
                            Some(automation.events_mut())
                        }
                        else {
                            if let Some(automation_type_value) = automation_type {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                        if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(automation_envelope.events_mut())
                                } else {
                                    let event_details = PluginParameter {
                                        id: Uuid::new_v4(),
                                        plugin_uuid: plugin_uuid,
                                        instrument: true,
                                        position: 0.0,
                                        index: automation_type_value,
                                        value: 0.0,
                                    };
                                    let mut new_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
                                    automation.envelopes_mut().push(new_envelope);
                                    if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
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
    let note_expression_type = state.note_expression_type_mut().clone();
    let automation_type = state.automation_type();
    let note_expression_note_id = state.note_expression_id();
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
    let automation_discrete = state.automation_discrete();

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
                        let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            riff_arr_automation
                        } else {
                            riff_arrangement.add_track_automation(track_uuid.clone());
                            riff_arrangement.automation_mut(&track_uuid).unwrap()
                        };
                        if automation_discrete {
                            Some(automation.events_mut())
                        }
                        else {
                            if let Some(automation_type_value) = automation_type {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) as i32 == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(automation_envelope.events_mut())
                                } else {

                                    let event_details = NoteExpression::new_with_params(
                                        note_expression_type,
                                        note_expression_port_index,
                                        note_expression_channel,
                                        0.0,
                                        note_expression_id,
                                        note_expression_key,
                                        0.0
                                    );
                                    let new_envelope = AutomationEnvelope::new(TrackEvent::NoteExpression(event_details));
                                    automation.envelopes_mut().push(new_envelope);
                                    if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                            if
                                            *(note_expression.expression_type()) == note_expression_type &&
                                                note_expression.port() ==  note_expression_port_index &&
                                                note_expression.channel() ==  note_expression_channel &&
                                                note_expression.note_id() ==  note_expression_id &&
                                                note_expression.key() ==  note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
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
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                match automation_edit_type {
                    AutomationEditType::Track => {
                        let automation = track_type.automation_mut();
                        if automation_discrete {
                            Some(automation.events_mut())
                        }
                        else {
                            if let Some(automation_type_value) = automation_type {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) as i32 == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(automation_envelope.events_mut())
                                } else {

                                    let event_details = NoteExpression::new_with_params(
                                        note_expression_type,
                                        note_expression_port_index,
                                        note_expression_channel,
                                        0.0,
                                        note_expression_id,
                                        note_expression_key,
                                        0.0
                                    );
                                    let new_envelope = AutomationEnvelope::new(TrackEvent::NoteExpression(event_details));
                                    automation.envelopes_mut().push(new_envelope);
                                    if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                            if
                                            *(note_expression.expression_type()) == note_expression_type &&
                                                note_expression.port() ==  note_expression_port_index &&
                                                note_expression.channel() ==  note_expression_channel &&
                                                note_expression.note_id() ==  note_expression_id &&
                                                note_expression.key() ==  note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
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
                            !(
                                (time - EVENT_DELETION_BEAT_TOLERANCE) <= note_expression.position() &&
                                note_expression.position() <= (time + EVENT_DELETION_BEAT_TOLERANCE) &&
                                (note_expression_note_id == -1 || note_expression_note_id == note_expression.note_id()) &&
                                note_expression_type as i32 == *(note_expression.expression_type()) as i32
                            )
                        },
                        _ => true,
                    }
                });
                events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
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
    let automation_discrete = state.automation_discrete();

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
                            let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                riff_arr_automation
                            } else {
                                riff_arrangement.add_track_automation(track_uuid.clone());
                                riff_arrangement.automation_mut(&track_uuid).unwrap()
                            };
                            if automation_discrete {
                                Some(automation.events_mut())
                            }
                            else {
                                if let Some(automation_type_value) = automation_type {
                                    if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(automation_envelope.events_mut())
                                    } else {
                                        let event_details = PluginParameter {
                                            id: Uuid::new_v4(),
                                            plugin_uuid: Uuid::parse_str(selected_effect_uuid.as_str()).unwrap(),
                                            instrument: true,
                                            position: 0.0,
                                            index: automation_type_value,
                                            value: 0.0,
                                        };
                                        let mut new_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
                                        automation.envelopes_mut().push(new_envelope);
                                        if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                            let mut found = false;
                                            if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                    found = true;
                                                }
                                            }
                                            return found;
                                        }) {
                                            Some(envelope.events_mut())
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
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    match automation_edit_type {
                        AutomationEditType::Track => {
                            let automation = track_type.automation_mut();
                            if automation_discrete {
                                Some(automation.events_mut())
                            }
                            else {
                                if let Some(automation_type_value) = automation_type {
                                    if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(automation_envelope.events_mut())
                                    } else {
                                        let event_details = PluginParameter {
                                            id: Uuid::new_v4(),
                                            plugin_uuid: Uuid::parse_str(selected_effect_uuid.as_str()).unwrap(),
                                            instrument: true,
                                            position: 0.0,
                                            index: automation_type_value,
                                            value: 0.0,
                                        };
                                        let mut new_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
                                        automation.envelopes_mut().push(new_envelope);
                                        if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                            let mut found = false;
                                            if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                    found = true;
                                                }
                                            }
                                            return found;
                                        }) {
                                            Some(envelope.events_mut())
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
    let automation_discrete = state.automation_discrete();

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
                    let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                        riff_arr_automation
                    } else {
                        riff_arrangement.add_track_automation(track_uuid.clone());
                        riff_arrangement.automation_mut(&track_uuid).unwrap()
                    };
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_type_value) = automation_type {
                            if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::Controller(controller) = envelope.event_details() {
                                    if controller.controller() == automation_type_value {
                                        found = true;
                                    }
                                }
                                return found;
                            }) {
                                Some(automation_envelope.events_mut())
                            } else {
                                let event_details = Controller::new(0.0, automation_type_value, 0);
                                let new_envelope = AutomationEnvelope::new(TrackEvent::Controller(event_details));
                                automation.envelopes_mut().push(new_envelope);
                                if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::Controller(controller) = envelope.event_details() {
                                        if controller.controller() == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(envelope.events_mut())
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
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            match automation_edit_type {
                AutomationEditType::Track => {
                    let automation = track_type.automation_mut();
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_type_value) = automation_type {
                            if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::Controller(controller) = envelope.event_details() {
                                    if controller.controller() == automation_type_value {
                                        found = true;
                                    }
                                }
                                return found;
                            }) {
                                Some(automation_envelope.events_mut())
                            } else {
                                let event_details = Controller::new(0.0, automation_type_value, 0);
                                let new_envelope = AutomationEnvelope::new(TrackEvent::Controller(event_details));
                                automation.envelopes_mut().push(new_envelope);
                                if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::Controller(controller) = envelope.event_details() {
                                        if controller.controller() == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(envelope.events_mut())
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
                events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
            }
        }
    }
}

fn handle_automation_pitch_bend_delete(time: f64, state: &mut DAWState) {
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();
    let automation_discrete = state.automation_discrete();

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
                    let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                        riff_arr_automation
                    } else {
                        riff_arrangement.add_track_automation(track_uuid.clone());
                        riff_arrangement.automation_mut(&track_uuid).unwrap()
                    };
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                            let mut found = false;
                            if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                found = true;
                            }
                            return found;
                        }) {
                            Some(automation_envelope.events_mut())
                        } else {
                            let event_details = PitchBend::new(0.0, 0);
                            let new_envelope = AutomationEnvelope::new(TrackEvent::PitchBend(event_details));
                            automation.envelopes_mut().push(new_envelope);
                            if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                    found = true;
                                }
                                return found;
                            }) {
                                Some(envelope.events_mut())
                            }
                            else {
                                None
                            }
                        }
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
                    let automation = track_type.automation_mut();
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                            let mut found = false;
                            if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                found = true;
                            }
                            return found;
                        }) {
                            Some(automation_envelope.events_mut())
                        } else {
                            let event_details = PitchBend::new(0.0, 0);
                            let new_envelope = AutomationEnvelope::new(TrackEvent::PitchBend(event_details));
                            automation.envelopes_mut().push(new_envelope);
                            if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                    found = true;
                                }
                                return found;
                            }) {
                                Some(envelope.events_mut())
                            }
                            else {
                                None
                            }
                        }
                    }
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
                    TrackEvent::PitchBend(pitch_bend) => {
                        !((time - EVENT_DELETION_BEAT_TOLERANCE) <= pitch_bend.position() && pitch_bend.position() <= (time + EVENT_DELETION_BEAT_TOLERANCE))
                    }
                    _ => true,
                }
            });
            events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
        }
    }
}

fn handle_automation_cut(state: &Arc<Mutex<DAWState>>, edit_cursor_time_in_beats: f64) {
    match state.lock() {
        Ok(mut state) => {
            match state.automation_view_mode() {
                AutomationViewMode::Controllers => handle_automation_controller_cut(&mut state, edit_cursor_time_in_beats),
                AutomationViewMode::PitchBend => handle_automation_pitch_bend_cut(&mut state, edit_cursor_time_in_beats),
                AutomationViewMode::Instrument => handle_automation_instrument_cut(&mut state, edit_cursor_time_in_beats),
                AutomationViewMode::Effect => handle_automation_effect_cut(&mut state, edit_cursor_time_in_beats),
                AutomationViewMode::NoteExpression => handle_automation_note_expression_cut(&mut state, edit_cursor_time_in_beats),
                _ => (),
            }            
        }
        Err(_) => {

        }
    }
}

fn handle_automation_instrument_cut(state: &mut DAWState, edit_cursor_time_in_beats: f64) {
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
    let automation_discrete = state.automation_discrete();

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
                        if let Some(automation) = riff_arrangement.automation_mut(&track_uuid) {
                            if automation_discrete {
                                Some(automation.events_mut())
                            }
                            else {
                                if let Some(automation_type_value) = automation_type {
                                    if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(automation_envelope.events_mut())
                                    } else { None }
                                }
                                else { None }
                            }
                        } else { None }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                match automation_edit_type {
                    AutomationEditType::Track => {
                        let automation = track_type.automation_mut();
                        if automation_discrete {
                            Some(automation.events_mut())
                        }
                        else {
                            if let Some(automation_type_value) = automation_type {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                        if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(automation_envelope.events_mut())
                                } else {
                                    let event_details = PluginParameter {
                                        id: Uuid::new_v4(),
                                        plugin_uuid: plugin_uuid,
                                        instrument: true,
                                        position: 0.0,
                                        index: automation_type_value,
                                        value: 0.0,
                                    };
                                    let mut new_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
                                    automation.envelopes_mut().push(new_envelope);
                                    if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
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
                            if plugin_param.plugin_uuid().to_string() == plugin_uuid.to_string() && plugin_param.index == automation_type_value {
                                let mut track_event = event.clone();
                                // adjust the position to be relative to the edit cursor
                                track_event.set_position(track_event.position() - edit_cursor_time_in_beats);
                                events_to_copy.push(track_event);
                            }
                        }
                    }
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

    if !events_to_copy.is_empty() {
        state.automation_event_copy_buffer_mut().clear();
        for event in events_to_copy.iter() {
            state.automation_event_copy_buffer_mut().push(event.clone());
        }
    }
}

fn handle_automation_note_expression_cut(state: &mut DAWState, edit_cursor_time_in_beats: f64) {
    let selected = state.selected_automation().to_vec();
    let automation_type = state.automation_type();
    let mut events_to_copy: Vec<TrackEvent> = vec![];
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
    let automation_discrete = state.automation_discrete();

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
                        let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            riff_arr_automation
                        } else {
                            riff_arrangement.add_track_automation(track_uuid.clone());
                            riff_arrangement.automation_mut(&track_uuid).unwrap()
                        };
                        if automation_discrete {
                            Some(automation.events_mut())
                        }
                        else {
                            if let Some(automation_type_value) = automation_type {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) as i32 == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(automation_envelope.events_mut())
                                } else {

                                    let event_details = NoteExpression::new_with_params(
                                        note_expression_type,
                                        note_expression_port_index,
                                        note_expression_channel,
                                        0.0,
                                        note_expression_id,
                                        note_expression_key,
                                        0.0
                                    );
                                    let new_envelope = AutomationEnvelope::new(TrackEvent::NoteExpression(event_details));
                                    automation.envelopes_mut().push(new_envelope);
                                    if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                            if
                                            *(note_expression.expression_type()) == note_expression_type &&
                                                note_expression.port() ==  note_expression_port_index &&
                                                note_expression.channel() ==  note_expression_channel &&
                                                note_expression.note_id() ==  note_expression_id &&
                                                note_expression.key() ==  note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
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
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                match automation_edit_type {
                    AutomationEditType::Track => {
                        let automation = track_type.automation_mut();
                        if automation_discrete {
                            Some(automation.events_mut())
                        }
                        else {
                            if let Some(automation_type_value) = automation_type {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) as i32 == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(automation_envelope.events_mut())
                                } else {

                                    let event_details = NoteExpression::new_with_params(
                                        note_expression_type,
                                        note_expression_port_index,
                                        note_expression_channel,
                                        0.0,
                                        note_expression_id,
                                        note_expression_key,
                                        0.0
                                    );
                                    let new_envelope = AutomationEnvelope::new(TrackEvent::NoteExpression(event_details));
                                    automation.envelopes_mut().push(new_envelope);
                                    if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                            if
                                            *(note_expression.expression_type()) == note_expression_type &&
                                                note_expression.port() ==  note_expression_port_index &&
                                                note_expression.channel() ==  note_expression_channel &&
                                                note_expression.note_id() ==  note_expression_id &&
                                                note_expression.key() ==  note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
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
                events.retain(|event| {
                    match event {
                        TrackEvent::NoteExpression(note_expression) => {
                            !selected.contains(&note_expression.id())
                        },
                        _ => true,
                    }
                });
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

fn handle_automation_effect_cut(state: &mut DAWState, edit_cursor_time_in_beats: f64) {
    let selected = state.selected_automation().to_vec();
    let mut events_to_copy: Vec<TrackEvent> = vec![];
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
    let automation_discrete = state.automation_discrete();

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
                            let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                riff_arr_automation
                            } else {
                                riff_arrangement.add_track_automation(track_uuid.clone());
                                riff_arrangement.automation_mut(&track_uuid).unwrap()
                            };
                            if automation_discrete {
                                Some(automation.events_mut())
                            }
                            else {
                                if let Some(automation_type_value) = automation_type {
                                    if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(automation_envelope.events_mut())
                                    } else {
                                        let event_details = PluginParameter {
                                            id: Uuid::new_v4(),
                                            plugin_uuid: Uuid::parse_str(selected_effect_uuid.as_str()).unwrap(),
                                            instrument: true,
                                            position: 0.0,
                                            index: automation_type_value,
                                            value: 0.0,
                                        };
                                        let mut new_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
                                        automation.envelopes_mut().push(new_envelope);
                                        if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                            let mut found = false;
                                            if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                    found = true;
                                                }
                                            }
                                            return found;
                                        }) {
                                            Some(envelope.events_mut())
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
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    match automation_edit_type {
                        AutomationEditType::Track => {
                            let automation = track_type.automation_mut();
                            if automation_discrete {
                                Some(automation.events_mut())
                            }
                            else {
                                if let Some(automation_type_value) = automation_type {
                                    if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(automation_envelope.events_mut())
                                    } else {
                                        let event_details = PluginParameter {
                                            id: Uuid::new_v4(),
                                            plugin_uuid: Uuid::parse_str(selected_effect_uuid.as_str()).unwrap(),
                                            instrument: true,
                                            position: 0.0,
                                            index: automation_type_value,
                                            value: 0.0,
                                        };
                                        let mut new_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
                                        automation.envelopes_mut().push(new_envelope);
                                        if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                            let mut found = false;
                                            if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                    found = true;
                                                }
                                            }
                                            return found;
                                        }) {
                                            Some(envelope.events_mut())
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
                                if plugin_param.plugin_uuid().to_string() == selected_effect_uuid && plugin_param.index == automation_type_value {
                                    let mut track_event = event.clone();
                                    // adjust the position to be relative to the edit cursor
                                    track_event.set_position(track_event.position() - edit_cursor_time_in_beats);
                                    events_to_copy.push(track_event);
                                }
                            }
                        }

                        events.retain(|event| {
                            match event {
                                TrackEvent::AudioPluginParameter(plugin_param) => {
                                    !(plugin_param.plugin_uuid().to_string() == selected_effect_uuid && plugin_param.index == automation_type_value && selected.contains(&plugin_param.id()))
                                },
                                _ => true,
                            }
                        });
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

fn handle_automation_controller_cut(state: &mut DAWState, edit_cursor_time_in_beats: f64) {
    let selected = state.selected_automation().to_vec();
    let mut events_to_copy: Vec<TrackEvent> = vec![];
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
    let automation_discrete = state.automation_discrete();

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
                    let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                        riff_arr_automation
                    } else {
                        riff_arrangement.add_track_automation(track_uuid.clone());
                        riff_arrangement.automation_mut(&track_uuid).unwrap()
                    };
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_type_value) = automation_type {
                            if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::Controller(controller) = envelope.event_details() {
                                    if controller.controller() == automation_type_value {
                                        found = true;
                                    }
                                }
                                return found;
                            }) {
                                Some(automation_envelope.events_mut())
                            } else {
                                let event_details = Controller::new(0.0, automation_type_value, 0);
                                let new_envelope = AutomationEnvelope::new(TrackEvent::Controller(event_details));
                                automation.envelopes_mut().push(new_envelope);
                                if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::Controller(controller) = envelope.event_details() {
                                        if controller.controller() == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(envelope.events_mut())
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
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            match automation_edit_type {
                AutomationEditType::Track => {
                    let automation = track_type.automation_mut();
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_type_value) = automation_type {
                            if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::Controller(controller) = envelope.event_details() {
                                    if controller.controller() == automation_type_value {
                                        found = true;
                                    }
                                }
                                return found;
                            }) {
                                Some(automation_envelope.events_mut())
                            } else {
                                let event_details = Controller::new(0.0, automation_type_value, 0);
                                let new_envelope = AutomationEnvelope::new(TrackEvent::Controller(event_details));
                                automation.envelopes_mut().push(new_envelope);
                                if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::Controller(controller) = envelope.event_details() {
                                        if controller.controller() == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(envelope.events_mut())
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
                for event in events.iter().find(|event| selected.contains(&event.id())).iter() {
                    if let TrackEvent::Controller(controller) = event {
                        if controller.controller() == automation_type_value {
                            let mut track_event = (*event).clone();
                            // adjust the position to be relative to the edit cursor
                            track_event.set_position(track_event.position() - edit_cursor_time_in_beats);
                            events_to_copy.push(track_event);
                        }
                    }
                }
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

    if !events_to_copy.is_empty() {
        state.automation_event_copy_buffer_mut().clear();
        for event in events_to_copy.iter() {
            state.automation_event_copy_buffer_mut().push(event.clone());
        }
    }
}

fn handle_automation_pitch_bend_cut(state: &mut DAWState, edit_cursor_time_in_beats: f64) {
    let selected = state.selected_automation().to_vec();
    let mut events_to_copy: Vec<TrackEvent> = vec![];
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();
    let automation_discrete = state.automation_discrete();

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
                    let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                        riff_arr_automation
                    } else {
                        riff_arrangement.add_track_automation(track_uuid.clone());
                        riff_arrangement.automation_mut(&track_uuid).unwrap()
                    };
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                            let mut found = false;
                            if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                found = true;
                            }
                            return found;
                        }) {
                            Some(automation_envelope.events_mut())
                        } else {
                            let event_details = PitchBend::new(0.0, 0);
                            let new_envelope = AutomationEnvelope::new(TrackEvent::PitchBend(event_details));
                            automation.envelopes_mut().push(new_envelope);
                            if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                    found = true;
                                }
                                return found;
                            }) {
                                Some(envelope.events_mut())
                            }
                            else {
                                None
                            }
                        }
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
                    let automation = track_type.automation_mut();
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                            let mut found = false;
                            if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                found = true;
                            }
                            return found;
                        }) {
                            Some(automation_envelope.events_mut())
                        } else {
                            let event_details = PitchBend::new(0.0, 0);
                            let new_envelope = AutomationEnvelope::new(TrackEvent::PitchBend(event_details));
                            automation.envelopes_mut().push(new_envelope);
                            if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                    found = true;
                                }
                                return found;
                            }) {
                                Some(envelope.events_mut())
                            }
                            else {
                                None
                            }
                        }
                    }
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
                if let TrackEvent::PitchBend(pitch_bend) = event {
                    let mut track_event = event.clone();
                    // adjust the position to be relative to the edit cursor
                    track_event.set_position(track_event.position() - edit_cursor_time_in_beats);
                    events_to_copy.push(track_event);
                }
            }
            events.retain(|event| {
                match event {
                    TrackEvent::PitchBend(pitch_bend) => {
                        !selected.contains(&pitch_bend.id())
                    }
                    _ => true,
                }
            });
        }
    }

    if !events_to_copy.is_empty() {
        state.automation_event_copy_buffer_mut().clear();
        for event in events_to_copy.iter() {
            state.automation_event_copy_buffer_mut().push(event.clone());
        }
    }
}

fn handle_automation_translate_selected(state: &Arc<Mutex<DAWState>>, translate_direction: TranslateDirection, snap_in_beats: f64) {
    match state.lock() {
        Ok(mut state) => {
            match state.automation_view_mode() {
                AutomationViewMode::Controllers => handle_automation_controller_translate_selected(&mut state, translate_direction, snap_in_beats),
                AutomationViewMode::PitchBend => handle_automation_pitch_bend_translate_selected(&mut state, translate_direction, snap_in_beats),
                AutomationViewMode::Instrument => handle_automation_instrument_translate_selected(&mut state, translate_direction, snap_in_beats),
                AutomationViewMode::Effect => handle_automation_effect_translate_selected(&mut state, translate_direction, snap_in_beats),
                AutomationViewMode::NoteExpression => handle_automation_note_expression_translate_selected(&mut state, translate_direction, snap_in_beats),
                AutomationViewMode::NoteVelocities => handle_automation_note_velocities_translate_selected(&mut state, translate_direction),
            }
        }
        Err(_) => {

        }
    }
}

fn handle_automation_instrument_translate_selected(state: &mut DAWState, translate_direction: TranslateDirection, snap_in_beats: f64) {
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
    let automation_discrete = state.automation_discrete();

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
                        if let Some(automation) = riff_arrangement.automation_mut(&track_uuid) {
                            if automation_discrete {
                                Some(automation.events_mut())
                            }
                            else {
                                if let Some(automation_type_value) = automation_type {
                                    if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(automation_envelope.events_mut())
                                    } else { None }
                                }
                                else { None }
                            }
                        } else { None }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                match automation_edit_type {
                    AutomationEditType::Track => {
                        let automation = track_type.automation_mut();
                        if automation_discrete {
                            Some(automation.events_mut())
                        }
                        else {
                            if let Some(automation_type_value) = automation_type {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                        if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(automation_envelope.events_mut())
                                } else {
                                    let event_details = PluginParameter {
                                        id: Uuid::new_v4(),
                                        plugin_uuid: plugin_uuid,
                                        instrument: true,
                                        position: 0.0,
                                        index: automation_type_value,
                                        value: 0.0,
                                    };
                                    let mut new_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
                                    automation.envelopes_mut().push(new_envelope);
                                    if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
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
                    events.iter_mut().for_each(|event| {
                        match event {
                            TrackEvent::AudioPluginParameter(plugin_param) => {
                                let position = plugin_param.position();
                                if plugin_param.index == automation_type_value && selected.contains(&plugin_param.id()) {
                                    match translate_direction {
                                        TranslateDirection::Up => {
                                            if plugin_param.value() <= 0.99 {
                                                plugin_param.set_value(plugin_param.value() + 0.01);
                                            }
                                        }
                                        TranslateDirection::Down => {
                                            if plugin_param.value() >= 0.01 {
                                                plugin_param.set_value(plugin_param.value() - 0.01);
                                            }
                                        }
                                        TranslateDirection::Left => {
                                            if position > 0.0 && (position - snap_in_beats) >= 0.0 {
                                                plugin_param.set_position(position - snap_in_beats);
                                            }
                                        }
                                        TranslateDirection::Right => {
                                            plugin_param.set_position(position + snap_in_beats);
                                        }
                                    }
                                }
                            }
                            _ => (),
                        }
                    });
                    events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
                }
            }
        }
    }
}

fn handle_automation_note_expression_translate_selected(state: &mut DAWState, translate_direction: TranslateDirection, snap_in_beats: f64) {
    let selected = state.selected_automation().to_vec();
    let automation_type = state.automation_type();
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
    let automation_discrete = state.automation_discrete();

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
                        let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            riff_arr_automation
                        } else {
                            riff_arrangement.add_track_automation(track_uuid.clone());
                            riff_arrangement.automation_mut(&track_uuid).unwrap()
                        };
                        if automation_discrete {
                            Some(automation.events_mut())
                        }
                        else {
                            if let Some(automation_type_value) = automation_type {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) as i32 == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(automation_envelope.events_mut())
                                } else {

                                    let event_details = NoteExpression::new_with_params(
                                        note_expression_type,
                                        note_expression_port_index,
                                        note_expression_channel,
                                        0.0,
                                        note_expression_id,
                                        note_expression_key,
                                        0.0
                                    );
                                    let new_envelope = AutomationEnvelope::new(TrackEvent::NoteExpression(event_details));
                                    automation.envelopes_mut().push(new_envelope);
                                    if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                            if
                                            *(note_expression.expression_type()) == note_expression_type &&
                                                note_expression.port() ==  note_expression_port_index &&
                                                note_expression.channel() ==  note_expression_channel &&
                                                note_expression.note_id() ==  note_expression_id &&
                                                note_expression.key() ==  note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
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
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                match automation_edit_type {
                    AutomationEditType::Track => {
                        let automation = track_type.automation_mut();
                        if automation_discrete {
                            Some(automation.events_mut())
                        }
                        else {
                            if let Some(automation_type_value) = automation_type {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) as i32 == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(automation_envelope.events_mut())
                                } else {

                                    let event_details = NoteExpression::new_with_params(
                                        note_expression_type,
                                        note_expression_port_index,
                                        note_expression_channel,
                                        0.0,
                                        note_expression_id,
                                        note_expression_key,
                                        0.0
                                    );
                                    let new_envelope = AutomationEnvelope::new(TrackEvent::NoteExpression(event_details));
                                    automation.envelopes_mut().push(new_envelope);
                                    if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                            if
                                            *(note_expression.expression_type()) == note_expression_type &&
                                                note_expression.port() ==  note_expression_port_index &&
                                                note_expression.channel() ==  note_expression_channel &&
                                                note_expression.note_id() ==  note_expression_id &&
                                                note_expression.key() ==  note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
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
                for event in events.iter_mut().filter(|event| selected.contains(&event.id())) {
                    if let TrackEvent::NoteExpression(note_expression) = event {
                        let position = note_expression.position();
                        match translate_direction {
                            TranslateDirection::Up => {
                                if note_expression.value() <= 0.99 {
                                    note_expression.set_value(note_expression.value() + 0.01);
                                }
                            }
                            TranslateDirection::Down => {
                                if note_expression.value() >= 0.01 {
                                    note_expression.set_value(note_expression.value() - 0.01);
                                }
                            }
                            TranslateDirection::Left => {
                                if position > 0.0 && (position - snap_in_beats) >= 0.0 {
                                    note_expression.set_position(position - snap_in_beats);
                                }
                            }
                            TranslateDirection::Right => {
                                note_expression.set_position(position + snap_in_beats);
                            }
                        }
                    }
                }
                events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
            }
        }
    }
}

fn handle_automation_effect_translate_selected(state: &mut DAWState, translate_direction: TranslateDirection, snap_in_beats: f64) {
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
    let automation_discrete = state.automation_discrete();

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
                            let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                riff_arr_automation
                            } else {
                                riff_arrangement.add_track_automation(track_uuid.clone());
                                riff_arrangement.automation_mut(&track_uuid).unwrap()
                            };
                            if automation_discrete {
                                Some(automation.events_mut())
                            }
                            else {
                                if let Some(automation_type_value) = automation_type {
                                    if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(automation_envelope.events_mut())
                                    } else {
                                        let event_details = PluginParameter {
                                            id: Uuid::new_v4(),
                                            plugin_uuid: Uuid::parse_str(selected_effect_uuid.as_str()).unwrap(),
                                            instrument: true,
                                            position: 0.0,
                                            index: automation_type_value,
                                            value: 0.0,
                                        };
                                        let mut new_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
                                        automation.envelopes_mut().push(new_envelope);
                                        if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                            let mut found = false;
                                            if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                    found = true;
                                                }
                                            }
                                            return found;
                                        }) {
                                            Some(envelope.events_mut())
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
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    match automation_edit_type {
                        AutomationEditType::Track => {
                            let automation = track_type.automation_mut();
                            if automation_discrete {
                                Some(automation.events_mut())
                            }
                            else {
                                if let Some(automation_type_value) = automation_type {
                                    if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(automation_envelope.events_mut())
                                    } else {
                                        let event_details = PluginParameter {
                                            id: Uuid::new_v4(),
                                            plugin_uuid: Uuid::parse_str(selected_effect_uuid.as_str()).unwrap(),
                                            instrument: true,
                                            position: 0.0,
                                            index: automation_type_value,
                                            value: 0.0,
                                        };
                                        let mut new_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
                                        automation.envelopes_mut().push(new_envelope);
                                        if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                            let mut found = false;
                                            if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                    found = true;
                                                }
                                            }
                                            return found;
                                        }) {
                                            Some(envelope.events_mut())
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
                        events.iter_mut().for_each(|event| {
                            match event {
                                TrackEvent::AudioPluginParameter(plugin_param) => {
                                    let position = plugin_param.position();
                                    if plugin_param.index == automation_type_value && selected.contains(&plugin_param.id()) {
                                        match translate_direction {
                                            TranslateDirection::Up => {
                                                if plugin_param.value() <= 0.99 {
                                                    plugin_param.set_value(plugin_param.value() + 0.01);
                                                }
                                            }
                                            TranslateDirection::Down => {
                                                if plugin_param.value() >= 0.01 {
                                                    plugin_param.set_value(plugin_param.value() - 0.01);
                                                }
                                            }
                                            TranslateDirection::Left => {
                                                if position > 0.0 && (position - snap_in_beats) >= 0.0 {
                                                    plugin_param.set_position(position - snap_in_beats);
                                                }
                                            }
                                            TranslateDirection::Right => {
                                                plugin_param.set_position(position + snap_in_beats);
                                            }
                                        }
                                    }
                                }
                                _ => (),
                            }
                        });
                        events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
                    }
                }
            }
        }
    }
}

fn handle_automation_controller_translate_selected(state: &mut DAWState, translate_direction: TranslateDirection, snap_in_beats: f64) {
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
    let automation_discrete = state.automation_discrete();

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
                    let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                        riff_arr_automation
                    } else {
                        riff_arrangement.add_track_automation(track_uuid.clone());
                        riff_arrangement.automation_mut(&track_uuid).unwrap()
                    };
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_type_value) = automation_type {
                            if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::Controller(controller) = envelope.event_details() {
                                    if controller.controller() == automation_type_value {
                                        found = true;
                                    }
                                }
                                return found;
                            }) {
                                Some(automation_envelope.events_mut())
                            } else {
                                let event_details = Controller::new(0.0, automation_type_value, 0);
                                let new_envelope = AutomationEnvelope::new(TrackEvent::Controller(event_details));
                                automation.envelopes_mut().push(new_envelope);
                                if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::Controller(controller) = envelope.event_details() {
                                        if controller.controller() == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(envelope.events_mut())
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
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            match automation_edit_type {
                AutomationEditType::Track => {
                    let automation = track_type.automation_mut();
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_type_value) = automation_type {
                            if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::Controller(controller) = envelope.event_details() {
                                    if controller.controller() == automation_type_value {
                                        found = true;
                                    }
                                }
                                return found;
                            }) {
                                Some(automation_envelope.events_mut())
                            } else {
                                let event_details = Controller::new(0.0, automation_type_value, 0);
                                let new_envelope = AutomationEnvelope::new(TrackEvent::Controller(event_details));
                                automation.envelopes_mut().push(new_envelope);
                                if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::Controller(controller) = envelope.event_details() {
                                        if controller.controller() == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(envelope.events_mut())
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
                events.iter_mut().for_each(|event| {
                    match event {
                        TrackEvent::Controller(controller) => {
                            let position = controller.position();
                            if controller.controller() == automation_type_value && selected.contains(&controller.id()) {
                                match translate_direction {
                                    TranslateDirection::Up => {
                                        if controller.value() < 127 {
                                            controller.set_value(controller.value() + 1);
                                        }
                                    }
                                    TranslateDirection::Down => {
                                        if controller.value() > 0 {
                                            controller.set_value(controller.value() - 1);
                                        }
                                    }
                                    TranslateDirection::Left => {
                                        if position > 0.0 && (position - snap_in_beats) >= 0.0 {
                                            controller.set_position(position - snap_in_beats);
                                        }
                                    }
                                    TranslateDirection::Right => {
                                        controller.set_position(position + snap_in_beats);
                                    }
                                }
                            }
                        }
                        _ => (),
                    }
                });
                events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
            }
        }
    }
}

fn handle_automation_note_velocities_translate_selected(state: &mut DAWState, translate_direction: TranslateDirection) {
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

        if let Some(events) = events {
            for event in events.iter_mut() {
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
                    _ => {}
                }
            }
        }
    }
}

fn handle_automation_pitch_bend_translate_selected(state: &mut DAWState, translate_direction: TranslateDirection, snap_in_beats: f64) {
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
    let automation_discrete = state.automation_discrete();

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
                    let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                        riff_arr_automation
                    } else {
                        riff_arrangement.add_track_automation(track_uuid.clone());
                        riff_arrangement.automation_mut(&track_uuid).unwrap()
                    };
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                            let mut found = false;
                            if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                found = true;
                            }
                            return found;
                        }) {
                            Some(automation_envelope.events_mut())
                        } else {
                            let event_details = PitchBend::new(0.0, 0);
                            let new_envelope = AutomationEnvelope::new(TrackEvent::PitchBend(event_details));
                            automation.envelopes_mut().push(new_envelope);
                            if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                    found = true;
                                }
                                return found;
                            }) {
                                Some(envelope.events_mut())
                            }
                            else {
                                None
                            }
                        }
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
                    let automation = track_type.automation_mut();
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                            let mut found = false;
                            if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                found = true;
                            }
                            return found;
                        }) {
                            Some(automation_envelope.events_mut())
                        } else {
                            let event_details = PitchBend::new(0.0, 0);
                            let new_envelope = AutomationEnvelope::new(TrackEvent::PitchBend(event_details));
                            automation.envelopes_mut().push(new_envelope);
                            if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                    found = true;
                                }
                                return found;
                            }) {
                                Some(envelope.events_mut())
                            }
                            else {
                                None
                            }
                        }
                    }
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
            events.iter_mut().for_each(|event| {
                match event {
                    TrackEvent::PitchBend(pitch_bend) => {
                        let position = pitch_bend.position();
                        if selected.contains(&pitch_bend.id()) {
                            match translate_direction {
                                TranslateDirection::Up => {
                                    if pitch_bend.value() < 8192 {
                                        pitch_bend.set_value(pitch_bend.value() + 1);
                                    }
                                }
                                TranslateDirection::Down => {
                                    if pitch_bend.value() > -8192 {
                                        pitch_bend.set_value(pitch_bend.value() - 1);
                                    }
                                }
                                TranslateDirection::Left => {
                                    if position > 0.0 && (position - snap_in_beats) >= 0.0 {
                                        pitch_bend.set_position(position - snap_in_beats);
                                    }
                                }
                                TranslateDirection::Right => {
                                    pitch_bend.set_position(position + snap_in_beats);
                                }
                            }
                        }
                    }
                    _ => (),
                }
            });
            events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
        }
    }
}


fn handle_automation_copy(state: &Arc<Mutex<DAWState>>, edit_cursor_time_in_beats: f64) {
    match state.lock() {
        Ok(mut state) => {
            match state.automation_view_mode() {
                AutomationViewMode::Controllers => handle_automation_controller_copy(&mut state, edit_cursor_time_in_beats),
                AutomationViewMode::PitchBend => handle_automation_pitch_bend_copy(&mut state, edit_cursor_time_in_beats),
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
    let automation_discrete = state.automation_discrete();

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
                        if let Some(automation) = riff_arrangement.automation_mut(&track_uuid) {
                            if automation_discrete {
                                Some(automation.events_mut())
                            }
                            else {
                                if let Some(automation_type_value) = automation_type {
                                    if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(automation_envelope.events_mut())
                                    } else { None }
                                }
                                else { None }
                            }
                        } else { None }
                    } else { None }
                } else { None }
            } else {
                match automation_edit_type {
                    AutomationEditType::Track => {
                        let automation = track_type.automation_mut();
                        if automation_discrete {
                            Some(automation.events_mut())
                        }
                        else {
                            if let Some(automation_type_value) = automation_type {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                        if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(automation_envelope.events_mut())
                                } else {
                                    let event_details = PluginParameter {
                                        id: Uuid::new_v4(),
                                        plugin_uuid: plugin_uuid,
                                        instrument: true,
                                        position: 0.0,
                                        index: automation_type_value,
                                        value: 0.0,
                                    };
                                    let mut new_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
                                    automation.envelopes_mut().push(new_envelope);
                                    if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
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
                            if plugin_param.plugin_uuid().to_string() == plugin_uuid.to_string() && plugin_param.index == automation_type_value {
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
    let automation_type = state.automation_type();
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
    let note_expression_type = state.note_expression_type().clone();
    let note_expression_id = state.note_expression_id();
    let note_expression_port_index = state.note_expression_port_index() as i16;
    let note_expression_channel = state.note_expression_channel() as i16;
    let note_expression_key = state.note_expression_key();
    let automation_discrete = state.automation_discrete();

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
                        let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            riff_arr_automation
                        } else {
                            riff_arrangement.add_track_automation(track_uuid.clone());
                            riff_arrangement.automation_mut(&track_uuid).unwrap()
                        };
                        if automation_discrete {
                            Some(automation.events_mut())
                        }
                        else {
                            if let Some(automation_type_value) = automation_type {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) as i32 == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(automation_envelope.events_mut())
                                } else {

                                    let event_details = NoteExpression::new_with_params(
                                        note_expression_type,
                                        note_expression_port_index,
                                        note_expression_channel,
                                        0.0,
                                        note_expression_id,
                                        note_expression_key,
                                        0.0
                                    );
                                    let new_envelope = AutomationEnvelope::new(TrackEvent::NoteExpression(event_details));
                                    automation.envelopes_mut().push(new_envelope);
                                    if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                            if
                                            *(note_expression.expression_type()) == note_expression_type &&
                                                note_expression.port() ==  note_expression_port_index &&
                                                note_expression.channel() ==  note_expression_channel &&
                                                note_expression.note_id() ==  note_expression_id &&
                                                note_expression.key() ==  note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
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
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                match automation_edit_type {
                    AutomationEditType::Track => {
                        let automation = track_type.automation_mut();
                        if automation_discrete {
                            Some(automation.events_mut())
                        }
                        else {
                            if let Some(automation_type_value) = automation_type {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) as i32 == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(automation_envelope.events_mut())
                                } else {

                                    let event_details = NoteExpression::new_with_params(
                                        note_expression_type,
                                        note_expression_port_index,
                                        note_expression_channel,
                                        0.0,
                                        note_expression_id,
                                        note_expression_key,
                                        0.0
                                    );
                                    let new_envelope = AutomationEnvelope::new(TrackEvent::NoteExpression(event_details));
                                    automation.envelopes_mut().push(new_envelope);
                                    if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                            if
                                            *(note_expression.expression_type()) == note_expression_type &&
                                                note_expression.port() ==  note_expression_port_index &&
                                                note_expression.channel() ==  note_expression_channel &&
                                                note_expression.note_id() ==  note_expression_id &&
                                                note_expression.key() ==  note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
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
    let automation_discrete = state.automation_discrete();

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
                            let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                riff_arr_automation
                            } else {
                                riff_arrangement.add_track_automation(track_uuid.clone());
                                riff_arrangement.automation_mut(&track_uuid).unwrap()
                            };
                            if automation_discrete {
                                Some(automation.events_mut())
                            }
                            else {
                                if let Some(automation_type_value) = automation_type {
                                    if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(automation_envelope.events_mut())
                                    } else {
                                        let event_details = PluginParameter {
                                            id: Uuid::new_v4(),
                                            plugin_uuid: Uuid::parse_str(selected_effect_uuid.as_str()).unwrap(),
                                            instrument: true,
                                            position: 0.0,
                                            index: automation_type_value,
                                            value: 0.0,
                                        };
                                        let mut new_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
                                        automation.envelopes_mut().push(new_envelope);
                                        if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                            let mut found = false;
                                            if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                    found = true;
                                                }
                                            }
                                            return found;
                                        }) {
                                            Some(envelope.events_mut())
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
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    match automation_edit_type {
                        AutomationEditType::Track => {
                            let automation = track_type.automation_mut();
                            if automation_discrete {
                                Some(automation.events_mut())
                            }
                            else {
                                if let Some(automation_type_value) = automation_type {
                                    if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(automation_envelope.events_mut())
                                    } else {
                                        let event_details = PluginParameter {
                                            id: Uuid::new_v4(),
                                            plugin_uuid: Uuid::parse_str(selected_effect_uuid.as_str()).unwrap(),
                                            instrument: true,
                                            position: 0.0,
                                            index: automation_type_value,
                                            value: 0.0,
                                        };
                                        let mut new_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
                                        automation.envelopes_mut().push(new_envelope);
                                        if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                            let mut found = false;
                                            if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                    found = true;
                                                }
                                            }
                                            return found;
                                        }) {
                                            Some(envelope.events_mut())
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
                                if plugin_param.plugin_uuid().to_string() == selected_effect_uuid && plugin_param.index == automation_type_value {
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
    let automation_discrete = state.automation_discrete();

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
                    let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                        riff_arr_automation
                    } else {
                        riff_arrangement.add_track_automation(track_uuid.clone());
                        riff_arrangement.automation_mut(&track_uuid).unwrap()
                    };
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_type_value) = automation_type {
                            if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::Controller(controller) = envelope.event_details() {
                                    if controller.controller() == automation_type_value {
                                        found = true;
                                    }
                                }
                                return found;
                            }) {
                                Some(automation_envelope.events_mut())
                            } else {
                                let event_details = Controller::new(0.0, automation_type_value, 0);
                                let new_envelope = AutomationEnvelope::new(TrackEvent::Controller(event_details));
                                automation.envelopes_mut().push(new_envelope);
                                if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::Controller(controller) = envelope.event_details() {
                                        if controller.controller() == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(envelope.events_mut())
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
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            match automation_edit_type {
                AutomationEditType::Track => {
                    let automation = track_type.automation_mut();
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_type_value) = automation_type {
                            if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::Controller(controller) = envelope.event_details() {
                                    if controller.controller() == automation_type_value {
                                        found = true;
                                    }
                                }
                                return found;
                            }) {
                                Some(automation_envelope.events_mut())
                            } else {
                                let event_details = Controller::new(0.0, automation_type_value, 0);
                                let new_envelope = AutomationEnvelope::new(TrackEvent::Controller(event_details));
                                automation.envelopes_mut().push(new_envelope);
                                if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::Controller(controller) = envelope.event_details() {
                                        if controller.controller() == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(envelope.events_mut())
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

fn handle_automation_pitch_bend_copy(state: &mut DAWState, edit_cursor_time_in_beats: f64) {
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
    let automation_discrete = state.automation_discrete();

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
                    let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                        riff_arr_automation
                    } else {
                        riff_arrangement.add_track_automation(track_uuid.clone());
                        riff_arrangement.automation_mut(&track_uuid).unwrap()
                    };
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                            let mut found = false;
                            if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                found = true;
                            }
                            return found;
                        }) {
                            Some(automation_envelope.events_mut())
                        } else {
                            let event_details = PitchBend::new(0.0, 0);
                            let new_envelope = AutomationEnvelope::new(TrackEvent::PitchBend(event_details));
                            automation.envelopes_mut().push(new_envelope);
                            if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                    found = true;
                                }
                                return found;
                            }) {
                                Some(envelope.events_mut())
                            }
                            else {
                                None
                            }
                        }
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
                    let automation = track_type.automation_mut();
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                            let mut found = false;
                            if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                found = true;
                            }
                            return found;
                        }) {
                            Some(automation_envelope.events_mut())
                        } else {
                            let event_details = PitchBend::new(0.0, 0);
                            let new_envelope = AutomationEnvelope::new(TrackEvent::PitchBend(event_details));
                            automation.envelopes_mut().push(new_envelope);
                            if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                    found = true;
                                }
                                return found;
                            }) {
                                Some(envelope.events_mut())
                            }
                            else {
                                None
                            }
                        }
                    }
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
                if let TrackEvent::PitchBend(pitch_bend) = event {
                    let mut track_event = event.clone();
                    // adjust the position to be relative to the edit cursor
                    track_event.set_position(track_event.position() - edit_cursor_time_in_beats);
                    events_to_copy.push(track_event);
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
                AutomationViewMode::PitchBend => handle_automation_pitch_bend_paste(&mut state, edit_cursor_time_in_beats),
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
    let automation_discrete = state.automation_discrete();

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
                        if let Some(automation) = riff_arrangement.automation_mut(&track_uuid) {
                            if automation_discrete {
                                Some(automation.events_mut())
                            }
                            else {
                                if let Some(automation_type_value) = automation_type {
                                    if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(automation_envelope.events_mut())
                                    } else { None }
                                }
                                else { None }
                            }
                        } else { None }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                match automation_edit_type {
                    AutomationEditType::Track => {
                        let automation = track_type.automation_mut();
                        if automation_discrete {
                            Some(automation.events_mut())
                        }
                        else {
                            if let Some(automation_type_value) = automation_type {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                        if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(automation_envelope.events_mut())
                                } else {
                                    let event_details = PluginParameter {
                                        id: Uuid::new_v4(),
                                        plugin_uuid: plugin_uuid,
                                        instrument: true,
                                        position: 0.0,
                                        index: automation_type_value,
                                        value: 0.0,
                                    };
                                    let mut new_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
                                    automation.envelopes_mut().push(new_envelope);
                                    if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
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
                            if plugin_param.plugin_uuid().to_string() == plugin_uuid.to_string() && plugin_param.index == automation_type_value {
                                let mut track_event = event.clone();

                                track_event.set_id(Uuid::new_v4().to_string());

                                // adjust the position to be relative to the edit cursor
                                track_event.set_position(edit_cursor_time_in_beats + track_event.position());
                                events.push(track_event);
                            }
                        }
                    }
                    events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
                }
            }
        }
    }
}

fn handle_automation_note_expression_paste(state: &mut DAWState, edit_cursor_time_in_beats: f64) {
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
    let note_expression_type = state.note_expression_type().clone();
    let note_expression_id = state.note_expression_id();
    let note_expression_port_index = state.note_expression_port_index() as i16;
    let note_expression_channel = state.note_expression_channel() as i16;
    let note_expression_key = state.note_expression_key();
    let automation_discrete = state.automation_discrete();

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
                        let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            riff_arr_automation
                        } else {
                            riff_arrangement.add_track_automation(track_uuid.clone());
                            riff_arrangement.automation_mut(&track_uuid).unwrap()
                        };
                        if automation_discrete {
                            Some(automation.events_mut())
                        }
                        else {
                            if let Some(automation_type_value) = automation_type {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) as i32 == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(automation_envelope.events_mut())
                                } else {

                                    let event_details = NoteExpression::new_with_params(
                                        note_expression_type,
                                        note_expression_port_index,
                                        note_expression_channel,
                                        0.0,
                                        note_expression_id,
                                        note_expression_key,
                                        0.0
                                    );
                                    let new_envelope = AutomationEnvelope::new(TrackEvent::NoteExpression(event_details));
                                    automation.envelopes_mut().push(new_envelope);
                                    if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                            if
                                            *(note_expression.expression_type()) == note_expression_type &&
                                                note_expression.port() ==  note_expression_port_index &&
                                                note_expression.channel() ==  note_expression_channel &&
                                                note_expression.note_id() ==  note_expression_id &&
                                                note_expression.key() ==  note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
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
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                match automation_edit_type {
                    AutomationEditType::Track => {
                        let automation = track_type.automation_mut();
                        if automation_discrete {
                            Some(automation.events_mut())
                        }
                        else {
                            if let Some(automation_type_value) = automation_type {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) as i32 == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(automation_envelope.events_mut())
                                } else {

                                    let event_details = NoteExpression::new_with_params(
                                        note_expression_type,
                                        note_expression_port_index,
                                        note_expression_channel,
                                        0.0,
                                        note_expression_id,
                                        note_expression_key,
                                        0.0
                                    );
                                    let new_envelope = AutomationEnvelope::new(TrackEvent::NoteExpression(event_details));
                                    automation.envelopes_mut().push(new_envelope);
                                    if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                            if
                                            *(note_expression.expression_type()) == note_expression_type &&
                                                note_expression.port() ==  note_expression_port_index &&
                                                note_expression.channel() ==  note_expression_channel &&
                                                note_expression.note_id() ==  note_expression_id &&
                                                note_expression.key() ==  note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
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

                        track_event.set_id(Uuid::new_v4().to_string());

                        // adjust the position to be relative to the edit cursor
                        track_event.set_position(edit_cursor_time_in_beats + track_event.position());
                        events.push(track_event);
                    }
                }
                events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
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
    let automation_discrete = state.automation_discrete();

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
                            let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                riff_arr_automation
                            } else {
                                riff_arrangement.add_track_automation(track_uuid.clone());
                                riff_arrangement.automation_mut(&track_uuid).unwrap()
                            };
                            if automation_discrete {
                                Some(automation.events_mut())
                            }
                            else {
                                if let Some(automation_type_value) = automation_type {
                                    if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(automation_envelope.events_mut())
                                    } else {
                                        let event_details = PluginParameter {
                                            id: Uuid::new_v4(),
                                            plugin_uuid: Uuid::parse_str(selected_effect_uuid.as_str()).unwrap(),
                                            instrument: true,
                                            position: 0.0,
                                            index: automation_type_value,
                                            value: 0.0,
                                        };
                                        let mut new_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
                                        automation.envelopes_mut().push(new_envelope);
                                        if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                            let mut found = false;
                                            if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                    found = true;
                                                }
                                            }
                                            return found;
                                        }) {
                                            Some(envelope.events_mut())
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
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    match automation_edit_type {
                        AutomationEditType::Track => {
                            let automation = track_type.automation_mut();
                            if automation_discrete {
                                Some(automation.events_mut())
                            }
                            else {
                                if let Some(automation_type_value) = automation_type {
                                    if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(automation_envelope.events_mut())
                                    } else {
                                        let event_details = PluginParameter {
                                            id: Uuid::new_v4(),
                                            plugin_uuid: Uuid::parse_str(selected_effect_uuid.as_str()).unwrap(),
                                            instrument: true,
                                            position: 0.0,
                                            index: automation_type_value,
                                            value: 0.0,
                                        };
                                        let mut new_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
                                        automation.envelopes_mut().push(new_envelope);
                                        if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                            let mut found = false;
                                            if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                    found = true;
                                                }
                                            }
                                            return found;
                                        }) {
                                            Some(envelope.events_mut())
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
                                if plugin_param.plugin_uuid().to_string() == selected_effect_uuid && plugin_param.index == automation_type_value {
                                    let mut track_event = event.clone();

                                    track_event.set_id(Uuid::new_v4().to_string());

                                    // adjust the position to be relative to the edit cursor
                                    track_event.set_position(edit_cursor_time_in_beats + track_event.position());
                                    events.push(track_event);
                                }
                            }
                        }
                        events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
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
    let automation_discrete = state.automation_discrete();

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
                    let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                        riff_arr_automation
                    } else {
                        riff_arrangement.add_track_automation(track_uuid.clone());
                        riff_arrangement.automation_mut(&track_uuid).unwrap()
                    };
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_type_value) = automation_type {
                            if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::Controller(controller) = envelope.event_details() {
                                    if controller.controller() == automation_type_value {
                                        found = true;
                                    }
                                }
                                return found;
                            }) {
                                Some(automation_envelope.events_mut())
                            } else {
                                let event_details = Controller::new(0.0, automation_type_value, 0);
                                let new_envelope = AutomationEnvelope::new(TrackEvent::Controller(event_details));
                                automation.envelopes_mut().push(new_envelope);
                                if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::Controller(controller) = envelope.event_details() {
                                        if controller.controller() == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(envelope.events_mut())
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
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            match automation_edit_type {
                AutomationEditType::Track => {
                    let automation = track_type.automation_mut();
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_type_value) = automation_type {
                            if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::Controller(controller) = envelope.event_details() {
                                    if controller.controller() == automation_type_value {
                                        found = true;
                                    }
                                }
                                return found;
                            }) {
                                Some(automation_envelope.events_mut())
                            } else {
                                let event_details = Controller::new(0.0, automation_type_value, 0);
                                let new_envelope = AutomationEnvelope::new(TrackEvent::Controller(event_details));
                                automation.envelopes_mut().push(new_envelope);
                                if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::Controller(controller) = envelope.event_details() {
                                        if controller.controller() == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(envelope.events_mut())
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

                            track_event.set_id(Uuid::new_v4().to_string());

                            // adjust the position to be relative to the edit cursor
                            track_event.set_position(edit_cursor_time_in_beats + track_event.position());
                            events.push(track_event);
                        }
                    }
                }
                events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
            }
        }
    }
}

fn handle_automation_pitch_bend_paste(state: &mut DAWState, edit_cursor_time_in_beats: f64) {
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
    let automation_discrete = state.automation_discrete();

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
                    let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                        riff_arr_automation
                    } else {
                        riff_arrangement.add_track_automation(track_uuid.clone());
                        riff_arrangement.automation_mut(&track_uuid).unwrap()
                    };
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                            let mut found = false;
                            if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                found = true;
                            }
                            return found;
                        }) {
                            Some(automation_envelope.events_mut())
                        } else {
                            let event_details = PitchBend::new(0.0, 0);
                            let new_envelope = AutomationEnvelope::new(TrackEvent::PitchBend(event_details));
                            automation.envelopes_mut().push(new_envelope);
                            if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                    found = true;
                                }
                                return found;
                            }) {
                                Some(envelope.events_mut())
                            }
                            else {
                                None
                            }
                        }
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
                    let automation = track_type.automation_mut();
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                            let mut found = false;
                            if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                found = true;
                            }
                            return found;
                        }) {
                            Some(automation_envelope.events_mut())
                        } else {
                            let event_details = PitchBend::new(0.0, 0);
                            let new_envelope = AutomationEnvelope::new(TrackEvent::PitchBend(event_details));
                            automation.envelopes_mut().push(new_envelope);
                            if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                    found = true;
                                }
                                return found;
                            }) {
                                Some(envelope.events_mut())
                            }
                            else {
                                None
                            }
                        }
                    }
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
                if let TrackEvent::PitchBend(pitch_bend) = event {
                    let mut track_event = event.clone();

                    track_event.set_id(Uuid::new_v4().to_string());

                    // adjust the position to be relative to the edit cursor
                    track_event.set_position(edit_cursor_time_in_beats + track_event.position());
                    events.push(track_event);
                }
            }
            events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
        }
    }
}


fn handle_automation_quantise(state: &Arc<Mutex<DAWState>>, snap_in_beats: f64, quantise_strength: f64) {
    match state.lock() {
        Ok(mut state) => {
            match state.automation_view_mode() {
                AutomationViewMode::Controllers => handle_automation_controller_quantise(&mut state, snap_in_beats, quantise_strength),
                AutomationViewMode::PitchBend => handle_automation_pitch_bend_quantise(&mut state, snap_in_beats, quantise_strength),
                AutomationViewMode::Instrument => handle_automation_instrument_quantise(&mut state, snap_in_beats, quantise_strength),
                AutomationViewMode::Effect => handle_automation_effect_quantise(&mut state, snap_in_beats, quantise_strength),
                AutomationViewMode::NoteExpression => handle_automation_note_expression_quantise(&mut state, snap_in_beats, quantise_strength),
                _ => (),
            }
        }
        Err(_) => {

        }
    }
}

fn handle_automation_instrument_quantise(state: &mut DAWState, snap_in_beats: f64, quantise_strength: f64) {
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
    let automation_discrete = state.automation_discrete();

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
                        if let Some(automation) = riff_arrangement.automation_mut(&track_uuid) {
                            if automation_discrete {
                                Some(automation.events_mut())
                            }
                            else {
                                if let Some(automation_type_value) = automation_type {
                                    if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(automation_envelope.events_mut())
                                    } else { None }
                                }
                                else { None }
                            }
                        } else { None }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                match automation_edit_type {
                    AutomationEditType::Track => {
                        let automation = track_type.automation_mut();
                        if automation_discrete {
                            Some(automation.events_mut())
                        }
                        else {
                            if let Some(automation_type_value) = automation_type {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                        if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(automation_envelope.events_mut())
                                } else {
                                    let event_details = PluginParameter {
                                        id: Uuid::new_v4(),
                                        plugin_uuid: plugin_uuid,
                                        instrument: true,
                                        position: 0.0,
                                        index: automation_type_value,
                                        value: 0.0,
                                    };
                                    let mut new_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
                                    automation.envelopes_mut().push(new_envelope);
                                    if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
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
                    for event in events.iter_mut().filter(|event| selected.contains(&event.id())) {
                        if let TrackEvent::AudioPluginParameter(plugin_param) = event {
                            if plugin_param.plugin_uuid() == plugin_uuid.to_string() && plugin_param.index == automation_type_value {
                                let calculated_snap = DAWUtils::quantise(plugin_param.position(), snap_in_beats, quantise_strength, false);

                                if calculated_snap.snapped {
                                    plugin_param.set_position(calculated_snap.snapped_value);
                                }
                            }
                        }
                    }
                    events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
                }
            }
        }
    }
}

fn handle_automation_note_expression_quantise(state: &mut DAWState, snap_in_beats: f64, quantise_strength: f64) {
    let selected = state.selected_automation().to_vec();
    let automation_type = state.automation_type();
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
    let automation_discrete = state.automation_discrete();

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
                        let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            riff_arr_automation
                        } else {
                            riff_arrangement.add_track_automation(track_uuid.clone());
                            riff_arrangement.automation_mut(&track_uuid).unwrap()
                        };
                        if automation_discrete {
                            Some(automation.events_mut())
                        }
                        else {
                            if let Some(automation_type_value) = automation_type {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) as i32 == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(automation_envelope.events_mut())
                                } else {

                                    let event_details = NoteExpression::new_with_params(
                                        note_expression_type,
                                        note_expression_port_index,
                                        note_expression_channel,
                                        0.0,
                                        note_expression_id,
                                        note_expression_key,
                                        0.0
                                    );
                                    let new_envelope = AutomationEnvelope::new(TrackEvent::NoteExpression(event_details));
                                    automation.envelopes_mut().push(new_envelope);
                                    if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                            if
                                            *(note_expression.expression_type()) == note_expression_type &&
                                                note_expression.port() ==  note_expression_port_index &&
                                                note_expression.channel() ==  note_expression_channel &&
                                                note_expression.note_id() ==  note_expression_id &&
                                                note_expression.key() ==  note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
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
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                match automation_edit_type {
                    AutomationEditType::Track => {
                        let automation = track_type.automation_mut();
                        if automation_discrete {
                            Some(automation.events_mut())
                        }
                        else {
                            if let Some(automation_type_value) = automation_type {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) as i32 == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(automation_envelope.events_mut())
                                } else {

                                    let event_details = NoteExpression::new_with_params(
                                        note_expression_type,
                                        note_expression_port_index,
                                        note_expression_channel,
                                        0.0,
                                        note_expression_id,
                                        note_expression_key,
                                        0.0
                                    );
                                    let new_envelope = AutomationEnvelope::new(TrackEvent::NoteExpression(event_details));
                                    automation.envelopes_mut().push(new_envelope);
                                    if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                            if
                                            *(note_expression.expression_type()) == note_expression_type &&
                                                note_expression.port() ==  note_expression_port_index &&
                                                note_expression.channel() ==  note_expression_channel &&
                                                note_expression.note_id() ==  note_expression_id &&
                                                note_expression.key() ==  note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
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
                for event in events.iter_mut().filter(|event| selected.contains(&event.id())) {
                    if let TrackEvent::NoteExpression(note_expression) = event {
                        let calculated_snap = DAWUtils::quantise(note_expression.position(), snap_in_beats, quantise_strength, false);

                        if calculated_snap.snapped {
                            note_expression.set_position(calculated_snap.snapped_value);
                        }
                    }
                }
                events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
            }
        }
    }
}

fn handle_automation_effect_quantise(state: &mut DAWState, snap_in_beats: f64, quantise_strength: f64) {
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
    let automation_discrete = state.automation_discrete();

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
                            let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                riff_arr_automation
                            } else {
                                riff_arrangement.add_track_automation(track_uuid.clone());
                                riff_arrangement.automation_mut(&track_uuid).unwrap()
                            };
                            if automation_discrete {
                                Some(automation.events_mut())
                            }
                            else {
                                if let Some(automation_type_value) = automation_type {
                                    if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(automation_envelope.events_mut())
                                    } else {
                                        let event_details = PluginParameter {
                                            id: Uuid::new_v4(),
                                            plugin_uuid: Uuid::parse_str(selected_effect_uuid.as_str()).unwrap(),
                                            instrument: true,
                                            position: 0.0,
                                            index: automation_type_value,
                                            value: 0.0,
                                        };
                                        let mut new_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
                                        automation.envelopes_mut().push(new_envelope);
                                        if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                            let mut found = false;
                                            if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                    found = true;
                                                }
                                            }
                                            return found;
                                        }) {
                                            Some(envelope.events_mut())
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
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    match automation_edit_type {
                        AutomationEditType::Track => {
                            let automation = track_type.automation_mut();
                            if automation_discrete {
                                Some(automation.events_mut())
                            }
                            else {
                                if let Some(automation_type_value) = automation_type {
                                    if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(automation_envelope.events_mut())
                                    } else {
                                        let event_details = PluginParameter {
                                            id: Uuid::new_v4(),
                                            plugin_uuid: Uuid::parse_str(selected_effect_uuid.as_str()).unwrap(),
                                            instrument: true,
                                            position: 0.0,
                                            index: automation_type_value,
                                            value: 0.0,
                                        };
                                        let mut new_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
                                        automation.envelopes_mut().push(new_envelope);
                                        if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                            let mut found = false;
                                            if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                    found = true;
                                                }
                                            }
                                            return found;
                                        }) {
                                            Some(envelope.events_mut())
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
                        for event in events.iter_mut().filter(|event| selected.contains(&event.id())) {
                            if let TrackEvent::AudioPluginParameter(plugin_param) = event {
                                if plugin_param.plugin_uuid().to_string() == selected_effect_uuid && plugin_param.index == automation_type_value {
                                    let calculated_snap = DAWUtils::quantise(plugin_param.position(), snap_in_beats, quantise_strength, false);

                                    if calculated_snap.snapped {
                                        plugin_param.set_position(calculated_snap.snapped_value);
                                    }
                                }
                            }
                        }
                        events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
                    }
                }
            }
        }
    }
}

fn handle_automation_controller_quantise(state: &mut DAWState, snap_in_beats: f64, quantise_strength: f64) {
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
    let automation_discrete = state.automation_discrete();

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
                    let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                        riff_arr_automation
                    } else {
                        riff_arrangement.add_track_automation(track_uuid.clone());
                        riff_arrangement.automation_mut(&track_uuid).unwrap()
                    };
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_type_value) = automation_type {
                            if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::Controller(controller) = envelope.event_details() {
                                    if controller.controller() == automation_type_value {
                                        found = true;
                                    }
                                }
                                return found;
                            }) {
                                Some(automation_envelope.events_mut())
                            } else {
                                let event_details = Controller::new(0.0, automation_type_value, 0);
                                let new_envelope = AutomationEnvelope::new(TrackEvent::Controller(event_details));
                                automation.envelopes_mut().push(new_envelope);
                                if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::Controller(controller) = envelope.event_details() {
                                        if controller.controller() == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(envelope.events_mut())
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
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            match automation_edit_type {
                AutomationEditType::Track => {
                    let automation = track_type.automation_mut();
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_type_value) = automation_type {
                            if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::Controller(controller) = envelope.event_details() {
                                    if controller.controller() == automation_type_value {
                                        found = true;
                                    }
                                }
                                return found;
                            }) {
                                Some(automation_envelope.events_mut())
                            } else {
                                let event_details = Controller::new(0.0, automation_type_value, 0);
                                let new_envelope = AutomationEnvelope::new(TrackEvent::Controller(event_details));
                                automation.envelopes_mut().push(new_envelope);
                                if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::Controller(controller) = envelope.event_details() {
                                        if controller.controller() == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(envelope.events_mut())
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
                for event in events.iter_mut().filter(|event| selected.contains(&event.id())) {
                    if let TrackEvent::Controller(controller) = event {
                        if controller.controller() == automation_type_value {
                            let calculated_snap = DAWUtils::quantise(controller.position(), snap_in_beats, quantise_strength, false);

                            if calculated_snap.snapped {
                                controller.set_position(calculated_snap.snapped_value);
                            }
                        }
                    }
                }
                events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
            }
        }
    }
}

fn handle_automation_pitch_bend_quantise(state: &mut DAWState, snap_in_beats: f64, quantise_strength: f64) {
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
    let automation_discrete = state.automation_discrete();

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
                    let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                        riff_arr_automation
                    } else {
                        riff_arrangement.add_track_automation(track_uuid.clone());
                        riff_arrangement.automation_mut(&track_uuid).unwrap()
                    };
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                            let mut found = false;
                            if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                found = true;
                            }
                            return found;
                        }) {
                            Some(automation_envelope.events_mut())
                        } else {
                            let event_details = PitchBend::new(0.0, 0);
                            let new_envelope = AutomationEnvelope::new(TrackEvent::PitchBend(event_details));
                            automation.envelopes_mut().push(new_envelope);
                            if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                    found = true;
                                }
                                return found;
                            }) {
                                Some(envelope.events_mut())
                            }
                            else {
                                None
                            }
                        }
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
                    let automation = track_type.automation_mut();
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                            let mut found = false;
                            if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                found = true;
                            }
                            return found;
                        }) {
                            Some(automation_envelope.events_mut())
                        } else {
                            let event_details = PitchBend::new(0.0, 0);
                            let new_envelope = AutomationEnvelope::new(TrackEvent::PitchBend(event_details));
                            automation.envelopes_mut().push(new_envelope);
                            if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                    found = true;
                                }
                                return found;
                            }) {
                                Some(envelope.events_mut())
                            }
                            else {
                                None
                            }
                        }
                    }
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
            for event in events.iter_mut().filter(|event| selected.contains(&event.id())) {
                if let TrackEvent::PitchBend(pitch_bend) = event {
                    let calculated_snap = DAWUtils::quantise(pitch_bend.position(), snap_in_beats, quantise_strength, false);

                    if calculated_snap.snapped {
                        pitch_bend.set_position(calculated_snap.snapped_value);
                    }
                }
            }
            events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
        }
    }
}




























fn handle_automation_change(state: &Arc<Mutex<DAWState>>, changed_events: Vec<(TrackEvent, TrackEvent)>) {
    match state.lock() {
        Ok(mut state) => {
            match state.automation_view_mode() {
                AutomationViewMode::Controllers => handle_automation_controller_change(&mut state, changed_events),
                AutomationViewMode::PitchBend => handle_automation_pitch_bend_change(&mut state, changed_events),
                AutomationViewMode::Instrument => handle_automation_instrument_change(&mut state, changed_events),
                AutomationViewMode::Effect => handle_automation_effect_change(&mut state, changed_events),
                AutomationViewMode::NoteExpression => handle_automation_note_expression_change(&mut state, changed_events),
                _ => (),
            }
        }
        Err(_) => {

        }
    }
}

fn handle_automation_instrument_change(state: &mut DAWState, changed_events: Vec<(TrackEvent, TrackEvent)>) {
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
    let automation_discrete = state.automation_discrete();

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
                        if let Some(automation) = riff_arrangement.automation_mut(&track_uuid) {
                            if automation_discrete {
                                Some(automation.events_mut())
                            }
                            else {
                                if let Some(automation_type_value) = automation_type {
                                    if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(automation_envelope.events_mut())
                                    } else { None }
                                }
                                else { None }
                            }
                        } else { None }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                match automation_edit_type {
                    AutomationEditType::Track => {
                        let automation = track_type.automation_mut();
                        if automation_discrete {
                            Some(automation.events_mut())
                        }
                        else {
                            if let Some(automation_type_value) = automation_type {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                        if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(automation_envelope.events_mut())
                                } else {
                                    let event_details = PluginParameter {
                                        id: Uuid::new_v4(),
                                        plugin_uuid: plugin_uuid,
                                        instrument: true,
                                        position: 0.0,
                                        index: automation_type_value,
                                        value: 0.0,
                                    };
                                    let mut new_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
                                    automation.envelopes_mut().push(new_envelope);
                                    if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
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
                for (_, changed) in changed_events.iter() {
                    if let Some(event) = events.iter_mut().find(|event| changed.id() == event.id()) {
                        if let TrackEvent::AudioPluginParameter(change) = changed {
                            if let TrackEvent::AudioPluginParameter(plugin_param) = event {
                                plugin_param.set_position(change.position());
                                plugin_param.set_value(change.value());
                            }
                        }
                    }
                }
                events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
            }
        }
    }
}

fn handle_automation_note_expression_change(state: &mut DAWState, changed_events: Vec<(TrackEvent, TrackEvent)>) {
    let selected = state.selected_automation().to_vec();
    let automation_type = state.automation_type();
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
    let automation_discrete = state.automation_discrete();

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
                        let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            riff_arr_automation
                        } else {
                            riff_arrangement.add_track_automation(track_uuid.clone());
                            riff_arrangement.automation_mut(&track_uuid).unwrap()
                        };
                        if automation_discrete {
                            Some(automation.events_mut())
                        }
                        else {
                            if let Some(automation_type_value) = automation_type {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) as i32 == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(automation_envelope.events_mut())
                                } else {

                                    let event_details = NoteExpression::new_with_params(
                                        note_expression_type,
                                        note_expression_port_index,
                                        note_expression_channel,
                                        0.0,
                                        note_expression_id,
                                        note_expression_key,
                                        0.0
                                    );
                                    let new_envelope = AutomationEnvelope::new(TrackEvent::NoteExpression(event_details));
                                    automation.envelopes_mut().push(new_envelope);
                                    if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                            if
                                            *(note_expression.expression_type()) == note_expression_type &&
                                                note_expression.port() ==  note_expression_port_index &&
                                                note_expression.channel() ==  note_expression_channel &&
                                                note_expression.note_id() ==  note_expression_id &&
                                                note_expression.key() ==  note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
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
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                match automation_edit_type {
                    AutomationEditType::Track => {
                        let automation = track_type.automation_mut();
                        if automation_discrete {
                            Some(automation.events_mut())
                        }
                        else {
                            if let Some(automation_type_value) = automation_type {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) as i32 == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(automation_envelope.events_mut())
                                } else {

                                    let event_details = NoteExpression::new_with_params(
                                        note_expression_type,
                                        note_expression_port_index,
                                        note_expression_channel,
                                        0.0,
                                        note_expression_id,
                                        note_expression_key,
                                        0.0
                                    );
                                    let new_envelope = AutomationEnvelope::new(TrackEvent::NoteExpression(event_details));
                                    automation.envelopes_mut().push(new_envelope);
                                    if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                            if
                                            *(note_expression.expression_type()) == note_expression_type &&
                                                note_expression.port() ==  note_expression_port_index &&
                                                note_expression.channel() ==  note_expression_channel &&
                                                note_expression.note_id() ==  note_expression_id &&
                                                note_expression.key() ==  note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
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
                for (_, changed) in changed_events.iter() {
                    if let Some(event) = events.iter_mut().find(|event| changed.id() == event.id()) {
                        if let TrackEvent::AudioPluginParameter(change) = changed {
                            if let TrackEvent::NoteExpression(note_expression) = event {
                                note_expression.set_position(change.position());
                                note_expression.set_value(change.value() as f64);
                            }
                        }
                    }
                }
                events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
            }
        }
    }
}

fn handle_automation_effect_change(state: &mut DAWState, changed_events: Vec<(TrackEvent, TrackEvent)>) {
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
    let automation_discrete = state.automation_discrete();

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
                            let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                riff_arr_automation
                            } else {
                                riff_arrangement.add_track_automation(track_uuid.clone());
                                riff_arrangement.automation_mut(&track_uuid).unwrap()
                            };
                            if automation_discrete {
                                Some(automation.events_mut())
                            }
                            else {
                                if let Some(automation_type_value) = automation_type {
                                    if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(automation_envelope.events_mut())
                                    } else {
                                        let event_details = PluginParameter {
                                            id: Uuid::new_v4(),
                                            plugin_uuid: Uuid::parse_str(selected_effect_uuid.as_str()).unwrap(),
                                            instrument: true,
                                            position: 0.0,
                                            index: automation_type_value,
                                            value: 0.0,
                                        };
                                        let mut new_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
                                        automation.envelopes_mut().push(new_envelope);
                                        if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                            let mut found = false;
                                            if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                    found = true;
                                                }
                                            }
                                            return found;
                                        }) {
                                            Some(envelope.events_mut())
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
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    match automation_edit_type {
                        AutomationEditType::Track => {
                            let automation = track_type.automation_mut();
                            if automation_discrete {
                                Some(automation.events_mut())
                            }
                            else {
                                if let Some(automation_type_value) = automation_type {
                                    if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(automation_envelope.events_mut())
                                    } else {
                                        let event_details = PluginParameter {
                                            id: Uuid::new_v4(),
                                            plugin_uuid: Uuid::parse_str(selected_effect_uuid.as_str()).unwrap(),
                                            instrument: true,
                                            position: 0.0,
                                            index: automation_type_value,
                                            value: 0.0,
                                        };
                                        let mut new_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
                                        automation.envelopes_mut().push(new_envelope);
                                        if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                            let mut found = false;
                                            if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
                                                    found = true;
                                                }
                                            }
                                            return found;
                                        }) {
                                            Some(envelope.events_mut())
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
                        for (_, changed) in changed_events.iter() {
                            if let Some(event) = events.iter_mut().find(|event| changed.id() == event.id()) {
                                if let TrackEvent::AudioPluginParameter(change) = changed {
                                    if let TrackEvent::AudioPluginParameter(plugin_param) = event {
                                        plugin_param.set_position(change.position());
                                        plugin_param.set_value(change.value());
                                    }
                                }
                            }
                        }
                        events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
                    }
                }
            }
        }
    }
}

fn handle_automation_controller_change(state: &mut DAWState, changed_events: Vec<(TrackEvent, TrackEvent)>) {
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
    let automation_discrete = state.automation_discrete();

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
                    let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                        riff_arr_automation
                    } else {
                        riff_arrangement.add_track_automation(track_uuid.clone());
                        riff_arrangement.automation_mut(&track_uuid).unwrap()
                    };
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_type_value) = automation_type {
                            if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::Controller(controller) = envelope.event_details() {
                                    if controller.controller() == automation_type_value {
                                        found = true;
                                    }
                                }
                                return found;
                            }) {
                                Some(automation_envelope.events_mut())
                            } else {
                                let event_details = Controller::new(0.0, automation_type_value, 0);
                                let new_envelope = AutomationEnvelope::new(TrackEvent::Controller(event_details));
                                automation.envelopes_mut().push(new_envelope);
                                if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::Controller(controller) = envelope.event_details() {
                                        if controller.controller() == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(envelope.events_mut())
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
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            match automation_edit_type {
                AutomationEditType::Track => {
                    let automation = track_type.automation_mut();
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_type_value) = automation_type {
                            if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::Controller(controller) = envelope.event_details() {
                                    if controller.controller() == automation_type_value {
                                        found = true;
                                    }
                                }
                                return found;
                            }) {
                                Some(automation_envelope.events_mut())
                            } else {
                                let event_details = Controller::new(0.0, automation_type_value, 0);
                                let new_envelope = AutomationEnvelope::new(TrackEvent::Controller(event_details));
                                automation.envelopes_mut().push(new_envelope);
                                if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::Controller(controller) = envelope.event_details() {
                                        if controller.controller() == automation_type_value {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(envelope.events_mut())
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
                for (_, changed) in changed_events.iter() {
                    if let Some(event) = events.iter_mut().find(|event| changed.id() == event.id()) {
                        if let TrackEvent::AudioPluginParameter(change) = changed {
                            if let TrackEvent::Controller(controller) = event {
                                controller.set_position(change.position());
                                controller.set_value(change.value() as i32);
                            }
                        }
                    }
                }
                events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
            }
        }
    }
}

fn handle_automation_pitch_bend_change(state: &mut DAWState, changed_events: Vec<(TrackEvent, TrackEvent)>) {
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
    let automation_discrete = state.automation_discrete();

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
                    let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                        riff_arr_automation
                    } else {
                        riff_arrangement.add_track_automation(track_uuid.clone());
                        riff_arrangement.automation_mut(&track_uuid).unwrap()
                    };
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                            let mut found = false;
                            if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                found = true;
                            }
                            return found;
                        }) {
                            Some(automation_envelope.events_mut())
                        } else {
                            let event_details = PitchBend::new(0.0, 0);
                            let new_envelope = AutomationEnvelope::new(TrackEvent::PitchBend(event_details));
                            automation.envelopes_mut().push(new_envelope);
                            if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                    found = true;
                                }
                                return found;
                            }) {
                                Some(envelope.events_mut())
                            }
                            else {
                                None
                            }
                        }
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
                    let automation = track_type.automation_mut();
                    if automation_discrete {
                        Some(automation.events_mut())
                    }
                    else {
                        if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                            let mut found = false;
                            if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                found = true;
                            }
                            return found;
                        }) {
                            Some(automation_envelope.events_mut())
                        } else {
                            let event_details = PitchBend::new(0.0, 0);
                            let new_envelope = AutomationEnvelope::new(TrackEvent::PitchBend(event_details));
                            automation.envelopes_mut().push(new_envelope);
                            if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                let mut found = false;
                                if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                    found = true;
                                }
                                return found;
                            }) {
                                Some(envelope.events_mut())
                            }
                            else {
                                None
                            }
                        }
                    }
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
            for (_, changed) in changed_events.iter() {
                if let Some(event) = events.iter_mut().find(|event| changed.id() == event.id()) {
                    if let TrackEvent::AudioPluginParameter(change) = changed {
                        if let TrackEvent::PitchBend(pitch_bend) = event {
                            pitch_bend.set_position(change.position());
                            pitch_bend.set_value(change.value() as i32);
                        }
                    }
                }
            }
            events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
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

fn create_jack_time_critical_event_processing_thread(
    tx_from_ui: Sender<DAWEvents>,
    jack_time_critical_midi_receiver: Receiver<AudioLayerTimeCriticalOutwardEvent>,
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
                    match jack_time_critical_midi_receiver.try_recv() {
                        Ok(audio_layer_outward_event) => {
                            match audio_layer_outward_event {
                                AudioLayerTimeCriticalOutwardEvent::MidiEvent(jack_midi_event) => {
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
                                                            } else if (128..=143).contains(&midi_msg_type) {
                                                                state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::StopNoteImmediate(jack_midi_event.data[1] as i32, midi_channel));
                                                            } else if (176..=191).contains(&midi_msg_type) {
                                                                state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::PlayControllerImmediate(jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32, midi_channel));
                                                            } else if (224..=239).contains(&midi_msg_type) {
                                                                state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::PlayPitchBendImmediate(jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32, midi_channel));
                                                            } else {
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
                                    }
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
                                    }
                                    match state.lock() {
                                        Ok(mut state) => {
                                            let tempo = state.project().song().tempo();
                                            let sample_rate = state.configuration.audio.sample_rate as f64;;
                                            let current_view = state.current_view().clone();
                                            let selected_riff_arrangement_uuid = if let Some(selected_riff_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                                                Some(selected_riff_arrangement_uuid.to_string())
                                            }
                                            else {
                                                None
                                            };
                                            let playing = state.playing();
                                            let recording = state.recording();

                                            if playing && recording {
                                                let play_mode = state.play_mode();
                                                let playing_riff_set = state.playing_riff_set().clone();
                                                let mut riff_changed = false;

                                                match selected_riff_track_uuid {
                                                    Some(track_uuid) => {
                                                        match state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == *track_uuid) {
                                                            Some(track_type) => match track_type {
                                                                TrackType::InstrumentTrack(track) => {
                                                                    match current_view {
                                                                        CurrentView::Track => {
                                                                            match selected_riff_uuid {
                                                                                Some(uuid) => {
                                                                                    for riff in track.riffs_mut().iter_mut() {
                                                                                        if riff.uuid().to_string() == *uuid {
                                                                                            if (144..=159).contains(&midi_msg_type) { //note on
                                                                                                let actual_position = tempo / 60.0 * jack_midi_event.delta_frames as f64 / sample_rate;
                                                                                                let adjusted_position = ((actual_position * 1000.0) as i32 % ((riff.length() * 1000.0) as i32)) as f64 / 1000.0;
                                                                                                debug!(
                                                                                                    "Adding note to riff: delta frames={}, actual_position={}, adjusted_position={}, note={}, velocity={}",
                                                                                                    jack_midi_event.delta_frames,
                                                                                                    actual_position,
                                                                                                    adjusted_position,
                                                                                                    jack_midi_event.data[1] as i32,
                                                                                                    jack_midi_event.data[2] as i32);
                                                                                                let note = Note::new_with_params(
                                                                                                    MidiPolyphonicExpressionNoteId::ALL as i32, adjusted_position, jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32, 0.2);
                                                                                                recorded_playing_notes.insert(note.note(), note.position());
                                                                                                riff.events_mut().push(TrackEvent::Note(note));
                                                                                                riff.events_mut().sort_by(|a, b| a.position().partial_cmp(&b.position()).unwrap());
                                                                                            } else if (128..=143).contains(&midi_msg_type) { // note off
                                                                                                let note_number = jack_midi_event.data[1] as i32;
                                                                                                if let Some(note_position) = recorded_playing_notes.get_mut(&note_number) {
                                                                                                    let actual_position = tempo / 60.0 * jack_midi_event.delta_frames as f64 / sample_rate;
                                                                                                    let adjusted_position = ((actual_position * 1000.0) as i32 % ((riff.length() * 1000.0) as i32)) as f64 / 1000.0;
                                                                                                    // find the note in the riff
                                                                                                    for track_event in riff.events_mut().iter_mut() {
                                                                                                        if track_event.position() == *note_position {
                                                                                                            if let TrackEvent::Note(note) = track_event {
                                                                                                                if note.note() == note_number {
                                                                                                                    note.set_length(adjusted_position - note.position());
                                                                                                                    riff_changed = true;
                                                                                                                    break;
                                                                                                                }
                                                                                                            }
                                                                                                        }
                                                                                                    }
                                                                                                }
                                                                                                recorded_playing_notes.remove(&note_number);
                                                                                            }

                                                                                            break;
                                                                                        }
                                                                                    }
                                                                                },
                                                                                None => debug!("Jack midi receiver - no selected riff."),
                                                                            }
                                                                            // add the controller events to the track automation
                                                                            if (176..=191).contains(&midi_msg_type) { // Controller - including modulation wheel
                                                                                debug!("Adding controller to track automation: delta frames={}, controller={}, value={}", jack_midi_event.delta_frames, jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32);
                                                                                track.automation_mut().events_mut().push(
                                                                                    TrackEvent::Controller(
                                                                                        Controller::new(
                                                                                            tempo / 60.0 * jack_midi_event.delta_frames as f64 / sample_rate, jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32)));
                                                                                track.automation_mut().events_mut().sort_by(|a, b| a.position().partial_cmp(&b.position()).unwrap());
                                                                            } else if (224..=239).contains(&midi_msg_type) {
                                                                                debug!("Adding pitch bend to track_automation: delta frames={}, lsb={}, msb={}", jack_midi_event.delta_frames, jack_midi_event.data[1], jack_midi_event.data[2]);
                                                                                track.automation_mut().events_mut().push(
                                                                                    TrackEvent::PitchBend(
                                                                                        PitchBend::new_from_midi_bytes(
                                                                                            tempo / 60.0 * jack_midi_event.delta_frames as f64 / sample_rate, jack_midi_event.data[1], jack_midi_event.data[2])));
                                                                                track.automation_mut().events_mut().sort_by(|a, b| a.position().partial_cmp(&b.position()).unwrap());
                                                                            }
                                                                        }
                                                                        CurrentView::RiffSet => {
                                                                            match selected_riff_uuid {
                                                                                Some(uuid) => {
                                                                                    for riff in track.riffs_mut().iter_mut() {
                                                                                        if riff.uuid().to_string() == *uuid {
                                                                                            if (144..=159).contains(&midi_msg_type) { //note on
                                                                                                let actual_position = tempo / 60.0 * jack_midi_event.delta_frames as f64 / sample_rate;
                                                                                                let adjusted_position = ((actual_position * 1000.0) as i32 % ((riff.length() * 1000.0) as i32)) as f64 / 1000.0;
                                                                                                debug!(
                                                                                                    "Adding note to riff: delta frames={}, actual_position={}, adjusted_position={}, note={}, velocity={}",
                                                                                                    jack_midi_event.delta_frames,
                                                                                                    actual_position,
                                                                                                    adjusted_position,
                                                                                                    jack_midi_event.data[1] as i32,
                                                                                                    jack_midi_event.data[2] as i32);
                                                                                                let note = Note::new_with_params(
                                                                                                    MidiPolyphonicExpressionNoteId::ALL as i32, adjusted_position, jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32, 0.2);
                                                                                                recorded_playing_notes.insert(note.note(), note.position());
                                                                                                riff.events_mut().push(TrackEvent::Note(note));
                                                                                                riff.events_mut().sort_by(|a, b| a.position().partial_cmp(&b.position()).unwrap());
                                                                                            } else if (128..=143).contains(&midi_msg_type) { // note off
                                                                                                let note_number = jack_midi_event.data[1] as i32;
                                                                                                if let Some(note_position) = recorded_playing_notes.get_mut(&note_number) {
                                                                                                    let actual_position = tempo / 60.0 * jack_midi_event.delta_frames as f64 / sample_rate;
                                                                                                    let adjusted_position = ((actual_position * 1000.0) as i32 % ((riff.length() * 1000.0) as i32)) as f64 / 1000.0;
                                                                                                    // find the note in the riff
                                                                                                    for track_event in riff.events_mut().iter_mut() {
                                                                                                        if track_event.position() == *note_position {
                                                                                                            if let TrackEvent::Note(note) = track_event {
                                                                                                                if note.note() == note_number {
                                                                                                                    note.set_length(adjusted_position - note.position());
                                                                                                                    riff_changed = true;
                                                                                                                    break;
                                                                                                                }
                                                                                                            }
                                                                                                        }
                                                                                                    }
                                                                                                }
                                                                                                recorded_playing_notes.remove(&note_number);
                                                                                            } else if (176..=191).contains(&midi_msg_type) { // Controller - including modulation wheel
                                                                                                debug!("Adding controller to riff: delta frames={}, controller={}, value={}", jack_midi_event.delta_frames, jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32);
                                                                                                riff.events_mut().push(
                                                                                                    TrackEvent::Controller(
                                                                                                        Controller::new(
                                                                                                            tempo / 60.0 * jack_midi_event.delta_frames as f64 / sample_rate, jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32)));
                                                                                                riff.events_mut().sort_by(|a, b| a.position().partial_cmp(&b.position()).unwrap());
                                                                                            } else if (224..=239).contains(&midi_msg_type) {
                                                                                                debug!("Adding pitch bend to riff: delta frames={}, lsb={}, msb={}", jack_midi_event.delta_frames, jack_midi_event.data[1], jack_midi_event.data[2]);
                                                                                                riff.events_mut().push(
                                                                                                    TrackEvent::PitchBend(
                                                                                                        PitchBend::new_from_midi_bytes(
                                                                                                            tempo / 60.0 * jack_midi_event.delta_frames as f64 / sample_rate, jack_midi_event.data[1], jack_midi_event.data[2])));
                                                                                                riff.events_mut().sort_by(|a, b| a.position().partial_cmp(&b.position()).unwrap());
                                                                                            }

                                                                                            break;
                                                                                        }
                                                                                    }
                                                                                },
                                                                                None => debug!("Jack midi receiver - no selected riff."),
                                                                            }
                                                                        }
                                                                        CurrentView::RiffSequence => {
                                                                            // not doing anything for sequences at this point in time
                                                                        }
                                                                        CurrentView::RiffArrangement => {
                                                                            if let Some(selected_riff_arrangement_uuid) = selected_riff_arrangement_uuid {
                                                                                if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangements_mut().iter_mut().find(|riff_arrangement| riff_arrangement.uuid() == selected_riff_arrangement_uuid.to_string()) {
                                                                                    let track_automation = if let Some(track_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                                                                        track_automation
                                                                                    }
                                                                                    else {
                                                                                        riff_arrangement.add_track_automation(track_uuid.clone());
                                                                                        riff_arrangement.automation_mut(&track_uuid).unwrap()
                                                                                    };

                                                                                    if (176..=191).contains(&midi_msg_type) { // Controller - including modulation wheel
                                                                                        debug!("Adding controller to riff arrangement: delta frames={}, controller={}, value={}", jack_midi_event.delta_frames, jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32);
                                                                                        track_automation.events_mut().push(
                                                                                            TrackEvent::Controller(
                                                                                                Controller::new(
                                                                                                    tempo / 60.0 * jack_midi_event.delta_frames as f64 / sample_rate, jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32)));
                                                                                        track_automation.events_mut().sort_by(|a, b| a.position().partial_cmp(&b.position()).unwrap());
                                                                                    } else if (224..=239).contains(&midi_msg_type) {
                                                                                        debug!("Adding pitch bend to riff arrangement: delta frames={}, lsb={}, msb={}", jack_midi_event.delta_frames, jack_midi_event.data[1], jack_midi_event.data[2]);
                                                                                        track_automation.events_mut().push(
                                                                                            TrackEvent::PitchBend(
                                                                                                PitchBend::new_from_midi_bytes(
                                                                                                    tempo / 60.0 * jack_midi_event.delta_frames as f64 / sample_rate, jack_midi_event.data[1], jack_midi_event.data[2])));
                                                                                        track_automation.events_mut().sort_by(|a, b| a.position().partial_cmp(&b.position()).unwrap());
                                                                                    }
                                                                                }
                                                                            }
                                                                        }
                                                                        CurrentView::RiffGrid => {
                                                                            // not doing anything for riff grids at this point in time
                                                                        }
                                                                    }
                                                                },
                                                                TrackType::AudioTrack(_) => (),
                                                                TrackType::MidiTrack(_) => (),
                                                            },
                                                            None => (),
                                                        }

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
                                    }
                                }
                                AudioLayerTimeCriticalOutwardEvent::TrackVolumePanLevel(jack_midi_event) => {
                                    match state.lock() {
                                        Ok(mut state) => {
                                            if jack_midi_event.data[0] as i32 >= 176 && (jack_midi_event.data[0] as i32 <= (176 + 15)) {
                                                debug!("Main - jack_event_prcessing_thread processing loop - jack AudioLayerTimeCriticalOutwardEvent::TrackVolumePanLevel - received a controller message: {} {} {}", jack_midi_event.data[0], jack_midi_event.data[1], jack_midi_event.data[2]);
                                                // need to send some track volume (176) or pan (177) messages
                                                let position_in_frames = jack_midi_event.delta_frames;
                                                let position_in_beats = (position_in_frames as f64) / state.configuration.audio.sample_rate as f64 * state.project().song().tempo() / 60.0;
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
                                                debug!("Main - jack_event_prcessing_thread processing loop - jack AudioLayerTimeCriticalOutwardEvent::TrackVolumePanLevel - received a unknown message: {} {} {}", jack_midi_event.data[0], jack_midi_event.data[1], jack_midi_event.data[2]);
                                            }
                                        }
                                        Err(_) => {}
                                    }
                                }
                            }
                        }
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
                       jack_time_critical_midi_sender: &Sender<AudioLayerTimeCriticalOutwardEvent>,
                       jack_audio_coast: &Arc<Mutex<TrackBackgroundProcessorMode>>,
                       gui: &mut MainWindow,
                       vst_host_time_info: &Arc<RwLock<TimeInfo>>,
) {
    match jack_midi_receiver.try_recv() {
        Ok(audio_layer_outward_event) => {
            match audio_layer_outward_event {
                AudioLayerOutwardEvent::PlayPositionInFrames(play_position_in_frames) => {
                    match state.lock() {
                        Ok(mut state) => {
                            let bpm = state.get_project().song().tempo();
                            let time_signature_numerator = state.get_project().song().time_signature_numerator();
                            let sample_rate = state.configuration.audio.sample_rate as f64;
                            let play_position_in_beats = play_position_in_frames as f64 / sample_rate * bpm / 60.0;

                            let current_bar = play_position_in_beats as i32 / time_signature_numerator as i32 + 1;
                            let current_beat_in_bar = play_position_in_beats as i32 % time_signature_numerator as i32 + 1;
                            gui.ui.song_position_txt_ctrl.set_label(format!("{:03}:{:03}:000", current_bar, current_beat_in_bar).as_str());

                            let time_in_secs = play_position_in_frames as f64 / sample_rate;
                            let minutes = time_in_secs as i32 / 60;
                            let seconds = time_in_secs as i32 % 60;
                            let milli_seconds = ((time_in_secs - (time_in_secs as u64) as f64) * 1000.0) as u64;
                            gui.ui.song_time_txt_ctrl.set_label(format!("{:03}:{:02}:{:03}", minutes, seconds, milli_seconds).as_str());

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
                            else if let Some(_) = state.playing_riff_grid() {
                                gui.repaint_riff_grid_view_drawing_area(play_position_in_beats);
                            }
                            else if let Some(riff_arrangement_uuid) = state.playing_riff_arrangement() {
                                if let Some(playing_riff_arrangement_summary_data) = state.playing_riff_arrangement_summary_data() {
                                    gui.repaint_riff_arrangement_view_active_drawing_areas(riff_arrangement_uuid.as_str(), play_position_in_beats, playing_riff_arrangement_summary_data);
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
                    debug!("Midi generic MMC event: ");
                    let command_byte = mmc_sysex_bytes[4];
                    match command_byte {
                        1 => { let _ = tx_from_ui.send(DAWEvents::TransportStop); }
                        2 => { let _ = tx_from_ui.send(DAWEvents::TransportPlay); }
                        4 => { let _ = tx_from_ui.send(DAWEvents::TransportMoveForward); }
                        5 => { let _ = tx_from_ui.send(DAWEvents::TransportMoveBack); }
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
                            state.restart_jack(rx_to_audio.clone(), jack_midi_sender.clone(), jack_midi_sender_ui.clone(), jack_time_critical_midi_sender.clone(), jack_audio_coast.clone(), vst_host_time_info.clone());
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
                            let play_position_in_beats = state.play_position_in_frames() as f64 / state.configuration.audio.sample_rate as f64 * state.project().song().tempo() / 60.0;
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

            // if playing and recording add automation to the correct domain entity based on the current view.
            if state.playing() && state.recording() {
                if let Some(event) = automation_event {
                    let current_view = state.current_view().clone();
                    let selected_riff_arrangement_uuid = if let Some(selected_riff_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                        Some(selected_riff_arrangement_uuid.to_string())
                    }
                    else {
                        None
                    };
                    if let CurrentView::RiffArrangement = current_view {
                        if let Some(selected_riff_arrangement_uuid) = selected_riff_arrangement_uuid {
                            if let Some(riff_arrangement) = state.get_project().song_mut().riff_arrangement_mut(selected_riff_arrangement_uuid) {
                                let automation = if let Some(automation) = riff_arrangement.automation_mut(&automation_track_uuid) {
                                    automation
                                }
                                else {
                                    riff_arrangement.add_track_automation(automation_track_uuid.clone());
                                    riff_arrangement.automation_mut(&automation_track_uuid).unwrap()
                                };

                                automation.events_mut().push(event);
                                automation.events_mut().sort_by(|a, b| a.position().partial_cmp(&b.position()).unwrap());
                            }
                        }
                    }
                    else if let CurrentView::Track = current_view {
                        if let Some(track_type) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track_type| track_type.uuid().to_string() == automation_track_uuid) {
                            match track_type {
                                TrackType::InstrumentTrack(track) => {
                                    track.automation_mut().events_mut().push(event);
                                    track.automation_mut().events_mut().sort_by(|a, b| a.position().partial_cmp(&b.position()).unwrap());
                                }
                                TrackType::AudioTrack(track) => {
                                    track.automation_mut().events_mut().push(event);
                                    track.automation_mut().events_mut().sort_by(|a, b| a.position().partial_cmp(&b.position()).unwrap());
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }

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
    let piano_roll_grid_vertical_scroll_position = range_max - (note_vertical_position_on_grid / piano_roll_drawing_area_height * range_max);

    piano_roll_vertical_adj.set_value(piano_roll_grid_vertical_scroll_position);
}
