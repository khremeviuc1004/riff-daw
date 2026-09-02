use masonry::properties::types::{AsUnit, CrossAxisAlignment, MainAxisAlignment};
use uuid::Uuid;
use xilem::view::{button, flex_col, flex_row, label, portal, sized_box, split, text_input, Flex, FlexSequence, FlexSpacer, Portal, Split};
use crate::actions::daw_events_RiffSequenceAdd;
use crate::icons::ICON_PLUS;
use crate::state::RiffDAWState;
use crate::views::{icon, riff_seq_head_panel_sequence, riff_seq_riff_set_head_panel_sequence, riff_seq_riff_set_riffs_panel_sequence, riff_seq_view_riff_seq_selector, riff_seq_view_riff_set_selector, track_panel_sequence};


pub fn riff_sequence_view_toolbar(
    data: &RiffDAWState
) -> Flex<impl FlexSequence<RiffDAWState>, RiffDAWState> {
    flex_row(
        (
            FlexSpacer::Flex(1.0)
        )
    )
}


pub fn riff_sequence_view(
    data: &RiffDAWState)
    -> Split<Portal<Flex<impl FlexSequence<RiffDAWState>, RiffDAWState>, RiffDAWState, ()>, Portal<Flex<impl FlexSequence<RiffDAWState>, RiffDAWState>, RiffDAWState, ()>, RiffDAWState> {
    split (
        portal(
            flex_col(
                (
                    sized_box(
                        flex_row((
                            sized_box(label("Sequences")).width(100.px()),
                            riff_seq_view_riff_seq_selector(data, data.riff_sequence_view_state.riff_seq_to_select_index.clone()),
                        ))
                    ).width(200.px()),
                    sized_box(
                        flex_row((
                            sized_box(label("Riff Sets")).width(100.px()),
                            riff_seq_view_riff_set_selector(data, data.riff_sequence_view_state.add_to_seq_riff_set_index.clone()),
                        ))
                    ).width(200.px()),
                    sized_box(
                        flex_row((
                            sized_box(label("New Sequence")).width(100.px()),
                            sized_box(text_input(data.riff_sequence_view_state.add_riff_sequence_name.clone(), |state: &mut RiffDAWState, new_name| {
                                state.riff_sequence_view_state.add_riff_sequence_name = new_name;
                            })).width(200.px()),
                            button(icon(ICON_PLUS.to_string()), |state| {
                                daw_events_RiffSequenceAdd(state, Uuid::new_v4().to_string());
                                state.riff_sequence_view_state.add_riff_sequence_name.clear();
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
                    flex_row(
                        riff_seq_head_panel_sequence::<RiffDAWState>(data)
                    )
                        .main_axis_alignment(MainAxisAlignment::Start)
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .gap(1.px()),
                    flex_row(
                        riff_seq_riff_set_head_panel_sequence::<RiffDAWState>(data)
                    )
                        .main_axis_alignment(MainAxisAlignment::Start)
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .gap(1.px()),
                    flex_row(
                        riff_seq_riff_set_riffs_panel_sequence::<RiffDAWState>(data)
                    )
                        .main_axis_alignment(MainAxisAlignment::Start)
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .gap(1.px())
                )
            )
                .main_axis_alignment(MainAxisAlignment::Start)
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .gap(1.px())
        )
    ).split_point(0.2)
}