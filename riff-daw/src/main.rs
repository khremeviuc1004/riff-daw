mod constants;
mod domain;
mod event;
mod utils;
mod state;
mod views;
mod icons;
mod audio;
mod audio_plugin_util;

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::DerefMut;
use std::os::fd::AsRawFd;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use flexi_logger::Logger;
use itertools::Itertools;
use log::debug;
use masonry::properties::{Background, ContentColor, CornerRadius, Padding};
use masonry::properties::types::{AsUnit, CrossAxisAlignment, MainAxisAlignment};
use masonry::theme::default_property_set;
use masonry_winit::winit::error::EventLoopError;
use uuid::Uuid;
use xilem::{EventLoop, EventLoopBuilder, WidgetView, WindowId, Xilem, window, WindowView, Color, dpi};
use xilem::palette::css::WHITE;
use xilem::style::Style;
use xilem::view::{flex_col, flex_row, label, text_button, text_input, worker, FlexSequence, FlexSpacer, GridExt};
use xilem_core::{fork, View, ViewMarker};
use crate::actions::{midi_AudioLayerOutwardEvent_GeneralMMCEvent, midi_AudioLayerOutwardEvent_MidiControlEvent, midi_AudioLayerOutwardEvent_PlayPositionInFrames, midi_AudioLayerTimeCriticalOutwardEvent_MidiEvent, track_change_type_Pan, track_change_type_Volume};
use crate::domain::{Project, NoteExpressionType, PlayMode, Track, TrackBackgroundProcessor, DAWConfiguration, TrackBackgroundProcessorOutwardEvent, TrackType, PluginParameterDetail};
use crate::event::{AudioLayerEvent, AudioLayerOutwardEvent, AudioLayerTimeCriticalOutwardEvent, AutomationEditType, DAWEvents, GeneralTrackType, OperationModeType, TrackChangeType};
use crate::state::{AutomationViewMode, EventEditView, MidiPolyphonicExpressionNoteId, RiffDAWMainView, RiffDAWState, RiffView};
use crate::audio_layer_manager::AudioLayerManager;
use crate::constants::MUSICAL_ITEM_LENGTH_OPTIONS;
use crate::history::HistoryManager;
use crate::views::{close_dialog, dialog_view, main_view, portal, settings_dialog, track_details_panel};

mod vst3_cxx_bridge;
mod audio_layer_manager;
mod actions;
mod history;
mod dawproject_parser;

#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

thread_local!(static THREAD_POOL: RefCell<rayon::ThreadPool> = RefCell::new(
    rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .thread_name(|index: usize| format!("daw_evt_thrd-{}", index))
        .build()
        .unwrap()));


