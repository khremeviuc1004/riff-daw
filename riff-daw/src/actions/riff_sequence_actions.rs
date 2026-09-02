use log::debug;
use uuid::Uuid;
use crate::domain::{DAWUtils, RiffSequence, Track};
use crate::event::{DAWEvents, NotificationType};
use crate::state::RiffDAWState;

pub fn daw_events_RiffSequencePlay(state: &mut RiffDAWState, riff_sequence_uuid: String) {
    state.play_riff_sequence(riff_sequence_uuid.clone());
    state.set_playing_riff_sequence(Some(riff_sequence_uuid.clone()));
    if let Some(playing_riff_sequence_summary_data) = state.playing_riff_sequence_summary_data() {
        // gui.repaint_riff_sequence_view_riff_sequence_active_drawing_areas(&riff_sequence_uuid, 0.0, playing_riff_sequence_summary_data);
    }
}

pub fn daw_events_RiffSequenceAdd(state: &mut RiffDAWState, riff_sequence_uuid: String) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let mut riff_seq = RiffSequence::new_with_uuid(Uuid::parse_str(riff_sequence_uuid.as_str()).unwrap());
            riff_seq.set_name(state.riff_sequence_view_state.add_riff_sequence_name.clone());
            project.song_mut().add_riff_sequence(riff_seq);
            // gui.update_available_riff_sequences_in_riff_arrangement_blades(&state);
        },
        Err(_) => debug!("Main - rx_ui processing loop - riff sequence add - could not get lock on state"),
    }
}

pub fn daw_events_RiffSequenceCopy(state: &mut RiffDAWState, uuid: String) {
    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(riff_sequence) = project.song_mut().riff_sequence(uuid) {
            let mut new_riff_sequence = riff_sequence.clone();
            let new_name = format!("Copy of {}", new_riff_sequence.name());

            new_riff_sequence.set_name(new_name);
            new_riff_sequence.set_uuid(Uuid::new_v4());

            state.riff_sequence_view_state.set_selected_riff_sequence_uuid(Some(new_riff_sequence.uuid()));
            project.song_mut().add_riff_sequence(new_riff_sequence);
        }
    }
}

pub fn daw_events_RiffSequenceDelete(state: &mut RiffDAWState, riff_sequence_uuid: String) {
    // check if any riff sequences or arrangements are using this riff - if so then show a warning dialog
    let found_info = match state.get_project().lock().as_mut() {
        Ok(project) => {
            let mut found_info = vec![];

            // check riff arrangements
            for riff_arrangement in project.song().riff_arrangements().iter() {
                for riff_item in riff_arrangement.items().iter() {
                    if let Some(riff_sequence) = project.song().riff_sequence(riff_item.item_uuid().to_string()) {
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
        match state.get_project().lock().as_mut() {
            Ok(project) => {
                // remove the riff sequence from the song
                project.song_mut().remove_riff_sequence(riff_sequence_uuid.clone());
            },
            Err(_) => debug!("Main - rx_ui processing loop - riff sequence delete - could not get lock on state"),
        };
    } else {
        let mut error_message = String::from("Could not delete riff sequence:\n");

        for message in found_info.iter() {
            error_message.push_str(message.as_str());
            error_message.push_str("\n");
        }

        let res = rfd::MessageDialog::new()
            .set_title("Error")
            .set_description(error_message)
            .set_buttons(rfd::MessageButtons::OkCancel)
            .set_level(rfd::MessageLevel::Error)
            .show();
    }
}

pub fn daw_events_RiffSequenceNameChange(state: &mut RiffDAWState, riff_sequence_uuid: String, name: String) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            if let Some(riff_sequence) = project.song_mut().riff_sequence_mut(riff_sequence_uuid) {
                riff_sequence.set_name(name);
            }
        },
        Err(_) => debug!("Main - rx_ui processing loop - riff sequence name change - could not get lock on state"),
    }
}

pub fn daw_events_RiffSequenceSelected(state: &mut RiffDAWState, riff_sequence_uuid: String) {
    state.riff_sequence_view_state.set_selected_riff_sequence_uuid(Some(riff_sequence_uuid));
}

pub fn daw_events_RiffSequenceRiffSetAdd(state: &mut RiffDAWState, riff_sequence_uuid: String, riff_set_uuid: String, riff_set_reference_uuid: Uuid) {
    debug!("Main - rx_ui processing loop - riff sequence - riff set add: {}, {}", riff_sequence_uuid.as_str(), riff_set_uuid.as_str());
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let selected_riff_set_instance_details = if let Some(selected_riff_set_uuid) = state.riff_sequence_riff_set_reference_selected_uuid() {
                Some(selected_riff_set_uuid.clone())
            }
            else { None};
            let selected_riff_set_position = if let Some(riff_sequence) = project.song().riff_sequences().iter().find(|riff_sequence| riff_sequence.uuid() == riff_sequence_uuid) {
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
                if let Some(riff_sequence) = project.song_mut().riff_sequence_mut(riff_sequence_uuid.clone()) {
                    riff_sequence.add_riff_set_at_position(riff_set_reference_uuid, riff_set_uuid.clone(), selected_riff_set_position + 1);
                }
            }
            else {
                if let Some(riff_sequence) = project.song_mut().riff_sequence_mut(riff_sequence_uuid.clone()) {
                    riff_sequence.add_riff_set(riff_set_reference_uuid, riff_set_uuid.clone());
                }
            }
            let riff_set_name = if let Some(riff_set) = project.song().riff_sets().iter().find(|riff_set| riff_set.uuid() == riff_set_uuid.clone()) {
                riff_set.name().to_string()
            }
            else {
                "".to_string()
            };
            let track_uuids: Vec<String> = project.song().tracks().iter().map(|track| track.uuid().to_string()).collect();
            // gui.add_riff_sequence_riff_set_blade(
            // tx_from_ui,
            // riff_sequence_uuid,
            // riff_set_reference_uuid.to_string(),
            // riff_set_uuid,
            // track_uuids,
            // // gui.selected_style_provider.clone(),
            // riff_set_name,
            // state_arc,
            // );
        },
        Err(_) => debug!("Main - rx_ui processing loop - riff sequence - riff set add - could not get lock on state"),
    }
}

