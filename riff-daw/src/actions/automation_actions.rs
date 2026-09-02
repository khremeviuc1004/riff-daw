use itertools::Itertools;
use crate::constants::{CONTROLLER_TYPES, EVENT_DELETION_BEAT_TOLERANCE};
use crate::domain::{AutomationEnvelope, Controller, DAWItemID, DAWItemPosition, NoteExpression, PitchBend, PluginParameter, Track, TrackEvent, TrackType, UuidWrapper};
use crate::event::{AutomationEditType, CurrentView, TranslateDirection};
use crate::state::{AutomationViewMode, RiffDAWState};
use crate::utils::DAWUtils;



// FIXME work in progress trying to figure out how to appease the mighty borrow checker
// pub fn get_current_context_automation_events(state: &mut Project) -> (String, Option<i32>, Option<String>, CurrentView, AutomationEditType, Option<&mut Vec<TrackEvent>>, Option<String>) {
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
//     let plugin_uuid = if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
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
//             if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
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
//     } else if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
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


pub fn handle_automation_add(time: f64, value: i32, state: &mut RiffDAWState) {
        match state.automation_view_mode() {
            AutomationViewMode::Controllers => handle_automation_controller_add(time, value, state),
            AutomationViewMode::PitchBend => handle_automation_pitch_bend_add(time, value, state),
            AutomationViewMode::Instrument => handle_automation_instrument_add(time, value, state),
            AutomationViewMode::Effect => handle_automation_effect_add(time, value, state),
            AutomationViewMode::NoteExpression => handle_automation_note_expression_add(time, value, state),
            _ => (),
        }
}

pub fn handle_automation_instrument_add(time: f64, value: i32, state: &mut RiffDAWState) {
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_view_state.instrument_parameter_type;
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();
    let automation_discrete = state.automation_discrete();

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
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
                        if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
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
                                            id: UuidWrapper::new_v4(),
                                            plugin_uuid: UuidWrapper::new_from_string(plugin_uuid.clone()),
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
                                            id: UuidWrapper::new_v4(),
                                            plugin_uuid: UuidWrapper::new_from_string(plugin_uuid.clone()),
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
                        id: UuidWrapper::new_v4(),
                        plugin_uuid: UuidWrapper::new_from_string(plugin_uuid.clone()),
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
}

pub fn handle_automation_note_expression_add(time: f64, value: i32, state: &mut RiffDAWState) {
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let note_expression_type = state.automation_view_state.note_expression_type.clone();
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

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
            if let TrackType::InstrumentTrack(_instrument_track) = track_type {
                let events = if let CurrentView::RiffArrangement = current_view {
                    let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                        Some(selected_arrangement_uuid.clone())
                    } else {
                        None
                    };

                    // get the arrangement
                    if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                        if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                            let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                riff_arr_automation
                            } else {
                                riff_arrangement.add_track_automation(track_uuid.clone());
                                riff_arrangement.automation_mut(&track_uuid).unwrap()
                            };
                            if automation_discrete {
                                Some(automation.events_mut())
                            } else {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) == note_expression_type {
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
                                                note_expression.port() == note_expression_port_index &&
                                                note_expression.channel() == note_expression_channel &&
                                                note_expression.note_id() == note_expression_id &&
                                                note_expression.key() == note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
                                    } else {
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
                            } else {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type())  == note_expression_type {
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
                                                note_expression.port() == note_expression_port_index &&
                                                note_expression.channel() == note_expression_channel &&
                                                note_expression.note_id() == note_expression_id &&
                                                note_expression.key() == note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
                                    } else {
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
}

pub fn handle_automation_effect_add(time: f64, value: i32, state: &mut RiffDAWState) {
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_view_state.effect_parameter_type;
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

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
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
                            if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                                let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                    riff_arr_automation
                                } else {
                                    riff_arrangement.add_track_automation(track_uuid.clone());
                                    riff_arrangement.automation_mut(&track_uuid).unwrap()
                                };
                                if automation_discrete {
                                    Some(automation.events_mut())
                                } else {
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
                                                id: UuidWrapper::new_v4(),
                                                plugin_uuid: UuidWrapper::new_from_string(selected_effect_uuid.clone()),
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
                                            } else {
                                                None
                                            }
                                        }
                                    } else {
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
                                } else {
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
                                                id: UuidWrapper::new_v4(),
                                                plugin_uuid: UuidWrapper::new_from_string(selected_effect_uuid.clone()),
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
                                            } else {
                                                None
                                            }
                                        }
                                    } else {
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
                            id: UuidWrapper::new_v4(),
                            plugin_uuid: UuidWrapper::new_from_string(selected_effect_uuid.clone()),
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
}

