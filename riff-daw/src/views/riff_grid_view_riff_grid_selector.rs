use crate::state::RiffDAWState;
use crate::views::{riff_grid_selector};
use xilem::view::FlexSequence;
use crate::actions::{daw_events_RiffGridSelected};
use crate::icons::ICON_LAYERS_SELECTED_BOTTOM;

fn up_change_callback(state: &mut RiffDAWState) {
    let mut new_index: i32 = state.riff_grid_view_state.riff_grid_to_select_index as i32 - 1;
    if new_index < 0 {
        new_index = 0;
    }
    state.riff_grid_view_state.riff_grid_to_select_index = new_index as usize;
}

fn down_change_callback(state: &mut RiffDAWState) {
    if let Ok(project) = state.project.lock() {
        let mut new_index = state.riff_grid_view_state.riff_grid_to_select_index + 1;
        if new_index >= project.song.riff_grids().len() {
            new_index = 0;
        }
        state.riff_grid_view_state.riff_grid_to_select_index = new_index;
    }
}

fn select_change_callback(state: &mut RiffDAWState) {
    let mut riff_seq_uuid = None;

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(riff_seq) = project.song.riff_grids().get(state.riff_grid_view_state.riff_grid_to_select_index.clone()) {
            riff_seq_uuid = Some(riff_seq.uuid().clone());
        }
    }

    if let Some(riff_seq_uuid) = riff_seq_uuid.as_ref() {
        daw_events_RiffGridSelected(state, riff_seq_uuid.clone());
    }
}

pub fn riff_grid_view_riff_grid_selector(data: &RiffDAWState, active_index: usize) -> impl FlexSequence<RiffDAWState> + use<> {
    riff_grid_selector(data, active_index, up_change_callback, down_change_callback, select_change_callback, ICON_LAYERS_SELECTED_BOTTOM.to_string())
}

