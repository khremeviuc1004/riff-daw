use std::cell::{Cell, RefCell};
use std::fmt::Debug;
use std::rc::Rc;

use masonry::accesskit::{Node, Role};
use masonry::core::{
    keyboard::{Key, KeyState, NamedKey},
    AccessCtx, BoxConstraints, ChildrenIds, EventCtx, KeyboardEvent, LayoutCtx, NewWidget,
    NoAction, PaintCtx, PointerEvent, PropertiesMut, PropertiesRef, RegisterCtx, ScrollDelta,
    StyleProperty, TextEvent, Update, UpdateCtx, Widget, WidgetId, WidgetMut, WidgetPod,
};
use masonry::core::Properties;
use masonry::kurbo::{Affine, Point, Rect, Size};
use masonry::properties::ContentColor;
use masonry::vello::peniko::Color;
use masonry::vello::Scene;
use masonry::widgets::Label;
use tracing::{Span, trace_span};

use xilem::{EventLoop, Pod, ViewCtx, WidgetView, WindowOptions, Xilem};
use xilem::core::{MessageContext, MessageResult, Mut, View, ViewMarker};

// ── Layout constants ──────────────────────────────────────────────

const PANEL_WIDTH: f64 = 220.0;
const FILTER_BOX_HEIGHT: f64 = 34.0;
const ROW_HEIGHT: f64 = 28.0;
const MAX_VISIBLE_ROWS: usize = 6;
const SCROLLBAR_WIDTH: f64 = 6.0;
const FONT_SIZE: f32 = 14.0;

// ── Shared state for cross-layer communication ────────────────────

struct SharedState {
    selection: Cell<Option<usize>>,
    overlay_id: Cell<Option<WidgetId>>,
    /// Row index (within the filtered list) currently highlighted.
    highlight: Cell<usize>,
    /// Keyboard events captured by the focused button while the overlay is
    /// open. Drained by the panel on every animation frame.
    key_queue: RefCell<Vec<KeyboardEvent>>,
}

// ── DropdownItem: clickable row in the overlay ────────────────────

struct DropdownItem {
    text: String,
    /// Index into the full item list (what gets reported on selection, and
    /// used to check whether this row is highlighted).
    index: usize,
    shared: Rc<SharedState>,
}

impl Widget for DropdownItem {
    type Action = NoAction;

    fn on_pointer_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &PointerEvent,
    ) {
        if let PointerEvent::Down(event) = event
            && event.button == Some(masonry::core::PointerButton::Primary)
        {
            self.shared.selection.set(Some(self.index));
            if let Some(overlay_id) = self.shared.overlay_id.take() {
                ctx.remove_layer(overlay_id);
            }
        }
    }

    fn layout(
        &mut self,
        _ctx: &mut LayoutCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        bc: &BoxConstraints,
    ) -> Size {
        bc.constrain(Size::new(PANEL_WIDTH, ROW_HEIGHT))
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, scene: &mut Scene) {
        let size = ctx.size();
        let rect = size.to_rect();

        let is_highlighted = self.shared.highlight.get() == self.index;
        let bg_color = if is_highlighted || ctx.is_hovered() {
            masonry::vello::peniko::Color::from_rgb8(208, 222, 246)
        } else {
            masonry::vello::peniko::Color::WHITE
        };

        scene.fill(
            masonry::vello::peniko::Fill::NonZero,
            Affine::IDENTITY,
            bg_color,
            Some(Affine::IDENTITY),
            &rect,
        );

        let brush = masonry::vello::peniko::Brush::Solid(masonry::vello::peniko::Color::BLACK);
        let (fcx, lcx) = ctx.text_contexts();
        let mut builder = lcx.ranged_builder(fcx, &self.text, 1.0, true);
        builder.push_default(masonry::core::StyleProperty::FontSize(FONT_SIZE));
        let mut text_layout = builder.build(&self.text);
        text_layout.break_all_lines(None);
        text_layout.align(None, masonry::TextAlign::Start, masonry::TextAlignOptions::default());

        let text_x = 10.0;
        let text_y = (size.height - FONT_SIZE as f64) / 2.0;

        masonry::core::render_text(
            scene,
            Affine::translate((text_x, text_y)),
            &text_layout,
            &[brush],
            true,
        );
    }

    fn accessibility_role(&self) -> Role {
        Role::ListItem
    }

    fn accessibility(
        &mut self,
        _ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        node: &mut Node,
    ) {
        node.set_label(self.text.clone());
        node.add_action(masonry::accesskit::Action::Click);
    }

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }

    fn make_trace_span(&self, id: WidgetId) -> Span {
        trace_span!("DropdownItem", id = id.trace())
    }
}

// ── DropdownPanel: overlay root with filter box + scrollable list ──

