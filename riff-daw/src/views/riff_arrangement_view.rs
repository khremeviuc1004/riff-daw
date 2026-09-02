use masonry::properties::types::{AsUnit, CrossAxisAlignment, MainAxisAlignment};
use uuid::Uuid;
use xilem::view::{button, flex_col, flex_row, label, portal, sized_box, split, text_input, Flex, FlexSequence, FlexSpacer, Portal, Split};
use crate::actions::daw_events_RiffArrangementAdd;
use crate::icons::{ICON_PLUS};
use crate::state::RiffDAWState;
use crate::views::{icon, riff_arr_riff_items_head_panel_sequence, riff_arr_riff_items_panel_sequence, riff_arr_view_riff_grid_selector, riff_arr_view_riff_seq_selector, riff_arr_view_riff_set_selector, riff_arrangement_head_panel, riff_arrangement_selector, track_panel_sequence};


pub fn riff_arrangement_toolbar(
    data: &RiffDAWState
) -> Flex<impl FlexSequence<RiffDAWState>, RiffDAWState> {
    flex_row(
        (
            FlexSpacer::Flex(1.0)
        )
    )
}

pub fn riff_arrangement_view(
    data: &RiffDAWState
) -> Split<Portal<Flex<impl FlexSequence<RiffDAWState>, RiffDAWState>, RiffDAWState, ()>, Portal<Flex<impl FlexSequence<RiffDAWState>, RiffDAWState>, RiffDAWState, ()>, RiffDAWState> {
    split (
        portal(
            flex_col(
                (
                    sized_box(
                        flex_row((
                            sized_box(label("Arrangements")).width(100.px()),
                            riff_arrangement_selector(data, data.riff_arrangement_view_state.riff_arr_to_select_index.clone()),
                        ))
                    ).width(200.px()),
                    sized_box(
                        flex_row((
                            sized_box(label("Grids")).width(100.px()),
                            riff_arr_view_riff_grid_selector(data, data.riff_arrangement_view_state.add_to_arr_riff_grid_index.clone()),
                        ))
                    ).width(200.px()),
                    sized_box(
                        flex_row((
                            sized_box(label("Sequences")).width(100.px()),
                            riff_arr_view_riff_seq_selector(data, data.riff_arrangement_view_state.add_to_arr_riff_seq_index.clone()),
                        ))
                    ).width(200.px()),
                    sized_box(
                        flex_row((
                            sized_box(label("Riff Sets")).width(100.px()),
                            riff_arr_view_riff_set_selector(data, data.riff_arrangement_view_state.add_to_arr_riff_set_index.clone()),
                        ))
                    ).width(200.px()),
                    sized_box(
                        flex_row((
                            label("New arrangement"),
                            sized_box(text_input(data.riff_arrangement_view_state.add_riff_arrangement_name.clone(), |state: &mut RiffDAWState, new_name| {
                                state.riff_arrangement_view_state.add_riff_arrangement_name = new_name;
                            })).width(200.px()),
                            button(icon(ICON_PLUS.to_string()), |state| {
                                daw_events_RiffArrangementAdd(state, Uuid::new_v4());
                                state.riff_arrangement_view_state.add_riff_arrangement_name.clear();
                            }),
                        ))
                    ).width(200.px()),
                    track_panel_sequence::<RiffDAWState>(data, 39.px()),
                    FlexSpacer::Fixed(60000.px())
                )
            )
                .main_axis_alignment(MainAxisAlignment::Start)
                .cross_axis_alignment(CrossAxisAlignment::Start),
        ),
        portal(
            flex_col(
                (
                    flex_row(riff_arrangement_head_panel(data)).main_axis_alignment(MainAxisAlignment::Start),
                    flex_row(riff_arr_riff_items_head_panel_sequence(data)).main_axis_alignment(MainAxisAlignment::Start),
                    flex_row(riff_arr_riff_items_panel_sequence::<RiffDAWState>(data)).main_axis_alignment(MainAxisAlignment::Start),
                )
            )
                .main_axis_alignment(MainAxisAlignment::Start)
                .cross_axis_alignment(CrossAxisAlignment::Start),
        )
    ).split_point(0.2)
}