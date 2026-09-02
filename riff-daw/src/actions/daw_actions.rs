use std::{collections::HashMap, default::Default, sync::{Arc, Mutex}, time::Duration};
use std::cell::RefCell;
use std::thread;

use apres::MIDI;
use crossbeam_channel::{Receiver, Sender, unbounded};
use flexi_logger::{LogSpecification, Logger};
use indexmap::IndexMap;
use itertools::Itertools;
use jack::MidiOut;
use log::*;
use mlua::{Lua, MultiValue, Value};
use parking_lot::RwLock;
use rfd::MessageLevel;
use simple_clap_host_helper_lib::plugin::library::PluginLibrary;
use thread_priority::{ThreadBuilder, ThreadPriority};
use uuid::Uuid;
use vst::host::PluginLoader;
use vst::api::TimeInfo;
use crate::constants::{TRACK_VIEW_TRACK_PANEL_HEIGHT, LUA_GLOBAL_STATE, VST_PATH_ENVIRONMENT_VARIABLE_NAME, CLAP_PATH_ENVIRONMENT_VARIABLE_NAME, DAW_AUTO_SAVE_THREAD_NAME};
use crate::actions::automation_actions::{handle_automation_add, handle_automation_change, handle_automation_copy, handle_automation_cut, handle_automation_delete, handle_automation_paste, handle_automation_quantise, handle_automation_translate_selected};
use crate::audio_plugin_util::scan_for_audio_plugins;
use crate::domain::{get_plugin_details, AudioEffectTrack, AudioPlugin, AudioRouting, AudioRoutingNodeType, AudioTrack, AutomationEnvelope, Controller, DAWItemID, DAWItemLength, DAWItemPosition, InstrumentTrack, Loop, MidiTrack, Project, Riff, RiffItem, RiffItemType, RiffReference, RiffReferenceMode, RiffSequence, RiffSet, SampleData, SampleReference, Track, AudioMode, TrackEvent, TrackEventRouting, TrackEventRoutingNodeType, TrackType};
use crate::event::{AudioLayerEvent, AudioLayerInwardEvent, AutomationChangeData, AutomationEditType, CurrentView, DAWEvents, GeneralTrackType, LoopChangeType, MasterChannelChangeType, NoteExpressionData, NotificationType, OperationModeType, RiffGridChangeType, ShowType, TrackBackgroundProcessorInwardEvent, TrackChangeType, TranslateDirection, TranslationEntityType, TransportChangeType};
use crate::history::{RiffAdd, RiffChangeLengthOfSelectedAction, RiffCutSelectedAction, RiffDelete, RiffDeleteNoteAction, RiffPasteSelectedAction, RiffQuantiseSelectedAction, RiffTranslateSelectedAction};
use crate::state::{AutomationViewMode, MidiPolyphonicExpressionNoteId, RiffDAWState};
use crate::utils::DAWUtils;

pub fn daw_events_UpdateProgressBarMessage(state: &mut RiffDAWState, message: String) {
    // gui.ui.dialogue_progress_bar.set_text(Some(message.as_str()));
}

pub fn daw_events_UpdateUI (state: &mut RiffDAWState) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            // gui.update_ui_from_state(tx_from_ui, &mut state, state_arc);
        }
        Err(_) => debug!("Main - rx_ui processing loop - Export Wave File - could not get lock on state"),
    }

    // gui.ui.track_drawing_area.queue_draw();
    // gui.ui.piano_roll_drawing_area.queue_draw();
    // gui.ui.sample_roll_drawing_area.queue_draw();
    // gui.ui.automation_drawing_area.queue_draw();
}

pub fn daw_events_UpdateUIPlugins (state: &mut RiffDAWState) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            // gui.update_available_audio_plugins_in_ui(&state.configuration.scanned_instrument_plugins.successfully_scanned, &state.configuration.scanned_effect_plugins.successfully_scanned);
        }
        Err(_) => debug!("Main - rx_ui processing loop - DAWEvents::UpdateUIPlugins - could not get lock on state"),
    }
}