struct DropdownPanel<T> {
    shared: Rc<SharedState>,
    /// Items in order of appearance; the tuple holds (value, display text).
    items: Vec<(T, String)>,
    /// Current filter query typed into the filter box.
    filter: String,
    /// One widget pod per item, in the original item order. Rows that don't
    /// match the filter are *stashed* rather than removed, so the child list
    /// never changes while the overlay is open.
    pods: Vec<WidgetPod<DropdownItem>>,
    /// Indices into `items` matching the current filter, in display order.
    filtered: Vec<usize>,
    /// Highlighted row position within `filtered`.
    highlighted_row: usize,
    scroll_offset: f64,
    viewport_height: f64,
    /// Size the panel wants to be. Layer roots receive the window's tight
    /// constraints, so the real layout size can be larger; painting and
    /// hit-testing must stay within this rect.
    visual_size: Size,
}

impl<T: 'static> DropdownPanel<T> {
    fn new(shared: Rc<SharedState>, items: Vec<(T, String)>) -> Self {
        shared.highlight.set(0);
        let pods = items
            .iter()
            .enumerate()
            .map(|(index, (_, text))| {
                WidgetPod::new(DropdownItem {
                    text: text.clone(),
                    index,
                    shared: Rc::clone(&shared),
                })
            })
            .collect();
        Self {
            shared,
            items,
            filter: String::new(),
            pods,
            filtered: Vec::new(),
            highlighted_row: 0,
            scroll_offset: 0.0,
            viewport_height: MAX_VISIBLE_ROWS as f64 * ROW_HEIGHT,
            visual_size: Size::ZERO,
        }
            .with_all_visible()
    }

    fn with_all_visible(mut self) -> Self {
        self.filtered = (0..self.items.len()).collect();
        self
    }

    /// Rebuild the visible rows so they only contain items matching `filter`.
    ///
    /// Non-matching rows are stashed (hidden from paint, pointer events and
    /// the accessibility tree); the child list itself never changes.
    fn apply_filter(&mut self, ctx: &mut UpdateCtx<'_>) {
        let needle = self.filter.to_lowercase();
        self.filtered.clear();
        for (i, (_, text)) in self.items.iter().enumerate() {
            if needle.is_empty() || text.to_lowercase().contains(&needle) {
                self.filtered.push(i);
            }
        }

        let mut visible = vec![false; self.pods.len()];
        for &index in &self.filtered {
            visible[index] = true;
        }
        for (index, pod) in self.pods.iter_mut().enumerate() {
            ctx.set_stashed(pod, !visible[index]);
        }

        self.highlighted_row = 0;
        if let Some(&first) = self.filtered.first() {
            self.shared.highlight.set(first);
        }
        self.scroll_offset = 0.0;
        ctx.request_layout();
    }

    fn select_row(&mut self, row: usize, ctx: &mut UpdateCtx<'_>) {
        let Some(&index) = self.filtered.get(row) else {
            return;
        };
        self.shared.selection.set(Some(index));
        self.close(ctx);
    }

    fn close(&mut self, ctx: &mut UpdateCtx<'_>) {
        if let Some(overlay_id) = self.shared.overlay_id.take() {
            ctx.remove_layer(overlay_id);
        }
    }

    /// Scroll (if needed) so the highlighted row is fully visible.
    fn ensure_row_visible(&mut self) {
        let top = self.highlighted_row as f64 * ROW_HEIGHT;
        let bottom = top + ROW_HEIGHT;
        if top < self.scroll_offset {
            self.scroll_offset = top;
        } else if bottom > self.scroll_offset + self.viewport_height {
            self.scroll_offset = bottom - self.viewport_height;
        }
    }

    /// Move the highlight to `row` (a position within `filtered`).
    fn move_highlight(&mut self, row: usize) {
        self.highlighted_row = row;
        if let Some(&index) = self.filtered.get(row) {
            self.shared.highlight.set(index);
        }
        self.ensure_row_visible();
    }

    fn max_scroll_offset(&self) -> f64 {
        (self.filtered.len() as f64 * ROW_HEIGHT - self.viewport_height).max(0.0)
    }

    /// Consume a keyboard event forwarded by the button, updating
    /// filter/highlight/selection state. Returns `true` if handled.
    fn process_key(&mut self, key_event: &KeyboardEvent, ctx: &mut UpdateCtx<'_>) -> bool {
        if key_event.state != KeyState::Down {
            return false;
        }
        match &key_event.key {
            Key::Character(text)
            if !key_event.modifiers.ctrl()
                && !key_event.modifiers.alt()
                && !key_event.modifiers.meta() =>
                {
                    self.filter.push_str(text.as_str());
                    self.apply_filter(ctx);
                    true
                }
            Key::Named(NamedKey::Backspace) => {
                self.filter.pop();
                self.apply_filter(ctx);
                true
            }
            Key::Named(NamedKey::Escape) => {
                if self.filter.is_empty() {
                    self.close(ctx);
                } else {
                    self.filter.clear();
                    self.apply_filter(ctx);
                }
                true
            }
            Key::Named(NamedKey::Enter) => {
                self.select_row(self.highlighted_row, ctx);
                true
            }
            Key::Named(NamedKey::ArrowDown)
            if self.highlighted_row + 1 < self.filtered.len() =>
                {
                    self.move_highlight(self.highlighted_row + 1);
                    ctx.request_layout();
                    true
                }
            Key::Named(NamedKey::ArrowUp) if self.highlighted_row > 0 => {
                self.move_highlight(self.highlighted_row - 1);
                ctx.request_layout();
                true
            }
            _ => false,
        }
    }
}

