use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use indexmap::IndexMap;
use itertools::Itertools;
use jack::MidiOut;
use log::{debug, error};
use uuid::Uuid;
use crate::actions::automation_actions::{handle_automation_add, handle_automation_change, handle_automation_copy, handle_automation_cut, handle_automation_delete, handle_automation_paste, handle_automation_quantise, handle_automation_translate_selected};
use crate::constants::MUSICAL_ITEM_LENGTH_OPTIONS;
use crate::domain::{get_plugin_details, AudioEffectTrack, AudioLayerInwardEvent, AudioPlugin, AudioRouting, AudioRoutingNodeType, AudioTrack, AutomationEnvelope, Controller, DAWItemID, DAWItemLength, DAWItemPosition, DAWUtils, GeneralTrackType, InstrumentTrack, MidiTrack, Riff, RiffItemType, RiffReference, RiffReferenceMode, SampleReference, Track, TrackBackgroundProcessorInwardEvent, TrackEvent, TrackEventRouting, TrackEventRoutingNodeType, TrackType};
use crate::event::{AudioLayerEvent, AutomationChangeData, AutomationEditType, CurrentView, DAWEvents, NoteExpressionData, NotificationType, TranslateDirection, TranslationEntityType};
use crate::history::{RiffAdd, RiffAddNoteAction, RiffChangeLengthOfSelectedAction, RiffCutSelectedAction, RiffDelete, RiffDeleteNoteAction, RiffPasteSelectedAction, RiffQuantiseSelectedAction, RiffTranslateSelectedAction};
use crate::state::{AutomationViewMode, MidiPolyphonicExpressionNoteId, RiffDAWState};





pub fn track_change_type_Added(state: &mut RiffDAWState, track_change_track_type: GeneralTrackType, track_uuid: Option<String>) {
    let mut track_uuid = None;
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let mut instrument_track_senders_local = HashMap::new();
            let mut instrument_track_receivers_local = HashMap::new();
            let sample_rate = state.configuration.audio.sample_rate as f64;
            let block_size = state.configuration.audio.block_size as f64;;
            let tempo = project.song().tempo();
            let time_signature_numerator = project.song().time_signature_numerator();
            let time_signature_denominator = project.song().time_signature_denominator();

            match track_change_track_type {
                GeneralTrackType::InstrumentTrack => {
                    debug!("Adding an instrument track to the state...");
                    let track = InstrumentTrack::new();
                    track_uuid = Some(track.uuid().to_string());
                    // gui.add_track(track.name(), track.uuid(), tx_ui, state_arc, track_change_track_type, None, track.volume(), track.pan(), false, false);
                    project.song_mut().add_track(TrackType::InstrumentTrack(track));
                    if let Some(track_type) = project.song_mut().tracks_mut().last_mut() {
                        state.init_track(
                            track_type,
                            None,
                            None,
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
                    project.song_mut().add_track(TrackType::AudioTrack(track));
                    debug!("Added an audio track to the state.");
                    if let Some(track_type) = project.song_mut().tracks_mut().last_mut() {
                        state.init_track(
                            track_type,
                            None,
                            None,
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
                    project.song_mut().add_track(TrackType::MidiTrack(track));
                    if let Some(track_type) = project.song_mut().tracks_mut().last_mut() {
                        state.init_track(
                            track_type,
                            None,
                            None,
                            sample_rate,
                            block_size,
                            tempo,
                            time_signature_numerator as i32,
                            time_signature_denominator as i32,
                        );
                    }
                    thread::sleep(Duration::from_secs(1));
                    // if let Some(jack_client) = state.jack_client() {
                    //     if let Ok(midi_out_port) = jack_client.register_port(uuid.as_str(), MidiOut::default()) {
                    //         match tx_to_audio.send(AudioLayerInwardEvent::NewMidiOutPortForTrack(uuid, midi_out_port)) {
                    //             Ok(_) => (),
                    //             Err(error) => debug!("Problem using tx_to_audio to send new midi out port message to jack layer: {}", error),
                    //         }
                    //     }
                    // }
                    debug!("Added a midi track to the state.");
                }
                _ => {}
            }

            // gui.clear_ui();
            // gui.update_ui_from_state(tx_from_ui, &mut state, state_arc);

            state.update_track_senders_and_receivers(instrument_track_senders_local, instrument_track_receivers_local);
            // gui.update_available_audio_plugins_in_ui(&state.configuration.scanned_instrument_plugins.successfully_scanned, &state.configuration.scanned_effect_plugins.successfully_scanned);
        }
        Err(_) => debug!("Main - rx_ui processing loop - Track Added - could not get lock on state"),
    }
}

pub fn track_change_type_Deleted(state: &mut RiffDAWState, track_uuid: Option<String>) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let mut state = state;

            match track_uuid {
                Some(track_uuid) => {
                    project.song_mut().delete_track(track_uuid.clone());
                    if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
                        if let Err(error) = audio_layer_sender.send(AudioLayerEvent::AudioLayerInward(AudioLayerInwardEvent::RemoveTrack(track_uuid.clone()))) {
                            debug!("Main - rx_ui processing loop - Track Deleted - could send delete track to audio layer: {}", error);
                        }
                    }
                },
                None => debug!("Main - rx_ui processing loop - Track Deleted - could not find track"),
            }
        },
        Err(_) => debug!("Main - rx_ui processing loop - Track Deleted - could not get lock on state"),
    }
    // gui.ui.track_drawing_area.queue_draw();
}

pub fn track_change_type_Modified(state: &mut RiffDAWState, track_uuid: Option<String>) {
    debug!("pub fn track_change_type_Modified not yet implemented!");
}

pub fn track_change_type_SoloOn(state: &mut RiffDAWState, track_uuid: Option<String>) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let mut tracks_to_mute = vec![];
            let mut tracks_to_unmute = vec![];
            {
                let track_uuid = track_uuid.unwrap();
                for track in project.song_mut().tracks_mut() {
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

pub fn track_change_type_SoloOff(state: &mut RiffDAWState, track_uuid: Option<String>) {
    debug!("Main - rx_ui processing loop - turn solo off - received event from the UI.");
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let mut tracks_to_mute = vec![];
            let mut tracks_to_unmute = vec![];
            {
                let track_uuid = track_uuid.unwrap();
                let mut found_solo_track = false;
                for track in project.song_mut().tracks_mut() {
                    if track.uuid().to_string() == track_uuid {
                        track.set_solo(false);
                    } else if track.solo() {
                        found_solo_track = true;
                    }
                }
                for track in project.song_mut().tracks_mut() {
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

pub fn track_change_type_Mute(state: &mut RiffDAWState, track_uuid: Option<String>) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let mut state = state;
            let track_uuid = track_uuid.unwrap();
            match project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                Some(track) => track.set_mute(true),
                None => (),
            };
            state.send_to_track_background_processor(track_uuid, TrackBackgroundProcessorInwardEvent::Mute);
        },
        Err(_) => debug!("Main - rx_ui processing loop - Save As File - could not get lock on state"),
    }
}

pub fn track_change_type_Unmute(state: &mut RiffDAWState, track_uuid: Option<String>) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let mut state = state;
            let track_uuid = track_uuid.unwrap();
            match project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                Some(track) => track.set_mute(false),
                None => (),
            };
            state.send_to_track_background_processor(track_uuid, TrackBackgroundProcessorInwardEvent::Unmute);
        },
        Err(_) => debug!("Main - rx_ui processing loop - Save As File - could not get lock on state"),
    }
}

// pub fn track_change_type_MidiOutputDeviceChanged(state: &mut RiffDAWState, midi_device_name: String, track_uuid: Option<String>) {
//     let track_uuid = track_uuid.unwrap();
//     match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             let previous_midi_device_name = match project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
//                 Some(track_type) => match track_type {
//                     TrackType::InstrumentTrack(_) => "".to_string(),
//                     TrackType::AudioTrack(_) => "".to_string(),
//                     TrackType::MidiTrack(mut track) => {
//                         let previous_midi_device_name = track.midi_device_mut().name().to_string();
//                         track.midi_device_mut().set_name(midi_device_name.clone());
//                         previous_midi_device_name
//                     },
//                 },
//                 None => "".to_string(),
//             };
//             if !previous_midi_device_name.is_empty() {
//                 state.jack_midi_connection_remove(track_uuid.clone(), previous_midi_device_name);
//             }
//             state.jack_midi_connection_add(track_uuid, midi_device_name);
//         },
//         Err(_) => debug!("Main - rx_ui processing loop - track instrument changed - could not get lock on state"),
//     }
// }
//
// pub fn track_change_type_MidiInputDeviceChanged(state: &mut RiffDAWState, track_uuid: Option<String>) {
//     debug!("pub fn track_change_type_MidiInputDeviceChanged not yet implemented!");
// }
//
// pub fn track_change_type_MidiOutputChannelChanged(state: &mut RiffDAWState, midi_channel: i32, track_uuid: Option<String>) {
//     let track_uuid = track_uuid.unwrap();
//     match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             match project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
//                 Some(track_type) => match track_type {
//                     TrackType::InstrumentTrack(_) => (),
//                     TrackType::AudioTrack(_) => (),
//                     TrackType::MidiTrack(mut track) => {
//                         track.midi_device_mut().set_midi_channel(midi_channel);
//                     },
//                 },
//                 None => (),
//             }
//         },
//         Err(_) => debug!("Main - rx_ui processing loop - track instrument changed - could not get lock on state"),
//     }
// }
//
// pub fn track_change_type_MidiInputChannelChanged(state: &mut RiffDAWState, track_uuid: Option<String>) {
//     debug!("pub fn track_change_type_MidiInputChannelChanged not yet implemented!");
// }

pub fn track_change_type_InstrumentChanged(state: &mut RiffDAWState, instrument_details: String, track_uuid: Option<String>) {
    if let Some(track_uuid) = track_uuid {
        state.load_instrument(instrument_details, track_uuid);
    }
    else {
        debug!("Main - rx_ui processing loop - track instrument changed - could not get lock on state");
    }
}

pub fn track_change_type_ShowInstrument(state: &mut RiffDAWState, track_uuid: Option<String>) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            if let Some(track_uuid) = track_uuid {
                match project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                    Some(track_type) => match track_type {
                        TrackType::InstrumentTrack(track) => {
                            if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
                                let _ = audio_layer_sender.send(AudioLayerEvent::TrackBackgroundProcessorInward(TrackBackgroundProcessorInwardEvent::ShowInstrument, track.uuid()));
                            }
                        },
                        TrackType::AudioTrack(_) => (),
                        TrackType::MidiTrack(_) => (),
                    },
                    None => (),
                }
            }
        },
        Err(_) => debug!("Main - rx_ui processing loop - show track instrument - could not get lock on state"),
    }
}

pub fn track_change_type_TrackNameChanged(state: &mut RiffDAWState, track_name: String, track_uuid: Option<String>) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let mut state = state;
            let track_uuid = track_uuid.unwrap();
            debug!("Track name changed: \"{}\", name=\"{}\"", track_name.as_str(), &track_uuid);
            match project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                Some(track) => {
                    track.set_name(track_name.clone());
                    // gui.change_track_name(track_uuid.clone(), track_name);
                },
                None => (),
            };
        },
        Err(_) => debug!("Main - rx_ui processing loop - Save As File - could not get lock on state"),
    };
}