pub fn daw_events_Notification(state: &mut RiffDAWState, notification_type: NotificationType, message: String) {
    let message_level = match notification_type {
        NotificationType::Info => { MessageLevel::Info }
        NotificationType::Warning => { MessageLevel::Warning }
        NotificationType::Error => { MessageLevel::Error }
        _ => { MessageLevel::Info }
    };

    rfd::MessageDialog::new()
        .set_level(message_level)
        .set_title("Message")
        .set_description(message)
        .set_buttons(rfd::MessageButtons::OkCancel)
        .show();
}


pub fn daw_events_TranslateHorizontalChange(state: &mut RiffDAWState, value: i32) {
    debug!("Event: TranslateHorizontalChange"); 
}

pub fn daw_events_TranslateVerticalChange(state: &mut RiffDAWState, value: i32) {
    debug!("Event: TranslateVerticalChange"); 
}

pub fn daw_events_TransportChange(state: &mut RiffDAWState, transport_change_type: TransportChangeType, value1: f64, value2: f64) {
    debug!("Event: TransportChange"); 
}

pub fn daw_events_ViewAutomationChange(state: &mut RiffDAWState, show_automation_events: bool) {
    debug!("Event: ViewAutomationChange: {}", show_automation_events); 
}

pub fn daw_events_ViewNoteChange(state: &mut RiffDAWState, show_note_events: bool) {
    debug!("Event: ViewNoteChange: {}", show_note_events); 
}

pub fn daw_events_ViewPanChange(state: &mut RiffDAWState, show_pan_events: bool) {
    debug!("Event: ViewPanChange: {}", show_pan_events); 
}

pub fn daw_events_ViewVolumeChange(state: &mut RiffDAWState, show_volume_events: bool) {
    debug!("Event: ViewVolumeChange: {}", show_volume_events); 
}

pub fn daw_events_PlayNoteImmediate(state: &mut RiffDAWState, note: i32) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let track_uuid = state.selected_track();
            match track_uuid {
                Some(track_uuid) => {
                    match project.song().tracks().iter().find(|track| track.uuid().to_string() == track_uuid) {
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
}

pub fn daw_events_StopNoteImmediate(state: &mut RiffDAWState, note: i32) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let track_uuid = state.selected_track();
            match track_uuid {
                Some(track_uuid) => {
                    match project.song().tracks().iter().find(|track| track.uuid().to_string() == track_uuid) {
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
}

pub fn daw_events_Panic (state: &mut RiffDAWState) {
    debug!("Sending note off messages to everything...");
    match state.get_project().lock().as_mut() {
        Ok(project) => for track in project.song().tracks().iter() {
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
}

pub fn daw_events_MasterChannelChange(state: &mut RiffDAWState, channel_change_type: MasterChannelChangeType) {
    match channel_change_type {
        MasterChannelChangeType::VolumeChange(volume) => {
            debug!("Master channel volume change: {}", volume);
            if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
                match audio_layer_sender.send(AudioLayerEvent::AudioLayerInward(AudioLayerInwardEvent::Volume(volume as f32))) {
                    Ok(_) => (),
                    Err(error) => debug!("Problem using tx_to_audio to send master volume message to jack layer: {}", error),
                }
            }
        },
        MasterChannelChangeType::PanChange(pan) => {
            debug!("Master channel pan change: {}", pan);
            if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
                match audio_layer_sender.send(AudioLayerEvent::AudioLayerInward(AudioLayerInwardEvent::Pan(pan as f32))) {
                    Ok(_) => (),
                    Err(error) => debug!("Problem using tx_to_audio to send master pan message to jack layer: {}", error),
                }
            }
        },
    }
}

pub fn daw_events_PlayPositionInBeats(state: &mut RiffDAWState, play_position_in_beats: f64) {
    debug!("Received DAWEvents::PlayPositionInBeats");
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let bpm = project.song().tempo();
            let sample_rate = state.configuration.audio.sample_rate as f64;
            let block_size = state.configuration.audio.block_size as f64;
            let play_position_in_frames = 60.0 * play_position_in_beats / bpm * sample_rate;

            state.set_play_position_in_frames(play_position_in_frames as u32);

            {
                let state = state;
                for track in project.song().tracks().iter() {
                    state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetBlockPosition((play_position_in_frames / block_size) as i32));
                }
            }
        },
        Err(_) => debug!("Main - rx_ui processing loop - play position in beats - could not get lock on state"),
    };
}

pub fn daw_events_TrimAllNoteDurations (state: &mut RiffDAWState) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            {
                for track_type in project.song_mut().tracks_mut().iter_mut() {
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

pub fn daw_events_Shutdown (state: &mut RiffDAWState) {
    if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
        let _ = audio_layer_sender.send(AudioLayerEvent::AudioMode(AudioMode::Coast));
    }
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            for track in project.song().tracks() {
                state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Kill);
            }
        },
        Err(_) => debug!("Main - rx_ui processing loop - Open File - could not get lock on state"),
    }
    if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
        match audio_layer_sender.send(AudioLayerEvent::AudioLayerInward(AudioLayerInwardEvent::Shutdown)) {
            Ok(_) => {}
            Err(_) => {}
        }
    }
}