impl<T: 'static> Widget for DropdownPanel<T> {
    type Action = NoAction;

    fn on_pointer_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &PointerEvent,
    ) {
        if let PointerEvent::Scroll(scroll) = event {
            let delta = match scroll.delta {
                ScrollDelta::PixelDelta(pos) => -pos.y,
                ScrollDelta::LineDelta(_, y) => -y as f64 * ROW_HEIGHT * 3.0,
                ScrollDelta::PageDelta(_, y) => -y as f64 * self.viewport_height,
            };
            self.scroll_offset =
                (self.scroll_offset + delta).clamp(0.0, self.max_scroll_offset());
            ctx.request_layout();
        }
    }

    /// Keyboard input is forwarded by the focused button into the shared key
    /// queue; this pump drains it once per animation frame while open.
    fn on_anim_frame(
        &mut self,
        ctx: &mut UpdateCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        _interval: u64,
    ) {
        let keys: Vec<KeyboardEvent> = self.shared.key_queue.borrow_mut().drain(..).collect();
        for key_event in &keys {
            if self.process_key(key_event, ctx) {
                ctx.request_render();
            }
        }
        if self.shared.overlay_id.get().is_some() {
            ctx.request_anim_frame();
        }
    }

    fn update(
        &mut self,
        ctx: &mut UpdateCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &Update,
    ) {
        if let Update::WidgetAdded = event {
            // Start pumping forwarded keyboard events.
            ctx.request_anim_frame();
        }
    }

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        for pod in &mut self.pods {
            ctx.register_child(pod);
        }
    }

    fn layout(
        &mut self,
        ctx: &mut LayoutCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        bc: &BoxConstraints,
    ) -> Size {
        let visible_rows = self.filtered.len().min(MAX_VISIBLE_ROWS);
        let visual_size =
            Size::new(PANEL_WIDTH, FILTER_BOX_HEIGHT + visible_rows as f64 * ROW_HEIGHT);
        self.visual_size = visual_size;
        let size = bc.constrain(visual_size);
        // Scroll math uses the *visual* height, not the constrained one.
        self.viewport_height = (visual_size.height - FILTER_BOX_HEIGHT).max(0.0);

        self.scroll_offset = self.scroll_offset.clamp(0.0, self.max_scroll_offset());

        let child_bc = BoxConstraints::tight(Size::new(visual_size.width, ROW_HEIGHT));
        for (row, &index) in self.filtered.iter().enumerate() {
            let pod = &mut self.pods[index];
            ctx.run_layout(pod, &child_bc);
            let y = FILTER_BOX_HEIGHT + row as f64 * ROW_HEIGHT - self.scroll_offset;
            ctx.place_child(pod, Point::new(0.0, y));
        }

        // Clip the rows to the list viewport (does not affect this widget's own paint).
        ctx.set_clip_path(Rect::new(
            0.0,
            FILTER_BOX_HEIGHT,
            visual_size.width,
            FILTER_BOX_HEIGHT + self.viewport_height,
        ));

        size
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, scene: &mut Scene) {
        // The layer may be stretched beyond our visual size by tight window
        // constraints; only paint within the visual rect.
        let size = self.visual_size;
        if size.is_zero_area() {
            return;
        }
        let white = masonry::vello::peniko::Color::WHITE;
        let border = masonry::vello::peniko::Color::from_rgb8(128, 128, 128);

        // Outer border around the whole panel.
        scene.stroke(
            &masonry::kurbo::Stroke::new(1.0),
            Affine::IDENTITY,
            border,
            Some(Affine::IDENTITY),
            &size.to_rect(),
        );

        // ── Filter box ──
        let filter_rect = Rect::new(0.0, 0.0, size.width, FILTER_BOX_HEIGHT);
        scene.fill(
            masonry::vello::peniko::Fill::NonZero,
            Affine::IDENTITY,
            white,
            Some(Affine::IDENTITY),
            &filter_rect,
        );
        // Divider between filter box and list.
        scene.stroke(
            &masonry::kurbo::Stroke::new(1.0),
            Affine::IDENTITY,
            border,
            Some(Affine::IDENTITY),
            &Rect::new(0.0, FILTER_BOX_HEIGHT - 0.5, size.width, FILTER_BOX_HEIGHT),
        );

        let (fcx, lcx) = ctx.text_contexts();
        let placeholder = "Type to filter...";
        let showing_placeholder = self.filter.is_empty();
        let display_text = if showing_placeholder {
            placeholder
        } else {
            self.filter.as_str()
        };

        let mut builder = lcx.ranged_builder(fcx, display_text, 1.0, true);
        builder.push_default(StyleProperty::FontSize(FONT_SIZE));
        let mut text_layout = builder.build(display_text);
        text_layout.break_all_lines(None);
        text_layout.align(None, masonry::TextAlign::Start, masonry::TextAlignOptions::default());

        let text_color = if showing_placeholder {
            masonry::vello::peniko::Color::from_rgb8(160, 160, 160)
        } else {
            masonry::vello::peniko::Color::BLACK
        };
        let brush = masonry::vello::peniko::Brush::Solid(text_color);
        let text_y = (FILTER_BOX_HEIGHT - FONT_SIZE as f64) / 2.0;
        masonry::core::render_text(
            scene,
            Affine::translate((10.0, text_y)),
            &text_layout,
            &[brush],
            true,
        );

        // Caret (always drawn at the end of the text).
        let caret_x = 10.0 + f64::from(text_layout.width()) + 1.0;
        scene.fill(
            masonry::vello::peniko::Fill::NonZero,
            Affine::IDENTITY,
            masonry::vello::peniko::Color::BLACK,
            Some(Affine::IDENTITY),
            &Rect::new(caret_x, 8.0, caret_x + 1.0, FILTER_BOX_HEIGHT - 8.0),
        );

        // ── List area ──
        let viewport_rect = Rect::new(
            0.0,
            FILTER_BOX_HEIGHT,
            size.width,
            FILTER_BOX_HEIGHT + self.viewport_height,
        );
        scene.fill(
            masonry::vello::peniko::Fill::NonZero,
            Affine::IDENTITY,
            white,
            Some(Affine::IDENTITY),
            &viewport_rect,
        );

        if self.filtered.is_empty() {
            let message = "No matches";
            let mut builder = lcx.ranged_builder(fcx, message, 1.0, true);
            builder.push_default(StyleProperty::FontSize(FONT_SIZE));
            let mut no_matches = builder.build(message);
            no_matches.break_all_lines(None);
            no_matches.align(
                None,
                masonry::TextAlign::Start,
                masonry::TextAlignOptions::default(),
            );
            let brush = masonry::vello::peniko::Brush::Solid(masonry::vello::peniko::Color::from_rgb8(
                160, 160, 160,
            ));
            let msg_x = (size.width - f64::from(no_matches.width())) / 2.0;
            let msg_y = FILTER_BOX_HEIGHT + (self.viewport_height - FONT_SIZE as f64) / 2.0;
            masonry::core::render_text(
                scene,
                Affine::translate((msg_x, msg_y)),
                &no_matches,
                &[brush],
                true,
            );
        }

        // Scrollbar, shown only when the content overflows the viewport.
        let max_offset = self.max_scroll_offset();
        if max_offset > 0.0 {
            let track_rect = Rect::new(
                size.width - SCROLLBAR_WIDTH - 1.0,
                FILTER_BOX_HEIGHT + 1.0,
                size.width - 1.0,
                FILTER_BOX_HEIGHT + self.viewport_height - 1.0,
            );
            scene.fill(
                masonry::vello::peniko::Fill::NonZero,
                Affine::IDENTITY,
                masonry::vello::peniko::Color::from_rgb8(230, 230, 230),
                Some(Affine::IDENTITY),
                &track_rect,
            );

            let track_height = track_rect.height();
            let thumb_height = (track_height * self.viewport_height
                / (self.filtered.len() as f64 * ROW_HEIGHT))
                .max(20.0);
            let thumb_y = track_rect.y0
                + (track_height - thumb_height) * (self.scroll_offset / max_offset);
            scene.fill(
                masonry::vello::peniko::Fill::NonZero,
                Affine::IDENTITY,
                masonry::vello::peniko::Color::from_rgb8(150, 150, 150),
                Some(Affine::IDENTITY),
                &Rect::new(track_rect.x0 + 1.0, thumb_y, track_rect.x1 - 1.0, thumb_y + thumb_height),
            );
        }
    }

    fn accessibility_role(&self) -> Role {
        Role::ListBox
    }

    fn accessibility(
        &mut self,
        _ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        node: &mut Node,
    ) {
        node.set_label(format!(
            "Dropdown options, {} shown, filter: {}",
            self.filtered.len(),
            self.filter
        ));
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::from_slice(
            &self.pods.iter().map(|pod| pod.id()).collect::<Vec<_>>(),
        )
    }

    fn make_trace_span(&self, id: WidgetId) -> Span {
        trace_span!("DropdownPanel", id = id.trace())
    }
}

