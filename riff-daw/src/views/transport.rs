use masonry::properties::types::AsUnit;
use xilem::view::{button, flex_row, Flex, FlexSequence, FlexSpacer};
use crate::actions::{transport_goto_end_action, transport_goto_start_action, transport_move_back_action, transport_move_forward_action, transport_pause_action, transport_play_action, transport_record_off_action, transport_record_on_action, transport_stop_action};
use crate::icons::{ICON_PLAYER_PAUSE, ICON_PLAYER_PLAY, ICON_PLAYER_RECORD, ICON_PLAYER_SKIP_BACK, ICON_PLAYER_SKIP_FORWARD, ICON_PLAYER_STOP, ICON_PLAYER_TRACK_NEXT, ICON_PLAYER_TRACK_PREV};
use crate::state::RiffDAWState;
use crate::views::icon;

pub fn transport() -> Flex<impl FlexSequence<RiffDAWState>, RiffDAWState> {
    flex_row(
        (
            button(icon(ICON_PLAYER_TRACK_PREV.to_string()), |state: &mut RiffDAWState| {
                transport_goto_start_action(state);
            }),
            button(icon(ICON_PLAYER_SKIP_BACK.to_string()), |state: &mut RiffDAWState| {
                transport_move_back_action(state);
            }),
            button(icon(ICON_PLAYER_RECORD.to_string()), |state: &mut RiffDAWState| {
                // state.recording = !state.recording;
                if state.recording {
                    transport_record_off_action(state);
                }
                else {
                    transport_record_on_action(state);
                }
            }),
            button(icon(ICON_PLAYER_PAUSE.to_string()), |state: &mut RiffDAWState| {
                transport_pause_action(state);
            }),
            button(icon(ICON_PLAYER_STOP.to_string()), |state: &mut RiffDAWState| {
                transport_stop_action(state);
            }),
            button(icon(ICON_PLAYER_PLAY.to_string()), |state: &mut RiffDAWState| {
                transport_play_action(state);
            }),
            button(icon(ICON_PLAYER_SKIP_FORWARD.to_string()), |state: &mut RiffDAWState| {
                transport_move_forward_action(state);
            }),
            button(icon(ICON_PLAYER_TRACK_NEXT.to_string()), |state: &mut RiffDAWState| {
                transport_goto_end_action(state);
            }),
            FlexSpacer::Flex(1.0)
        )
    )
        .gap(0.5.px())
}
