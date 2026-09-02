use xilem::view::{flex_row, label, text_button, Flex, FlexSequence, FlexSpacer, portal, Portal, flex_col};
use crate::state::RiffDAWState;
use crate::views::{mixer_panel};

pub fn mixer_view_toolbar(
    data: &RiffDAWState,
) -> Flex<impl FlexSequence<RiffDAWState>, RiffDAWState> {
    flex_row(
        FlexSpacer::Flex(1.0)
    )
}



pub fn mixer_view(
    data: &RiffDAWState
) -> Portal<Flex<impl FlexSequence<RiffDAWState>, RiffDAWState>, RiffDAWState, ()> {

    portal(
        flex_col(
            mixer_panel(data)
        )
    )
}