use std::ops::Range;
use crate::icons::{ICON_ARROW_DOWN, ICON_ARROW_UP};
use crate::state::RiffDAWState;
use crate::views::{icon};
use masonry::properties::types::AsUnit;
use xilem::view::{button, flex_col, indexed_stack, label, sized_box, FlexSequence};

pub fn generic_selector(
    items: Vec<&str>,
    selected_index: usize,
    up_callback: fn(&mut RiffDAWState),
    down_callback: fn(&mut RiffDAWState),
) -> impl FlexSequence<RiffDAWState> + use<> {
    let options: Vec<_> = items.iter().map(|item_name| {
        flex_col(sized_box::<RiffDAWState, (), xilem::view::Label>(label((*item_name).clone())).width(100.px()))
    }).collect();

    (
        indexed_stack(
            options
        )
        .active(selected_index),
        button(icon(ICON_ARROW_UP.to_string()), up_callback),
        button(icon(ICON_ARROW_DOWN.to_string()), down_callback),
    )
}

pub fn generic_number_selector(
    items: Range<i32>,
    selected_index: usize,
    up_callback: fn(&mut RiffDAWState),
    down_callback: fn(&mut RiffDAWState),
) -> impl FlexSequence<RiffDAWState> + use<> {
    let options: Vec<_> = items.map(|item_name| {
        flex_col(sized_box::<RiffDAWState, (), xilem::view::Label>(label((item_name.to_string().as_str()).clone())).width(100.px()))
    }).collect();

    (
        indexed_stack(
            options
        )
            .active(selected_index),
        button(icon(ICON_ARROW_UP.to_string()), up_callback),
        button(icon(ICON_ARROW_DOWN.to_string()), down_callback),
    )
}