pub fn handle_automation_controller_add(time: f64, value: i32, state: &mut RiffDAWState) {
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let controller_type = state.automation_view_state.controller_type_index.clone();
    let automation_edit_type = state.automation_edit_type();
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_discrete = state.automation_discrete();

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
            let events = if let CurrentView::RiffArrangement = current_view {
                let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                    Some(selected_arrangement_uuid.clone())
                } else {
                    None
                };

                // get the arrangement
                if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                    if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                        let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            riff_arr_automation
                        } else {
                            riff_arrangement.add_track_automation(track_uuid.clone());
                            riff_arrangement.automation_mut(&track_uuid).unwrap()
                        };
                        if automation_discrete {
                            Some(automation.events_mut())
                        } else {
                            if let Some(controller_type_index) = controller_type.as_ref() {
                                let controller_type_value = *controller_type_index as i32;
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::Controller(controller) = envelope.event_details() {
                                        if controller.controller() == CONTROLLER_TYPES.get(controller_type_value as usize).unwrap().0 {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(automation_envelope.events_mut())
                                } else {
                                    let event_details = Controller::new(0.0, controller_type_value.clone(), 0);
                                    let new_envelope = AutomationEnvelope::new(TrackEvent::Controller(event_details));
                                    automation.envelopes_mut().push(new_envelope);
                                    if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::Controller(controller) = envelope.event_details() {
                                            if controller.controller() == CONTROLLER_TYPES.get(controller_type_value as usize).unwrap().0 {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
                                    } else {
                                        None
                                    }
                                }
                            } else {
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
                        } else {
                            if let Some(controller_type_value) = controller_type.as_ref() {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::Controller(controller) = envelope.event_details() {
                                        if controller.controller() == CONTROLLER_TYPES.get(*controller_type_value as usize).unwrap().0 {
                                            found = true;
                                        }
                                    }
                                    return found;
                                }) {
                                    Some(automation_envelope.events_mut())
                                } else {
                                    let event_details = Controller::new(0.0, (*controller_type_value as i32), 0);
                                    let new_envelope = AutomationEnvelope::new(TrackEvent::Controller(event_details));
                                    automation.envelopes_mut().push(new_envelope);
                                    if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::Controller(controller) = envelope.event_details() {
                                            if controller.controller() == CONTROLLER_TYPES.get(*controller_type_value as usize).unwrap().0 {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
                                    } else {
                                        None
                                    }
                                }
                            } else {
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

            if let Some(controller_type_value) = controller_type.as_ref() {
                if let Some(events) = events {
                    let controller = Controller::new(time, CONTROLLER_TYPES.get(*controller_type_value as usize).unwrap().0, value);
                    events.push(TrackEvent::Controller(controller));
                    events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
                }
            }
        }
    }
}

pub fn handle_automation_pitch_bend_add(time: f64, value: i32, state: &mut RiffDAWState) {
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

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
            let events = if let CurrentView::RiffArrangement = current_view {
                let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                    Some(selected_arrangement_uuid.clone())
                } else {
                    None
                };

                // get the arrangement
                if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                    if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                        let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            riff_arr_automation
                        } else {
                            riff_arrangement.add_track_automation(track_uuid.clone());
                            riff_arrangement.automation_mut(&track_uuid).unwrap()
                        };
                        if automation_discrete {
                            Some(automation.events_mut())
                        } else {
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
                                } else {
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
                        } else {
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
                                } else {
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
}

pub fn handle_automation_delete(time: f64, state: &mut RiffDAWState) {
    match state.automation_view_mode() {
        AutomationViewMode::Controllers => handle_automation_controller_delete(time, state),
        AutomationViewMode::PitchBend => handle_automation_pitch_bend_delete(time, state),
        AutomationViewMode::Instrument => handle_automation_instrument_delete(time, state),
        AutomationViewMode::Effect => handle_automation_effect_delete(time, state),
        AutomationViewMode::NoteExpression => handle_automation_note_expression_delete(time, state),
        _ => (),
    }
}

pub fn handle_automation_instrument_delete(time: f64, state: &mut RiffDAWState) {
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_view_state.instrument_parameter_type.clone();
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();
    let automation_discrete = state.automation_discrete();

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
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
                        if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                            if let Some(automation) = riff_arrangement.automation_mut(&track_uuid) {
                                if automation_discrete {
                                    Some(automation.events_mut())
                                } else {
                                    if let Some(automation_type_value) = automation_type.as_ref() {
                                        if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                            let mut found = false;
                                            if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                if param.index == *automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                                    found = true;
                                                }
                                            }
                                            return found;
                                        }) {
                                            Some(automation_envelope.events_mut())
                                        } else { None }
                                    } else { None }
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
                            } else {
                                if let Some(automation_type_value) = automation_type.as_ref() {
                                    if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == *automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(automation_envelope.events_mut())
                                    } else {
                                        let event_details = PluginParameter {
                                            id: UuidWrapper::new_v4(),
                                            plugin_uuid: UuidWrapper::new_from_string(plugin_uuid.clone()),
                                            instrument: true,
                                            position: 0.0,
                                            index: automation_type_value.clone(),
                                            value: 0.0,
                                        };
                                        let mut new_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
                                        automation.envelopes_mut().push(new_envelope);
                                        if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                            let mut found = false;
                                            if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                if param.index == *automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                                    found = true;
                                                }
                                            }
                                            return found;
                                        }) {
                                            Some(envelope.events_mut())
                                        } else {
                                            None
                                        }
                                    }
                                } else {
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

                if let Some(automation_type_value) = automation_type.as_ref() {
                    if let Some(events) = events {
                        events.retain(|event| {
                            match event {
                                TrackEvent::AudioPluginParameter(plugin_parameter) => {
                                    !(plugin_parameter.index == *automation_type_value &&
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
}

pub fn handle_automation_note_expression_delete(time: f64, state: &mut RiffDAWState) {
    let note_expression_type = state.note_expression_type_mut().clone();
    let automation_type = state.automation_view_state.note_expression_type.clone();
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

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
            if let TrackType::InstrumentTrack(_instrument_track) = track_type {
                let events = if let CurrentView::RiffArrangement = current_view {
                    let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                        Some(selected_arrangement_uuid.clone())
                    } else {
                        None
                    };

                    // get the arrangement
                    if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                        if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                            let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                riff_arr_automation
                            } else {
                                riff_arrangement.add_track_automation(track_uuid.clone());
                                riff_arrangement.automation_mut(&track_uuid).unwrap()
                            };
                            if automation_discrete {
                                Some(automation.events_mut())
                            } else {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) == automation_type {
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
                                                note_expression.port() == note_expression_port_index &&
                                                note_expression.channel() == note_expression_channel &&
                                                note_expression.note_id() == note_expression_id &&
                                                note_expression.key() == note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
                                    } else {
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
                            } else {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) == automation_type {
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
                                                note_expression.port() == note_expression_port_index &&
                                                note_expression.channel() == note_expression_channel &&
                                                note_expression.note_id() == note_expression_id &&
                                                note_expression.key() == note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
                                    } else {
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
}

pub fn handle_automation_effect_delete(time: f64, state: &mut RiffDAWState) {
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_view_state.effect_parameter_type;
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

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
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
                            if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                                let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                    riff_arr_automation
                                } else {
                                    riff_arrangement.add_track_automation(track_uuid.clone());
                                    riff_arrangement.automation_mut(&track_uuid).unwrap()
                                };
                                if automation_discrete {
                                    Some(automation.events_mut())
                                } else {
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
                                                id: UuidWrapper::new_v4(),
                                                plugin_uuid: UuidWrapper::new_from_string(selected_effect_uuid.clone()),
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
                                            } else {
                                                None
                                            }
                                        }
                                    } else {
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
                                } else {
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
                                                id: UuidWrapper::new_v4(),
                                                plugin_uuid: UuidWrapper::new_from_string(selected_effect_uuid.clone()),
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
                                            } else {
                                                None
                                            }
                                        }
                                    } else {
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
}

pub fn handle_automation_controller_delete(time: f64, state: &mut RiffDAWState) {
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_view_state.controller_type_index.clone();
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();
    let automation_discrete = state.automation_discrete();

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
            let events = if let CurrentView::RiffArrangement = current_view {
                let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                    Some(selected_arrangement_uuid.clone())
                } else {
                    None
                };

                // get the arrangement
                if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                    if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                        let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            riff_arr_automation
                        } else {
                            riff_arrangement.add_track_automation(track_uuid.clone());
                            riff_arrangement.automation_mut(&track_uuid).unwrap()
                        };
                        if automation_discrete {
                            Some(automation.events_mut())
                        } else {
                            if let Some(automation_type_value) = automation_type.as_ref() {
                                let automation_type_value = CONTROLLER_TYPES.get(*automation_type_value as usize).unwrap().0;
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
                                    } else {
                                        None
                                    }
                                }
                            } else {
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
                        } else {
                            if let Some(automation_type_value) = automation_type.as_ref() {
                                let automation_type_value = CONTROLLER_TYPES.get(*automation_type_value as usize).unwrap().0;
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
                                    } else {
                                        None
                                    }
                                }
                            } else {
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

            if let Some(automation_type_value) = automation_type.as_ref() {
                let automation_type_value = CONTROLLER_TYPES.get(*automation_type_value as usize).unwrap().0;
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
}

pub fn handle_automation_pitch_bend_delete(time: f64, state: &mut RiffDAWState) {
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

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
            let events = if let CurrentView::RiffArrangement = current_view {
                let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                    Some(selected_arrangement_uuid.clone())
                } else {
                    None
                };

                // get the arrangement
                if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                    if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                        let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            riff_arr_automation
                        } else {
                            riff_arrangement.add_track_automation(track_uuid.clone());
                            riff_arrangement.automation_mut(&track_uuid).unwrap()
                        };
                        if automation_discrete {
                            Some(automation.events_mut())
                        } else {
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
                                } else {
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
                        } else {
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
                                } else {
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
}

pub fn handle_automation_cut(state: &mut RiffDAWState, edit_cursor_time_in_beats: f64) {
    match state.automation_view_mode() {
        AutomationViewMode::Controllers => handle_automation_controller_cut(state, edit_cursor_time_in_beats),
        AutomationViewMode::PitchBend => handle_automation_pitch_bend_cut(state, edit_cursor_time_in_beats),
        AutomationViewMode::Instrument => handle_automation_instrument_cut(state, edit_cursor_time_in_beats),
        AutomationViewMode::Effect => handle_automation_effect_cut(state, edit_cursor_time_in_beats),
        AutomationViewMode::NoteExpression => handle_automation_note_expression_cut(state, edit_cursor_time_in_beats),
        _ => (),
    }
}

pub fn handle_automation_instrument_cut(state: &mut RiffDAWState, edit_cursor_time_in_beats: f64) {
    let selected = state.selected_automation().to_vec();
    let mut events_to_copy = vec![];
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_view_state.instrument_parameter_type.clone();
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();
    let automation_discrete = state.automation_discrete();

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
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
                        if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                            if let Some(automation) = riff_arrangement.automation_mut(&track_uuid) {
                                if automation_discrete {
                                    Some(automation.events_mut())
                                } else {
                                    if let Some(automation_type_value) = automation_type.as_ref() {
                                        if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                            let mut found = false;
                                            if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                if param.index == *automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                                    found = true;
                                                }
                                            }
                                            return found;
                                        }) {
                                            Some(automation_envelope.events_mut())
                                        } else { None }
                                    } else { None }
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
                            } else {
                                if let Some(automation_type_value) = automation_type.as_ref() {
                                    if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                        let mut found = false;
                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                            if param.index == *automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(automation_envelope.events_mut())
                                    } else {
                                        let event_details = PluginParameter {
                                            id: UuidWrapper::new_v4(),
                                            plugin_uuid: UuidWrapper::new_from_string(plugin_uuid.clone()),
                                            instrument: true,
                                            position: 0.0,
                                            index: automation_type_value.clone(),
                                            value: 0.0,
                                        };
                                        let mut new_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
                                        automation.envelopes_mut().push(new_envelope);
                                        if let Some(envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                            let mut found = false;
                                            if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                if param.index == *automation_type_value && param.plugin_uuid() == plugin_uuid.to_string() {
                                                    found = true;
                                                }
                                            }
                                            return found;
                                        }) {
                                            Some(envelope.events_mut())
                                        } else {
                                            None
                                        }
                                    }
                                } else {
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

                if let Some(automation_type_value) = automation_type.as_ref() {
                    if let Some(events) = events {
                        for event in events.iter().filter(|event| selected.contains(&event.id())) {
                            if let TrackEvent::AudioPluginParameter(plugin_param) = event {
                                if plugin_param.plugin_uuid().to_string() == plugin_uuid.to_string() && plugin_param.index == *automation_type_value {
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
                                    !(plugin_param.index == *automation_type_value && selected.contains(&event.id()))
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

pub fn handle_automation_note_expression_cut(state: &mut RiffDAWState, edit_cursor_time_in_beats: f64) {
    let selected = state.selected_automation().to_vec();
    let automation_type = state.automation_view_state.note_expression_type.clone();
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

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
            if let TrackType::InstrumentTrack(_instrument_track) = track_type {
                let events = if let CurrentView::RiffArrangement = current_view {
                    let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                        Some(selected_arrangement_uuid.clone())
                    } else {
                        None
                    };

                    // get the arrangement
                    if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                        if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                            let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                riff_arr_automation
                            } else {
                                riff_arrangement.add_track_automation(track_uuid.clone());
                                riff_arrangement.automation_mut(&track_uuid).unwrap()
                            };
                            if automation_discrete {
                                Some(automation.events_mut())
                            } else {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) == automation_type {
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
                                                note_expression.port() == note_expression_port_index &&
                                                note_expression.channel() == note_expression_channel &&
                                                note_expression.note_id() == note_expression_id &&
                                                note_expression.key() == note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
                                    } else {
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
                            } else {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) == automation_type {
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
                                                note_expression.port() == note_expression_port_index &&
                                                note_expression.channel() == note_expression_channel &&
                                                note_expression.note_id() == note_expression_id &&
                                                note_expression.key() == note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
                                    } else {
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
    }

    if !events_to_copy.is_empty() {
        state.automation_event_copy_buffer_mut().clear();
        for event in events_to_copy.iter() {
            state.automation_event_copy_buffer_mut().push(event.clone());
        }
    }
}

pub fn handle_automation_effect_cut(state: &mut RiffDAWState, edit_cursor_time_in_beats: f64) {
    let selected = state.selected_automation().to_vec();
    let mut events_to_copy: Vec<TrackEvent> = vec![];
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_view_state.effect_parameter_type.clone();
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

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
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
                            if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                                let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                    riff_arr_automation
                                } else {
                                    riff_arrangement.add_track_automation(track_uuid.clone());
                                    riff_arrangement.automation_mut(&track_uuid).unwrap()
                                };
                                if automation_discrete {
                                    Some(automation.events_mut())
                                } else {
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
                                                id: UuidWrapper::new_v4(),
                                                plugin_uuid: UuidWrapper::new_from_string(selected_effect_uuid.clone()),
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
                                            } else {
                                                None
                                            }
                                        }
                                    } else {
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
                                } else {
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
                                                id: UuidWrapper::new_v4(),
                                                plugin_uuid: UuidWrapper::new_from_string(selected_effect_uuid.clone()),
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
                                            } else {
                                                None
                                            }
                                        }
                                    } else {
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
    }

    if !events_to_copy.is_empty() {
        state.automation_event_copy_buffer_mut().clear();
        for event in events_to_copy.iter() {
            state.automation_event_copy_buffer_mut().push(event.clone());
        }
    }
}

pub fn handle_automation_controller_cut(state: &mut RiffDAWState, edit_cursor_time_in_beats: f64) {
    let selected = state.selected_automation().to_vec();
    let mut events_to_copy: Vec<TrackEvent> = vec![];
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_view_state.controller_type_index.clone();
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();
    let automation_discrete = state.automation_discrete();

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
            let events = if let CurrentView::RiffArrangement = current_view {
                let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                    Some(selected_arrangement_uuid.clone())
                } else {
                    None
                };

                // get the arrangement
                if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                    if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                        let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            riff_arr_automation
                        } else {
                            riff_arrangement.add_track_automation(track_uuid.clone());
                            riff_arrangement.automation_mut(&track_uuid).unwrap()
                        };
                        if automation_discrete {
                            Some(automation.events_mut())
                        } else {
                            if let Some(automation_type_value) = automation_type.as_ref() {
                                let automation_type_value = CONTROLLER_TYPES.get(*automation_type_value as usize).unwrap().0;
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
                                    } else {
                                        None
                                    }
                                }
                            } else {
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
                        } else {
                            if let Some(automation_type_value) = automation_type.as_ref() {
                                let automation_type_value = CONTROLLER_TYPES.get(*automation_type_value as usize).unwrap().0;
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
                                    } else {
                                        None
                                    }
                                }
                            } else {
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

            if let Some(automation_type_value) = automation_type.as_ref() {
                let automation_type_value = CONTROLLER_TYPES.get(*automation_type_value as usize).unwrap().0;
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
    }

    if !events_to_copy.is_empty() {
        state.automation_event_copy_buffer_mut().clear();
        for event in events_to_copy.iter() {
            state.automation_event_copy_buffer_mut().push(event.clone());
        }
    }
}

