use masonry::properties::types::{AsUnit, CrossAxisAlignment, Length, MainAxisAlignment};
use xilem::view::{button, flex, flex_col, flex_row, indexed_stack, label, sized_box, text_button, text_input, Flex, FlexSequence};
use crate::icons::{ICON_ARROW_DOWN, ICON_ARROW_UP, ICON_PLUS};
use crate::views::{icon};
use crate::state::{RiffDAWState};
use itertools::Itertools;
use serde_json::to_string;
use uuid::Uuid;
use xilem::{AnyWidgetView, WidgetView};
use xilem_core::View;
use crate::actions::{daw_events_RiffArrangementCopy, daw_events_RiffArrangementCopySelectedToTrackViewCursorPosition, daw_events_RiffArrangementDelete, daw_events_RiffArrangementNameChange, daw_events_RiffArrangementPlay, daw_events_RiffArrangementRiffItemDelete, daw_events_RiffArrangementRiffItemSelect, daw_events_RiffGridPlay, daw_events_RiffSequenceCopy, daw_events_RiffSequenceCopySelectedToTrackViewCursorPosition, daw_events_RiffSequenceDelete, daw_events_RiffSequenceNameChange, daw_events_RiffSequencePlay, daw_events_RiffSequenceRiffSetAdd, daw_events_RiffSequenceRiffSetDelete, daw_events_RiffSequenceRiffSetMoveLeft, daw_events_RiffSequenceRiffSetMoveRight, daw_events_RiffSequenceRiffSetSelect, daw_events_RiffSetPlay};
use crate::domain::{RiffItemType, Track};
use crate::event::OperationModeType;
use crate::views::*;


pub fn riff_arrangement_head_panel(
    state: &RiffDAWState,
) -> impl FlexSequence<RiffDAWState> {
    let mut name = "Unknown".to_string();
    let mut riff_arr_uuid = "Unknown".to_string();

    if let Ok(project) = state.project.lock() {
        // get the selected riff sequence
        // if there is no selected riff sequence then set the first one as selected
        if let Some(selected_riff_arr_uuid) = state.riff_arrangement_view_state.selected_riff_arrangement_uuid.clone() {
            // find the selected riff sequence and get its name
            if let Some(riff_arr) = project.song().riff_arrangement(selected_riff_arr_uuid.clone()) {
                name = riff_arr.name().to_string();
                riff_arr_uuid = riff_arr.uuid().to_string();
            }
        } else if project.song.riff_arrangements().iter().count() > 0 {
            if let Some(riff_arr) = project.song.riff_arrangements().get(0) {
                name = riff_arr.name.clone();
                riff_arr_uuid = riff_arr.uuid().to_string();
            }
        }
    }

    let riff_arr_uuid_edit = riff_arr_uuid.clone();
    let riff_arr_uuid_play = riff_arr_uuid.clone();
    let riff_arr_uuid_copy = riff_arr_uuid.clone();
    let riff_arr_uuid_delete = riff_arr_uuid.clone();
    let riff_arr_uuid_copy_to_track = riff_arr_uuid.clone();

    // create the riff arr head panel
    (
        sized_box(text_input(name,  move |state: &mut RiffDAWState, new_name: String| daw_events_RiffArrangementNameChange(state, riff_arr_uuid_edit.clone(), new_name))).width(100.px()),
        text_button("P", move |state: &mut RiffDAWState| daw_events_RiffArrangementPlay(state, riff_arr_uuid_play.clone())),
        text_button("C", move |state: &mut RiffDAWState| daw_events_RiffArrangementCopy(state, riff_arr_uuid_copy.clone())),
        text_button("X", move |state: &mut RiffDAWState| daw_events_RiffArrangementDelete(state, riff_arr_uuid_delete.clone())),
        text_button("T", move |state: &mut RiffDAWState| daw_events_RiffArrangementCopySelectedToTrackViewCursorPosition(state, riff_arr_uuid_copy_to_track.clone())),
    )
}