pub fn track_change_type_EffectAdded(state: &mut RiffDAWState, uuid: Uuid, name: String, effect_details: String, track_uuid: Option<String>) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            match track_uuid {
                Some(track_uuid) => {
                    let track_uuid2 = track_uuid.clone();
                    let track_uuid_request_parameters = track_uuid.clone();
                    let fake1 = Arc::new(Mutex::new(HashMap::new()));
                    let fake2 = Arc::new(Mutex::new(HashMap::new()));
                    state.send_to_track_background_processor(track_uuid, TrackBackgroundProcessorInwardEvent::AddEffect(fake1, fake2, uuid.clone(), effect_details.clone()));
                    if let Some(sender) = state.audio_layer_sender.as_ref() {
                        let _ = sender.send(AudioLayerEvent::TrackBackgroundProcessorInward(TrackBackgroundProcessorInwardEvent::RequestEffectParameters(uuid.to_string()), track_uuid_request_parameters));
                    }
                    match project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid2) {
                        Some(track_type) => match track_type {
                            TrackType::InstrumentTrack(track) => {
                                let (sub_plugin_id, library_path, plugin_type) = get_plugin_details(effect_details.clone());
                                let effect = AudioPlugin::new_with_uuid(uuid.to_string(), name, library_path, sub_plugin_id, plugin_type);
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
}

pub fn track_change_type_EffectDeleted(state: &mut RiffDAWState, effect_uuid: String, track_uuid: Option<String>) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            match track_uuid.clone() {
                Some(track_hash) => state.send_to_track_background_processor(track_hash, TrackBackgroundProcessorInwardEvent::DeleteEffect(effect_uuid.clone())),
                None => (),
            }
            if let Some(track_uuid) = track_uuid {
                let track_uuid2 = track_uuid;
                if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid2) {
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

pub fn track_change_type_RiffAdd(state: &mut RiffDAWState, name: String, length: f64, track_uuid: Option<String>) {
    debug!("Main - rx_ui processing loop - riff add");

    let action = RiffAdd::new_with_track_id(Uuid::new_v4(), name, length, track_uuid);
    let history_manager = state.history_manager.clone();

    if let Ok(history_manager) = history_manager.lock().as_mut() {
        match history_manager.apply(state, Box::new(action)) {
            Ok(mut daw_events_to_propagate) => {
                for _ in 0..daw_events_to_propagate.len() {
                    let event = daw_events_to_propagate.remove(0);
                    // let _ = tx_from_ui.send(event);
                }
            }
            Err(error) => {
                error!("Main - rx_ui processing loop - riff add - error: {}", error);
            }
        }
    }
}

// pub fn track_change_type_RiffAddWithTrackIndex(state: &mut RiffDAWState, uuid: String, length: String, track_index: i32) {
//     debug!("Main - rx_ui processing loop - pub fn track_change_type_RiffAddWithTrackIndex");
//
//     // display a dialogue and prompt to get the riff name
//     let mut name = "".to_string();
//     while name.is_empty() {
//         if gui.ui.riff_name_dialogue.run() == gtk::ResponseType::Ok && gui.ui.riff_name_entry.text().len() > 0 {
//             name = gui.ui.riff_name_entry.text().to_string();
//             // gui.ui.riff_name_entry.set_text("");
//         }
//     }
//     // gui.ui.riff_name_dialogue.hide();
//
//     // get the track id
//     let track_id = if let Ok(project) = state.get_project().lock().as_mut() {
//         if let Some(track) = project.song().tracks().get(track_index as usize) {
//             Some(track.uuid().to_string())
//         }
//         else { None }
//     }
//     else { None };
//
//     let action = RiffAdd::new_with_track_id(Uuid::parse_str(uuid.clone().as_str()).unwrap(), name, length, &mut state.clone(), track_id.clone());
//     match state.history_manager.apply(state, Box::new(action)) {
//         Ok(mut daw_events_to_propagate) => {
//             // set the selected riff
//             if let Some(track_id) = track_id {
//                 state.set_selected_riff_uuid(track_id, uuid);
//             }
//
//             for _ in 0..daw_events_to_propagate.len() {
//                 let event = daw_events_to_propagate.remove(0);
//                 // let _ = tx_from_ui.send(event);
//             }
//         }
//         Err(error) => {
//             error!("Main - rx_ui processing loop - pub fn track_change_type_RiffAddWithTrackIndex - error: {}", error);
//         }
//     }
// }

pub fn track_change_type_RiffCopy(state: &mut RiffDAWState, uuid_to_copy: String, track_uuid: Option<String>) {
    debug!("Main - rx_ui processing loop - riff copy");
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            state.set_selected_riff_ref_uuid(None);

            match track_uuid {
                Some(track_uuid) => {
                    let new_riff_uuid = Uuid::new_v4();

                    state.set_selected_track(Some(track_uuid.clone()));
                    state.set_selected_riff_uuid(track_uuid.clone(), new_riff_uuid.to_string());

                    // get the riff to copy and clone it
                    match project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                        Some(track) => {
                            if let Some(riff) = track.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == uuid_to_copy) {
                                let mut new_riff = riff.clone();
                                new_riff.set_uuid(new_riff_uuid);
                                new_riff.set_name(format!("Copy of {}", new_riff.name()));
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
}

// pub fn track_change_type_RiffDelete(state: &mut RiffDAWState, riff_uuid: String, track_uuid: Option<String>) {
//     debug!("Need to handle track riff deleted.");
//
//     // check if any riff references are using this riff - if so then show a warning dialog
//     let found_info = match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             let mut found_info = vec![];
//             let mut riff_name = String::from("Unknown");
//
//             // process the track
//             if let Some(uuid) = track_uuid.clone() {
//                 if let Some(track) = project.song().tracks().iter().find(|track| track.uuid().to_string() == uuid) {
//                     // get the riff name
//                     if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_uuid) {
//                         riff_name = riff.name().to_string();
//                     }
//
//                     // check track riff refs
//                     for riff_ref in track.riff_refs().iter() {
//                         if riff_ref.linked_to() == riff_uuid {
//                             let message = format!("Track: \"{}\" has references to riff: \"{}\".", track.name(), riff_name.as_str());
//
//                             if !found_info.iter().any(|entry| *entry == message) {
//                                 found_info.push(message);
//                             }
//                         }
//                     }
//                 }
//             }
//
//             // check riff sets
//             for riff_set in project.song().riff_sets().iter() {
//                 for (_, riff_ref) in riff_set.riff_refs().iter() {
//                     if riff_ref.linked_to() == riff_uuid {
//                         let message = format!("Riff set: \"{}\" has a reference to riff: \"{}\".", riff_set.name(), riff_name.as_str());
//
//                         if !found_info.iter().any(|entry| *entry == message) {
//                             found_info.push(message);
//                         }
//                     }
//                 }
//             }
//
//             // check riff sequences
//             for riff_sequence in project.song().riff_sequences().iter() {
//                 for riff_set_item in riff_sequence.riff_sets().iter() {
//                     if let Some(riff_set) = project.song().riff_set(riff_set_item.item_uuid().to_string()) {
//                         for (_, riff_ref) in riff_set.riff_refs().iter() {
//                             if riff_ref.linked_to() == riff_uuid {
//                                 let message = format!("Riff sequence: \"{}\" has references to riff: \"{}\".", riff_sequence.name(), riff_name.as_str());
//
//                                 if !found_info.iter().any(|entry| *entry == message) {
//                                     found_info.push(message);
//                                 }
//                             }
//                         }
//                     }
//                 }
//             }
//
//             // check riff grids
//             if let Some(uuid) = track_uuid.clone() {
//                 for riff_grid in project.song().riff_grids().iter() {
//                     if let Some(riff_references) = riff_grid.track_riff_references(uuid.clone()) {
//                         for riff_reference in riff_references.iter() {
//                             if riff_reference.linked_to() == riff_uuid {
//                                 let message = format!("Riff grid: \"{}\" has references to riff: \"{}\".", riff_grid.name(), riff_name.as_str());
//
//                                 if !found_info.iter().any(|entry| *entry == message) {
//                                     found_info.push(message);
//                                 }
//                             }
//                         }
//                     }
//                 }
//             }
//
//             // check riff arrangements
//             for riff_arrangement in project.song().riff_arrangements().iter() {
//                 for riff_item in riff_arrangement.items().iter() {
//                     match *(riff_item.item_type()) {
//                         RiffItemType::RiffSet => {
//                             if let Some(riff_set) = project.song().riff_set(riff_item.item_uuid().to_string()) {
//                                 for (_, riff_ref) in riff_set.riff_refs().iter() {
//                                     if riff_ref.linked_to() == riff_uuid {
//                                         let message = format!("Riff arrangement: \"{}\" has references to riff: \"{}\".", riff_arrangement.name(), riff_name.as_str());
//
//                                         if !found_info.iter().any(|entry| *entry == message) {
//                                             found_info.push(message);
//                                         }
//                                     }
//                                 }
//                             }
//                         }
//                         RiffItemType::RiffSequence => {
//                             if let Some(riff_sequence) = project.song().riff_sequence(riff_item.uuid()) {
//                                 for riff_set_item in riff_sequence.riff_sets().iter() {
//                                     if let Some(riff_set) = project.song().riff_set(riff_set_item.item_uuid().to_string()) {
//                                         for (_, riff_ref) in riff_set.riff_refs().iter() {
//                                             if riff_ref.linked_to() == riff_uuid {
//                                                 let message = format!("Riff arrangement: \"{}\" has references to riff: \"{}\".", riff_arrangement.name(), riff_name.as_str());
//
//                                                 if !found_info.iter().any(|entry| *entry == message) {
//                                                     found_info.push(message);
//                                                 }
//                                             }
//                                         }
//                                     }
//                                 }
//                             }
//                         }
//                         RiffItemType::RiffGrid => {
//                             if let Some(uuid) = track_uuid.clone() {
//                                 if let Some(riff_grid) = project.song().riff_grid(riff_item.uuid()) {
//                                     if let Some(riff_references) = riff_grid.track_riff_references(uuid.clone()) {
//                                         for riff_reference in riff_references.iter() {
//                                             if riff_reference.linked_to() == riff_uuid {
//                                                 let message = format!("Riff arrangement: \"{}\" has references to riff: \"{}\".", riff_arrangement.name(), riff_name.as_str());
//
//                                                 if !found_info.iter().any(|entry| *entry == message) {
//                                                     found_info.push(message);
//                                                 }
//                                             }
//                                         }
//                                     }
//                                 }
//                             }
//                         }
//                     }
//                 }
//             }
//
//             found_info
//         }
//         Err(_) => {
//             debug!("Main - rx_ui processing loop - track_riff_delete - could not get lock on state");
//             vec![]
//         }
//     };
//
//     // if no riff refs are using this riff then delete it from the track
//     if found_info.len() == 0 {
//         let action = RiffDelete::new(riff_uuid, track_uuid);
//         match state.history_manager.apply(state, Box::new(action)) {
//             Ok(mut daw_events_to_propagate) => {
//                 for _ in 0..daw_events_to_propagate.len() {
//                     let event = daw_events_to_propagate.remove(0);
//                     let _ = tx_from_ui.send(event);
//                 }
//             }
//             Err(error) => {
//                 error!("Main - rx_ui processing loop - riff delete - error: {}", error);
//             }
//         }
//     } else {
//         let mut error_message = String::from("Could not delete riff:\n");
//
//         for message in found_info.iter() {
//             error_message.push_str(message.as_str());
//             error_message.push_str("\n");
//         }
//
//         let _ = tx_from_ui.send(DAWEvents::Notification(NotificationType::Error, error_message));
//     }
// }

pub fn track_change_type_RiffLengthChange(state: &mut RiffDAWState, riff_uuid: String, riff_length: f64, track_uuid: Option<String>) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let mut state = state;
            match track_uuid {
                Some(track_uuid) => {
                    state.set_selected_track(Some(track_uuid.clone()));
                    state.set_selected_riff_uuid(track_uuid.clone(), riff_uuid.clone());
                    state.set_selected_riff_ref_uuid(None);

                    match project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
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
}

pub fn track_change_type_RiffReferenceAdd(state: &mut RiffDAWState, track_index: i32, position: f64, track_uuid: Option<String>) {
    let mut selected_riff_uuid = None;
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let song = project.song();
            let tracks = song.tracks();

            match tracks.get(track_index as usize) {
                Some(track) => selected_riff_uuid = state.selected_riff_uuid(track.uuid().to_string()),
                None => debug!("Main - rx_ui processing loop - track riff reference added - no track at index."),
            };
        },
        Err(_) => debug!("Main - rx_ui processing loop - track_riff_edit - could not get lock on state"),
    };
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let song = project.song_mut();
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
}

pub fn track_change_type_RiffReferenceDelete(state: &mut RiffDAWState, track_index:i32, position: f64, track_uuid: Option<String>) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            {
                let mut state = state;
                let song = project.song_mut();
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
}

pub fn track_change_type_RiffAddNote(state: &mut RiffDAWState, new_notes: Vec<(i32, f64, f64)>, track_uuid: Option<String>) {

    let note_id = state.piano_roll_mpe_note_id().clone() as i32;
    for (note, position, duration) in new_notes.iter() {
        let action = RiffAddNoteAction::new(note_id, *position, *note, 127, *duration, state);
        let history_manager = state.history_manager.clone();
        if let Ok(history_manager) = history_manager.lock().as_mut() {
            if let Err(error) = history_manager.apply(state, Box::new(action)) {
                error!("Main - rx_ui processing loop - riff add note - error: {}", error);
            }
        }
    }

    let mut midi_channel = 0;
    let mut tempo = 140.0;
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            tempo = project.song().tempo();
            let track_uuid = state.selected_track();
            match track_uuid {
                Some(track_uuid) => {
                    match project.song().tracks().iter().find(|track| track.uuid().to_string() == track_uuid) {
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
        for (note, _position, duration) in new_notes {
            thread::sleep(Duration::from_millis((duration * 60.0 / tempo * 1000.0) as u64));
            match state.selected_track() {
                Some(track_uuid) => {
                    state.send_to_track_background_processor(track_uuid, TrackBackgroundProcessorInwardEvent::StopNoteImmediate(note, midi_channel));
                }
                None => {}
            }
        }
    }
}

pub fn track_change_type_RiffDeleteNote(state: &mut RiffDAWState, note_number: i32, position: f64, track_uuid: Option<String>) {
    let action = RiffDeleteNoteAction::new(position, note_number, state);
    let history_manager = state.history_manager.clone();
    if let Ok(history_manager) = history_manager.lock().as_mut() {
        if let Err(error) = history_manager.apply(state, Box::new(action)) {
            error!("Main - rx_ui processing loop - riff delete note - error: {}", error);
        }
    }
}

// pub fn track_change_type_RiffAddSample(state: &mut RiffDAWState, sample_reference_uuid: String, position: f64, track_uuid: Option<String>) {
//     let mut selected_riff_uuid = None;
//     let mut selected_riff_track_uuid = None;
//     match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             selected_riff_track_uuid = state.selected_track();
//
//             match selected_riff_track_uuid {
//                 Some(track_uuid) => {
//                     selected_riff_uuid = state.selected_riff_uuid(track_uuid.clone());
//                     selected_riff_track_uuid = Some(track_uuid);
//                 },
//                 None => (),
//             }
//         },
//         Err(_) => debug!("Main - rx_ui processing loop - riff add sample - could not get lock on state"),
//     };
//     match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             let mut state = state;
//
//             match selected_riff_track_uuid {
//                 Some(track_uuid) => {
//                     for track in project.song_mut().tracks_mut().iter_mut() {
//                         match selected_riff_uuid.clone() {
//                             Some(riff_uuid) => {
//                                 for riff in track.riffs_mut().iter_mut() {
//                                     if riff.uuid().to_string() == *riff_uuid {
//                                         riff.events_mut().push(TrackEvent::Sample(SampleReference::new(position, sample_reference_uuid.clone())));
//                                         break;
//                                     }
//                                 }
//                             }
//                             None => debug!("Main - rx_ui processing loop - riff add sample - problem getting selected riff index"),
//                         }
//                     }
//
//                     // FIXME - this only needs to happen once per sample_data not every time it is added to a riff reference
//                     // find the sample and then the sample data
//                     if let Some(sample) = project.song().samples().get(&sample_reference_uuid) {
//                         if let Some(sample_data) = state.sample_data().get(&sample.sample_data_uuid().to_string()) {
//                             // send the sample data to the track background processor
//                             state.send_to_track_background_processor(track_uuid, TrackBackgroundProcessorInwardEvent::SetSample(sample_data.clone()));
//                         }
//                     }
//                 },
//                 None => debug!("Main - rx_ui processing loop - riff add sample  - problem getting selected riff track number"),
//             };
//         },
//         Err(_) => debug!("Main - rx_ui processing loop - riff add sample - could not get lock on state"),
//     };
//     // gui.ui.sample_roll_drawing_area.queue_draw();
//     // gui.ui.track_drawing_area.queue_draw();
// }
//
// pub fn track_change_type_RiffDeleteSample(state: &mut RiffDAWState, sample_reference_uuid: String, position: f64, track_uuid: Option<String>) {
//     debug!("Main - rx_ui processing loop - riff delete sample: sample_reference_uuid={}, position={}", sample_reference_uuid, position);
//     let mut selected_riff_uuid = None;
//     let mut selected_riff_track_uuid = None;
//     match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             selected_riff_track_uuid = state.selected_track();
//
//             match selected_riff_track_uuid {
//                 Some(track_uuid) => {
//                     selected_riff_uuid = state.selected_riff_uuid(track_uuid.clone());
//                     selected_riff_track_uuid = Some(track_uuid);
//                 },
//                 None => (),
//             }
//         },
//         Err(_) => debug!("Main - rx_ui processing loop - riff delete sample - could not get lock on state"),
//     };
//     match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             let mut state = state;
//
//             match selected_riff_track_uuid {
//                 Some(_track_uuid) => {
//                     for track in project.song_mut().tracks_mut().iter_mut() {
//                         match selected_riff_uuid.clone() {
//                             Some(riff_uuid) => {
//                                 for riff in track.riffs_mut().iter_mut() {
//                                     if riff.uuid().to_string() == *riff_uuid {
//                                         debug!("Main - rx_ui processing loop - riff delete sample - found the riff");
//                                         riff.events_mut().retain(|event| match event {
//                                             TrackEvent::Sample(sample) => !((sample.position() - 0.01) <= position && position <= (sample.position() + 0.25)),
//                                             _ => true,
//                                         });
//                                     }
//                                     break;
//                                 }
//                             }
//                             None => debug!("Main - rx_ui processing loop - riff delete sample - problem getting selected riff index"),
//                         }
//                     }
//                 },
//                 None => debug!("Main - rx_ui processing loop - riff delete sample  - problem getting selected riff track number"),
//             };
//         },
//         Err(_) => debug!("Main - rx_ui processing loop - riff delete sample - could not get lock on state"),
//     };
//     // gui.ui.sample_roll_drawing_area.queue_draw();
//     // gui.ui.track_drawing_area.queue_draw();
// }

pub fn track_change_type_RiffSelect(state: &mut RiffDAWState, riff_id: String, track_uuid: Option<String>) {
    let mut riff_id_final = riff_id.clone();
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let mut state = state;
            match track_uuid {
                Some(track_uuid) => {
                    state.set_selected_track(Some(track_uuid.clone()));
                    state.set_selected_riff_ref_uuid(None);
                    //find the riff id via a riff set - alternate path
                    let riff_id_option = if let Some(riff_set) = project.song().riff_sets().iter().find(|riff_set| riff_set.uuid() == riff_id) {
                        if let Some((_, riff_ref)) = riff_set.riff_refs().iter().find(|(current_track_uuid, _riff_ref)| current_track_uuid.to_string() == track_uuid) {
                            Some(riff_ref.linked_to())
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    // map(|riff_set| riff_set.riff_refs().get(&track_uuid))
                    for track in project.song_mut().tracks_mut().iter_mut() {
                        if track.uuid().to_string() == track_uuid.clone() {
                            // if let TrackType::AudioTrack(_) = track {
                            //     match tx_from_ui.send(DAWEvents::SampleRollSetTrackName(track.name().to_string())) {
                            //         Ok(_) => (),
                            //         Err(_) => (),
                            //     }
                            // } else {
                            //     match tx_from_ui.send(DAWEvents::PianoRollSetTrackName(track.name().to_string())) {
                            //         Ok(_) => (),
                            //         Err(_) => (),
                            //     }
                            // }
                            let riff_option = if let Some(riff_id) = riff_id_option {
                                riff_id_final = riff_id.clone();
                                track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_id)
                            } else {
                                track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_id)
                            };
                            if let Some(riff) = riff_option {
                                // if let TrackType::AudioTrack(_) = track {
                                //     match tx_from_ui.send(DAWEvents::SampleRollSetRiffName(riff.name().to_string())) {
                                //         Ok(_) => (),
                                //         Err(_) => (),
                                //     }
                                // } else {
                                //     match tx_from_ui.send(DAWEvents::PianoRollSetRiffName(riff.name().to_string())) {
                                //         Ok(_) => (),
                                //         Err(_) => (),
                                //     }
                                // }
                                break;
                            }
                            break;
                        }
                    }

                    state.set_selected_riff_uuid(track_uuid, riff_id_final.clone());
                    state.selected_riff_uuid = riff_id_final;
                },
                None => debug!("Main - rx_ui processing loop - track_riff_selected - no track number specified."),
            }
        },
        Err(_) => debug!("Main - rx_ui processing loop - track_riff_selected - could not get lock on state"),
    };
}

// pub fn track_change_type_RiffSelectWithTrackIndex(state: &mut RiffDAWState, track_index: i32, position: f64, track_uuid: Option<String>) {
//     debug!("Main - rx_ui processing loop - pub fn track_change_type_RiffSelectWithTrackIndex");
//     match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             // get the track
//             let track_riff = if let Some(track) = project.song_mut().tracks_mut().get_mut(track_index as usize) {
//                 let track_uuid = track.uuid().to_string();
//                 let track_name = track.name().to_string();
//                 let riff_details = track.riffs_mut().iter_mut().map(|riff| (riff.id(), (riff.name().to_string(), riff.length()))).collect::<HashMap<String, (String, f64)>>();
//                 let mut riff_name = None;
//                 if let Some(riff_ref) = track.riff_refs_mut().iter_mut().find(|riff_ref| {
//                     if let Some((name, riff_length)) = riff_details.get(&riff_ref.linked_to()) {
//                         riff_name = Some(name.to_string());
//                         let riff_ref_end_position = riff_ref.position() + *riff_length;
//                         if riff_ref.position() <= position && position <= riff_ref_end_position {
//                             true
//                         }
//                         else { false }
//                     }
//                     else { false }
//                 }) {
//                     if let Some(riff_name) = riff_name {
//                         if riff_name.as_str() != "empty" {
//                             Some((track_uuid, riff_ref.linked_to(), track_name.to_string(), riff_name))
//                         }
//                         else { None }
//                     }
//                     else { None }
//                 }
//                 else { None }
//             }
//             else { None };
//
//             if let Some((track_uuid, riff_uuid, track_name, riff_name)) = track_riff {
//                 state.set_selected_riff_uuid(track_uuid.clone(), riff_uuid);
//                 state.set_selected_track(Some(track_uuid));
//                 // gui.set_piano_roll_selected_track_name_label(track_name.as_str());
//                 // gui.set_piano_roll_selected_riff_name_label(riff_name.as_str());
//                 // gui.ui.piano_roll_drawing_area.queue_draw();
//             }
//         }
//         Err(_) => debug!("Main - rx_ui processing loop - pub fn track_change_type_RiffSelectWithTrackIndex - could not get lock on state"),
//     }
// }

pub fn track_change_type_RiffEventsSelectMultiple(state: &mut RiffDAWState, x: f64, y: i32, x2: f64, y2: i32, add_to_select: bool, track_uuid: Option<String>) {
    let mut selected = Vec::new();
    let mut selected_riff_uuid = None;
    let mut selected_riff_track_uuid = state.selected_track();

    match selected_riff_track_uuid {
        Some(track_uuid) => {
            selected_riff_uuid = state.selected_riff_uuid(track_uuid.clone());
            selected_riff_track_uuid = Some(track_uuid);
        },
        None => (),
    }

    match state.get_project().lock().as_mut() {
        Ok(project) => {

            match selected_riff_track_uuid {
                Some(track_uuid) => {
                    match project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
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
    // gui.ui.piano_roll_drawing_area.queue_draw();
}

// pub fn track_change_type_RiffEventsSelectSingle(state: &mut RiffDAWState, x: f64, y: i32, add_to_select: bool, track_uuid: Option<String>) {
//     let mut selected = Vec::new();
//     let mut selected_riff_uuid = None;
//     let mut selected_riff_track_uuid = None;
//     match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             selected_riff_track_uuid = state.selected_track();
//
//             match selected_riff_track_uuid {
//                 Some(track_uuid) => {
//                     selected_riff_uuid = state.selected_riff_uuid(track_uuid.clone());
//                     selected_riff_track_uuid = Some(track_uuid);
//                 },
//                 None => (),
//             }
//         },
//         Err(_) => debug!("Main - rx_ui processing loop - RiffEventsSelectSingle - could not get lock on state"),
//     };
//     match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             let mut state = state;
//
//             match selected_riff_track_uuid {
//                 Some(track_uuid) => {
//                     match project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
//                         Some(track) => {
//                             match selected_riff_uuid {
//                                 Some(riff_uuid) => {
//                                     'outer_loop:
//                                     for riff in track.riffs_mut().iter_mut() {
//                                         if riff.uuid().to_string() == *riff_uuid {
//                                             // store the notes for an undo
//                                             for track_event in riff.events_mut().iter_mut() {
//                                                 if let TrackEvent::Note(note) = track_event {
//                                                     if note.note() == y && note.position()<= x && x <= (note.position() + note.length()) {
//                                                         debug!("RiffEventsSelectSingle Note selected: x={}, y={}, note position={}, note={}, note duration={}", x, y, note.position(), note.note(), note.length());
//                                                         selected.push(note.id());
//                                                         break 'outer_loop;
//                                                     }
//                                                 }
//                                             }
//                                             break;
//                                         }
//                                     }
//                                 },
//                                 None => debug!("Main - rx_ui processing loop - RiffEventsSelectSingle - problem getting selected riff index"),
//                             }
//                         },
//                         None => ()
//                     }
//                 },
//                 None => debug!("Main - rx_ui processing loop - RiffEventsSelectSingle  - problem getting selected riff track number"),
//             };
//
//             if !selected.is_empty() {
//                 let mut state = state;
//                 if !add_to_select {
//                     state.selected_riff_events_mut().clear();
//                 }
//                 state.selected_riff_events_mut().append(&mut selected);
//             } else {
//                 state.selected_riff_events_mut().clear();
//             }
//         },
//         Err(_) => debug!("Main - rx_ui processing loop - RiffEventsSelectSingle - could not get lock on state"),
//     }
//     // gui.ui.piano_roll_drawing_area.queue_draw();
// }
//
// pub fn track_change_type_RiffEventsDeselectMultiple(state: &mut RiffDAWState, x: f64, y: i32, x2: f64, y2: i32) {
//     let mut selected = Vec::new();
//     let mut selected_riff_uuid = None;
//     let mut selected_riff_track_uuid = None;
//     match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             selected_riff_track_uuid = state.selected_track();
//
//             match selected_riff_track_uuid {
//                 Some(track_uuid) => {
//                     selected_riff_uuid = state.selected_riff_uuid(track_uuid.clone());
//                     selected_riff_track_uuid = Some(track_uuid);
//                 },
//                 None => (),
//             }
//         },
//         Err(_) => debug!("Main - rx_ui processing loop - RiffEventsDeselectMultiple - could not get lock on state"),
//     };
//     match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             let mut state = state;
//
//             match selected_riff_track_uuid {
//                 Some(track_uuid) => {
//                     match project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
//                         Some(track) => {
//                             match selected_riff_uuid {
//                                 Some(riff_uuid) => {
//                                     for riff in track.riffs_mut().iter_mut() {
//                                         if riff.uuid().to_string() == *riff_uuid {
//                                             // store the notes for an undo
//                                             for track_event in riff.events_mut().iter_mut() {
//                                                 if let TrackEvent::Note(note) = track_event {
//                                                     if y <= note.note() && note.note() <= y2 && x <= note.position() && (note.position() + note.length()) <= x2 {
//                                                         debug!("Note selected: x={}, y={}, x2={}, y2={}, note position={}, note={}, note duration={}", x, y, x2, y2, note.position(), note.note(), note.length());
//                                                         selected.push(note.id());
//                                                     }
//                                                 }
//                                             }
//                                             break;
//                                         }
//                                     }
//                                 },
//                                 None => debug!("Main - rx_ui processing loop - RiffEventsDeselectMultiple - problem getting selected riff index"),
//                             }
//                         },
//                         None => ()
//                     }
//                 },
//                 None => debug!("Main - rx_ui processing loop - RiffEventsDeselectMultiple  - problem getting selected riff track number"),
//             };
//
//             if !selected.is_empty() {
//                 let mut state = state;
//                 state.selected_riff_events_mut().retain(|riff_ref_id| !selected.contains(riff_ref_id));
//             } else {
//                 state.selected_riff_events_mut().clear();
//             }
//         },
//         Err(_) => debug!("Main - rx_ui processing loop - RiffEventsDeselectMultiple - could not get lock on state"),
//     }
//     // gui.ui.piano_roll_drawing_area.queue_draw();
// }

pub fn track_change_type_RiffEventsDeselectSingle(state: &mut RiffDAWState, x: f64, y: i32) {
    let mut selected = Vec::new();
    let mut selected_riff_uuid = None;
    let mut selected_riff_track_uuid = None;

    selected_riff_track_uuid = state.selected_track();

    match selected_riff_track_uuid {
        Some(track_uuid) => {
            selected_riff_uuid = state.selected_riff_uuid(track_uuid.clone());
            selected_riff_track_uuid = Some(track_uuid);
        },
        None => (),
    }

    match state.get_project().lock().as_mut() {
        Ok(project) => {

            match selected_riff_track_uuid {
                Some(track_uuid) => {
                    match project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
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
        },
        Err(_) => debug!("Main - rx_ui processing loop - RiffEventsDeselectSingle - could not get lock on state"),
    }

    if !selected.is_empty() {
        state.selected_riff_events_mut().retain(|riff_ref_id| !selected.contains(riff_ref_id));
    } else {
        state.selected_riff_events_mut().clear();
    }
}
pub fn track_change_type_RiffEventsSelectAll(state: &mut RiffDAWState) {
    let mut selected = Vec::new();
    let mut selected_riff_uuid = None;
    let mut selected_riff_track_uuid = None;

    selected_riff_track_uuid = state.selected_track();

    match selected_riff_track_uuid {
        Some(track_uuid) => {
            selected_riff_uuid = state.selected_riff_uuid(track_uuid.clone());
            selected_riff_track_uuid = Some(track_uuid);
        },
        None => (),
    }

    match state.get_project().lock() {
        Ok(mut project) => {
            match selected_riff_track_uuid {
                Some(track_uuid) => {
                    match project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
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
        },
        Err(_) => debug!("Main - rx_ui processing loop - RiffEventsSelectAll - could not get lock on state"),
    }

    if !selected.is_empty() {
        state.selected_riff_events_mut().clear();
        state.selected_riff_events_mut().append(&mut selected);
    } else {
        state.selected_riff_events_mut().clear();
    }
}

pub fn track_change_type_RiffEventsDeselectAll(state: &mut RiffDAWState) {
    state.selected_riff_events_mut().clear();
}

pub fn track_change_type_RiffCutSelected(state: &mut RiffDAWState) {
    let mut selected_riff_uuid = None;
    let mut selected_riff_track_uuid = None;
    let mut selected_riff_events = vec![];

    match state.get_project().lock().as_mut() {
        Ok(project) => {
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
        let action = RiffCutSelectedAction::new(selected_riff_track_uuid, selected_riff_uuid, selected_riff_events);
        let history_manager = state.history_manager.clone();
        if let Ok(history_manager) = history_manager.lock().as_mut() {
            if let Err(error) = history_manager.apply(state, Box::new(action)) {
                error!("Main - rx_ui processing loop - riff cut selected notes - error: {}", error);
            }
        }
    }
}

pub fn track_change_type_RiffTranslateSelected(state: &mut RiffDAWState, translation_entity_type: TranslationEntityType, translate_direction: TranslateDirection) {
    let mut selected_riff_uuid = None;
    let mut selected_riff_track_uuid = state.selected_track();
    let mut selected_riff_track_uuid2 = state.selected_track().clone();
    let mut selected_riff_events = vec![];

    match selected_riff_track_uuid {
        Some(track_uuid) => {
            selected_riff_uuid = state.selected_riff_uuid(track_uuid.clone()).clone();
            selected_riff_events = state.selected_riff_events_mut().clone();
        },
        None => (),
    }

    {
        let mut snap_in_beats = 1.0;
        // match gui.piano_roll_grid() {
        //     Some(piano_roll_grid) => match piano_roll_grid.lock() {
        //         Ok(piano_roll) => snap_in_beats = piano_roll.snap_position_in_beats(),
        //         Err(_) => (),
        //     },
        //     None => (),
        // }
        let action = RiffTranslateSelectedAction::new(
            selected_riff_track_uuid2.clone(),
            selected_riff_uuid,
            selected_riff_events,
            translation_entity_type,
            translate_direction,
            snap_in_beats
        );
        let history_manager = state.history_manager.clone();
        if let Ok(history_manager) = history_manager.lock().as_mut() {
            if let Err(error) = history_manager.apply(state, Box::new(action)) {
                error!("Main - rx_ui processing loop - riff translate selected - error: {}", error);
            }
        }
    }
}

// pub fn track_change_type_Record(state: &mut RiffDAWState, _record: bool) {
//     // TODO implement arming of tracks for recording into???
//     // match state.get_project().lock().as_mut() {
//     //     Ok(project) => {
//     //         // state.set_recording(record);
//     //     },
//     //     Err(_) => debug!("Main - rx_ui processing loop - transport goto start - could not get lock on state"),
//     // }
// }

pub fn track_change_type_RiffQuantiseSelected(state: &mut RiffDAWState) {
    let mut selected_riff_uuid = None;
    let mut selected_riff_track_uuid = None;
    let mut selected_riff_events = vec![];

    selected_riff_track_uuid = state.selected_track();

    match selected_riff_track_uuid {
        Some(track_uuid) => {
            selected_riff_uuid = state.selected_riff_uuid(track_uuid.clone());
            selected_riff_track_uuid = Some(track_uuid);
            selected_riff_events = state.selected_riff_events_mut().clone();
        },
        None => (),
    }

    {
        let snap_amount_str = *(MUSICAL_ITEM_LENGTH_OPTIONS.get(state.piano_roll_state.piano_roll_selected_snap).unwrap());
        let mut snap_in_beats = DAWUtils::get_snap_quantise_value_in_beats_from_choice_text(snap_amount_str, 4.0);
        let mut snap_strength = state.piano_roll_state.piano_roll_quantise_quantise_strength as f64 / 100.0;
        let mut snap_start = state.piano_roll_state.piano_roll_quantise_start;
        let mut snap_end = state.piano_roll_state.piano_roll_quantise_end;
        let action = RiffQuantiseSelectedAction::new(
            selected_riff_events,
            selected_riff_track_uuid,
            selected_riff_uuid,
            snap_in_beats,
            snap_strength,
            snap_start,
            snap_end,
        );
        let history_manager = state.history_manager.clone();
        if let Ok(history_manager) = history_manager.lock().as_mut() {
            if let Err(error) = history_manager.apply(state, Box::new(action)) {
                error!("Main - rx_ui processing loop - riff translate selected - error: {}", error);
            }
        }
    }
}

pub fn track_change_type_RiffCopySelected(state: &mut RiffDAWState) {
    let mut selected_riff_uuid = None;
    let mut selected_riff_track_uuid = None;
    match state.get_project().lock().as_mut() {
        Ok(project) => {
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
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            {
                let selected = state.selected_riff_events().to_vec();

                match selected_riff_track_uuid {
                    Some(track_uuid) => {
                        match project.song().tracks().iter().find(|track| track.uuid().to_string() == track_uuid) {
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

    let mut edit_cursor_position_in_beats = state.piano_roll_state.piano_roll_edit_cursor_position.clone();
    if !copy_buffer.is_empty() {
        let mut state = state;
        state.track_event_copy_buffer_mut().clear();
        copy_buffer.iter().for_each(|event| {
            let value = event.clone();
            match value {
                TrackEvent::ActiveSense => debug!("pub fn track_change_type_RiffCopySelectedNotes ActiveSense not yet implemented!"),
                TrackEvent::AfterTouch => debug!("pub fn track_change_type_RiffCopySelectedNotes AfterTouch not yet implemented!"),
                TrackEvent::ProgramChange => debug!("pub fn track_change_type_RiffCopySelectedNotes ProgramChange not yet implemented!"),
                TrackEvent::Note(note) => {
                    let mut note_value = note;
                    note_value.set_position(note_value.position() - edit_cursor_position_in_beats);
                    state.track_event_copy_buffer_mut().push(TrackEvent::Note(note_value));
                },
                TrackEvent::NoteOn(_) => debug!("pub fn track_change_type_RiffCopySelectedNotes NoteOn not yet implemented!"),
                TrackEvent::NoteOff(_) => debug!("pub fn track_change_type_RiffCopySelectedNotes NoteOff not yet implemented!"),
                TrackEvent::Controller(_) => debug!("pub fn track_change_type_RiffCopySelectedNotes Controller not yet implemented!"),
                TrackEvent::PitchBend(_pitch_bend) => debug!("pub fn track_change_type_RiffCopySelectedNotes PitchBend not yet implemented!"),
                TrackEvent::KeyPressure => debug!("pub fn track_change_type_RiffCopySelectedNotes KeyPressure not yet implemented!"),
                TrackEvent::AudioPluginParameter(_) => debug!("pub fn track_change_type_RiffCopySelectedNotes AudioPluginParameter not yet implemented!"),
                TrackEvent::Sample(_sample) => debug!("pub fn track_change_type_RiffCopySelectedNotes Sample not yet implemented!"),
                TrackEvent::Measure(_) => {}
                TrackEvent::NoteExpression(_) => {}
            }
        });
    }
}

pub fn track_change_type_RiffPasteSelected(state: &mut RiffDAWState) {
    let mut selected_riff_uuid = None;
    let mut selected_riff_track_uuid = state.selected_track();

    match selected_riff_track_uuid {
        Some(track_uuid) => {
            selected_riff_uuid = state.selected_riff_uuid(track_uuid.clone());
            selected_riff_track_uuid = Some(track_uuid);
        },
        None => (),
    }

    let edit_cursor_position_in_secs = state.piano_roll_state.piano_roll_edit_cursor_position;
    {
        let action = RiffPasteSelectedAction::new(selected_riff_track_uuid, selected_riff_uuid, edit_cursor_position_in_secs);
        let history_manager = state.history_manager.clone();
        if let Ok(history_manager) = history_manager.lock().as_mut() {
            if let Err(error) = history_manager.apply(state, Box::new(action)) {
                error!("Main - rx_ui processing loop - riff paste selected notes - error: {}", error);
            }
        }
    }
}

pub fn track_change_type_RiffReferenceCutSelected(state: &mut RiffDAWState) {
    let mut copy_buffer: Vec<RiffReference> = vec![];

    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let current_view = state.current_view();
            if let CurrentView::RiffGrid = current_view {
                // let selected_riff_references = state.selected_riff_grid_riff_references().clone();
                // let edit_cursor_position_in_secs = if let Some(grid) = gui.riff_grid() {
                //     match grid.lock() {
                //         Ok(grid) => {
                //             grid.edit_cursor_time_in_beats()
                //         }
                //         Err(_) => 0.0,
                //     }
                // } else {
                //     0.0
                // };
                //
                // // get the selected riff grid
                // if let Some(selected_riff_grid) = state.selected_riff_grid_uuid().clone() {
                //     if let Some(riff_grid) = project.song_mut().riff_grid_mut(selected_riff_grid.clone()) {
                //         for track in riff_grid.tracks_mut().map(|track_uuid| track_uuid.clone()).collect_vec().iter() {
                //             if let Some(riff_refs) = riff_grid.track_riff_references_mut(track.clone()) {
                //                 riff_refs.retain(|riff_ref| {
                //                     if selected_riff_references.clone().contains(&riff_ref.uuid().to_string()) {
                //                         let mut value = riff_ref.clone();
                //                         value.set_position(value.position() - edit_cursor_position_in_secs);
                //                         value.set_track_id(track.clone());
                //                         copy_buffer.push(value);
                //                         false
                //                     } else { true }
                //                 });
                //             }
                //         }
                //     }
                // }
            }
            else if let CurrentView::Track = current_view {
                let selected_riff_references = state.selected_track_grid_riff_references().clone();
                let edit_cursor_position_in_secs = state.track_grid_state.track_grid_edit_cursor_position;

                for track in project.song_mut().tracks_mut().iter_mut() {
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

    match state.get_project().lock().as_mut() {
        Ok(project) => {
            if !copy_buffer.is_empty() {
                debug!("Riff references copy buffer length: {}", copy_buffer.len());
                let mut state = state;
                state.track_grid_riff_references_copy_buffer_mut().clear();
                copy_buffer.iter().for_each(|event| state.track_grid_riff_references_copy_buffer_mut().push(event.clone()));
            }
        },
        Err(_) => (),
    }
    // gui.ui.piano_roll_drawing_area.queue_draw();
    // gui.ui.track_drawing_area.queue_draw();
}

pub fn track_change_type_RiffReferenceCopySelected(state: &mut RiffDAWState) {
    let mut copy_buffer: Vec<RiffReference> = vec![];
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let current_view = state.current_view();
            if let CurrentView::RiffGrid = current_view {
                // let selected_riff_references = state.selected_riff_grid_riff_references().clone();
                // let edit_cursor_position_in_secs = if let Some(grid) = gui.riff_grid() {
                //     match grid.lock() {
                //         Ok(grid) => {
                //             grid.edit_cursor_time_in_beats()
                //         }
                //         Err(_) => 0.0,
                //     }
                // } else {
                //     0.0
                // };
                //
                // // get the selected riff grid
                // if let Some(selected_riff_grid) = state.selected_riff_grid_uuid().clone() {
                //     if let Some(riff_grid) = project.song_mut().riff_grid_mut(selected_riff_grid.clone()) {
                //         for track in riff_grid.tracks_mut().map(|track_uuid| track_uuid.clone()).collect_vec().iter() {
                //             if let Some(riff_refs) = riff_grid.track_riff_references_mut(track.clone()) {
                //                 riff_refs.iter().filter(|riff_ref| selected_riff_references.clone().contains(&riff_ref.uuid().to_string())).for_each(|riff_ref|  {
                //                     let mut value = riff_ref.clone();
                //                     value.set_position(value.position() - edit_cursor_position_in_secs);
                //                     value.set_track_id(track.clone());
                //                     copy_buffer.push(value);
                //                 });
                //             }
                //         }
                //     }
                // }
            }
            else if let CurrentView::Track = current_view {
                let selected_riff_references = state.selected_track_grid_riff_references().clone();
                let edit_cursor_position_in_secs = state.track_grid_state.track_grid_edit_cursor_position;

                for track in project.song_mut().tracks_mut().iter_mut() {
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

    if !copy_buffer.is_empty() {
        debug!("Riff references copy buffer length: {}", copy_buffer.len());
        state.track_grid_riff_references_copy_buffer_mut().clear();
        copy_buffer.iter().for_each(|event| state.track_grid_riff_references_copy_buffer_mut().push(event.clone()));
    }
}

pub fn track_change_type_RiffReferencePaste(state: &mut RiffDAWState) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let current_view = state.current_view();
            if let CurrentView::RiffGrid = current_view {
                // let edit_cursor_position_in_secs = if let Some(riff_grid) = gui.riff_grid() {
                //     match riff_grid.lock() {
                //         Ok(grid) => {
                //             grid.edit_cursor_time_in_beats()
                //         },
                //         Err(_) => 0.0,
                //     }
                // } else {
                //     0.0
                // };
                // let mut copy_buffer: Vec<RiffReference> = vec![];
                // let mut copy_buffer_riff_ref_ids: Vec<String> = vec![];
                //
                // state.riff_grid_riff_references_copy_buffer().iter().for_each(|riff_ref| {
                //     copy_buffer.push(riff_ref.clone());
                //     copy_buffer_riff_ref_ids.push(riff_ref.uuid().to_string());
                // });
                //
                // if let Some(selected_riff_grid) = state.selected_riff_grid_uuid().clone() {
                //     if let Some(riff_grid) = project.song_mut().riff_grid_mut(selected_riff_grid.clone()) {
                //         for track_uuid in riff_grid.tracks_mut().map(|track_uuid| track_uuid.clone()).collect_vec().iter() {
                //             for riff_ref in copy_buffer.iter() {
                //                 if track_uuid == riff_ref.track_id() {
                //                     if let Some(riff_refs) = riff_grid.track_riff_references_mut(track_uuid.clone()) {
                //                         riff_refs.push(RiffReference::new(riff_ref.linked_to(), riff_ref.position() + edit_cursor_position_in_secs));
                //                     }
                //                 }
                //             }
                //         }
                //     }
                // }
            }
            else if let CurrentView::Track = current_view {
                let edit_cursor_position_in_secs = state.track_grid_state.track_grid_edit_cursor_position;
                let mut copy_buffer: Vec<RiffReference> = vec![];
                let mut copy_buffer_riff_ref_ids: Vec<String> = vec![];
                state.track_grid_riff_references_copy_buffer().iter().for_each(|riff_ref| {
                    copy_buffer.push(riff_ref.clone());
                    copy_buffer_riff_ref_ids.push(riff_ref.uuid().to_string());
                });
                let mut state = state;

                for track in project.song_mut().tracks_mut().iter_mut() {
                    let track_uuid = track.uuid_mut().to_string();
                    for riff_ref in copy_buffer.iter() {
                        if track_uuid == riff_ref.track_id() {
                            track.riff_refs_mut().push(RiffReference::new(riff_ref.linked_to(), riff_ref.position() + edit_cursor_position_in_secs));
                        }
                    }
                }

                // re-calculate the song length
                project.song_mut().recalculate_song_length();
            }
        },
        Err(_) => debug!("Main - rx_ui processing loop - riff references paste selected - could not get lock on state"),
    };
    // gui.ui.track_drawing_area.queue_draw();
}

// pub fn track_change_type_Selected(state: &mut RiffDAWState, track_uuid: Option<String>) {
//     match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             state.set_selected_track(track_uuid.clone());
//             // gui.update_automation_ui_from_state(&mut state);
//             match track_uuid {
//                 Some(track_uuid) => {
//                     let riff_uuid = if let Some(riff_uuid) = state.selected_riff_uuid_mut(track_uuid.clone()) {
//                         riff_uuid.clone()
//                     } else {
//                         "".to_string()
//                     };
//
//                     for track in project.song_mut().tracks_mut().iter_mut() {
//                         if track.uuid().to_string() == track_uuid.clone() {
//                             if !riff_uuid.is_empty() {
//                                 if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == *riff_uuid) {
//                                     let riff_name = riff.name();
//
//                                     scroll_notes_into_view(gui, riff);
//
//                                     if let TrackType::AudioTrack(_) = track {
//                                         // gui.set_sample_roll_selected_riff_name_label(riff_name);
//                                     } else {
//                                         // gui.set_piano_roll_selected_riff_name_label(riff_name);
//                                     }
//                                 } else if let TrackType::AudioTrack(_) = track {
//                                     // gui.set_sample_roll_selected_riff_name_label("");
//                                 } else {
//                                     // gui.set_piano_roll_selected_riff_name_label("");
//                                 }
//                             } else if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == *riff_uuid) {
//                                 let riff_name = riff.name();
//
//                                 scroll_notes_into_view(gui, riff);
//
//                                 if let TrackType::AudioTrack(_) = track {
//                                     // gui.set_sample_roll_selected_riff_name_label(riff_name);
//                                 } else {
//                                     // gui.set_piano_roll_selected_riff_name_label(riff_name);
//                                 }
//                             } else if let TrackType::AudioTrack(_) = track {
//                                 // gui.set_sample_roll_selected_riff_name_label("");
//                             } else {
//                                 // gui.set_piano_roll_selected_riff_name_label("");
//                             }
//                             if let TrackType::AudioTrack(_) = track {
//                                 // gui.set_sample_roll_selected_track_name_label(track.name());
//                             } else {
//                                 // gui.set_piano_roll_selected_track_name_label(track.name());
//                             }
//                             break;
//                         }
//                     }
//                 },
//                 None => debug!("Main - rx_ui processing loop - track_riff_selected - no track number specified."),
//             }
//         },
//         Err(_) => debug!("Main - rx_ui processing loop - track selected - could not get lock on state"),
//     }
//     // gui.ui.piano_roll_drawing_area.queue_draw();
//     // gui.ui.automation_drawing_area.queue_draw();
// }

pub fn track_change_type_RiffChangeLengthOfSelected(state: &mut RiffDAWState, lengthen: bool) {
    let mut selected_riff_uuid = None;
    let mut selected_riff_track_uuid = state.selected_track();
    let mut selected_riff_events = vec![];

    match selected_riff_track_uuid {
        Some(track_uuid) => {
            selected_riff_uuid = state.selected_riff_uuid(track_uuid.clone());
            selected_riff_track_uuid = Some(track_uuid);
            selected_riff_events = state.selected_riff_events_mut().clone();
        },
        None => (),
    }

    {
        let length_increment_in_beats = DAWUtils::get_snap_quantise_value_in_beats_from_choice_text(MUSICAL_ITEM_LENGTH_OPTIONS.get(state.piano_roll_state.selected_piano_roll_note_adj).unwrap(), 4.0);
        let action = RiffChangeLengthOfSelectedAction::new(
            selected_riff_track_uuid,
            selected_riff_uuid,
            selected_riff_events,
            length_increment_in_beats,
            lengthen,
        );
        let history_manager = state.history_manager.clone();
        if let Ok(history_manager) = history_manager.lock().as_mut() {
            if let Err(error) = history_manager.apply(state, Box::new(action)) {
                error!("Main - rx_ui processing loop - riff change selected notes length - error: {}", error);
            }
        }
    }
}

pub fn track_change_type_RiffNameChange(state: &mut RiffDAWState, riff_uuid: String, name: String, track_uuid: Option<String>) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let mut state = state;

            match track_uuid {
                Some(track_uuid) => {
                    match project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
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
    // gui.ui.piano_roll_drawing_area.queue_draw();
    // gui.ui.track_drawing_area.queue_draw();
}

// pub fn track_change_type_AutomationSelectMultiple(state: &mut RiffDAWState, time_lower: f64, value_lower: i32, time_higher: f64, value_higher: i32, add_to_select: bool) {
//     match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             let note_expression_type = state.note_expression_type().clone();
//             let note_expression_note_id = state.note_expression_id();
//             let automation_view_mode = {
//                 match state.automation_view_mode() {
//                     AutomationViewMode::NoteVelocities => AutomationViewMode::NoteVelocities,
//                     AutomationViewMode::Controllers => AutomationViewMode::Controllers,
//                     AutomationViewMode::PitchBend => AutomationViewMode::PitchBend,
//                     AutomationViewMode::Instrument => AutomationViewMode::Instrument,
//                     AutomationViewMode::Effect => AutomationViewMode::Effect,
//                     AutomationViewMode::NoteExpression => AutomationViewMode::NoteExpression,
//                 }
//             };
//             let automation_type = state.automation_type();
//             let mut state = state;
//             let track_uuid = state.selected_track();
//             let selected_riff_uuid = if let Some(track_uuid) = track_uuid.clone() {
//                 state.selected_riff_uuid(track_uuid)
//             } else {
//                 None
//             };
//             let selected_effect_plugin_uuid = if let Some(uuid) = state.selected_effect_plugin_uuid() {
//                 uuid.clone()
//             } else {
//                 "".to_string()
//             };
//             let current_view = state.current_view().clone();
//             let automation_edit_type = state.automation_edit_type();
//             let song = project.song();
//             let tracks = song.tracks();
//
//             let mut selected = Vec::new();
//
//             match track_uuid {
//                 Some(track_uuid) =>
//                     {
//                         match tracks.iter().find(|track| track.uuid().to_string() == track_uuid) {
//                             Some(track_type) => {
//                                 let events = if let AutomationViewMode::NoteVelocities = automation_view_mode {
//                                     if let Some(selected_riff_uuid) = selected_riff_uuid {
//                                         if let Some(riff) = track_type.riffs().iter().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
//                                             Some(riff.events_vec())
//                                         } else { None }
//                                     } else { None }
//                                 } else if let CurrentView::RiffArrangement = current_view {
//                                     let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
//                                         Some(selected_arrangement_uuid.clone())
//                                     } else { None };
//
//                                     // get the arrangement
//                                     if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
//                                         if let Some(riff_arrangement) = project.song().riff_arrangement(selected_arrangement_uuid.clone()) {
//                                             if let Some(automation) = riff_arrangement.automation(&track_uuid) {
//                                                 if state.automation_discrete() {
//                                                     Some(automation.events())
//                                                 }
//                                                 else {
//                                                     if let Some(automation_type_value) = automation_type {
//                                                         if let Some(automation_envelope) = automation.envelopes().iter().find(|envelope| {
//                                                             let mut found = false;
//
//                                                             // need to know what kind of events we are looking for in order to get the appropriate envelope
//                                                             match automation_view_mode {
//                                                                 AutomationViewMode::NoteVelocities => {}
//                                                                 AutomationViewMode::Controllers => {
//                                                                     if let TrackEvent::Controller(controller) = envelope.event_details() {
//                                                                         if controller.controller() == automation_type_value {
//                                                                             found = true;
//                                                                         }
//                                                                     }
//                                                                 }
//                                                                 AutomationViewMode::PitchBend => {
//                                                                     if let TrackEvent::PitchBend(_) = envelope.event_details() {
//                                                                         found = true;
//                                                                     }
//                                                                 }
//                                                                 AutomationViewMode::Instrument => {
//                                                                     let plugin_uuid = if let TrackType::InstrumentTrack(instrument_track) = track_type {
//                                                                         instrument_track.instrument().uuid().to_string()
//                                                                     } else {
//                                                                         "".to_string()
//                                                                     };
//                                                                     if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
//                                                                         if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid {
//                                                                             found = true;
//                                                                         }
//                                                                     }
//                                                                 }
//                                                                 AutomationViewMode::Effect => {
//                                                                     if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
//                                                                         if param.index == automation_type_value && param.plugin_uuid() == selected_effect_plugin_uuid {
//                                                                             found = true;
//                                                                         }
//                                                                     }
//                                                                 }
//                                                                 AutomationViewMode::NoteExpression => {
//                                                                     if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
//                                                                         if *note_expression.expression_type() as i32 == automation_type_value {
//                                                                             found = true;
//                                                                         }
//                                                                     }
//                                                                 }
//                                                             }
//                                                             return found;
//                                                         }) {
//                                                             Some(automation_envelope.events())
//                                                         } else { None }
//                                                     }
//                                                     else { None }
//                                                 }
//                                             } else { None }
//                                         } else { None }
//                                     } else { None }
//                                 } else {
//                                     match automation_edit_type {
//                                         AutomationEditType::Track => {
//                                             let automation = track_type.automation();
//                                             if state.automation_discrete() {
//                                                 Some(automation.events())
//                                             }
//                                             else {
//                                                 if let Some(automation_type_value) = automation_type {
//                                                     if let Some(automation_envelope) = automation.envelopes().iter().find(|envelope| {
//                                                         let mut found = false;
//
//                                                         // need to know what kind of events we are looking for in order to get the appropriate envelope
//                                                         match automation_view_mode {
//                                                             AutomationViewMode::NoteVelocities => {}
//                                                             AutomationViewMode::Controllers => {
//                                                                 if let TrackEvent::Controller(controller) = envelope.event_details() {
//                                                                     if controller.controller() == automation_type_value {
//                                                                         found = true;
//                                                                     }
//                                                                 }
//                                                             }
//                                                             AutomationViewMode::PitchBend => {
//                                                                 if let TrackEvent::PitchBend(_) = envelope.event_details() {
//                                                                     found = true;
//                                                                 }
//                                                             }
//                                                             AutomationViewMode::Instrument => {
//                                                                 let plugin_uuid = if let TrackType::InstrumentTrack(instrument_track) = track_type {
//                                                                     instrument_track.instrument().uuid().to_string()
//                                                                 } else {
//                                                                     "".to_string()
//                                                                 };
//                                                                 if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
//                                                                     if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid {
//                                                                         found = true;
//                                                                     }
//                                                                 }
//                                                             }
//                                                             AutomationViewMode::Effect => {
//                                                                 if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
//                                                                     if param.index == automation_type_value && param.plugin_uuid() == selected_effect_plugin_uuid {
//                                                                         found = true;
//                                                                     }
//                                                                 }
//                                                             }
//                                                             AutomationViewMode::NoteExpression => {
//                                                                 if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
//                                                                     if *note_expression.expression_type() as i32 == automation_type_value {
//                                                                         found = true;
//                                                                     }
//                                                                 }
//                                                             }
//                                                         }
//                                                         return found;
//                                                     }) {
//                                                         Some(automation_envelope.events())
//                                                     } else { None }
//                                                 }
//                                                 else { None }
//                                             }
//                                         }
//                                         AutomationEditType::Riff => {
//                                             if let Some(selected_riff_uuid) = selected_riff_uuid {
//                                                 if let Some(riff) = track_type.riffs().iter().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
//                                                     Some(riff.events_vec())
//                                                 } else { None }
//                                             } else { None }
//                                         }
//                                     }
//                                 };
//
//                                 if let Some(events) = events {
//                                     match automation_view_mode {
//                                         AutomationViewMode::NoteVelocities => {
//                                             for event in events.iter() {
//                                                 match event {
//                                                     TrackEvent::Note(note) => {
//                                                         let position = note.position();
//                                                         if time_lower <= position && (position + note.length()) <= time_higher
//                                                             && value_lower <= note.velocity() && note.velocity() <= value_higher {
//                                                             selected.push(note.id());
//                                                         }
//                                                     },
//                                                     _ => {},
//                                                 }
//                                             }
//                                         }
//                                         AutomationViewMode::Controllers => {
//                                             if let Some(automation_type_value) = automation_type {
//                                                 events.iter().for_each(|event| {
//                                                     match event {
//                                                         TrackEvent::Controller(controller) => {
//                                                             let position = controller.position();
//                                                             if controller.controller() == automation_type_value &&
//                                                                 time_lower <= position && position <= time_higher
//                                                                 && value_lower <= controller.value() && controller.value() <= value_higher {
//                                                                 selected.push(controller.id());
//                                                             }
//                                                         },
//                                                         _ => (),
//                                                     }
//                                                 })
//                                             }
//                                         }
//                                         AutomationViewMode::PitchBend => {
//                                             let value_lower = (value_lower as f32 / 127.0 * 16384.0 - 8192.0) as i32;
//                                             let value_higher = (value_higher as f32 / 127.0 * 16384.0 - 8192.0) as i32;
//                                             events.iter().for_each(|event| {
//                                                 match event {
//                                                     TrackEvent::PitchBend(pitch_bend) => {
//                                                         let position = pitch_bend.position();
//                                                         if time_lower <= position && position <= time_higher
//                                                             && value_lower <= pitch_bend.value() && pitch_bend.value() <= value_higher {
//                                                             selected.push(pitch_bend.id());
//                                                         }
//                                                     }
//                                                     _ => (),
//                                                 }
//                                             })
//                                         }
//                                         AutomationViewMode::Instrument => {
//                                             if let Some(automation_type_value) = automation_type {
//                                                 // get the instrument plugin uuid
//                                                 let instrument_plugin_id = if let TrackType::InstrumentTrack(instrument_track) = track_type {
//                                                     instrument_track.instrument().uuid()
//                                                 } else {
//                                                     return;
//                                                 };
//
//                                                 events.iter().for_each(|event| {
//                                                     match event {
//                                                         TrackEvent::AudioPluginParameter(plugin_param) => {
//                                                             let position = plugin_param.position();
//                                                             if plugin_param.index == automation_type_value &&
//                                                                 plugin_param.plugin_uuid.to_string() == instrument_plugin_id.to_string() &&
//                                                                 time_lower <= position &&
//                                                                 position <= time_higher {
//                                                                 selected.push(plugin_param.id());
//                                                             }
//                                                         },
//                                                         _ => (),
//                                                     }
//                                                 })
//                                             }
//                                         }
//                                         AutomationViewMode::Effect => {
//                                             if let Some(automation_type_value) = automation_type {
//                                                 events.iter().for_each(|event| {
//                                                     match event {
//                                                         TrackEvent::AudioPluginParameter(plugin_param) => {
//                                                             let position = plugin_param.position();
//                                                             if plugin_param.index == automation_type_value &&
//                                                                 plugin_param.plugin_uuid.to_string() == selected_effect_plugin_uuid &&
//                                                                 time_lower <= position &&
//                                                                 position <= time_higher {
//                                                                 selected.push(plugin_param.id());
//                                                             }
//                                                         },
//                                                         _ => (),
//                                                     }
//                                                 })
//                                             }
//                                         }
//                                         AutomationViewMode::NoteExpression => {
//                                             events.iter().for_each(|event| {
//                                                 match event {
//                                                     TrackEvent::NoteExpression(note_expression) => {
//                                                         let position = note_expression.position();
//                                                         if time_lower <= position &&
//                                                             position <= time_higher &&
//                                                             note_expression_type as i32 == *(note_expression.expression_type()) as i32 &&
//                                                             note_expression_note_id == note_expression.note_id() {
//                                                             selected.push(note_expression.id());
//                                                         }
//                                                     }
//                                                     _ => (),
//                                                 }
//                                             })
//                                         }
//                                     }
//                                 }
//                             },
//                             None => ()
//                         }
//                     },
//                 None => debug!("Main - rx_ui processing loop - AutomationSelectMultiple - problem getting selected track number"),
//             }
//
//             let mut state = state;
//             if !add_to_select {
//                 state.selected_automation_mut().clear();
//             }
//
//             if !selected.is_empty() {
//                 state.selected_automation_mut().append(&mut selected);
//             }
//         },
//         Err(_) => debug!("Main - rx_ui processing loop - AutomationSelectMultiple - could not get lock on state"),
//     };
//     // gui.ui.track_drawing_area.queue_draw();
//     // gui.ui.automation_drawing_area.queue_draw();
// }
//
// pub fn track_change_type_AutomationDeselectMultiple(state: &mut RiffDAWState, time_lower: f64, value_lower: i32, time_higher: f64, value_higher: i32) {
//     match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             let note_expression_type = state.note_expression_type().clone();
//             let note_expression_note_id = state.note_expression_id();
//             let automation_view_mode = {
//                 match state.automation_view_mode() {
//                     AutomationViewMode::NoteVelocities => AutomationViewMode::NoteVelocities,
//                     AutomationViewMode::Controllers => AutomationViewMode::Controllers,
//                     AutomationViewMode::PitchBend => AutomationViewMode::PitchBend,
//                     AutomationViewMode::Instrument => AutomationViewMode::Instrument,
//                     AutomationViewMode::Effect => AutomationViewMode::Effect,
//                     AutomationViewMode::NoteExpression => AutomationViewMode::NoteExpression,
//                 }
//             };
//             let automation_type = state.automation_type();
//             let mut state = state;
//             let track_uuid = state.selected_track();
//             let selected_riff_uuid = if let Some(track_uuid) = track_uuid.clone() {
//                 state.selected_riff_uuid(track_uuid)
//             } else {
//                 None
//             };
//             let selected_effect_plugin_uuid = if let Some(uuid) = state.selected_effect_plugin_uuid() {
//                 uuid.clone()
//             } else {
//                 "".to_string()
//             };
//             let current_view = state.current_view().clone();
//             let automation_edit_type = state.automation_edit_type();
//             let song = project.song();
//             let tracks = song.tracks();
//
//             let mut selected = Vec::new();
//
//             match track_uuid {
//                 Some(track_uuid) =>
//                     {
//                         match tracks.iter().find(|track| track.uuid().to_string() == track_uuid) {
//                             Some(track_type) => {
//                                 let events = if let AutomationViewMode::NoteVelocities = automation_view_mode {
//                                     if let Some(selected_riff_uuid) = selected_riff_uuid {
//                                         if let Some(riff) = track_type.riffs().iter().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
//                                             Some(riff.events_vec())
//                                         } else {
//                                             None
//                                         }
//                                     } else {
//                                         None
//                                     }
//                                 } else if let CurrentView::RiffArrangement = current_view {
//                                     let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
//                                         Some(selected_arrangement_uuid.clone())
//                                     } else {
//                                         None
//                                     };
//
//                                     // get the arrangement
//                                     if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
//                                         if let Some(riff_arrangement) = project.song().riff_arrangement(selected_arrangement_uuid.clone()) {
//                                             if let Some(riff_arr_automation) = riff_arrangement.automation(&track_uuid) {
//                                                 Some(riff_arr_automation.events())
//                                             } else {
//                                                 None
//                                             }
//                                         } else {
//                                             None
//                                         }
//                                     } else {
//                                         None
//                                     }
//                                 } else {
//                                     match automation_edit_type {
//                                         AutomationEditType::Track => {
//                                             Some(track_type.automation().events())
//                                         }
//                                         AutomationEditType::Riff => {
//                                             if let Some(selected_riff_uuid) = selected_riff_uuid {
//                                                 if let Some(riff) = track_type.riffs().iter().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
//                                                     Some(riff.events_vec())
//                                                 } else {
//                                                     None
//                                                 }
//                                             } else {
//                                                 None
//                                             }
//                                         }
//                                     }
//                                 };
//
//                                 if let Some(events) = events {
//                                     match automation_view_mode {
//                                         AutomationViewMode::NoteVelocities => {
//                                             for event in events.iter() {
//                                                 match event {
//                                                     TrackEvent::Note(note) => {
//                                                         let position = note.position();
//                                                         if time_lower <= position && (position + note.length()) <= time_higher
//                                                             && value_lower <= note.velocity() && note.velocity() <= value_higher {
//                                                             selected.push(note.id());
//                                                         }
//                                                     },
//                                                     _ => {},
//                                                 }
//                                             }
//                                         }
//                                         AutomationViewMode::Controllers => {
//                                             if let Some(automation_type_value) = automation_type {
//                                                 events.iter().for_each(|event| {
//                                                     match event {
//                                                         TrackEvent::Controller(controller) => {
//                                                             let position = controller.position();
//                                                             if controller.controller() == automation_type_value &&
//                                                                 time_lower <= position && position <= time_higher
//                                                                 && value_lower <= controller.value() && controller.value() <= value_higher {
//                                                                 selected.push(controller.id());
//                                                             }
//                                                         },
//                                                         _ => (),
//                                                     }
//                                                 })
//                                             }
//                                         }
//                                         AutomationViewMode::PitchBend => {
//                                             let value_lower = (value_lower as f32 / 127.0 * 16384.0 - 8192.0) as i32;
//                                             let value_higher = (value_higher as f32 / 127.0 * 16384.0 - 8192.0) as i32;
//                                             events.iter().for_each(|event| {
//                                                 match event {
//                                                     TrackEvent::PitchBend(pitch_bend) => {
//                                                         let position = pitch_bend.position();
//                                                         if time_lower <= position && position <= time_higher
//                                                             && value_lower <= pitch_bend.value() && pitch_bend.value() <= value_higher {
//                                                             selected.push(pitch_bend.id());
//                                                         }
//                                                     }
//                                                     _ => (),
//                                                 }
//                                             })
//                                         }
//                                         AutomationViewMode::Instrument => {
//                                             if let Some(automation_type_value) = automation_type {
//                                                 // get the instrument plugin uuid
//                                                 let instrument_plugin_id = if let TrackType::InstrumentTrack(instrument_track) = track_type {
//                                                     instrument_track.instrument().uuid()
//                                                 } else {
//                                                     return;
//                                                 };
//
//                                                 events.iter().for_each(|event| {
//                                                     match event {
//                                                         TrackEvent::AudioPluginParameter(plugin_param) => {
//                                                             let position = plugin_param.position();
//                                                             if plugin_param.index == automation_type_value &&
//                                                                 plugin_param.plugin_uuid.to_string() == instrument_plugin_id.to_string() &&
//                                                                 time_lower <= position &&
//                                                                 position <= time_higher {
//                                                                 selected.push(plugin_param.id());
//                                                             }
//                                                         },
//                                                         _ => (),
//                                                     }
//                                                 })
//                                             }
//                                         }
//                                         AutomationViewMode::Effect => {
//                                             if let Some(automation_type_value) = automation_type {
//                                                 events.iter().for_each(|event| {
//                                                     match event {
//                                                         TrackEvent::AudioPluginParameter(plugin_param) => {
//                                                             let position = plugin_param.position();
//                                                             if plugin_param.index == automation_type_value &&
//                                                                 plugin_param.plugin_uuid.to_string() == selected_effect_plugin_uuid &&
//                                                                 time_lower <= position &&
//                                                                 position <= time_higher {
//                                                                 selected.push(plugin_param.id());
//                                                             }
//                                                         },
//                                                         _ => (),
//                                                     }
//                                                 })
//                                             }
//                                         }
//                                         AutomationViewMode::NoteExpression => {
//                                             events.iter().for_each(|event| {
//                                                 match event {
//                                                     TrackEvent::NoteExpression(note_expression) => {
//                                                         let position = note_expression.position();
//                                                         if time_lower <= position &&
//                                                             position <= time_higher &&
//                                                             note_expression_type as i32 == *(note_expression.expression_type()) as i32 &&
//                                                             note_expression_note_id == note_expression.note_id() {
//                                                             selected.push(note_expression.id());
//                                                         }
//                                                     }
//                                                     _ => (),
//                                                 }
//                                             })
//                                         }
//                                     }
//                                 }
//                             },
//                             None => ()
//                         }
//                     },
//                 None => debug!("Main - rx_ui processing loop - AutomationSelectMultiple - problem getting selected track number"),
//             }
//
//             let mut state = state;
//             if !selected.is_empty() {
//                 state.selected_automation_mut().retain(|automation_id| !selected.contains(automation_id));
//             }
//         },
//         Err(_) => debug!("Main - rx_ui processing loop - AutomationSelectMultiple - could not get lock on state"),
//     };
//     // gui.ui.track_drawing_area.queue_draw();
//     // gui.ui.automation_drawing_area.queue_draw();
// }
//
// pub fn track_change_type_AutomationSelectAll(state: &mut RiffDAWState) {
//     match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             let note_expression_type = state.note_expression_type().clone();
//             let note_expression_note_id = state.note_expression_id();
//             let note_expression_type = state.note_expression_type().clone();
//             let note_expression_port_index = state.note_expression_port_index() as i16;
//             let note_expression_channel = state.note_expression_channel() as i16;
//             let note_expression_key = state.note_expression_key();
//             let automation_view_mode = {
//                 match state.automation_view_mode() {
//                     AutomationViewMode::NoteVelocities => AutomationViewMode::NoteVelocities,
//                     AutomationViewMode::Controllers => AutomationViewMode::Controllers,
//                     AutomationViewMode::PitchBend => AutomationViewMode::PitchBend,
//                     AutomationViewMode::Instrument => AutomationViewMode::Instrument,
//                     AutomationViewMode::Effect => AutomationViewMode::Effect,
//                     AutomationViewMode::NoteExpression => AutomationViewMode::NoteExpression,
//                 }
//             };
//             let automation_type = state.automation_type();
//             let mut state = state;
//             let track_uuid = state.selected_track();
//             let selected_riff_uuid = if let Some(track_uuid) = track_uuid.clone() {
//                 state.selected_riff_uuid(track_uuid)
//             } else {
//                 None
//             };
//             let selected_effect_plugin_uuid = if let Some(uuid) = state.selected_effect_plugin_uuid() {
//                 uuid.clone()
//             } else {
//                 "".to_string()
//             };
//             let current_view = state.current_view().clone();
//             let automation_edit_type = state.automation_edit_type();
//             let song = project.song();
//             let tracks = song.tracks();
//             let automation_discrete = state.automation_discrete();
//             let mut selected = Vec::new();
//
//             match track_uuid {
//                 Some(track_uuid) =>
//                     {
//                         match tracks.iter().find(|track| track.uuid().to_string() == track_uuid) {
//                             Some(track_type) => {
//                                 let events = if let AutomationViewMode::NoteVelocities = automation_view_mode {
//                                     if let Some(selected_riff_uuid) = selected_riff_uuid {
//                                         if let Some(riff) = track_type.riffs().iter().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
//                                             Some(riff.events_vec())
//                                         } else {
//                                             None
//                                         }
//                                     } else {
//                                         None
//                                     }
//                                 } else if let CurrentView::RiffArrangement = current_view {
//                                     let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
//                                         Some(selected_arrangement_uuid.clone())
//                                     } else {
//                                         None
//                                     };
//
//                                     // get the arrangement
//                                     if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
//                                         if let Some(riff_arrangement) = project.song().riff_arrangement(selected_arrangement_uuid.clone()) {
//                                             if let Some(automation) = riff_arrangement.automation(&track_uuid) {
//                                                 if automation_discrete {
//                                                     Some(automation.events())
//                                                 }
//                                                 else {
//                                                     // find the relevant envelope
//                                                     if let Some(automation_type_value) = automation_type {
//                                                         let instrument_plugin_uuid = if let TrackType::InstrumentTrack(track) = track_type {
//                                                             track.instrument().uuid().to_string()
//                                                         }
//                                                         else {
//                                                             "".to_string()
//                                                         };
//
//                                                         match automation_view_mode {
//                                                             AutomationViewMode::NoteVelocities => {
//                                                                 let find_fn = |envelope: &&AutomationEnvelope| {
//                                                                     let mut found = false;
//                                                                     if let TrackEvent::Note(_) = envelope.event_details() {
//                                                                         found = true;
//                                                                     }
//                                                                     return found;
//                                                                 };
//                                                                 if let Some(automation_envelope) = automation.envelopes().iter().find(find_fn) {
//                                                                     Some(automation_envelope.events())
//                                                                 } else { None }
//                                                             }
//                                                             AutomationViewMode::Controllers => {
//                                                                 let find_fn = |envelope: &&AutomationEnvelope| {
//                                                                     let mut found = false;
//                                                                     if let TrackEvent::Controller(controller) = envelope.event_details() {
//                                                                         if controller.controller() == automation_type_value {
//                                                                             found = true;
//                                                                         }
//                                                                     }
//                                                                     return found;
//                                                                 };
//                                                                 if let Some(automation_envelope) = automation.envelopes().iter().find(find_fn) {
//                                                                     Some(automation_envelope.events())
//                                                                 } else { None }
//                                                             }
//                                                             AutomationViewMode::PitchBend => {
//                                                                 let find_fn = |envelope: &&AutomationEnvelope| {
//                                                                     let mut found = false;
//                                                                     if let TrackEvent::PitchBend(_) = envelope.event_details() {
//                                                                         found = true;
//                                                                     }
//                                                                     return found;
//                                                                 };
//                                                                 if let Some(automation_envelope) = automation.envelopes().iter().find(find_fn) {
//                                                                     Some(automation_envelope.events())
//                                                                 } else { None }
//                                                             }
//                                                             AutomationViewMode::Instrument => {
//                                                                 let find_fn = |envelope: &&AutomationEnvelope| {
//                                                                     let mut found = false;
//                                                                     if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
//                                                                         if param.index == automation_type_value && param.plugin_uuid() == instrument_plugin_uuid {
//                                                                             found = true;
//                                                                         }
//                                                                     }
//                                                                     return found;
//                                                                 };
//                                                                 if let Some(automation_envelope) = automation.envelopes().iter().find(find_fn) {
//                                                                     Some(automation_envelope.events())
//                                                                 } else { None }
//                                                             }
//                                                             AutomationViewMode::Effect => {
//                                                                 let find_fn = |envelope: &&AutomationEnvelope| {
//                                                                     let mut found = false;
//                                                                     if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
//                                                                         if param.index == automation_type_value && param.plugin_uuid() == selected_effect_plugin_uuid {
//                                                                             found = true;
//                                                                         }
//                                                                     }
//                                                                     return found;
//                                                                 };
//                                                                 if let Some(automation_envelope) = automation.envelopes().iter().find(find_fn) {
//                                                                     Some(automation_envelope.events())
//                                                                 } else { None }
//                                                             }
//                                                             AutomationViewMode::NoteExpression => {
//                                                                 let find_fn = |envelope: &&AutomationEnvelope| {
//                                                                     let mut found = false;
//                                                                     if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
//                                                                         if
//                                                                         *(note_expression.expression_type()) == note_expression_type &&
//                                                                             note_expression.port() == note_expression_port_index &&
//                                                                             note_expression.channel() == note_expression_channel &&
//                                                                             note_expression.note_id() == note_expression_note_id &&
//                                                                             note_expression.key() == note_expression_key
//                                                                         {
//                                                                             found = true;
//                                                                         }
//                                                                     }
//                                                                     return found;
//                                                                 };
//                                                                 if let Some(automation_envelope) = automation.envelopes().iter().find(find_fn) {
//                                                                     Some(automation_envelope.events())
//                                                                 } else { None }
//                                                             }
//                                                         }
//                                                     }
//                                                     else { None }
//                                                 }
//                                             } else {
//                                                 None
//                                             }
//                                         } else {
//                                             None
//                                         }
//                                     } else {
//                                         None
//                                     }
//                                 } else {
//                                     match automation_edit_type {
//                                         AutomationEditType::Track => {
//                                             Some(track_type.automation().events())
//                                         }
//                                         AutomationEditType::Riff => {
//                                             if let Some(selected_riff_uuid) = selected_riff_uuid {
//                                                 if let Some(riff) = track_type.riffs().iter().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
//                                                     Some(riff.events_vec())
//                                                 } else {
//                                                     None
//                                                 }
//                                             } else {
//                                                 None
//                                             }
//                                         }
//                                     }
//                                 };
//
//                                 if let Some(events) = events {
//                                     match automation_view_mode {
//                                         AutomationViewMode::NoteVelocities => {
//                                             for event in events.iter() {
//                                                 match event {
//                                                     TrackEvent::Note(note) => {
//                                                         selected.push(note.id());
//                                                     }
//                                                     _ => {}
//                                                 }
//                                             }
//                                         }
//                                         AutomationViewMode::Controllers => {
//                                             if let Some(automation_type_value) = automation_type {
//                                                 events.iter().for_each(|event| {
//                                                     match event {
//                                                         TrackEvent::Controller(controller) => {
//                                                             if controller.controller() == automation_type_value {
//                                                                 selected.push(controller.id());
//                                                             }
//                                                         }
//                                                         _ => (),
//                                                     }
//                                                 })
//                                             }
//                                         }
//                                         AutomationViewMode::PitchBend => {
//                                             events.iter().for_each(|event| {
//                                                 match event {
//                                                     TrackEvent::PitchBend(pitch_bend) => {
//                                                         selected.push(pitch_bend.id());
//                                                     }
//                                                     _ => (),
//                                                 }
//                                             })
//                                         }
//                                         AutomationViewMode::Instrument => {
//                                             if let Some(automation_type_value) = automation_type {
//                                                 // get the instrument plugin uuid
//                                                 let instrument_plugin_id = if let TrackType::InstrumentTrack(instrument_track) = track_type {
//                                                     instrument_track.instrument().uuid()
//                                                 } else {
//                                                     return;
//                                                 };
//
//                                                 events.iter().for_each(|event| {
//                                                     match event {
//                                                         TrackEvent::AudioPluginParameter(plugin_param) => {
//                                                             if plugin_param.index == automation_type_value &&
//                                                                 plugin_param.plugin_uuid.to_string() == instrument_plugin_id.to_string() {
//                                                                 selected.push(plugin_param.id());
//                                                             }
//                                                         }
//                                                         _ => (),
//                                                     }
//                                                 })
//                                             }
//                                         }
//                                         AutomationViewMode::Effect => {
//                                             if let Some(automation_type_value) = automation_type {
//                                                 events.iter().for_each(|event| {
//                                                     match event {
//                                                         TrackEvent::AudioPluginParameter(plugin_param) => {
//                                                             if plugin_param.index == automation_type_value &&
//                                                                 plugin_param.plugin_uuid.to_string() == selected_effect_plugin_uuid {
//                                                                 selected.push(plugin_param.id());
//                                                             }
//                                                         }
//                                                         _ => (),
//                                                     }
//                                                 })
//                                             }
//                                         }
//                                         AutomationViewMode::NoteExpression => {
//                                             events.iter().for_each(|event| {
//                                                 match event {
//                                                     TrackEvent::NoteExpression(note_expression) => {
//                                                         if note_expression_type as i32 == *(note_expression.expression_type()) as i32 &&
//                                                             note_expression_note_id == note_expression.note_id() {
//                                                             selected.push(note_expression.id());
//                                                         }
//                                                     }
//                                                     _ => (),
//                                                 }
//                                             })
//                                         }
//                                     }
//                                 }
//                             },
//                             None => ()
//                         }
//                     },
//                 None => debug!("Main - rx_ui processing loop - AutomationSelectAll - problem getting selected track number"),
//             }
//
//             let mut state = state;
//             state.selected_automation_mut().clear();
//
//             if !selected.is_empty() {
//                 state.selected_automation_mut().append(&mut selected);
//             }
//         },
//         Err(_) => debug!("Main - rx_ui processing loop - AutomationSelectAll - could not get lock on state"),
//     };
//     // gui.ui.track_drawing_area.queue_draw();
//     // gui.ui.automation_drawing_area.queue_draw();
// }
//
// pub fn track_change_type_AutomationDeselectAll(state: &mut RiffDAWState) {
//     match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             state.selected_automation_mut().clear();
//             // gui.ui.track_drawing_area.queue_draw();
//             // gui.ui.automation_drawing_area.queue_draw();
//         }
//         Err(_) => debug!("Main - rx_ui processing loop - AutomationSelectMultiple - could not get lock on state"),
//     }
// }
//
// pub fn track_change_type_AutomationAdd(state: &mut RiffDAWState, automation: Vec<(f64, i32)>) {
//     for automation_item in automation.iter() {
//         handle_automation_add(automation_item.0, automation_item.1, state);
//     }
//     // gui.ui.track_drawing_area.queue_draw();
//     // gui.ui.automation_drawing_area.queue_draw();
// }
//
// pub fn track_change_type_AutomationDelete(state: &mut RiffDAWState, time: f64) {
//     handle_automation_delete(time, state);
//     // gui.ui.track_drawing_area.queue_draw();
//     // gui.ui.automation_drawing_area.queue_draw();
// }
//
// pub fn track_change_type_AutomationTranslateSelected(state: &mut RiffDAWState, _translation_entity_type: TranslationEntityType, translate_direction: TranslateDirection) {
//     let mut snap_in_beats = 1.0;
//     match gui.automation_grid() {
//         Some(controller_grid) => match controller_grid.lock() {
//             Ok(grid) => snap_in_beats = grid.snap_position_in_beats(),
//             Err(_) => (),
//         },
//         None => (),
//     }
//     handle_automation_translate_selected(state, translate_direction, snap_in_beats);
//     // gui.ui.track_drawing_area.queue_draw();
//     // gui.ui.automation_drawing_area.queue_draw();
// }
//
// pub fn track_change_type_AutomationChange(state: &mut RiffDAWState, change: Vec<(TrackEvent, TrackEvent)>) {
//     debug!("pub fn track_change_type_AutomationChange");
//     handle_automation_change(state, change);
//     // gui.ui.track_drawing_area.queue_draw();
//     // gui.ui.automation_drawing_area.queue_draw();
// }
//
// pub fn track_change_type_AutomationQuantiseSelected(state: &mut RiffDAWState) {
//     let mut snap_in_beats = 1.0;
//     let mut quantise_strength = 1.0;
//     match gui.automation_grid() {
//         Some(grid) => match grid.lock() {
//             Ok(grid) => {
//                 snap_in_beats = grid.snap_position_in_beats();
//                 quantise_strength = grid.snap_strength();
//             }
//             Err(_) => (),
//         },
//         None => (),
//     }
//
//     handle_automation_quantise(&state, snap_in_beats, quantise_strength);
//     // gui.ui.track_drawing_area.queue_draw();
//     // gui.ui.automation_drawing_area.queue_draw();
// }
//
// pub fn track_change_type_AutomationCut(state: &mut RiffDAWState) {
//     let edit_cursor_time_in_beats = if let Some(grid) = gui.automation_grid() {
//         match grid.lock() {
//             Ok(grid) => grid.edit_cursor_time_in_beats(),
//             Err(_) => 0.0,
//         }
//     } else { 0.0 };
//     handle_automation_cut(&state, edit_cursor_time_in_beats);
//     // gui.ui.track_drawing_area.queue_draw();
//     // gui.ui.automation_drawing_area.queue_draw();
// }
//
// pub fn track_change_type_AutomationCopy(state: &mut RiffDAWState) {
//     let edit_cursor_time_in_beats = if let Some(grid) = gui.automation_grid() {
//         match grid.lock() {
//             Ok(grid) => grid.edit_cursor_time_in_beats(),
//             Err(_) => 0.0,
//         }
//     } else { 0.0 };
//     handle_automation_copy(&state, edit_cursor_time_in_beats);
//     // gui.ui.track_drawing_area.queue_draw();
//     // gui.ui.automation_drawing_area.queue_draw();
// }
//
// pub fn track_change_type_AutomationPaste(state: &mut RiffDAWState) {
//     let edit_cursor_time_in_beats = if let Some(grid) = gui.automation_grid() {
//         match grid.lock() {
//             Ok(grid) => grid.edit_cursor_time_in_beats(),
//             Err(_) => 0.0,
//         }
//     } else { 0.0 };
//     handle_automation_paste(&state, edit_cursor_time_in_beats);
//     // gui.ui.track_drawing_area.queue_draw();
//     // gui.ui.automation_drawing_area.queue_draw();
// }
//
// pub fn track_change_type_AutomationTypeChange(state: &mut RiffDAWState, automation_type: AutomationChangeData) {
//     match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             match automation_type {
//                 AutomationChangeData::ParameterType(automation_type) => state.set_automation_type(Some(automation_type)),
//                 AutomationChangeData::NoteExpression(note_expression_data) => {
//                     match note_expression_data {
//                         NoteExpressionData::NoteId(id) => state.set_note_expression_id(id),
//                         NoteExpressionData::PortIndex(port_index) => state.set_note_expression_port_index(port_index),
//                         NoteExpressionData::Channel(channel) => state.set_note_expression_channel(channel),
//                         NoteExpressionData::Key(key) => state.set_note_expression_key(key),
//                         NoteExpressionData::Type(exp_type) => state.set_note_expression_type(exp_type),
//                     }
//                 }
//             }
//         }
//         Err(_) => debug!("Main - rx_ui processing loop - automation type change - could not get lock on state"),
//     };
//     // gui.ui.track_drawing_area.queue_draw();
//     // gui.ui.automation_drawing_area.queue_draw();
// }
//
// pub fn track_change_type_EffectSelected(state: &mut RiffDAWState, effect_uuid: String) {
//     match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             state.set_selected_effect_plugin_uuid(Some(effect_uuid.clone()));
//             if let Some(uuid) = state.selected_track() {
//                 // gui.update_automation_effect_parameters_combo(&mut state, uuid, effect_uuid);
//             }
//         },
//         Err(_) => debug!("Main - rx_ui processing loop - automation view  effect change - could not get lock on state"),
//     };
//     // gui.ui.track_drawing_area.queue_draw();
//     // gui.ui.automation_drawing_area.queue_draw();
// }
//
// pub fn track_change_type_EffectToggleWindowVisibility(state: &mut RiffDAWState, effect_uuid: String, track_uuid: Option<String>) {
//     match track_uuid {
//         Some(track_uuid) => {
//             let mut xid = 0;
//             match state.get_project().lock().as_mut() {
//                 Ok(project) => {
//                     match project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
//                         Some(track_type) => {
//                             let track_name = track_type.name().to_string();
//                             match track_type {
//                                 TrackType::InstrumentTrack(track) => {
//                                     debug!("track name={}, # of effects={}", track.name(), track.effects().len());
//                                     for effect in track.effects_mut().iter_mut() {
//                                         debug!("effect name={}, effect uuid={}, search for effect uuid={}", effect.name(), effect.uuid(), effect_uuid.as_str());
//                                         if effect.uuid().to_string() == effect_uuid {
//                                             if let Some(window) = audio_plugin_windows.get(&effect_uuid) {
//                                                 if window.is_visible() {
//                                                     window.hide();
//                                                 } else {
//                                                     window.show_all();
//                                                 }
//                                             } else {
//                                                 let win = Window::new(WindowType::Toplevel);
//                                                 win.set_title(format!("Track: {} - Effect: {}", track_name, effect.name()).as_str());
//                                                 win.connect_delete_event(|window, _| {
//                                                     window.hide();
//                                                     gtk::Inhibit(true)
//                                                 });
//                                                 win.set_height_request(800);
//                                                 win.set_width_request(900);
//                                                 win.set_resizable(true);
//                                                 win.show_all();
//                                                 audio_plugin_windows.insert(effect_uuid.clone(), win.clone());
//
//                                                 let window = win.clone();
//                                                 {
//                                                     glib::idle_add_local(move || {
//                                                         if window.is_visible() {
//                                                             window.queue_draw();
//                                                         }
//                                                         glib::Continue(true)
//                                                     });
//                                                 }
//
//                                                 unsafe {
//                                                     match win.window() {
//                                                         Some(gdk_window) => {
//                                                             xid = gdk_x11_window_get_xid(gdk_window);
//                                                             debug!("xid: {}", xid);
//                                                         },
//                                                         None => debug!("Couldn't get gdk window."),
//                                                     }
//                                                 }
//                                             }
//
//                                             break;
//                                         }
//                                     }
//                                 },
//                                 TrackType::AudioTrack(_) => (),
//                                 TrackType::MidiTrack(_) => (),
//                             }
//                         },
//                         None => ()
//                     }
//                 },
//                 Err(_) => debug!("Main - rx_ui processing loop - track effect toggle window visibility - could not get lock on state"),
//             };
//             if xid != 0 {
//                 match state.get_project().lock().as_mut() {
//                     Ok(project) => {
//                         state.send_to_track_background_processor(track_uuid, TrackBackgroundProcessorInwardEvent::SetEffectWindowId(effect_uuid, xid));
//                     },
//                     Err(_) => debug!("Could not get read only lock on state."),
//                 }
//             }
//         },
//         None => (),
//     }
// }

pub fn track_change_type_Volume(state: &mut RiffDAWState, position: Option<f64>, volume: f32, track_uuid: Option<String>) {
    debug!("Received volume change: track={}, volume={}", track_uuid.clone().unwrap(), volume);
    if let Some(track_uuid) = track_uuid {
        match state.get_project().lock().as_mut() {
            Ok(project) => {
                let recording = *state.recording_mut();
                let playing = *state.playing_mut();
                let play_position_in_frames = state.play_position_in_frames() as f64;
                let sample_rate = state.configuration.audio.sample_rate as f64;
                let bpm = project.song_mut().tempo();
                let play_position_in_beats = play_position_in_frames / sample_rate * bpm / 60.0;
                let mut midi_channel = 0;

                for track in project.song_mut().tracks_mut().iter_mut() {
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

pub fn track_change_type_Pan(state: &mut RiffDAWState, position: Option<f64>, pan: f32, track_uuid: Option<String>) {
    debug!("Received pan change: track={}, pan={}", track_uuid.clone().unwrap(), pan);
    if let Some(track_uuid) = track_uuid {
        match state.get_project().lock().as_mut() {
            Ok(project) => {
                let recording = *state.recording_mut();
                let playing = *state.playing_mut();
                let play_position_in_frames = state.play_position_in_frames() as f64;
                let sample_rate = state.configuration.audio.sample_rate as f64;
                let bpm = project.song_mut().tempo();
                let play_position_in_beats = play_position_in_frames / sample_rate * bpm / 60.0;
                let mut midi_channel = 0;

                for track in project.song_mut().tracks_mut().iter_mut() {
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

// pub fn track_change_type_TrackColourChanged(state: &mut RiffDAWState, red: f64, green: f64, blue: f64, alpha: f64, track_uuid: Option<String>) {
//     if let Some(track_uuid) = track_uuid {
//         match state.get_project().lock().as_mut() {
//             Ok(project) => {
//                 for track in project.song_mut().tracks_mut().iter_mut() {
//                     if track.uuid().to_string() == track_uuid {
//                         track.set_colour(red, green, blue, alpha);
//                         // gui.ui.track_drawing_area.queue_draw();
//                         // gui.ui.piano_roll_drawing_area.queue_draw();
//                         // gui.ui.sample_roll_drawing_area.queue_draw();
//                         // gui.ui.automation_drawing_area.queue_draw();
//                         break;
//                     }
//                 }
//             },
//             Err(_) => debug!("Could not get read only lock on state."),
//         }
//     }
// }
//
// pub fn track_change_type_RiffColourChanged(state: &mut RiffDAWState, uuid: String, red: f64, green: f64, blue: f64, alpha: f64, track_uuid: Option<String>) {
//     if let Some(track_uuid) = track_uuid {
//         match state.get_project().lock().as_mut() {
//             Ok(project) => {
//                 for track in project.song_mut().tracks_mut().iter_mut() {
//                     if track.uuid().to_string() == track_uuid {
//                         // find the riff and update it
//                         for riff in track.riffs_mut().iter_mut() {
//                             if riff.uuid().to_string() == uuid {
//                                 riff.set_colour(Some((red, green, blue, alpha)));
//                                 break;
//                             }
//                         }
//                         // gui.ui.track_drawing_area.queue_draw();
//                         // gui.ui.piano_roll_drawing_area.queue_draw();
//                         // gui.ui.sample_roll_drawing_area.queue_draw();
//                         // gui.ui.automation_drawing_area.queue_draw();
//                         break;
//                     }
//                 }
//             },
//             Err(_) => debug!("Could not get read only lock on state."),
//         }
//     }
// }
//
// pub fn track_change_type_CopyTrack(state: &mut RiffDAWState, track_uuid: Option<String>) {
//     let state_arc = state.clone();
//     match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             // find the track to copy
//             if let Some(track_uuid) = track_uuid {
//                 if let Some(track_type) = project.song().tracks().iter().find(|track_type| track_type.uuid().to_string() == track_uuid) {
//                     let mut new_track = InstrumentTrack::new();
//                     let mut instrument_track_senders2 = HashMap::new();
//                     let mut instrument_track_receivers2 = HashMap::new();
//                     let sample_rate = state.configuration.audio.sample_rate as f64;;
//                     let block_size = state.configuration.audio.block_size as f64;;
//                     let tempo = project.song().tempo();
//                     let time_signature_numerator = project.song().time_signature_numerator();
//                     let time_signature_denominator = project.song().time_signature_denominator();
//
//                     // copy what is needed from the originating track - a bit difficult to make this clone-able
//                     let mut new_name = "Copy of ".to_string();
//                     new_name.push_str(track_type.name());
//                     new_track.set_name(new_name);
//                     for riff in track_type.riffs().iter() {
//                         if riff.name() != "empty" {
//                             let mut new_riff = riff.clone();
//                             new_riff.set_uuid(Uuid::new_v4());
//                             new_track.riffs_mut().push(new_riff);
//                         }
//                     }
//
//                     // need to copy the instrument and effect details
//
//                     let mut new_track_type = TrackType::InstrumentTrack(new_track);
//
//
//                     state.init_track(
//                         &mut new_track_type,
//                         None,
//                         None,
//                         sample_rate,
//                         block_size,
//                         tempo,
//                         time_signature_numerator as i32,
//                         time_signature_denominator as i32,
//                     );
//
//                     project.song_mut().tracks_mut().push(new_track_type);
//                     state.update_track_senders_and_receivers(instrument_track_senders2, instrument_track_receivers2);
//
//                     // gui.clear_ui();
//                     // gui.update_ui_from_state(tx_from_ui.clone(), &mut state, state_arc.clone());
//                 }
//             }
//         },
//         Err(_) => todo!(),
//     }
// }
//
// pub fn track_change_type_RouteMidiTo(state: &mut RiffDAWState, routing: TrackEventRouting, track_uuid: Option<String>) {
//     match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             if let Some(track_from_uuid) = track_uuid {
//                 state.send_midi_routing_to_track_background_processors(track_from_uuid.clone(), routing.clone());
//
//                 // add the new routing to the track
//                 if let Some(track) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_from_uuid) {
//                     track.midi_routings_mut().push(routing);
//                 }
//             }
//         }
//         Err(error) => {
//             debug!("Problem locking state when routing midi to a track: {}", error);
//         }
//     }
// }
//
// pub fn track_change_type_RemoveMidiRouting(state: &mut RiffDAWState, route_uuid: String, track_uuid: Option<String>) {
//     match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             if let Some(track_from_uuid) = track_uuid {
//                 // get the destination track uuid
//                 let destination_track_uuid = if let Some(track) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_from_uuid.clone()) {
//                     'splashdown: {
//                         for index in 0..track.midi_routings().len() {
//                             if let Some(route) = track.midi_routings().get(index) {
//                                 if route.uuid() == route_uuid {
//                                     // extract the track uuid from the destination part of the route
//                                     let destination_track_uuid = match &route.destination {
//                                         TrackEventRoutingNodeType::Track(track_uuid) => track_uuid.clone(),
//                                         TrackEventRoutingNodeType::Instrument(track_uuid, _) => track_uuid.clone(),
//                                         TrackEventRoutingNodeType::Effect(track_uuid, _) => track_uuid.clone(),
//                                     };
//
//                                     track.midi_routings_mut().remove(index);
//                                     break 'splashdown Some(destination_track_uuid);
//                                 }
//                             }
//                         }
//                         None
//                     }
//                 } else {
//                     None
//                 };
//
//                 // delete the routing from the source track background processor
//                 state.send_to_track_background_processor(track_from_uuid.clone(), TrackBackgroundProcessorInwardEvent::RemoveTrackEventSendRouting(route_uuid.clone()));
//
//                 // delete the routing from the destination track
//                 if let Some(destination_track_uuid) = destination_track_uuid {
//                     state.send_to_track_background_processor(destination_track_uuid, TrackBackgroundProcessorInwardEvent::RemoveTrackEventReceiveRouting(route_uuid.clone()));
//                 }
//             }
//         }
//         Err(error) => {
//             debug!("Problem locking state when routing midi to a track: {}", error);
//         }
//     }
// }
//
// pub fn track_change_type_UpdateMidiRouting(state: &mut RiffDAWState, route_uuid: String, midi_channel: i32, start_note: i32, end_note: i32, track_uuid: Option<String>) {
//     match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             if let Some(track_from_uuid) = track_uuid {
//                 // get the destination track uuid
//                 let details = if let Some(track) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_from_uuid.clone()) {
//                     'splashdown: {
//                         for index in 0..track.midi_routings().len() {
//                             if let Some(route) = track.midi_routings_mut().get_mut(index) {
//                                 if route.uuid() == route_uuid {
//                                     // extract the track uuid from the destination part of the route
//                                     let destination_track_uuid = match &route.destination {
//                                         TrackEventRoutingNodeType::Track(track_uuid) => track_uuid.clone(),
//                                         TrackEventRoutingNodeType::Instrument(track_uuid, _) => track_uuid.clone(),
//                                         TrackEventRoutingNodeType::Effect(track_uuid, _) => track_uuid.clone(),
//                                     };
//
//                                     route.channel = midi_channel as u8;
//                                     route.note_range = (start_note as u8, end_note as u8);
//
//                                     break 'splashdown Some((route.clone(), destination_track_uuid));
//                                 }
//                             }
//                         }
//                         None
//                     }
//                 } else {
//                     None
//                 };
//
//                 if let Some((route, destination_track_uuid)) = details {
//                     // delete the routing from the source track background processor
//                     state.send_to_track_background_processor(track_from_uuid.clone(), TrackBackgroundProcessorInwardEvent::UpdateTrackEventSendRouting(route_uuid.clone(), route.clone()));
//
//                     // delete the routing from the destination track
//                     state.send_to_track_background_processor(destination_track_uuid, TrackBackgroundProcessorInwardEvent::UpdateTrackEventReceiveRouting(route_uuid.clone(), route));
//                 }
//             }
//         }
//         Err(error) => {
//             debug!("Problem locking state when routing midi to a track: {}", error);
//         }
//     }
// }
//
// pub fn track_change_type_RouteAudioTo(state: &mut RiffDAWState, routing: AudioRouting, track_uuid: Option<String>) {
//     match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             if let Some(track_from_uuid) = track_uuid {
//                 state.send_audio_routing_to_track_background_processors(track_from_uuid.clone(), routing.clone());
//
//                 // add the new routing to the track
//                 if let Some(track) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_from_uuid) {
//                     track.audio_routings_mut().push(routing);
//                 }
//             }
//         }
//         Err(error) => {
//             debug!("Problem locking state when routing audio to a track: {}", error);
//         }
//     }
// }
//
// pub fn track_change_type_RemoveAudioRouting(state: &mut RiffDAWState, route_uuid: String, track_uuid: Option<String>) {
//     match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             if let Some(track_from_uuid) = track_uuid {
//                 // get the destination track uuid
//                 let destination_track_uuid = if let Some(track) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_from_uuid.clone()) {
//                     'splashdown: {
//                         for index in 0..track.audio_routings().len() {
//                             if let Some(route) = track.audio_routings().get(index) {
//                                 if route.uuid() == route_uuid {
//                                     // extract the track uuid from the destination part of the route
//                                     let destination_track_uuid = match &route.destination {
//                                         AudioRoutingNodeType::Track(track_uuid) => track_uuid.clone(),
//                                         AudioRoutingNodeType::Instrument(track_uuid, _, _, _) => track_uuid.clone(),
//                                         AudioRoutingNodeType::Effect(track_uuid, _, _, _) => track_uuid.clone(),
//                                     };
//
//                                     track.audio_routings_mut().remove(index);
//                                     break 'splashdown Some(destination_track_uuid);
//                                 }
//                             }
//                         }
//                         None
//                     }
//                 } else {
//                     None
//                 };
//
//                 // delete the routing from the source track background processor
//                 state.send_to_track_background_processor(track_from_uuid.clone(), TrackBackgroundProcessorInwardEvent::RemoveAudioSendRouting(route_uuid.clone()));
//
//                 // delete the routing from the destination track
//                 if let Some(destination_track_uuid) = destination_track_uuid {
//                     state.send_to_track_background_processor(destination_track_uuid, TrackBackgroundProcessorInwardEvent::RemoveAudioReceiveRouting(route_uuid.clone()));
//                 }
//             }
//         }
//         Err(error) => {
//             debug!("Problem locking state when routing audio to a track: {}", error);
//         }
//     }
// }
//
// pub fn track_change_type_TrackMoveToPosition(state: &mut RiffDAWState, move_to_position: usize, track_uuid: Option<String>) {
//     debug!("Main - rx_ui processing loop - track move to position");
//     if let Some(track_uuid) = track_uuid {
//         let state_arc = state.clone();
//         match state.get_project().lock().as_mut() {
//             Ok(project) => {
//                 project.song_mut().track_move_to_position(track_uuid, move_to_position);
//                 // gui.clear_ui();
//                 // gui.update_ui_from_state(tx_from_ui.clone(), &mut state, state_arc);
//             },
//             Err(_) => debug!("Main - rx_ui processing loop - track move to position - could not get lock on state"),
//         };
//         // gui.ui.riff_sets_box.queue_draw();
//     }
// }
//
// pub fn track_change_type_RiffEventChange(state: &mut RiffDAWState, change: Vec<(TrackEvent, TrackEvent)>) {
//     let mut selected_riff_uuid = None;
//     let mut selected_riff_track_uuid = None;
//     match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             selected_riff_track_uuid = state.selected_track();
//
//             match selected_riff_track_uuid {
//                 Some(track_uuid) => {
//                     selected_riff_uuid = state.selected_riff_uuid(track_uuid.clone());
//                     selected_riff_track_uuid = Some(track_uuid);
//                 },
//                 None => (),
//             }
//         },
//         Err(_) => debug!("Main - rx_ui processing loop - riff translate event - could not get lock on state"),
//     }
//     if let Some(selected_riff_uuid) = selected_riff_uuid {
//         if let Some(selected_riff_track_uuid) = selected_riff_track_uuid {
//             match state.get_project().lock().as_mut() {
//                 Ok(project) => {
//                     for (original_event_copy, changed_event) in change.iter() {
//                         match original_event_copy {
//                             TrackEvent::Note(original_note_copy) => {
//                                 if let Some(track) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == selected_riff_track_uuid) {
//                                     if let Some(riff) = track.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == selected_riff_uuid) {
//                                         for event in riff.events_mut().iter_mut() {
//                                             if let TrackEvent::Note(mut note) = event {
//                                                 if *note == *original_note_copy {
//                                                     if let TrackEvent::Note(translated_event_copy) = changed_event {
//                                                         note.set_position(translated_event_copy.position());
//                                                         note.set_note(translated_event_copy.note());
//                                                         note.set_length(translated_event_copy.length());
//                                                         break;
//                                                     }
//                                                 }
//                                             }
//                                         }
//                                     }
//                                 }
//                             }
//                             _ => {}
//                         }
//                     }
//                 }
//                 Err(_error) => debug!("Main - rx_ui processing loop - riff translate event - could not get lock on state"),
//             }
//         }
//     }
//     // gui.ui.piano_roll_drawing_area.queue_draw();
// }
//
// pub fn track_change_type_RiffReferenceChange(state: &mut RiffDAWState, mut change: Vec<(Riff, Riff)>) {
//     match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             let mut snap_position_in_beats = 1.0;
//             match gui.riff_grid() {
//                 Some(riff_grid) => match riff_grid.lock() {
//                     Ok(grid) => {
//                         snap_position_in_beats = grid.snap_position_in_beats();
//                     }
//                     Err(_) => (),
//                 },
//                 None => (),
//             }
//
//             for track in project.song_mut().tracks_mut().iter_mut() {
//                 let mut unused_changes = vec![];
//                 for (original_riff, changed_riff) in change.iter() {
//                     let mut used = false;
//                     let mut riff_id = "".to_string();
//
//                     if let Some(riff_ref) = track.riff_refs_mut().iter_mut().find(|riff_ref| riff_ref.uuid().to_string() == changed_riff.uuid().to_string()) {
//                         let delta = riff_ref.position() - changed_riff.position();
//
//                         riff_id = riff_ref.linked_to();
//
//                         if delta < -0.000001 || delta > 0.000001 {
//                             let calculated_value = DAWUtils::quantise(changed_riff.position(), snap_position_in_beats, 1.0, false);
//                             if calculated_value.snapped {
//                                 riff_ref.set_position(calculated_value.snapped_value);
//                             }
//                         }
//                         used = true;
//                     }
//
//                     if let Some(riff) = track.riffs_mut().iter_mut().find(|riff| riff.id() == riff_id) {
//                         let delta = riff.length() - changed_riff.length();
//                         if delta < -0.000001 || delta > 0.000001 {
//                             let calculated_value = DAWUtils::quantise(changed_riff.length(), snap_position_in_beats, 1.0, true);
//                             if calculated_value.snapped {
//                                 riff.set_length(calculated_value.snapped_value);
//                             }
//                         }
//                         used = true;
//                     }
//
//                     if !used {
//                         unused_changes.push((original_riff.clone(), changed_riff.clone()));
//                     }
//                 }
//                 change.clear();
//                 change.append(&mut unused_changes);
//             }
//             // gui.ui.track_drawing_area.queue_draw();
//         }
//         Err(_error) => debug!("Main - rx_ui processing loop - riff reference change - could not get lock on state"),
//     }
// }
//
// pub fn track_change_type_TrackDetails(state: &mut RiffDAWState, show: bool, track_uuid: Option<String>) {
//     if let Some(track_uuid) = track_uuid {
//         match state.get_project().lock().as_mut() {
//             Ok(project) => {
//                 state.set_selected_track(Some(track_uuid.clone()));
//
//                 // TODO this needs to be shown in the UI
//             }
//             Err(_error) => debug!("Main - rx_ui processing loop - track details - could not get lock on state"),
//         }
//         if let Some((_, dialogue)) = gui.track_details_dialogues.iter().find(|(dialogue_track_uuid, _dialogue)| dialogue_track_uuid.to_string() == track_uuid) {
//             if show {
//                 dialogue.track_details_dialogue.show_all();
//             } else {
//                 dialogue.track_details_dialogue.hide();
//             }
//         }
//     }
// }
//
// pub fn track_change_type_UpdateTrackDetails(state: &mut RiffDAWState, track_uuid: Option<String>) {
//     if let Some(track_uuid) = track_uuid {
//         match state.get_project().lock().as_mut() {
//             Ok(project) => {
//                 let midi_input_devices: Vec<String> = state.midi_devices();
//
//                 let mut instrument_plugins: IndexMap<String, String> = IndexMap::new();
//                 let instrument_keys = state.configuration.scanned_instrument_plugins.successfully_scanned.iter().sorted_by(|(_key1, value1), (_key2, value2)| value1.cmp(value2)).map(|(key, value)| key).collect_vec();
//                 for key in instrument_keys.iter() {
//                     if let Some(value) = state.configuration.scanned_instrument_plugins.successfully_scanned.get(*key) {
//                         let adjusted_key = key.replace(char::from(0), "");
//                         let adjusted_value = value.replace(char::from(0), "");
//                         instrument_plugins.insert(adjusted_key, adjusted_value);
//                     }
//                 }
//
//                 let mut effect_plugins: IndexMap<String, String> = IndexMap::new();
//                 let effect_keys = state.configuration.scanned_effect_plugins.successfully_scanned.iter().sorted_by(|(_key1, value1), (_key2, value2)| value1.cmp(value2)).map(|(key, value)| key).collect_vec();
//                 for key in effect_keys.iter() {
//                     if let Some(value) = state.configuration.scanned_effect_plugins.successfully_scanned.get(*key) {
//                         let adjusted_key = key.replace(char::from(0), "");
//                         let adjusted_value = value.replace(char::from(0), "");
//                         effect_plugins.insert(adjusted_key, adjusted_value);
//                     }
//                 }
//
//                 for (mut track_number, track) in project.song_mut().tracks_mut().iter_mut().enumerate() {
//                     if track.uuid().to_string() == track_uuid {
//                         let mut track_number = track_number as i32;
//                         // gui.update_track_details_dialogue(&midi_input_devices, &mut instrument_plugins, &mut effect_plugins, &mut track_number, &track);
//                         break;
//                     }
//                 }
//             }
//             Err(_error) => debug!("Main - rx_ui processing loop - update track details - could not get lock on state"),
//         }
//     }
// }
//
// pub fn track_change_type_RiffSetStartNote(state: &mut RiffDAWState, note_number: i32, position: f64) {
//     let mut selected_riff_uuid = None;
//     let mut selected_riff_track_uuid = None;
//
//     match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             selected_riff_track_uuid = state.selected_track();
//
//             match selected_riff_track_uuid {
//                 Some(track_uuid) => {
//                     selected_riff_uuid = state.selected_riff_uuid(track_uuid.clone());
//                     selected_riff_track_uuid = Some(track_uuid);
//                 },
//                 None => (),
//             }
//         },
//         Err(_) => debug!("Main - rx_ui processing loop - set riff start note - could not get lock on state"),
//     }
//
//     match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             let mut state = state;
//
//             match selected_riff_track_uuid {
//                 Some(track_uuid) => {
//                     for track in project.song_mut().tracks_mut().iter_mut() {
//                         if track.uuid().to_string() == track_uuid {
//                             match selected_riff_uuid {
//                                 Some(riff_uuid) => {
//                                     for riff in track.riffs_mut().iter_mut() {
//                                         if riff.uuid().to_string() == *riff_uuid {
//                                             // find the current start note
//                                             let current_start_note_details = if let Some(current_start_note) = riff.events_mut().iter_mut().find(|event| match event {
//                                                 TrackEvent::Note(note) => note.note() == note_number && note.position() <= position && position <= (note.position() + note.length()) && note.riff_start_note(),
//                                                 _ => false,
//                                             }) {
//                                                 if let TrackEvent::Note(note) = current_start_note {
//                                                     Some((note.note(), note.position(), note.length()))
//                                                 } else {
//                                                     None
//                                                 }
//                                             } else {
//                                                 None
//                                             };
//
//                                             // reset the previous start note
//                                             riff.events_mut().iter_mut().for_each(|event| {
//                                                 if let TrackEvent::Note(note) = event {
//                                                     note.set_riff_start_note(false);
//                                                 }
//                                             });
//                                             let note = riff.events_mut().iter_mut().find(|event| match event {
//                                                 TrackEvent::Note(note) => note.note() == note_number && note.position() <= position && position <= (note.position() + note.length()),
//                                                 _ => false,
//                                             });
//                                             if let Some(event) = note {
//                                                 match event {
//                                                     TrackEvent::Note(note) => {
//                                                         debug!("Set riff start note: position={}, note={}, velocity={}, duration={}", note.position(), note.note(), note.velocity(), note.length());
//                                                         if let Some((current_start_note_number, current_start_note_position, current_start_note_length)) = current_start_note_details {
//                                                             if note.note() != current_start_note_number || note.position() != current_start_note_position || note.length() != current_start_note_length {
//                                                                 note.set_riff_start_note(true);
//                                                             }
//                                                         } else {
//                                                             note.set_riff_start_note(true);
//                                                         }
//                                                     }
//                                                     _ => {}
//                                                 }
//                                             }
//                                             break;
//                                         }
//                                     }
//                                 }
//                                 None => debug!("problem getting selected riff index"),
//                             }
//
//                             break;
//                         }
//                     }
//                 },
//                 None => debug!("problem getting selected riff track number"),
//             };
//         },
//         Err(_) => debug!("Main - rx_ui processing loop - set riff start note - could not get lock on state"),
//     }
// }
//
// pub fn track_change_type_RiffReferencePlayMode(state: &mut RiffDAWState, track_number: i32, position: f64) {
//     // FIXME need to take into account the context - current view etc.
//     match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             let mut found = None;
//             match state.current_view().clone() {
//                 CurrentView::Track => {
//                     if let Some(track) = project.song_mut().tracks_mut().get_mut(track_number as usize) {
//                         for riff_ref in track.riff_refs().iter().filter(|riff_ref| riff_ref.position() <= position) {
//                             if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_ref.linked_to()) {
//                                 // position is inside the riff ref
//                                 if riff_ref.position() <= position && position <= (riff_ref.position() + riff.length()) {
//                                     if position <= (riff_ref.position() + 1.0) {
//                                         found = Some((riff_ref.uuid(), RiffReferenceMode::Start));
//                                         break;
//                                     } else if position >= (riff_ref.position() + riff.length() - 1.0) {
//                                         found = Some((riff_ref.uuid(), RiffReferenceMode::End));
//                                         break;
//                                     } else {
//                                         found = Some((riff_ref.uuid(), RiffReferenceMode::Normal));
//                                         break;
//                                     }
//                                 }
//                             }
//                         }
//
//                         if let Some((riff_ref_uuid, mode)) = found {
//                             if let Some(riff_ref) = track.riff_refs_mut().iter_mut().find(|riff_ref| riff_ref.uuid() == riff_ref_uuid) {
//                                 riff_ref.set_mode(mode);
//                             }
//                         }
//                     }
//                 }
//                 CurrentView::RiffSet => {
//                     debug!("*****************************No idea what to do with a riff set when setting a riff ref mode.");
//                 }
//                 CurrentView::RiffGrid => {
//                     let track_details = if let Some(track) = project.song().tracks().get(track_number as usize) {
//                         let mut riff_lengths = HashMap::new();
//                         for riff in track.riffs().iter() {
//                             riff_lengths.insert(riff.uuid().to_string(), riff.length());
//                         }
//                         Some((track.uuid().to_string(), riff_lengths))
//                     } else {
//                         None
//                     };
//                     if let Some((track_uuid, riff_lengths)) = track_details {
//                         if let Some(selected_riff_grid_uuid) = state.selected_riff_grid_uuid().clone() {
//                             if let Some(riff_grid) = project.song_mut().riff_grid_mut(selected_riff_grid_uuid.clone()) {
//                                 if let Some(track_riff_refs) = riff_grid.track_riff_references(track_uuid.clone()) {
//                                     for riff_ref in track_riff_refs.iter().filter(|riff_ref| riff_ref.position() <= position) {
//                                         if let Some(riff_length) = riff_lengths.get(&riff_ref.linked_to()) {
//                                             // position is inside the riff ref
//                                             if riff_ref.position() <= position && position <= (riff_ref.position() + riff_length) {
//                                                 if position <= (riff_ref.position() + 1.0) {
//                                                     found = Some((riff_ref.uuid(), RiffReferenceMode::Start));
//                                                     break;
//                                                 } else if position >= (riff_ref.position() + riff_length - 1.0) {
//                                                     found = Some((riff_ref.uuid(), RiffReferenceMode::End));
//                                                     break;
//                                                 } else {
//                                                     found = Some((riff_ref.uuid(), RiffReferenceMode::Normal));
//                                                     break;
//                                                 }
//                                             }
//                                         }
//                                     }
//
//                                     if let Some((riff_ref_uuid, mode)) = found {
//                                         if let Some(riff_refs) = riff_grid.track_riff_references_mut(track_uuid) {
//                                             if let Some(riff_ref) = riff_refs.iter_mut().find(|riff_ref| riff_ref.uuid() == riff_ref_uuid) {
//                                                 riff_ref.set_mode(mode);
//                                             }
//                                         }
//                                     }
//                                 }
//                             }
//                         }
//                     }
//                 }
//                 _ => {}
//             }
//         },
//         Err(_) => debug!("Main - rx_ui processing loop - set riff reference play mode - could not get lock on state"),
//     }
// }
//
// pub fn track_change_type_RiffReferenceDragCopy(state: &mut RiffDAWState, mut new_riff_references_details: Vec<(f64, String)>) {
//     match state.get_project().lock().as_mut() {
//         Ok(project) => {
//             let mut snap_position_in_beats = 1.0;
//             match gui.track_grid() {
//                 Some(track_grid) => match track_grid.lock() {
//                     Ok(grid) => snap_position_in_beats = grid.snap_position_in_beats(),
//                     Err(_) => (),
//                 },
//                 None => (),
//             }
//
//             for track_type in project.song_mut().tracks_mut().iter_mut() {
//                 let mut unused_changes = vec![];
//                 for (position, original_riff_ref_uuid) in new_riff_references_details.iter() {
//                     // get the original riff ref linked to value
//                     let linked_to = if let Some(original_riff_ref) = track_type.riff_refs_mut().iter_mut().find(|riff_ref| riff_ref.id() == original_riff_ref_uuid.clone()) {
//                         Some(original_riff_ref.linked_to())
//                     } else {
//                         None
//                     };
//                     if let Some(linked_to) = linked_to {
//                         let snap_delta = position % snap_position_in_beats;
//                         let new_position = position - snap_delta;
//                         if new_position >= 0.0 {
//                             let riff_ref = RiffReference::new(linked_to, new_position);
//                             match track_type {
//                                 TrackType::InstrumentTrack(track) => {
//                                     track.riff_refs_mut().push(riff_ref);
//                                 }
//                                 TrackType::MidiTrack(track) => {
//                                     track.riff_refs_mut().push(riff_ref);
//                                 }
//                                 _ => {}
//                             }
//                         }
//                     } else {
//                         unused_changes.push((*position, original_riff_ref_uuid.clone()));
//                     }
//                 }
//
//                 new_riff_references_details.clear();
//                 new_riff_references_details.append(&mut unused_changes);
//             }
//         },
//         Err(_) => debug!("Main - rx_ui processing loop - add new riff reference to track - could not get lock on state"),
//     }
//     // gui.ui.track_drawing_area.queue_draw();
// }

pub fn track_change_type_RiffReferencesSelectMultiple(state: &mut RiffDAWState, x: f64, y: i32, x2: f64, y2: i32, add_to_select: bool) {
    debug!("Main - rx_ui processing loop - pub fn track_change_type_RiffReferencesSelectMultiple: x={}, y={}, x2={}, y2={}, add_to_select={}", x, y, x2, y2, add_to_select);
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let mut selected = Vec::new();
            let mut state = state;

            for (index, track) in project.song_mut().tracks_mut().iter_mut().enumerate() {
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
    // gui.ui.track_drawing_area.queue_draw();
}

pub fn track_change_type_RiffReferencesSelectSingle(state: &mut RiffDAWState, x: f64, y: i32, add_to_select: bool) {
    debug!("Main - rx_ui processing loop - pub fn track_change_type_RiffReferencesSelectSingle: x={}, y={}, add_to_select={}", x, y, add_to_select);
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let mut selected = Vec::new();
            let mut state = state;

            if let Some(track) = project.song_mut().tracks_mut().get_mut(y as usize) {
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
        Err(_) => debug!("Main - rx_ui processing loop - pub fn track_change_type_RiffReferencesSelectSingle - could not get lock on state"),
    }
    // gui.ui.track_drawing_area.queue_draw();
}

pub fn track_change_type_RiffReferencesDeselectMultiple(state: &mut RiffDAWState, x: f64, y: i32, x2: f64, y2: i32) {
    debug!("Main - rx_ui processing loop - pub fn track_change_type_RiffReferencesDeselectMultiple: x={}, y={}, x2={}, y2={}", x, y, x2, y2);
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let mut selected = Vec::new();
            let mut state = state;

            for (index, track) in project.song_mut().tracks_mut().iter_mut().enumerate() {
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
    // gui.ui.track_drawing_area.queue_draw();
}

pub fn track_change_type_RiffReferencesDeselectSingle(state: &mut RiffDAWState, x: f64, y: i32) {
    debug!("Main - rx_ui processing loop - pub fn track_change_type_RiffReferencesDeselectSingle: x={}, y={},", x, y);
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let mut selected = Vec::new();
            let mut state = state;

            if let Some(track) = project.song_mut().tracks_mut().get_mut(y as usize) {
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
        Err(_) => debug!("Main - rx_ui processing loop - pub fn track_change_type_RiffReferencesDeselectSingle - could not get lock on state"),
    }
    // gui.ui.track_drawing_area.queue_draw();
}

pub fn track_change_type_RiffReferencesSelectAll(state: &mut RiffDAWState) {
    debug!("Main - rx_ui processing loop - pub fn track_change_type_RiffReferencesSelectAll");
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let mut selected = Vec::new();
            for track in project.song_mut().tracks_mut().iter_mut() {
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
    // gui.ui.track_drawing_area.queue_draw();
}

pub fn track_change_type_RiffReferencesDeselectAll(state: &mut RiffDAWState) {
    debug!("Main - rx_ui processing loop - pub fn track_change_type_RiffReferencesDeselectAll");
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            state.selected_track_grid_riff_references_mut().clear();
        }
        Err(_) => debug!("Main - rx_ui processing loop - RiffReferencesDeselectAll - could not get lock on state"),
    }
    // gui.ui.track_drawing_area.queue_draw();

}

pub fn track_change_type_RiffReferenceIncrementRiff(state: &mut RiffDAWState, track_index: i32, position: f64) {
    debug!("Main - rx_ui processing loop - pub fn track_change_type_RiffReferenceIncrementRiff: track_index={}, position={}", track_index, position);
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            // get the track
            let track_riff = if let Some(track) = project.song_mut().tracks_mut().get_mut(track_index as usize) {
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
                            // gui.ui.track_drawing_area.queue_draw();

                            // if let Some(riff) = track.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == riff_id.clone()) {
                            //     scroll_notes_into_view(gui, riff);
                            // }

                            Some((track_uuid, riff_id.clone(), track_name.to_string(), name.clone()))
                        } else { None }
                    } else { None }
                } else { None }
            }
            else { None };

            if let Some((track_uuid, riff_uuid, track_name, riff_name)) = track_riff {
                state.set_selected_riff_uuid(track_uuid.clone(), riff_uuid);
                state.set_selected_track(Some(track_uuid));
                // gui.set_piano_roll_selected_track_name_label(track_name.as_str());
                // gui.set_piano_roll_selected_riff_name_label(riff_name.as_str());

                // gui.ui.piano_roll_drawing_area.queue_draw();
            }
        }
        Err(_) => debug!("Main - rx_ui processing loop - pub fn track_change_type_RiffReferenceIncrementRiff - could not get lock on state"),
    }
}

// pub fn daw_events_TrackEffectParameterChange(state: &mut RiffDAWState, effect_number: i32, effect_param_number: i32) {
//     debug!("Event: TrackEffectParameterChange");
// }
//
// pub fn daw_events_TrackInstrumentParameterChange(state: &mut RiffDAWState, instrument_param_number: i32) {
//     debug!("Event: TrackInstrumentParameterChange");
// }
//
// pub fn daw_events_TrackSelectedPatternChange(state: &mut RiffDAWState, track_num: i32, pattern_index: i32) {
//     debug!("Event: TrackSelectedPatternChange");
// }
