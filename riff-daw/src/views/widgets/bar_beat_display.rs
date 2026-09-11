//! A bar/beat position display widget.
//!
//! Displays the current transport position as `BBB:BB` (bar : beat) and keeps
//! itself repainting while the transport is playing. Position updates arrive
//! asynchronously from the audio thread; Xilem rebuilds the view on each
//! message but does not schedule a repaint, so this widget requests animation
//! frames to force masonry to re-run its layout/paint passes.

use masonry::accesskit::{Node, Role};
use masonry::core::{
    BoxConstraints, BrushIndex, ChildrenIds, LayoutCtx, NoAction, PaintCtx, PropertiesMut,
    PropertiesRef, RegisterCtx, Update, UpdateCtx, Widget, WidgetId, WidgetMut,
};
use masonry::kurbo::{Affine, Size, Vec2};
use masonry::palette;
use masonry::parley::style::{FontFamily, FontStack, GenericFamily, StyleProperty};
use masonry::parley::{FontContext, LayoutContext};
use masonry::peniko::Color;
use masonry::theme;
use masonry::vello::Scene;
use masonry::{TextAlign, TextAlignOptions};
use tracing::{Span, trace_span};

const BAR_BEAT_FONT_SIZE: f32 = theme::TEXT_SIZE_NORMAL;

/// A masonry widget that renders the transport bar/beat position and requests
/// animation frames while playing so the display updates live.
pub struct BarBeatDisplayWidget {
    text: String,
    playing: bool,
}

impl BarBeatDisplayWidget {
    /// Create a new bar/beat display widget.
    pub fn new(text: String, playing: bool) -> Self {
        Self { text, playing }
    }

    /// Update the displayed text and mark the widget for repaint.
    pub fn set_text(this: &mut WidgetMut<'_, Self>, text: String) {
        if this.widget.text != text {
            this.widget.text = text;
            this.ctx.request_render();
        }
        if this.widget.playing {
            this.ctx.request_anim_frame();
        }
    }

    /// Update the playing state, starting or stopping the animation loop.
    pub fn set_playing(this: &mut WidgetMut<'_, Self>, playing: bool) {
        if this.widget.playing != playing {
            this.widget.playing = playing;
            this.ctx.request_render();
        }
        if playing {
            this.ctx.request_anim_frame();
        }
    }

    fn measure_text(
        font_context: &mut FontContext,
        layout_context: &mut LayoutContext<BrushIndex>,
        text: &str,
    ) -> Size {
        let mut text_layout_builder = layout_context.ranged_builder(font_context, text, 1.0, true);
        text_layout_builder.push_default(StyleProperty::FontStack(FontStack::Single(
            FontFamily::Generic(GenericFamily::SansSerif),
        )));
        text_layout_builder.push_default(StyleProperty::FontSize(BAR_BEAT_FONT_SIZE));
        let text_layout = text_layout_builder.build(text.to_owned());
        Size::new(text_layout.width() as f64, text_layout.height() as f64)
    }

    fn draw_text(
        &self,
        ctx: &mut PaintCtx<'_>,
        scene: &mut Scene,
        x: f64,
        y: f64,
        text: &str,
        color: Color,
    ) {
        let (font_context, layout_context) = ctx.text_contexts();
        let mut text_layout_builder =
            layout_context.ranged_builder(font_context, text, 1.0, true);
        text_layout_builder.push_default(StyleProperty::FontStack(FontStack::Single(
            FontFamily::Generic(GenericFamily::SansSerif),
        )));
        text_layout_builder.push_default(StyleProperty::FontSize(BAR_BEAT_FONT_SIZE));
        let mut text_layout = text_layout_builder.build(text.to_owned());
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

impl Widget for BarBeatDisplayWidget {
    type Action = NoAction;

    fn on_anim_frame(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, _interval: u64) {
        if self.playing {
            ctx.request_anim_frame();
        }
        ctx.request_paint_only();
    }

    fn update(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, event: &Update) {
        match event {
            Update::WidgetAdded => {
                if self.playing {
                    ctx.request_anim_frame();
                }
            }
            Update::HoveredChanged(_)
            | Update::ActiveChanged(_)
            | Update::FocusChanged(_)
            | Update::DisabledChanged(_) => {
                ctx.request_paint_only();
            }
            _ => {}
        }
    }

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}

    fn layout(
        &mut self,
        ctx: &mut LayoutCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        bc: &BoxConstraints,
    ) -> Size {
        let (font_context, layout_context) = ctx.text_contexts();
        bc.constrain(Self::measure_text(font_context, layout_context, &self.text))
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, scene: &mut Scene) {
        let size = ctx.size();
        let (font_context, layout_context) = ctx.text_contexts();
        let text_size = Self::measure_text(font_context, layout_context, &self.text);
        let x = 0.0;
        let y = ((size.height - text_size.height) / 2.0).max(0.0);
        self.draw_text(ctx, scene, x, y, self.text.as_str(), palette::css::WHITE);
    }

    fn accessibility_role(&self) -> Role {
        Role::Label
    }

    fn accessibility(
        &mut self,
        _ctx: &mut masonry::core::AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        node: &mut Node,
    ) {
        node.set_label(self.text.clone());
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }

    fn make_trace_span(&self, id: WidgetId) -> Span {
        trace_span!("BarBeatDisplay", id = id.trace())
    }
}