pub fn daw_events_Undo (state: &mut RiffDAWState) {
    let history_manager = state.history_manager.clone();
    if let Ok(history_manager) = history_manager.lock().as_mut() {
        if let Err(error) = history_manager.undo(state) {
            debug!("{}", error);
        }
    }
}

pub fn daw_events_Redo (state: &mut RiffDAWState) {
    let history_manager = state.history_manager.clone();
    if let Ok(history_manager) = history_manager.lock().as_mut() {
        if let Err(error) = history_manager.redo(state) {
            debug!("{}", error);
        }
    }
}

pub fn daw_events_RunLuaScript(state: &mut RiffDAWState, script: String) {
    // match lua.load(script.as_str()).eval::<MultiValue>() {
    //     Ok(values) => {
    //         if let Some(console_output_text_buffer) = gui.ui.scripting_console_output_text_view.buffer() {
    //             let console_output_text = format!("{}\n>> ",
    //                                               values
    //                                                   .iter()
    //                                                   .map(|value| {
    //                                                       match value {
    //                                                           Value::Nil => "Nil".to_string(),
    //                                                           Value::Boolean(data) => format!("{}", data),
    //                                                           Value::LightUserData(_data) => "LightUserData".to_string(),
    //                                                           Value::Integer(data) => format!("{}", data),
    //                                                           Value::Number(data) => format!("{}", data),
    //                                                           Value::String(data) => data.to_str().unwrap().to_string(),
    //                                                           Value::Table(_data) => "Table".to_string(),
    //                                                           Value::Function(_data) => "Function".to_string(),
    //                                                           Value::Thread(_data) => "Thread".to_string(),
    //                                                           Value::UserData(_data) => "AnyUserData".to_string(),
    //                                                           Value::Error(data) => format!("{:?}", data),
    //                                                           _ => "".to_string(),
    //                                                       }
    //                                                   })
    //                                                   .collect::<Vec<_>>()
    //                                                   .join("\t")
    //             );
    //             console_output_text_buffer.insert(&mut console_output_text_buffer.end_iter(), console_output_text.as_str());
    //         }
    //     }
    //     Err(error) => {
    //         if let Some(console_output_text_buffer) = gui.ui.scripting_console_output_text_view.buffer() {
    //             let console_output_text = format!("{}\n>> ", error);
    //             console_output_text_buffer.insert(&mut console_output_text_buffer.end_iter(), console_output_text.as_str());
    //         }
    //     }
    // }
}

pub fn daw_events_HideProgressDialogue (state: &mut RiffDAWState) {
    // gui.ui.progress_dialogue.hide();
}

pub fn daw_events_RepaintRiffArrangementBox (state: &mut RiffDAWState) {
    // gui.ui.riff_arrangement_box.queue_draw();
}

pub fn daw_events_RepaintRiffSetsBox (state: &mut RiffDAWState) {
    // gui.ui.riff_sets_box.queue_draw();
}

pub fn daw_events_RepaintRiffSequencesBox (state: &mut RiffDAWState) {
    // gui.ui.riff_sequences_box.queue_draw();
}