// ── DropdownList: button that manages overlay layer ───────────────

pub struct DropdownList<T> {
    child: WidgetPod<dyn Widget>,
    /// Items in order of appearance; each tuple holds (value, display text).
    items: Vec<(T, String)>,
    selected: Option<T>,
    open: bool,
    layer_root_id: Option<WidgetId>,
    shared: Rc<SharedState>,
    button_height: f64,
    button_width: f64,
}

impl<T: Clone + 'static> DropdownList<T> {
    pub fn new(child: NewWidget<impl Widget + ?Sized>, items: Vec<(T, String)>) -> Self {
        Self {
            child: child.erased().to_pod(),
            items,
            selected: None,
            open: false,
            layer_root_id: None,
            shared: Rc::new(SharedState {
                selection: Cell::new(None),
                overlay_id: Cell::new(None),
                highlight: Cell::new(0),
                key_queue: RefCell::new(Vec::new()),
            }),
            button_height: 30.0,
            button_width: 200.0,
        }
    }

    /// Set the width of the button (and its label), in pixels.
    pub fn with_button_width(mut self, width: f64) -> Self {
        self.button_width = width;
        self
    }

    /// Open the overlay panel just below the button.
    fn open_overlay(&mut self, ctx: &mut EventCtx<'_>, position: Point) {
        let panel = DropdownPanel::new(Rc::clone(&self.shared), self.items.clone());
        let overlay: NewWidget<dyn Widget> = panel.with_auto_id().erased();
        let overlay_id = overlay.id();

        self.shared.overlay_id.set(Some(overlay_id));
        self.layer_root_id = Some(overlay_id);
        ctx.create_layer(overlay, position);
        self.open = true;
        // Take text focus so keyboard input can be forwarded to the panel's
        // filter box, and poll shared state every frame while open.
        ctx.request_focus();
        ctx.request_anim_frame();
    }

    /// Close the overlay panel (if open).
    fn close_overlay(&mut self, ctx: &mut EventCtx<'_>) {
        if let Some(id) = self.layer_root_id.take() {
            ctx.remove_layer(id);
        }
        self.shared.overlay_id.set(None);
        self.open = false;
    }
}