pub fn handle_automation_pitch_bend_cut(state: &mut RiffDAWState, edit_cursor_time_in_beats: f64) {
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

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
            let events = if let CurrentView::RiffArrangement = current_view {
                let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                    Some(selected_arrangement_uuid.clone())
                } else {
                    None
                };

                // get the arrangement
                if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                    if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                        let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            riff_arr_automation
                        } else {
                            riff_arrangement.add_track_automation(track_uuid.clone());
                            riff_arrangement.automation_mut(&track_uuid).unwrap()
                        };
                        if automation_discrete {
                            Some(automation.events_mut())
                        } else {
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
                                } else {
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
                        } else {
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
                                } else {
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
    }

    if !events_to_copy.is_empty() {
        state.automation_event_copy_buffer_mut().clear();
        for event in events_to_copy.iter() {
            state.automation_event_copy_buffer_mut().push(event.clone());
        }
    }
}

pub fn handle_automation_translate_selected(state: &mut RiffDAWState, translate_direction: TranslateDirection, snap_in_beats: f64) {
    match state.automation_view_mode() {
        AutomationViewMode::Controllers => handle_automation_controller_translate_selected(state, translate_direction, snap_in_beats),
        AutomationViewMode::PitchBend => handle_automation_pitch_bend_translate_selected(state, translate_direction, snap_in_beats),
        AutomationViewMode::Instrument => handle_automation_instrument_translate_selected(state, translate_direction, snap_in_beats),
        AutomationViewMode::Effect => handle_automation_effect_translate_selected(state, translate_direction, snap_in_beats),
        AutomationViewMode::NoteExpression => handle_automation_note_expression_translate_selected(state, translate_direction, snap_in_beats),
        AutomationViewMode::NoteVelocities => handle_automation_note_velocities_translate_selected(state, translate_direction),
    }
}

