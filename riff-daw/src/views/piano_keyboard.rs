use std::any::{type_name, Any};
use masonry::accesskit::{Node, Role};
use masonry::core::{
    AccessCtx, AccessEvent, BoxConstraints, ChildrenIds, EventCtx, LayoutCtx,
    PaintCtx, PointerEvent, PropertiesMut, PropertiesRef, RegisterCtx,
    TextEvent, Widget, WidgetId,
};
use masonry::kurbo::{Affine, BezPath, Point, Rect, Size, Stroke, Vec2};
use masonry::palette;
use masonry::parley::style::{FontFamily, FontStack, GenericFamily, StyleProperty};
use masonry::peniko::{Color, Fill};
use masonry::vello::Scene;
use masonry::{TextAlign, TextAlignOptions};
use tracing::{Span, trace_span};
use xilem::{Pod, ViewCtx, };
use xilem_core::{MessageContext, MessageResult, Mut, View, ViewMarker};
use crate::state::RiffDAWState;

#[derive(Debug)]
pub enum PianoKeyBoardEvent {
    NoteOn(i32),
    NoteOff(i32),
}

#[derive(Debug)]
pub struct PianoKeyboardWidget{
    entity_height_in_pixels: f64,
    white_key_length: f64,
    black_key_length: f64,

    height: f64,
    width: f64,

    // zoom
    zoom_vertical: f64,
    zoom_factor: f64,
}

impl Widget for PianoKeyboardWidget {
    type Action = PianoKeyBoardEvent;

    fn on_pointer_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &PointerEvent,
    ) {
        match event {
            PointerEvent::Down(pointer_button_event) => {
                ctx.capture_pointer();
                // Changes in pointer capture impact appearance, but not accessibility node
                ctx.request_paint_only();
                let button = pointer_button_event.button.as_ref().unwrap();
                let position = ctx.local_position(pointer_button_event.state.position);
                println!("Piano keyboard button {:?} pressed, button={}, position: x={}, y={}", ctx.widget_id(), *button as i32, position.x, position.y);
                let key_width = self.height / 128.0;
                let note = (self.height - position.y) / key_width;
                ctx.submit_action::<Self::Action>(PianoKeyBoardEvent::NoteOn(note as i32));
            }
            PointerEvent::Up(pointer_button_event) => {
                if ctx.is_active() && ctx.is_hovered() {
                    let position = ctx.local_position(pointer_button_event.state.position);
                    let button = pointer_button_event.button;
                    println!("Piano keyboard button {:?} released, button={}, position: x={}, y={}", ctx.widget_id(), *(button.as_ref().unwrap()) as i32, position.x, position.y);
                    let key_width = self.height / 128.0;
                    let note = (self.height - position.y) / key_width;
                    ctx.submit_action::<Self::Action>(PianoKeyBoardEvent::NoteOff(note as i32));
                }
                // Changes in pointer capture impact appearance, but not accessibility node
                ctx.request_paint_only();
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
        Size::new(100.0, 10. * 128.)
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

        self.paint_keyboard(ctx, scene);
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
            format!("Piano keyboard."),
        );
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }

    fn make_trace_span(&self, id: WidgetId) -> Span {
        trace_span!("CustomWidget", id = id.trace())
    }
}

impl PianoKeyboardWidget{

