//! A xilem view wrapper around the [`BeatGridRulerWidget`].
//!
//! This is the view counterpart of the gtk3 `BeatGridRuler` from `grid.rs`,
//! converted to the xilem / masonry view system. It wraps the ruler widget in a
//! `Pod` the same way the sibling [`BeatGrid`] view wraps `BeatGridWidget`.

use xilem::core::{MessageContext, MessageResult, Mut, View, ViewMarker};
use xilem::{Pod, ViewCtx};
use crate::views::widgets::BeatGridRulerWidget;

/// A horizontal beat / bar ruler that scrolls in lock-step with a `BeatGrid`.
pub struct BeatGridRuler<State, Action> {
    height: f64,
    width: f64,
    beat_width_in_pixels: f64,
    zoom_horizontal: f64,
    zoom_vertical: f64,
    beats_per_bar: i32,
    _phantom: std::marker::PhantomData<fn() -> (State, Action)>,
}

/// Builder for a `BeatGridRuler`.
///
/// * `zoom` - the (uniform) zoom level.
/// * `beat_width_in_pixels` - the width, in pixels, of a single beat at
///   zoom level 1.0.
/// * `beats_per_bar` - the number of beats in each bar (e.g. 4 for 4/4).
/// * `width` - the content width the ruler spans (so it can scroll in
///   lock-step with the associated grid).
pub fn beat_grid_ruler<State: 'static, Action: 'static>(
    zoom: f64,
    beat_width_in_pixels: f64,
    beats_per_bar: i32,
    width: f64,
) -> BeatGridRuler<State, Action> {
    BeatGridRuler {
        height: 30.0,
        width,
        beat_width_in_pixels,
        zoom_horizontal: zoom,
        zoom_vertical: zoom,
        beats_per_bar,
        _phantom: std::marker::PhantomData,
    }
}

impl<State: 'static, Action: 'static> BeatGridRuler<State, Action> {
    /// Create a new ruler with independent horizontal and vertical zoom levels.
    pub fn new_with_individual_zoom_level(
        zoom_horizontal: f64,
        zoom_vertical: f64,
        beat_width_in_pixels: f64,
        beats_per_bar: i32,
        width: f64,
    ) -> Self {
        Self {
            height: 30.0,
            width,
            beat_width_in_pixels,
            zoom_horizontal,
            zoom_vertical,
            beats_per_bar,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Set the number of beats per bar.
    pub fn set_beats_per_bar(mut self, beats_per_bar: i32) -> Self {
        self.beats_per_bar = beats_per_bar;
        self
    }

    /// Set the horizontal zoom level.
    pub fn set_horizontal_zoom(mut self, zoom: f64) -> Self {
        self.zoom_horizontal = zoom;
        self
    }

    /// Set the vertical zoom level.
    pub fn set_vertical_zoom(mut self, zoom: f64) -> Self {
        self.zoom_vertical = zoom;
        self
    }

    /// Set the ruler height.
    pub fn set_height(mut self, height: f64) -> Self {
        self.height = height;
        self
    }

    /// Set the ruler content width.
    pub fn set_width(mut self, width: f64) -> Self {
        self.width = width;
        self
    }
}

impl<State, Action> ViewMarker for BeatGridRuler<State, Action> {}

impl<State: 'static, Action: 'static> View<State, Action, ViewCtx> for BeatGridRuler<State, Action> {
    type Element = Pod<BeatGridRulerWidget>;
    type ViewState = ();

    fn build(&self, ctx: &mut ViewCtx, _app_state: &mut State) -> (Self::Element, Self::ViewState) {
        let pod = ctx.with_action_widget(|ctx| {
            ctx.create_pod(
                BeatGridRulerWidget::new_with_individual_zoom_level(
                    self.zoom_horizontal,
                    self.zoom_vertical,
                    self.beat_width_in_pixels,
                    self.beats_per_bar,
                    self.width,
                )
            )
        });
        (pod, ())
    }

    fn rebuild(
        &self,
        prev: &Self,
        _view_state: &mut Self::ViewState,
        _ctx: &mut ViewCtx,
        element: Mut<'_, Self::Element>,
        _app_state: &mut State,
    ) {
        if prev.zoom_horizontal != self.zoom_horizontal {
            element.widget.set_horizontal_zoom(self.zoom_horizontal);
        }
        if prev.zoom_vertical != self.zoom_vertical {
            element.widget.set_vertical_zoom(self.zoom_vertical);
        }
        if prev.beats_per_bar != self.beats_per_bar {
            element.widget.set_beats_per_bar(self.beats_per_bar);
        }
        if prev.width != self.width {
            element.widget.set_width(self.width);
        }
        if prev.height != self.height {
            element.widget.set_height(self.height);
        }
    }

    fn teardown(
        &self,
        _view_state: &mut Self::ViewState,
        _ctx: &mut ViewCtx,
        _element: Mut<'_, Self::Element>,
    ) {
    }

    fn message(
        &self,
        _view_state: &mut Self::ViewState,
        _message: &mut MessageContext,
        _element: Mut<'_, Self::Element>,
        _app_state: &mut State,
    ) -> MessageResult<Action> {
        MessageResult::Stale
    }
}