pub fn riff_arr_riff_items_head_panel_sequence(
    state: &RiffDAWState,
) -> Box<AnyWidgetView<RiffDAWState>> {
    let mut riff_item_heads = vec![];

    if let Ok(project) = state.project.lock() {
        // get the selected riff arr
        // if there is no selected riff arr then set the first one as selected
        let selected_riff_arr_uuid = if let Some(selected_riff_arr_uuid) = state.riff_arrangement_view_state.selected_riff_arrangement_uuid.clone() {
            selected_riff_arr_uuid.clone()
        } else if project.song.riff_arrangements().iter().count() > 0 {
            if let Some(seq) = project.song.riff_arrangements().get(0) {
                seq.uuid.clone()
            } else { "".to_string() }
        } else { "".to_string() };

        riff_item_heads = if let Some(riff_arr) = project.song.riff_arrangements().iter().find(|riff_arr| riff_arr.uuid == selected_riff_arr_uuid) {
            riff_arr.items.iter().map(|riff_item| {
                match riff_item.item_type() {
                    RiffItemType::RiffSet => {
                        let riff_set = project.song.riff_sets().iter().find(|riff_set| riff_set.uuid == *riff_item.item_uuid()).unwrap();
                        riff_arr_riff_item_head_panel_riff_set_column(state, selected_riff_arr_uuid.clone(), riff_set.name.clone(), riff_item.uuid(), riff_item.item_uuid().to_string())
                    }
                    RiffItemType::RiffSequence => {
                        let riff_sequence = project.song.riff_sequences().iter().find(|riff_sequence| riff_sequence.uuid == *riff_item.item_uuid()).unwrap();
                        riff_arr_riff_items_head_panel_riff_sequence_column(state, selected_riff_arr_uuid.clone(), riff_sequence.name().to_string(), riff_item.uuid(), riff_item.item_uuid().to_string())
                    }
                    RiffItemType::RiffGrid => {
                        let riff_grid = project.song.riff_grids().iter().find(|riff_grid| riff_grid.uuid == *riff_item.item_uuid()).unwrap();
                        riff_arr_riff_items_head_panel_riff_grid_column(state, selected_riff_arr_uuid.clone(), riff_grid.name().to_string(), riff_item.uuid(), riff_item.item_uuid().to_string())
                    }
                }
            }).collect()
        }
        else {
            vec![]
        }
    }

    (flex_row(
        riff_item_heads
    )
        .main_axis_alignment(MainAxisAlignment::Start)
        .cross_axis_alignment(CrossAxisAlignment::Start))
        .gap(1.px())
        .boxed()
}

