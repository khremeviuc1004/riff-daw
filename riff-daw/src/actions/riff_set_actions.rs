use log::debug;
use uuid::Uuid;
use crate::domain::{DAWUtils, RiffItemType, RiffReference, RiffSet, Track};
use crate::event::{DAWEvents, NotificationType};
use crate::state::RiffDAWState;

pub fn daw_events_RiffSetMoveToPosition(state: &mut RiffDAWState, riff_set_uuid: String, to_position_in_container: usize) {
    debug!("Main - rx_ui processing loop - riff set move to position: {}", riff_set_uuid.as_str());
    match state.get_project().lock().as_mut() {
        Ok(mut project) => {
            project.song_mut().riff_set_move_to_position(riff_set_uuid, to_position_in_container);
        },
        Err(_) => debug!("Main - rx_ui processing loop - riff set move to position - could not get lock on state"),
    };
}

pub fn daw_events_RiffSetSelect(state: &mut RiffDAWState, riff_set_uuid: String, selected: bool) {
    debug!("Main - rx_ui processing loop - riff set selected uuid={}, selected={}", riff_set_uuid.as_str(), selected);
    match state.get_project().lock().as_mut() {
        Ok(mut project) => {
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

pub fn daw_events_RiffSetAdd(state: &mut RiffDAWState, uuid: Uuid, name: String) {
    match state.get_project().lock().as_mut() {
        Ok(mut project) => {
            let selected_riff_set_uuid = if let Some(selected_riff_set_uuid) = state.riff_set_selected_uuid() {
                Some(selected_riff_set_uuid.to_string())
            }
            else { None};
            let song = project.song_mut();
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
        },
        Err(_) => (),
    }
}

pub fn daw_events_RiffSetDelete(state: &mut RiffDAWState, uuid: String) {
    // check if any riff sequences or arrangements are using this riff - if so then show a warning dialog
    let found_info = match state.get_project().lock().as_mut() {
        Ok(project) => {
            let mut found_info = vec![];

            // check riff sequences
            for riff_sequence in project.song().riff_sequences().iter() {
                for riff_set_item in riff_sequence.riff_sets().iter() {
                    if let Some(riff_set) = project.song().riff_set(riff_set_item.item_uuid().to_string()) {
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
            for riff_arrangement in project.song().riff_arrangements().iter() {
                for riff_item in riff_arrangement.items().iter() {
                    match *(riff_item.item_type()) {
                        RiffItemType::RiffSet => {
                            if let Some(riff_set) = project.song().riff_set(riff_item.item_uuid().to_string()) {
                                if riff_set.uuid() == uuid {
                                    let message = format!("Riff arrangement: \"{}\" has references to riff set: \"{}\".", riff_arrangement.name(), riff_set.name());

                                    if !found_info.iter().any(|entry| *entry == message) {
                                        found_info.push(message);
                                    }
                                }
                            }
                        }
                        RiffItemType::RiffSequence => {
                            if let Some(riff_sequence) = project.song().riff_sequence(riff_item.uuid()) {
                                for riff_set_item in riff_sequence.riff_sets().iter() {
                                    if let Some(riff_set) = project.song().riff_set(riff_set_item.item_uuid().to_string()) {
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

    // if no riff sequence or riff arrangement is using this riff set then delete it from the project/song
    if found_info.len() == 0 {
        match state.get_project().lock().as_mut() {
            Ok(mut project) => {
                let song = project.song_mut();
                // remove the riff set from the song
                song.remove_riff_set(uuid.clone());
            },
            Err(_) => (),
        }
    } else {
        let mut error_message = String::from("Could not delete riff set:\n");

        for message in found_info.iter() {
            error_message.push_str(message.as_str());
            error_message.push_str("\n");
        }

        let res = rfd::MessageDialog::new()
            .set_title("Error")
            .set_description(error_message)
            .set_buttons(rfd::MessageButtons::Ok)
            .set_level(rfd::MessageLevel::Error)
            .show();
    }
}

pub fn daw_events_RiffSetCopy(state: &mut RiffDAWState, uuid: String, new_copy_riff_set_uuid: Uuid) {
    match state.get_project().lock().as_mut() {
        Ok(mut project) => {
            project.song_mut().riff_set_copy(uuid, new_copy_riff_set_uuid.clone());
        },
        Err(_) => (),
    }
}

pub fn daw_events_RiffSetNameChange(state: &mut RiffDAWState, uuid: String, name: String) {
    match state.get_project().lock().as_mut() {
        Ok(mut project) => {
            let song = project.song_mut();
            match song.riff_set_mut(uuid.clone()) {
                Some(riff_set) => riff_set.set_name(name.clone()),
                None => debug!("Could not find the riff set to change the name of."),
            }
        },
        Err(error) => debug!("Could not lock the state when trying to change a riff set name: {}", error),
    }
}

pub fn daw_events_RiffSetPlay(state: &mut RiffDAWState, uuid: String) {
    debug!("Main - rx_ui processing loop - riff set play: {}", uuid);
    state.play_riff_set(uuid);
}

pub fn daw_events_RiffSetTrackIncrementRiff(state: &mut RiffDAWState, riff_set_uuid: String, track_uuid: String) {
    debug!("Main - rx_ui processing loop - riff set track incr riff: {}, {}", riff_set_uuid.as_str(), track_uuid.as_str());
    state.riff_set_increment_riff_for_track(riff_set_uuid.clone(), track_uuid.clone());
    if let Some(playing_riff_set_uuid) = state.playing_riff_set_mut() {
        if *playing_riff_set_uuid == riff_set_uuid {
            state.play_riff_set_update_track_as_riff(riff_set_uuid.clone(), track_uuid.clone());
        }
    }
}

pub fn daw_events_RiffSetTrackSetRiff(state: &mut RiffDAWState, riff_set_uuid: String, track_uuid: String, riff_uuid: String) {
    debug!("Main - rx_ui processing loop - riff set track set riff: riff set={}, track={}, riff={}", riff_set_uuid.as_str(), track_uuid.as_str(), riff_uuid.as_str());
    state.riff_set_riff_for_track(riff_set_uuid, track_uuid, riff_uuid);
    // gui.ui.riff_sets_box.queue_draw();
}

pub fn daw_events_RiffSetCopySelectedToTrackViewCursorPosition(state: &mut RiffDAWState, uuid: String) {
    // get the current track cursor position and convert it to beats
    DAWUtils::copy_riff_set_to_position(uuid, state.track_grid_state.track_grid_edit_cursor_position, state);
}