fn app_logic(data: &mut RiffDAWState) -> impl Iterator<Item = WindowView<RiffDAWState>> + use<> {

    let track_details_panel = track_details_panel(data);

    std::iter::once(
        window(
            data.main_window_id,
            "Riff DAW",
            fork(
                flex_col(
                    main_view(data)
                )
                    .main_axis_alignment(MainAxisAlignment::Start)
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .must_fill_major_axis(true),
                worker(
                    |proxy, mut rx| async move {
                        let mut audio_layer_manager = AudioLayerManager::new();
                        let mut last_run_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis();

                        loop {
                            let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis();

                            if (now - last_run_time) > 50 {
                                if !audio_layer_manager.handle_events(&mut rx, &proxy) {
                                    break;
                                }

                                audio_layer_manager.handle_incoming_jack_events(&proxy);
                                last_run_time = now;
                            }

                            audio_layer_manager.handle_incoming_jack_immediate_events(&proxy);
                        }
                    },
                    |state: &mut RiffDAWState, sender| {
                        state.audio_layer_sender = Some(sender);

                        // init the default project tracks
                        let sample_rate = state.configuration.audio.sample_rate as f64;
                        let block_size = state.configuration.audio.block_size as f64;
                        if let Ok(project) = state.project().lock().as_mut() {
                            let tempo = project.song().tempo();
                            let time_signature_numerator = project.song().time_signature_numerator();
                            let time_signature_denominator = project.song().time_signature_denominator();
                            let mut sample_references = HashMap::new();
                            let mut samples_data = HashMap::new();
                            for track in project.song_mut().tracks_mut().iter_mut() {
                                state.init_track(
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
                    },
                    |state: &mut RiffDAWState, event: AudioLayerEvent| {
                        // println!("Received an audio layer outward message.");
                        match event {
                            AudioLayerEvent::TrackBackgroundProcessorOutward(track_uuid, event) => {
                                match event {
                                    TrackBackgroundProcessorOutwardEvent::InstrumentName(name) => {
                                        println!("InstrumentName: {}", name.as_str());
                                        if let Ok(project) = state.get_project().lock().as_mut() {
                                            if let Some(track) = project.song_mut().track_mut(&Uuid::parse_str(track_uuid.as_str()).unwrap()) {
                                                if let TrackType::InstrumentTrack(instrument_track) = track {
                                                    instrument_track.instrument.name = name;
                                                }
                                            }
                                        }
                                    }
                                    TrackBackgroundProcessorOutwardEvent::InstrumentParameters(instrument_parameters) => {
                                        let mut parameter_details = vec![];
                                        let mut plugin_uuid = String::new();
                                        debug!("Received instrument plugin parameter details.");
                                        instrument_parameters.iter().for_each(|(param_index, track_uuid_orig, plugin_uuid_orig, param_name, param_label, param_value, param_text)| {
                                            println!("Received plugin parameter details for: track uuid={}, plugin uuid={},  param - index={},  param - name={}, label={}, value={}, text={}",
                                                     track_uuid_orig, plugin_uuid_orig.clone(), param_index, param_name, param_label, param_value, param_text);
                                            plugin_uuid.clear();
                                            plugin_uuid.push_str(plugin_uuid_orig.to_string().as_str()); // plugin uuid
                                            parameter_details.push(PluginParameterDetail {
                                                index: *param_index,
                                                name: param_name.clone(),
                                                label: param_label.clone(),
                                                text: param_text.clone(),
                                            });
                                        });

                                        if let Some(audio_plugin_parameters) = state.audio_plugin_parameters_mut().get_mut(&track_uuid) {
                                            audio_plugin_parameters.insert(plugin_uuid, parameter_details);
                                        }
                                        else {
                                            let mut plugins_to_plugin_params_map = HashMap::new();
                                            plugins_to_plugin_params_map.insert(plugin_uuid, parameter_details);
                                            state.audio_plugin_parameters_mut().insert(String::from(track_uuid.as_str()), plugins_to_plugin_params_map);
                                        }
                                    }
                                    TrackBackgroundProcessorOutwardEvent::EffectParameters(effect_parameters) => {
                                        let mut parameter_details = vec![];
                                        debug!("Received effect plugin parameter details.");
                                        let mut effect_plugin_uuid = String::new();
                                        // vector of plugin uuid, param index, param name, param label, param value, param text
                                        effect_parameters.iter().for_each(|(plugin_uuid, param_index, param_name, param_label, param_value, param_text)| {
                                            println!("Received effect plugin parameter details for: track uuid={}, plugin uuid={},  param - index={},  param - name={}, label={}, value={}, text={}",
                                                     track_uuid.as_str(), plugin_uuid.as_str(), param_index, param_name.as_str(), param_label.as_str(), param_value, param_text.as_str());
                                            effect_plugin_uuid = plugin_uuid.clone();
                                            parameter_details.push(PluginParameterDetail {
                                                index: *param_index,
                                                name: param_name.clone(),
                                                label: param_label.clone(),
                                                text: param_text.clone(),
                                            });
                                        });

                                        if let Some(audio_plugin_parameters) = state.audio_plugin_parameters_mut().get_mut(&track_uuid) {
                                            audio_plugin_parameters.insert(effect_plugin_uuid, parameter_details);
                                        }
                                        else {
                                            let mut plugins_to_plugin_params_map = HashMap::new();
                                            plugins_to_plugin_params_map.insert(effect_plugin_uuid, parameter_details);
                                            state.audio_plugin_parameters_mut().insert(String::from(track_uuid.as_str()), plugins_to_plugin_params_map);
                                        }
                                    }
                                    TrackBackgroundProcessorOutwardEvent::GetPresetData(instrument_preset_data, effects_preset_data) => {
                                        println!("GetPresetData: {}", instrument_preset_data);
                                        if let Ok(project) = state.get_project().lock().as_mut() {
                                            if let Some(track) = project.song_mut().track_mut(&Uuid::parse_str(track_uuid.as_str()).unwrap()) {
                                                if let TrackType::InstrumentTrack(instrument_track) = track {
                                                    instrument_track.instrument.set_preset_data(instrument_preset_data);
                                                    for (index, effect) in instrument_track.effects.iter_mut().enumerate() {
                                                        if let Some(preset_data) = effects_preset_data.get(index) {
                                                            effect.set_preset_data(preset_data.clone());
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    TrackBackgroundProcessorOutwardEvent::InstrumentPluginWindowSize(_, _, _) => {}
                                    TrackBackgroundProcessorOutwardEvent::EffectPluginWindowSize(_, _, _, _) => {}
                                    TrackBackgroundProcessorOutwardEvent::Automation(_, _, _, _, _) => {}
                                    TrackBackgroundProcessorOutwardEvent::TrackRenderAudioConsumer(_) => {}
                                    TrackBackgroundProcessorOutwardEvent::ChannelLevels(_, _, _) => {}
                                }
                            }
                            AudioLayerEvent::AudioLayerOutward(event) => {
                                match event {
                                    AudioLayerOutwardEvent::MidiControlEvent(midi_control_event) => {
                                        midi_AudioLayerOutwardEvent_MidiControlEvent(state, midi_control_event);
                                    }
                                    AudioLayerOutwardEvent::GeneralMMCEvent(mmc_event) => {
                                        midi_AudioLayerOutwardEvent_GeneralMMCEvent(state, mmc_event);
                                    }
                                    AudioLayerOutwardEvent::PlayPositionInFrames(play_position_in_frames) => {
                                        midi_AudioLayerOutwardEvent_PlayPositionInFrames(state, play_position_in_frames);
                                    }
                                    AudioLayerOutwardEvent::JackRestartRequired => {

                                    }
                                    AudioLayerOutwardEvent::JackConnect(xxx, yyy) => {

                                    }
                                    AudioLayerOutwardEvent::MasterChannelLevels(volume, pan) => {

                                    }
                                }
                            }
                            AudioLayerEvent::AudioLayerTimeCriticalOutwardEvent(event) => {
                                match event {
                                    AudioLayerTimeCriticalOutwardEvent::MidiEvent(midiEvent) => {
                                        midi_AudioLayerTimeCriticalOutwardEvent_MidiEvent(state, midiEvent);
                                    }
                                    AudioLayerTimeCriticalOutwardEvent::TrackVolumePanLevel(jack_midi_event) => {
                                        match state.get_project().lock() {
                                            Ok(project) => {
                                                if jack_midi_event.data[0] as i32 >= 176 && (jack_midi_event.data[0] as i32 <= (176 + 15)) {
                                                    debug!("Main - jack_event_prcessing_thread processing loop - jack AudioLayerTimeCriticalOutwardEvent::TrackVolumePanLevel - received a controller message: {} {} {}", jack_midi_event.data[0], jack_midi_event.data[1], jack_midi_event.data[2]);
                                                    // need to send some track volume (176) or pan (177) messages
                                                    let position_in_frames = jack_midi_event.delta_frames;
                                                    let position_in_beats = (position_in_frames as f64) / state.configuration.audio.sample_rate as f64 * project.song().tempo() / 60.0;
                                                    let track_index = jack_midi_event.data[1] as i32 - 1;
                                                    if let Some(track) = project.song().tracks().get(track_index as usize) {
                                                        if jack_midi_event.data[0] as i32 == 176 {
                                                            track_change_type_Volume(state, Some(position_in_beats), jack_midi_event.data[2] as f32 / 127.0, Some(track.uuid()));
                                                        } else {
                                                            track_change_type_Pan(state, Some(position_in_beats), (jack_midi_event.data[2] as f32 - 63.5) / 63.5, Some(track.uuid()));
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
                            AudioLayerEvent::AudioLayerInward(_) => {}
                            AudioLayerEvent::TrackBackgroundProcessorInward(_, _) => {}
                            AudioLayerEvent::AddTrackBackgroundProcessor(_, _) => {}
                            AudioLayerEvent::DeleteTrackBackgroundProcessor(_) => {}
                            AudioLayerEvent::AudioMode(_) => {}
                            AudioLayerEvent::Shutdown => {}
                            _ => {}
                        }
                    },
                )
            )
        )
            .with_options(|o| o.on_close(|state: &mut RiffDAWState| {
                state.running = false;
                state.configuration.bookmark_paths = state.file_dialog.bookmarks.items
                    .iter()
                    .map(|path| path.to_string_lossy().to_string())
                    .collect();
                state.configuration.save();
                if let Some(sender) = state.audio_layer_sender.as_mut() {
                    let _ = sender.send(AudioLayerEvent::Shutdown);
                }
            }))
    )
        .chain(
            data
                .open_file_dialogue
                .iter()
                .find(|(window_id, show_window)| **show_window)
                .map(|(window_id, _)|  {
                    let window_id = *window_id;
                    window(
                        window_id,
                        "Open file...",
                        flex_col((
                            // label("Fuck me!"),
                            text_button("Close".to_string(), move |state: &mut RiffDAWState| {
                                state.open_file_dialogue.insert(window_id.clone(), false);
                            }),
                        ))
                    )
                        .with_options(|options| {
                            options.on_close(|_state: &mut RiffDAWState| {
                                println!("Attempted to close the open file chooser.")
                            })
                        })
                }),
        )
        .chain(
            data
                .save_file_dialogue
                .iter()
                .find(|(window_id, show_window)| **show_window)
                .map(|(window_id, _)|  {
                    let window_id = *window_id;
                    window(
                        window_id,
                        "Save file...",
                        flex_col((
                            text_button("Close".to_string(), move |state: &mut RiffDAWState| {
                                state.save_file_dialogue.insert(window_id.clone(), false);
                            }),
                        ))
                    )
                        .with_options(|options| {
                            options.on_close(|state: &mut RiffDAWState| {
                                println!("Attempted to close the save file chooser.");
                            })
                        })
                }),
        )
        .chain(
            data
                .track_details_window
                .iter()
                .find(|(window_id, show_window)| **show_window)
                .map(|(window_id, _)|  {
                    let window_id = *window_id;
                    let window = window(
                        window_id.clone(),
                        "Track Details...",
                        flex_col((
                            track_details_panel,
                            flex_row((
                                FlexSpacer::Flex(1.0),
                                text_button("Close".to_string(), move |state: &mut RiffDAWState| {
                                    state.track_details_window.insert(window_id.clone(), false);
                                }),
                                FlexSpacer::Fixed(20.px()),
                            ))
                        )).main_axis_alignment(MainAxisAlignment::Start).cross_axis_alignment(CrossAxisAlignment::Start).must_fill_major_axis(true)
                    )
                        .with_options(|options| {
                            options.on_close(|state: &mut RiffDAWState| {
                                println!("Attempted to close an track details window.");
                            })
                                .with_min_inner_size(dpi::Size::Physical(dpi::PhysicalSize {width: 1300, height: 700}))
                        });
                    window
                }),
        )
        .chain(
            data
                .riff_name_window
                .iter()
                .find(|(window_id, show_window)| **show_window)
                .map(|(window_id, _)|  {
                    let window_id = *window_id;
                    let window = window(
                        window_id.clone(),
                        "Riff name...",
                        flex_col((
                            label("Riff name"),
                            text_input("".to_string(), |state: &mut RiffDAWState, name: String| {
                                state.riff_name = Some(name);
                            }),
                            flex_row((
                                text_button("Cancel".to_string(), move |state: &mut RiffDAWState| {
                                    state.riff_name_window.insert(window_id.clone(), false);
                                    state.riff_name = None;
                                }),
                                text_button("Ok".to_string(), move |state: &mut RiffDAWState| {
                                    state.riff_name_window.insert(window_id.clone(), false);
                                }),
                            ))
                        )).main_axis_alignment(MainAxisAlignment::Start).cross_axis_alignment(CrossAxisAlignment::Start).must_fill_major_axis(true)
                    )
                        .with_options(|options| {
                            options.on_close(|state: &mut RiffDAWState| {
                                println!("Attempted to close an track details window.");
                            })
                        });
                    window
                }),
        )
        .chain(
            data
                .settings_window
                .iter()
                .find(|(window_id, show_window)| **show_window)
                .map(|(window_id, _)|  {
                    let window_id = *window_id;
                    let window = window(
                        window_id.clone(),
                        "Settings",
                        settings_dialog(window_id.clone()),
                    )
                        .with_options(move |options| {
                            options.on_close(move |state: &mut RiffDAWState| {
                                state.settings_window.insert(window_id, false);
                            })
                                .with_initial_inner_size(xilem::dpi::LogicalSize::new(400.0, 300.0))
                        });
                    window
                }),
        )
        .chain(data.file_dialog.dialog_window_id.map(|dialog_window_id| {
            window(
                dialog_window_id,
                data.file_dialog.dialog.mode.title(),
                dialog_view(data),
            )
                .with_options(|o| {
                    o.with_initial_inner_size(xilem::dpi::LogicalSize::new(760.0, 520.0))
                        .on_close(|state: &mut RiffDAWState| close_dialog(state))
                })
        }))    .collect::<Vec<_>>()
        .into_iter()
}

fn run(event_loop: EventLoopBuilder) -> Result<(), EventLoopError> {

    let logger_init_result = Logger::try_with_env();
    let _logger = if let Ok(logger) = logger_init_result {
        let logger = logger
            // .log_to_file(FileSpec::default())
            // .write_mode(WriteMode::Async)
            .log_to_stdout()
            .start();
        Some(logger)
    }
    else {
        None
    };

    let mut data = RiffDAWState::default();

    let mut default_properties = default_property_set();

    default_properties.insert::<masonry::widgets::Button, _>(Background::Color(WHITE));
    default_properties.insert::<masonry::widgets::Button, _>(Padding::all(4.));
    default_properties.insert::<masonry::widgets::Button, _>(CornerRadius::all(4.));
    default_properties.insert::<masonry::widgets::Label, _>(ContentColor::new(Color::from_rgba8(0, 0, 255, 255)));

    let app = Xilem::new(
        data,
        app_logic
    ).with_default_properties(default_properties);

    app.run_in(event_loop)
}

fn main() -> Result<(), EventLoopError> {
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

    run(EventLoop::with_user_event())
}