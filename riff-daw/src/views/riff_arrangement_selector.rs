use masonry::properties::types::AsUnit;
use crate::state::RiffDAWState;
use crate::views::{icon};
use xilem::view::{button, flex_col, indexed_stack, label, sized_box, FlexSequence};
use crate::actions::{ daw_events_RiffArrangementSelected};
use crate::icons::{ICON_ARROW_DOWN, ICON_ARROW_UP, ICON_LAYERS_SELECTED_BOTTOM};

fn up_change_callback(state: &mut RiffDAWState) {
    let mut new_index: i32 = state.riff_arrangement_view_state.riff_arr_to_select_index as i32 - 1;
    if new_index < 0 {
        new_index = 0;
    }
    state.riff_arrangement_view_state.riff_arr_to_select_index = new_index as usize;
}

fn down_change_callback(state: &mut RiffDAWState) {
    if let Ok(project) = state.project.lock() {
        let mut new_index = state.riff_arrangement_view_state.riff_arr_to_select_index + 1;
        if new_index >= project.song.riff_arrangements().len() {
            new_index = 0;
        }
        state.riff_arrangement_view_state.riff_arr_to_select_index = new_index;
    }
}

fn select_change_callback(state: &mut RiffDAWState) {
    let mut riff_arr_uuid = None;

    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(riff_arr) = project.song.riff_arrangements().get(state.riff_arrangement_view_state.riff_arr_to_select_index.clone()) {
            riff_arr_uuid = Some(riff_arr.uuid().clone());
        }
    }

    if let Some(riff_arr_uuid) = riff_arr_uuid.as_ref() {
        daw_events_RiffArrangementSelected(state, riff_arr_uuid.clone());
    }
}

pub fn riff_arrangement_selector(data: &RiffDAWState, active_index: usize) -> impl FlexSequence<RiffDAWState> + use<> {
    let mut options: Vec<_> = vec![];

    if let Ok(project) = data.project.lock() {
        options = project.song.riff_arrangements().iter().map(|riff_arr| {
            flex_col(sized_box::<RiffDAWState, (), xilem::view::Label>(label((*riff_arr).name().clone())).width(100.px()))
        }).collect();
    }

    (
        indexed_stack(
            options
        )
            .active(active_index),
        button(icon(ICON_ARROW_UP.to_string()), up_change_callback),
        button(icon(ICON_ARROW_DOWN.to_string()), down_change_callback),
        button(icon(ICON_LAYERS_SELECTED_BOTTOM.to_string()), select_change_callback),
    )
}