impl<T: Clone + Debug + Send + Sync + 'static> Widget for DropdownList<T> {
    type Action = T;

    fn on_pointer_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &PointerEvent,
    ) {
        // Fast path: an item was selected in the overlay.
        if let Some(index) = self.shared.selection.take() {
            let value = self.items[index].0.clone();
            self.selected = Some(value.clone());
            self.open = false;
            ctx.submit_action::<T>(value);
        }

        if let PointerEvent::Down(event) = event
            && event.button == Some(masonry::core::PointerButton::Primary)
        {
            if self.open {
                self.close_overlay(ctx);
            } else {
                let origin = ctx.window_origin();
                let position =
                    origin + masonry::kurbo::Vec2::new(0.0, -15.0);
                self.open_overlay(ctx, position);
            }
        }
    }

    fn on_text_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &TextEvent,
    ) {
        if !self.open {
            return;
        }
        let TextEvent::Keyboard(key_event) = event else {
            return;
        };
        if key_event.state != KeyState::Down {
            return;
        }
        // The button holds text focus while the dropdown is open; forward key
        // presses to the overlay panel, which consumes them on its next
        // animation frame.
        self.shared.key_queue.borrow_mut().push(key_event.clone());
        ctx.request_anim_frame();
        ctx.set_handled();
    }

    fn on_anim_frame(
        &mut self,
        ctx: &mut UpdateCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        _interval: u64,
    ) {
        if let Some(index) = self.shared.selection.take() {
            let value = self.items[index].0.clone();
            self.selected = Some(value.clone());
            self.open = false;
            ctx.submit_action::<T>(value);
        } else if self.open && self.shared.overlay_id.get().is_some() {
            ctx.request_anim_frame();
        } else {
            // The panel closed itself (item click or Escape).
            self.open = false;
        }
    }

    fn update(&mut self, _ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, _event: &Update) {
    }

    fn accepts_focus(&self) -> bool {
        true
    }

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        ctx.register_child(&mut self.child);
    }

    fn layout(
        &mut self,
        ctx: &mut LayoutCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        bc: &BoxConstraints,
    ) -> Size {
        let child_bc = BoxConstraints::new(
            Size::new(self.button_width, bc.min().height),
            Size::new(self.button_width, bc.max().height),
        );
        let child_size = ctx.run_layout(&mut self.child, &child_bc);
        ctx.place_child(&mut self.child, Point::ORIGIN);
        self.button_height = child_size.height;
        Size::new(self.button_width, child_size.height)
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, scene: &mut Scene) {
        let size = ctx.size();
        let rect = size.to_rect();

        scene.fill(
            masonry::vello::peniko::Fill::NonZero,
            Affine::IDENTITY,
            masonry::vello::peniko::Color::WHITE,
            Some(Affine::IDENTITY),
            &rect,
        );
        scene.stroke(
            &masonry::kurbo::Stroke::new(1.0),
            Affine::IDENTITY,
            masonry::vello::peniko::Color::from_rgb8(128, 128, 128),
            Some(Affine::IDENTITY),
            &rect,
        );

        let arrow_x = size.width - 20.0;
        let arrow_y = (size.height - 6.0) / 2.0;
        scene.fill(
            masonry::vello::peniko::Fill::NonZero,
            Affine::IDENTITY,
            masonry::vello::peniko::Color::BLACK,
            Some(Affine::IDENTITY),
            &Rect::new(arrow_x, arrow_y, arrow_x + 10.0, arrow_y + 6.0),
        );
    }

    fn accessibility_role(&self) -> Role {
        Role::ComboBox
    }

    fn accessibility(
        &mut self,
        _ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        node: &mut Node,
    ) {
        node.add_action(masonry::accesskit::Action::Click);
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::from_slice(&[self.child.id()])
    }

    fn make_trace_span(&self, id: WidgetId) -> Span {
        trace_span!("DropdownList", id = id.trace())
    }
}