    fn paint_keyboard(&mut self, context: &mut PaintCtx<'_>, scene: &mut Scene) {
        let bounds = context.size();

        let x_start: f64 = 0.0;
        let mut y_start = bounds.height;
        let adjusted_entity_height_in_pixels = (1280. / 128.) * self.zoom_vertical;

        let mut octaves_drawn = 0;
        let mut keys_drawn = 0;
        for index in 0..11 {
            if y_start >= 0.0 && y_start <= bounds.height {
                println!("y_start: {}", y_start);
                self.paint_white_left_key(context, x_start, y_start, adjusted_entity_height_in_pixels, scene);
                keys_drawn += 1;
                let octave = format!("C{}", index - 2);


                let (fontContext, layoutContext) = context.text_contexts();
                let mut text_layout_builder = layoutContext.ranged_builder(fontContext, octave.as_str(), 1.0, true);
                text_layout_builder.push_default(
                    StyleProperty::FontStack(
                        FontStack::Single(
                            FontFamily::Generic(GenericFamily::Serif),
                        )
                    )
                );
                text_layout_builder.push_default(StyleProperty::FontSize(8.0));
                let mut text_layout = text_layout_builder.build(octave.as_str());
                text_layout.break_all_lines(None);
                text_layout.align(None, TextAlign::Start, TextAlignOptions::default());

                // We can pass a transform matrix to rotate the text we render
                let fill_color = Color::from_rgba8(0x00, 0x00, 0x00, 0x7F);
                masonry::core::render_text(
                    scene,
                    Affine::translate(Vec2 { x: self.white_key_length - 25.0, y: y_start - adjusted_entity_height_in_pixels - 3. }),
                    &text_layout,
                    &[fill_color.into()],
                    true,
                );



                // canvas.draw_text_align(octave, Point { x: self.white_key_length - 25.0, y: y_start - 2.0 }, &font, &paint, Align::Left);
                y_start -= adjusted_entity_height_in_pixels;
                self.paint_black_key(context, x_start, y_start, adjusted_entity_height_in_pixels, scene);
                keys_drawn += 1;
                y_start -= adjusted_entity_height_in_pixels;
                self.paint_white_t_key(context, x_start, y_start, adjusted_entity_height_in_pixels, scene);
                keys_drawn += 1;
                y_start -= adjusted_entity_height_in_pixels;
                self.paint_black_key(context, x_start, y_start, adjusted_entity_height_in_pixels, scene);
                keys_drawn += 1;
                y_start -= adjusted_entity_height_in_pixels;
                self.paint_white_right_key(context, x_start, y_start, adjusted_entity_height_in_pixels, scene);
                keys_drawn += 1;
                y_start -= adjusted_entity_height_in_pixels;
                self.paint_white_left_key(context, x_start, y_start, adjusted_entity_height_in_pixels, scene);
                keys_drawn += 1;
                y_start -= adjusted_entity_height_in_pixels;
                self.paint_black_key(context, x_start, y_start, adjusted_entity_height_in_pixels, scene);
                keys_drawn += 1;
                y_start -= adjusted_entity_height_in_pixels;
                self.paint_white_t_key(context, x_start, y_start, adjusted_entity_height_in_pixels, scene);
                keys_drawn += 1;
                y_start -= adjusted_entity_height_in_pixels;
                self.paint_black_key(context, x_start, y_start, adjusted_entity_height_in_pixels, scene);
                keys_drawn += 1;
                y_start -= adjusted_entity_height_in_pixels;
                self.paint_white_t_key(context, x_start, y_start, adjusted_entity_height_in_pixels, scene);
                keys_drawn += 1;
                y_start -= adjusted_entity_height_in_pixels;
                self.paint_black_key(context, x_start, y_start, adjusted_entity_height_in_pixels, scene);
                keys_drawn += 1;
                y_start -= adjusted_entity_height_in_pixels;
                self.paint_white_right_key(context, x_start, y_start, adjusted_entity_height_in_pixels, scene);
                keys_drawn += 1;
                y_start -= adjusted_entity_height_in_pixels;

                octaves_drawn += 1;
            }
        }

        println!("octaves_drawn: {}, keys_drawn={}", octaves_drawn, keys_drawn);
    }

    fn paint_white_left_key(&mut self, context: &mut PaintCtx<'_>, x: f64, y: f64, adjusted_entity_height_in_pixels: f64, scene: &mut Scene) {
        let mut path = BezPath::new();
        path.move_to(Point{x, y});
        path.line_to(Point{x: x + self.white_key_length, y});
        path.line_to(Point{x: x + self.white_key_length, y: y - (adjusted_entity_height_in_pixels * 1.5)});
        path.line_to(Point{x: x + self.black_key_length, y: y - (adjusted_entity_height_in_pixels * 1.5)});

        let stroke_color = Color::from_rgb8(0, 0, 0);
        scene.stroke(
            &Stroke::new(1.0),
            Affine::IDENTITY,
            stroke_color,
            None,
            &path,
        );
    }

    fn paint_white_t_key(&mut self, context: &mut PaintCtx<'_>, x: f64, y: f64, adjusted_entity_height_in_pixels: f64, scene: &mut Scene) {
        let mut path = BezPath::new();
        path.move_to(Point{x: x + self.black_key_length, y: y + (adjusted_entity_height_in_pixels / 2.0)});
        path.line_to(Point{x: x + self.white_key_length, y: y + (adjusted_entity_height_in_pixels / 2.0)});
        path.line_to(Point{x: x + self.white_key_length, y: y - (adjusted_entity_height_in_pixels * 1.5)});
        path.line_to(Point{x: x + self.black_key_length, y: y - (adjusted_entity_height_in_pixels * 1.5)});

        let stroke_color = Color::from_rgb8(0, 0, 0);
        scene.stroke(
            &Stroke::new(1.0),
            Affine::IDENTITY,
            stroke_color,
            None,
            &path,
        );
    }

