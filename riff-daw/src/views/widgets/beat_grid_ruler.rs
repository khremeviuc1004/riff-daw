//! A horizontal time ruler widget (bar / beat numbers and grid lines).
//!
//! Conversion of the gtk3 `BeatGridRuler` from `grid.rs` into a native xilem
//! / masonry `Widget`. The original painted with cairo; this version draws the
//! ruler directly into a vello [`Scene`], mirroring the rendering approach used
//! by the sibling `BeatGridWidget` in this project.

use masonry::accesskit::{Node, Role};
use masonry::core::{
    BoxConstraints, ChildrenIds, EventCtx, LayoutCtx, PaintCtx, PointerEvent,
    PropertiesMut, PropertiesRef, RegisterCtx, TextEvent, Update, UpdateCtx, Widget, WidgetId,
};
use masonry::kurbo::{Affine, BezPath, Point, Rect, Size, Stroke, Vec2};
use masonry::palette;
use masonry::parley::style::{FontFamily, FontStack, GenericFamily, StyleProperty};
use masonry::parley::FontWeight;
use masonry::peniko::{Color, Fill};
use masonry::vello::Scene;
use masonry::{TextAlign, TextAlignOptions};
use tracing::{Span, trace_span};

/// The height of the horizontal ruler widget.
const RULER_HEIGHT: f64 = 30.0;
/// A horizontal beat / bar ruler, drawn in vello, that synchronises
/// horizontally with the associated `BeatGridWidget`.
pub struct BeatGridRulerWidget {
    height: f64,
    width: f64,

    beat_width_in_pixels: f64,
    zoom_horizontal: f64,
    zoom_vertical: f64,
    zoom_factor: f64,
    beats_per_bar: i32,
}

impl BeatGridRulerWidget {
    /// Create a new ruler.
    ///
    /// * `zoom` - the (uniform) zoom level.
    /// * `beat_width_in_pixels` - the width, in pixels, of a single beat at
    ///   zoom level 1.0.
    /// * `beats_per_bar` - the number of beats in each bar (e.g. 4 for 4/4).
    /// * `width` - the content width the ruler spans (so it can scroll in
    ///   lock-step with the associated grid).
    pub fn new(
        zoom: f64,
        beat_width_in_pixels: f64,
        beats_per_bar: i32,
        width: f64,
    ) -> Self {
        Self {
            height: RULER_HEIGHT,
            width,
            beat_width_in_pixels,
            zoom_horizontal: zoom,
            zoom_vertical: zoom,
            zoom_factor: 0.01,
            beats_per_bar,
        }
    }

    /// Create a new ruler with independent horizontal and vertical zoom levels.
    pub fn new_with_individual_zoom_level(
        zoom_horizontal: f64,
        zoom_vertical: f64,
        beat_width_in_pixels: f64,
        beats_per_bar: i32,
        width: f64,
    ) -> Self {
        Self {
            height: RULER_HEIGHT,
            width,
            beat_width_in_pixels,
            zoom_horizontal,
            zoom_vertical,
            zoom_factor: 0.01,
            beats_per_bar,
        }
    }

    /// Set the number of beats per bar.
    pub fn set_beats_per_bar(&mut self, beats_per_bar: i32) {
        self.beats_per_bar = beats_per_bar;
    }

    /// Set the ruler height.
    pub fn set_height(&mut self, height: f64) {
        self.height = height;
    }

    /// Set the ruler content width.
    pub fn set_width(&mut self, width: f64) {
        self.width = width;
    }

    /// Set the horizontal zoom level.
    pub fn set_horizontal_zoom(&mut self, zoom: f64) {
        self.zoom_horizontal = zoom;
    }

    /// Set the vertical zoom level.
    pub fn set_vertical_zoom(&mut self, zoom: f64) {
        self.zoom_vertical = zoom;
    }

    /// The current horizontal zoom level.
    pub fn zoom_horizontal(&self) -> f64 {
        self.zoom_horizontal
    }

    /// The current vertical zoom level.
    pub fn zoom_vertical(&self) -> f64 {
        self.zoom_vertical
    }

    /// The beat width in pixels at zoom level 1.0.
    pub fn beat_width_in_pixels(&self) -> f64 {
        self.beat_width_in_pixels
    }

    /// Zoom in horizontally.
    pub fn zoom_horizontal_in(&mut self) {
        if self.zoom_horizontal < 7.0 {
            self.zoom_horizontal += self.zoom_factor;
        }
    }

    /// Zoom out horizontally.
    pub fn zoom_horizontal_out(&mut self) {
        if self.zoom_horizontal > (self.zoom_factor * 2.0) {
            self.zoom_horizontal -= self.zoom_factor;
        }
    }

    /// Zoom in vertically.
    pub fn zoom_vertical_in(&mut self) {
        if self.zoom_vertical < 7.0 {
            self.zoom_vertical += self.zoom_factor;
        }
    }

    /// Zoom out vertically.
    pub fn zoom_vertical_out(&mut self) {
        if self.zoom_vertical > (self.zoom_factor * 2.0) {
            self.zoom_vertical -= self.zoom_factor;
        }
    }

