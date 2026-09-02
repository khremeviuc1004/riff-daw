//! A window with three side-by-side scrollable country lists whose horizontal
//! and vertical scrolling can be synchronised independently of each other.
//!
//! Xilem 0.4's built-in `portal` view neither reports nor accepts viewport
//! offsets, so this app includes a small custom widget ([`SyncScrollArea`])
//! which scrolls in both axes, paints its own scrollbars, and submits a
//! [`Scrolled`] action whenever the user changes the viewport position.
//!
//! The [`synced_scroll`] view wraps that widget. Instead of a single shared
//! offset, each axis is keyed by a variable name stored in the app state:
//! since horizontal and vertical use separate, independent variables, the two
//! axes can be synchronised (or left independent) on a per-axis basis. When a
//! widget reports a viewport change, the view writes each axis back to its own
//! variable; on the following rebuild every view applies its combined offset
//! imperatively via [`SyncScrollArea::set_offset`], so areas sharing an axis
//! variable stay in lockstep without feedback loops (a programmatic offset
//! change emits no action).
//!
//! To demonstrate the independence, the three lists are wired as follows:
//! * "List 1" and "List 2" share the `h12` variable, so they are horizontally
//!   synchronised but scroll vertically independently.
//! * "List 2" and "List 3" share the `v23` variable, so they are vertically
//!   synchronised (List 3 also scrolls horizontally independently).
//!
//! Every other combination is possible: change a pane's variable names to
//! whatever grouping you want.

use std::collections::HashMap;
use std::iter::repeat_n;
use std::marker::PhantomData;

use masonry::accesskit::{Node, Role};
use masonry::core::{
    BoxConstraints, ChildrenIds, ComposeCtx, EventCtx, FromDynWidget, LayoutCtx, NewWidget,
    PaintCtx, PointerEvent, PointerScrollEvent, PropertiesMut, PropertiesRef, RegisterCtx,
    ScrollDelta, TextEvent, Update, UpdateCtx, Widget, WidgetMut, WidgetPod,
};
use masonry::dpi::PhysicalPosition;
use masonry::kurbo::{Point, Rect, Size, Vec2};
use masonry::peniko::Color;
use masonry::util::fill_color;
use masonry::vello::Scene;
use xilem::core::{MessageContext, MessageResult, Mut, View, ViewMarker};
use xilem::view::{CrossAxisAlignment, FlexExt as _, flex_col, flex_row, label};
use xilem::winit::error::EventLoopError;
use xilem::{EventLoop, Pod, ViewCtx, WidgetView, WindowOptions, Xilem};


/// A named per-axis scroll offset. Panes sharing the same `key` on an axis are
/// synchronised on that axis; a pane that uses its own unique key scrolls that
/// axis independently of the others.
pub trait SyncState {
    /// The current offset along the axis stored under `key`.
    fn axis(&self, key: &str) -> f64;
    /// Store a new offset `value` along the axis stored under `key`.
    fn set_axis(&mut self, key: &str, value: f64);
}


// --- MARK: SCROLL AREA WIDGET ---

/// Thickness of the painted scrollbar tracks/thumbs.
const BAR_THICKNESS: f64 = 12.0;
/// Shortest allowed scrollbar thumb length.
const MIN_THUMB_LENGTH: f64 = 32.0;
/// Pixels per "line" for line-based (mouse wheel) scroll deltas.
const LINE_SCROLL_PIXELS: f64 = 120.0;

const TRACK_COLOR: Color = Color::from_rgba8(0x80, 0x80, 0x80, 0x30);
const THUMB_COLOR: Color = Color::from_rgba8(0xA0, 0xA0, 0xA0, 0xE0);

/// Action submitted by [`SyncScrollArea`] whenever the user changes its viewport.
///
/// Carries the new viewport position in content coordinates.
#[derive(Debug)]
pub struct Scrolled(pub Point);