pub fn handle_automation_instrument_translate_selected(state: &mut RiffDAWState, translate_direction: TranslateDirection, snap_in_beats: f64) {
    let selected = state.selected_automation().to_vec();
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_view_state.instrument_parameter_type.clone();
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();
    let automation_discrete = state.automation_discrete();

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
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
                        if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                            if let Some(automation) = riff_arrangement.automation_mut(&track_uuid) {
                                if automation_discrete {
                                    Some(automation.events_mut())
                                } else {
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
                                    } else { None }
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
                            } else {
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
                                            id: UuidWrapper::new_v4(),
                                            plugin_uuid: UuidWrapper::new_from_string(plugin_uuid.clone()),
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
                                        } else {
                                            None
                                        }
                                    }
                                } else {
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

pub fn handle_automation_note_expression_translate_selected(state: &mut RiffDAWState, translate_direction: TranslateDirection, snap_in_beats: f64) {
    let selected = state.selected_automation().to_vec();
    let automation_type = state.automation_view_state.note_expression_type.clone();
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

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
            if let TrackType::InstrumentTrack(_instrument_track) = track_type {
                let events = if let CurrentView::RiffArrangement = current_view {
                    let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                        Some(selected_arrangement_uuid.clone())
                    } else {
                        None
                    };

                    // get the arrangement
                    if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                        if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                            let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                riff_arr_automation
                            } else {
                                riff_arrangement.add_track_automation(track_uuid.clone());
                                riff_arrangement.automation_mut(&track_uuid).unwrap()
                            };
                            if automation_discrete {
                                Some(automation.events_mut())
                            } else {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) == automation_type {
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
                                                note_expression.port() == note_expression_port_index &&
                                                note_expression.channel() == note_expression_channel &&
                                                note_expression.note_id() == note_expression_id &&
                                                note_expression.key() == note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
                                    } else {
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
                            } else {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) == automation_type {
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
                                                note_expression.port() == note_expression_port_index &&
                                                note_expression.channel() == note_expression_channel &&
                                                note_expression.note_id() == note_expression_id &&
                                                note_expression.key() == note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
                                    } else {
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
}

pub fn handle_automation_effect_translate_selected(state: &mut RiffDAWState, translate_direction: TranslateDirection, snap_in_beats: f64) {
    let selected = state.selected_automation().to_vec();
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_view_state.effect_parameter_type.clone();
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

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
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
                            if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                                let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                    riff_arr_automation
                                } else {
                                    riff_arrangement.add_track_automation(track_uuid.clone());
                                    riff_arrangement.automation_mut(&track_uuid).unwrap()
                                };
                                if automation_discrete {
                                    Some(automation.events_mut())
                                } else {
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
                                                id: UuidWrapper::new_v4(),
                                                plugin_uuid: UuidWrapper::new_from_string(selected_effect_uuid.clone()),
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
                                            } else {
                                                None
                                            }
                                        }
                                    } else {
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
                                } else {
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
                                                id: UuidWrapper::new_v4(),
                                                plugin_uuid: UuidWrapper::new_from_string(selected_effect_uuid.clone()),
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
                                            } else {
                                                None
                                            }
                                        }
                                    } else {
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
}

