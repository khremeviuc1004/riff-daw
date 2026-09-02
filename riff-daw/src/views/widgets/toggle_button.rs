// Copyright 2019 the Xilem Authors and the Druid Authors
// SPDX-License-Identifier: Apache-2.0

//! A toggle button widget.

use std::any::TypeId;
use std::sync::Arc;
use masonry::accesskit::{Node, Role, Toggled};
use masonry_core::core::HasProperty;
use tracing::{Span, trace, trace_span};
use masonry::ui_events::keyboard::Key;
use vello::Scene;
use vello::kurbo::Rect;
use vello::kurbo::{Affine, BezPath, Cap, Dashes, Join, Size, Stroke};

use masonry::core::{
    AccessCtx, AccessEvent, ArcStr, BoxConstraints, ChildrenIds, EventCtx, LayoutCtx, NewWidget,
    PaintCtx, PointerEvent, PropertiesMut, PropertiesRef, RegisterCtx, TextEvent, Update,
    UpdateCtx, Widget, WidgetId, WidgetMut, WidgetPod,
};
use masonry::kurbo::RoundedRect;
use masonry::peniko::color::AlphaColor;
use masonry::peniko::Fill;
use masonry::peniko::Mix::Color;
use masonry::properties::{
    ActiveBackground, Background, BorderColor, BorderWidth,
    CornerRadius, DisabledBackground, DisabledCheckmarkColor, HoveredBorderColor, Padding,
};
use masonry::{palette, theme};
use masonry::util::{fill, stroke};
use masonry::widgets::Label;
use vello_svg::append;
use crate::icons::ICON_ARROW_LEFT;

/// A togglebutton that can be toggled.
///
///
/// Emits [`ToggleButtonToggled`] when it should toggle.
/// Note that the checked state does not automatically toggle, and so one of
/// the responses to a `ToggleButtonToggled` is to call [`ToggleButtonWidget::set_toggled`]
/// on the originating widget.
///
/// This allows higher-level components to choose how the togglebutton responds,
/// and ensure that its value is based on their correct source of truth.
pub struct ToggleButtonWidget {
    toggled: bool,
    svg_icon: Arc<str>,
}

impl ToggleButtonWidget {
    /// Create a new `ToggleButton` with a text label.
    pub fn new(toggled: bool, svg_icon: Arc<str>) -> Self {
        Self {
            toggled,
            svg_icon,
        }
    }
}

// --- MARK: WIDGETMUT
impl ToggleButtonWidget {
    /// toggle or untoggle the button.
    pub fn set_toggled(this: &mut WidgetMut<'_, Self>, toggled: bool) {
        this.widget.toggled = toggled;
        // Checked state impacts appearance and accessibility node
        this.ctx.request_render();
    }
}

impl HasProperty<DisabledBackground> for ToggleButtonWidget {}
impl HasProperty<ActiveBackground> for ToggleButtonWidget {}
impl HasProperty<Background> for ToggleButtonWidget {}
impl HasProperty<HoveredBorderColor> for ToggleButtonWidget {}
impl HasProperty<BorderColor> for ToggleButtonWidget {}
impl HasProperty<BorderWidth> for ToggleButtonWidget {}
impl HasProperty<CornerRadius> for ToggleButtonWidget {}
impl HasProperty<Padding> for ToggleButtonWidget {}
impl HasProperty<DisabledCheckmarkColor> for ToggleButtonWidget {}

/// The action type emitted by [`ToggleButtonWidget`] when it is activated.
///
/// The field is the target toggle state (i.e. true is "this togglebutton would like to become checked").
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct ToggleButtonToggled(pub bool);

// --- MARK: IMPL WIDGET
impl Widget for ToggleButtonWidget {
    type Action = ToggleButtonToggled;