pub fn daw_events_RiffReferenceRegenerateIds (state: &mut RiffDAWState) {
    debug!("Main - rx_ui processing loop - DAWEvents::RiffReferenceRegenerateIds");
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            for riff_grid in project.song_mut().riff_grids_mut().iter_mut() {
                for track_uuid in riff_grid.tracks_mut().map(|track_uuid| track_uuid.clone()).collect_vec().iter() {
                    if let Some(track_riff_references) = riff_grid.track_riff_references_mut(track_uuid.clone()) {
                        track_riff_references.iter_mut().for_each(|riff_ref| riff_ref.set_id(Uuid::new_v4().to_string()));
                    }
                }
            }

            let track_uuids = project.song_mut().tracks_mut().iter_mut().map(|track| track.uuid().to_string()).collect_vec();
            for riff_set in project.song_mut().riff_sets_mut().iter_mut() {
                for track_uuid in track_uuids.iter() {
                    if let Some(riff_ref) = riff_set.get_riff_ref_for_track_mut(track_uuid.clone()) {
                        riff_ref.set_id(Uuid::new_v4().to_string());
                    }
                }
            }

            for track in project.song_mut().tracks_mut() {
                track.riff_refs_mut().iter_mut().for_each(|riff_ref| riff_ref.set_id(Uuid::new_v4().to_string()));
            }

            // gui.clear_ui();
            // gui.update_ui_from_state(tx_from_ui.clone(), &mut state, state_arc);
        },
        Err(_) => debug!("Main - rx_ui processing loop - DAWEvents::RiffReferenceRegenerateIds - could not get lock on state"),
    }
}

pub fn daw_events_AudioConfigurationChanged(state: &mut RiffDAWState, sample_rate: i32, block_size: i32) {
    debug!("Main - rx_ui processing loop - DAWEvents::AudioConfigurationChanged");
    // gui.clear_ui();
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            state.close_all_tracks();
            state.reset_state();
            state.configuration.audio.sample_rate = sample_rate;
            state.configuration.audio.block_size = block_size;

            // {
            //     let mut time_info =  vst_host_time_info.write();
            //     time_info.sample_pos = 0.0;
            //     time_info.sample_rate = state.configuration.audio.sample_rate as f64;
            //     time_info.nanoseconds = 0.0;
            //     time_info.ppq_pos = 0.0;
            //     time_info.tempo = project.song().tempo();
            //     time_info.bar_start_pos = 0.0;
            //     time_info.cycle_start_pos = 0.0;
            //     time_info.cycle_end_pos = 0.0;
            //     time_info.time_sig_numerator = project.song().time_signature_numerator() as i32;
            //     time_info.time_sig_denominator = project.song().time_signature_denominator() as i32;
            //     time_info.smpte_offset = 0;
            //     time_info.smpte_frame_rate = vst::api::SmpteFrameRate::Smpte24fps;
            //     time_info.samples_to_next_clock = 0;
            //     time_info.flags = 3;
            // }
            //
            // // update the transport
            // TRANSPORT.get().write().sample_rate = sample_rate as f64;
            // TRANSPORT.get().write().block_size = block_size as f64;
            //
            // state.stop_jack();
            // state.start_jack(rx_to_audio.clone(), jack_midi_sender.clone(), jack_midi_sender_ui.clone(), jack_time_critical_midi_sender.clone(), jack_audio_coast.clone(), vst_host_time_info.clone());

            let mut instrument_track_senders2 = HashMap::new();
            let mut instrument_track_receivers2 = HashMap::new();
            let mut sample_references = HashMap::new();
            let mut samples_data = HashMap::new();
            let sample_rate = state.configuration.audio.sample_rate as f64;
            let block_size = state.configuration.audio.block_size as f64;
            let tempo = project.song().tempo();
            let time_signature_numerator = project.song().time_signature_numerator();
            let time_signature_denominator = project.song().time_signature_denominator();
            for track in project.song_mut().tracks_mut().iter_mut() {
                state.init_track(
                    track,
                    Some(&sample_references),
                    Some(&samples_data),
                    sample_rate,
                    block_size,
                    tempo,
                    time_signature_numerator as i32,
                    time_signature_denominator as i32,
                );
            }
            state.update_track_senders_and_receivers(instrument_track_senders2, instrument_track_receivers2);

            // gui.update_ui_from_state(tx_from_ui, &mut state, state_arc);
            if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
                match audio_layer_sender.send(AudioLayerEvent::AudioLayerInward(AudioLayerInwardEvent::Tempo(project.song().tempo()))) {
                    Ok(_) => (),
                    Err(error) => debug!("Problem using tx_to_audio to send tempo message to jack layer: {}", error),
                }
            }
        },
        Err(_) => debug!("Main - rx_ui processing loop - DAWEvents::AudioConfigurationChanged - could not get lock on state"),
    }
}

