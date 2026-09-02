use masonry::properties::types::{AsUnit, CrossAxisAlignment, Length, MainAxisAlignment};
use xilem::view::{flex_col, flex_row, label, sized_box, text_button, text_input, FlexSequence};
use crate::state::{RiffDAWState};
use crate::actions::{daw_events_RiffSequenceCopy, daw_events_RiffSequenceCopySelectedToTrackViewCursorPosition, daw_events_RiffSequenceDelete, daw_events_RiffSequenceNameChange, daw_events_RiffSequencePlay, daw_events_RiffSequenceRiffSetDelete, daw_events_RiffSequenceRiffSetMoveLeft, daw_events_RiffSequenceRiffSetMoveRight, daw_events_RiffSequenceRiffSetSelect, daw_events_RiffSetPlay};
use crate::domain::Track;
use crate::event::OperationModeType;
use crate::views::*;


pub fn riff_seq_head_panel_sequence<State>(
    state: &RiffDAWState,
) -> impl FlexSequence<RiffDAWState> {
    let mut riff_seq_uuid = "Unknown".to_string();
    let mut name = "Unknown".to_string();

    if let Ok(project) = state.project.lock() {
        // get the selected riff sequence
        // if there is no selected riff sequence then set the first one as selected
        if let Some(selected_riff_seq_uuid) = state.riff_sequence_view_state.selected_riff_sequence_uuid.clone() {
            // find the selected riff sequence and get its name
            if let Some(riff_seq) = project.song().riff_sequence(selected_riff_seq_uuid.clone()) {
                name = riff_seq.name().to_string();
                riff_seq_uuid = riff_seq.uuid().to_string();
            }
        } else if project.song.riff_sequences().iter().count() > 0 {
            if let Some(riff_seq) = project.song.riff_sequences().get(0) {
                name = riff_seq.name.clone();
                riff_seq_uuid = riff_seq.uuid().to_string();
            }
        }
    }

    let riff_seq_uuid_edit = riff_seq_uuid.clone();
    let riff_seq_uuid_play = riff_seq_uuid.clone();
    let riff_seq_uuid_copy = riff_seq_uuid.clone();
    let riff_seq_uuid_delete = riff_seq_uuid.clone();
    let riff_seq_uuid_copy_to_track = riff_seq_uuid.clone();

    // create the riff seq head panel
    (
        sized_box(text_input(name,  move |state: &mut RiffDAWState, new_name: String| daw_events_RiffSequenceNameChange(state, riff_seq_uuid_edit.clone(), new_name))).width(100.px()),
        text_button("P", move |state: &mut RiffDAWState| daw_events_RiffSequencePlay(state, riff_seq_uuid_play.clone())),
        text_button("C", move |state: &mut RiffDAWState| daw_events_RiffSequenceCopy(state, riff_seq_uuid_copy.clone())),
        text_button("X", move |state: &mut RiffDAWState| daw_events_RiffSequenceDelete(state, riff_seq_uuid_delete.clone())),
        text_button("T", move |state: &mut RiffDAWState| daw_events_RiffSequenceCopySelectedToTrackViewCursorPosition(state, riff_seq_uuid_copy_to_track.clone())),
    )
}