pub fn handle_automation_controller_translate_selected(state: &mut RiffDAWState, translate_direction: TranslateDirection, snap_in_beats: f64) {
    let selected = state.selected_automation().to_vec();
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_view_state.controller_type_index.clone();
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();
    let automation_discrete = state.automation_discrete();

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
            let events = if let CurrentView::RiffArrangement = current_view {
                let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                    Some(selected_arrangement_uuid.clone())
                } else {
                    None
                };

                // get the arrangement
                if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                    if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                        let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            riff_arr_automation
                        } else {
                            riff_arrangement.add_track_automation(track_uuid.clone());
                            riff_arrangement.automation_mut(&track_uuid).unwrap()
                        };
                        if automation_discrete {
                            Some(automation.events_mut())
                        } else {
                            if let Some(automation_type_value) = automation_type.as_ref() {
                                let automation_type_value = CONTROLLER_TYPES.get(*automation_type_value as usize).unwrap().0;
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
                                    } else {
                                        None
                                    }
                                }
                            } else {
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
                        } else {
                            if let Some(automation_type_value) = automation_type.as_ref() {
                                let automation_type_value = CONTROLLER_TYPES.get(*automation_type_value as usize).unwrap().0;
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
                                    } else {
                                        None
                                    }
                                }
                            } else {
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

            if let Some(automation_type_value) = automation_type.as_ref() {
                let automation_type_value = CONTROLLER_TYPES.get(*automation_type_value as usize).unwrap().0;
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
}

pub fn handle_automation_note_velocities_translate_selected(state: &mut RiffDAWState, translate_direction: TranslateDirection) {
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

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
            let events = if let CurrentView::RiffArrangement = current_view {
                let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                    Some(selected_arrangement_uuid.clone())
                } else {
                    None
                };

                // get the arrangement
                if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                    if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
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
}

pub fn handle_automation_pitch_bend_translate_selected(state: &mut RiffDAWState, translate_direction: TranslateDirection, snap_in_beats: f64) {
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

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
            let events = if let CurrentView::RiffArrangement = current_view {
                let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                    Some(selected_arrangement_uuid.clone())
                } else {
                    None
                };

                // get the arrangement
                if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                    if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                        let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            riff_arr_automation
                        } else {
                            riff_arrangement.add_track_automation(track_uuid.clone());
                            riff_arrangement.automation_mut(&track_uuid).unwrap()
                        };
                        if automation_discrete {
                            Some(automation.events_mut())
                        } else {
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
                                } else {
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
                        } else {
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
                                } else {
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
}


pub fn handle_automation_copy(state: &mut RiffDAWState, edit_cursor_time_in_beats: f64) {
    match state.automation_view_mode() {
        AutomationViewMode::Controllers => handle_automation_controller_copy(state, edit_cursor_time_in_beats),
        AutomationViewMode::PitchBend => handle_automation_pitch_bend_copy(state, edit_cursor_time_in_beats),
        AutomationViewMode::Instrument => handle_automation_instrument_copy(state, edit_cursor_time_in_beats),
        AutomationViewMode::Effect => handle_automation_effect_copy(state, edit_cursor_time_in_beats),
        AutomationViewMode::NoteExpression => handle_automation_note_expression_copy(state, edit_cursor_time_in_beats),
        _ => (),
    }
}

pub fn handle_automation_instrument_copy(state: &mut RiffDAWState, edit_cursor_time_in_beats: f64) {
    let selected = state.selected_automation().to_vec();
    let mut events_to_copy = vec![];
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_view_state.instrument_parameter_type.clone();
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();
    let automation_discrete = state.automation_discrete();

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
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
                        if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                            if let Some(automation) = riff_arrangement.automation_mut(&track_uuid) {
                                if automation_discrete {
                                    Some(automation.events_mut())
                                } else {
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
                                    } else { None }
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
                            } else {
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
                                            id: UuidWrapper::new_v4(),
                                            plugin_uuid: UuidWrapper::new_from_string(plugin_uuid.clone()),
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
                                        } else {
                                            None
                                        }
                                    }
                                } else {
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
    }

    if !events_to_copy.is_empty() {
        state.automation_event_copy_buffer_mut().clear();
        for event in events_to_copy.iter() {
            state.automation_event_copy_buffer_mut().push(event.clone());
        }
    }
}

pub fn handle_automation_note_expression_copy(state: &mut RiffDAWState, edit_cursor_time_in_beats: f64) {
    let selected = state.selected_automation().to_vec();
    let automation_type = state.automation_view_state.note_expression_type.clone();
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

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
            if let TrackType::InstrumentTrack(_instrument_track) = track_type {
                let events = if let CurrentView::RiffArrangement = current_view {
                    let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                        Some(selected_arrangement_uuid.clone())
                    } else {
                        None
                    };

                    // get the arrangement
                    if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                        if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                            let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                riff_arr_automation
                            } else {
                                riff_arrangement.add_track_automation(track_uuid.clone());
                                riff_arrangement.automation_mut(&track_uuid).unwrap()
                            };
                            if automation_discrete {
                                Some(automation.events_mut())
                            } else {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) == automation_type {
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
                                                note_expression.port() == note_expression_port_index &&
                                                note_expression.channel() == note_expression_channel &&
                                                note_expression.note_id() == note_expression_id &&
                                                note_expression.key() == note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
                                    } else {
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
                            } else {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) == automation_type {
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
                                                note_expression.port() == note_expression_port_index &&
                                                note_expression.channel() == note_expression_channel &&
                                                note_expression.note_id() == note_expression_id &&
                                                note_expression.key() == note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
                                    } else {
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
    }

    if !events_to_copy.is_empty() {
        state.automation_event_copy_buffer_mut().clear();
        for event in events_to_copy.iter() {
            state.automation_event_copy_buffer_mut().push(event.clone());
        }
    }
}

pub fn handle_automation_effect_copy(state: &mut RiffDAWState, edit_cursor_time_in_beats: f64) {
    let selected = state.selected_automation().to_vec();
    let mut events_to_copy = vec![];
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_view_state.effect_parameter_type.clone();
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

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
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
                            if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                                let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                    riff_arr_automation
                                } else {
                                    riff_arrangement.add_track_automation(track_uuid.clone());
                                    riff_arrangement.automation_mut(&track_uuid).unwrap()
                                };
                                if automation_discrete {
                                    Some(automation.events_mut())
                                } else {
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
                                                id: UuidWrapper::new_v4(),
                                                plugin_uuid: UuidWrapper::new_from_string(selected_effect_uuid.clone()),
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
                                            } else {
                                                None
                                            }
                                        }
                                    } else {
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
                                } else {
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
                                                id: UuidWrapper::new_v4(),
                                                plugin_uuid: UuidWrapper::new_from_string(selected_effect_uuid.clone()),
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
                                            } else {
                                                None
                                            }
                                        }
                                    } else {
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
    }

    if !events_to_copy.is_empty() {
        state.automation_event_copy_buffer_mut().clear();
        for event in events_to_copy.iter() {
            state.automation_event_copy_buffer_mut().push(event.clone());
        }
    }
}

