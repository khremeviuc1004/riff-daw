use masonry::properties::types::{AsUnit, CrossAxisAlignment, Length, MainAxisAlignment};
use uuid::Uuid;
use xilem::view::{flex_col, flex_row, label, sized_box, text_button, FlexSequence};
use crate::actions::{daw_events_RiffSetCopy, daw_events_RiffSetCopySelectedToTrackViewCursorPosition, daw_events_RiffSetDelete, daw_events_RiffSetPlay, daw_events_RiffSetSelect, daw_events_RiffSetTrackIncrementRiff, daw_events_RiffSetTrackSetRiff, track_change_type_RiffAdd, track_change_type_RiffCutSelected, track_change_type_RiffReferenceIncrementRiff, track_change_type_RiffSelect};
use crate::event::{OperationModeType};
use crate::views::{riff_track_grid_with_size};
use crate::state::{RiffDAWState};
use crate::views::*;
use crate::domain::{Track};

pub fn riff_set_head_panel_sequence<State>(
    state: &RiffDAWState,
) -> impl FlexSequence<RiffDAWState> {
    state.project.lock().unwrap().song.riff_sets().iter().enumerate().map(|(riff_set_index, riff_set)| {
        let index3 = riff_set_index;
        let riff_set_uuid_play = riff_set.uuid.clone();
        let riff_set_uuid_copy_to_track = riff_set.uuid.clone();
        let riff_set_uuid_select = riff_set.uuid.clone();
        let riff_set_uuid_copy = riff_set.uuid.clone();
        let riff_set_uuid_delete = riff_set.uuid.clone();
        let riff_set_uuid_drag = riff_set.uuid.clone();

        sized_box(
            flex_col(
                (
                    sized_box(label(riff_set.name.as_str())).width(Length::px(50.)),
                    flex_row(
                        (
                            text_button("P", move |state: &mut RiffDAWState| daw_events_RiffSetPlay(state, riff_set_uuid_play.clone())),
                            text_button("T", move |state: &mut RiffDAWState| daw_events_RiffSetCopySelectedToTrackViewCursorPosition(state, riff_set_uuid_copy_to_track.clone())),
                            text_button("B", move |state: &mut RiffDAWState| daw_events_RiffSetSelect(state, riff_set_uuid_select.clone(), true)), // FIXME need to toggle of depending on state
                        )
                    )
                        .main_axis_alignment(MainAxisAlignment::Start)
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .gap(1.px()),
                    flex_row(
                        (
                            text_button("C", move |state: &mut RiffDAWState| daw_events_RiffSetCopy(state, riff_set_uuid_copy.clone(), Uuid::new_v4())),
                            text_button("X", move |state: &mut RiffDAWState| daw_events_RiffSetDelete(state, riff_set_uuid_delete.clone())),
                            text_button("D", move |state: &mut RiffDAWState| {
                                // FIXME implement riff set view riff set drag
                            }),
                        )
                    )
                        .main_axis_alignment(MainAxisAlignment::Start)
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .gap(1.px())
                )
            )
                .main_axis_alignment(MainAxisAlignment::Start)
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .gap(1.px()),
        )
            .width(50.px())
    }).collect::<Vec<_>>()
}


pub fn riff_set_riffs_panel_sequence<State>(
    state: &RiffDAWState,
) -> impl FlexSequence<RiffDAWState> {
    let track_uuid_in_order = state.project.lock().unwrap().song.tracks().iter().map(|track| track.uuid().clone()).collect::<Vec<_>>();
    let riff_set_columns = state.project.lock().unwrap().song.riff_sets().iter().map(|riff_set| {
        flex_col(
            track_uuid_in_order.iter().map(|track_uuid| riff_track_grid_with_size(
                state.project.clone(),
                50.0,
                84.0,
                vec![],
                OperationModeType::PointMode,
                track_uuid.clone(),
                riff_set.uuid.clone(),
            )
                .on_riff_set_track_increment_riff(Box::new(|data, riff_set_uuid, track_uuid| daw_events_RiffSetTrackIncrementRiff(data, riff_set_uuid, track_uuid)))
                .on_riff_select(Box::new(|data, riff_set_uuid, track_uuid| track_change_type_RiffSelect(data, riff_set_uuid, Some(track_uuid))))
                .on_riff_add(Box::new(|data, uuid, name, track_uuid, duration| track_change_type_RiffAdd(data, name, duration, Some(track_uuid))))
                .on_riff_set_track_set_riff(Box::new(|data, riff_set_uuid, track_uuid, new_riff_uuid| daw_events_RiffSetTrackSetRiff(data, riff_set_uuid, track_uuid, new_riff_uuid)))
            ).collect::<Vec<_>>()
        )
            .main_axis_alignment(MainAxisAlignment::Start)
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .gap(1.px())
    }).collect::<Vec<_>>();

    (flex_row(
        riff_set_columns
    )
        .main_axis_alignment(MainAxisAlignment::Start)
        .cross_axis_alignment(CrossAxisAlignment::Start))
        .gap(1.px())
}
