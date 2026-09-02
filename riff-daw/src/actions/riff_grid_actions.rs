use std::collections::HashMap;
use log::debug;
use uuid::Uuid;
use crate::constants::TRACK_VIEW_TRACK_PANEL_HEIGHT;
use crate::domain::{DAWUtils, RiffGrid, RiffReference};
use crate::event::{DAWEvents, NotificationType, RiffGridChangeType};
use crate::state::RiffDAWState;

pub fn daw_events_RiffGridAdd(state: &mut RiffDAWState, riff_grid_uuid: String, name: String) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let mut riff_grid = RiffGrid::new_with_uuid(Uuid::parse_str(riff_grid_uuid.as_str()).unwrap());
            riff_grid.set_name(name);
            project.song_mut().add_riff_grid(riff_grid);
            // gui.update_available_riff_grids_in_riff_arrangement_blades(&state);
        },
        Err(_) => debug!("Main - rx_ui processing loop - riff grid add - could not get lock on state"),
    };
    // gui.ui.riff_grid_box.queue_draw();
}

// pub fn daw_events_RiffGridDelete(state: &mut RiffDAWState, riff_grid_uuid: String) {
//     // check if any riff grids or arrangements are using this riff - if so then show a warning dialog
//     let found_info = match state.lock() {
//         Ok(state) => {
//             let mut found_info = vec![];
//
//             // check riff arrangements
//             for riff_arrangement in state.project().song().riff_arrangements().iter() {
//                 for riff_item in riff_arrangement.items().iter() {
//                     if let Some(riff_grid) = state.project().song().riff_grid(riff_item.item_uuid().to_string()) {
//                         if riff_grid.uuid() == riff_grid_uuid {
//                             let message = format!("Riff arrangement: \"{}\" has references to riff grid: \"{}\".", riff_arrangement.name(), riff_grid.name());
//
//                             if !found_info.iter().any(|entry| *entry == message) {
//                                 found_info.push(message);
//                             }
//                         }
//                     }
//                 }
//             }
//
//             found_info
//         }
//         Err(_) => {
//             debug!("Main - rx_ui processing loop - riff grid delete - could not get lock on state");
//             vec![]
//         }
//     };
//
//     // if the riff grid is not used then delete it from the project/song
//     if found_info.len() == 0 {
//         match state.lock() {
//             Ok(mut state) => {
//                 // remove the riff grid from the song
//                 state.get_project().song_mut().remove_riff_grid(riff_grid_uuid.clone());
//                 // remove the riff grid from arrangement riff grid pick list
//                 // gui.update_available_riff_grids_in_riff_arrangement_blades(&state);
//                 // remove the riff grid from the grid combobox in the riff grid view
//                 // gui.update_riff_grids_combobox_in_riff_grid_view(&state, false);
//             },
//             Err(_) => debug!("Main - rx_ui processing loop - riff grid delete - could not get lock on state"),
//         };
//         // gui.ui.riff_grid_box.queue_draw();
//     } else {
//         let mut error_message = String::from("Could not delete riff grid:\n");
//
//         for message in found_info.iter() {
//             error_message.push_str(message.as_str());
//             error_message.push_str("\n");
//         }
//
//         let _ = tx_from_ui.send(DAWEvents::Notification(NotificationType::Error, error_message));
//     }
// }

pub fn daw_events_RiffGridSelected(state: &mut RiffDAWState, riff_grid_uuid: String) {
    state.riff_grid_view_state.set_selected_riff_grid_uuid(Some(riff_grid_uuid.clone()));
}

