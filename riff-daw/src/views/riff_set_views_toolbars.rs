use xilem::view::{flex_row, indexed_stack, text_button, Flex, FlexSequence};
use masonry::properties::types::Length;
use crate::state::{RiffDAWState, RiffView};
use crate::views::{riff_arrangement_toolbar, riff_grid_toolbar, riff_sequence_view_toolbar, riff_set_view_toolbar};

pub fn riff_set_views_toolbars(
    data: &RiffDAWState,
) -> Flex<impl FlexSequence<RiffDAWState>, RiffDAWState> {
    flex_row(
        (
            text_button("Riff Set View", |state: &mut RiffDAWState| state.riff_view = RiffView::RiffSet),
            text_button("Riff Sequence View", |state: &mut RiffDAWState| state.riff_view = RiffView::RiffSequence),
            text_button("Riff Grid View", |state: &mut RiffDAWState| state.riff_view = RiffView::RiffGrid),
            text_button("Riff Arrangement View", |state: &mut RiffDAWState| state.riff_view = RiffView::RiffArrangement),
            indexed_stack(
                (
                    riff_set_view_toolbar(data),
                    riff_sequence_view_toolbar(data),
                    riff_grid_toolbar(data),
                    riff_arrangement_toolbar(data)
                )
            ).active(data.riff_view.clone() as usize)
        )
    ).gap(Length::px(10.))
}