pub fn riff_seq_riff_set_head_panel_sequence<State>(
    state: &RiffDAWState,
) -> impl FlexSequence<RiffDAWState> {
    let mut riff_set_heads = vec![];

    if let Ok(project) = state.project.lock() {
        // get the selected riff sequence
        // if there is no selected riff sequence then set the first one as selected
        let selected_riff_seq_uuid = if let Some(selected_riff_seq_uuid) = state.riff_sequence_view_state.selected_riff_sequence_uuid.clone() {
            selected_riff_seq_uuid.clone()
        } else if project.song.riff_sequences().iter().count() > 0 {
            if let Some(seq) = project.song.riff_sequences().get(0) {
                seq.uuid.clone()
            } else { "".to_string() }
        } else { "".to_string() };

        // get the select riff seq
        let riff_set_uuids = if let Some(riff_seq) = project.song.riff_sequences().iter().find(|riff_seq| riff_seq.uuid == selected_riff_seq_uuid) {
            riff_seq.riff_sets().iter().map(|riff_item| (riff_item.uuid.clone(), riff_item.item_uuid.clone())).collect::<Vec<_>>()
        } else {
            vec![]
        };

        for (riff_set_ref_uuid, riff_set_uuid) in riff_set_uuids.iter() {
            let riff_set_ref_uuid_play = riff_set_ref_uuid.clone();
            let riff_set_ref_uuid_select = riff_set_ref_uuid.clone();
            let riff_set_ref_uuid_delete = riff_set_ref_uuid.clone();
            let riff_set_ref_uuid_move_left = riff_set_ref_uuid.clone();
            let riff_set_ref_uuid_move_right = riff_set_ref_uuid.clone();
            let riff_set_ref_uuid_drag = riff_set_ref_uuid.clone();

            let riff_seq_uuid_play = selected_riff_seq_uuid.clone();
            let riff_seq_uuid_select = selected_riff_seq_uuid.clone();
            let riff_seq_uuid_delete = selected_riff_seq_uuid.clone();
            let riff_seq_uuid_move_left = selected_riff_seq_uuid.clone();
            let riff_seq_uuid_move_right = selected_riff_seq_uuid.clone();
            let riff_seq_uuid_drag = selected_riff_seq_uuid.clone();

            let riff_set_uuid_play = riff_set_uuid.clone();

            riff_set_heads.push(project.song.riff_sets().iter().find(|riff_set| riff_set.uuid == *riff_set_uuid).map(|riff_set| {
                sized_box(
                    flex_col(
                        (
                            sized_box(label(riff_set.name.as_str())).width(Length::px(37.)),
                            flex_row(
                                (
                                    text_button("P", move |state: &mut RiffDAWState| daw_events_RiffSetPlay(state, riff_set_uuid_play.clone())),
                                    text_button("B", move |state: &mut RiffDAWState| daw_events_RiffSequenceRiffSetSelect(state, riff_seq_uuid_play.clone(), riff_set_ref_uuid_select.clone(), true)), // FIXME need to beable to set the selected riff sequence riff set
                                    text_button("X", move |state: &mut RiffDAWState| daw_events_RiffSequenceRiffSetDelete(state, riff_seq_uuid_delete.clone(), riff_set_ref_uuid_delete.clone())),
                                )
                            )
                                .main_axis_alignment(MainAxisAlignment::Start)
                                .cross_axis_alignment(CrossAxisAlignment::Start)
                                .gap(3.px()),
                            flex_row(
                                (
                                    text_button("<", move |state: &mut RiffDAWState| daw_events_RiffSequenceRiffSetMoveLeft(state, riff_seq_uuid_move_left.clone(), riff_set_ref_uuid_move_left.clone())),
                                    text_button(">", move |state: &mut RiffDAWState| daw_events_RiffSequenceRiffSetMoveRight(state, riff_seq_uuid_move_right.clone(), riff_set_ref_uuid_move_right.clone())),
                                    text_button("D", move |state: &mut RiffDAWState| println!("Riff seq riff set ref drag")), // FIXME need to implement riff seq riff set drag
                                )
                            )
                                .main_axis_alignment(MainAxisAlignment::Start)
                                .cross_axis_alignment(CrossAxisAlignment::Start)
                                .gap(0.px())
                        )
                    )
                        .main_axis_alignment(MainAxisAlignment::Start)
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .gap(3.px()),
                )
                    .width(37.px())
            }));
        }
    }

    riff_set_heads
}

pub fn riff_seq_riff_set_riffs_panel_sequence<State>(
    state: &RiffDAWState,
) -> impl FlexSequence<RiffDAWState> {
    // get the selected riff sequence
    // if there is no selected riff sequence then set the first one as selected
    let selected_riff_seq_uuid= if let Some(selected_riff_seq_uuid) = state.riff_sequence_view_state.selected_riff_sequence_uuid.clone() {
        selected_riff_seq_uuid.clone()
    }
    else if let Ok(project) = state.project.lock() {
        if project.song.riff_sequences().iter().count() > 0 {
            if let Some(seq) = project.song.riff_sequences().get(0) {
                seq.uuid.clone()
            }
            else { "".to_string() }
        }
        else { "".to_string() }
    }
    else { "".to_string() };

    // get the select riff seq
    let track_uuid_in_order = state.project.lock().unwrap().song.tracks().iter().map(|track| track.uuid().clone()).collect::<Vec<_>>();
    let riff_set_columns = if let Some(riff_seq) = state.project.lock().unwrap().song.riff_sequences().iter().find(|riff_seq| riff_seq.uuid == selected_riff_seq_uuid) {
        // iterate through the riff sets in the riff seq
        riff_seq.riff_sets().iter().map(|riff_set| {
            flex_col(
                track_uuid_in_order.iter().map(|track_uuid| riff_track_grid_with_size(
                    state.project.clone(),
                    50.0,
                    84.0,
                    vec![],
                    OperationModeType::PointMode,
                    track_uuid.clone(),
                    riff_set.item_uuid.clone(),
                )
                ).collect::<Vec<_>>()
            )
                .main_axis_alignment(MainAxisAlignment::Start)
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .gap(1.px())
        }).collect::<Vec<_>>()
    }
    else { vec![] };

    (flex_row(
        riff_set_columns
    )
        .main_axis_alignment(MainAxisAlignment::Start)
        .cross_axis_alignment(CrossAxisAlignment::Start))
        .gap(1.px())
}