pub fn handle_automation_controller_copy(state: &mut RiffDAWState, edit_cursor_time_in_beats: f64) {
    let selected = state.selected_automation().to_vec();
    let mut events_to_copy = vec![];
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_view_state.controller_type_index.clone();
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();
    let automation_discrete = state.automation_discrete();

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
            let events = if let CurrentView::RiffArrangement = current_view {
                let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                    Some(selected_arrangement_uuid.clone())
                } else {
                    None
                };

                // get the arrangement
                if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                    if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                        let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            riff_arr_automation
                        } else {
                            riff_arrangement.add_track_automation(track_uuid.clone());
                            riff_arrangement.automation_mut(&track_uuid).unwrap()
                        };
                        if automation_discrete {
                            Some(automation.events_mut())
                        } else {
                            if let Some(automation_type_value) = automation_type.as_ref() {
                                let automation_type_value = CONTROLLER_TYPES.get(*automation_type_value as usize).unwrap().0;
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
                                    } else {
                                        None
                                    }
                                }
                            } else {
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
                        } else {
                            if let Some(automation_type_value) = automation_type.as_ref() {
                                let automation_type_value = CONTROLLER_TYPES.get(*automation_type_value as usize).unwrap().0;
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
                                    } else {
                                        None
                                    }
                                }
                            } else {
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

            if let Some(automation_type_value) = automation_type.as_ref() {
                let automation_type_value = CONTROLLER_TYPES.get(*automation_type_value as usize).unwrap().0;
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
    }

    if !events_to_copy.is_empty() {
        state.automation_event_copy_buffer_mut().clear();
        for event in events_to_copy.iter() {
            state.automation_event_copy_buffer_mut().push(event.clone());
        }
    }
}

pub fn handle_automation_pitch_bend_copy(state: &mut RiffDAWState, edit_cursor_time_in_beats: f64) {
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

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
            let events = if let CurrentView::RiffArrangement = current_view {
                let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                    Some(selected_arrangement_uuid.clone())
                } else {
                    None
                };

                // get the arrangement
                if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                    if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                        let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            riff_arr_automation
                        } else {
                            riff_arrangement.add_track_automation(track_uuid.clone());
                            riff_arrangement.automation_mut(&track_uuid).unwrap()
                        };
                        if automation_discrete {
                            Some(automation.events_mut())
                        } else {
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
                                } else {
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
                        } else {
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
                                } else {
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
    }

    if !events_to_copy.is_empty() {
        state.automation_event_copy_buffer_mut().clear();
        for event in events_to_copy.iter() {
            state.automation_event_copy_buffer_mut().push(event.clone());
        }
    }
}

pub fn handle_automation_paste(state: &mut RiffDAWState, edit_cursor_time_in_beats: f64) {
    match state.automation_view_mode() {
        AutomationViewMode::Controllers => handle_automation_controller_paste(state, edit_cursor_time_in_beats),
        AutomationViewMode::PitchBend => handle_automation_pitch_bend_paste(state, edit_cursor_time_in_beats),
        AutomationViewMode::Instrument => handle_automation_instrument_paste(state, edit_cursor_time_in_beats),
        AutomationViewMode::Effect => handle_automation_effect_paste(state, edit_cursor_time_in_beats),
        AutomationViewMode::NoteExpression => handle_automation_note_expression_paste(state, edit_cursor_time_in_beats),
        _ => (),
    }
}

pub fn handle_automation_instrument_paste(state: &mut RiffDAWState, edit_cursor_time_in_beats: f64) {
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_view_state.instrument_parameter_type.clone();
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

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
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
                        if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                            if let Some(automation) = riff_arrangement.automation_mut(&track_uuid) {
                                if automation_discrete {
                                    Some(automation.events_mut())
                                } else {
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
                                    } else { None }
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
                            } else {
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
                                            id: UuidWrapper::new_v4(),
                                            plugin_uuid: UuidWrapper::new_from_string(plugin_uuid.clone()),
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
                                        } else {
                                            None
                                        }
                                    }
                                } else {
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

                                    track_event.set_id(UuidWrapper::new_v4().uuid.to_string());

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

pub fn handle_automation_note_expression_paste(state: &mut RiffDAWState, edit_cursor_time_in_beats: f64) {
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_view_state.note_expression_type.clone();
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

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
            if let TrackType::InstrumentTrack(_instrument_track) = track_type {
                let events = if let CurrentView::RiffArrangement = current_view {
                    let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                        Some(selected_arrangement_uuid.clone())
                    } else {
                        None
                    };

                    // get the arrangement
                    if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                        if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                            let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                riff_arr_automation
                            } else {
                                riff_arrangement.add_track_automation(track_uuid.clone());
                                riff_arrangement.automation_mut(&track_uuid).unwrap()
                            };
                            if automation_discrete {
                                Some(automation.events_mut())
                            } else {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) == automation_type {
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
                                                note_expression.port() == note_expression_port_index &&
                                                note_expression.channel() == note_expression_channel &&
                                                note_expression.note_id() == note_expression_id &&
                                                note_expression.key() == note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
                                    } else {
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
                            } else {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) == automation_type {
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
                                                note_expression.port() == note_expression_port_index &&
                                                note_expression.channel() == note_expression_channel &&
                                                note_expression.note_id() == note_expression_id &&
                                                note_expression.key() == note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
                                    } else {
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
                        if let TrackEvent::NoteExpression(note_expression) = event {
                            let mut track_event = event.clone();

                            track_event.set_id(UuidWrapper::new_v4().uuid.to_string());

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
}

pub fn handle_automation_effect_paste(state: &mut RiffDAWState, edit_cursor_time_in_beats: f64) {
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_view_state.effect_parameter_type.clone();
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

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
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
                            if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                                let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                    riff_arr_automation
                                } else {
                                    riff_arrangement.add_track_automation(track_uuid.clone());
                                    riff_arrangement.automation_mut(&track_uuid).unwrap()
                                };
                                if automation_discrete {
                                    Some(automation.events_mut())
                                } else {
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
                                                id: UuidWrapper::new_v4(),
                                                plugin_uuid: UuidWrapper::new_from_string(selected_effect_uuid.clone()),
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
                                            } else {
                                                None
                                            }
                                        }
                                    } else {
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
                                } else {
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
                                                id: UuidWrapper::new_v4(),
                                                plugin_uuid: UuidWrapper::new_from_string(selected_effect_uuid.clone()),
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
                                            } else {
                                                None
                                            }
                                        }
                                    } else {
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

                                        track_event.set_id(UuidWrapper::new_v4().uuid.to_string());

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
}

