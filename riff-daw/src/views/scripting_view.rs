use xilem::view::{flex_row, label, text_button, Flex, FlexSequence, FlexSpacer, Label};
use crate::state::RiffDAWState;



pub fn scripting_view_toolbar(
    data: &RiffDAWState,
) -> Flex<impl FlexSequence<RiffDAWState>, RiffDAWState> {
    flex_row(
        (
            text_button("5", |state| println!("Tool bar button")),
            text_button("5", |state| println!("Tool bar button")),
            text_button("5", |state| println!("Tool bar button")),
            text_button("5", |state| println!("Tool bar button")),
            text_button("5", |state| println!("Tool bar button")),
            text_button("5", |state| println!("Tool bar button")),
            text_button("5", |state| println!("Tool bar button")),
            text_button("5", |state| println!("Tool bar button")),
            FlexSpacer::Flex(1.0)
        )
    )
}


pub fn scripting_view(
    data: &RiffDAWState
) -> Label {
    label("Scripting")
}