    fn paint_white_right_key(&mut self, context: &mut PaintCtx<'_>, x: f64, y: f64, adjusted_entity_height_in_pixels: f64, scene: &mut Scene) {
        let mut path = BezPath::new();
        path.move_to(Point{x: x + self.black_key_length, y: y + (adjusted_entity_height_in_pixels / 2.0)});
        path.line_to(Point{x: x + self.white_key_length, y: y + (adjusted_entity_height_in_pixels / 2.0)});
        path.line_to(Point{x: x + self.white_key_length, y: y - adjusted_entity_height_in_pixels});
        path.line_to(Point{x, y: y - adjusted_entity_height_in_pixels});

        let stroke_color = Color::from_rgb8(0, 0, 0);
        scene.stroke(
            &Stroke::new(1.0),
            Affine::IDENTITY,
            stroke_color,
            None,
            &path,
        );
    }

    fn paint_black_key(&self, context: &mut PaintCtx<'_>, x: f64, y: f64, adjusted_entity_height_in_pixels: f64, scene: &mut Scene) {
        let rect = Rect::new(x, y - adjusted_entity_height_in_pixels, x + self.black_key_length, y);

        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            palette::css::BLACK,
            None,
            &rect,
        );
    }
}


pub fn piano_keyboard<State: 'static, Action: 'static>() -> PianoKeyboard<State, Action> {
    PianoKeyboard { on_note_on: None, on_note_off: None }
}

pub struct PianoKeyboard<State, Action> {
    on_note_on: Option<Box<dyn Fn(&mut State, i32, i32) -> Action + Send + Sync + 'static>>,
    on_note_off: Option<Box<dyn Fn(&mut State, i32, i32) -> Action + Send + Sync + 'static>>,
}

impl<State, Action> PianoKeyboard<State, Action> {

    pub fn on_note_on(mut self, on_note_on: Box<dyn Fn(&mut State, i32, i32) -> Action + Send + Sync + 'static>) -> Self {
        self.on_note_on = Some(on_note_on);
        self
    }

    pub fn on_note_off(mut self, on_note_off: Box<dyn Fn(&mut State, i32, i32) -> Action + Send + Sync + 'static>) -> Self {
        self.on_note_off = Some(on_note_off);
        self
    }

}

impl<State, Action> ViewMarker for PianoKeyboard<State, Action> {}

impl<State: 'static, Action: 'static> View<State, Action, ViewCtx> for PianoKeyboard<State, Action> {
    type Element = Pod<PianoKeyboardWidget>;
    type ViewState = ();

    fn build(&self, ctx: &mut ViewCtx, app_state: &mut State) -> (Self::Element, Self::ViewState) {
        let pod = ctx.with_action_widget(|cx| cx.create_pod(
                PianoKeyboardWidget{
                    entity_height_in_pixels: 5.0,
                    white_key_length: 100.0,
                    black_key_length: 50.0,
                    height: 1000.0,
                    width: 400.0,
                    zoom_vertical: 1.0,
                    zoom_factor: 0.01,
                }
            )
        );
        (pod, ())
    }

    fn rebuild(&self, prev: &Self, view_state: &mut Self::ViewState, ctx: &mut ViewCtx, element: Mut<'_, Self::Element>, app_state: &mut State) {
        // println!("Piano keyboard widget rebuild requested.")
    }

    fn teardown(&self, view_state: &mut Self::ViewState, ctx: &mut ViewCtx, element: Mut<'_, Self::Element>) {
        // println!("Piano keyboard widget tear down requested.")
    }

    fn message(&self, view_state: &mut Self::ViewState, message: &mut MessageContext, element: Mut<'_, Self::Element>, app_state: &mut State) -> MessageResult<Action> {
        let mut message_result = MessageResult::Stale;
        match message.take_message::<PianoKeyBoardEvent>() {
            Some(event) => {
                match event.as_ref() {
                    PianoKeyBoardEvent::NoteOn(note) => {
                        println!("^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ PianoKeyboard - Note on");
                        if let Some(on_note_on) = self.on_note_on.as_ref() {
                            message_result = MessageResult::Action(on_note_on(app_state, *note, 0));
                        }
                    }
                    PianoKeyBoardEvent::NoteOff(note) => {
                        println!("^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ PianoKeyboard - Note off");
                        if let Some(on_note_off) = self.on_note_off.as_ref() {
                            message_result = MessageResult::Action(on_note_off(app_state, *note, 0));
                        }
                    }
                }

                message_result
            }
            None => {
                tracing::error!(
                    "Wrong message type in PianoKeyboard::message: {message:?} expected {}",
                    type_name::<PianoKeyBoardEvent>()
                );
                MessageResult::Stale
            }
        }
    }
}
