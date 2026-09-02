use itertools::Itertools;
use crate::icons::{ICON_ARROW_DOWN, ICON_ARROW_UP};
use crate::state::RiffDAWState;
use crate::views::{icon};
use masonry::properties::types::AsUnit;
use xilem::view::{button, flex_col, indexed_stack, label, sized_box, FlexSequence};
use crate::constants::MUSICAL_ITEM_LENGTH_OPTIONS;

pub fn track_grid_snap_quantize_selector(
    data: &RiffDAWState,
) -> impl FlexSequence<RiffDAWState> + use<> {
    let track_grid_selected_snap = data.track_grid_state.track_grid_selected_snap.clone();
    let options: Vec<_> = MUSICAL_ITEM_LENGTH_OPTIONS.iter().map(|snap_quantise| {
        flex_col(sized_box::<RiffDAWState, (), xilem::view::Label>(label((*snap_quantise).clone())).width(100.px()))
    }).collect();

    (
        indexed_stack(
            options
        )
        .active(track_grid_selected_snap),
        button(
            icon(ICON_ARROW_UP.to_string()),
            |state: &mut RiffDAWState| {
                let mut new_index: i32 = state.track_grid_state.track_grid_selected_snap as i32 - 1;
                if new_index < 0 {
                    new_index = 0;
                }
                state.track_grid_state.track_grid_selected_snap =  new_index as usize;
            }
        ),
        button(
            icon(ICON_ARROW_DOWN.to_string()),
            |state: &mut RiffDAWState| {
                let mut new_index = state.track_grid_state.track_grid_selected_snap + 1;
                if new_index >= MUSICAL_ITEM_LENGTH_OPTIONS.len() {
                    new_index = 0;
                }
                state.track_grid_state.track_grid_selected_snap =  new_index;
            }
        ),
    )
}
