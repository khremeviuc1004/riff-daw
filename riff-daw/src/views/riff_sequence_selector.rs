use xilem::view::{button, flex_col, indexed_stack, label, sized_box, FlexSequence};
use masonry::properties::types::AsUnit;
use crate::icons::{ICON_ARROW_DOWN, ICON_ARROW_UP, ICON_LAYERS_SELECTED_BOTTOM};
use crate::state::RiffDAWState;
use crate::views::icon;

pub fn riff_sequence_selector(
    data: &RiffDAWState,
    active_index: usize,
    up_change_callback: fn(&mut RiffDAWState),
    down_change_callback: fn(&mut RiffDAWState),
    select_change_callback: fn(&mut RiffDAWState),
    svg_icon: String
) -> impl FlexSequence<RiffDAWState> + use<> {
    let mut options: Vec<_> = vec![];

    if let Ok(project) = data.project.lock() {
        options = project.song.riff_sequences().iter().map(|riff_seq| {
            flex_col(sized_box::<RiffDAWState, (), xilem::view::Label>(label((*riff_seq).name().clone())).width(100.px()))
        }).collect();
    }

    (
        indexed_stack(
            options
        )
            .active(active_index),
        button(icon(ICON_ARROW_UP.to_string()), up_change_callback),
        button(icon(ICON_ARROW_DOWN.to_string()), down_change_callback),
        button(icon(svg_icon), select_change_callback),
    )
}