    fn on_pointer_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &PointerEvent,
    ) {
        match event {
            PointerEvent::Down { .. } => {
                ctx.capture_pointer();
                trace!("ToggleButton {:?} pressed", ctx.widget_id());
            }
            PointerEvent::Up { .. } => {
                if ctx.is_active() && ctx.is_hovered() {
                    ctx.submit_action::<Self::Action>(ToggleButtonToggled(!self.toggled));
                    trace!("ToggleButton {:?} released", ctx.widget_id());
                }
            }
            _ => (),
        }
    }

    fn on_text_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &TextEvent,
    ) {
        match event {
            TextEvent::Keyboard(event) if event.state.is_up() => {
                if matches!(&event.key, Key::Character(c) if c == " ") {
                    ctx.submit_action::<Self::Action>(ToggleButtonToggled(!self.toggled));
                }
            }
            _ => (),
        }
    }

    fn accepts_focus(&self) -> bool {
        // ToggleButton can be tab-focused...
        true
    }

    fn on_access_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &AccessEvent,
    ) {
        match event.action {
            masonry::accesskit::Action::Click => {
                ctx.submit_action::<Self::Action>(ToggleButtonToggled(!self.toggled));
            }
            _ => {}
        }
    }

    fn update(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, event: &Update) {
        match event {
            Update::HoveredChanged(_)
            | Update::ActiveChanged(_)
            | Update::FocusChanged(_)
            | Update::DisabledChanged(_) => {
                ctx.request_paint_only();
            }

            _ => {}
        }
    }

    fn property_changed(&mut self, ctx: &mut UpdateCtx<'_>, property_type: TypeId) {
        DisabledBackground::prop_changed(ctx, property_type);
        ActiveBackground::prop_changed(ctx, property_type);
        Background::prop_changed(ctx, property_type);
        HoveredBorderColor::prop_changed(ctx, property_type);
        BorderColor::prop_changed(ctx, property_type);
        BorderWidth::prop_changed(ctx, property_type);
        CornerRadius::prop_changed(ctx, property_type);
        Padding::prop_changed(ctx, property_type);
        DisabledCheckmarkColor::prop_changed(ctx, property_type);
    }

    fn layout(
        &mut self,
        ctx: &mut LayoutCtx<'_>,
        props: &mut PropertiesMut<'_>,
        bc: &BoxConstraints,
    ) -> Size {
        let border = props.get::<BorderWidth>();
        let padding = props.get::<Padding>();

        let x_padding = theme::WIDGET_CONTROL_COMPONENT_PADDING;
        let check_side = theme::BASIC_WIDGET_HEIGHT;

        let check_size = Size::new(check_side, check_side);
        let (check_size, _) = padding.layout_up(check_size, 0.);
        let (check_size, _) = border.layout_up(check_size, 0.);

        let our_size = bc.constrain(check_size);
        our_size
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, props: &PropertiesRef<'_>, scene: &mut Scene) {
        let is_pressed = ctx.is_active();
        let is_hovered = ctx.is_hovered();

        let toggle_size = theme::BASIC_WIDGET_HEIGHT;
        let size = Size::new(toggle_size, toggle_size);

        let border_width = props.get::<BorderWidth>();
        let border_radius = props.get::<CornerRadius>();

        let bg = if ctx.is_disabled() {
            &props.get::<DisabledBackground>().0
        } else if is_pressed {
            &props.get::<ActiveBackground>().0
        } else {
            props.get::<Background>()
        };

        let bg_rect = border_width.bg_rect(size, border_radius);
        let border_rect = border_width.border_rect(size, border_radius);

        let border_color = if is_hovered {
            &props.get::<HoveredBorderColor>().0
        } else {
            props.get::<BorderColor>()
        };

        // Paint the togglebutton background and border
        let brush = bg.get_peniko_brush_for_rect(bg_rect.rect());
        fill(scene, &bg_rect, &brush);
        stroke(scene, &border_rect, border_color.color, border_width.width);

        // Paint the toggled rectangle
        if self.toggled {
            let _ = append(scene, self.svg_icon.as_ref());
        }
        // Paint focus indicator around the entire widget (box + label)
        if ctx.is_focus_target() || is_hovered {
            let widget_size = ctx.size();

            let focus_rect = Rect::new(0.0, 0.0, widget_size.width, widget_size.height);

            let focus_rect = focus_rect.inflate(2.0, 2.0);

            let focus_color = theme::FOCUS_COLOR;
            let focus_width = 2.0;
            let focus_radius = 4.0;

            let focus_stroke = Stroke {
                width: focus_width,
                join: Join::Round,
                miter_limit: 10.0,
                start_cap: Cap::Round,
                end_cap: Cap::Round,
                dash_pattern: Dashes::default(),
                dash_offset: 0.0,
            };
            let focus_path = focus_rect.to_rounded_rect(focus_radius);
            scene.stroke(
                &focus_stroke,
                Affine::IDENTITY,
                focus_color,
                None,
                &focus_path,
            );
        }
    }

    fn accessibility_role(&self) -> Role {
        Role::CheckBox
    }

    fn accessibility(
        &mut self,
        _ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        node: &mut Node,
    ) {
        node.add_action(masonry::accesskit::Action::Click);
        if self.toggled {
            node.set_toggled(Toggled::True);
        } else {
            node.set_toggled(Toggled::False);
        }
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::from_slice(&[])
    }

    fn make_trace_span(&self, id: WidgetId) -> Span {
        trace_span!("ToggleButton", id = id.trace())
    }

    fn get_debug_text(&self) -> Option<String> {
        if self.toggled {
            Some("[X]".to_string())
        } else {
            Some("[ ]".to_string())
        }
    }

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
    }
}