impl<T: Clone + Debug + Send + Sync + 'static> DropdownList<T> {
    pub fn child_mut<'t>(this: &'t mut WidgetMut<'_, Self>) -> WidgetMut<'t, dyn Widget> {
        this.ctx.get_mut(&mut this.widget.child)
    }
}

// ── Xilem View wrapper ────────────────────────────────────────────

/// A reusable dropdown view.
///
/// `T` is the type of the value reported when the user makes a selection.
/// `items` is a list of `(value, display_text)` tuples; the display text is
/// shown in the list and filter, while `value` is returned to the callback.
pub struct DropdownView<State: 'static, T> {
    items: Vec<(T, String)>,
    selected: Option<T>,
    callback: Box<dyn Fn(&mut State, T) + Send + Sync>,
    button_width: f64,
}

/// Create a dropdown that calls `callback(app_state, value)` whenever the user
/// selects an item. `selected` is the value currently selected (used to label
/// the button); pass `None` to show the placeholder.
pub fn dropdown_view<State: 'static, T>(
    items: Vec<(T, String)>,
    selected: Option<T>,
    callback: impl Fn(&mut State, T) + Send + Sync + 'static,
) -> DropdownView<State, T>
where
    T: PartialEq + Clone + Send + Sync + Debug + 'static,
{
    DropdownView {
        items,
        selected,
        callback: Box::new(callback),
        button_width: 200.0,
    }
}

/// Resolve the display text to show on the button for the given selection.
fn selected_display<T: PartialEq>(items: &[(T, String)], selected: &Option<T>) -> String {
    match selected {
        Some(value) => items
            .iter()
            .find(|(v, _)| v == value)
            .map(|(_, display)| display.clone())
            .unwrap_or_else(|| "Select...".to_string()),
        None => "Select...".to_string(),
    }
}

impl<State: 'static, T> DropdownView<State, T> {
    /// Set the width of the button (its label), in pixels.
    pub fn with_button_width(mut self, width: f64) -> Self {
        self.button_width = width;
        self
    }
}

impl<State: 'static, T> ViewMarker for DropdownView<State, T> {}

