use masonry::properties::types::AsUnit;
use xilem::core::{MessageContext, MessageResult, Mut, View, ViewMarker};
use xilem::view::{button, flex_row, Flex, FlexSequence, FlexSpacer};
use xilem::{Pod, ViewCtx};
use crate::actions::{transport_goto_end_action, transport_goto_start_action, transport_move_back_action, transport_move_forward_action, transport_pause_action, transport_play_action, transport_record_off_action, transport_record_on_action, transport_stop_action};
use crate::icons::{ICON_PLAYER_PAUSE, ICON_PLAYER_PLAY, ICON_PLAYER_RECORD, ICON_PLAYER_SKIP_BACK, ICON_PLAYER_SKIP_FORWARD, ICON_PLAYER_STOP, ICON_PLAYER_TRACK_NEXT, ICON_PLAYER_TRACK_PREV};
use crate::state::RiffDAWState;
use crate::views::icon;
use crate::views::widgets::{BarBeatDisplayWidget};

pub struct BarBeatDisplay {
    text: String,
    playing: bool,
}

pub fn bar_beat_display(text: String, playing: bool) -> BarBeatDisplay {
    BarBeatDisplay { text, playing }
}

impl ViewMarker for BarBeatDisplay {}

impl<State, Action> View<State, Action, ViewCtx> for BarBeatDisplay {
    type Element = Pod<BarBeatDisplayWidget>;
    type ViewState = ();

    fn build(&self, ctx: &mut ViewCtx, _: &mut State) -> (Self::Element, Self::ViewState) {
        let pod = ctx.create_pod(BarBeatDisplayWidget::new(self.text.clone(), self.playing));
        (pod, ())
    }

    fn rebuild(
        &self,
        prev: &Self,
        (): &mut Self::ViewState,
        _ctx: &mut ViewCtx,
        mut element: Mut<'_, Self::Element>,
        _: &mut State,
    ) {
        if prev.text != self.text {
            BarBeatDisplayWidget::set_text(&mut element, self.text.clone());
        }
        if prev.playing != self.playing {
            BarBeatDisplayWidget::set_playing(&mut element, self.playing);
        }
    }

    fn teardown(
        &self,
        (): &mut Self::ViewState,
        ctx: &mut ViewCtx,
        element: Mut<'_, Self::Element>,
    ) {
        ctx.teardown_leaf(element);
    }

    fn message(
        &self,
        (): &mut Self::ViewState,
        message: &mut MessageContext,
        _element: Mut<'_, Self::Element>,
        _app_state: &mut State,
    ) -> MessageResult<Action> {
        tracing::error!(
            ?message,
            "Message arrived in BarBeatDisplay::message, but BarBeatDisplay doesn't consume any messages, this is a bug"
        );
        MessageResult::Stale
    }
}

pub fn transport(display_text: String, playing: bool) -> Flex<impl FlexSequence<RiffDAWState>, RiffDAWState> {
    flex_row(
        (
            button(icon(ICON_PLAYER_TRACK_PREV.to_string()), |state: &mut RiffDAWState| {
                transport_goto_start_action(state);
            }),
            button(icon(ICON_PLAYER_SKIP_BACK.to_string()), |state: &mut RiffDAWState| {
                transport_move_back_action(state);
            }),
            button(icon(ICON_PLAYER_RECORD.to_string()), |state: &mut RiffDAWState| {
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
            bar_beat_display(display_text, playing),
            FlexSpacer::Flex(1.0)
        )
    )
        .gap(0.5.px())
}