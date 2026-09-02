use crate::state::RiffDAWState;
use crate::views::riff_set_selector;
use uuid::Uuid;
use xilem::view::FlexSequence;
use crate::actions::daw_events_RiffSequenceRiffSetAdd;
fn up_change_callback(state: &mut RiffDAWState) {
    let mut new_index: i32 = state.riff_sequence_view_state.add_to_seq_riff_set_index as i32 - 1;
    if new_index < 0 {
        new_index = 0;
    }
    state.riff_sequence_view_state.add_to_seq_riff_set_index = new_index as usize;
}

fn down_change_callback(state: &mut RiffDAWState) {
    if let Ok(project) = state.project.lock() {
        let mut new_index = state.riff_sequence_view_state.add_to_seq_riff_set_index + 1;
        if new_index >= project.song.riff_sets().len() {
            new_index = 0;
        }
        state.riff_sequence_view_state.add_to_seq_riff_set_index = new_index;
    }
}

fn select_change_callback(state: &mut RiffDAWState) {
    let mut riff_set_uuid = None;
    if let Ok(project) = state.get_project().lock().as_mut() {
        if let Some(riff_set) = project.song.riff_sets().get(state.riff_sequence_view_state.add_to_seq_riff_set_index.clone()) {
            riff_set_uuid = Some(riff_set.uuid().clone());
        }
    }
    if let Some(riff_seq_uuid) = state.riff_sequence_view_state.selected_riff_sequence_uuid() {
        if let Some(riff_set_uuid) = riff_set_uuid {
            daw_events_RiffSequenceRiffSetAdd(state, riff_seq_uuid.clone(), riff_set_uuid, Uuid::new_v4());
        }
    }
}

pub fn riff_seq_view_riff_set_selector(data: &RiffDAWState, active_index: usize) -> impl FlexSequence<RiffDAWState> + use<> {
    riff_set_selector(data, active_index, up_change_callback, down_change_callback, select_change_callback)
}