impl<State: 'static, T> View<State, (), ViewCtx> for DropdownView<State, T>
where
    T: PartialEq + Clone + Send + Sync + Debug + 'static,
{
    type Element = Pod<DropdownList<T>>;
    type ViewState = ();

    fn build(
        &self,
        ctx: &mut ViewCtx,
        _app_state: &mut State,
    ) -> (Self::Element, Self::ViewState) {
        let display = selected_display(&self.items, &self.selected);
        let child_label =
            NewWidget::new_with_props(Label::new(display), Properties::new().with(
                ContentColor::new(Color::BLACK),
            ));

        let widget = DropdownList::new(child_label, self.items.clone())
            .with_button_width(self.button_width);
        (
            ctx.with_action_widget(|ctx| ctx.create_pod(widget)),
            (),
        )
    }

    fn rebuild(
        &self,
        _prev: &Self,
        _view_state: &mut Self::ViewState,
        _ctx: &mut ViewCtx,
        mut element: Mut<'_, Self::Element>,
        _app_state: &mut State,
    ) {
        let new_text = selected_display(&self.items, &self.selected);
        let mut child = DropdownList::child_mut(&mut element);
        let mut label = child.downcast::<Label>();
        Label::set_text(&mut label, new_text);
    }

    fn teardown(
        &self,
        _view_state: &mut Self::ViewState,
        ctx: &mut ViewCtx,
        element: Mut<'_, Self::Element>,
    ) {
        ctx.teardown_leaf(element);
    }

    fn message(
        &self,
        _view_state: &mut Self::ViewState,
        message: &mut MessageContext,
        _element: Mut<'_, Self::Element>,
        app_state: &mut State,
    ) -> MessageResult<()> {
        match message.take_message::<T>() {
            Some(value) => {
                (self.callback)(app_state, *value);
                MessageResult::Action(())
            }
            None => MessageResult::Stale,
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use masonry::core::{KeyboardEvent, WidgetRef};
    use masonry::core::keyboard::Code;
    use masonry::kurbo::Vec2;
    use masonry::testing::TestHarness;
    use masonry::theme::default_property_set;

    const TEST_ITEMS: &[&str] = &[
        "United States", "Canada", "United Kingdom", "Germany", "France",
        "Japan", "Australia", "Brazil", "India", "South Korea",
        "Italy", "Spain", "Mexico", "Netherlands", "Sweden",
        "Norway", "Denmark", "Finland", "Poland", "Portugal",
        "Greece", "Ireland", "Switzerland", "Austria", "Belgium",
        "Argentina", "New Zealand",
    ];

    fn make_harness() -> TestHarness<DropdownList<usize>> {
        // Value type is the item's index, so a selected value can be compared
        // against the original list position.
        let items: Vec<(usize, String)> = TEST_ITEMS
            .iter()
            .enumerate()
            .map(|(i, s)| (i, s.to_string()))
            .collect();
        let root =
            DropdownList::new(Label::new("Select...").with_auto_id(), items).with_auto_id();
        TestHarness::create(default_property_set(), root)
    }

    /// Opens the dropdown by clicking the button; returns the overlay panel's
    /// id. The button keeps text focus and forwards keys to the panel.
    fn open_dropdown(harness: &mut TestHarness<DropdownList<usize>>) -> WidgetId {
        harness.mouse_click_on(harness.root_id());
        let panel_id = root_of(harness)
            .shared
            .overlay_id
            .get()
            .expect("dropdown panel should be created when opened");
        // Let the panel run one animation frame (it registers + starts pumping).
        harness.animate_ms(16);
        panel_id
    }

    fn root_of(harness: &TestHarness<DropdownList<usize>>) -> WidgetRef<'_, DropdownList<usize>> {
        harness.root_widget().downcast::<DropdownList<usize>>().unwrap()
    }

    fn panel_of<'h>(
        harness: &'h TestHarness<DropdownList<usize>>,
        id: WidgetId,
    ) -> WidgetRef<'h, DropdownPanel<usize>> {
        harness
            .get_widget_with_id(id)
            .downcast::<DropdownPanel<usize>>()
            .expect("overlay root should be a DropdownPanel")
    }

    fn press_key(harness: &mut TestHarness<DropdownList<usize>>, key: Key) {
        let event = TextEvent::Keyboard(KeyboardEvent::key_down(key, Code::Unidentified));
        harness.process_text_event(event);
        // Give the panel an animation frame to drain the forwarded key.
        harness.animate_ms(16);
    }

    fn type_text(harness: &mut TestHarness<DropdownList<usize>>, text: &str) {
        for c in text.chars() {
            press_key(harness, Key::Character(c.to_string().into()));
        }
    }

    #[test]
    fn opening_focuses_button_and_creates_panel() {
        let mut harness = make_harness();
        let panel_id = open_dropdown(&mut harness);
        assert_eq!(harness.focused_widget_id(), Some(harness.root_id()));
        assert!(root_of(&harness).open);
        assert!(panel_of(&harness, panel_id).filtered.len() == TEST_ITEMS.len());
    }

    #[test]
    fn filter_narrows_and_restores_items() {
        let mut harness = make_harness();
        let panel_id = open_dropdown(&mut harness);

        type_text(&mut harness, "un");
        assert_eq!(panel_of(&harness, panel_id).filtered.len(), 2);

        // No matches at all.
        type_text(&mut harness, "zzz");
        assert!(panel_of(&harness, panel_id).filtered.is_empty());

        // Three backspaces restore the "un" prefix and its matches.
        for _ in 0..3 {
            press_key(&mut harness, Key::Named(NamedKey::Backspace));
        }
        assert_eq!(panel_of(&harness, panel_id).filtered.len(), 2);
    }

    #[test]
    fn filter_is_case_insensitive_substring() {
        let mut harness = make_harness();
        let panel_id = open_dropdown(&mut harness);

        type_text(&mut harness, "LAND");
        let matches = panel_of(&harness, panel_id).filtered.clone();
        assert_eq!(matches.len(), 6); // Netherlands, Finland, Switzerland, Ireland, Poland, New Zealand
        for &index in &matches {
            assert!(TEST_ITEMS[index].to_lowercase().contains("land"));
        }
    }

    #[test]
    fn enter_selects_highlighted_item_and_emits_action() {
        let mut harness = make_harness();
        open_dropdown(&mut harness);

        type_text(&mut harness, "germany");
        press_key(&mut harness, Key::Named(NamedKey::Enter));

        // Selection propagates from the overlay to the button on the next frame.
        harness.animate_ms(16);
        let (index, widget_id) = harness
            .pop_action::<usize>()
            .expect("selection should emit an action");
        assert_eq!(index, 3); // "Germany"
        assert_eq!(widget_id, harness.root_id());
    }

    #[test]
    fn arrow_keys_navigate_then_select() {
        let mut harness = make_harness();
        let panel_id = open_dropdown(&mut harness);

        press_key(&mut harness, Key::Named(NamedKey::ArrowDown));
        press_key(&mut harness, Key::Named(NamedKey::ArrowDown));
        assert_eq!(panel_of(&harness, panel_id).highlighted_row, 2);
        // Rows 0-2 fit in the 6-row viewport: no scrolling needed yet.
        assert_eq!(panel_of(&harness, panel_id).scroll_offset, 0.0);

        // Move the highlight to row 6 (index 6, "Australia"): past the
        // viewport end, so the list must scroll by one row.
        for _ in 2..6 {
            press_key(&mut harness, Key::Named(NamedKey::ArrowDown));
        }
        assert_eq!(panel_of(&harness, panel_id).highlighted_row, 6);
        assert!(panel_of(&harness, panel_id).scroll_offset >= ROW_HEIGHT);

        press_key(&mut harness, Key::Named(NamedKey::Enter));
        harness.animate_ms(16);
        let (index, _) = harness.pop_action::<usize>().expect("action after Enter");
        assert_eq!(index, 6); // "Australia"
    }

    #[test]
    fn mouse_wheel_scrolls_the_list() {
        let mut harness = make_harness();
        let panel_id = open_dropdown(&mut harness);

        assert_eq!(panel_of(&harness, panel_id).scroll_offset, 0.0);
        // Move pointer over the list area (the panel lives in an overlay
        // layer, so the checked helper would hit-test the wrong layer), then
        // scroll "down".
        harness.mouse_move_to_unchecked(panel_id);
        harness.mouse_wheel(Vec2::new(0.0, -120.0));

        let panel = panel_of(&harness, panel_id);
        assert!(panel.scroll_offset > 0.0);
        // Content is 27 rows tall, viewport shows 6: offset must be clamped.
        assert!(panel.scroll_offset <= ROW_HEIGHT * 21.0);
    }

    #[test]
    fn clicking_a_row_selects_it() {
        let mut harness = make_harness();
        let panel_id = open_dropdown(&mut harness);

        let first_row_id = panel_of(&harness, panel_id).children()[0].id();
        // The row lives in an overlay layer, so use the unchecked move.
        harness.mouse_move_to_unchecked(first_row_id);
        harness.mouse_button_press(masonry::core::PointerButton::Primary);
        harness.mouse_button_release(masonry::core::PointerButton::Primary);
        harness.animate_ms(16);

        let (index, _) = harness.pop_action::<usize>().expect("action after click");
        assert_eq!(index, 0); // "United States"
    }

    #[test]
    fn escape_clears_filter_then_closes() {
        let mut harness = make_harness();
        let panel_id = open_dropdown(&mut harness);

        type_text(&mut harness, "un");
        assert_eq!(panel_of(&harness, panel_id).filtered.len(), 2);

        // First Escape clears the filter.
        press_key(&mut harness, Key::Named(NamedKey::Escape));
        assert_eq!(
            panel_of(&harness, panel_id).filtered.len(),
            TEST_ITEMS.len()
        );

        // Second Escape closes the dropdown.
        press_key(&mut harness, Key::Named(NamedKey::Escape));
        harness.animate_ms(16);
        let root = harness
            .get_widget_with_id(harness.root_id())
            .downcast::<DropdownList<usize>>()
            .unwrap();
        assert!(root.shared.overlay_id.get().is_none());
        assert!(!root.open);
    }
}
