use crate::state::RiffDAWState;
use crate::views::{riff_grid_selector};
use xilem::view::FlexSequence;
use crate::actions::{daw_events_RiffArrangementRiffItemAdd};
use crate::domain::RiffItemType;
use crate::icons::ICON_PLUS;

fn up_change_callback(state: &mut RiffDAWState) {
    let mut new_index: i32 = state.riff_arrangement_view_state.add_to_arr_riff_grid_index as i32 - 1;
    if new_index < 0 {
        new_index = 0;
    }
    state.riff_arrangement_view_state.add_to_arr_riff_grid_index = new_index as usize;
}

fn down_change_callback(state: &mut RiffDAWState) {
    if let Ok(project) = state.project.lock() {
        let mut new_index = state.riff_arrangement_view_state.add_to_arr_riff_grid_index + 1;
        if new_index >= project.song.riff_grids().len() {
            new_index = 0;
        }
        state.riff_arrangement_view_state.add_to_arr_riff_grid_index = new_index;
    }
}

fn select_change_callback(state: &mut RiffDAWState) {
    let mut riff_grid_uuid = None;
    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(riff_grid) = project.song.riff_grids().get(state.riff_arrangement_view_state.add_to_arr_riff_grid_index.clone()) {
            riff_grid_uuid = Some(riff_grid.uuid().clone());
        }
    }
    if let Some(riff_arr_uuid) = state.riff_arrangement_view_state.selected_riff_arrangement_uuid.as_ref() {
        if let Some(riff_grid_uuid) = riff_grid_uuid {
            daw_events_RiffArrangementRiffItemAdd(state, riff_arr_uuid.clone(), riff_grid_uuid, RiffItemType::RiffGrid);
        }
    }
}

pub fn riff_arr_view_riff_grid_selector(data: &RiffDAWState, active_index: usize) -> impl FlexSequence<RiffDAWState> + use<> {
    riff_grid_selector(data, active_index, up_change_callback, down_change_callback, select_change_callback, ICON_PLUS.to_string())
}