/// Maximum reachable viewport position for the given sizes.
fn max_offset(portal_size: Size, content_size: Size) -> Size {
    (content_size - portal_size).clamp(Size::ZERO, Size::new(f64::INFINITY, f64::INFINITY))
}

/// Scroll progress (0..=1) of an offset along one axis.
fn progress_of(offset: f64, portal_len: f64, content_len: f64) -> f64 {
    let max = (content_len - portal_len).max(0.0);
    if max > 0.0 {
        (offset / max).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Thumb length along one axis.
fn thumb_len(portal_len: f64, content_len: f64) -> f64 {
    if content_len <= 0.0 {
        return portal_len.min(MIN_THUMB_LENGTH);
    }
    let ratio = (portal_len / content_len).clamp(0.0, 1.0);
    ((ratio * portal_len).max(MIN_THUMB_LENGTH)).min(portal_len.max(0.0))
}

/// Thumb start position along one axis.
fn thumb_start(progress: f64, portal_len: f64, length: f64) -> f64 {
    progress * (portal_len - length).max(0.0)
}

/// Offset implied by a scrollbar thumb progress along one axis.
fn offset_of_progress(progress: f64, portal_len: f64, content_len: f64) -> f64 {
    progress.clamp(0.0, 1.0) * (content_len - portal_len).max(0.0)
}

/// Compute the smallest range shift so that `target` becomes visible in `viewport`.
fn compute_pan_range(
    mut viewport: std::ops::Range<f64>,
    target: std::ops::Range<f64>,
) -> std::ops::Range<f64> {
    if target.start <= viewport.start && viewport.end <= target.end {
        return viewport;
    }
    if viewport.start <= target.start && target.end <= viewport.end {
        return viewport;
    }

    let target_width = f64::min(viewport.end - viewport.start, target.end - target.start);
    let viewport_width = viewport.end - viewport.start;

    if viewport.start >= target.start {
        viewport.start = target.end - target_width;
        viewport.end = viewport.start + viewport_width;
    } else {
        viewport.end = target.start + target_width;
        viewport.start = viewport.end - viewport_width;
    }

    viewport
}

/// A two-axis scroll area which reports user-initiated viewport changes via a
/// [`Scrolled`] action, and paints its own scrollbars.
pub struct SyncScrollArea<W: Widget + ?Sized> {
    content: WidgetPod<W>,
    offset: Point,
    content_size: Size,
    v_drag_anchor: Option<f64>,
    h_drag_anchor: Option<f64>,
}

impl<W: Widget + ?Sized> SyncScrollArea<W> {
    pub fn new(content: NewWidget<W>) -> Self {
        Self {
            content: content.to_pod(),
            offset: Point::ORIGIN,
            content_size: Size::ZERO,
            v_drag_anchor: None,
            h_drag_anchor: None,
        }
    }

    /// Set `self.offset` to `pos`, clamped to the valid range. Returns whether it changed.
    fn set_offset_raw(&mut self, portal_size: Size, content_size: Size, pos: Point) -> bool {
        let max = max_offset(portal_size, content_size);
        let pos = Point::new(pos.x.clamp(0.0, max.width), pos.y.clamp(0.0, max.height));
        if (pos - self.offset).hypot2() > 1e-12 {
            self.offset = pos;
            true
        } else {
            false
        }
    }

    /// Apply an interactively requested offset, notifying the app if it changed.
    fn apply_interactive_offset(&mut self, ctx: &mut EventCtx<'_>, pos: Point) {
        if self.set_offset_raw(ctx.size(), self.content_size, pos) {
            ctx.request_compose();
            ctx.request_render();
            ctx.submit_action::<Scrolled>(Scrolled(self.offset));
        }
    }
}

// --- MARK: WIDGETMUT ---

impl<W: Widget + FromDynWidget + ?Sized> SyncScrollArea<W> {
    pub fn content_mut<'t>(this: &'t mut WidgetMut<'_, Self>) -> WidgetMut<'t, W> {
        this.ctx.get_mut(&mut this.widget.content)
    }

    /// The current viewport position.
    pub fn get_offset(this: &WidgetMut<'_, Self>) -> Point {
        this.widget.offset
    }

    /// Imperatively set the viewport position (clamped to the valid range).
    ///
    /// This emits no action, which is what lets the sibling list follow the
    /// shared offset without creating a feedback loop.
    pub fn set_offset(this: &mut WidgetMut<'_, Self>, pos: Point) {
        let portal_size = this.ctx.size();
        let content_size = this.ctx.get_mut(&mut this.widget.content).ctx.size();
        if this.widget.set_offset_raw(portal_size, content_size, pos) {
            this.ctx.request_compose();
            this.ctx.request_render();
        }
    }
}

// --- MARK: IMPL WIDGET ---

impl<W: Widget + FromDynWidget + ?Sized> Widget for SyncScrollArea<W> {
    type Action = Scrolled;

    fn on_pointer_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &PointerEvent,
    ) {
        match event {
            PointerEvent::Scroll(PointerScrollEvent { delta, .. }) => {
                let delta = match *delta {
                    ScrollDelta::PixelDelta(PhysicalPosition::<f64> { x, y }) => -Vec2 { x, y },
                    ScrollDelta::LineDelta(x, y) => {
                        -Vec2 {
                            x: x as f64,
                            y: y as f64,
                        } * LINE_SCROLL_PIXELS
                    }
                    _ => Vec2::ZERO,
                } * ctx.get_scale_factor();
                self.apply_interactive_offset(ctx, self.offset + delta);
                self.apply_interactive_offset(ctx, self.offset + delta);
            }
            PointerEvent::Down(button) => {
                let pos = ctx.local_position(button.state.position);
                let size = ctx.size();

                let v_track = Rect::new(size.width - BAR_THICKNESS, 0.0, size.width, size.height);
                let h_track = Rect::new(0.0, size.height - BAR_THICKNESS, size.width, size.height);

                if v_track.contains(pos) && self.content_size.height > size.height {
                    let length = thumb_len(size.height, self.content_size.height);
                    let progress =
                        progress_of(self.offset.y, size.height, self.content_size.height);
                    let start = thumb_start(progress, size.height, length);
                    // Distance from the thumb's leading edge to the grab point.
                    self.v_drag_anchor = Some(pos.y - start);
                    ctx.capture_pointer();
                    ctx.request_render();
                } else if h_track.contains(pos) && self.content_size.width > size.width {
                    let length = thumb_len(size.width, self.content_size.width);
                    let progress = progress_of(self.offset.x, size.width, self.content_size.width);
                    let start = thumb_start(progress, size.width, length);
                    self.h_drag_anchor = Some(pos.x - start);
                    ctx.capture_pointer();
                    ctx.request_render();
                }
            }
            PointerEvent::Move(update) => {
                if self.v_drag_anchor.is_none() && self.h_drag_anchor.is_none() {
                    return;
                }
                let pos = ctx.local_position(update.current.position);
                let size = ctx.size();

                let mut target = self.offset;
                if let Some(anchor) = self.v_drag_anchor {
                    let length = thumb_len(size.height, self.content_size.height);
                    let travel = (size.height - length).max(1.0);
                    let start = (pos.y - anchor).clamp(0.0, travel);
                    target.y =
                        offset_of_progress(start / travel, size.height, self.content_size.height);
                }
                if let Some(anchor) = self.h_drag_anchor {
                    let length = thumb_len(size.width, self.content_size.width);
                    let travel = (size.width - length).max(1.0);
                    let start = (pos.x - anchor).clamp(0.0, travel);
                    target.x =
                        offset_of_progress(start / travel, size.width, self.content_size.width);
                }
                self.apply_interactive_offset(ctx, target);
            }
            PointerEvent::Up(..) | PointerEvent::Cancel(..)
            if self.v_drag_anchor.take().is_some() || self.h_drag_anchor.take().is_some() =>
                {
                    ctx.request_render();
                }
            _ => (),
        }
    }

    fn on_text_event(
        &mut self,
        _ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        _event: &TextEvent,
    ) {
    }

    fn update(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, event: &Update) {
        if let Update::RequestPanToChild(target) = event {
            let portal_size = ctx.size();
            let viewport = Rect::from_origin_size(self.offset, portal_size);
            let new_x = compute_pan_range(
                viewport.min_x()..viewport.max_x(),
                target.min_x()..target.max_x(),
            )
                .start;
            let new_y = compute_pan_range(
                viewport.min_y()..viewport.max_y(),
                target.min_y()..target.max_y(),
            )
                .start;
            if self.set_offset_raw(portal_size, self.content_size, Point::new(new_x, new_y)) {
                ctx.request_compose();
                ctx.request_render();
                ctx.submit_action::<Scrolled>(Scrolled(self.offset));
            }
        }
    }

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        ctx.register_child(&mut self.content);
    }

    fn layout(
        &mut self,
        ctx: &mut LayoutCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        bc: &BoxConstraints,
    ) -> Size {
        // Measure the content against unconstrained bounds: widgets such as
        // `Label` clamp themselves to the given maximum, which would collapse
        // the content to the viewport size and make it impossible to scroll.
        let child_bc = BoxConstraints::new(Size::ZERO, Size::new(f64::INFINITY, f64::INFINITY));
        let content_size = ctx.run_layout(&mut self.content, &child_bc);
        let portal_size = bc.constrain(content_size);
        self.content_size = content_size;

        // Re-clamp the offset against the (possibly changed) sizes.
        self.set_offset_raw(portal_size, content_size, self.offset);

        ctx.set_clip_path(portal_size.to_rect());
        ctx.place_child(&mut self.content, Point::ZERO);

        portal_size
    }

    fn compose(&mut self, ctx: &mut ComposeCtx<'_>) {
        ctx.set_child_scroll_translation(
            &mut self.content,
            Vec2::new(-self.offset.x, -self.offset.y),
        );
    }

    fn paint(&mut self, _ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, _scene: &mut Scene) {}

    fn post_paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        _props: &PropertiesRef<'_>,
        scene: &mut Scene,
    ) {
        let size = ctx.size();

        if self.content_size.height > size.height {
            fill_color(
                scene,
                &Rect::new(size.width - BAR_THICKNESS, 0.0, size.width, size.height),
                TRACK_COLOR,
            );
            let length = thumb_len(size.height, self.content_size.height);
            let progress = progress_of(self.offset.y, size.height, self.content_size.height);
            let start = thumb_start(progress, size.height, length);
            fill_color(
                scene,
                &Rect::new(
                    size.width - BAR_THICKNESS,
                    start,
                    size.width,
                    start + length,
                ),
                THUMB_COLOR,
            );
        }

        if self.content_size.width > size.width {
            fill_color(
                scene,
                &Rect::new(0.0, size.height - BAR_THICKNESS, size.width, size.height),
                TRACK_COLOR,
            );
            let length = thumb_len(size.width, self.content_size.width);
            let progress = progress_of(self.offset.x, size.width, self.content_size.width);
            let start = thumb_start(progress, size.width, length);
            fill_color(
                scene,
                &Rect::new(
                    start,
                    size.height - BAR_THICKNESS,
                    start + length,
                    size.height,
                ),
                THUMB_COLOR,
            );
        }
    }

    fn accessibility_role(&self) -> Role {
        Role::GenericContainer
    }

    fn accessibility(
        &mut self,
        _ctx: &mut masonry::core::AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        node: &mut Node,
    ) {
        node.set_clips_children();
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::from_slice(&[self.content.id()])
    }
}

