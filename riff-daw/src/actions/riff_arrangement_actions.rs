use log::debug;
use uuid::Uuid;
use crate::domain::{DAWItemLength, DAWUtils, RiffArrangement, RiffItem, RiffItemType, Track};
use crate::event::DAWEvents;
use crate::state::RiffDAWState;

pub fn daw_events_RiffArrangementPlay(state: &mut RiffDAWState, riff_arrangement_uuid: String) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let selected_riff_arrangement_play_position = if let Some(riff_arrangement) = project.song().riff_arrangement(riff_arrangement_uuid.clone()) {
                // let selected_riff_item_index = gui.get_selected_riff_arrangement_play_position();
                let mut play_position_in_beats = 0.0;
                for (index, riff_item) in riff_arrangement.items().iter().enumerate() {
                    // if index >= selected_riff_item_index {
                    //     break;
                    // }
                    // FIXME riff set lengths need to be determined using the lowest common factor not straight up find the largest (the largest may not be the actual lowest common factor)
                    if let RiffItemType::RiffSet = riff_item.item_type() {
                        // grab the item
                        if let Some(riff_set) = project.song().riff_set(riff_item.item_uuid().to_string()) {
                            let mut riff_lengths = vec![];
                            for (track_uuid, riff_set_uuid) in riff_set.riff_refs().iter().map(|(track_uuid, value)| (track_uuid.to_string(), value.linked_to().to_string())).collect::<Vec<(String, String)>>().iter() {
                                if let Some(track) = project.song().track(track_uuid.to_string()) {
                                    if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == *riff_set_uuid) {
                                        riff_lengths.push(riff.length() as i32);
                                    }
                                }
                            }
                            let (product, unique_riff_lengths) = RiffDAWState::get_length_product(riff_lengths);
                            play_position_in_beats += RiffDAWState::get_lowest_common_factor(unique_riff_lengths, product) as f64;
                        }
                    }
                    else if let Some(riff_sequence) = project.song().riff_sequence(riff_item.item_uuid().to_string()) {
                        for riff_item in riff_sequence.riff_sets().iter() {
                            if let Some(riff_set) = project.song().riff_set(riff_item.item_uuid().to_string()) {
                                let mut riff_lengths = vec![];
                                for (track_uuid, riff_set_uuid) in riff_set.riff_refs().iter().map(|(track_uuid, value)| (track_uuid.to_string(), value.linked_to().to_string())).collect::<Vec<(String, String)>>().iter() {
                                    if let Some(track) = project.song().track(track_uuid.to_string()) {
                                        if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == *riff_set_uuid) {
                                            riff_lengths.push(riff.length() as i32);
                                        }
                                    }
                                }
                                let (product, unique_riff_lengths) = RiffDAWState::get_length_product(riff_lengths);
                                play_position_in_beats += RiffDAWState::get_lowest_common_factor(unique_riff_lengths, product) as f64;
                            }
                        }
                    }
                    else if let Some(riff_grid) = project.song().riff_grid(riff_item.item_uuid().to_string()) {
                        play_position_in_beats += DAWUtils::get_riff_grid_length(&riff_grid, &state);
                    }
                }
                play_position_in_beats
            }
            else {
                0.0
            };
            state.play_riff_arrangement(riff_arrangement_uuid.clone(), selected_riff_arrangement_play_position);
            state.set_playing_riff_arrangement(Some(riff_arrangement_uuid.clone()));
            if let Some(playing_riff_arrangement_summary_data) = state.playing_riff_arrangement_summary_data() {
                // gui.repaint_riff_arrangement_view_active_drawing_areas(&riff_arrangement_uuid, 0.0, playing_riff_arrangement_summary_data);
            }
        },
        Err(_) => debug!("Main - rx_ui processing loop - riff arrangement play - could not get lock on state"),
    }
}

pub fn daw_events_RiffArrangementAdd(state: &mut RiffDAWState, riff_arrangement_uuid: Uuid) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let mut arrangement = RiffArrangement::new_with_uuid(riff_arrangement_uuid);
            arrangement.set_name(state.riff_arrangement_view_state.add_riff_arrangement_name.clone());

            // add an automation object for each track
            for track in project.song().tracks().iter() {
                arrangement.add_track_automation(track.uuid().to_string());
            }

            project.song_mut().add_riff_arrangement(arrangement);
        },
        Err(_) => debug!("Main - rx_ui processing loop - riff arrangement add - could not get lock on state"),
    }
}

pub fn daw_events_RiffArrangementDelete(state: &mut RiffDAWState, riff_arrangement_uuid: String) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            // remove the riff arrangement from the song
            project.song_mut().remove_riff_arrangement(riff_arrangement_uuid);
        },
        Err(_) => debug!("Main - rx_ui processing loop - riff arrangement delete - could not get lock on state"),
    }
}

pub fn daw_events_RiffArrangementSelected(state: &mut RiffDAWState, riff_arrangement_uuid: String) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            state.set_selected_riff_arrangement_uuid(Some(riff_arrangement_uuid));
        },
        Err(_) => debug!("Main - rx_ui processing loop - riff arrangement selected - could not get lock on state"),
    }
}

pub fn daw_events_RiffArrangementNameChange(state: &mut RiffDAWState, riff_arrangement_uuid: String, name: String) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(riff_arrangement_uuid) {
                riff_arrangement.set_name(name);
                // gui.update_riff_arrangements_combobox_in_riff_arrangement_view(&state, true);
            }
        },
        Err(_) => debug!("Main - rx_ui processing loop - riff arrangement name change - could not get lock on state"),
    }
}

