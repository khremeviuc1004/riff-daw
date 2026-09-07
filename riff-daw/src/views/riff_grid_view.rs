use masonry::properties::types::{AsUnit, CrossAxisAlignment, MainAxisAlignment};
use uuid::Uuid;
use xilem::view::{button, flex_col, flex_row, label, sized_box, split, text_input, Flex, FlexExt, FlexSequence, FlexSpacer};
use crate::actions::{daw_events_RiffGridAdd};
use crate::event::OperationModeType;
use crate::icons::ICON_PLUS;
use crate::state::RiffDAWState;
use crate::views::{beat_grid_ruler, icon, riff_grid_view_riff_grid_selector, riff_grid_with_size, synced_scroll, track_panel_sequence};
use xilem::WidgetView;

pub fn riff_grid_toolbar(
    data: &RiffDAWState
) -> Flex<impl FlexSequence<RiffDAWState>, RiffDAWState> {
    flex_row(
        (
            FlexSpacer::Flex(1.0)
        )
    )
}


pub fn riff_grid_view(
    data: &RiffDAWState
) -> impl WidgetView<RiffDAWState, ()> + 'static {
    split (
        flex_col((
            sized_box(label("")).height(30.px()),
            synced_scroll(
                flex_col(
                    (
                        sized_box(
                            flex_row((
                                sized_box(label("Grids")).width(100.px()),
                                riff_grid_view_riff_grid_selector(data, data.riff_grid_view_state.riff_grid_to_select_index.clone()),
                            ))
                        ).width(200.px()),
                        sized_box(
                            flex_row((
                                label("New grid"),
                                sized_box(text_input(data.riff_grid_view_state.add_riff_grid_name.clone(), |state: &mut RiffDAWState, new_name| {
                                    state.riff_grid_view_state.add_riff_grid_name = new_name;
                                })).width(200.px()),
                                button(icon(ICON_PLUS.to_string()), |state| {
                                    daw_events_RiffGridAdd(state, Uuid::new_v4().to_string(), state.riff_grid_view_state.add_riff_grid_name.clone());
                                    state.riff_grid_view_state.add_riff_grid_name.clear();
                                }),
                            ))
                        ).width(200.px()),
                        track_panel_sequence::<RiffDAWState>(data, 39.px()),
                        FlexSpacer::Fixed(60000.px())
                    )
                )
                    .main_axis_alignment(MainAxisAlignment::Start)
                    .cross_axis_alignment(CrossAxisAlignment::Start),
                "riff_grid_selector_horizontal",
                "riff_grid_view_vertical"
            ),
        )),
        flex_col((
            synced_scroll(
                beat_grid_ruler(1.0, 50.0, 4, 60000.0),
                "riff_grid_horizontal",
                "riff_grid_ruler_vertical"
            ),
            synced_scroll(
                flex_row(
                    riff_grid_with_size(
                        data.project.clone(),
                        60000.0,
                        60000.0,
                        data.selected_riff_grid_riff_references.clone(),
                        OperationModeType::PointMode,
                        data.riff_grid_view_state.selected_riff_grid_uuid.clone()
                    )
                ),
                "riff_grid_horizontal",
                "riff_grid_view_vertical"
            ).flex(1.0)
        ))
            .must_fill_major_axis(true),
    ).split_point(0.2)
}