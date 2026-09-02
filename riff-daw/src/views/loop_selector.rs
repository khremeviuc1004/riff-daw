use crate::icons::{ICON_ARROW_DOWN, ICON_ARROW_UP, ICON_MINUS, ICON_PLUS, ICON_REPEAT, ICON_REPEAT_OFF};
use crate::state::RiffDAWState;
use crate::views::{icon};
use masonry::properties::types::AsUnit;
use uuid::Uuid;
use xilem::view::{button, flex_col, indexed_stack, label, sized_box, FlexSequence};
use crate::actions::{daw_events_LoopChange};
use crate::event::LoopChangeType;

pub fn loop_selector(
    data: &mut RiffDAWState,
) -> impl FlexSequence<RiffDAWState> + use<> {
    let selected_loop = data.selected_loop.clone();
    let mut options: Vec<_> = vec![];
    let looping = data.looping.clone();

    if let Ok(project) = data.project.lock() {
        options = project.song.loops().iter().map(|song_loop| {
            let mut loop_name = (*song_loop).name().clone().to_string();
            loop_name.push_str(" ");
            loop_name.push_str(song_loop.start_position.to_string().as_str());
            loop_name.push_str(":");
            loop_name.push_str(song_loop.start_position.to_string().as_str());
            flex_col(sized_box::<RiffDAWState, (), xilem::view::Label>(label(loop_name)).width(100.px()))
        }).collect();
    }

    if options.is_empty() {
        options.push(flex_col(sized_box::<RiffDAWState, (), xilem::view::Label>(label("None")).width(100.px())));
    }

    (
        label("Loop"),
        indexed_stack(
            options
        )
        .active(if selected_loop == usize::MAX { 0 } else {selected_loop}),
        button(
            icon(ICON_ARROW_UP.to_string()),
            |state: &mut RiffDAWState| {
                if state.selected_loop != usize::MAX {
                    let mut new_index: i32 = state.selected_loop as i32 - 1;
                    if new_index < 0 {
                        new_index = 0;
                    }
                    state.selected_loop = new_index as usize;
                }
            }
        ),
        button(
            icon(ICON_ARROW_DOWN.to_string()),
            |state: &mut RiffDAWState| {
                if state.selected_loop != usize::MAX {
                    if let Ok(project) = state.project.lock() {
                        let mut new_index = state.selected_loop + 1;
                        if new_index >= project.song.loops().len() {
                            new_index = 0;
                        }
                        state.selected_loop = new_index;
                    }
                }
            }
        ),
        button(icon(ICON_PLUS.to_string()), |state: &mut RiffDAWState| {
            let uuid = Uuid::new_v4();
            println!("Add a new loop: {}", uuid.to_string());
            daw_events_LoopChange(state, LoopChangeType::Added("Loop".to_string()), uuid);
        }),
        button(icon(ICON_MINUS.to_string()), |state: &mut RiffDAWState| {
            if state.selected_loop != usize::MAX {
                // FIXME loop delete
                // let uuid = if let Ok(project) = state.get_project().lock() {
                //     if let Some(song_loop) = project.song.loops_mut().get(&state.selected_loop) {
                //         Some(song_loop.)
                //     }
                //     else None
                // }
                // else None;
                println!("Deleting the selected loop: {:?}", state.selected_loop);
                // daw_events_LoopChange(state, LoopChangeType::Deleted, );
            }
        }),
        button(icon(if looping { ICON_REPEAT.to_string() } else { ICON_REPEAT_OFF.to_string() }), move|state: &mut RiffDAWState| {
            println!("Toggling loop mode on/off: {:?}", !looping);
            daw_events_LoopChange(state, if looping { LoopChangeType::LoopOff } else { LoopChangeType::LoopOn }, Uuid::new_v4());
        }),
    )
}