pub fn daw_events_RiffSequenceRiffSetDelete(state: &mut RiffDAWState, riff_sequence_uuid: String, riff_set_reference_uuid: String) {
    debug!("Main - rx_ui processing loop - riff sequence - riff sequence delete: {}, {}", riff_sequence_uuid.as_str(), riff_set_reference_uuid.as_str());
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            if let Some(riff_sequence) = project.song_mut().riff_sequence_mut(riff_sequence_uuid.clone()) {
                // remove the riff item referencing a riff set from the riff sequence
                riff_sequence.remove_riff_set(riff_set_reference_uuid);
            }
        },
        Err(_) => debug!("Main - rx_ui processing loop - riff sequence - riff set delete - could not get lock on state"),
    }
}

pub fn daw_events_RiffSequenceRiffSetMoveLeft(state: &mut RiffDAWState, riff_sequence_uuid: String, riff_set_reference_uuid: String) {
    debug!("Main - rx_ui processing loop - riff sequence - riff set reference move left: {}, {}", riff_sequence_uuid.as_str(), riff_set_reference_uuid.as_str());
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            if let Some(riff_sequence) = project.song_mut().riff_sequence_mut(riff_sequence_uuid) {
                riff_sequence.riff_set_move_left(riff_set_reference_uuid);
            }
        },
        Err(_) => debug!("Main - rx_ui processing loop - riff sequence - riff set reference move left - could not get lock on state"),
    }
}

pub fn daw_events_RiffSequenceRiffSetMoveRight(state: &mut RiffDAWState, riff_sequence_uuid: String, riff_set_uuid: String) {
    debug!("Main - rx_ui processing loop - riff sequence - riff set reference move right: {}, {}", riff_sequence_uuid.as_str(), riff_set_uuid.as_str());
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            if let Some(riff_sequence) = project.song_mut().riff_sequence_mut(riff_sequence_uuid) {
                riff_sequence.riff_set_move_right(riff_set_uuid);
            }
        },
        Err(_) => debug!("Main - rx_ui processing loop - riff sequence - riff set reference move right - could not get lock on state"),
    }
}

pub fn daw_events_RiffSequenceCopySelectedToTrackViewCursorPosition(state: &mut RiffDAWState, uuid: String) {
    DAWUtils::copy_riff_sequence_to_position(uuid, state.track_grid_state.track_grid_edit_cursor_position, state);
}

pub fn daw_events_RiffSequenceRiffSetSelect(state: &mut RiffDAWState, riff_sequence_uuid: String, riff_set_reference_uuid: String, selected: bool) {
    debug!("Main - rx_ui processing loop - riff sequence={} riff set reference selected uuid={}, selected={}", riff_sequence_uuid.as_str(), riff_set_reference_uuid.as_str(), selected);
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
}

pub fn daw_events_RiffSequenceRiffSetMoveToPosition(state: &mut RiffDAWState, riff_sequence_uuid: String, riff_set_uuid: String, to_position_in_container: usize) {
    debug!("Main - rx_ui processing loop - riff sequence riff set move to position: {}", riff_set_uuid.as_str());
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            project.song_mut().riff_sequence_riff_set_move_to_position(riff_sequence_uuid, riff_set_uuid, to_position_in_container);
        },
        Err(_) => debug!("Main - rx_ui processing loop - riff sequence riff set move to position - could not get lock on state"),
    }
}