pub fn handle_automation_controller_paste(state: &mut RiffDAWState, edit_cursor_time_in_beats: f64) {
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_view_state.controller_type_index.clone();
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

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
            let events = if let CurrentView::RiffArrangement = current_view {
                let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                    Some(selected_arrangement_uuid.clone())
                } else {
                    None
                };

                // get the arrangement
                if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                    if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                        let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            riff_arr_automation
                        } else {
                            riff_arrangement.add_track_automation(track_uuid.clone());
                            riff_arrangement.automation_mut(&track_uuid).unwrap()
                        };
                        if automation_discrete {
                            Some(automation.events_mut())
                        } else {
                            if let Some(automation_type_value) = automation_type.as_ref() {
                                let automation_type_value = CONTROLLER_TYPES.get(*automation_type_value as usize).unwrap().0;
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
                                    } else {
                                        None
                                    }
                                }
                            } else {
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
                        } else {
                            if let Some(automation_type_value) = automation_type.as_ref() {
                                let automation_type_value = CONTROLLER_TYPES.get(*automation_type_value as usize).unwrap().0;
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
                                    } else {
                                        None
                                    }
                                }
                            } else {
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

            if let Some(automation_type_value) = automation_type.as_ref() {
                let automation_type_value = CONTROLLER_TYPES.get(*automation_type_value as usize).unwrap().0;
                if let Some(events) = events {
                    for event in automation_event_copy_buffer {
                        if let TrackEvent::Controller(controller) = event {
                            if controller.controller() == automation_type_value {
                                let mut track_event = event.clone();

                                track_event.set_id(UuidWrapper::new_v4().uuid.to_string());

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

pub fn handle_automation_pitch_bend_paste(state: &mut RiffDAWState, edit_cursor_time_in_beats: f64) {
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

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
            let events = if let CurrentView::RiffArrangement = current_view {
                let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                    Some(selected_arrangement_uuid.clone())
                } else {
                    None
                };

                // get the arrangement
                if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                    if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                        let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            riff_arr_automation
                        } else {
                            riff_arrangement.add_track_automation(track_uuid.clone());
                            riff_arrangement.automation_mut(&track_uuid).unwrap()
                        };
                        if automation_discrete {
                            Some(automation.events_mut())
                        } else {
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
                                } else {
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
                        } else {
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
                                } else {
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

                        track_event.set_id(UuidWrapper::new_v4().uuid.to_string());

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


pub fn handle_automation_quantise(state: &mut RiffDAWState, snap_in_beats: f64, quantise_strength: f64) {
    match state.automation_view_mode() {
        AutomationViewMode::Controllers => handle_automation_controller_quantise(state, snap_in_beats, quantise_strength),
        AutomationViewMode::PitchBend => handle_automation_pitch_bend_quantise(state, snap_in_beats, quantise_strength),
        AutomationViewMode::Instrument => handle_automation_instrument_quantise(state, snap_in_beats, quantise_strength),
        AutomationViewMode::Effect => handle_automation_effect_quantise(state, snap_in_beats, quantise_strength),
        AutomationViewMode::NoteExpression => handle_automation_note_expression_quantise(state, snap_in_beats, quantise_strength),
        _ => (),
    }
}

pub fn handle_automation_instrument_quantise(state: &mut RiffDAWState, snap_in_beats: f64, quantise_strength: f64) {
    let selected = state.selected_automation().to_vec();
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_view_state.instrument_parameter_type.clone();
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();
    let automation_discrete = state.automation_discrete();

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
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
                        if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                            if let Some(automation) = riff_arrangement.automation_mut(&track_uuid) {
                                if automation_discrete {
                                    Some(automation.events_mut())
                                } else {
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
                                    } else { None }
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
                            } else {
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
                                            id: UuidWrapper::new_v4(),
                                            plugin_uuid: UuidWrapper::new_from_string(plugin_uuid.clone()),
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
                                        } else {
                                            None
                                        }
                                    }
                                } else {
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
}

pub fn handle_automation_note_expression_quantise(state: &mut RiffDAWState, snap_in_beats: f64, quantise_strength: f64) {
    let selected = state.selected_automation().to_vec();
    let automation_type = state.automation_view_state.note_expression_type.clone();
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

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
            if let TrackType::InstrumentTrack(_instrument_track) = track_type {
                let events = if let CurrentView::RiffArrangement = current_view {
                    let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                        Some(selected_arrangement_uuid.clone())
                    } else {
                        None
                    };

                    // get the arrangement
                    if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                        if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                            let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                riff_arr_automation
                            } else {
                                riff_arrangement.add_track_automation(track_uuid.clone());
                                riff_arrangement.automation_mut(&track_uuid).unwrap()
                            };
                            if automation_discrete {
                                Some(automation.events_mut())
                            } else {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) == automation_type {
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
                                                note_expression.port() == note_expression_port_index &&
                                                note_expression.channel() == note_expression_channel &&
                                                note_expression.note_id() == note_expression_id &&
                                                note_expression.key() == note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
                                    } else {
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
                            } else {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) == automation_type {
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
                                                note_expression.port() == note_expression_port_index &&
                                                note_expression.channel() == note_expression_channel &&
                                                note_expression.note_id() == note_expression_id &&
                                                note_expression.key() == note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
                                    } else {
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
}

pub fn handle_automation_effect_quantise(state: &mut RiffDAWState, snap_in_beats: f64, quantise_strength: f64) {
    let selected = state.selected_automation().to_vec();
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_view_state.effect_parameter_type.clone();
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

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
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
                            if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                                let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                    riff_arr_automation
                                } else {
                                    riff_arrangement.add_track_automation(track_uuid.clone());
                                    riff_arrangement.automation_mut(&track_uuid).unwrap()
                                };
                                if automation_discrete {
                                    Some(automation.events_mut())
                                } else {
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
                                                id: UuidWrapper::new_v4(),
                                                plugin_uuid: UuidWrapper::new_from_string(selected_effect_uuid.clone()),
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
                                            } else {
                                                None
                                            }
                                        }
                                    } else {
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
                                } else {
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
                                                id: UuidWrapper::new_v4(),
                                                plugin_uuid: UuidWrapper::new_from_string(selected_effect_uuid.clone()),
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
                                            } else {
                                                None
                                            }
                                        }
                                    } else {
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
}

pub fn handle_automation_controller_quantise(state: &mut RiffDAWState, snap_in_beats: f64, quantise_strength: f64) {
    let selected = state.selected_automation().to_vec();
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_view_state.controller_type_index.clone();
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();
    let automation_discrete = state.automation_discrete();

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
            let events = if let CurrentView::RiffArrangement = current_view {
                let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                    Some(selected_arrangement_uuid.clone())
                } else {
                    None
                };

                // get the arrangement
                if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                    if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                        let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            riff_arr_automation
                        } else {
                            riff_arrangement.add_track_automation(track_uuid.clone());
                            riff_arrangement.automation_mut(&track_uuid).unwrap()
                        };
                        if automation_discrete {
                            Some(automation.events_mut())
                        } else {
                            if let Some(automation_type_value) = automation_type.as_ref() {
                                let automation_type_value = CONTROLLER_TYPES.get(*automation_type_value as usize).unwrap().0;
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
                                    } else {
                                        None
                                    }
                                }
                            } else {
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
                        } else {
                            if let Some(automation_type_value) = automation_type.as_ref() {
                                let automation_type_value = CONTROLLER_TYPES.get(*automation_type_value as usize).unwrap().0;
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
                                    } else {
                                        None
                                    }
                                }
                            } else {
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

            if let Some(automation_type_value) = automation_type.as_ref() {
                let automation_type_value = CONTROLLER_TYPES.get(*automation_type_value as usize).unwrap().0;
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
}

pub fn handle_automation_pitch_bend_quantise(state: &mut RiffDAWState, snap_in_beats: f64, quantise_strength: f64) {
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

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
            let events = if let CurrentView::RiffArrangement = current_view {
                let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                    Some(selected_arrangement_uuid.clone())
                } else {
                    None
                };

                // get the arrangement
                if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                    if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                        let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            riff_arr_automation
                        } else {
                            riff_arrangement.add_track_automation(track_uuid.clone());
                            riff_arrangement.automation_mut(&track_uuid).unwrap()
                        };
                        if automation_discrete {
                            Some(automation.events_mut())
                        } else {
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
                                } else {
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
                        } else {
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
                                } else {
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
}




























pub fn handle_automation_change(state: &mut RiffDAWState, changed_events: Vec<(TrackEvent, TrackEvent)>) {
    match state.automation_view_mode() {
        AutomationViewMode::Controllers => handle_automation_controller_change(state, changed_events),
        AutomationViewMode::PitchBend => handle_automation_pitch_bend_change(state, changed_events),
        AutomationViewMode::Instrument => handle_automation_instrument_change(state, changed_events),
        AutomationViewMode::Effect => handle_automation_effect_change(state, changed_events),
        AutomationViewMode::NoteExpression => handle_automation_note_expression_change(state, changed_events),
        _ => (),
    }
}

pub fn handle_automation_instrument_change(state: &mut RiffDAWState, changed_events: Vec<(TrackEvent, TrackEvent)>) {
    let selected = state.selected_automation().to_vec();
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_view_state.instrument_parameter_type.clone();
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();
    let automation_discrete = state.automation_discrete();

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
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
                        if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                            if let Some(automation) = riff_arrangement.automation_mut(&track_uuid) {
                                if automation_discrete {
                                    Some(automation.events_mut())
                                } else {
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
                                    } else { None }
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
                            } else {
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
                                            id: UuidWrapper::new_v4(),
                                            plugin_uuid: UuidWrapper::new_from_string(plugin_uuid.clone()),
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
                                        } else {
                                            None
                                        }
                                    }
                                } else {
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
}

pub fn handle_automation_note_expression_change(state: &mut RiffDAWState, changed_events: Vec<(TrackEvent, TrackEvent)>) {
    let selected = state.selected_automation().to_vec();
    let automation_type = state.automation_view_state.note_expression_type.clone();
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

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
            if let TrackType::InstrumentTrack(_instrument_track) = track_type {
                let events = if let CurrentView::RiffArrangement = current_view {
                    let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                        Some(selected_arrangement_uuid.clone())
                    } else {
                        None
                    };

                    // get the arrangement
                    if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                        if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                            let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                riff_arr_automation
                            } else {
                                riff_arrangement.add_track_automation(track_uuid.clone());
                                riff_arrangement.automation_mut(&track_uuid).unwrap()
                            };
                            if automation_discrete {
                                Some(automation.events_mut())
                            } else {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) == automation_type {
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
                                                note_expression.port() == note_expression_port_index &&
                                                note_expression.channel() == note_expression_channel &&
                                                note_expression.note_id() == note_expression_id &&
                                                note_expression.key() == note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
                                    } else {
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
                            } else {
                                if let Some(automation_envelope) = automation.envelopes_mut().iter_mut().find(|envelope| {
                                    let mut found = false;
                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                        if *(note_expression.expression_type()) == automation_type {
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
                                                note_expression.port() == note_expression_port_index &&
                                                note_expression.channel() == note_expression_channel &&
                                                note_expression.note_id() == note_expression_id &&
                                                note_expression.key() == note_expression_key
                                            {
                                                found = true;
                                            }
                                        }
                                        return found;
                                    }) {
                                        Some(envelope.events_mut())
                                    } else {
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
}

pub fn handle_automation_effect_change(state: &mut RiffDAWState, changed_events: Vec<(TrackEvent, TrackEvent)>) {
    let selected = state.selected_automation().to_vec();
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_view_state.effect_parameter_type.clone();
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

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
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
                            if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                                let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                    riff_arr_automation
                                } else {
                                    riff_arrangement.add_track_automation(track_uuid.clone());
                                    riff_arrangement.automation_mut(&track_uuid).unwrap()
                                };
                                if automation_discrete {
                                    Some(automation.events_mut())
                                } else {
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
                                                id: UuidWrapper::new_v4(),
                                                plugin_uuid: UuidWrapper::new_from_string(selected_effect_uuid.clone()),
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
                                            } else {
                                                None
                                            }
                                        }
                                    } else {
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
                                } else {
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
                                                id: UuidWrapper::new_v4(),
                                                plugin_uuid: UuidWrapper::new_from_string(selected_effect_uuid.clone()),
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
                                            } else {
                                                None
                                            }
                                        }
                                    } else {
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
}