    /// Paint the horizontal scale (bar / beat numbers and grid lines).
    fn paint_horizontal_scale(&mut self, context: &mut PaintCtx<'_>, scene: &mut Scene) {
        let adjusted_beat_width_in_pixels = self.beat_width_in_pixels * self.zoom_horizontal;
        let bounds: Rect = context.size().to_rect();
        let clip_x1 = bounds.x0;
        let clip_x2 = bounds.x1;
        let clip_y2 = bounds.y1;
        let clip_x1_in_beats = clip_x1 / adjusted_beat_width_in_pixels;
        // go to the first beat to the left of the view port e.g. bar 2 beat 3 = beat 2 * 4 + 3 = beat 11
        let mut current_x = clip_x1_in_beats.floor() * adjusted_beat_width_in_pixels;
        let mut bar_index = (clip_x1_in_beats / (self.beats_per_bar as f64)) as i32 + 1; // get the bar
        let mut beat_in_bar_index = (clip_x1_in_beats as i32 % self.beats_per_bar) + 1;

        while current_x < clip_x2 {
            if beat_in_bar_index == 1 {
                // bar number, drawn towards the bottom of the ruler
                let font_size = if self.zoom_horizontal < 0.08 { 7.0 } else { 10.0 };
                self.draw_text(
                    context,
                    scene,
                    current_x,
                    clip_y2 - 20.0,
                    &format!("{}", bar_index),
                    Color::from_rgba8(127, 127, 127, 255),
                    font_size as f32,
                    FontWeight::NORMAL,
                );
            }

            if self.zoom_horizontal > 0.11 {
                // beat-within-bar number, drawn just above the bottom edge
                self.draw_text(
                    context,
                    scene,
                    current_x,
                    clip_y2 - 5.0,
                    &format!("{}", beat_in_bar_index),
                    Color::from_rgba8(127, 127, 127, 127),
                    8.0,
                    FontWeight::NORMAL,
                );
            }

            if self.zoom_horizontal > 0.11 || beat_in_bar_index == 1 {
                // vertical grid line for this beat
                let stroke_color = if beat_in_bar_index == 1 {
                    Color::from_rgba8(127, 127, 127, 255)
                } else {
                    Color::from_rgba8(127, 127, 127, 127)
                };

                let mut path = BezPath::new();
                path.move_to(Point { x: current_x, y: bounds.y0 });
                path.line_to(Point { x: current_x, y: bounds.y1 });
                scene.stroke(
                    &Stroke::new(0.3),
                    Affine::IDENTITY,
                    stroke_color,
                    None,
                    &path,
                );
            }

            current_x += adjusted_beat_width_in_pixels;

            if beat_in_bar_index == self.beats_per_bar {
                beat_in_bar_index = 1;
                bar_index += 1;
            } else {
                beat_in_bar_index += 1;
            }
        }
    }

    /// Render a single line of text into the scene.
    fn draw_text(
        &self,
        context: &mut PaintCtx<'_>,
        scene: &mut Scene,
        x: f64,
        y: f64,
        text: &str,
        color: Color,
        size: f32,
        font_weight: FontWeight,
    ) {
        let (font_context, layout_context) = context.text_contexts();
        let mut text_layout_builder = layout_context.ranged_builder(font_context, text.clone(), 1.0, true);
        text_layout_builder.push_default(
            StyleProperty::FontStack(
                FontStack::Single(
                    FontFamily::Generic(GenericFamily::SansSerif),
                )
            )
        );
        text_layout_builder.push_default(StyleProperty::FontSize(size));
        text_layout_builder.push_default(StyleProperty::FontWeight(font_weight));
        let mut text_layout = text_layout_builder.build(text.clone());
        text_layout.break_all_lines(None);
        text_layout.align(None, TextAlign::Start, TextAlignOptions::default());

        masonry::core::render_text(
            scene,
            Affine::translate(Vec2 { x, y }),
            &text_layout,
            &[color.into()],
            true,
        );
    }
}

impl Widget for BeatGridRulerWidget {
    type Action = ();

    fn on_pointer_event(
        &mut self,
        _ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        _event: &PointerEvent,
    ) {
    }

    fn on_text_event(
        &mut self,
        _ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        _event: &TextEvent,
    ) {
    }

    fn on_access_event(
        &mut self,
        _ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        _event: &masonry::core::AccessEvent,
    ) {
    }

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}

    fn layout(
        &mut self,
        _layout_ctx: &mut LayoutCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        _bc: &BoxConstraints,
    ) -> Size {
        Size::new(self.width, self.height)
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, scene: &mut Scene) {
        let size = ctx.size();
        let rect = size.to_rect();

        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            palette::css::WHITE,
            None,
            &rect,
        );

        self.paint_horizontal_scale(ctx, scene);
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
        node.set_label("Beat grid ruler.");
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }

    fn make_trace_span(&self, id: WidgetId) -> Span {
        trace_span!("BeatGridRuler", id = id.trace())
    }

    fn update(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, _event: &Update) {
        ctx.request_paint_only();
    }

    fn accepts_focus(&self) -> bool {
        false
    }
}
