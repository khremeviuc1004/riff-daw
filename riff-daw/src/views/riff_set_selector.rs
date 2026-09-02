use xilem::view::{button, flex_col, indexed_stack, label, sized_box, FlexSequence};
use masonry::properties::types::AsUnit;
use crate::icons::{ICON_ARROW_DOWN, ICON_ARROW_UP, ICON_PLUS};
use crate::state::RiffDAWState;
use crate::views::icon;

pub fn riff_set_selector(
    data: &RiffDAWState,
    active_index: usize,
    up_change_callback: fn(&mut RiffDAWState),
    down_change_callback: fn(&mut RiffDAWState),
    select_change_callback: fn(&mut RiffDAWState)
) -> impl FlexSequence<RiffDAWState> + use<> {
    let mut options: Vec<_> = vec![];

    if let Ok(project) = data.project.lock() {
        options = project.song.riff_sets().iter().map(|riff_set| {
            flex_col(sized_box::<RiffDAWState, (), xilem::view::Label>(label((*riff_set).name().clone())).width(100.px()))
        }).collect();
    }

    (
        indexed_stack(
            options
        )
        .active(active_index),
        button(icon(ICON_ARROW_UP.to_string()), up_change_callback),
        button(icon(ICON_ARROW_DOWN.to_string()), down_change_callback),
        button(icon(ICON_PLUS.to_string()), select_change_callback),
    )
}