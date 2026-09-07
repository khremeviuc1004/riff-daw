use masonry::properties::types::{AsUnit, CrossAxisAlignment, MainAxisAlignment};
use uuid::Uuid;
use xilem::view::{button, flex_col, flex_row, label, sized_box, split, text_input, Flex, FlexSequence, FlexSpacer};
use crate::actions::daw_events_RiffSequenceAdd;
use crate::icons::ICON_PLUS;
use crate::state::RiffDAWState;
use crate::views::{icon, riff_seq_head_panel_sequence, riff_seq_riff_set_head_panel_sequence, riff_seq_riff_set_riffs_panel_sequence, riff_seq_view_riff_seq_selector, riff_seq_view_riff_set_selector, synced_scroll, track_panel_sequence};
use xilem::WidgetView;


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
    -> impl WidgetView<RiffDAWState, ()> + 'static {
    split (
        synced_scroll(
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
            "riff_seq_selector_horizontal",
            "riff_seq_view_vertical"
        ),
        synced_scroll(
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
                .gap(1.px()),
            "riff_seq_content_horizontal",
            "riff_seq_view_vertical"
        )
    ).split_point(0.2)
}