// --- MARK: VIEW ---

/// The view type for [`synced_scroll`].
pub struct SyncedScroll<State, Action, V> {
    child: V,
    /// The state key for this area's horizontal offset.
    x_var: &'static str,
    /// The state key for this area's vertical offset.
    y_var: &'static str,
    phantom: PhantomData<fn() -> (State, Action)>,
}

/// A two-axis scroll area around `child`, synchronised per axis through the app state.
///
/// The horizontal axis reads from and writes to the state variable `x_var`
/// and the vertical axis to `y_var`. Areas that share the same variable name
/// on an axis move in lockstep on that axis; an area using a unique variable
/// name scrolls that axis independently. Because horizontal and vertical use
/// separate variables, syncing on one axis never affects the other.
///
/// When the user scrolls this area, each axis's new position is written back
/// to its own variable. On every rebuild, the area is moved to the combined
/// offset from `x_var`/`y_var` if it differs from its current position. Since
/// programmatic moves emit no action, this cannot loop.
pub fn synced_scroll<State, Action, V>(
    child: V,
    x_var: &'static str,
    y_var: &'static str,
) -> SyncedScroll<State, Action, V>
where
    State: SyncState + 'static,
    Action: 'static,
    V: WidgetView<State, Action>,
{
    SyncedScroll {
        child,
        x_var,
        y_var,
        phantom: PhantomData,
    }
}