// pub fn daw_events_RiffGridChange(state: &mut RiffDAWState, riff_grid_change_type: RiffGridChangeType, track_uuid: Option<String>) {
//     match riff_grid_change_type {
//         RiffGridChangeType::RiffReferenceAdd{ track_index, position } => {
//             match state.lock() {
//                 Ok(mut state) => {
//                     let mut selected_riff_uuid = None;
//                     let mut track_uuid = None;
//
//                     match state.project().song().tracks().get(track_index as usize) {
//                         Some(track) => {
//                             selected_riff_uuid = state.selected_riff_uuid(track.uuid().to_string());
//                             track_uuid = Some(track.uuid().to_string());
//                         }
//                         None => debug!("Main - rx_ui processing loop - riff grid riff reference added - no track at index."),
//                     }
//
//                     let selected_riff_grid_uuid = state.selected_riff_grid_uuid().clone();
//                     if let Some(selected_riff_grid_uuid) = selected_riff_grid_uuid {
//                         if let Some(track_uuid) = track_uuid {
//                             if let Some(selected_riff_uuid) = selected_riff_uuid {
//                                 match state.get_project().song_mut().riff_grids_mut().iter_mut().find(|riff_grid| riff_grid.uuid().to_string() == selected_riff_grid_uuid.to_string()) {
//                                     Some(riff_grid) => {
//                                         riff_grid.add_riff_reference_to_track(track_uuid, selected_riff_uuid.clone(), position);
//                                     }
//                                     None => debug!("Main - rx_ui processing loop - riff grid riff reference added - no riff grid with uuid."),
//                                 }
//                             }
//                         }
//                     }
//                 },
//                 Err(_) => debug!("Main - rx_ui processing loop - riff grid riff reference added - could not get lock on state"),
//             }
//             // gui.ui.riff_grid_drawing_area.queue_draw();
//         }
//         RiffGridChangeType::RiffReferenceDelete{track_index, position} => {
//             match state.lock() {
//                 Ok(mut state) => {
//                     let mut track_uuid = None;
//                     let mut track_riffs = vec![];
//
//                     match state.project().song().tracks().get(track_index as usize) {
//                         Some(track) => {
//                             track_uuid = Some(track.uuid().to_string());
//                             track_riffs = track.riffs().iter().map(|riff| (riff.id(), riff.length())).collect_vec();
//                         }
//                         None => debug!("Main - rx_ui processing loop - riff grid riff reference deleted - no track at index."),
//                     }
//
//                     if let Some(selected_riff_grid_uuid) = state.selected_riff_grid_uuid().clone() {
//                         if let Some(track_uuid) = track_uuid {
//                             match state.get_project().song_mut().riff_grids_mut().iter_mut().find(|riff_grid| riff_grid.uuid().to_string() == selected_riff_grid_uuid.to_string()) {
//                                 Some(riff_grid) => {
//                                     if let Some(riff_references) = riff_grid.track_riff_references_mut(track_uuid.clone()) {
//                                         riff_references.retain(|riff_ref| {
//                                             let riff_uuid = riff_ref.linked_to();
//                                             let mut retain = true;
//                                             for riff in track_riffs.iter() {
//                                                 if riff.0 == riff_uuid {
//                                                     let riff_length = riff.1;
//                                                     if riff_ref.position() <= position &&
//                                                         position <= (riff_ref.position() + riff_length) {
//                                                         retain = false;
//                                                     } else {
//                                                         retain = true;
//                                                     }
//                                                     break;
//                                                 }
//                                             }
//                                             retain
//                                         });
//                                     }
//                                 }
//                                 None => debug!("Main - rx_ui processing loop - riff grid riff reference deleted - no riff grid with uuid."),
//                             }
//                         }
//                     }
//                 },
//                 Err(_) => debug!("Main - rx_ui processing loop - riff grid riff reference deleted - could not get lock on state"),
//             }
//             // gui.ui.riff_grid_drawing_area.queue_draw();
//         }
//         RiffGridChangeType::RiffReferenceCutSelected => {
//             match state.lock() {
//                 Ok(mut state) => {
//                     let selected_riff_references = state.selected_riff_grid_riff_references().clone();
//                     let selected_riff_grid_uuid = if let Some(selected_riff_grid_uuid) = state.selected_riff_grid_uuid() {
//                         selected_riff_grid_uuid.clone()
//                     }
//                     else {
//                         "".to_string()
//                     };
//                     let edit_cursor_position_in_secs = if let Some(riff_grid_beat_grid) = gui.riff_grid() {
//                         match riff_grid_beat_grid.lock() {
//                             Ok(grid) => {
//                                 grid.edit_cursor_time_in_beats()
//                             },
//                             Err(_) => 0.0,
//                         }
//                     } else {
//                         0.0
//                     };
//                     let mut copy_buffer: Vec<RiffReference> = vec![];
//
//                     if let Some(riff_grid) = state.get_project().song_mut().riff_grid_mut(selected_riff_grid_uuid) {
//                         let track_uuids = riff_grid.tracks().map(|key| key.clone()).collect_vec();
//                         for track_uuid in track_uuids {
//                             if let Some(track_riff_refs) = riff_grid.track_riff_references_mut(track_uuid.clone()) {
//                                 track_riff_refs.retain(|riff_ref| {
//                                     if selected_riff_references.clone().contains(&riff_ref.uuid().to_string()) {
//                                         let mut value = riff_ref.clone();
//                                         value.set_position(value.position() - edit_cursor_position_in_secs);
//                                         value.set_track_id(track_uuid.clone());
//                                         copy_buffer.push(value);
//                                         false
//                                     } else { true }
//                                 });
//                             }
//                         }
//
//                         // gui.ui.riff_grid_drawing_area.queue_draw();
//                     }
//
//                     state.riff_grid_riff_references_copy_buffer_mut().clear();
//                     for riff_ref in copy_buffer.iter() {
//                         state.riff_grid_riff_references_copy_buffer_mut().push(riff_ref.clone());
//                     }
//                 }
//                 Err(_) => debug!("Main - rx_ui processing loop - riff grid riff reference cut - could not get lock on state"),
//             }
//         }
//         RiffGridChangeType::RiffReferenceCopySelected => {
//             match state.lock() {
//                 Ok(mut state) => {
//                     let selected_riff_references = state.selected_riff_grid_riff_references().clone();
//                     let selected_riff_grid_uuid = if let Some(selected_riff_grid_uuid) = state.selected_riff_grid_uuid() {
//                         selected_riff_grid_uuid.clone()
//                     }
//                     else {
//                         "".to_string()
//                     };
//                     let edit_cursor_position_in_secs = if let Some(riff_grid_beat_grid) = gui.riff_grid() {
//                         match riff_grid_beat_grid.lock() {
//                             Ok(grid) => {
//                                 grid.edit_cursor_time_in_beats()
//                             },
//                             Err(_) => 0.0,
//                         }
//                     } else {
//                         0.0
//                     };
//                     let mut copy_buffer: Vec<RiffReference> = vec![];
//
//                     if let Some(riff_grid) = state.get_project().song_mut().riff_grid_mut(selected_riff_grid_uuid) {
//                         let track_uuids = riff_grid.tracks().map(|key| key.clone()).collect_vec();
//                         for track_uuid in track_uuids {
//                             if let Some(track_riff_refs) = riff_grid.track_riff_references_mut(track_uuid.clone()) {
//                                 track_riff_refs.iter().filter(|riff_ref| selected_riff_references.clone().contains(&riff_ref.uuid().to_string())).for_each(|riff_ref| {
//                                     let mut value = riff_ref.clone();
//                                     value.set_position(value.position() - edit_cursor_position_in_secs);
//                                     value.set_track_id(track_uuid.clone());
//                                     copy_buffer.push(value);
//                                 });
//                             }
//                         }
//
//                         // gui.ui.riff_grid_drawing_area.queue_draw();
//                     }
//
//                     state.riff_grid_riff_references_copy_buffer_mut().clear();
//                     for riff_ref in copy_buffer.iter() {
//                         state.riff_grid_riff_references_copy_buffer_mut().push(riff_ref.clone());
//                     }
//                 }
//                 Err(_) => debug!("Main - rx_ui processing loop - riff grid riff reference copy - could not get lock on state"),
//             }
//         }
//         RiffGridChangeType::RiffReferencePaste => {
//             match state.lock() {
//                 Ok(mut state) => {
//                     let selected_riff_grid_uuid = if let Some(selected_riff_grid_uuid) = state.selected_riff_grid_uuid() {
//                         selected_riff_grid_uuid.clone()
//                     }
//                     else {
//                         "".to_string()
//                     };
//                     let edit_cursor_position_in_secs = if let Some(riff_grid_beat_grid) = gui.riff_grid() {
//                         match riff_grid_beat_grid.lock() {
//                             Ok(grid) => {
//                                 grid.edit_cursor_time_in_beats()
//                             },
//                             Err(_) => 0.0,
//                         }
//                     } else {
//                         0.0
//                     };
//                     let mut copy_buffer: Vec<RiffReference> = vec![];
//                     state.riff_grid_riff_references_copy_buffer().iter().for_each(|riff_ref| copy_buffer.push(riff_ref.clone()));
//                     let mut state = state;
//
//                     if let Some(riff_grid) = state.get_project().song_mut().riff_grid_mut(selected_riff_grid_uuid) {
//                         let track_uuids = riff_grid.tracks().map(|key| key.clone()).collect_vec();
//                         for track_uuid in track_uuids {
//                             if let Some(track_riff_refs) = riff_grid.track_riff_references_mut(track_uuid.clone()) {
//                                 let mut copy_buffer_riff_refs_to_remove = vec![];
//                                 for riff_ref in copy_buffer.iter() {
//                                     if track_uuid == riff_ref.track_id() {
//                                         track_riff_refs.push(RiffReference::new(riff_ref.linked_to(), riff_ref.position() + edit_cursor_position_in_secs));
//                                         copy_buffer_riff_refs_to_remove.push(riff_ref.uuid().to_string());
//                                     }
//                                 }
//                                 copy_buffer.retain(|riff_ref| !copy_buffer_riff_refs_to_remove.contains(&riff_ref.uuid().to_string()));
//                             }
//                         }
//
//                         // gui.ui.riff_grid_drawing_area.queue_draw();
//                     }
//                 }
//                 Err(_) => debug!("Main - rx_ui processing loop - riff grid riff reference paste - could not get lock on state"),
//             }
//         }
//         RiffGridChangeType::RiffReferenceChange(change) => {
//             debug!("Main - rx_ui processing loop - riff grid riff reference change.");
//             // just interested in position changes - the changed riff actually refers to riff reference by uuid
//             match state.lock() {
//                 Ok(mut state) => {
//                     let mut snap_position_in_beats = 1.0;
//                     let selected_riff_grid_uuid = if let Some(selected_riff_grid_uuid) = state.selected_riff_grid_uuid() {
//                         selected_riff_grid_uuid.clone()
//                     }
//                     else {
//                         "".to_string()
//                     };
//                     match gui.riff_grid() {
//                         Some(riff_grid) => match riff_grid.lock() {
//                             Ok(grid) => snap_position_in_beats = grid.snap_position_in_beats(),
//                             Err(_) => (),
//                         },
//                         None => (),
//                     }
//
//                     let mut riff_id = "".to_string();
//                     let mut track_id = "".to_string();
//                     if let Some(riff_grid) = state.get_project().song_mut().riff_grid_mut(selected_riff_grid_uuid) {
//                         let track_uuids = { riff_grid.tracks().map(|key| key.to_string()).collect_vec() };
//                         for track_uuid in track_uuids {
//                             for (_, changed_riff) in change.iter() {
//                                 for riff_refs in riff_grid.track_riff_references_mut(track_uuid.to_string()) {
//
//                                     if let Some(riff_ref) = riff_refs.iter_mut().find(|riff_ref| riff_ref.uuid().to_string() == changed_riff.uuid().to_string()) {
//                                         let delta = riff_ref.position() - changed_riff.position();
//
//                                         track_id = track_uuid.clone();
//                                         riff_id = riff_ref.linked_to();
//
//                                         if delta < -0.000001 || delta > 0.000001 {
//                                             let calculated_value = DAWUtils::quantise(changed_riff.position(), snap_position_in_beats, 1.0, false);
//                                             if calculated_value.snapped {
//                                                 riff_ref.set_position(calculated_value.snapped_value);
//                                             }
//                                         }
//                                     }
//                                 }
//                             }
//                         }
//                     }
//
//                     // gui.ui.riff_grid_drawing_area.queue_draw();
//                 }
//                 Err(_) => debug!("Main - rx_ui processing loop - riff grid - riff reference change - could not get lock on state"),
//             }
//         }
//         RiffGridChangeType::RiffReferenceDragCopy(mut new_riff_references_details) => {
//             match state.lock() {
//                 Ok(mut state) => {
//                     let mut snap_position_in_beats = 1.0;
//                     match gui.riff_grid() {
//                         Some(riff_grid) => match riff_grid.lock() {
//                             Ok(grid) => snap_position_in_beats = grid.snap_position_in_beats(),
//                             Err(_) => (),
//                         },
//                         None => (),
//                     }
//
//                     // get the selected riff grid
//                     if let Some(selected_riff_grid_uuid) = state.selected_riff_grid_uuid().clone() {
//                         if let Some(riff_grid) = state.get_project().song_mut().riff_grid_mut(selected_riff_grid_uuid) {
//                             let track_uuids = { riff_grid.tracks().map(|key| key.to_string()).collect_vec() };
//                             for track_uuid in track_uuids {
//                                 // get the original riff ref linked to value
//                                 if let Some(riff_references) = riff_grid.track_riff_references_mut(track_uuid.clone()) {
//                                     let mut unused_changes = vec![];
//                                     for (position, original_riff_ref_uuid) in new_riff_references_details.iter() {
//                                         let linked_to = if let Some(original_riff_ref) = riff_references.iter_mut().find(|riff_ref| riff_ref.id() == original_riff_ref_uuid.clone()) {
//                                             Some(original_riff_ref.linked_to())
//                                         } else {
//                                             None
//                                         };
//                                         if let Some(linked_to) = linked_to {
//                                             let snap_delta = position % snap_position_in_beats;
//                                             let new_position = position - snap_delta;
//                                             if new_position >= 0.0 {
//                                                 let riff_ref = RiffReference::new(linked_to, new_position);
//                                                 riff_references.push(riff_ref);
//                                             }
//                                         }
//                                         else {
//                                             unused_changes.push((*position, original_riff_ref_uuid.clone()));
//                                         }
//                                     }
//
//                                     new_riff_references_details.clear();
//                                     new_riff_references_details.append(&mut unused_changes);
//                                 }
//                             }
//                         }
//                     }
//                 }
//                 Err(_) => debug!("Main - rx_ui processing loop - add new riff reference to riff grid track - could not get lock on state"),
//             }
//             // gui.ui.riff_grid_drawing_area.queue_draw();
//         }
//         RiffGridChangeType::RiffReferencesSelectMultiple(x1, y1, x2, y2, add_to_select) => {
//             debug!("Main - rx_ui processing loop - RiffGridChangeType::RiffReferencesSelectMultiple: x1={}, y1={}, x2={}, y2={}, add_to_select={}", x1, y1, x2, y2, add_to_select);
//             let mut selected = Vec::new();
//             match state.lock() {
//                 Ok(state) => {
//                     let mut state = state;
//                     // get the selected riff grid
//                     if let Some(selected_riff_grid_uuid) = state.selected_riff_grid_uuid().clone() {
//                         let mut riff_lengths = HashMap::new();
//                         let mut track_uuids = vec![];
//                         for track in state.project().song().tracks().iter() {
//                             track_uuids.push(track.uuid().to_string());
//                             for riff in track.riffs().iter() {
//                                 riff_lengths.insert(riff.uuid().to_string(), riff.length());
//                             }
//                         }
//
//                         if let Some(riff_grid) = state.get_project().song_mut().riff_grid_mut(selected_riff_grid_uuid) {
//                             for (index, track_uuid) in track_uuids.iter().enumerate() {
//                                 let track_number = index as i32;
//                                 if y1 < track_number && track_number < y2 {
//                                     if let Some(riff_references) = riff_grid.track_riff_references_mut(track_uuid.clone()) {
//                                         for riff_ref in riff_references.iter_mut() {
//                                             if let Some(riff_length) = riff_lengths.get(&riff_ref.linked_to()) {
//                                                 if x1 <= riff_ref.position() && (riff_ref.position() + riff_length) <= x2 {
//                                                     debug!("Riff grid - Riff ref selected: x1={}, y1={}, x2={}, y2={}, position={}, track={}, length={}", x1, y1, x2, y2, riff_ref.position(), track_uuid.as_str(), riff_length);
//                                                     selected.push(riff_ref.uuid().to_string());
//                                                 }
//                                             }
//                                         }
//                                     }
//                                 }
//                             }
//                         }
//                     }
//
//                     if !selected.is_empty() {
//                         let mut state = state;
//                         if !add_to_select {
//                             state.selected_riff_grid_riff_references_mut().clear();
//                         }
//                         state.selected_riff_grid_riff_references_mut().append(&mut selected);
//                     }
//                     else {
//                         state.selected_riff_grid_riff_references_mut().clear();
//                     }
//                 },
//                 Err(_) => debug!("Main - rx_ui processing loop - riff grid riff references select multiple - could not get lock on state"),
//             }
//             // gui.ui.riff_grid_drawing_area.queue_draw();
//         }
//         RiffGridChangeType::RiffReferencesSelectSingle(x1, y1, add_to_select) => {
//             debug!("Main - rx_ui processing loop - RiffGridChangeType::RiffReferencesSelectSingle: x1={}, y1={}, add_to_select={}", x1, y1, add_to_select);
//             let mut selected = Vec::new();
//             match state.lock() {
//                 Ok(state) => {
//                     let mut state = state;
//                     // get the selected riff grid
//                     if let Some(selected_riff_grid_uuid) = state.selected_riff_grid_uuid().clone() {
//                         let mut riff_lengths = HashMap::new();
//                         let mut track_uuids = vec![];
//                         for track in state.project().song().tracks().iter() {
//                             track_uuids.push(track.uuid().to_string());
//                             for riff in track.riffs().iter() {
//                                 riff_lengths.insert(riff.uuid().to_string(), riff.length());
//                             }
//                         }
//
//                         if let Some(riff_grid) = state.get_project().song_mut().riff_grid_mut(selected_riff_grid_uuid) {
//                             if let Some(track_uuid) = track_uuids.get(y1 as usize) {
//                                 if let Some(riff_references) = riff_grid.track_riff_references_mut(track_uuid.clone()) {
//                                     for riff_ref in riff_references.iter_mut() {
//                                         if let Some(riff_length) = riff_lengths.get(&riff_ref.linked_to()) {
//                                             if riff_ref.position() <= x1 && x1 <= (riff_ref.position() + riff_length) {
//                                                 debug!("Riff grid - Riff ref select single: x1={}, y1={}, position={}, track={}, length={}", x1, y1, riff_ref.position(), track_uuid.as_str(), riff_length);
//                                                 selected.push(riff_ref.uuid().to_string());
//                                                 break;
//                                             }
//                                         }
//                                     }
//                                 }
//                             }
//                         }
//                     }
//
//                     if !selected.is_empty() {
//                         let mut state = state;
//                         if !add_to_select {
//                             state.selected_riff_grid_riff_references_mut().clear();
//                         }
//                         state.selected_riff_grid_riff_references_mut().append(&mut selected);
//                     }
//                     else {
//                         state.selected_riff_grid_riff_references_mut().clear();
//                     }
//                 },
//                 Err(_) => debug!("Main - rx_ui processing loop - riff grid riff references select single - could not get lock on state"),
//             }
//             // gui.ui.riff_grid_drawing_area.queue_draw();
//         }
//         RiffGridChangeType::RiffReferencesDeselectMultiple(x1, y1, x2, y2) => {
//             debug!("Main - rx_ui processing loop - RiffGridChangeType::RiffReferencesDeselectMultiple: x1={}, y1={}, x2={}, y2={}", x1, y1, x2, y2);
//             let mut selected = Vec::new();
//             match state.lock() {
//                 Ok(state) => {
//                     let mut state = state;
//                     // get the selected riff grid
//                     if let Some(selected_riff_grid_uuid) = state.selected_riff_grid_uuid().clone() {
//                         let mut riff_lengths = HashMap::new();
//                         let mut track_uuids = vec![];
//                         for track in state.project().song().tracks().iter() {
//                             track_uuids.push(track.uuid().to_string());
//                             for riff in track.riffs().iter() {
//                                 riff_lengths.insert(riff.uuid().to_string(), riff.length());
//                             }
//                         }
//
//                         if let Some(riff_grid) = state.get_project().song_mut().riff_grid_mut(selected_riff_grid_uuid) {
//                             for (index, track_uuid) in track_uuids.iter().enumerate() {
//                                 let track_number = index as i32;
//                                 if y1 < track_number && track_number < y2 {
//                                     if let Some(riff_references) = riff_grid.track_riff_references_mut(track_uuid.clone()) {
//                                         for riff_ref in riff_references.iter_mut() {
//                                             if let Some(riff_length) = riff_lengths.get(&riff_ref.linked_to()) {
//                                                 if x1 <= riff_ref.position() && (riff_ref.position() + riff_length) <= x2 {
//                                                     debug!("Riff grid - Riff ref deselected: x1={}, y1={}, x2={}, y2={}, position={}, track={}, length={}", x1, y1, x2, y2, riff_ref.position(), track_uuid.as_str(), riff_length);
//                                                     selected.push(riff_ref.uuid().to_string());
//                                                 }
//                                             }
//                                         }
//                                     }
//                                 }
//                             }
//                         }
//                     }
//
//                     if !selected.is_empty() {
//                         let mut state = state;
//                         state.selected_riff_grid_riff_references_mut().retain(|riff_ref_id| !selected.contains(riff_ref_id));
//                     }
//                     else {
//                         state.selected_riff_grid_riff_references_mut().clear();
//                     }
//                 },
//                 Err(_) => debug!("Main - rx_ui processing loop - riff grid riff references deselect multiple - could not get lock on state"),
//             }
//             // gui.ui.riff_grid_drawing_area.queue_draw();
//         }
//         RiffGridChangeType::RiffReferencesDeselectSingle(x1, y1) => {
//             debug!("Main - rx_ui processing loop - RiffGridChangeType::RiffReferencesDeselectSingle: x1={}, y1={}", x1, y1);
//             let mut selected = Vec::new();
//             match state.lock() {
//                 Ok(mut state) => {
//                     // get the selected riff grid
//                     if let Some(selected_riff_grid_uuid) = state.selected_riff_grid_uuid().clone() {
//                         let mut riff_lengths = HashMap::new();
//                         let mut track_uuids = vec![];
//                         for track in state.project().song().tracks().iter() {
//                             track_uuids.push(track.uuid().to_string());
//                             for riff in track.riffs().iter() {
//                                 riff_lengths.insert(riff.uuid().to_string(), riff.length());
//                             }
//                         }
//
//                         if let Some(riff_grid) = state.get_project().song_mut().riff_grid_mut(selected_riff_grid_uuid) {
//                             if let Some(track_uuid) = track_uuids.get(y1 as usize) {
//                                 if let Some(riff_references) = riff_grid.track_riff_references_mut(track_uuid.clone()) {
//                                     for riff_ref in riff_references.iter_mut() {
//                                         if let Some(riff_length) = riff_lengths.get(&riff_ref.linked_to()) {
//                                             if riff_ref.position() <= x1 && x1 <= (riff_ref.position() + riff_length) {
//                                                 debug!("Riff grid - Riff ref select single: x1={}, y1={}, position={}, track={}, length={}", x1, y1, riff_ref.position(), track_uuid.as_str(), riff_length);
//                                                 selected.push(riff_ref.uuid().to_string());
//                                                 break;
//                                             }
//                                         }
//                                     }
//                                 }
//                             }
//                         }
//                     }
//
//                     if !selected.is_empty() {
//                         let mut state = state;
//                         state.selected_riff_grid_riff_references_mut().retain(|riff_ref_id| !selected.contains(riff_ref_id));
//                     }
//                     else {
//                         state.selected_riff_grid_riff_references_mut().clear();
//                     }
//                 }
//                 Err(_) => debug!("Main - rx_ui processing loop - riff grid riff references deselect single - could not get lock on state"),
//             }
//             // gui.ui.riff_grid_drawing_area.queue_draw();
//         }
//         RiffGridChangeType::RiffReferencesSelectAll => {
//             debug!("Main - rx_ui processing loop - RiffGridChangeType::RiffReferencesSelectAll");
//             let mut selected = Vec::new();
//             match state.lock() {
//                 Ok(state) => {
//                     let mut state = state;
//                     // get the selected riff grid
//                     if let Some(selected_riff_grid_uuid) = state.selected_riff_grid_uuid().clone() {
//                         let mut riff_lengths = HashMap::new();
//                         let mut track_uuids = vec![];
//                         for track in state.project().song().tracks().iter() {
//                             track_uuids.push(track.uuid().to_string());
//                             for riff in track.riffs().iter() {
//                                 riff_lengths.insert(riff.uuid().to_string(), riff.length());
//                             }
//                         }
//
//                         if let Some(riff_grid) = state.get_project().song_mut().riff_grid_mut(selected_riff_grid_uuid) {
//                             for track_uuid in track_uuids.iter() {
//                                 if let Some(riff_references) = riff_grid.track_riff_references_mut(track_uuid.clone()) {
//                                     for riff_ref in riff_references.iter_mut() {
//                                         selected.push(riff_ref.uuid().to_string());
//                                     }
//                                 }
//                             }
//                         }
//                     }
//
//                     if !selected.is_empty() {
//                         let mut state = state;
//                         state.selected_riff_grid_riff_references_mut().clear();
//                         state.selected_riff_grid_riff_references_mut().append(&mut selected);
//                     }
//                     else {
//                         state.selected_riff_grid_riff_references_mut().clear();
//                     }
//                 },
//                 Err(_) => debug!("Main - rx_ui processing loop - riff grid riff references select all - could not get lock on state"),
//             }
//             // gui.ui.riff_grid_drawing_area.queue_draw();
//         }
//         RiffGridChangeType::RiffReferencesDeselectAll => {
//             debug!("Main - rx_ui processing loop - RiffGridChangeType::RiffReferencesDeselectAll");
//             match state.lock() {
//                 Ok(mut state) => {
//                     state.selected_riff_grid_riff_references_mut().clear();
//                     // gui.ui.riff_grid_drawing_area.queue_draw();
//                 }
//                 Err(_) => debug!("Main - rx_ui processing loop - riff grid riff references deselect all - could not get lock on state"),
//             }
//         }
//         RiffGridChangeType::RiffReferenceIncrementRiff { track_index, position } => {
//             debug!("Main - rx_ui processing loop - RiffGridChangeType::RiffReferenceIncrementRiff: track_index={}, position={}", track_index, position);
//             match state.lock() {
//                 Ok(mut state) => {
//                     let selected_riff_grid_uuid = state.selected_riff_grid_uuid().clone();
//
//                     // get the track
//                     let track_riff = if let Some(track) = state.get_project().song_mut().tracks_mut().get_mut(track_index as usize) {
//                         let track_uuid = track.uuid().to_string();
//                         let track_name = track.name().to_string();
//                         let riff_ids = track.riffs_mut().iter_mut().map(|riff| (riff.id(), riff.name().to_string())).collect_vec();
//                         let riff_details = track.riffs_mut().iter_mut().map(|riff| (riff.id(), (riff.name().to_string(), riff.length()))).collect::<HashMap<String, (String, f64)>>();
//                         let mut riff_name = None;
//
//                         // need to use the selected riff grid
//                         let track_riff = if let Some(selected_riff_grid_uuid) = selected_riff_grid_uuid {
//                             // find the riff grid
//                             if let Some(riff_grid) = state.get_project().song_mut().riff_grid_mut(selected_riff_grid_uuid) {
//                                 if let Some(riff_grid_track_riff_refs) = riff_grid.track_riff_references_mut(track_uuid.clone()) {
//                                     if let Some(riff_ref) = riff_grid_track_riff_refs.iter_mut().find(|riff_ref| {
//                                         if let Some((name, riff_length)) = riff_details.get(&riff_ref.linked_to()) {
//                                             riff_name = Some(name.to_string());
//                                             let riff_ref_end_position = riff_ref.position() + *riff_length;
//                                             if riff_ref.position() <= position && position <= riff_ref_end_position {
//                                                 true
//                                             }
//                                             else { false }
//                                         }
//                                         else { false }
//                                     }) {
//                                         if let Some(index) = riff_ids.iter().position(|(id, _)| id.clone() == riff_ref.linked_to()) {
//                                             let next_index = if (index + 1) < riff_ids.iter().count() {
//                                                 index + 1
//                                             }
//                                             else { 0 };
//
//                                             if let Some((riff_id, riff_name)) = riff_ids.get(next_index) {
//                                                 riff_ref.set_linked_to(riff_id.clone());
//                                                 // gui.ui.track_drawing_area.queue_draw();
//
//                                                 Some((track_uuid, riff_ref.linked_to(), track_name.to_string(), riff_name.clone()))
//                                             } else { None }
//                                         } else { None }
//                                     } else { None }
//                                 } else { None }
//                             } else { None  }
//                         } else { None };
//
//                         if let Some((track_uuid, riff_uuid, track_name, riff_name)) = &track_riff {
//                             state.set_selected_riff_uuid(track_uuid.clone(), riff_uuid.clone());
//                             state.set_selected_track(Some(track_uuid.clone()));
//                             // gui.set_piano_roll_selected_track_name_label(track_name.as_str());
//                             // gui.set_piano_roll_selected_riff_name_label(riff_name.as_str());
//                             // gui.ui.piano_roll_drawing_area.queue_draw();
//                         }
//
//                         track_riff
//                     } else { None };
//
//                     if let Some(track) = state.project().song().tracks().get(track_index as usize) {
//                         if let Some((track_uuid, riff_uuid, track_name, riff_name)) = track_riff {
//                             if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_uuid.clone()) {
//                                 scroll_notes_into_view(gui, riff);
//                             }
//                         }
//                     }
//                 }
//                 Err(_) => debug!("Main - rx_ui processing loop - RiffGridChangeType::RiffReferenceIncrementRiff - could not get lock on state"),
//             }
//         }
//         RiffGridChangeType::RiffSelectWithTrackIndex{ track_index, position } => {
//             debug!("Main - rx_ui processing loop - RiffGridChangeType::RiffSelectWithTrackIndex");
//             match state.lock() {
//                 Ok(mut state) => {
//                     let selected_riff_grid_uuid = state.selected_riff_grid_uuid().clone();
//
//                     // get the track
//                     let track_riff = if let Some(track) = state.get_project().song_mut().tracks_mut().get_mut(track_index as usize) {
//                         let track_uuid = track.uuid().to_string();
//                         let track_name = track.name().to_string();
//                         let riff_details = track.riffs_mut().iter_mut().map(|riff| (riff.id(), (riff.name().to_string(), riff.length()))).collect::<HashMap<String, (String, f64)>>();
//                         let mut riff_name = None;
//
//                         // need to use the selected riff grid
//                         if let Some(selected_riff_grid_uuid) = selected_riff_grid_uuid {
//                             // find the riff grid
//                             if let Some(riff_grid) = state.get_project().song_mut().riff_grid(selected_riff_grid_uuid) {
//                                 if let Some(riff_grid_track_riff_refs) = riff_grid.track_riff_references(track_uuid.clone()) {
//                                     if let Some(riff_ref) = riff_grid_track_riff_refs.iter().find(|riff_ref| {
//                                         if let Some((name, riff_length)) = riff_details.get(&riff_ref.linked_to()) {
//                                             riff_name = Some(name.to_string());
//                                             let riff_ref_end_position = riff_ref.position() + *riff_length;
//                                             if riff_ref.position() <= position && position <= riff_ref_end_position {
//                                                 true
//                                             }
//                                             else { false }
//                                         }
//                                         else { false }
//                                     }) {
//                                         if let Some(riff_name) = riff_name {
//                                             if riff_name.as_str() != "empty" {
//                                                 Some((track_uuid, riff_ref.linked_to(), track_name.to_string(), riff_name))
//                                             } else { None }
//                                         } else { None }
//                                     } else { None }
//                                 } else { None }
//                             } else { None  }
//                         } else { None }
//                     }
//                     else { None };
//
//                     if let Some((track_uuid, riff_uuid, track_name, riff_name)) = track_riff {
//                         state.set_selected_riff_uuid(track_uuid.clone(), riff_uuid);
//                         state.set_selected_track(Some(track_uuid));
//                         // gui.set_piano_roll_selected_track_name_label(track_name.as_str());
//                         // gui.set_piano_roll_selected_riff_name_label(riff_name.as_str());
//                         // gui.ui.piano_roll_drawing_area.queue_draw();
//                     }
//                 }
//                 Err(_) => debug!("Main - rx_ui processing loop - RiffGridChangeType::RiffSelectWithTrackIndex - could not get lock on state"),
//             }
//         }
//     }
// }
//
// pub fn daw_events_RiffGridNameChange(state: &mut RiffDAWState, name: String) {
//     match state.lock() {
//         Ok(mut state) => {
//             let selected_riff_grid_uuid = if let Some(selected_riff_grid_uuid) = state.selected_riff_grid_uuid() {
//                 selected_riff_grid_uuid.to_string()
//             }
//             else {
//                 "".to_string()
//             };
//             if let Some(riff_grid) = state.get_project().song_mut().riff_grid_mut(selected_riff_grid_uuid) {
//                 riff_grid.set_name(name);
//                 // gui.update_riff_grids_combobox_in_riff_grid_view(&state, true);
//                 // gui.update_available_riff_grids_in_riff_arrangement_blades(&state);
//             }
//         }
//         Err(_) => debug!("Main - rx_ui processing loop - riff grid name change - could not get lock on state"),
//     }
// }