pub fn riff_arr_riff_items_panel_sequence<State>(
    state: &RiffDAWState,
) -> impl FlexSequence<RiffDAWState> {
    let mut riff_item_columns = vec![];

    // get the selected riff arr
    // if there is no selected riff arr then set the first one as selected
    let selected_riff_arr_uuid = if let Some(selected_riff_arr_uuid) = state.riff_arrangement_view_state.selected_riff_arrangement_uuid.clone() {
        selected_riff_arr_uuid.clone()
    }
    else if let Ok(project) = state.project.lock() {
        if project.song.riff_arrangements().iter().count() > 0 {
            if let Some(riff_arr) = project.song.riff_arrangements().get(0) {
                riff_arr.uuid.clone()
            }
            else { "".to_string() }
        }
        else { "".to_string() }
    }
    else { "".to_string() };

    // get the select riff arr
    if let Ok(project) = state.project.lock() {
         let track_uuid_in_order = project.song.tracks().iter().map(|track| track.uuid().clone()).collect::<Vec<_>>();
         riff_item_columns = if let Some(riff_arr) = project.song.riff_arrangements().iter().find(|riff_arr| riff_arr.uuid == selected_riff_arr_uuid) {
             let riff_arr_name = riff_arr.name.clone();

             // iterate through the riff items in the riff arr
             riff_arr.items().iter().map(|riff_item|  {
                 match riff_item.item_type() {
                     RiffItemType::RiffSet => {
                         Box::new(flex_col(
                             track_uuid_in_order.iter().map(|track_uuid| riff_track_grid_with_size(
                                 state.project.clone(),
                                 50.0,
                                 84.0,
                                 vec![],
                                 OperationModeType::PointMode,
                                 track_uuid.clone(),
                                 riff_item.item_uuid.clone(),
                             )
                             ).collect::<Vec<_>>()
                         )
                             .main_axis_alignment(MainAxisAlignment::Start)
                             .cross_axis_alignment(CrossAxisAlignment::Start)
                             .gap(1.px())) as Box<AnyWidgetView<RiffDAWState>>
                     }
                     RiffItemType::RiffSequence => {
                         // get the riff sequence
                         let mut riff_sets = vec![];
                         let mut number_of_riff_sets = 0;
                         if let Some(riff_seq) = project.song.riff_sequence(riff_item.item_uuid.clone()) {
                             number_of_riff_sets = riff_seq.riff_sets().iter().count();
                             // loop through the riff sets in the riff sequence
                             for riff_set in riff_seq.riff_sets().iter() {
                                 // get the riff set
                                 if let Some(riff_set) = project.song.riff_set(riff_set.item_uuid.clone()) {
                                     riff_sets.push(
                                     flex_col(
                                         track_uuid_in_order.iter().map(|track_uuid| riff_track_grid_with_size(
                                                 state.project.clone(),
                                                 50.0,
                                                 84.0,
                                                 vec![],
                                                 OperationModeType::PointMode,
                                                 track_uuid.clone(),
                                                 riff_item.item_uuid.clone(),
                                             )
                                         ).collect::<Vec<_>>()
                                     )
                                         .main_axis_alignment(MainAxisAlignment::Start)
                                         .cross_axis_alignment(CrossAxisAlignment::Start)
                                         .gap(1.px())
                                     );
                                 }
                             }
                         }

                         if number_of_riff_sets <= 2 {
                             // need to make up the difference
                             Box::new(flex_row((riff_sets, sized_box(label("".to_string())).width(Length::px(84.0 * (3.0 - (number_of_riff_sets as f64))))))) as Box<AnyWidgetView<RiffDAWState>>
                         }
                         else {
                             Box::new(flex_row(riff_sets)) as Box<AnyWidgetView<RiffDAWState>>
                         }
                     }
                     RiffItemType::RiffGrid => {
                         // FIXME Need to get the width in beats of the riff grid and set the sized box width
                         Box::new(flex_col(
                             sized_box(
                                 portal(
                                     riff_grid_with_size(
                                         state.project.clone(),
                                         60000.0,
                                         60000.0,
                                         state.selected_riff_grid_riff_references.clone(),
                                         OperationModeType::PointMode,
                                         state.riff_grid_view_state.selected_riff_grid_uuid.clone()
                                     )
                                 )
                             ).width(400.px())
                         )) as Box<AnyWidgetView<RiffDAWState>>
                     }
                 }
             }).collect()
         }
         else {
            vec![]
        };
    }

    (flex_row(
        riff_item_columns
    )
        .main_axis_alignment(MainAxisAlignment::Start)
        .cross_axis_alignment(CrossAxisAlignment::Start))
        .gap(1.px())
}


fn riff_arr_riff_items_head_panel_riff_sequence_column(
    state: &RiffDAWState,
    riff_arrangement_uuid: String,
    riff_sequence_name: String,
    riff_seq_ref_uuid: String,
    riff_seq_uuid: String,
) -> Box<AnyWidgetView<RiffDAWState>> {
    let riff_seq_uuid_edit = riff_seq_uuid.clone();
    let riff_seq_uuid_play = riff_seq_uuid.clone();
    let riff_arrangement_uuid_delete = riff_arrangement_uuid.clone();
    let riff_seq_uuid_ref_delete = riff_seq_ref_uuid.clone();
    let riff_arrangement_uuid_select = riff_arrangement_uuid.clone();
    let riff_seq_uuid_ref_select = riff_seq_ref_uuid.clone();

    flex_row((
        sized_box(text_input(riff_sequence_name,  move |state: &mut RiffDAWState, new_name: String| daw_events_RiffSequenceNameChange(state, riff_seq_uuid_edit.clone(), new_name))).width(100.px()),
        text_button("P", move |state: &mut RiffDAWState| daw_events_RiffSequencePlay(state, riff_seq_uuid_play.clone())),
        text_button("X", move |state: &mut RiffDAWState| daw_events_RiffArrangementRiffItemDelete(state, riff_arrangement_uuid_delete.clone(), riff_seq_uuid_ref_delete.clone())),
        text_button("B", move |state: &mut RiffDAWState| daw_events_RiffArrangementRiffItemSelect(state, riff_arrangement_uuid_select.clone(), riff_seq_uuid_ref_select.clone(), true)),
    )).boxed()
}