pub fn handle_automation_controller_change(state: &mut RiffDAWState, changed_events: Vec<(TrackEvent, TrackEvent)>) {
    let selected = state.selected_automation().to_vec();
    let track_uuid = state.selected_track().unwrap_or("".to_string());
    let automation_type = state.automation_view_state.controller_type_index.clone();
    let selected_riff_uuid = if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
        Some(selected_riff_uuid.clone())
    }
    else {
        None
    };
    let current_view = state.current_view().clone();
    let automation_edit_type = state.automation_edit_type();
    let automation_discrete = state.automation_discrete();

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
            let events = if let CurrentView::RiffArrangement = current_view {
                let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                    Some(selected_arrangement_uuid.clone())
                } else {
                    None
                };

                // get the arrangement
                if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                    if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                        let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            riff_arr_automation
                        } else {
                            riff_arrangement.add_track_automation(track_uuid.clone());
                            riff_arrangement.automation_mut(&track_uuid).unwrap()
                        };
                        if automation_discrete {
                            Some(automation.events_mut())
                        } else {
                            if let Some(automation_type_value) = automation_type.as_ref() {
                                let automation_type_value = CONTROLLER_TYPES.get(*automation_type_value as usize).unwrap().0;
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
                                    } else {
                                        None
                                    }
                                }
                            } else {
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
                        } else {
                            if let Some(automation_type_value) = automation_type.as_ref() {
                                let automation_type_value = CONTROLLER_TYPES.get(*automation_type_value as usize).unwrap().0;
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
                                    } else {
                                        None
                                    }
                                }
                            } else {
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
}

pub fn handle_automation_pitch_bend_change(state: &mut RiffDAWState, changed_events: Vec<(TrackEvent, TrackEvent)>) {
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

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(track_type) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
            let events = if let CurrentView::RiffArrangement = current_view {
                let selected_riff_arrangement_uuid = if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                    Some(selected_arrangement_uuid.clone())
                } else {
                    None
                };

                // get the arrangement
                if let Some(selected_arrangement_uuid) = selected_riff_arrangement_uuid {
                    if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(selected_arrangement_uuid.clone()) {
                        let automation = if let Some(riff_arr_automation) = riff_arrangement.automation_mut(&track_uuid) {
                            riff_arr_automation
                        } else {
                            riff_arrangement.add_track_automation(track_uuid.clone());
                            riff_arrangement.automation_mut(&track_uuid).unwrap()
                        };
                        if automation_discrete {
                            Some(automation.events_mut())
                        } else {
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
                                } else {
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
                        } else {
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
                                } else {
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
}