pub fn daw_events_RiffGridPlay(state: &mut RiffDAWState, riff_grid_uuid: String) {
    state.play_riff_grid(riff_grid_uuid.clone());
    state.set_playing_riff_grid(Some(riff_grid_uuid.clone()));
}

// pub fn daw_events_RiffGridCopy(state: &mut RiffDAWState, riff_grid_uuid: String) {
//     match state.lock() {
//         Ok(mut state) => {
//             let mut copied_riff_grid = RiffGrid::new();
//             if let Some(riff_grid) = state.project().song().riff_grid(riff_grid_uuid) {
//                 copied_riff_grid.set_name(format!("Copy of {}", riff_grid.name()));
//                 for track_uuid in riff_grid.tracks() {
//                     for track_riff_ref in riff_grid.track_riff_references(track_uuid.clone()).unwrap().iter() {
//                         copied_riff_grid.add_riff_reference_to_track(track_uuid.clone(), track_riff_ref.linked_to(), track_riff_ref.position());
//                     }
//                 }
//             }
//             state.set_selected_riff_grid_uuid(Some(copied_riff_grid.uuid()));
//             state.get_project().song_mut().add_riff_grid(copied_riff_grid);
//             // gui.update_available_riff_grids_in_riff_arrangement_blades(&state);
//             // gui.update_riff_grids_combobox_in_riff_grid_view(&state, false);
//         }
//         Err(_) => debug!("Main - rx_ui processing loop - riff grid copy - could not get lock on state"),
//     }
// }
//
// pub fn daw_events_RiffGridCopySelectedToTrackViewCursorPosition(state: &mut RiffDAWState, uuid: String) {
//     // get the current track cursor position and convert it to beats
//     let edit_cursor_position_in_beats = match &gui.track_grid {
//         Some(track_grid) => match track_grid.lock() {
//             Ok(grid) => grid.edit_cursor_time_in_beats(),
//             Err(_) => 0.0
//         },
//         None => 0.0
//     };
//
//     DAWUtils::copy_riff_grid_to_position(uuid, edit_cursor_position_in_beats, state.clone());
// }
//
// pub fn daw_events_RiffGridVerticalScaleChanged(state: &mut RiffDAWState, vertical_scale: f64) {
//
//     let widget_height = (TRACK_VIEW_TRACK_PANEL_HEIGHT as f64 * vertical_scale) as i32;
//     for track_panel in gui.ui.riff_grid_track_panel.children().iter_mut() {
//         debug!("Riff grid - Track panel height: {}", track_panel.allocation().height);
//         track_panel.set_height_request(widget_height);
//     }
//     // gui.ui.track_panel_scrolled_window.queue_draw();
//     // gui.ui.riff_grid_track_panel.queue_draw();
//     // gui.ui.riff_grid_drawing_area.queue_draw();
// }