pub fn daw_events_LoopChange(state: &mut RiffDAWState, change_type: LoopChangeType, uuid: Uuid) {
    debug!("Event: LoopChange");
    match change_type {
        LoopChangeType::LoopOn => {
            match state.get_project().lock() {
                Ok(project) => {
                    let mut start_block = 0;
                    let mut end_block = 44;
                    let sample_rate = state.configuration.audio.sample_rate as f64;
                    let block_size = state.configuration.audio.block_size as f64;
                    let song = project.song();
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
                    // match tx_to_audio.send(AudioLayerInwardEvent::ExtentsChange(end_block - start_block)) {
                    //     Ok(_) => (),
                    //     Err(error) => debug!("Problem using tx_to_audio to send message to jack layer when turning looping on: {}", error),
                    // }
                    for track in tracks {
                        state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::LoopExtents(start_block, end_block));
                        state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Loop(true));
                    }

                    state.set_looping(true);
                },
                Err(_) => debug!("Main - rx_ui processing loop - set active loop - could not get lock on state"),
            }
        }
        LoopChangeType::LoopOff => {
            state.set_looping(false);
            match state.project().lock() {
                Ok(project) => {
                    let song = project.song();
                    let tracks = song.tracks();
                    for track in tracks {
                        state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Loop(false));
                    }
                },
                Err(_) => debug!("Main - rx_ui processing loop - set active loop - could not get lock on state"),
            }
        }
        LoopChangeType::ActiveLoopChanged(uuid) => {
            state.set_active_loop(uuid.clone());
            match state.project().lock() {
                Ok(project) => {
                    let mut start_block = 0;
                    let mut end_block = 44;
                    let sample_rate = state.configuration.audio.sample_rate as f64;
                    let block_size = state.configuration.audio.block_size as f64;
                    let song = project.song();
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
                },
                Err(_) => debug!("Main - rx_ui processing loop - set active loop - could not get lock on state"),
            }
        },
        LoopChangeType::LoopLimitLeftChanged(start_position) => {
            match state.get_project().lock().as_mut() {
                Ok(project) => {
                    let sample_rate = state.configuration.audio.sample_rate as f64;
                    let block_size = state.configuration.audio.block_size as f64;

                    match state.active_loop() {
                        Some(active_loop_uuid) => {
                            let song = project.song();
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
                            match project.song_mut().loops_mut().iter_mut().find(|current_loop| current_loop.uuid().to_string() == active_loop_uuid.to_string()) {
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
        },
        LoopChangeType::LoopLimitRightChanged(end_position) => {
            match state.get_project().lock().as_mut() {
                Ok(project) => {
                    let sample_rate = state.configuration.audio.sample_rate as f64;
                    let block_size = state.configuration.audio.block_size as f64;

                    match state.active_loop() {
                        Some(active_loop_uuid) => {
                            let song = project.song();
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
                            match project.song_mut().loops_mut().iter_mut().find(|current_loop| current_loop.uuid().to_string() == active_loop_uuid.to_string()) {
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
        },
        LoopChangeType::Added(loop_name) => {
            match state.get_project().lock().as_mut() {
                Ok(project) => {
                    project.song_mut().add_loop(Loop::new_with_uuid_and_name(uuid, loop_name));
                },
                Err(_) => debug!("Main - rx_ui processing loop - loop add - could not get lock on state"),
            }
        }
        LoopChangeType::Deleted => {
            match state.get_project().lock().as_mut() {
                Ok(project) => {
                    project.song_mut().delete_loop(uuid);
                },
                Err(_) => debug!("Main - rx_ui processing loop - loop delete - could not get lock on state"),
            }
        }
        LoopChangeType::NameChanged(name) => {
            match state.get_project().lock().as_mut() {
                Ok(project) => {
                    project.song_mut().change_loop_name(uuid, name);
                },
                Err(_) => debug!("Main - rx_ui processing loop - loop name change - could not get lock on state"),
            }
        },
    }
}