impl<State, Action, V> ViewMarker for SyncedScroll<State, Action, V> {}

impl<State, Action, V> View<State, Action, ViewCtx> for SyncedScroll<State, Action, V>
where
    State: SyncState + 'static,
    Action: 'static,
    V: WidgetView<State, Action>,
{
    type Element = Pod<SyncScrollArea<V::Widget>>;
    type ViewState = V::ViewState;

    fn build(&self, ctx: &mut ViewCtx, app_state: &mut State) -> (Self::Element, Self::ViewState) {
        let (child, child_state) = self.child.build(ctx, app_state);
        let element = ctx.with_action_widget(|_| Pod::new(SyncScrollArea::new(child.new_widget)));
        (element, child_state)
    }

    fn rebuild(
        &self,
        prev: &Self,
        child_state: &mut Self::ViewState,
        ctx: &mut ViewCtx,
        mut element: Mut<'_, Self::Element>,
        app_state: &mut State,
    ) {
        let desired = Point::new(app_state.axis(self.x_var), app_state.axis(self.y_var));
        if SyncScrollArea::get_offset(&element) != desired {
            SyncScrollArea::set_offset(&mut element, desired);
        }

        let child_element = SyncScrollArea::content_mut(&mut element);
        self.child
            .rebuild(&prev.child, child_state, ctx, child_element, app_state);
    }

    fn teardown(
        &self,
        child_state: &mut Self::ViewState,
        ctx: &mut ViewCtx,
        mut element: Mut<'_, Self::Element>,
    ) {
        let child_element = SyncScrollArea::content_mut(&mut element);
        self.child.teardown(child_state, ctx, child_element);
    }

    fn message(
        &self,
        child_state: &mut Self::ViewState,
        message: &mut MessageContext,
        mut element: Mut<'_, Self::Element>,
        app_state: &mut State,
    ) -> MessageResult<Action> {
        if message.take_first().is_some() {
            let child_element = SyncScrollArea::content_mut(&mut element);
            return self
                .child
                .message(child_state, message, child_element, app_state);
        }

        match message.take_message::<Scrolled>() {
            Some(scrolled) => {
                // Route each axis to its own independent variable, then request a
                // rebuild so the sibling panes that share those variables are moved
                // into lockstep. We mutate state directly, so we don't re-run the
                // app logic; RequestRebuild re-applies the (already updated) state.
                let pos = scrolled.0;
                app_state.set_axis(self.x_var, pos.x);
                app_state.set_axis(self.y_var, pos.y);
                MessageResult::RequestRebuild
            }
            None => {
                tracing::error!("Wrong message type in SyncedScroll::message");
                MessageResult::Stale
            }
        }
    }
}
