// Copyright 2024 the Xilem Authors
// SPDX-License-Identifier: Apache-2.0

use xilem::core::{MessageContext, MessageResult, Mut, View, ViewMarker};
use xilem::{Pod, ViewCtx};

use masonry::core::ArcStr;
use crate::views::widgets::{ToggleButtonToggled, ToggleButtonWidget};

pub fn toggle_button<F, State, Action>(
    icon: impl Into<ArcStr>,
    toggled: bool,
    callback: F,
) -> ToggleButtonView<F>
where
    F: Fn(&mut State, bool) -> Action + Send + 'static,
{
    ToggleButtonView {
        icon: icon.into(),
        callback,
        toggled,
        disabled: false,
    }
}

pub struct ToggleButtonView<F> {
    icon: ArcStr,
    toggled: bool,
    callback: F,
    disabled: bool,
}

impl<F> ToggleButtonView<F> {
    /// Set the disabled state of the widget.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl<F> ViewMarker for ToggleButtonView<F> {}
impl<F, State, Action> View<State, Action, ViewCtx> for ToggleButtonView<F>
where
    F: Fn(&mut State, bool) -> Action + Send + Sync + 'static,
{
    type Element = Pod<ToggleButtonWidget>;
    type ViewState = ();

    fn build(&self, ctx: &mut ViewCtx, _: &mut State) -> (Self::Element, Self::ViewState) {
        ctx.with_leaf_action_widget(|ctx| {
            let mut pod = ctx.create_pod(ToggleButtonWidget::new(self.toggled, self.icon.clone()));
            pod.new_widget.options.disabled = self.disabled;
            pod
        })
    }

    fn rebuild(
        &self,
        prev: &Self,
        (): &mut Self::ViewState,
        _ctx: &mut ViewCtx,
        mut element: Mut<'_, Self::Element>,
        _: &mut State,
    ) {
        if prev.disabled != self.disabled {
            element.ctx.set_disabled(self.disabled);
        }
        if prev.toggled != self.toggled {
            ToggleButtonWidget::set_toggled(&mut element, self.toggled);
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
        app_state: &mut State,
    ) -> MessageResult<Action> {
        debug_assert!(
            message.remaining_path().is_empty(),
            "id path should be empty in ToggleButton::message"
        );
        match message.take_message::<ToggleButtonToggled>() {
            Some(checked) => MessageResult::Action((self.callback)(app_state, checked.0)),
            None => {
                tracing::error!("Wrong message type in ToggleButton::message, got {message:?}.");
                MessageResult::Stale
            }
        }
    }
}