pub fn daw_events_RiffArrangementMoveRiffItemToPosition(state: &mut RiffDAWState, riff_arrangement_uuid: String, riff_item_compound_uuid: String, position: usize) {
    debug!("Main - rx_ui processing loop - riff arrangement={} move riff set={} to position={}", riff_arrangement_uuid.as_str(), riff_item_compound_uuid.as_str(), position);
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            project.song_mut().riff_arrangement_move_riff_item_to_position(riff_arrangement_uuid, riff_item_compound_uuid, position);
        },
        Err(_) => debug!("Main - rx_ui processing loop - riff arrangement move riff set to position - could not get lock on state"),
    };
}

pub fn daw_events_RiffArrangementRiffItemAdd(state: &mut RiffDAWState, riff_arrangement_uuid: String, item_referred_to_uuid: String, riff_item_type: RiffItemType) {
    debug!("Main - rx_ui processing loop - riff arrangement={} - riff item add: {}, {}, {}", riff_arrangement_uuid.as_str(), riff_arrangement_uuid.as_str(), item_referred_to_uuid.as_str(), match riff_item_type.clone() { RiffItemType::RiffSet => { "RiffSet" } RiffItemType::RiffSequence => {"RiffSequence"} RiffItemType::RiffGrid => {"RiffGrid"}} );
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let item_uuid = Uuid::new_v4();
            let selected_riff_item_details = if let Some(selected_riff_item_uuid) = state.riff_arrangement_view_state.riff_arrangement_riff_item_selected_uuid() {
                Some(selected_riff_item_uuid.clone())
            }
            else { None};
            let selected_riff_item_position = if let Some(riff_arrangement) = project.song().riff_arrangements().iter().find(|riff_arrangement| riff_arrangement.uuid() == riff_arrangement_uuid) {
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
                if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(riff_arrangement_uuid.clone()) {
                    riff_arrangement.add_item_at_position(RiffItem::new_with_uuid_string(item_uuid.to_string(), riff_item_type.clone(), item_referred_to_uuid.clone()), selected_riff_item_position + 1);
                }
            }
            else {
                if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(riff_arrangement_uuid.clone()) {
                    riff_arrangement.add_item(RiffItem::new_with_uuid_string(item_uuid.to_string(), riff_item_type.clone(), item_referred_to_uuid.clone()));
                }
            }
        },
        Err(_) => debug!("Main - rx_ui processing loop - riff arrangement - riff item add - could not get lock on state"),
    }
}

pub fn daw_events_RiffArrangementRiffItemDelete(state: &mut RiffDAWState, riff_arrangement_uuid: String, item_uuid: String) {
    debug!("Main - rx_ui processing loop - riff arrangement={} - riff item delete: {}", riff_arrangement_uuid.as_str(), item_uuid.as_str());
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            if let Some(riff_arrangement) = project.song_mut().riff_arrangement_mut(riff_arrangement_uuid) {
                riff_arrangement.remove_item(item_uuid);
            }
        },
        Err(_) => debug!("Main - rx_ui processing loop - riff arrangement - riff item delete - could not get lock on state"),
    }
}

pub fn daw_events_RiffArrangementCopySelectedToTrackViewCursorPosition(state: &mut RiffDAWState, uuid: String) {
    // get the current track cursor position and convert it to beats
    DAWUtils::copy_riff_arrangement_to_position(uuid, state.track_grid_state.track_grid_edit_cursor_position, state);
}

pub fn daw_events_RiffArrangementRiffItemSelect(state: &mut RiffDAWState, riff_arrangement_uuid: String, riff_item_uuid: String, selected: bool) {
    debug!("Main - rx_ui processing loop - riff arrangement={} riff item reference selected uuid={}, selected={}", riff_arrangement_uuid.as_str(), riff_item_uuid.as_str(), selected);
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let mut set_selection_to_none = false;
            if selected {
                state.riff_arrangement_view_state.set_riff_arrangement_riff_item_selected_uuid(Some((riff_arrangement_uuid, riff_item_uuid)));
            }
            else if let Some((riff_arrangement_uuid_selected, riff_item_uuid_selected)) = state.riff_arrangement_view_state.riff_arrangement_riff_item_selected_uuid() {
                if riff_arrangement_uuid == *riff_arrangement_uuid_selected && riff_item_uuid == *riff_item_uuid_selected {
                    set_selection_to_none = true;
                }
            }
            if set_selection_to_none {
                state.riff_arrangement_view_state.set_riff_arrangement_riff_item_selected_uuid(None);
            }
        },
        Err(_) => debug!("Main - rx_ui processing loop - riff arrangement riff item reference selected uuid - could not get lock on state"),
    }
}

pub fn daw_events_RiffArrangementCopy(state: &mut RiffDAWState, uuid: String) {
    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(riff_arrangement) = project.song_mut().riff_arrangement(uuid) {
            let mut new_riff_arrangement = riff_arrangement.clone();
            let mut new_name = "Copy of ".to_string();

            new_name.push_str(new_riff_arrangement.name());
            new_riff_arrangement.set_name(new_name);
            new_riff_arrangement.set_uuid(Uuid::new_v4());

            state.set_selected_riff_arrangement_uuid(Some(new_riff_arrangement.uuid()));
            project.song_mut().add_riff_arrangement(new_riff_arrangement);
        }
    }
}

pub fn daw_events_RiffArrangementToggleOverview(state: &mut RiffDAWState, show: bool) {
    if show {
        // gui.ui.riff_arrangement_overview_drawing_area.show();
    }
    else {
        // gui.ui.riff_arrangement_overview_drawing_area.hide();
    }
}