fn riff_arr_riff_item_head_panel_riff_set_column(
    state: &RiffDAWState,
    riff_arrangement_uuid: String,
    riff_set_name: String,
    riff_set_ref_uuid: String,
    riff_set_uuid: String,
) -> Box<AnyWidgetView<RiffDAWState>> {
    let riff_set_uuid_play = riff_set_uuid.clone();
    // let riff_set_uuid_select = riff_set.uuid.clone();
    let riff_arrangement_uuid_delete = riff_arrangement_uuid.clone();
    let riff_set_ref_uuid_delete = riff_set_ref_uuid.clone();

    flex_col((
        sized_box(label(riff_set_name)).width(Length::px(84.)),
        flex_row(
            (
                text_button("P", move |state: &mut RiffDAWState| daw_events_RiffSetPlay(state, riff_set_uuid_play.clone())),
                // text_button("B", move |state: &mut RiffDAWState| daw_events_RiffArrangementRiffItemSelect(state, riff_set_uuid_play.clone(), riff_set_ref_uuid_select.clone(), true)), // FIXME need to beable to set the selected riff sequence riff set
                text_button("X", move |state: &mut RiffDAWState| daw_events_RiffArrangementRiffItemDelete(state, riff_arrangement_uuid_delete.clone(), riff_set_ref_uuid_delete.clone())),
            )
        )
            .main_axis_alignment(MainAxisAlignment::Start)
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .gap(1.px()),
        // flex_row(
        //     (
        //         text_button("<", move |state: &mut RiffDAWState| daw_events_RiffSequenceRiffSetMoveLeft(state, riff_seq_uuid_move_left.clone(), riff_set_ref_uuid_move_left.clone())),
        //         text_button(">", move |state: &mut RiffDAWState| daw_events_RiffSequenceRiffSetMoveRight(state, riff_seq_uuid_move_right.clone(), riff_set_ref_uuid_move_right.clone())),
        //         // text_button("D", move |state: &mut RiffDAWState| println!("Riff seq riff set ref drag")), // FIXME need to implement riff seq riff set drag
        //     )
        // )
        //     .main_axis_alignment(MainAxisAlignment::Start)
        //     .cross_axis_alignment(CrossAxisAlignment::Start)
        //     .gap(0.px())
    )).boxed()
}


fn riff_arr_riff_items_head_panel_riff_grid_column(
    state: &RiffDAWState,
    riff_arrangement_uuid: String,
    riff_grid_name: String,
    riff_grid_ref_uuid: String,
    riff_grid_uuid: String,
) -> Box<AnyWidgetView<RiffDAWState>> {
    let riff_grid_uuid_edit = riff_grid_uuid.clone();
    let riff_grid_uuid_play = riff_grid_uuid.clone();
    let riff_arrangement_uuid_delete = riff_arrangement_uuid.clone();
    let riff_grid_uuid_ref_delete = riff_grid_ref_uuid.clone();
    let riff_arrangement_uuid_select = riff_arrangement_uuid.clone();
    let riff_grid_uuid_ref_select = riff_grid_ref_uuid.clone();

    flex_row((
        sized_box(text_input(riff_grid_name, move |state: &mut RiffDAWState, new_name: String| daw_events_RiffSequenceNameChange(state, riff_grid_uuid_edit.clone(), new_name))).width(100.px()),
        text_button("P", move |state: &mut RiffDAWState| daw_events_RiffGridPlay(state, riff_grid_uuid_play.clone())),
        text_button("X", move |state: &mut RiffDAWState| daw_events_RiffArrangementRiffItemDelete(state, riff_arrangement_uuid_delete.clone(), riff_grid_uuid_ref_delete.clone())),
        text_button("B", move |state: &mut RiffDAWState| daw_events_RiffArrangementRiffItemSelect(state, riff_arrangement_uuid_select.clone(), riff_grid_uuid_ref_select.clone(), true)),
    )).boxed()
}
