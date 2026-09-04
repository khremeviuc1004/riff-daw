use masonry::properties::types::{AsUnit, CrossAxisAlignment, MainAxisAlignment};
use uuid::Uuid;
use xilem::view::{button, flex_col, flex_row, label, portal, sized_box, split, text_input, Flex, FlexExt, FlexSequence, FlexSpacer};
use crate::actions::daw_events_RiffSetAdd;
use crate::icons::ICON_PLUS;
use crate::state::RiffDAWState;
use crate::views::{icon, riff_set_head_panel_sequence, riff_set_riffs_panel_sequence, track_panel_sequence};



pub fn riff_set_view_toolbar(
    data: &RiffDAWState
) -> Flex<impl FlexSequence<RiffDAWState>, RiffDAWState> {
    flex_row(
        (
            FlexSpacer::Flex(1.0)
        )
    )
}

pub fn riff_set_view(data: &RiffDAWState) -> Flex<impl FlexSequence<RiffDAWState>, RiffDAWState> {
    flex_col(
        (
            split (
                sized_box(
                    flex_row(
                        (
                            label("New riff set"),
                            sized_box(text_input(data.riff_set_view_state.add_riff_set_name.clone(), |state: &mut RiffDAWState, new_name| {
                                state.riff_set_view_state.add_riff_set_name = new_name;
                            })).width(200.px()),
                            button(icon(ICON_PLUS.to_string()), |state: &mut RiffDAWState| {
                                daw_events_RiffSetAdd(state, Uuid::new_v4(), state.riff_set_view_state.add_riff_set_name.clone());
                                state.riff_set_view_state.add_riff_set_name.clear();
                            }),
                        )
                    ).main_axis_alignment(MainAxisAlignment::Start)
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                ).width(200.px()).height(80.px()),
                portal(
                    flex_row(
                        (
                            riff_set_head_panel_sequence::<RiffDAWState>(data),
                            FlexSpacer::Fixed(60000.px())
                        )
                    )
                        .main_axis_alignment(MainAxisAlignment::Start)
                        .cross_axis_alignment(CrossAxisAlignment::Start),
                ),
            ).split_point(0.2).flex(1.0),
            split (
                portal(
                    flex_col(
                        (
                            track_panel_sequence::<RiffDAWState>(data, 39.px()),
                            FlexSpacer::Fixed(60000.px())
                        )
                    )
                        .main_axis_alignment(MainAxisAlignment::Start)
                        .cross_axis_alignment(CrossAxisAlignment::Start),
                ),
                portal(
                    flex_col(
                        riff_set_riffs_panel_sequence::<RiffDAWState>(data)
                    )
                )
            ).split_point(0.2).flex(1.0),
        )
    )
        .must_fill_major_axis(true)
}