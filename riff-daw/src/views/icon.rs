use masonry::accesskit::{Node, Role};
use masonry::core::{
    AccessCtx, AccessEvent, BoxConstraints, ChildrenIds, ErasedAction, EventCtx, LayoutCtx,
    NewWidget, NoAction, PaintCtx, PointerEvent, PropertiesMut, PropertiesRef, RegisterCtx,
    TextEvent, Widget, WidgetId,
};
use masonry::kurbo::{Affine, BezPath, Line, Point, Rect, Size, Stroke};
use masonry::palette;
use masonry::parley::style::{FontFamily, FontStack, GenericFamily, StyleProperty};
use masonry::peniko::{Color, Fill};
use masonry::theme::default_property_set;
use masonry::vello::Scene;
use masonry::{TextAlign, TextAlignOptions};
use masonry_winit::app::{AppDriver, DriverCtx, NewWindow, WindowId};
use masonry_winit::winit::window::Window;
use tracing::{Span, trace_span};

use masonry::properties::types::AsUnit;
use vello::peniko::color::AlphaColor;
use vello_svg::append;
// use winit::error::EventLoopError;
use xilem::style::Style;
use xilem::view::{
    CrossAxisAlignment, GridExt, MainAxisAlignment, flex_col, flex_row, grid, label, portal,
    sized_box, text_button,
};
use xilem::{EventLoop, Pod, ViewCtx, WidgetView, WindowOptions, Xilem};
use xilem_core::one_of::Either;
use xilem_core::{MessageContext, MessageResult, Mut, View, ViewMarker};
use crate::icons::{ICON_ARROW_LEFT, ICON_ARROW_RIGHT};

#[derive(Debug)]
pub struct IconWidget {
    svg: String
}

impl Widget for IconWidget {
    type Action = NoAction;

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
        _event: &AccessEvent,
    ) {
    }

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}

    fn layout(
        &mut self,
        _layout_ctx: &mut LayoutCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        bc: &BoxConstraints,
    ) -> Size {
        let size = Size::new(20., 20.);
        bc.constrain(size)
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, scene: &mut Scene) {
        let _ = append(scene, self.svg.as_str());
    }

    fn accessibility_role(&self) -> Role {
        Role::Window
    }

    fn accessibility(
        &mut self,
        _ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        node: &mut Node,
    ) {
        node.set_label(
            format!("An icon."),
        );
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }

    fn make_trace_span(&self, id: WidgetId) -> Span {
        trace_span!("CustomWidget", id = id.trace())
    }
}

pub fn icon(svg: String) -> Icon {
    Icon {
        svg
    }
}

pub struct Icon {
    svg: String
}

impl ViewMarker for Icon {}

impl<State, Action> View<State, Action, ViewCtx> for Icon {
    type Element = Pod<IconWidget>;
    type ViewState = ();

    fn build(&self, ctx: &mut ViewCtx, app_state: &mut State) -> (Self::Element, Self::ViewState) {
        let pod = ctx.create_pod(
            IconWidget{
                svg: self.svg.clone()
            }
        );
        (pod, ())
    }

    fn rebuild(&self, prev: &Self, view_state: &mut Self::ViewState, ctx: &mut ViewCtx, element: Mut<'_, Self::Element>, app_state: &mut State) {
        // println!("Icon widget rebuild requested.")
    }

    fn teardown(&self, view_state: &mut Self::ViewState, ctx: &mut ViewCtx, element: Mut<'_, Self::Element>) {
        // println!("Icon widget tear down requested.")
    }

    fn message(&self, view_state: &mut Self::ViewState, message: &mut MessageContext, element: Mut<'_, Self::Element>, app_state: &mut State) -> MessageResult<Action> {
        // println!("Icon widget message received: {:?}", message);
        MessageResult::Stale
    }
}
