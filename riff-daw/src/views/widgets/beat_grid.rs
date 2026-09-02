use std::any::Any;
use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};
use geo::AffineTransform;
use itertools::Itertools;
use masonry::accesskit::{Node, Role};
use masonry::core::{
    AccessCtx, AccessEvent, BoxConstraints, ChildrenIds, ErasedAction, EventCtx, LayoutCtx,
    NewWidget, NoAction, PaintCtx, PointerEvent, PropertiesMut, PropertiesRef, RegisterCtx,
    TextEvent, Widget, WidgetId,
};
use masonry::kurbo::{Affine, BezPath, Line, Point, Rect, Size, Stroke, Vec2};
use masonry::palette;
use masonry::parley::style::{FontFamily, FontStack, GenericFamily, StyleProperty};
use masonry::peniko::{Color, Fill};
use masonry::theme::default_property_set;
use masonry::vello::Scene;
use masonry::{TextAlign, TextAlignOptions};
use masonry::parley::{FontWeight, Layout};
use masonry_winit::app::{AppDriver, DriverCtx, NewWindow, WindowId};
use masonry_winit::winit::window::Window;
use tracing::{Span, trace_span, trace};

use masonry::properties::types::AsUnit;
use masonry::ui_events::keyboard::{Key, NamedKey};
use masonry_core::core::{render_text, BrushIndex, CursorIcon, PointerButton, PointerButtonEvent, QueryCtx, Update, UpdateCtx};
use masonry_core::core::keyboard::KeyState;
use masonry_core::dpi::PhysicalPosition;
use strum_macros::Display;
use uuid::Uuid;
use vello::peniko::color::AlphaColor;
use crate::constants::NOTE_NAMES;
use crate::domain::{DAWItemID, DAWItemLength, DAWItemPosition, DAWItemVerticalIndex, NoteExpressionType, Project, Riff, RiffItemType, RiffReference, RiffReferenceMode, Track, TrackEvent, TrackType, UuidWrapper};
use crate::event::{AutomationEditType, CurrentView, DAWEvents, LoopChangeType, OperationModeType, RiffGridChangeType, TrackChangeType, TranslateDirection, TranslationEntityType};
use crate::state::{AutomationViewMode, MidiPolyphonicExpressionNoteId, RiffDAWState};
use crate::utils::DAWUtils;

#[derive(Debug, Clone)]
pub enum DrawMode {
    Point,
    Line,
    Curve,
    Triplet,
}

#[derive(Debug, Clone)]
pub enum DrawingAreaType {
    PianoRoll,
    TrackGrid,
    Automation,
    Riff,
    RiffGrid,
    RiffArrangement,
}

#[derive(Debug, Clone, Display)]
pub enum EditMode {
    Inactive,
    ChangeStart,
    Move,
    ChangeEnd,
}

#[derive(Debug, Clone)]
pub enum DragCycle {
    NotStarted,
    MousePressed,
    Dragging,
    MouseReleased,
    CtrlMousePressed,
    CtrlDragging,
    CtrlMouseReleased,
}

#[derive(Debug)]
pub enum MouseButton {
    Button1,
    Button2,
    Button3,
}

pub trait MouseHandler {
    fn handle_mouse_motion(&mut self, cx: &mut EventCtx, x: f64, y: f64, mouse_button: MouseButton, control_key: bool, shift_key: bool, alt_key: bool);
    fn handle_mouse_press(&mut self, cx: &mut EventCtx, x: f64, y: f64, mouse_button: MouseButton, control_key: bool, shift_key: bool, alt_key: bool);
    fn handle_mouse_release(&mut self, cx: &mut EventCtx, x: f64, y: f64, mouse_button: MouseButton, control_key: bool, shift_key: bool, alt_key: bool, data: String);
}


pub trait CustomPainter {
    fn paint_custom(&mut self,
                    context: &mut PaintCtx<'_>,
                    entity_height_in_pixels: f64,
                    beat_width_in_pixels: f64,
                    zoom_horizontal: f64,
                    zoom_vertical: f64,
                    drawing_area_widget_name: Option<String>,
                    mouse_pointer_x: f64,
                    mouse_pointer_y: f64,
                    mouse_pointer_previous_x: f64,
                    mouse_pointer_previous_y: f64,
                    draw_mode_on: bool,
                    draw_mode: DrawMode,
                    draw_mode_start_x: f64,
                    draw_mode_start_y: f64,
                    draw_mode_end_x: f64,
                    draw_mode_end_y: f64,
                    scene: &mut Scene,
                    operation_mode: &OperationModeType,
                    drag_started: bool,
                    edit_drag_cycle: &DragCycle,
    ) -> (f64, f64);
    fn track_cursor_time_in_beats(&self) -> f64;
    fn set_track_cursor_time_in_beats(&mut self, track_cursor_time_in_beats: f64);

    fn as_any(&mut self) -> &mut dyn Any;

    fn draw_text(&self, context: &mut PaintCtx<'_>, scene: &mut Scene, x: f64, y: f64, text: &str, color: Color, size: f32, font_weight: FontWeight) {
        let (fontContext, layoutContext) = context.text_contexts();
        let mut text_layout_builder = layoutContext.ranged_builder(fontContext, text.clone(), 1.0, true);
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

        // We can pass a transform matrix to rotate the text we render
        masonry::core::render_text(
            scene,
            Affine::translate(Vec2 { x, y }),
            &text_layout,
            &[color.into()],
            true,
        );
    }
}

pub struct PianoRollCustomPainter {
    pub track_cursor_time_in_beats: f64,
    pub project: Arc<Mutex<Project>>,
    pub piano_roll_mpe_note_id: MidiPolyphonicExpressionNoteId,
    pub selected_track_uuid: String,
    pub selected_riff_uuid: String,
    pub selected_riff_events: Vec<String>,
}

impl CustomPainter for PianoRollCustomPainter {
    fn paint_custom(&mut self,
                    context: &mut PaintCtx<'_>,
                    entity_height_in_pixels: f64,
                    beat_width_in_pixels: f64,
                    zoom_horizontal: f64,
                    zoom_vertical: f64,
                    drawing_area_widget_name: Option<String>,
                    mouse_pointer_x: f64,
                    mouse_pointer_y: f64,
                    mouse_pointer_previous_x: f64,
                    mouse_pointer_previous_y: f64,
                    draw_mode_on: bool,
                    draw_mode: DrawMode,
                    draw_mode_start_x: f64,
                    draw_mode_start_y: f64,
                    draw_mode_end_x: f64,
                    draw_mode_end_y: f64,
                    scene: &mut Scene,
                    operation_mode: &OperationModeType,
                    drag_started: bool,
                    edit_drag_cycle: &DragCycle) -> (f64, f64) {
        let panel_size: Rect = context.size().to_rect();
        let height = panel_size.height();
        let adjusted_entity_height_in_pixels = height / 128.0; // FIXME

        match self.project.lock() {
            Ok(project) => {
                let note_expression_note_id = self.piano_roll_mpe_note_id.clone() as i32;
                let adjusted_beat_width_in_pixels = beat_width_in_pixels * zoom_horizontal;
                // let mut edit_mode = EditMode::Inactive;

                match project.song.tracks().iter().enumerate().find(|(track_index, track)| track.uuid() == self.selected_track_uuid) {
                    Some((track_index, track)) => {
                        let (red, green, blue, _) = track.colour();

                        for riff in track.riffs().iter() {
                            if riff.uuid.uuid.to_string() == self.selected_riff_uuid {
                                let unselected_event_colour = if let Some((red, green, blue, _)) = riff.colour {
                                     (red, green, blue, 1.0)
                                } else {
                                    (red, green, blue, 1.0)
                                };

                                // find all the selected notes
                                let selected_riff_events = self.selected_riff_events.clone();
                                let mut selected_notes = vec![];
                                for event in riff.events.iter().filter(|event| {
                                    if let TrackEvent::Note(note) = event {
                                        selected_riff_events.contains(&note.id.uuid.to_string())
                                    } else {
                                        false
                                    }
                                }) {
                                    if let TrackEvent::Note(note) = event {
                                        selected_notes.push(note.clone());
                                    }
                                }

                                for track_event in riff.events.iter() {
                                    let mut event_colour = unselected_event_colour.clone();

                                    match track_event {
                                        TrackEvent::Note(note) => {
                                            if note_expression_note_id == -1 || note_expression_note_id == note.note_id {
                                                let note_number = note.note;
                                                let note_y_pos_inverted = note_number as f64 * adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels;
                                                let x = note.position * adjusted_beat_width_in_pixels;
                                                // let x_original = x;
                                                let y = height - note_y_pos_inverted;
                                                // let y_original = y;
                                                let width = note.length * adjusted_beat_width_in_pixels;

                                                let is_selected = self.selected_riff_events.iter().any(|id| id.as_str() == note.id.uuid.to_string().as_str());
                                                if is_selected {
                                                    event_colour = (0.0, 0.0, 1.0, 1.0);
                                                }
                                                // context.set_source_rgba(event_colour.0, event_colour.1, event_colour.2, event_colour.3);

                                                // self.edit_item_handler.handle_item_edit(
                                                //     context,
                                                //     note,
                                                //     operation_mode,
                                                //     mouse_pointer_x,
                                                //     mouse_pointer_y,
                                                //     mouse_pointer_previous_x,
                                                //     mouse_pointer_previous_y,
                                                //     adjusted_entity_height_in_pixels,
                                                //     adjusted_beat_width_in_pixels,
                                                //     x,
                                                //     y,
                                                //     width,
                                                //     height,
                                                //     drawing_area,
                                                //     edit_drag_cycle,
                                                //     tx_from_ui.clone(),
                                                //     true,
                                                //     track_uuid.clone(),
                                                //     note,
                                                //     true,
                                                //     track_index as f64,
                                                //     is_selected,
                                                //     selected_notes.clone()
                                                // );

                                                let rect = Rect::new(x, y, x + width, y + adjusted_entity_height_in_pixels);
                                                // println!("################################# Draw note - note_y_pos_inverted={}, note={}, height={}, x={}, y={}, width={}, adjusted_entity_height_in_pixels={}", note_y_pos_inverted, note_number, height, x, y, width, adjusted_entity_height_in_pixels);
                                                let note_colour = Color::from_rgb8((event_colour.0 * 255.0) as u8, (event_colour.1 * 255.0) as u8, (event_colour.2 * 255.0) as u8);
                                                scene.fill(
                                                    Fill::NonZero,
                                                    Affine::IDENTITY,
                                                    note_colour.clone(),
                                                    None,
                                                    &rect,
                                                );

                                                if note.note_id > -1 {
                                                    self.draw_text(context, scene, x, y - adjusted_entity_height_in_pixels, format!("{}", note.note_id).as_str(), note_colour.clone(), 8.0, FontWeight::EXTRA_BOLD);
                                                }


                                                if note.riff_start_note {
                                                    let rect = Rect::new(x, y, x + 2.0, y + adjusted_entity_height_in_pixels);
                                                    scene.fill(
                                                        Fill::NonZero,
                                                        Affine::IDENTITY,
                                                        palette::css::BLACK,
                                                        None,
                                                        &rect,
                                                    );
                                                }
                                            }
                                        }
                                        _ => (),
                                    }
                                }
                                break;
                            }
                        }
                    }
                    None => println!("Piano roll: Could find selected track."),
                }
            }
            _ => println!("Piano roll: Could lock the project.")
        }

        // Draw the note and the octave at the cursor
        // FIXME get the cursor position
        // context.move_to(mouse_pointer_x, mouse_pointer_y);
        // context.set_font_size(12.0);
        let note_number = ((height - mouse_pointer_y) / adjusted_entity_height_in_pixels) as i32;
        let note_name_index = (note_number % 12) as usize;

        if note_name_index >= 0 && note_name_index < NOTE_NAMES.len() && mouse_pointer_x > 3.0 && mouse_pointer_y > 3.0 {
            let note_name = NOTE_NAMES[note_name_index];
            let octave_number = note_number / 12 - 2;
            // self.draw_text(context, scene, x, y - adjusted_entity_height_in_pixels, format!("{}{}", note_name, octave_number).as_str(), palette::css::BLACK);
            self.draw_text(context, scene, mouse_pointer_x + 5.0, mouse_pointer_y,
                           format!("Piano roll: mouse x={}, mouse y={}, note={} {}{}", mouse_pointer_x, mouse_pointer_y, note_number, note_name, octave_number).as_str(), Color::BLACK, 8.0, FontWeight::EXTRA_BOLD);
        }

        (0.0, 0.0)
    }

    fn track_cursor_time_in_beats(&self) -> f64 {
        self.track_cursor_time_in_beats
    }

    fn set_track_cursor_time_in_beats(&mut self, track_cursor_time_in_beats: f64) {
        self.track_cursor_time_in_beats = track_cursor_time_in_beats;
    }

    fn as_any(&mut self) -> &mut dyn Any {
        self
    }
}


pub struct PianoRollMouseCoordHelper;

impl BeatGridMouseCoordHelper for PianoRollMouseCoordHelper {
    type Action = DAWEvents;

    fn get_entity_vertical_value(&self, y: f64, entity_height_in_pixels: f64, zoom_vertical: f64) -> f64 {
        ((127.0 * entity_height_in_pixels * zoom_vertical) - y) / (entity_height_in_pixels * zoom_vertical)
    }

    fn select_single(&self, cx: &mut EventCtx, x: f64, y: i32, add_to_select: bool) {
        let _ = cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffEventsSelectSingle(x, y, add_to_select), None));
    }

    fn select_multiple(&self, cx: &mut EventCtx, x: f64, y: i32, x2: f64, y2: i32, add_to_select: bool) {
        let _ = cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffEventsSelectMultiple(x, y2, x2, y, add_to_select), None));
    }

    fn deselect_single(&self, cx: &mut EventCtx, x: f64, y: i32) {
        let _ = cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffEventsDeselectSingle(x, y), None));
    }

    fn deselect_multiple(&self, cx: &mut EventCtx, x: f64, y: i32, x2: f64, y2: i32) {
        let _ = cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffEventsDeselectMultiple(x, y2, x2, y), None));
    }

    fn add_entity(&self, cx: &mut EventCtx, y_index: i32, time: f64, duration: f64, _entity_uuid: String) {
        let _ = cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffAddNote(vec![(y_index, time, duration)]), None));
    }

    fn add_entity_extra(&self, cx: &mut EventCtx, y_index: i32, time: f64, duration: f64, entity_uuid: String) {
    }

    fn delete_entity(&self, cx: &mut EventCtx, y_index: i32, time: f64, _entity_uuid: String) {
        let _ = cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffDeleteNote(y_index, time), None));
    }

    fn cut_selected(&self, cx: &mut EventCtx) {
        let _ = cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffCutSelected, None));
    }

    fn copy_selected(&self, cx: &mut EventCtx) {
        let _ = cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffCopySelected, None));
    }

    fn paste_selected(&self, cx: &mut EventCtx) {
        let _ = cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffPasteSelected, None));
    }

    fn handle_translate_up(&self, cx: &mut EventCtx) {
        let _ = cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffTranslateSelected(TranslationEntityType::Note, TranslateDirection::Up), None));
    }

    fn handle_translate_down(&self, cx: &mut EventCtx) {
        let _ = cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffTranslateSelected(TranslationEntityType::Note, TranslateDirection::Down), None));
    }

    fn handle_translate_left(&self, cx: &mut EventCtx) {
        let _ = cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffTranslateSelected(TranslationEntityType::Note, TranslateDirection::Left), None));
    }

    fn handle_translate_right(&self, cx: &mut EventCtx) {
        let _ = cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffTranslateSelected(TranslationEntityType::Note, TranslateDirection::Right), None));
    }

    fn handle_quantise(&self, cx: &mut EventCtx) {
        let _ = cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffQuantiseSelected, None));
    }

    fn handle_increase_entity_length(&self, cx: &mut EventCtx) {
        let _ = cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffChangeLengthOfSelected(true), None));
    }

    fn handle_decrease_entity_length(&self, cx: &mut EventCtx) {
        let _ = cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffChangeLengthOfSelected(false), None));
    }

    fn set_start_note(&self, cx: &mut EventCtx, y_index: i32, time: f64) {
        let _ = cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffSetStartNote(y_index, time), None));
    }

    fn set_riff_reference_play_mode(&self, cx: &mut EventCtx, y_index: i32, time: f64) {
        let _ = cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffReferencePlayMode(y_index, time), None));
    }

    fn handle_windowed_zoom(&self, cx: &mut EventCtx, x1: f64, y1: f64, x2: f64, y2: f64) {
        let _ = cx.submit_action::<Self::Action>(DAWEvents::PianoRollWindowedZoom {x1, y1, x2, y2});
    }

    fn cycle_entity_selection(&self, cx: &mut EventCtx, y_index: i32, time: f64) {
    }

    fn select_underlying_entity(&self, cx: &mut EventCtx, y_index: i32, time: f64) {
    }
}


pub struct TrackGridMouseCoordHelper;

impl BeatGridMouseCoordHelper for TrackGridMouseCoordHelper {
    type Action = DAWEvents;

    fn get_entity_vertical_value(&self, y: f64, entity_height_in_pixels: f64, zoom_vertical: f64) -> f64 {
        y / entity_height_in_pixels * zoom_vertical
    }

    fn select_single(&self, cx: &mut EventCtx, x: f64, y: i32, add_to_select: bool) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffReferencesSelectSingle(x as f64, y, add_to_select), None));
    }

    fn select_multiple(&self, cx: &mut EventCtx, x: f64, y: i32, x2: f64, y2: i32, add_to_select: bool) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffReferencesSelectMultiple(x as f64, y, x2 as f64, y2, add_to_select), None));
    }

    fn deselect_single(&self, cx: &mut EventCtx, x: f64, y: i32) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffReferencesDeselectSingle(x as f64, y), None));
    }

    fn deselect_multiple(&self, cx: &mut EventCtx, x: f64, y: i32, x2: f64, y2: i32) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffReferencesDeselectMultiple(x as f64, y, x2 as f64, y2), None));
    }

    fn add_entity(&self, cx: &mut EventCtx, y_index: i32, time: f64, _duration: f64, _entity_uuid: String) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffReferenceAdd(y_index, time as f64), None));
    }

    fn add_entity_extra(&self, cx: &mut EventCtx, y_index: i32, time: f64, duration: f64, entity_uuid: String) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffAddWithTrackIndex(entity_uuid, duration as f64, y_index), None));
    }

    fn delete_entity(&self, cx: &mut EventCtx, y_index: i32, time: f64, _entity_uuid: String) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffReferenceDelete(y_index, time as f64), None));
    }

    fn cut_selected(&self, cx: &mut EventCtx) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffReferenceCutSelected, None));
    }

    fn copy_selected(&self, cx: &mut EventCtx) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffReferenceCopySelected, None));
    }

    fn paste_selected(&self, cx: &mut EventCtx) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffReferencePaste, None));
    }

    fn handle_translate_up(&self, _cx: &mut EventCtx) {

    }

    fn handle_translate_down(&self, _cx: &mut EventCtx) {

    }

    fn handle_translate_left(&self, _cx: &mut EventCtx) {

    }

    fn handle_translate_right(&self, _cx: &mut EventCtx) {

    }

    fn handle_quantise(&self, _cx: &mut EventCtx) {

    }

    fn handle_increase_entity_length(&self, _cx: &mut EventCtx) {
    }

    fn handle_decrease_entity_length(&self, _cx: &mut EventCtx) {
    }

    fn set_start_note(&self, cx: &mut EventCtx, y_index: i32, time: f64) {
    }

    fn set_riff_reference_play_mode(&self, cx: &mut EventCtx, y_index: i32, time: f64) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffReferencePlayMode(y_index, time as f64), None));
    }

    fn handle_windowed_zoom(&self, cx: &mut EventCtx, x1: f64, y1: f64, x2: f64, y2: f64) {
    }

    fn cycle_entity_selection(&self, cx: &mut EventCtx, y_index: i32, time: f64) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffReferenceIncrementRiff{track_index: y_index, position: time as f64}, None));
    }

    fn select_underlying_entity(&self, cx: &mut EventCtx, y_index: i32, time: f64) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffSelectWithTrackIndex{track_index: y_index, position: time as f64}, None));
    }
}


pub struct AutomationMouseCoordHelper;

impl BeatGridMouseCoordHelper for AutomationMouseCoordHelper {
    type Action = DAWEvents;

    fn get_entity_vertical_value(&self, y: f64, entity_height_in_pixels: f64, zoom_vertical: f64) -> f64 {
        let value = ((127.0 * entity_height_in_pixels * zoom_vertical) - y) / (entity_height_in_pixels * zoom_vertical);
        if value < 0.0 {
            0.0
        }
        else {
            value
        }
    }

    fn select_single(&self, cx: &mut EventCtx, x: f64, y: i32, add_to_select: bool) {
    }

    fn select_multiple(&self, cx: &mut EventCtx, x: f64, y: i32, x2: f64, y2: i32, add_to_select: bool) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::AutomationSelectMultiple(x, y2, x2, y, add_to_select), None));
    }

    fn deselect_single(&self, cx: &mut EventCtx, x: f64, y: i32) {
    }

    fn deselect_multiple(&self, cx: &mut EventCtx, x: f64, y: i32, x2: f64, y2: i32) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::AutomationDeselectMultiple(x, y2, x2, y), None));
    }

    fn add_entity(&self, cx: &mut EventCtx, y_index: i32, time: f64, _duration: f64, _entity_uuid: String) {
        let mut new_entities = vec![];
        new_entities.push((time, y_index));
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::AutomationAdd(new_entities), None));
    }

    fn add_entity_extra(&self, cx: &mut EventCtx, y_index: i32, time: f64, duration: f64, entity_uuid: String) {
    }

    fn delete_entity(&self, cx: &mut EventCtx, _y_index: i32, time: f64, _entity_uuid: String) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::AutomationDelete(time), None));
    }

    fn cut_selected(&self, cx: &mut EventCtx) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::AutomationCut, None));
    }

    fn copy_selected(&self, cx: &mut EventCtx) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::AutomationCopy, None));
    }

    fn paste_selected(&self, cx: &mut EventCtx) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::AutomationPaste, None));
    }

    fn handle_translate_up(&self, cx: &mut EventCtx) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::AutomationTranslateSelected(TranslationEntityType::Any, TranslateDirection::Up), None));
    }

    fn handle_translate_down(&self, cx: &mut EventCtx) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::AutomationTranslateSelected(TranslationEntityType::Any, TranslateDirection::Down), None));
    }

    fn handle_translate_left(&self, cx: &mut EventCtx) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::AutomationTranslateSelected(TranslationEntityType::Any, TranslateDirection::Left), None));
    }

    fn handle_translate_right(&self, cx: &mut EventCtx) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::AutomationTranslateSelected(TranslationEntityType::Any, TranslateDirection::Right), None));
    }

    fn handle_quantise(&self, cx: &mut EventCtx) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::AutomationQuantiseSelected, None));
    }

    fn handle_increase_entity_length(&self, cx: &mut EventCtx) {

    }

    fn handle_decrease_entity_length(&self, cx: &mut EventCtx) {

    }

    fn set_start_note(&self, cx: &mut EventCtx, y_index: i32, time: f64) {
    }

    fn set_riff_reference_play_mode(&self, cx: &mut EventCtx, y_index: i32, time: f64) {
    }

    fn handle_windowed_zoom(&self, cx: &mut EventCtx, x1: f64, y1: f64, x2: f64, y2: f64) {
    }

    fn cycle_entity_selection(&self, cx: &mut EventCtx, y_index: i32, time: f64) {
    }

    fn select_underlying_entity(&self, cx: &mut EventCtx, y_index: i32, time: f64) {
    }
}

pub struct AutomationCustomPainter {
    pub(crate) project: Arc<Mutex<Project>>,
    // pub edit_item_handler: AutomationEditItemHandler,
    pub automation_type: Option<i32>,
    pub note_expression_type: NoteExpressionType,
    pub note_expression_note_id: i32,
    pub type_to_show: AutomationViewMode,
    pub current_view: CurrentView,
    pub automation_discrete: bool,
    pub selected_effect_uuid: Option<String>,
    pub selected_track_uuid: Option<String>,
    pub selected_riff_uuid: Option<String>,
    pub selected_automation: Vec<String>,
    pub automation_edit_type: AutomationEditType,
    pub selected_riff_arrangement_uuid: Option<String>,
}

impl AutomationCustomPainter {
    pub fn new_with_edit_item_handler(project: Arc<Mutex<Project>>/*, edit_item_handler: AutomationEditItemHandler*/) -> AutomationCustomPainter {
        AutomationCustomPainter {
            project,
            // edit_item_handler,
            automation_type: None,
            note_expression_type: NoteExpressionType::Volume,
            note_expression_note_id: -1,
            type_to_show: AutomationViewMode::NoteVelocities,
            current_view: CurrentView::Track,
            automation_discrete: true,
            selected_effect_uuid: None,
            selected_track_uuid: None,
            selected_riff_uuid: None,
            selected_automation: vec![],
            automation_edit_type: AutomationEditType::Track,
            selected_riff_arrangement_uuid: None,
        }
    }

    fn draw_riff(&self, context: &mut PaintCtx<'_>, scene: &mut Scene, height: f64, entity_height_in_pixels: f64, beat_width_in_pixels: f64, zoom: f64, adjusted_beat_width_in_pixels: f64, riff: &Riff, track: &TrackType) {
        let duration_in_beats = riff.length;
        let x = riff.position * adjusted_beat_width_in_pixels;
        let y = height / 2.0;
        let width = duration_in_beats * beat_width_in_pixels * zoom;
        let (red, green, blue, alpha) = track.colour();

        // draw the riff ref rectangle
        let rect = Rect::new(x + 1.0, y + 1.0, width - 2.0, entity_height_in_pixels * 15.0 - 2.0);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::from_rgba8((red * 255.0) as u8, (green * 255.0) as u8, (blue * 255.0) as u8, (alpha * 255.0) as u8),
            None,
            &rect,
        );

        // draw the riff name
        let mut name = riff.name();
        self.draw_text(context, scene, x + 5.0, y + 10.0, format!("Track: {}", name).as_str(), Color::from_rgba8(0,0,0, 255), 9.0, FontWeight::EXTRA_BOLD);

        // draw the notes
        for track_event in riff.events.iter() {
            if let TrackEvent::Note(note) = track_event {
                let mut path = BezPath::new();
                let note_x = (riff.position + note.position) * adjusted_beat_width_in_pixels;

                // draw note
                let note_y = height / 2.0 + entity_height_in_pixels * 15.0 - (entity_height_in_pixels * 15.0 / 127.0 * note.note as f64);
                path.move_to(Point { x: note_x, y: note_y });
                path.line_to(Point { x: note_x + note.length * adjusted_beat_width_in_pixels, y: note_y });
                scene.stroke(
                    &Stroke::new(1.0),
                    Affine::IDENTITY,
                    Color::BLACK,
                    None,
                    &path,
                );
            }
        }
    }

    fn draw_track(&self, context: &mut PaintCtx<'_>, scene: &mut Scene, height: f64, entity_height_in_pixels: f64, beat_width_in_pixels: f64, zoom: f64, adjusted_beat_width_in_pixels: f64, track: &TrackType, riff_refs: &Vec<RiffReference>) {
        let (red, green, blue, _) = track.colour();

        // draw the track name

        for riff_ref in riff_refs.iter() {
            let linked_to_riff_uuid = riff_ref.linked_to();

            for riff in track.riffs().iter() {
                let colour = if let Some((red, green, blue, _)) = riff.colour() {
                    Color::from_rgba8((*red * 255.0) as u8, (*green * 255.0) as u8, (*blue * 255.0) as u8, 255)
                }
                else {
                    Color::from_rgba8((red * 255.0) as u8, (green * 255.0) as u8, (blue * 255.0) as u8, 255)
                };

                if riff.uuid().to_string() == linked_to_riff_uuid {
                    let duration_in_beats = riff.length;
                    let x = riff_ref.position * adjusted_beat_width_in_pixels;
                    let y = height / 2.0;
                    let width = duration_in_beats * beat_width_in_pixels * zoom;

                    // draw the riff ref rectangle
                    let rect = Rect::new(x + 1.0, y + 1.0, width - 2.0, entity_height_in_pixels * 15.0 - 2.0);
                    scene.fill(
                        Fill::NonZero,
                        Affine::IDENTITY,
                        colour,
                        None,
                        &rect,
                    );

                    // draw the riff name
                    let name = riff.name();
                    self.draw_text(context, scene, x + 5.0, y + 10.0, format!("Track: {}", name).as_str(), Color::from_rgba8(0,0,0, 255), 9.0, FontWeight::EXTRA_BOLD);

                    // draw the notes
                    for track_event in riff.events() {
                        if let TrackEvent::Note(note) = track_event {
                            let mut path = BezPath::new();
                            let note_x = (riff_ref.position + note.position) * adjusted_beat_width_in_pixels;

                            // draw note
                            let note_y = height / 2.0 + entity_height_in_pixels * 15.0 - (entity_height_in_pixels * 15.0 / 127.0 * note.note as f64);
                            path.move_to(Point { x: note_x, y: note_y });
                            path.line_to(Point { x: note_x + note.length * adjusted_beat_width_in_pixels, y: note_y });
                            scene.stroke(
                                &Stroke::new(1.0),
                                Affine::IDENTITY,
                                Color::BLACK,
                                None,
                                &path,
                            );
                        }
                    }
                }
            }
        }
    }

    fn draw_track_name(&self, context: &mut PaintCtx<'_>, scene: &mut Scene, name: &str) {
        self.draw_text(context, scene, 5.0, 10.0, format!("Track: {}", name).as_str(), Color::from_rgb8(0,0,0), 9.0, FontWeight::EXTRA_BOLD);
    }

    fn draw_riff_name(&self, context: &mut PaintCtx<'_>, scene: &mut Scene, name: &str) {
        self.draw_text(context, scene, 5.0, 20.0, format!("Riff: {}", name).as_str(), Color::from_rgb8(0,0,0), 9.0, FontWeight::EXTRA_BOLD);
    }

    fn draw_line(&self, context: &mut PaintCtx<'_>, scene: &mut Scene, x_start: f64, y_start: f64, x_end: f64, y_end: f64) {
        let line_colour = Color::BLACK;
        let mut path = BezPath::new();
        path.move_to(Point { x: x_start, y: y_start });
        path.line_to(Point { x: x_end, y: y_end});
        scene.stroke(
            &Stroke::new(1.0),
            Affine::IDENTITY,
            line_colour,
            None,
            &path,
        );
    }

    fn draw_automation(&self, context: &mut PaintCtx<'_>, scene: &mut Scene, height: f64, automation_discrete: bool, mut previous_point_x: &mut f64, mut previous_point_y: &mut f64, default_line_width: f64, x: f64, y: f64, automation_value: f64) {
        let mut path = BezPath::new();
        let mut line_width = default_line_width;

        if automation_discrete {
            path.move_to(Point { x: x, y: height });
        } else {
            path.move_to(Point { x: *previous_point_x, y: *previous_point_y });

            line_width = 0.75;
        }

        path.line_to(Point { x, y });
        scene.stroke(
            &Stroke::new(line_width),
            Affine::IDENTITY,
            Color::from_rgba8(0, 0, 0, 255),
            None,
            &path,
        );


        if !automation_discrete {
            let rect = Rect::new(x - 5.0, y - 5.0, 10.0, 10.0);
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                Color::from_rgba8(255, 0, 0, 255),
                None,
                &rect,
            );
        }

        self.draw_text(context, scene, x + 10.0, y, format!("{:.3}", automation_value).as_str(), Color::from_rgb8(0,0,0), 9.0, FontWeight::EXTRA_BOLD);

        if !automation_discrete {
            *previous_point_x = x;
            *previous_point_y = y;
        }
    }

    fn draw_riff_set_riff_refs(&self,
                               context: &mut PaintCtx<'_>,
                               scene: &mut Scene,
        height: f64,
        entity_height_in_pixels: f64,
        beat_width_in_pixels: f64,
        zoom_horizontal: f64,
        adjusted_beat_width_in_pixels: f64,
        track_uuid: &String,
        track: &&TrackType,
        running_position: &mut f64,
        riff_set_uuid: String
    ) {
        if let Ok(project) = self.project.lock().as_ref() {
            if let Some(riff_set) = project.song().riff_set(riff_set_uuid) {
                let mut riff_lengths = vec![];
                let mut riff_to_draw = Riff::new_with_position_length_and_colour(Uuid::new_v4(), 0.0, 4.0, None);

                // get the number of repeats
                for track in project.song().tracks().iter() {
                    // get the riff_ref
                    if let Some(riff_ref) = riff_set.get_riff_ref_for_track(track.uuid().to_string()) {
                        // get the riff
                        if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_ref.linked_to()) {
                            riff_lengths.push(riff.length as i32);

                            if track_uuid.to_string() == track.uuid().to_string() {
                                riff_to_draw = riff.clone();
                            }
                        }
                    }
                }

                let (product, unique_riff_lengths) = RiffDAWState::get_length_product(riff_lengths);

                let lowest_common_factor_in_beats = RiffDAWState::get_lowest_common_factor(unique_riff_lengths, product);

                // draw the riff reference x number of times
                for x in 0..((lowest_common_factor_in_beats as f64 / riff_to_draw.length) as i32) {
                    riff_to_draw.set_position(*running_position);
                    if riff_to_draw.name != "empty" {
                        self.draw_riff(context, scene, height, entity_height_in_pixels, beat_width_in_pixels, zoom_horizontal, adjusted_beat_width_in_pixels, &riff_to_draw, &track);
                    }
                    *running_position += riff_to_draw.length;
                }
            }
        }
    }
}


impl CustomPainter for AutomationCustomPainter {
    fn paint_custom(&mut self,
                    context: &mut PaintCtx<'_>,
                    entity_height_in_pixels: f64,
                    beat_width_in_pixels: f64,
                    zoom_horizontal: f64,
                    zoom_vertical: f64,
                    drawing_area_widget_name: Option<String>,
                    mouse_pointer_x: f64,
                    mouse_pointer_y: f64,
                    mouse_pointer_previous_x: f64,
                    mouse_pointer_previous_y: f64,
                    draw_mode_on: bool,
                    draw_mode: DrawMode,
                    draw_mode_start_x: f64,
                    draw_mode_start_y: f64,
                    draw_mode_end_x: f64,
                    draw_mode_end_y: f64,
                    scene: &mut Scene,
                    operation_mode: &OperationModeType,
                    drag_started: bool,
                    edit_drag_cycle: &DragCycle,
    ) -> (f64, f64) {
        if let Ok(project) = self.project.lock() {
            let height = context.size().height;
            let width = context.size().width;
            let automation_type = self.automation_type;
            let note_expression_type = self.note_expression_type.clone();
            let note_expression_note_id = self.note_expression_note_id;
            let adjusted_beat_width_in_pixels = beat_width_in_pixels * zoom_horizontal;
            let adjusted_entity_height_in_pixels = entity_height_in_pixels * zoom_vertical;
            let type_to_show = &self.type_to_show;
            let current_view = &self.current_view;
            let automation_discrete = self.automation_discrete;
            let selected_effect_uuid = if let Some(selected_effect_uuid) = self.selected_effect_uuid.as_ref() {
                selected_effect_uuid.clone()
            }
            else {
                "".to_string()
            };

            if let AutomationViewMode::NoteVelocities = type_to_show {
                match self.selected_track_uuid.as_ref() {
                    Some(track_uuid) => match self.selected_riff_uuid.as_ref() {
                        Some(riff_uuid) => match project.song().tracks().iter().find(|track| track.uuid().to_string() == *track_uuid) {
                            Some(track) => {
                                // let (red, green, blue, _) = track.colour();
                                let red = 0.0;
                                let green = 0.0;
                                let blue = 0.0;
                                let name = track.name().to_string();

                                // draw the track name
                                self.draw_track_name(context, scene, name.as_str());

                                for riff in track.riffs().iter() {
                                    if riff.uuid().to_string() == *riff_uuid {
                                        self.draw_riff_name(context, scene, riff.name());

                                        let unselected_event_colour = if let Some((red, green, blue, _)) = riff.colour() {
                                            (*red, *green, *blue, 1.0)
                                        }
                                        else {
                                            (red, green, blue, 1.0)
                                        };

                                        for track_event in riff.events() {
                                            let mut event_colour = unselected_event_colour.clone();
                                            match track_event {
                                                TrackEvent::Note(note) => {
                                                    let note_number = note.note;
                                                    let note_velocity = note.velocity;
                                                    let note_velocity_y_pos_inverted = note_velocity as f64 * adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels;
                                                    let note_y_pos_inverted = note_number as f64 * adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels;
                                                    let x = note.position * adjusted_beat_width_in_pixels;
                                                    let y_velocity = height - note_velocity_y_pos_inverted;
                                                    let y_note = height - note_y_pos_inverted;
                                                    let note_width = note.length * adjusted_beat_width_in_pixels;

                                                    let is_selected = self.selected_automation.iter().any(|id| id.as_str() == note.id.uuid.to_string().as_str());
                                                    if is_selected {
                                                        event_colour = (0.0, 0.0, 1.0, 1.0);
                                                    }

                                                    let paint_colour =                                                             Color::from_rgba8(
                                                        (event_colour.0 * 255.0) as u8,
                                                        (event_colour.1 * 255.0) as u8,
                                                        (event_colour.2 * 255.0) as u8,
                                                        (event_colour.3 * 255.0) as u8
                                                    );


                                                    let mut path = BezPath::new();
                                                    path.move_to(Point{x, y: height});
                                                    path.line_to(Point {x, y: y_velocity + 5.0});
                                                    scene.stroke(
                                                        &Stroke::new(1.0),
                                                        Affine::IDENTITY,
                                                        paint_colour.clone(),
                                                        None,
                                                        &path,
                                                    );

                                                    // context.arc(x, y_velocity, 5.0, 0.0, 6.3 /* 2 * PI */);
                                                    // match context.fill() {
                                                    //     Ok(_) => (),
                                                    //     Err(error) => println!("Problem drawing note velocity circle in controller view: {:?}", error),
                                                    // }

                                                    let rect = Rect::new(x, y_note, x + note_width, y_note + adjusted_entity_height_in_pixels);
                                                    scene.fill(
                                                        Fill::NonZero,
                                                        Affine::IDENTITY,
                                                        paint_colour.clone(),
                                                        None,
                                                        &rect,
                                                    );
                                                },
                                                _ => (),
                                            }
                                        }
                                        break;
                                    }
                                }
                            },
                            None => (),
                        },
                        None => (),
                    },
                    None => (),
                }
            }
            else {
                if let Some(track_uuid) = self.selected_track_uuid.as_ref() {
                    if let Some(track) = project.song().tracks().iter().find(|track| track.uuid().to_string() == *track_uuid) {
                        // let (red, green, blue, _) = track.colour();
                        let red = 0.0;
                        let green = 0.0;
                        let blue = 0.0;
                        let name = track.name().to_string();

                        // draw the track name
                        self.draw_track_name(context, scene, name.as_str());

                        let events = match current_view {
                            CurrentView::Track => if let AutomationEditType::Track = self.automation_edit_type {
                                let automation = track.automation();
                                if automation_discrete {
                                    Some(automation.events())
                                }
                                else {
                                    if let Some(automation_type_value) = automation_type {
                                        if let Some(automation_envelope) = automation.envelopes().iter().find(|envelope| {
                                            let mut found = false;

                                            // need to know what kind of events we are looking for in order to get the appropriate envelope
                                            match type_to_show {
                                                AutomationViewMode::NoteVelocities => {

                                                }
                                                AutomationViewMode::Controllers => {
                                                    if let TrackEvent::Controller(controller) = envelope.event_details() {
                                                        if controller.controller == automation_type_value {
                                                            found = true;
                                                        }
                                                    }
                                                }
                                                AutomationViewMode::PitchBend => {
                                                    if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                                        found = true;
                                                    }
                                                }
                                                AutomationViewMode::Instrument => {
                                                    let plugin_uuid = if let TrackType::InstrumentTrack(instrument_track) = track {
                                                        instrument_track.instrument.uuid.to_string()
                                                    }
                                                    else {
                                                        "".to_string()
                                                    };
                                                    if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                        if param.index == automation_type_value && param.plugin_uuid.uuid.to_string() == plugin_uuid {
                                                            found = true;
                                                        }
                                                    }
                                                }
                                                AutomationViewMode::Effect => {
                                                    if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                        if param.index == automation_type_value && param.plugin_uuid.uuid.to_string() == selected_effect_uuid {
                                                            found = true;
                                                        }
                                                    }
                                                }
                                                AutomationViewMode::NoteExpression => {
                                                    if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                                        if *note_expression.expression_type() as i32 == automation_type_value {
                                                            found = true;
                                                        }
                                                    }
                                                }
                                            }
                                            return found;
                                        }) {
                                            Some(automation_envelope.events())
                                        } else { None }
                                    }
                                    else { None }
                                }
                            }
                            else {
                                None
                            }
                            CurrentView::RiffSet => if let AutomationEditType::Riff = self.automation_edit_type {
                                if let Some(selected_riff_uuid) = self.selected_riff_uuid.as_ref() {
                                    if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
                                        self.draw_riff_name(context, scene, riff.name());
                                        Some(riff.events_vec())
                                    }
                                    else { None }
                                }
                                else { None }
                            }
                            else {
                                None
                            }
                            CurrentView::RiffSequence => None,
                            CurrentView::RiffGrid => None,
                            CurrentView::RiffArrangement => if let CurrentView::RiffArrangement = current_view {
                                // get the arrangement
                                if let Some(selected_arrangement_uuid) = self.selected_riff_arrangement_uuid.as_ref() {
                                    if let Some(riff_arrangement) = project.song().riff_arrangement(selected_arrangement_uuid.clone()){
                                        if let Some(automation) = riff_arrangement.automation(&track_uuid) {
                                            if automation_discrete {
                                                Some(automation.events())
                                            }
                                            else {
                                                if let Some(automation_type_value) = automation_type {
                                                    if let Some(automation_envelope) = automation.envelopes().iter().find(|envelope| {
                                                        let mut found = false;

                                                        // need to know what kind of events we are looking for in order to get the appropriate envelope
                                                        match type_to_show {
                                                            AutomationViewMode::NoteVelocities => {

                                                            }
                                                            AutomationViewMode::Controllers => {
                                                                if let TrackEvent::Controller(controller) = envelope.event_details() {
                                                                    if controller.controller == automation_type_value {
                                                                        found = true;
                                                                    }
                                                                }
                                                            }
                                                            AutomationViewMode::PitchBend => {
                                                                if let TrackEvent::PitchBend(_) = envelope.event_details() {
                                                                    found = true;
                                                                }
                                                            }
                                                            AutomationViewMode::Instrument => {
                                                                let plugin_uuid = if let TrackType::InstrumentTrack(instrument_track) = track {
                                                                    instrument_track.instrument.uuid.to_string()
                                                                }
                                                                else {
                                                                    "".to_string()
                                                                };
                                                                if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                                    if param.index == automation_type_value && param.plugin_uuid.uuid.to_string() == plugin_uuid {
                                                                        found = true;
                                                                    }
                                                                }
                                                            }
                                                            AutomationViewMode::Effect => {
                                                                if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                                    if param.index == automation_type_value && param.plugin_uuid.uuid.to_string() == selected_effect_uuid {
                                                                        found = true;
                                                                    }
                                                                }
                                                            }
                                                            AutomationViewMode::NoteExpression => {
                                                                if let TrackEvent::NoteExpression(note_expression) = envelope.event_details() {
                                                                    if *note_expression.expression_type() as i32 == automation_type_value {
                                                                        found = true;
                                                                    }
                                                                }
                                                            }
                                                        }
                                                        return found;
                                                    }) {
                                                        Some(automation_envelope.events())
                                                    } else { None }
                                                }
                                                else { None }
                                            }
                                        }
                                        else { None }
                                    }
                                    else { None }
                                }
                                else { None }
                            }
                            else {
                                None
                            }
                        };

                        // draw the riff refs so we know where to add automation
                        if let CurrentView::Track = current_view {
                            let riff_refs = track.riff_refs();
                            self.draw_track(context, scene, height, adjusted_entity_height_in_pixels, beat_width_in_pixels, zoom_horizontal, adjusted_beat_width_in_pixels, track, riff_refs);
                        }
                        else if let CurrentView::RiffArrangement = current_view { // draw the riff arrangement track riff refs if relevant so that the user can see where to place their automation
                            // get the current riff arrangement
                            if let Some(selected_arrangement_uuid) = self.selected_riff_arrangement_uuid.as_ref() {
                                if let Some(riff_arrangement) = project.song().riff_arrangement(selected_arrangement_uuid.clone()){
                                    // loop through all the riff sets, riff seqs and riff grids fetching the riff refs using the LCF on riff sets to get the number of iterations for a riff ref for a track
                                    let mut running_position = 0.0;
                                    for item in riff_arrangement.items().iter() {
                                        // instantiate and pack new riffs to represent the riff ref and the underlying riff all packed into one.
                                        match item.item_type() {
                                            RiffItemType::RiffSet => {
                                                self.draw_riff_set_riff_refs(
                                                    context,
                                                    scene,
                                                    height,
                                                    entity_height_in_pixels,
                                                    beat_width_in_pixels,
                                                    zoom_horizontal,
                                                    adjusted_beat_width_in_pixels,
                                                    &track_uuid,
                                                    &track,
                                                    &mut running_position,
                                                    item.item_uuid().to_string());
                                            }
                                            RiffItemType::RiffSequence => {
                                                if let Some(riff_sequence) = project.song().riff_sequence(item.item_uuid().to_string()) {
                                                    for riff_set in riff_sequence.riff_sets().iter() {
                                                        self.draw_riff_set_riff_refs(
                                                            context,
                                                            scene,
                                                            height,
                                                            entity_height_in_pixels,
                                                            beat_width_in_pixels,
                                                            zoom_horizontal,
                                                            adjusted_beat_width_in_pixels,
                                                            &track_uuid,
                                                            &track,
                                                            &mut running_position,
                                                            riff_set.item_uuid().to_string());
                                                    }
                                                }
                                            }
                                            RiffItemType::RiffGrid => {
                                                if let Some(riff_grid) = project.song().riff_grid(item.item_uuid().to_string()) {
                                                    let mut riff_grid_length = 0.0;
                                                    for track_uuid in riff_grid.tracks() {
                                                        if let Some(track) = project.song().tracks().iter().find(|track| track.uuid().to_string() == track_uuid.to_string()) {
                                                            if let Some(riff_refs) = riff_grid.track_riff_references(track_uuid.clone()) {
                                                                for riff_ref in riff_refs.iter() {
                                                                    if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_ref.linked_to()) {
                                                                        let max_rightward_position = riff_ref.position + riff.length;
                                                                        if max_rightward_position > riff_grid_length {
                                                                            riff_grid_length = max_rightward_position;
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                    if let Some(riff_refs) = riff_grid.track_riff_references(track_uuid.clone()) {
                                                        for riff_ref in riff_refs.iter() {
                                                            // get the riff
                                                            if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_ref.linked_to()) {
                                                                let mut riff_to_draw = riff.clone();
                                                                riff_to_draw.set_position(running_position + riff_ref.position);
                                                                if riff_to_draw.name() != "empty" {
                                                                    self.draw_riff(context, scene, height, entity_height_in_pixels, beat_width_in_pixels, zoom_horizontal, adjusted_beat_width_in_pixels, &riff_to_draw, &track);
                                                                }
                                                            }
                                                        }
                                                    }
                                                    running_position += riff_grid_length;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        if let Some(events) = events {
                            let unselected_event_colour = (red, green, blue, 1.0);
                            let mut previous_point_x = 0.0;
                            let mut previous_point_y = height;
                            let default_line_width = 0.5;
                            let selected_automation  = events.iter().filter(|event| self.selected_automation.contains(&event.id())).map(|event| event.clone()).collect_vec();
                            let mut selected_automation_b4_after_points: HashMap<String, ((String, f64, f64), (String, f64, f64))>  = HashMap::new();
                            {
                                let mut previous_point = ("start".to_string(), 0.0, height);
                                let mut current_id = "".to_string();
                                let mut process_post_point = false;
                                for event in events.iter() {
                                    if process_post_point {
                                        if let Some((b4_point, _)) = selected_automation_b4_after_points.get(&current_id) {
                                            selected_automation_b4_after_points.insert(
                                                current_id.clone(),
                                                (
                                                    (b4_point.0.clone(), b4_point.1, b4_point.2),
                                                    (event.id(), event.position() * adjusted_beat_width_in_pixels, height - event.value() * 127.0 * adjusted_entity_height_in_pixels)
                                                )
                                            );
                                            process_post_point = false;
                                        }
                                    }
                                    if self.selected_automation.contains(&event.id()) {
                                        current_id = event.id();
                                        selected_automation_b4_after_points.insert(current_id.clone(), ((previous_point.0, previous_point.1, previous_point.2), ("".to_string(), 0.0, 0.0)));
                                        // println!("id={}, x_pre={}, y_pre={}", current_id.as_str(), previous_point.0, previous_point.1);
                                        process_post_point = true;
                                    }

                                    previous_point = (event.id(), event.position() * adjusted_beat_width_in_pixels, height - event.value() * 127.0 * adjusted_entity_height_in_pixels);
                                }

                                if process_post_point {
                                    if let Some((b4_point, _)) = selected_automation_b4_after_points.get(&current_id) {
                                        selected_automation_b4_after_points.insert(
                                            current_id.clone(),
                                            (
                                                (b4_point.0.clone(), b4_point.1, b4_point.2),
                                                ("end".to_string(), width - previous_point.1, previous_point.2)
                                            )
                                        );
                                        process_post_point = false;
                                    }
                                }
                            }
                            // println!("selected_automation_b4_after_points entry count={}", selected_automation_b4_after_points.iter().count());
                            // selected_automation_b4_after_points.iter().for_each(|(key, (pre, post))| println!("id={}, x_pre={}, y_pre={}, x_post={}, y_post={}", key, pre.0, pre.1, post.0, post.1));
                            for track_event in events.iter() {
                                let mut event_colour = unselected_event_colour.clone();
                                let is_selected = self.selected_automation.iter().any(|id| {
                                    id.as_str() == track_event.id().as_str()
                                });

                                // if is_selected {
                                //     event_colour = (0.0, 0.0, 1.0, 1.0);
                                // }
                                // context.set_source_rgba(event_colour.0, event_colour.1, event_colour.2, event_colour.3);

                                // self.edit_item_handler.handle_item_edit(
                                //     context,
                                //     track_event,
                                //     operation_mode,
                                //     mouse_pointer_x,
                                //     mouse_pointer_y,
                                //     mouse_pointer_previous_x,
                                //     mouse_pointer_previous_y,
                                //     adjusted_entity_height_in_pixels,
                                //     adjusted_beat_width_in_pixels,
                                //     track_event.position * adjusted_beat_width_in_pixels,
                                //     track_event.value * 127.0 * adjusted_entity_height_in_pixels,
                                //     height,
                                //     drawing_area,
                                //     edit_drag_cycle,
                                //     tx_from_ui.clone(),
                                //     true,
                                //     track_uuid.clone(),
                                //     track_event,
                                //     is_selected,
                                //     selected_automation.clone(),
                                //     previous_point_x,
                                //     previous_point_y,
                                //     automation_discrete,
                                //     &selected_automation_b4_after_points
                                // );

                                match type_to_show {
                                    AutomationViewMode::Controllers => {
                                        if let TrackEvent::Controller(controller) = track_event {
                                            if let Some(automation_type_value) = automation_type {
                                                if controller.controller == automation_type_value {
                                                    let controller_value = controller.value;
                                                    let note_y_pos_inverted = controller_value as f64 * adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels;
                                                    let x = controller.position * adjusted_beat_width_in_pixels;
                                                    let y = height - note_y_pos_inverted;

                                                    self.draw_automation(context, scene, height, automation_discrete, &mut previous_point_x, &mut previous_point_y, default_line_width, x, y, controller_value as f64);
                                                }
                                            }
                                        }
                                    }
                                    AutomationViewMode::PitchBend => {
                                        if let TrackEvent::PitchBend(pitch_bend) = track_event {
                                            let pitch_bend_value = pitch_bend.value;
                                            let note_y_pos_inverted = ((pitch_bend_value as f64 + 8192.0) / 16384.0 * 127.0) * adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels;
                                            let x = pitch_bend.position * adjusted_beat_width_in_pixels;
                                            let y = height - note_y_pos_inverted;
                                            let mut path = BezPath::new();

                                            if automation_discrete {
                                                path.move_to(Point {x, y: height / 2.0});
                                            } else {
                                                path.move_to(Point{x: previous_point_x, y: previous_point_y});
                                            }

                                            path.line_to(Point {x, y});
                                            scene.stroke(
                                                &Stroke::new(0.75),
                                                Affine::IDENTITY,
                                                Color::from_rgba8(0, 0, 0, 255),
                                                None,
                                                &path,
                                            );


                                            // context.set_line_width(default_line_width);

                                            if !automation_discrete {
                                                let rect = Rect::new(x - 5.0, y - 5.0, 10.0, 10.0);
                                                scene.fill(
                                                    Fill::NonZero,
                                                    Affine::IDENTITY,
                                                    Color::from_rgba8(255, 0, 0, 255),
                                                    None,
                                                    &rect,
                                                );
                                            }

                                            if !automation_discrete {
                                                previous_point_x = x;
                                                previous_point_y = y;
                                            }
                                        }
                                    }
                                    AutomationViewMode::Instrument => {
                                        if let TrackEvent::AudioPluginParameter(audio_plugin_parameter) = track_event {
                                            if let Some(automation_type_value) = automation_type {
                                                if audio_plugin_parameter.index == automation_type_value && audio_plugin_parameter.instrument {
                                                    let parameter_value = audio_plugin_parameter.value;
                                                    let note_y_pos_inverted = parameter_value as f64 * 127.0 *  adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels;
                                                    let x = audio_plugin_parameter.position * adjusted_beat_width_in_pixels;
                                                    let y = height - note_y_pos_inverted;

                                                    self.draw_automation(context, scene, height, automation_discrete, &mut previous_point_x, &mut previous_point_y, default_line_width, x, y, parameter_value as f64);
                                                }
                                            }
                                        }
                                    }
                                    AutomationViewMode::Effect => {
                                        if let TrackEvent::AudioPluginParameter(audio_plugin_parameter) = track_event {
                                            if let Some(selected_effect_plugin_uuid) = self.selected_effect_uuid.as_ref() {
                                                if let Some(automation_type_value) = automation_type {
                                                    if audio_plugin_parameter.index == automation_type_value &&
                                                        !audio_plugin_parameter.instrument &&
                                                        audio_plugin_parameter.plugin_uuid.uuid.to_string() == *selected_effect_plugin_uuid {
                                                        let parameter_value = audio_plugin_parameter.value;
                                                        let note_y_pos_inverted = parameter_value as f64 * 127.0 *  adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels;
                                                        let x = audio_plugin_parameter.position * adjusted_beat_width_in_pixels;
                                                        let y = height - note_y_pos_inverted;

                                                        self.draw_automation(context, scene, height, automation_discrete, &mut previous_point_x, &mut previous_point_y, default_line_width, x, y, parameter_value as f64);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    AutomationViewMode::NoteExpression => {
                                        if let TrackEvent::NoteExpression(note_expression) = track_event {
                                            if note_expression_type as i32 == *(note_expression.expression_type()) as i32 && (note_expression_note_id == -1 || note_expression_note_id == note_expression.note_id())  {
                                                let note_expression_value = note_expression.value;
                                                let note_y_pos_inverted = note_expression_value * 127.0 *  adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels;
                                                let x = note_expression.position * adjusted_beat_width_in_pixels;
                                                let y = height - note_y_pos_inverted;

                                                self.draw_automation(context, scene, height, automation_discrete, &mut previous_point_x, &mut previous_point_y, default_line_width, x, y, note_expression_value);
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            if !automation_discrete {
                                let mut path = BezPath::new();
                                path.move_to(Point {x: previous_point_x, y: previous_point_y});
                                path.line_to(Point {x: width, y: previous_point_y});
                                scene.stroke(
                                    &Stroke::new(0.75),
                                    Affine::IDENTITY,
                                    Color::from_rgba8(0, 0, 0, 255),
                                    None,
                                    &path,
                                );
                            }
                        }
                    }
                }
            }
        }

        if draw_mode_on {
            if let DrawMode::Line = draw_mode {
                self.draw_line(context, scene, draw_mode_start_x, draw_mode_start_y, draw_mode_end_x, draw_mode_end_y);
            }
        }

        (
            entity_height_in_pixels,
            beat_width_in_pixels,
        )
    }

    fn track_cursor_time_in_beats(&self) -> f64 {
        0.0
    }

    fn set_track_cursor_time_in_beats(&mut self, track_cursor_time_in_beats: f64) {
    }

    fn as_any(&mut self) -> &mut dyn Any {
        self
    }
}


// pub struct AutomationEditItemHandler {
//     pub original_item: Option<TrackEvent>,
//     pub original_item_is_selected: bool,
//     pub original_selected_items: Vec<TrackEvent>,
//     pub selected_item_ids: Vec<String>,
//     pub dragged_item: Option<TrackEvent>,
//     pub referenced_item: Option<TrackEvent>,
//     pub changed_event_sender: Box<dyn Fn(Vec<(TrackEvent, TrackEvent)>, String, crossbeam_channel::Sender<DAWEvents>)>,
//     pub copied_event_sender: Box<dyn Fn(Vec<TrackEvent>, String, crossbeam_channel::Sender<DAWEvents>)>,
//     pub can_change_position: bool,
//     pub can_drag_copy: bool,
// }
//
// impl AutomationEditItemHandler {
//     pub fn new(
//         changed_event_sender: Box<dyn Fn(Vec<(TrackEvent, TrackEvent)>, String, crossbeam_channel::Sender<DAWEvents>)>,
//         copied_event_sender: Box<dyn Fn(Vec<TrackEvent>, String, crossbeam_channel::Sender<DAWEvents>)>,
//         can_drag_copy: bool
//     ) -> Self {
//         Self {
//             original_item: None,
//             original_item_is_selected: false,
//             original_selected_items: vec![],
//             selected_item_ids: vec![],
//             dragged_item: None,
//             referenced_item: None,
//             changed_event_sender,
//             copied_event_sender,
//             can_change_position: true,
//             can_drag_copy,
//         }
//     }
// }
//
// impl AutomationEditItemHandler {
//     pub fn handle_item_edit(
//         &mut self,
//         context: &Context,
//         item: &TrackEvent,
//         operation_mode: &OperationModeType,
//         mouse_pointer_x: f64,
//         mouse_pointer_y: f64,
//         mouse_pointer_previous_x: f64,
//         mouse_pointer_previous_y: f64,
//         adjusted_entity_height_in_pixels: f64,
//         adjusted_beat_width_in_pixels: f64,
//         x_original: f64,
//         y_original: f64,
//         canvas_height: f64,
//         drawing_area: &DrawingArea,
//         edit_drag_cycle: &DragCycle,
//         tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
//         invert_vertically: bool,
//         track_uuid: String,
//         referencing_item: &TrackEvent,
//         item_is_selected: bool,
//         selected_items: Vec<TrackEvent>,
//         previous_point_x: f64,
//         previous_point_y: f64,
//         automation_discrete: bool,
//         selected_automation_b4_after_points: &HashMap<String, ((String, f64, f64), (String, f64, f64))>
//     ) {
//         let mut edit_mode = EditMode::Inactive;
//
//         match operation_mode {
//             OperationModeType::Change => {
//                 let mut x = x_original;
//                 let mut y = y_original;
//                 let mut found_item_being_changed = false;
//                 // calculate the mouse position deltas
//                 let delta_x = mouse_pointer_x - mouse_pointer_previous_x;
//                 let delta_y = mouse_pointer_y - mouse_pointer_previous_y;
//                 let mut use_this_item = false;
//
//                 if (item.position - referencing_item.position).abs() > 1e-10 {
//                     x = referencing_item.position * adjusted_beat_width_in_pixels;
//                 }
//
//                 if let DragCycle::NotStarted = edit_drag_cycle {
//                     self.original_item = None;
//                     self.original_selected_items.clear();
//                     self.selected_item_ids.clear();
//                     self.dragged_item = None;
//                 }
//
//                 // make sure original item matches the iterated item
//                 if let Some(original_item) = self.original_item.as_ref() {
//                     if (original_item.position - item.position).abs() < 1e-10 &&
//                         original_item.id == item.id &&
//                         (original_item.value - item.value).abs() < 1e-10 {
//                         if let Some(dragged_item) = self.dragged_item.as_ref() {
//                             if dragged_item.id == referencing_item.id {
//                                 found_item_being_changed = true;
//                             }
//                         }
//                     }
//                 }
//
//                 if found_item_being_changed {
//                     if let Some(dragged_item) = self.dragged_item.as_ref() {
//                         println!("Automation - Dragged item found.");
//                         let vertical_y_position = dragged_item.value * 127.0 * adjusted_entity_height_in_pixels;
//                         x = dragged_item.position * adjusted_beat_width_in_pixels + delta_x;
//
//                         if invert_vertically {
//                             y = canvas_height - vertical_y_position + delta_y - adjusted_entity_height_in_pixels;
//                         }
//                         else {
//                             y = vertical_y_position + delta_y;
//                         }
//                     }
//                 }
//
//                 // draw drag position adjust handle if required
//                 if found_item_being_changed || ((self.can_change_position || self.can_drag_copy) &&
//                     mouse_pointer_x >= (x - 5.0) &&
//                     mouse_pointer_x <= (x + 5.0) &&
//                     (canvas_height - mouse_pointer_y) >= (y - 5.0) &&
//                     (canvas_height - mouse_pointer_y) <= (y + 5.0)) {
//                     //change the mode
//                     edit_mode = EditMode::Move;
//                     use_this_item = true;
//
//                     // change the prompt
//                     if let Some(window) = drawing_area.window() {
//                         window.set_cursor(Some(&gdk::Cursor::for_display(&window.display(), gdk::CursorType::Hand1)));
//                         // println!("Automation - drawing hand prompt.");
//                     }
//                 }
//
//                 match edit_mode {
//                     EditMode::Inactive => {
//                         // println!("Automation - EditMode::Inactive");
//                     }
//                     _ => {
//                         match edit_drag_cycle {
//                             DragCycle::MousePressed => {
//                                 println!("Automation - handle_item_edit EditDragCycle::MousePressed");
//                                 if use_this_item {
//                                     println!("Automation - handle_item_edit EditDragCycle::MousePressed - set original and dragged items.");
//                                     self.original_item = Some(item.clone());
//                                     self.original_item_is_selected = item_is_selected;
//                                     self.original_selected_items = selected_items;
//                                     if item.id != referencing_item.id {
//                                         let mut dragged_item = item.clone();
//
//                                         dragged_item.set_id(referencing_item.id);
//                                         dragged_item.set_position(referencing_item.position);
//                                         dragged_item.set_value(referencing_item.value);
//                                         self.dragged_item = Some(dragged_item);
//                                     }
//                                     else {
//                                         self.dragged_item = Some(item.clone());
//                                     }
//                                 }
//                             }
//                             DragCycle::Dragging => {
//                                 println!("Automation - handle_item_edit EditDragCycle::Dragging");
//
//                                 if found_item_being_changed {
//                                     if let EditMode::Move =  edit_mode {
//                                         if let Some(dragged_item) = self.dragged_item.as_mut() {
//                                             let delta_x = x - dragged_item.position * adjusted_beat_width_in_pixels;
//                                             let delta_y = y - (canvas_height - dragged_item.value * 127.0 * adjusted_entity_height_in_pixels);
//                                             let mut updated_selected_items: HashMap<String, (f64, f64)> = HashMap::new();
//
//                                             // draw the dragged item
//                                             if automation_discrete {
//                                                 context.move_to(x, canvas_height);
//                                             }
//                                             else {
//                                                 if let Some((b4_point, after_point)) = selected_automation_b4_after_points.get(&dragged_item.id) {
//                                                     // need a look ahead check for another selected item that positionally comes before this one and use it's new position as the b4 point
//                                                     println!("dragged id={}, x_b4_point={}, b4_point_y={}", dragged_item.id, b4_point.0, b4_point.1);
//                                                     if self.original_item_is_selected {
//                                                         if let Some(b4_item) = self.original_selected_items.iter().find(|event| event.id == b4_point.0.clone()) {
//                                                             context.move_to(
//                                                                 b4_item.position * adjusted_beat_width_in_pixels + delta_x,
//                                                                 if invert_vertically {
//                                                                     canvas_height - b4_item.value * 127.0 * adjusted_entity_height_in_pixels + delta_y
//                                                                 }
//                                                                 else {
//                                                                     b4_item.value * 127.0 * adjusted_entity_height_in_pixels + delta_y
//                                                                 }
//                                                             );
//                                                         }
//                                                         else {
//                                                             context.move_to(b4_point.1, b4_point.2);
//                                                         }
//
//                                                         updated_selected_items.insert(dragged_item.id, (x, y));
//                                                     }
//                                                     else {
//                                                         context.move_to(b4_point.1, b4_point.2);
//                                                     }
//                                                 }
//                                                 else {
//                                                     context.move_to(previous_point_x, previous_point_y);
//                                                 }
//                                             }
//
//                                             context.line_to(x, y);
//                                             let _ = context.stroke();
//
//                                             if ! automation_discrete {
//                                                 context.rectangle(x - 5.0, y - 5.0, 10.0, 10.0);
//                                                 let _ = context.fill();
//                                             }
//
//                                             // if the after point does not come from a selected item then draw its line - the square denoting the position will be drawn by the normal code
//                                             if let Some((_, after_point)) = selected_automation_b4_after_points.get(&dragged_item.id) {
//                                                 if !self.original_selected_items.iter().any(|event| event.id == after_point.0.clone()) {
//                                                     context.move_to(x, y);
//                                                     context.line_to(after_point.1, after_point.2);
//                                                     let _ = context.stroke();
//                                                 }
//                                             }
//
//                                             // draw the other selected items
//                                             if self.original_item_is_selected {
//                                                 for item in self.original_selected_items.iter() {
//                                                     if item.id != dragged_item.id {
//                                                         let x = item.position * adjusted_beat_width_in_pixels + delta_x;
//                                                         let y = if invert_vertically {
//                                                             canvas_height - item.value * 127.0 * adjusted_entity_height_in_pixels + delta_y
//                                                         }
//                                                         else {
//                                                             item.value * 127.0 * adjusted_entity_height_in_pixels + delta_y
//                                                         };
//
//                                                         if automation_discrete {
//                                                             context.move_to(x, canvas_height);
//                                                         }
//                                                         else {
//                                                             if let Some((b4_point, after_point)) = selected_automation_b4_after_points.get(&item.id) {
//                                                                 println!("selected id={}, b4_point_id={}, b4_point_x={}, b4_point_y={}, after_point_id={}, after_point_x={}, after_point_y={}", item.id, b4_point.0, b4_point.1, b4_point.2, after_point.0, after_point.1, after_point.2);
//                                                                 if let Some((b4_updated_x, b4_updated_y)) = updated_selected_items.get(&b4_point.0.clone()) {
//                                                                     context.move_to(*b4_updated_x, *b4_updated_y);
//                                                                 }
//                                                                 else {
//                                                                     context.move_to(b4_point.1, b4_point.2);
//                                                                 }
//
//                                                                 updated_selected_items.insert(item.id, (x, y));
//
//                                                             }
//                                                             else {
//                                                                 context.move_to(previous_point_x, previous_point_y);
//                                                             }
//                                                         }
//
//                                                         context.line_to(x, y);
//                                                         let _ = context.stroke();
//
//                                                         if ! automation_discrete {
//                                                             context.rectangle(x - 5.0, y - 5.0, 10.0, 10.0);
//                                                             let _ = context.fill();
//                                                         }
//
//                                                         // if the after point does not come from a selected item then draw its line - the square denoting the position will be drawn by the normal code
//                                                         if let Some((_, after_point)) = selected_automation_b4_after_points.get(&item.id) {
//                                                             if !self.original_selected_items.iter().any(|event| event.id == after_point.0.clone()) {
//                                                                 context.move_to(x, y);
//                                                                 context.line_to(after_point.1, after_point.2);
//                                                                 let _ = context.stroke();
//                                                             }
//                                                         }
//                                                     }
//                                                 }
//                                                 println!("updated_selected_items count={}", updated_selected_items.iter().count());
//                                             }
//                                         }
//                                     }
//                                 }
//                             }
//                             DragCycle::MouseReleased => {
//                                 println!("Automation - handle_item_edit EditDragCycle::MouseReleased");
//
//                                 if found_item_being_changed {
//                                     if let Some(original_item) = self.original_item.as_ref() {
//                                         if let Some(dragged_item) = self.dragged_item.as_mut() {
//                                             if let EditMode::Move =  edit_mode {
//                                                 let mut change = vec![];
//                                                 // calculate and set the position
//                                                 let position_in_beats = x /adjusted_beat_width_in_pixels;
//                                                 dragged_item.set_position(position_in_beats);
//
//                                                 // calculate and set the value
//                                                 let mut value = if invert_vertically {
//                                                     let y_pos_inverted = canvas_height - y;
//                                                     ((y_pos_inverted - adjusted_entity_height_in_pixels) / adjusted_entity_height_in_pixels) / 127.0
//                                                 }
//                                                 else {
//                                                     y / 127.0
//                                                 };
//
//                                                 if value < 0.0 {
//                                                     value = 0.0;
//                                                 }
//
//                                                 println!("Automation - Setting dragged item value to: {}", value);
//                                                 dragged_item.set_value(value);
//
//                                                 change.push((original_item.clone(), dragged_item.clone()));
//
//                                                 // handle the other selected items
//                                                 if self.original_item_is_selected {
//                                                     let delta_x = position_in_beats - original_item.position;
//                                                     let delta_y = value - original_item.value;
//                                                     for item in self.original_selected_items.iter() {
//                                                         if item.id != dragged_item.id {
//                                                             let mut changed_item = item.clone();
//                                                             let mut changed_item_value = changed_item.value + delta_y;
//
//                                                             if changed_item_value < 0.0 {
//                                                                 changed_item_value = 0.0;
//                                                             }
//
//                                                             changed_item.set_position(changed_item.position + delta_x);
//                                                             changed_item.set_value(changed_item.value + delta_y);
//                                                             change.push((item.clone(), changed_item));
//                                                         }
//                                                     }
//                                                 }
//
//                                                 if !change.is_empty() {
//                                                     (self.changed_event_sender)(change, track_uuid.clone(), tx_from_ui.clone());
//                                                 }
//                                             }
//                                         }
//                                     }
//
//                                     println!("Automation - handle_item_edit EditDragCycle::MouseReleased - unset original and dragged items.");
//                                     self.original_item = None;
//                                     self.original_selected_items.clear();
//                                     self.selected_item_ids.clear();
//                                     self.dragged_item = None;
//                                 }
//                             }
//                             DragCycle::CtrlMousePressed => {
//                                 println!("Automation - handle_item_edit EditDragCycle::CtrlMousePressed");
//                                 if use_this_item {
//                                     println!("Automation - handle_item_edit EditDragCycle::CtrlMousePressed - set original and dragged items.");
//                                     self.original_item = Some(item.clone());
//                                     self.original_item_is_selected = item_is_selected;
//                                     self.original_selected_items = selected_items;
//                                     if item.id != referencing_item.id {
//                                         let mut dragged_item = item.clone();
//
//                                         dragged_item.set_id(referencing_item.id);
//                                         dragged_item.set_position(referencing_item.position);
//                                         dragged_item.set_value(referencing_item.value);
//                                         self.dragged_item = Some(dragged_item);
//                                     }
//                                     else {
//                                         self.dragged_item = Some(item.clone());
//                                     }
//                                 }
//                             }
//                             DragCycle::CtrlDragging => {
//                                 println!("Automation - handle_item_edit EditDragCycle::CtrlDragging");
//
//                                 if found_item_being_changed {
//                                     if let EditMode::Move = edit_mode {
//                                         if let Some(dragged_item) = self.dragged_item.as_mut() {
//                                             // draw the dragged item
//                                             if automation_discrete {
//                                                 context.move_to(x, canvas_height);
//                                             }
//                                             else {
//                                                 context.move_to(previous_point_x, previous_point_y);
//                                             }
//                                             context.line_to(x, y);
//                                             let _ = context.stroke();
//
//                                             if ! automation_discrete {
//                                                 context.rectangle(x - 5.0, y - 5.0, 10.0, 10.0);
//                                                 let _ = context.fill();
//                                             }
//
//                                             // draw the other selected items
//                                             if self.original_item_is_selected {
//                                                 let delta_x = x - dragged_item.position * adjusted_beat_width_in_pixels;
//                                                 let delta_y = y - (canvas_height - dragged_item.value * 127.0 * adjusted_entity_height_in_pixels);
//                                                 for item in self.original_selected_items.iter() {
//                                                     if item.id != dragged_item.id {
//                                                         let x = item.position * adjusted_beat_width_in_pixels + delta_x;
//                                                         let y = if invert_vertically {
//                                                             canvas_height - item.value * 127.0 * adjusted_entity_height_in_pixels + delta_y
//                                                         }
//                                                         else {
//                                                             item.value * 127.0 * adjusted_entity_height_in_pixels + delta_y
//                                                         };
//
//                                                         if automation_discrete {
//                                                             context.move_to(x, canvas_height);
//                                                         }
//                                                         else {
//                                                             context.move_to(previous_point_x, previous_point_y);
//                                                         }
//
//                                                         context.line_to(x, y);
//                                                         let _ = context.stroke();
//
//                                                         if ! automation_discrete {
//                                                             context.rectangle(x - 5.0, y - 5.0, 10.0, 10.0);
//                                                             let _ = context.fill();
//                                                         }
//                                                     }
//                                                 }
//                                             }
//                                         }
//                                     }
//                                 }
//                             }
//                             DragCycle::CtrlMouseReleased => {
//                                 println!("Automation - handle_item_edit EditDragCycle::CtrlMouseReleased");
//
//                                 if found_item_being_changed {
//                                     if let Some(original_item) = self.original_item.as_ref() {
//                                         if let Some(dragged_item) = self.dragged_item.as_mut() {
//                                             if let EditMode::Move = edit_mode {
//                                                 let mut copied = vec![];
//                                                 // calculate and set the position
//                                                 let position_in_beats = x /adjusted_beat_width_in_pixels;
//                                                 dragged_item.set_position(position_in_beats);
//
//                                                 // calculate and set the vertical index
//                                                 let value = if invert_vertically {
//                                                     let y_pos_inverted = canvas_height - y;
//                                                     ((y_pos_inverted - adjusted_entity_height_in_pixels) / adjusted_entity_height_in_pixels) / 127.0
//                                                 }
//                                                 else {
//                                                     y / 127.0
//                                                 };
//
//                                                 println!("Automation - Setting dragged item vertical index to: {}", value);
//                                                 dragged_item.set_value(value * 127.0);
//
//                                                 copied.push(dragged_item.clone());
//
//                                                 // handle the other selected items
//                                                 if self.original_item_is_selected {
//                                                     let delta_x = position_in_beats - original_item.position;
//                                                     let delta_y = value - original_item.value;
//                                                     for item in self.original_selected_items.iter() {
//                                                         if item.id != dragged_item.id {
//                                                             let mut copied_item = item.clone();
//
//                                                             copied_item.set_position(copied_item.position + delta_x);
//                                                             copied_item.set_value((copied_item.value + delta_y) * 127.0);
//                                                             copied.push(copied_item);
//                                                         }
//                                                     }
//                                                 }
//
//                                                 if !copied.is_empty() {
//                                                     (self.copied_event_sender)(copied, track_uuid.clone(), tx_from_ui.clone());
//                                                 }
//                                             }
//                                         }
//                                     }
//
//                                     println!("Automation - handle_item_edit EditDragCycle::CtrlMouseReleased - unset original and dragged items.");
//                                     self.original_item = None;
//                                     self.original_selected_items.clear();
//                                     self.selected_item_ids.clear();
//                                     self.dragged_item = None;
//                                 }
//                             }
//                             DragCycle::NotStarted => {
//                                 println!("Automation - handle_item_edit EditDragCycle::NotStarted");
//                             }
//                         }
//                     }
//                 }
//             }
//             _ => {
//             }
//         }
//     }
// }

pub struct RiffSetTrackCustomPainter {
    pub project: Arc<Mutex<Project>>,
    pub track_cursor_time_in_beats: f64,
    pub track_uuid: String,
    pub riff_set_uuid: String,
}

impl RiffSetTrackCustomPainter {
    pub fn new(
        state: Arc<Mutex<Project>>,
        track_uuid: String,
        riff_set_uuid: String,
    ) -> RiffSetTrackCustomPainter {
        RiffSetTrackCustomPainter {
            project: state,
            track_cursor_time_in_beats: 0.0,
            track_uuid,
            riff_set_uuid,
        }
    }
}

impl CustomPainter for RiffSetTrackCustomPainter {
    fn paint_custom(&mut self,
                    context: &mut PaintCtx<'_>,
                    entity_height_in_pixels: f64,
                    beat_width_in_pixels: f64,
                    zoom_horizontal: f64,
                    zoom_vertical: f64,
                    drawing_area_widget_name: Option<String>,
                    mouse_pointer_x: f64,
                    mouse_pointer_y: f64,
                    mouse_pointer_previous_x: f64,
                    mouse_pointer_previous_y: f64,
                    draw_mode_on: bool,
                    draw_mode: DrawMode,
                    draw_mode_start_x: f64,
                    draw_mode_start_y: f64,
                    draw_mode_end_x: f64,
                    draw_mode_end_y: f64,
                    scene: &mut Scene,
                    operation_mode: &OperationModeType,
                    drag_started: bool,
                    edit_drag_cycle: &DragCycle,
    ) -> (f64, f64) {
        // debug!("RiffSetTrackCustomPainter::paint_custom - entered");
        let width = context.size().width;
        let height = context.size().height;

        match self.project.lock() {
            Ok(project) => {
                let mut adjusted_beat_width_in_pixels = beat_width_in_pixels * zoom_horizontal;
                let _adjusted_entity_height_in_pixels = entity_height_in_pixels * zoom_vertical;

                            let riff_set_uuid = self.riff_set_uuid.clone();
                            let riff_set_number = if let Some((position, _)) = project.song.riff_sets().iter().find_position(|riff_set| riff_set.uuid == riff_set_uuid.to_string()) {
                                position as i32 + 1
                            }
                            else {
                                1
                            };
                            let track_uuid = self.track_uuid.clone();
                            let (riff_ref_linked_to, mode) = {
                                if let Some(riff_set) = project.song.riff_sets().iter().find(|riff_set| riff_set.uuid == riff_set_uuid.to_string()) {
                                    if let Some(riff_ref) = riff_set.riff_refs.get(&track_uuid.clone()) {
                                        (riff_ref.linked_to.clone(), riff_ref.mode.clone())
                                    }
                                    else {
                                        ("".to_string(), RiffReferenceMode::Normal)
                                    }
                                }
                                else {
                                    ("".to_string(), RiffReferenceMode::Normal)
                                }
                            };

                            let number_of_beats_in_bar = project.song.time_signature_denominator();
                            let mut track = project.song.tracks().iter().find(|track| track.uuid() == track_uuid);

                            // get the track
                            match track {
                                Some(track) => {
                                    let track_colour = track.colour();

                                    // get the riff
                                    if let Some(riff) = track.riffs().iter().find(|current_riff| current_riff.uuid.uuid.to_string() == riff_ref_linked_to) {
                                        let colour = if riff.name != "empty" {
                                            // also zoom out to fit the entire riff
                                            let riff_width_in_pixels = riff.length * adjusted_beat_width_in_pixels;
                                            if riff_width_in_pixels > width {
                                                let zoom_factor = riff_width_in_pixels / width;
                                                adjusted_beat_width_in_pixels = beat_width_in_pixels * zoom_horizontal / zoom_factor;
                                            }

                                            if let Some((red, green, blue, alpha)) = riff.colour {
                                                Color::from_rgba8((red * 255.0) as u8, (green * 255.0) as u8, (blue * 255.0) as u8, (alpha * 255.0) as u8)
                                            }
                                            else {
                                                let (red, green, blue, alpha) = track_colour;
                                                Color::from_rgba8((red * 255.0) as u8, (green * 255.0) as u8, (blue * 255.0) as u8, (alpha * 255.0) as u8)
                                            }
                                        }
                                        else {
                                            Color::from_rgba8((0.5 * 255.0) as u8, (0.5 * 255.0) as u8, (0.5 * 255.0) as u8, (1.0 * 255.0) as u8)
                                        };

                                        let rect = Rect::new(0.0, 0.0, width, height);
                                        scene.fill(
                                            Fill::NonZero,
                                            Affine::IDENTITY,
                                            colour,
                                            None,
                                            &rect,
                                        );


                                        let mut use_note = match mode {
                                            RiffReferenceMode::Normal => true,
                                            RiffReferenceMode::Start => false,
                                            RiffReferenceMode::End => true,
                                        };

                                        for track_event in riff.events.iter() {

                                            match track_event {
                                                TrackEvent::Note(note) => {
                                                    use_note = match &mode {
                                                        RiffReferenceMode::Start => {
                                                            if !use_note && note.riff_start_note { true }
                                                            else if use_note { true }
                                                            else { false }
                                                        }
                                                        RiffReferenceMode::End => {
                                                            if use_note && note.riff_start_note { false }
                                                            else if !use_note { false }
                                                            else { true }
                                                        }
                                                        RiffReferenceMode::Normal => true,
                                                    };

                                                    if use_note {
                                                        let note_number = note.note;
                                                        let note_y_pos_inverted = note_number as f64 * entity_height_in_pixels + entity_height_in_pixels;
                                                        // let duration_in_beats = note.duration() * adjusted_beat_width_in_pixels;
                                                        let x = note.position * adjusted_beat_width_in_pixels;
                                                        let y = height - note_y_pos_inverted;
                                                        let width = note.length * adjusted_beat_width_in_pixels;
                                                        let rect = Rect::new(x, y, x + width, y + entity_height_in_pixels);
                                                        scene.fill(
                                                            Fill::NonZero,
                                                            Affine::IDENTITY,
                                                            Color::BLACK,
                                                            None,
                                                            &rect,
                                                        );
                                                    }
                                                },
                                                // TrackEvent::Sample(sample) => {
                                                //     context.set_source_rgba(1.0, 0.0, 0.0, 1.0);
                                                //     let sample_y_pos = 0.0;
                                                //     let x = sample.position() * adjusted_beat_width_in_pixels;
                                                //     let width = 1.0 * adjusted_beat_width_in_pixels;
                                                //     context.rectangle(x, sample_y_pos, width, height);
                                                //     let _ = context.fill();
                                                // }
                                                _ => (),
                                            }
                                        }

                                        // draw the riff name
                                        // context.set_source_rgba(0.0, 0.0, 0.0, 1.0);
                                        // context.move_to(1.0, 15.0);
                                        // context.set_font_size(9.0);
                                        let text_colour = Color::from_rgba8(0, 0, 0, 255);
                                        if riff.name == "empty" {
                                            self.draw_text(context, scene, 1.0, 7.0, "e", text_colour, 9.0, FontWeight::EXTRA_BOLD);
                                        }
                                        else {
                                            self.draw_text(context, scene, 1.0, 7.0, riff.name.as_str(), text_colour, 9.0, FontWeight::EXTRA_BOLD);

                                            // draw the track cursor
                                            let x = (self.track_cursor_time_in_beats as i32 % riff.length as i32) as f64 * adjusted_beat_width_in_pixels;
                                            let mut path = BezPath::new();
                                            path.move_to(Point { x, y: 0.0 });
                                            path.line_to(Point { x, y: height });
                                            scene.stroke(
                                                &Stroke::new(1.0),
                                                Affine::IDENTITY,
                                                text_colour,
                                                None,
                                                &path,
                                            );


                                            // draw the riff length
                                            self.draw_text(context, scene, width - 20.0, height - 9.0, format!("{}", riff.length / number_of_beats_in_bar).as_str(), text_colour, 9.0, FontWeight::EXTRA_BOLD);

                                            // draw the riff set number
                                            self.draw_text(context, scene, width / 2.0 - 18.0, 5.0, format!("{}", riff_set_number).as_str(), Color::from_rgba8(0, 0, 0, 25), 36.0, FontWeight::EXTRA_BOLD);

                                            // draw the riff reference play mode
                                            if let RiffReferenceMode::Start = mode {
                                                self.draw_text(context, scene, 1.0, height - 1.0, "start", text_colour, 9.0, FontWeight::EXTRA_BOLD);
                                            }
                                            else if let RiffReferenceMode::End = mode {
                                                self.draw_text(context, scene, 1.0, height - 1.0, "end", text_colour, 9.0, FontWeight::EXTRA_BOLD);
                                            }
                                        }

                                    }
                                },
                                None => (),
                            }
            }
            Err(_) => println!("Riff set track custom painter could not get state lock."),
        }

        // debug!("RiffSetTrackCustomPainter::paint_custom - entered");

        (
            entity_height_in_pixels,
            beat_width_in_pixels,
        )
    }

    fn track_cursor_time_in_beats(&self) -> f64 {
        self.track_cursor_time_in_beats
    }
    fn set_track_cursor_time_in_beats(&mut self, track_cursor_time_in_beats: f64) {
        self.track_cursor_time_in_beats = track_cursor_time_in_beats;
    }

    fn as_any(&mut self) -> &mut dyn Any {
        self
    }
}


pub struct RiffArrangementOverviewMouseCoordHelper;

impl BeatGridMouseCoordHelper for RiffArrangementOverviewMouseCoordHelper {
    type Action = DAWEvents;

    fn get_entity_vertical_value(&self, y: f64, entity_height_in_pixels: f64, zoom_vertical: f64) -> f64 {
        0.0
    }

    fn select_single(&self, cx: &mut EventCtx, x: f64, y: i32, add_to_select: bool) {
    }

    fn select_multiple(&self, cx: &mut EventCtx, x: f64, y: i32, x2: f64, y2: i32, add_to_select: bool) {
    }

    fn deselect_single(&self, cx: &mut EventCtx, x: f64, y: i32) {
    }

    fn deselect_multiple(&self, cx: &mut EventCtx, x: f64, y: i32, x2: f64, y2: i32) {
    }

    fn add_entity(&self, cx: &mut EventCtx, y_index: i32, time: f64, _duration: f64, _entity_uuid: String) {
    }

    fn add_entity_extra(&self, cx: &mut EventCtx, y_index: i32, time: f64, duration: f64, entity_uuid: String) {
    }

    fn delete_entity(&self, cx: &mut EventCtx, _y_index: i32, time: f64, _entity_uuid: String) {
    }

    fn cut_selected(&self, cx: &mut EventCtx) {
    }

    fn copy_selected(&self, cx: &mut EventCtx) {
    }

    fn paste_selected(&self, cx: &mut EventCtx) {
    }

    fn handle_translate_up(&self, cx: &mut EventCtx) {
    }

    fn handle_translate_down(&self, cx: &mut EventCtx) {
    }

    fn handle_translate_left(&self, cx: &mut EventCtx) {
    }

    fn handle_translate_right(&self, cx: &mut EventCtx) {
    }

    fn handle_quantise(&self, cx: &mut EventCtx) {
    }

    fn handle_increase_entity_length(&self, cx: &mut EventCtx) {
    }

    fn handle_decrease_entity_length(&self, cx: &mut EventCtx) {
    }

    fn set_start_note(&self, cx: &mut EventCtx, y_index: i32, time: f64) {
    }

    fn set_riff_reference_play_mode(&self, cx: &mut EventCtx, y_index: i32, time: f64) {
    }

    fn handle_windowed_zoom(&self, cx: &mut EventCtx, x1: f64, y1: f64, x2: f64, y2: f64) {
    }

    fn cycle_entity_selection(&self, cx: &mut EventCtx, y_index: i32, time: f64) {
    }

    fn select_underlying_entity(&self, cx: &mut EventCtx, y_index: i32, time: f64) {
    }
}


pub struct RiffGridMouseCoordHelper;

impl BeatGridMouseCoordHelper for RiffGridMouseCoordHelper {
    type Action = DAWEvents;

    fn get_entity_vertical_value(&self, y: f64, entity_height_in_pixels: f64, zoom_vertical: f64) -> f64 {
        y / entity_height_in_pixels * zoom_vertical
    }

    fn select_single(&self, cx: &mut EventCtx, x: f64, y: i32, add_to_select: bool) {
        cx.submit_action::<Self::Action>(DAWEvents::RiffGridChange(RiffGridChangeType::RiffReferencesSelectSingle(x, y, add_to_select), None));
    }

    fn select_multiple(&self, cx: &mut EventCtx, x: f64, y: i32, x2: f64, y2: i32, add_to_select: bool) {
        cx.submit_action::<Self::Action>(DAWEvents::RiffGridChange(RiffGridChangeType::RiffReferencesSelectMultiple(x, y, x2, y2, add_to_select), None));
    }

    fn deselect_single(&self, cx: &mut EventCtx, x: f64, y: i32) {
        cx.submit_action::<Self::Action>(DAWEvents::RiffGridChange(RiffGridChangeType::RiffReferencesDeselectSingle(x, y), None));
    }

    fn deselect_multiple(&self, cx: &mut EventCtx, x: f64, y: i32, x2: f64, y2: i32) {
        cx.submit_action::<Self::Action>(DAWEvents::RiffGridChange(RiffGridChangeType::RiffReferencesDeselectMultiple(x, y, x2, y2), None));
    }

    fn add_entity(&self, cx: &mut EventCtx, y_index: i32, time: f64, _duration: f64, _entity_uuid: String) {
        cx.submit_action::<Self::Action>(DAWEvents::RiffGridChange(RiffGridChangeType::RiffReferenceAdd{ track_index: y_index, position: time }, None));
    }

    fn add_entity_extra(&self, cx: &mut EventCtx, y_index: i32, time: f64, duration: f64, entity_uuid: String) {
    }

    fn delete_entity(&self, cx: &mut EventCtx, y_index: i32, time: f64, _entity_uuid: String) {
        cx.submit_action::<Self::Action>(DAWEvents::RiffGridChange(RiffGridChangeType::RiffReferenceDelete{ track_index: y_index, position: time }, None));
    }

    fn cut_selected(&self, cx: &mut EventCtx) {
        cx.submit_action::<Self::Action>(DAWEvents::RiffGridChange(RiffGridChangeType::RiffReferenceCutSelected, None));
    }

    fn copy_selected(&self, cx: &mut EventCtx) {
        cx.submit_action::<Self::Action>(DAWEvents::RiffGridChange(RiffGridChangeType::RiffReferenceCopySelected, None));
    }

    fn paste_selected(&self, cx: &mut EventCtx) {
        cx.submit_action::<Self::Action>(DAWEvents::RiffGridChange(RiffGridChangeType::RiffReferencePaste, None));
    }

    fn handle_translate_up(&self, cx: &mut EventCtx) {

    }

    fn handle_translate_down(&self, cx: &mut EventCtx) {

    }

    fn handle_translate_left(&self, cx: &mut EventCtx) {

    }

    fn handle_translate_right(&self, cx: &mut EventCtx) {

    }

    fn handle_quantise(&self, cx: &mut EventCtx) {

    }

    fn handle_increase_entity_length(&self, cx: &mut EventCtx) {
    }

    fn handle_decrease_entity_length(&self, cx: &mut EventCtx) {
    }

    fn set_start_note(&self, cx: &mut EventCtx, y_index: i32, time: f64) {
    }

    fn set_riff_reference_play_mode(&self, cx: &mut EventCtx, y_index: i32, time: f64) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffReferencePlayMode(y_index, time), None));
    }

    fn handle_windowed_zoom(&self, cx: &mut EventCtx, x1: f64, y1: f64, x2: f64, y2: f64) {
    }

    fn cycle_entity_selection(&self, cx: &mut EventCtx, y_index: i32, time: f64) {
        cx.submit_action::<Self::Action>(DAWEvents::RiffGridChange(RiffGridChangeType::RiffReferenceIncrementRiff{track_index: y_index, position: time}, None));
    }

    fn select_underlying_entity(&self, cx: &mut EventCtx, y_index: i32, time: f64) {
        cx.submit_action::<Self::Action>(DAWEvents::RiffGridChange(RiffGridChangeType::RiffSelectWithTrackIndex{track_index: y_index, position: time}, None));
    }
}


pub struct SampleRollMouseCoordHelper;

impl BeatGridMouseCoordHelper for SampleRollMouseCoordHelper {
    type Action = DAWEvents;

    fn get_entity_vertical_value(&self, y: f64, entity_height_in_pixels: f64, zoom_vertical: f64) -> f64 {
        ((127.0 * entity_height_in_pixels * zoom_vertical) - y) / (entity_height_in_pixels * zoom_vertical)
    }

    fn select_single(&self, cx: &mut EventCtx, x: f64, y: i32, add_to_select: bool) {
    }

    fn select_multiple(&self, cx: &mut EventCtx, x: f64, y: i32, x2: f64, y2: i32, add_to_select: bool) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::AutomationSelectMultiple(x, y2, x2, y, add_to_select), None));
    }

    fn deselect_single(&self, cx: &mut EventCtx, x: f64, y: i32) {
        todo!()
    }

    fn deselect_multiple(&self, cx: &mut EventCtx, x: f64, y: i32, x2: f64, y2: i32) {
    }

    fn add_entity(&self, cx: &mut EventCtx, _y_index: i32, time: f64, _duration: f64, entity_uuid: String) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffAddSample(entity_uuid, time), None));
    }

    fn add_entity_extra(&self, cx: &mut EventCtx, y_index: i32, time: f64, duration: f64, entity_uuid: String) {
    }

    fn delete_entity(&self, cx: &mut EventCtx, _y_index: i32, time: f64, entity_uuid: String) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffDeleteSample(entity_uuid, time), None));
    }

    fn cut_selected(&self, cx: &mut EventCtx) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffCutSelected, None));
    }

    fn copy_selected(&self, cx: &mut EventCtx) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffCopySelected, None));
    }

    fn paste_selected(&self, cx: &mut EventCtx) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffPasteSelected, None));
    }

    fn handle_translate_up(&self, cx: &mut EventCtx) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffTranslateSelected(TranslationEntityType::Note, TranslateDirection::Up), None));
    }

    fn handle_translate_down(&self, cx: &mut EventCtx) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffTranslateSelected(TranslationEntityType::Note, TranslateDirection::Down), None));
    }

    fn handle_translate_left(&self, cx: &mut EventCtx) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffTranslateSelected(TranslationEntityType::Note, TranslateDirection::Left), None));
    }

    fn handle_translate_right(&self, cx: &mut EventCtx) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffTranslateSelected(TranslationEntityType::Note, TranslateDirection::Right), None));
    }

    fn handle_quantise(&self, cx: &mut EventCtx) {
        cx.submit_action::<Self::Action>(DAWEvents::TrackChange(TrackChangeType::RiffQuantiseSelected, None));
    }

    fn handle_increase_entity_length(&self, cx: &mut EventCtx) {

    }

    fn handle_decrease_entity_length(&self, cx: &mut EventCtx) {

    }

    fn set_start_note(&self, cx: &mut EventCtx, y_index: i32, time: f64) {
    }

    fn set_riff_reference_play_mode(&self, cx: &mut EventCtx, y_index: i32, time: f64) {
    }

    fn handle_windowed_zoom(&self, cx: &mut EventCtx, x1: f64, y1: f64, x2: f64, y2: f64) {
    }

    fn cycle_entity_selection(&self, cx: &mut EventCtx, y_index: i32, time: f64) {
    }

    fn select_underlying_entity(&self, cx: &mut EventCtx, y_index: i32, time: f64) {
    }
}



pub struct TrackGridCustomPainter {
    pub project: Arc<Mutex<Project>>,
    pub show_automation: bool,
    pub show_note: bool,
    pub show_note_velocity: bool,
    pub show_pan: bool,
    pub looping: bool,
    pub active_loop: Option<String>,
    pub selected_track_grid_riff_references: Vec<String>,
    // pub edit_item_handler: EditItemHandler<Riff, RiffReference>,
}

impl TrackGridCustomPainter {
    pub fn new_with_edit_item_handler(project: Arc<Mutex<Project>>, /* edit_item_handler: EditItemHandler<Riff, RiffReference>*/) -> TrackGridCustomPainter {
        TrackGridCustomPainter {
            project,
            show_automation: false,
            show_note: true,
            show_note_velocity: false,
            show_pan: false,
            looping: false,
            active_loop: None,
            // edit_item_handler,
            selected_track_grid_riff_references: vec![],
        }
    }
    pub fn set_show_automation(&mut self, show_automation: bool) {
        self.show_automation = show_automation;
    }
    pub fn set_show_note(&mut self, show_note: bool) {
        self.show_note = show_note;
    }
    pub fn set_show_note_velocity(&mut self, show_note_velocity: bool) {
        self.show_note_velocity = show_note_velocity;
    }
    pub fn set_show_pan(&mut self, show_pan: bool) {
        self.show_pan = show_pan;
    }
}

impl CustomPainter for TrackGridCustomPainter {
    fn paint_custom(&mut self,
                    context: &mut PaintCtx<'_>,
                    entity_height_in_pixels: f64,
                    beat_width_in_pixels: f64,
                    zoom_horizontal: f64,
                    zoom_vertical: f64,
                    drawing_area_widget_name: Option<String>,
                    mouse_pointer_x: f64,
                    mouse_pointer_y: f64,
                    mouse_pointer_previous_x: f64,
                    mouse_pointer_previous_y: f64,
                    draw_mode_on: bool,
                    draw_mode: DrawMode,
                    draw_mode_start_x: f64,
                    draw_mode_start_y: f64,
                    draw_mode_end_x: f64,
                    draw_mode_end_y: f64,
                    scene: &mut Scene,
                    operation_mode: &OperationModeType,
                    drag_started: bool,
                    edit_drag_cycle: &DragCycle) -> (f64, f64) {
        let clip_rectangle = context.size().to_rect();
        let clip_x1 = clip_rectangle.x0;
        let clip_y1 = clip_rectangle.y0;
        let clip_x2 = clip_rectangle.x1;
        let clip_y2 = clip_rectangle.y1;
        // println!("TrackGridCustomPainter::paint_custom - entered...: clip_x1={}, clip_y1={}, clip_x2={}, clip_y2={},", clip_x1, clip_y1, clip_x2, clip_y2);

        match self.project.lock() {
            Ok(mut project) => {
                let adjusted_beat_width_in_pixels = beat_width_in_pixels * zoom_horizontal;
                let adjusted_entity_height_in_pixels = entity_height_in_pixels * zoom_vertical;

                // find all the selected notes
                let selected_riff_ref_ids = self.selected_track_grid_riff_references.clone();
                let mut selected_riff_references: Vec<Riff> = vec![];

                for (index, track) in project.song.tracks_mut().iter_mut().enumerate() {
                    for riff_reference in track.riff_refs().iter().filter(|riff_ref| selected_riff_ref_ids.clone().contains(&riff_ref.uuid.to_string())) {
                        if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid.uuid.to_string() == riff_reference.linked_to) {
                            let mut riff = riff.clone();
                            riff.uuid = UuidWrapper::new_from_string(riff_reference.uuid.clone());
                            riff.position = riff_reference.position;
                            riff.vertical_index = index as i32;
                            selected_riff_references.push(riff);
                        }
                    }
                }

                for (track_number, track) in project.song.tracks_mut().iter_mut().enumerate() {
                    let (red, green, blue, alpha) = track.colour();

                    for riff_ref in track.riff_refs().iter() {
                        let linked_to_riff_uuid = riff_ref.linked_to.clone();

                        let is_selected = selected_riff_ref_ids.iter().any(|id| *id == riff_ref.uuid.to_string());
                        let mut colour = if is_selected {
                            Color::from_rgba8(0, 0, 255, 255)
                        }
                        else {
                            Color::from_rgba8((red * 255.0) as u8, (green * 255.0) as u8, (blue * 255.0) as u8, (alpha * 255.0) as u8)
                        };

                        for riff in track.riffs().iter() {
                            if riff.uuid.uuid.to_string() == linked_to_riff_uuid {
                                let mut riff = riff.clone();
                                riff.uuid = UuidWrapper::new_from_string(riff_ref.uuid.clone());
                                riff.position = riff_ref.position;
                                let mut use_notes = match riff_ref.mode {
                                    RiffReferenceMode::Normal => true,
                                    RiffReferenceMode::Start => false,
                                    RiffReferenceMode::End => true,
                                };
                                if let Some((red, green, blue, alpha)) = riff.colour {
                                    colour = Color::from_rgba8((red * 255.0) as u8, (green * 255.0) as u8, (blue * 255.0) as u8, (alpha * 255.0) as u8);
                                }

                                let x = riff_ref.position * adjusted_beat_width_in_pixels;
                                let y = track_number as f64 * adjusted_entity_height_in_pixels;
                                let duration_in_beats = riff.length;
                                let width = duration_in_beats * beat_width_in_pixels * zoom_horizontal;

                                let riff_rect = Rect::new(
                                    x, y, x + width, y + adjusted_entity_height_in_pixels
                                );

                                // println!("Part: x1={}, y1={}, x2={}, y2={},", x, y, x + width, y + adjusted_entity_height_in_pixels);

                                // if x >= clip_x1 && x <= clip_x2 && y >= clip_y1 && y <= clip_y2 {
                                if riff_rect.overlaps(clip_rectangle.clone()) {
                                    // println!("Part in clip region");

                                    // self.edit_item_handler.handle_item_edit(
                                    //     context,
                                    //     &riff,
                                    //     operation_mode,
                                    //     mouse_pointer_x,
                                    //     mouse_pointer_y,
                                    //     mouse_pointer_previous_x,
                                    //     mouse_pointer_previous_y,
                                    //     adjusted_entity_height_in_pixels,
                                    //     adjusted_beat_width_in_pixels,
                                    //     x,
                                    //     y,
                                    //     width,
                                    //     height,
                                    //     drawing_area,
                                    //     edit_drag_cycle,
                                    //     tx_from_ui.clone(),
                                    //     false,
                                    //     track.uuid().to_string(),
                                    //     riff_ref,
                                    //     false,
                                    //     track_number as f64,
                                    //     is_selected,
                                    //     selected_riff_references.clone()
                                    // );

                                    let rect = Rect::new(x - 1.0, y + 1.0, x + width - 2.0, y + adjusted_entity_height_in_pixels - 2.0);
                                    scene.fill(
                                        Fill::NonZero,
                                        Affine::IDENTITY,
                                        colour,
                                        None,
                                        &rect,
                                    );

                                    // context.set_font_size(9.0);
                                    let mut name = riff.name.to_string();
                                    // let mut name_fits = false;
                                    // while !name_fits {
                                    //     if let Ok(text_extents) = context.text_extents(name.as_str()) {
                                    //         if (width - 2.0) < (text_extents.width as f64 + 10.0) {
                                    //             if !name.is_empty() {
                                    //                 name = name.as_str()[0..name.len() - 1].to_string();
                                    //             }
                                    //             else {
                                    //                 name_fits = true;
                                    //                 break;
                                    //             }
                                    //         }
                                    //         else {
                                    //             name_fits = true;
                                    //             break;
                                    //         }
                                    //     }
                                    // }

                                    self.draw_text(context, scene, x + 5.0, y + 10.0, name.as_str(), Color::BLACK, 8.0, FontWeight::EXTRA_BOLD);

                                    if let RiffReferenceMode::Start = riff_ref.mode {
                                        self.draw_text(context, scene, x, y + 8.0, "s", Color::BLACK, 8.0, FontWeight::EXTRA_BOLD);
                                    }
                                    else if let RiffReferenceMode::End = riff_ref.mode {
                                        self.draw_text(context, scene, x, y + 8.0, "e", Color::BLACK, 8.0, FontWeight::EXTRA_BOLD);
                                    }

                                    // draw notes
                                    for track_event in riff.events {
                                        match track_event {
                                            TrackEvent::ActiveSense => (),
                                            TrackEvent::AfterTouch => (),
                                            TrackEvent::ProgramChange => (),
                                            TrackEvent::Note(note) => {
                                                use_notes = match &riff_ref.mode {
                                                    RiffReferenceMode::Start => {
                                                        if !use_notes && note.riff_start_note { true }
                                                        else if use_notes { true }
                                                        else { false }
                                                    }
                                                    RiffReferenceMode::End => {
                                                        if use_notes && note.riff_start_note { false }
                                                        else if !use_notes { false }
                                                        else { true }
                                                    }
                                                    RiffReferenceMode::Normal => true,
                                                };

                                                if use_notes {
                                                    let note_x = (riff_ref.position + note.position) * adjusted_beat_width_in_pixels;

                                                    // draw note
                                                    if self.show_note {
                                                        let note_y = track_number as f64 * adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels - (adjusted_entity_height_in_pixels / 127.0 * note.note as f64);
                                                        // println!("Note: x={}, y={}, width={}, height={}", note_x, note_y, note.duration() * adjusted_beat_width_in_pixels, entity_height_in_pixels / 127.0);
                                                        let rect = Rect::new(note_x, note_y, note_x + note.length * adjusted_beat_width_in_pixels, note_y + adjusted_entity_height_in_pixels / 127.0);

                                                        scene.fill(
                                                            Fill::NonZero,
                                                            Affine::IDENTITY,
                                                            palette::css::BLACK,
                                                            None,
                                                            &rect,
                                                        );
                                                    }

                                                    // draw velocity
                                                    if self.show_note_velocity {
                                                        let velocity_y_start = track_number as f64 * adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels;
                                                        let stroke_color = Color::from_rgba8(0, 0, 0, 255);
                                                        let mut path = BezPath::new();

                                                        // println!("Note velocity: x={}, y={}, height={}", note_x, velocity_y_start, velocity_y_start - (entity_height_in_pixels / 127.0 * note.velocity as f64));
                                                        path.move_to(Point{x: note_x, y: velocity_y_start});
                                                        path.line_to(Point{x: note_x, y: velocity_y_start - (adjusted_entity_height_in_pixels / 127.0 * note.velocity as f64)});
                                                        scene.stroke(
                                                            &Stroke::new(1.0),
                                                            Affine::IDENTITY,
                                                            stroke_color,
                                                            None,
                                                            &path,
                                                        );
                                                    }
                                                }
                                            },
                                            TrackEvent::NoteOn(_) => (),
                                            TrackEvent::NoteOff(_) => (),
                                            TrackEvent::Controller(controller) => {
                                                let x_position = (riff_ref.position + controller.position) * adjusted_beat_width_in_pixels;
                                                let y_start = track_number as f64 * adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels;

                                                let mut path = BezPath::new();
                                                path.move_to(Point{x: x_position, y: y_start});
                                                path.line_to(Point{x: x_position, y: y_start - (adjusted_entity_height_in_pixels / 127.0 * (controller.value as f64))});
                                                let stroke_color = Color::from_rgba8(0, 0, 0, 255);
                                                scene.stroke(
                                                    &Stroke::new(1.0),
                                                    Affine::IDENTITY,
                                                    stroke_color,
                                                    None,
                                                    &path,
                                                );
                                            },
                                            TrackEvent::PitchBend(_pitch_bend) => (),
                                            TrackEvent::KeyPressure => (),
                                            TrackEvent::AudioPluginParameter(_parameter) => (),
                                            TrackEvent::Sample(_sample) => (),
                                            TrackEvent::Measure(_) => {}
                                            TrackEvent::NoteExpression(_) => {}
                                        }
                                    }
                                }
                                // else {
                                //     println!("Part not in clip region");
                                // }
                                break;
                            }
                        }
                    }

                    if self.show_automation {
                        for track_event in track.automation().events.iter() {
                            let x_position = track_event.position() * adjusted_beat_width_in_pixels;

                            if x_position >= clip_x1 && x_position <= clip_x2 {
                                match track_event {
                                    TrackEvent::ActiveSense => (),
                                    TrackEvent::AfterTouch => (),
                                    TrackEvent::ProgramChange => (),
                                    TrackEvent::Note(_) => {},
                                    TrackEvent::NoteOn(_) => (),
                                    TrackEvent::NoteOff(_) => (),
                                    TrackEvent::Controller(controller) => {
                                        let y_start = track_number as f64 * adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels;
                                        let mut path = BezPath::new();
                                        path.move_to(Point{x: x_position, y: y_start});
                                        path.line_to(Point{x: x_position, y: y_start - (adjusted_entity_height_in_pixels / 127.0 * (controller.value as f64))});
                                        let stroke_color = Color::from_rgba8(0, 0, 0, 255);
                                        scene.stroke(
                                            &Stroke::new(1.0),
                                            Affine::IDENTITY,
                                            stroke_color,
                                            None,
                                            &path,
                                        );
                                    },
                                    TrackEvent::PitchBend(_pitch_bend) => (),
                                    TrackEvent::KeyPressure => (),
                                    TrackEvent::AudioPluginParameter(_parameter) => (),
                                    TrackEvent::Sample(_sample) => (),
                                    TrackEvent::Measure(_) => {}
                                    TrackEvent::NoteExpression(_) => {}
                                }
                            }
                        }
                    }
                }

                if self.looping {
                    if let Some(active_loop_uuid) =  self.active_loop.as_ref() {
                        if let Some(active_loop) = project.song.loops().iter().find(|current_loop| current_loop.uuid.to_string() == active_loop_uuid.to_string()) {
                            let start_x = active_loop.start_position * adjusted_beat_width_in_pixels;
                            let end_x = active_loop.end_position * adjusted_beat_width_in_pixels;
                            let rect = Rect::new(start_x, 0.0, start_x + end_x - start_x, adjusted_entity_height_in_pixels);

                            scene.fill(
                                Fill::NonZero,
                                Affine::IDENTITY,
                                Color::from_rgba8(0, 255, 0 , 25),
                                None,
                                &rect,
                            );

                        }
                    }
                }
            },
            Err(_) => println!("Track grid custom painter could not get state lock."),
        }
        // println!("TrackGridCustomPainter::paint_custom - exited.");

        (
            entity_height_in_pixels,
            beat_width_in_pixels,
        )
    }

    fn track_cursor_time_in_beats(&self) -> f64 {
        0.0
    }

    fn set_track_cursor_time_in_beats(&mut self, track_cursor_time_in_beats: f64) {
    }

    fn as_any(&mut self) -> &mut dyn Any {
        self
    }
}

pub struct RiffGridCustomPainter {
    project: Arc<Mutex<Project>>,
    show_automation: bool,
    show_note: bool,
    show_note_velocity: bool,
    show_pan: bool,
    // pub edit_item_handler: EditItemHandler<Riff, RiffReference>,
    use_globally_selected_riff_grid: bool,
    riff_grid_uuid: Option<String>,
    track_cursor_time_in_beats: f64,
}

impl RiffGridCustomPainter {
    pub fn new_with_edit_item_handler(
        project: Arc<Mutex<Project>>,
        // edit_item_handler: EditItemHandler<Riff, RiffReference>,
        use_globally_selected_riff_grid: bool,
        riff_grid_uuid: Option<String>,
    ) -> RiffGridCustomPainter {
        RiffGridCustomPainter {
            project,
            show_automation: false,
            show_note: true,
            show_note_velocity: false,
            show_pan: false,
            // edit_item_handler,
            use_globally_selected_riff_grid,
            riff_grid_uuid,
            track_cursor_time_in_beats: 0.0,
        }
    }
    pub fn set_show_automation(&mut self, show_automation: bool) {
        self.show_automation = show_automation;
    }
    pub fn set_show_note(&mut self, show_note: bool) {
        self.show_note = show_note;
    }
    pub fn set_show_note_velocity(&mut self, show_note_velocity: bool) {
        self.show_note_velocity = show_note_velocity;
    }
    pub fn set_show_pan(&mut self, show_pan: bool) {
        self.show_pan = show_pan;
    }
}

impl CustomPainter for RiffGridCustomPainter {
    fn paint_custom(&mut self,
                    context: &mut PaintCtx<'_>,
                    entity_height_in_pixels: f64,
                    beat_width_in_pixels: f64,
                    zoom_horizontal: f64,
                    zoom_vertical: f64,
                    drawing_area_widget_name: Option<String>,
                    mouse_pointer_x: f64,
                    mouse_pointer_y: f64,
                    mouse_pointer_previous_x: f64,
                    mouse_pointer_previous_y: f64,
                    draw_mode_on: bool,
                    draw_mode: DrawMode,
                    draw_mode_start_x: f64,
                    draw_mode_start_y: f64,
                    draw_mode_end_x: f64,
                    draw_mode_end_y: f64,
                    scene: &mut Scene,
                    operation_mode: &OperationModeType,
                    drag_started: bool,
                    edit_drag_cycle: &DragCycle) -> (f64, f64) {
        let clip_rectangle = context.size().to_rect();
        let clip_x1 = clip_rectangle.x0;
        let clip_y1 = clip_rectangle.y0;
        let clip_x2 = clip_rectangle.x1;
        let clip_y2 = clip_rectangle.y1;

        match self.project.lock() {
            Ok(mut project) => {
                let adjusted_beat_width_in_pixels = beat_width_in_pixels * zoom_horizontal;
                let adjusted_entity_height_in_pixels = entity_height_in_pixels * zoom_vertical;
                let riff_grid_uuid_to_paint = if !self.use_globally_selected_riff_grid {
                    if let Some(riff_grid_uuid) = self.riff_grid_uuid.as_ref() {
                        riff_grid_uuid.to_string()
                    }
                    else {
                        "".to_string()
                    }
                }
                else if let Some(riff_grid_uuid) = self.riff_grid_uuid.as_ref() {
                    riff_grid_uuid.to_string()
                }
                else {
                    "".to_string()
                };

                // find all the selected riff refs
                // FIXME need to construct the custom painter with a selected_riff_grid_riff_references argument
                // let selected_riff_ref_ids = project.selected_riff_grid_riff_references().clone();
                let selected_riff_ref_ids = vec![];
                let mut riff_lengths = HashMap::new();
                for track in project.song().tracks().iter() {
                    for riff in track.riffs().iter() {
                        riff_lengths.insert(riff.id(), riff.length());
                    }
                }

                if let Some(riff_grid) = project.song().riff_grid(riff_grid_uuid_to_paint) {
                    let mut selected_riff_references = vec![];
                    let track_uuids = riff_grid.tracks().map(|key| key.clone()).collect_vec();

                    for (index, track_uuid) in track_uuids.iter().enumerate() {
                        if let Some(riff_references) = riff_grid.track_riff_references(track_uuid.clone()) {
                            for riff_reference in riff_references.iter().filter(|riff_ref| selected_riff_ref_ids.clone().contains(&riff_ref.uuid().to_string())) {
                                // find the riff length
                                if let Some(riff_length) = riff_lengths.get(&riff_reference.linked_to()) {
                                    let mut riff = Riff::new_with_position_length_and_colour(
                                        Uuid::parse_str(riff_reference.id().as_str()).unwrap(),
                                        riff_reference.position(),
                                        *riff_length,
                                        Some((0.0, 1.0, 0.0, 1.0)),
                                    );
                                    riff.set_vertical_index(index as i32);
                                    selected_riff_references.push(riff);
                                }
                            }
                        }
                    }

                    for (index, track) in project.song().tracks().iter().enumerate() {
                        let track_number = index as f64;
                        let (red, green, blue, alpha) = track.colour();

                        if let Some(riff_refs) = riff_grid.track_riff_references(track.uuid().to_string()) {
                            for riff_ref in riff_refs.iter() {
                                let linked_to_riff_uuid = riff_ref.linked_to();
                                let is_selected = selected_riff_ref_ids.iter().any(|id| *id == riff_ref.uuid().to_string());
                                let mut colour = if is_selected {
                                    Color::from_rgba8(0, 0, 255, 255)
                                }
                                else {
                                    Color::from_rgba8((red * 255.0) as u8, (green * 255.0)  as u8, (blue * 255.0) as u8, (alpha * 255.0) as u8)
                                };

                                if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == linked_to_riff_uuid) {
                                    let mut use_notes = match riff_ref.mode() {
                                        RiffReferenceMode::Normal => true,
                                        RiffReferenceMode::Start => false,
                                        RiffReferenceMode::End => true,
                                    };
                                    if let Some((red, green, blue, alpha)) = riff.colour() {
                                        colour = Color::from_rgba8((red * 255.0) as u8, (green * 255.0)  as u8, (blue * 255.0) as u8, (alpha * 255.0) as u8);
                                    }

                                    let x = riff_ref.position * adjusted_beat_width_in_pixels;
                                    let y = track_number as f64 * adjusted_entity_height_in_pixels;
                                    let duration_in_beats = riff.length;
                                    let width = duration_in_beats * beat_width_in_pixels * zoom_horizontal;

                                    let riff_rect = Rect::new(
                                        x, y, x + width, y + adjusted_entity_height_in_pixels
                                    );

                                    // debug!("Part: uuid={}, x1={}, y1={}, x2={}, y2={},", riff_ref.uuid().to_string(), x, y, x + width, y + adjusted_entity_height_in_pixels);

                                    // if x >= clip_x1 && x <= clip_x2 && y >= clip_y1 && y <= clip_y2 {
                                    if riff_rect.overlaps(clip_rectangle.clone()) {
                                        // debug!("Part in clip region");

                                        // self.edit_item_handler.handle_item_edit(
                                        //     context,
                                        //     riff,
                                        //     operation_mode,
                                        //     mouse_pointer_x,
                                        //     mouse_pointer_y,
                                        //     mouse_pointer_previous_x,
                                        //     mouse_pointer_previous_y,
                                        //     adjusted_entity_height_in_pixels,
                                        //     adjusted_beat_width_in_pixels,
                                        //     x,
                                        //     y,
                                        //     width,
                                        //     height,
                                        //     drawing_area,
                                        //     edit_drag_cycle,
                                        //     tx_from_ui.clone(),
                                        //     false,
                                        //     track.uuid().to_string(),
                                        //     riff_ref,
                                        //     false,
                                        //     track_number as f64,
                                        //     is_selected,
                                        //     selected_riff_references.clone()
                                        // );

                                    let rect = Rect::new(x - 1.0, y + 1.0, x + width - 2.0, y + adjusted_entity_height_in_pixels - 2.0);
                                    scene.fill(
                                        Fill::NonZero,
                                        Affine::IDENTITY,
                                        colour,
                                        None,
                                        &rect,
                                    );

                                    // context.set_font_size(9.0);
                                    let mut name = riff.name.to_string();
                                    // let mut name_fits = false;
                                    // while !name_fits {
                                    //     if let Ok(text_extents) = context.text_extents(name.as_str()) {
                                    //         if (width - 2.0) < (text_extents.width as f64 + 10.0) {
                                    //             if !name.is_empty() {
                                    //                 name = name.as_str()[0..name.len() - 1].to_string();
                                    //             }
                                    //             else {
                                    //                 name_fits = true;
                                    //                 break;
                                    //             }
                                    //         }
                                    //         else {
                                    //             name_fits = true;
                                    //             break;
                                    //         }
                                    //     }
                                    // }

                                    self.draw_text(context, scene, x + 5.0, y + 10.0, name.as_str(), Color::BLACK, 8.0, FontWeight::EXTRA_BOLD);

                                    if let RiffReferenceMode::Start = riff_ref.mode {
                                        self.draw_text(context, scene, x, y + 8.0, "s", Color::BLACK, 8.0, FontWeight::EXTRA_BOLD);
                                        }
                                    else if let RiffReferenceMode::End = riff_ref.mode {
                                        self.draw_text(context, scene, x, y + 8.0, "e", Color::BLACK, 8.0, FontWeight::EXTRA_BOLD);
                                        }

                                        // draw notes
                                        for track_event in riff.events() {
                                            match track_event {
                                                TrackEvent::ActiveSense => (),
                                                TrackEvent::AfterTouch => (),
                                                TrackEvent::ProgramChange => (),
                                                TrackEvent::Note(note) => {
                                                    use_notes = match &riff_ref.mode() {
                                                        RiffReferenceMode::Start => {
                                                            if !use_notes && note.riff_start_note() { true }
                                                            else if use_notes { true }
                                                            else { false }
                                                        }
                                                        RiffReferenceMode::End => {
                                                            if use_notes && note.riff_start_note() { false }
                                                            else if !use_notes { false }
                                                            else { true }
                                                        }
                                                        RiffReferenceMode::Normal => true,
                                                    };

                                                    if use_notes {
                                                        let note_x = (riff_ref.position() + note.position()) * adjusted_beat_width_in_pixels;

                                                        // draw note
                                                        if self.show_note {
                                                        let note_y = track_number as f64 * adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels - (adjusted_entity_height_in_pixels / 127.0 * note.note as f64);
                                                        // println!("Note: x={}, y={}, width={}, height={}", note_x, note_y, note.duration() * adjusted_beat_width_in_pixels, entity_height_in_pixels / 127.0);
                                                        let rect = Rect::new(note_x, note_y, note_x + note.length * adjusted_beat_width_in_pixels, note_y + adjusted_entity_height_in_pixels / 127.0);

                                                        scene.fill(
                                                            Fill::NonZero,
                                                            Affine::IDENTITY,
                                                            palette::css::BLACK,
                                                            None,
                                                            &rect,
                                                        );
                                                        }

                                                        // draw velocity
                                                        if self.show_note_velocity {
                                                        let velocity_y_start = track_number as f64 * adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels;
                                                        let stroke_color = Color::from_rgba8(0, 0, 0, 255);
                                                        let mut path = BezPath::new();

                                                        // println!("Note velocity: x={}, y={}, height={}", note_x, velocity_y_start, velocity_y_start - (entity_height_in_pixels / 127.0 * note.velocity as f64));
                                                        path.move_to(Point{x: note_x, y: velocity_y_start});
                                                        path.line_to(Point{x: note_x, y: velocity_y_start - (adjusted_entity_height_in_pixels / 127.0 * note.velocity as f64)});
                                                        scene.stroke(
                                                            &Stroke::new(1.0),
                                                            Affine::IDENTITY,
                                                            stroke_color,
                                                            None,
                                                            &path,
                                                        );
                                                        }
                                                    }
                                                },
                                                TrackEvent::NoteOn(_) => (),
                                                TrackEvent::NoteOff(_) => (),
                                                TrackEvent::Controller(controller) => {
                                                    let x_position = (riff_ref.position + controller.position) * adjusted_beat_width_in_pixels;
                                                    let y_start = track_number as f64 * adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels;

                                                    let mut path = BezPath::new();
                                                    path.move_to(Point{x: x_position, y: y_start});
                                                    path.line_to(Point{x: x_position, y: y_start - (adjusted_entity_height_in_pixels / 127.0 * (controller.value as f64))});
                                                    let stroke_color = Color::from_rgba8(0, 0, 0, 255);
                                                    scene.stroke(
                                                        &Stroke::new(1.0),
                                                        Affine::IDENTITY,
                                                        stroke_color,
                                                        None,
                                                        &path,
                                                    );
                                                },
                                                TrackEvent::PitchBend(_pitch_bend) => (),
                                                TrackEvent::KeyPressure => (),
                                                TrackEvent::AudioPluginParameter(_parameter) => (),
                                                TrackEvent::Sample(_sample) => (),
                                                TrackEvent::Measure(_) => {}
                                                TrackEvent::NoteExpression(_) => {}
                                            }
                                        }
                                    }
                                    else {
                                        // debug!("Part not in clip region");
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Err(_) => println!("Riff grid custom painter could not get state lock."),
        }
        // debug!("RiffGridCustomPainter::paint_custom - exited.");

        (
            entity_height_in_pixels,
            beat_width_in_pixels,
        )
    }

    fn track_cursor_time_in_beats(&self) -> f64 {
        self.track_cursor_time_in_beats
    }

    fn set_track_cursor_time_in_beats(&mut self, track_cursor_time_in_beats: f64) {
        self.track_cursor_time_in_beats = track_cursor_time_in_beats;
    }

    fn as_any(&mut self) -> &mut dyn Any {
        self
    }
}


pub trait BeatGridMouseCoordHelper {
    type Action: Any + Debug
    where
        Self: Sized;


    fn get_entity_vertical_value(&self, y: f64, entity_height_in_pixels: f64, zoom_vertical: f64) -> f64;
    fn get_snapped_to_time(&self, snap: f64, time: f64) -> f64 {
        let calculated_snap = DAWUtils::quantise(time as f64, snap as f64, 1.0, false);
        if calculated_snap.snapped {
            calculated_snap.snapped_value as f64
        } else {
            time
        }
    }

    fn get_time(&self, x: f64, beat_width_in_pixels: f64, zoom_horizontal: f64) -> f64 {
        x / (beat_width_in_pixels * zoom_horizontal)
    }
    fn select_single(&self, cx: &mut EventCtx, x: f64, y: i32, add_to_select: bool);
    fn select_multiple(&self, cx: &mut EventCtx, x: f64, y: i32, x2: f64, y2: i32, add_to_select: bool);
    fn deselect_single(&self, cx: &mut EventCtx, x: f64, y: i32);
    fn deselect_multiple(&self, cx: &mut EventCtx, x: f64, y: i32, x2: f64, y2: i32);
    fn add_entity(&self, cx: &mut EventCtx, y_index: i32, time: f64, duration: f64, entity_uuid: String);
    fn add_entity_extra(&self, cx: &mut EventCtx, y_index: i32, time: f64, duration: f64, entity_uuid: String);
    fn delete_entity(&self, cx: &mut EventCtx, y_index: i32, time: f64, entity_uuid: String);
    fn cut_selected(&self, cx: &mut EventCtx);
    fn copy_selected(&self, cx: &mut EventCtx);
    fn paste_selected(&self, cx: &mut EventCtx);
    fn handle_translate_up(&self, cx: &mut EventCtx);
    fn handle_translate_down(&self, cx: &mut EventCtx);
    fn handle_translate_left(&self, cx: &mut EventCtx);
    fn handle_translate_right(&self, cx: &mut EventCtx);
    fn handle_quantise(&self, cx: &mut EventCtx);
    fn handle_increase_entity_length(&self, cx: &mut EventCtx);
    fn handle_decrease_entity_length(&self, cx: &mut EventCtx);
    fn set_start_note(&self, cx: &mut EventCtx, y_index: i32, time: f64);
    fn set_riff_reference_play_mode(&self, cx: &mut EventCtx, y_index: i32, time: f64);
    fn handle_windowed_zoom(&self, cx: &mut EventCtx, x1: f64, y1: f64, x2: f64, y2: f64);
    fn cycle_entity_selection(&self, cx: &mut EventCtx, y_index: i32, time: f64);
    fn select_underlying_entity(&self, cx: &mut EventCtx, y_index: i32, time: f64);
}


pub struct BeatGridWidget{

    height: f64,
    width: f64,

    entity_height_in_pixels: f64,
    beat_width_in_pixels: f64,

    // zoom
    zoom_horizontal: f64,
    zoom_vertical: f64,
    zoom_factor: f64,

    beats_per_bar: i32,

    pub custom_painter: Option<Box<dyn CustomPainter>>,

    pub operation_mode: OperationModeType,

    mouse_coord_helper: Option<Box<dyn BeatGridMouseCoordHelper<Action=DAWEvents>>>,

    show_notes: bool,
    show_volume: bool,
    show_pan: bool,
    show_automation: bool,

    control_key_active: bool,
    shift_key_active: bool,
    alt_key_active: bool,

    drag_started: bool,

    snap_in_beats: f64,
    snap_strength: f64,
    snap_start: bool,
    snap_end: bool,
    new_entity_length_in_beats: f64,
    entity_length_increment_in_beats: f64,
    tempo: f64,

    triplet_spacing_in_beats: f64,

    //selection window
    draw_selection_window: bool,
    x_selection_window_position: f64,
    y_selection_window_position: f64,
    x_selection_window_position2: f64,
    y_selection_window_position2: f64,

    // zoom window
    draw_zoom_window: bool,
    x_zoom_window_position: f64,
    y_zoom_window_position: f64,
    x_zoom_window_position2: f64,
    y_zoom_window_position2: f64,

    //draw coords
    x_draw_position_start: f64,
    y_draw_position_start: f64,
    x_draw_position_end: f64,
    y_draw_position_end: f64,
    draw_item: bool,

    //track cursor
    track_cursor_time_in_beats: f64,

    // edit cursor
    edit_cursor_time_in_beats: f64,

    // mouse pointer coord
    mouse_pointer_position: (f64, f64),
    mouse_pointer_previous_position: (f64, f64),

    // tx_from_ui: crossbeam_channel::Sender<DAWEvents>,

    resize_drawing_area: bool,

    pub vertical_scale_painter: Option<Box<dyn CustomPainter>>,
    pub horizontal_scale_painter: Option<Box<dyn CustomPainter>>,

    // draw mode
    draw_mode_on: bool,
    draw_mode: DrawMode,
    draw_mode_x_start: f64,
    draw_mode_y_start: f64,
    draw_mode_x_end: f64,
    draw_mode_y_end: f64,

    // drag cycle state
    pub edit_drag_cycle: DragCycle,
    pub select_drag_cycle: DragCycle,
    pub windowed_zoom_drag_cycle: DragCycle,

    pub draw_play_cursor: bool,

    pub drawing_area_type: DrawingAreaType,

    pub last_mouse_movement_time_in_millis: u128,
}

impl Widget for BeatGridWidget {
    type Action = DAWEvents;

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
                println!("Button {:?} pressed, button={}, position: x={}, y={}", ctx.widget_id(), *button as i32, position.x, position.y);
                let mouse_button = if *button == PointerButton::Primary {
                    MouseButton::Button1
                }
                else if *button == PointerButton::Secondary {
                    MouseButton::Button2
                }
                else { // 4
                    MouseButton::Button3
                };
                self.handle_mouse_press(ctx, position.x, position.y, mouse_button, self.control_key_active, self.shift_key_active, self.alt_key_active);
            }
            PointerEvent::Up(pointer_button_event) => {
                if ctx.is_active() && ctx.is_hovered() {
                    let position = ctx.local_position(pointer_button_event.state.position);
                    let button = pointer_button_event.button;
                    // println!("Button {:?} released, button={}, position: x={}, y={}", ctx.widget_id(), *(button.as_ref().unwrap()) as i32, position.x, position.y);
                }
                let position = ctx.local_position(pointer_button_event.state.position);
                let button = pointer_button_event.button.as_ref().unwrap();
                // println!("Button {:?} released, button={}, position: x={}, y={}", ctx.widget_id(), *button as i32, position.x, position.y);
                let mouse_button = if *button == PointerButton::Primary {
                    MouseButton::Button1
                }
                else if *button == PointerButton::Secondary {
                    MouseButton::Button2
                }
                else { // 4
                    MouseButton::Button3
                };
                self.handle_mouse_release(ctx, position.x, position.y, mouse_button, self.control_key_active, self.shift_key_active, self.alt_key_active, "".to_string());
                // Changes in pointer capture impact appearance, but not accessibility node
                // ctx.request_paint_only();
                ctx.release_pointer();
            }
            PointerEvent::Move(pointer_button_event) => {
                let position = ctx.local_position(pointer_button_event.current.position);
                // let button = pointer_button_event.pointer.pointer_type;
                // println!("Mouse {:?} moved, button=, position: x={}, y={}", ctx.widget_id(), position.x, position.y);
                if (SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() - self.last_mouse_movement_time_in_millis) > 100 {
                    ctx.request_paint_only();
                    self.last_mouse_movement_time_in_millis = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
                }
                let mouse_button = if pointer_button_event.current.buttons.contains(PointerButton::Primary) {
                    MouseButton::Button1
                }
                else if pointer_button_event.current.buttons.contains(PointerButton::Secondary) {
                    MouseButton::Button2
                }
                else { // 4
                    MouseButton::Button3
                };
                self.handle_mouse_motion(ctx, position.x, position.y, mouse_button, self.control_key_active, self.shift_key_active, self.alt_key_active);
            }
            _ => (),
        }
    }

    fn on_text_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        props: &mut PropertiesMut<'_>,
        event: &TextEvent,
    ) {
        match event {
            TextEvent::Keyboard(key_event) => {
                let primary_key = key_event.key.to_string();

                println!(",,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,, key={}, binary={:?}, down={}, up={}", primary_key, key_event.modifiers, key_event.state.is_down(), key_event.state.is_up());
                println!(",,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,, modifiers: control={}, alt={}, shift={}", key_event.modifiers.ctrl(), key_event.modifiers.alt(), key_event.modifiers.shift());

                if key_event.state.is_down() {
                    // the actual primary is the key - last one down
                    match &key_event.key {
                        Key::Character(_) => {}
                        Key::Named(named_key) => match named_key {
                            NamedKey::Control => {
                                self.control_key_active = true;
                            }
                            NamedKey::Alt => {
                                self.alt_key_active = true;
                            }
                            NamedKey::Shift => {
                                self.shift_key_active = true;
                            }
                            _ => {}
                        }
                    }

                    // the secondaries are in the modifier
                    if key_event.modifiers.ctrl() {
                        self.control_key_active = true;
                    }
                    if key_event.modifiers.alt() {
                        self.alt_key_active = true;
                    }
                    if key_event.modifiers.shift() {
                        self.shift_key_active = true;
                    }
                }
                else {
                    self.control_key_active = false;
                    self.shift_key_active = false;
                    self.alt_key_active = false;
                }
            }
            _ => ()
        }
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

        self.paint_grid(ctx, scene);
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

    fn update(&mut self, ctx: &mut UpdateCtx<'_>, props: &mut PropertiesMut<'_>, event: &Update) {
        ctx.request_paint_only();
    }

    fn get_cursor(&self, ctx: &QueryCtx<'_>, pos: Point) -> CursorIcon {
        CursorIcon::Grab
    }

    fn accepts_focus(&self) -> bool {
        true
    }
}

impl BeatGridWidget{
    pub fn new(
        project: Arc<Mutex<Project>>,
        height: f64,
        width: f64,
        zoom_horizontal: f64,
        zoom_vertical: f64,
        entity_height_in_pixels: f64,
        beat_width_in_pixels: f64,
        beats_per_bar: i32,
        // tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        drawing_area_type: DrawingAreaType,
        piano_roll_mpe_note_id: i32,
        selected_riff_uuid: String,
        selected_riff_events: Vec<String>,
    ) -> BeatGridWidget {
        BeatGridWidget {
            height,
            width,

            entity_height_in_pixels,
            beat_width_in_pixels,

            zoom_horizontal,
            zoom_vertical,
            zoom_factor: 0.01,

            beats_per_bar,

            custom_painter: None,
            mouse_coord_helper: None,

            operation_mode: OperationModeType::PointMode,

            show_notes: true,
            show_volume: false,
            show_pan: false,
            show_automation: false,

            control_key_active: false,
            shift_key_active: false,
            alt_key_active: false,

            drag_started: false,

            snap_in_beats: 1.0,
            snap_strength: 1.0,
            snap_start: true,
            snap_end: false,
            new_entity_length_in_beats: 1.0,
            entity_length_increment_in_beats: 0.03125,
            tempo: 140.0,

            triplet_spacing_in_beats: 0.66666666,

            //selection window
            draw_selection_window: false,
            x_selection_window_position: 0.0,
            y_selection_window_position: 0.0,
            x_selection_window_position2: 0.0,
            y_selection_window_position2: 0.0,

            // zoom window
            draw_zoom_window: false,
            x_zoom_window_position: 0.0,
            y_zoom_window_position: 0.0,
            x_zoom_window_position2: 0.0,
            y_zoom_window_position2: 0.0,

            //draw coords
            x_draw_position_start: 0.0,
            y_draw_position_start: 0.0,
            x_draw_position_end: 0.0,
            y_draw_position_end: 0.0,
            draw_item: false,

            //track cursor
            track_cursor_time_in_beats: 0.0,

            // edit cursor
            edit_cursor_time_in_beats: 0.0,

            // mouse pointer coord
            mouse_pointer_position: (0.0, 0.0),
            mouse_pointer_previous_position: (0.0, 0.0),

            // tx_from_ui,

            resize_drawing_area: true,

            vertical_scale_painter: None,
            horizontal_scale_painter: None,

            draw_mode_on: false,
            draw_mode: DrawMode::Point,
            draw_mode_x_start: 0.0,
            draw_mode_y_start: 0.0,
            draw_mode_x_end: 0.0,
            draw_mode_y_end: 0.0,

            edit_drag_cycle: DragCycle::NotStarted,
            select_drag_cycle: DragCycle::NotStarted,
            windowed_zoom_drag_cycle: DragCycle::NotStarted,

            draw_play_cursor: true,

            drawing_area_type,

            last_mouse_movement_time_in_millis: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis(),
        }
    }

    pub fn new_with_custom(
        height: f64,
        width: f64,
        zoom_horizontal: f64,
        zoom_vertical: f64,
        entity_height_in_pixels: f64,
        beat_width_in_pixels: f64,
        beats_per_bar: i32,
        custom_painter: Option<Box<dyn CustomPainter>>,
        mouse_coord_helper: Option<Box<dyn BeatGridMouseCoordHelper<Action=DAWEvents>>>,
        // tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        resize_drawing_area: bool,
        drawing_area_type: DrawingAreaType,
        operation_mode: OperationModeType,
    ) -> BeatGridWidget {
        BeatGridWidget {
            height,
            width,

            entity_height_in_pixels,
            beat_width_in_pixels,

            zoom_horizontal,
            zoom_vertical,
            zoom_factor: 0.01,

            beats_per_bar,

            custom_painter,

            mouse_coord_helper,

            operation_mode,

            show_notes: true,
            show_volume: false,
            show_pan: false,
            show_automation: false,

            control_key_active: false,
            shift_key_active: false,
            alt_key_active: false,

            drag_started: false,

            snap_in_beats: 1.0,
            snap_strength: 1.0,
            snap_start: true,
            snap_end: false,
            new_entity_length_in_beats: 1.0,
            entity_length_increment_in_beats: 0.03125,
            tempo: 140.0,

            triplet_spacing_in_beats: 0.66666666,

            //selection window
            draw_selection_window: false,
            x_selection_window_position: 0.0,
            y_selection_window_position: 0.0,
            x_selection_window_position2: 0.0,
            y_selection_window_position2: 0.0,

            // zoom window
            draw_zoom_window: false,
            x_zoom_window_position: 0.0,
            y_zoom_window_position: 0.0,
            x_zoom_window_position2: 0.0,
            y_zoom_window_position2: 0.0,

            //draw coords
            x_draw_position_start: 0.0,
            y_draw_position_start: 0.0,
            x_draw_position_end: 0.0,
            y_draw_position_end: 0.0,
            draw_item: false,

            //track cursor
            track_cursor_time_in_beats: 0.0,

            // edit cursor
            edit_cursor_time_in_beats: 0.0,

            // mouse pointer coord
            mouse_pointer_position: (0.0, 0.0),
            mouse_pointer_previous_position: (0.0, 0.0),

            // tx_from_ui,

            resize_drawing_area,

            vertical_scale_painter: None,
            horizontal_scale_painter: None,

            draw_mode_on: false,
            draw_mode: DrawMode::Point,
            draw_mode_x_start: 0.0,
            draw_mode_y_start: 0.0,
            draw_mode_x_end: 0.0,
            draw_mode_y_end: 0.0,

            edit_drag_cycle: DragCycle::NotStarted,
            select_drag_cycle: DragCycle::NotStarted,
            windowed_zoom_drag_cycle: DragCycle::NotStarted,

            draw_play_cursor: true,

            drawing_area_type,

            last_mouse_movement_time_in_millis: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis(),
        }
    }

    pub fn new_with_painters(
        project: Arc<Mutex<Project>>,
        height: f64,
        width: f64,
        zoom_horizontal: f64,
        zoom_vertical: f64,
        entity_height_in_pixels: f64,
        beat_width_in_pixels: f64,
        beats_per_bar: i32,
        custom_painter: Option<Box<dyn CustomPainter>>,
        vertical_scale_painter: Option<Box<dyn CustomPainter>>,
        horizontal_scale_painter: Option<Box<dyn CustomPainter>>,
        mouse_coord_helper: Option<Box<dyn BeatGridMouseCoordHelper<Action=DAWEvents>>>,
        // tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        resize_drawing_area: bool,
        drawing_area_type: DrawingAreaType,
        piano_roll_mpe_note_id: i32,
        selected_riff_uuid: String,
        selected_riff_events: Vec<String>,
    ) -> BeatGridWidget {
        BeatGridWidget {
            height,
            width,

            entity_height_in_pixels,
            beat_width_in_pixels,

            zoom_horizontal,
            zoom_vertical,
            zoom_factor: 0.01,

            beats_per_bar,

            custom_painter,

            mouse_coord_helper,

            operation_mode: OperationModeType::PointMode,

            show_notes: true,
            show_volume: false,
            show_pan: false,
            show_automation: false,

            control_key_active: false,
            shift_key_active: false,
            alt_key_active: false,

            drag_started: false,

            snap_in_beats: 1.0,
            snap_strength: 1.0,
            snap_start: true,
            snap_end: false,
            new_entity_length_in_beats: 1.0,
            entity_length_increment_in_beats: 0.03125,
            tempo: 140.0,

            triplet_spacing_in_beats: 0.66666666,

            //selection window
            draw_selection_window: false,
            x_selection_window_position: 0.0,
            y_selection_window_position: 0.0,
            x_selection_window_position2: 0.0,
            y_selection_window_position2: 0.0,

            // zoom window
            draw_zoom_window: false,
            x_zoom_window_position: 0.0,
            y_zoom_window_position: 0.0,
            x_zoom_window_position2: 0.0,
            y_zoom_window_position2: 0.0,

            //draw coords
            x_draw_position_start: 0.0,
            y_draw_position_start: 0.0,
            x_draw_position_end: 0.0,
            y_draw_position_end: 0.0,
            draw_item: false,

            //track cursor
            track_cursor_time_in_beats: 0.0,

            // edit cursor
            edit_cursor_time_in_beats: 0.0,

            // mouse pointer coord
            mouse_pointer_position: (0.0, 0.0),
            mouse_pointer_previous_position: (0.0, 0.0),

            // tx_from_ui,

            resize_drawing_area,

            vertical_scale_painter,
            horizontal_scale_painter,

            draw_mode_on: false,
            draw_mode: DrawMode::Point,
            draw_mode_x_start: 0.0,
            draw_mode_y_start: 0.0,
            draw_mode_x_end: 0.0,
            draw_mode_y_end: 0.0,

            edit_drag_cycle: DragCycle::NotStarted,
            select_drag_cycle: DragCycle::NotStarted,
            windowed_zoom_drag_cycle: DragCycle::NotStarted,

            draw_play_cursor: true,

            drawing_area_type,

            last_mouse_movement_time_in_millis: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis(),
        }
    }

    fn paint_grid(&mut self, context: &mut PaintCtx<'_>, scene: &mut Scene) {
        let bounds_rect: Rect = context.size().to_rect();

        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            palette::css::WHITE,
            None,
            &bounds_rect,
        );


        let bounds = context.size();
        let height = bounds.height;
        let width = bounds.width;

        // if let Some(window) = drawing_area.window() {
        //     // window.set_cursor(Some(&gdk::Cursor::for_display(&window.display(), gdk::CursorType::Cross)));
        // }

        self.paint_vertical_scale(height, width, context, scene);
        self.paint_horizontal_scale(height, width, context, scene);
        self.paint_custom(height, width, /*scene.widget_name()*/"widget name".to_string(), context, scene);
        self.paint_loop_markers(height, width, context, scene);
        if self.draw_selection_window {
            self.paint_select_window(height, width, context, scene);
        }
        if self.draw_zoom_window {
            self.paint_zoom_window(height, width, context, scene);
        }
        if self.draw_play_cursor {
            self.paint_play_cursor(height, width, context, scene);
        }
        self.paint_edit_cursor(height, width, context, scene);
    }

    fn paint_vertical_scale(&mut self, height: f64, width: f64, context: &mut PaintCtx<'_>, scene: &mut Scene) {
        let entity_height_in_pixels = self.entity_height_in_pixels;
        let beat_width_in_pixels = self.beat_width_in_pixels;
        let zoom_horizontal = self.zoom_horizontal;
        let zoom_vertical = self.zoom_vertical;
        let operation_mode = self.operation_mode.clone();
        let edit_drag_cycle = self.edit_drag_cycle.clone();
        let x_selection_window_position = self.x_selection_window_position;
        let y_selection_window_position = self.y_selection_window_position;
        let _x_selection_window_position2 = self.x_selection_window_position2;
        let x_selection_window_position2 = self.x_selection_window_position2;
        // let tx_from_ui = self.tx_from_ui.clone();
        let clip_rect: Rect = context.size().to_rect();

        if let Some(vertical_scale_painter) = self.vertical_scale_painter.as_mut() {
            let (entity_height, entity_width) = vertical_scale_painter.paint_custom(
                context,
                entity_height_in_pixels,
                beat_width_in_pixels,
                zoom_horizontal,
                zoom_vertical,
                None,
                0.0,
                0.0,
                0.0,
                0.0,
                false,
                DrawMode::Point,
                0.0,
                0.0,
                0.0,
                0.0,
                scene,
                &operation_mode,
                false,
                &edit_drag_cycle,
            );


            // self.entity_height_in_pixels = entity_height;
            // self.beat_width_in_pixels = entity_width;
        }
        else {
            // paint.set_color(Color::rgba((0.9 * 255.0) as u8, (0.9 * 255.0) as u8, (0.9 * 255.0) as u8, (0.5 * 255.0) as u8));
            let adjusted_entity_height_in_pixels = self.entity_height_in_pixels * self.zoom_vertical;

            let mut current_y = 0.0;
            while current_y < clip_rect.height() {
                let row_number = current_y / adjusted_entity_height_in_pixels;

                if row_number as i32 % 2 == 0 {
                    let rect = Rect::new(0.0, current_y, clip_rect.width(), current_y + adjusted_entity_height_in_pixels);

                    scene.fill(
                        Fill::NonZero,
                        Affine::IDENTITY,
                        Color::from_rgb8(221, 221, 221),
                        None,
                        &rect,
                    );
                }

                current_y += adjusted_entity_height_in_pixels;
            }
        }
    }

    fn paint_horizontal_scale(&mut self, height: f64, width: f64, context: &mut PaintCtx<'_>, scene: &mut Scene) {
        let entity_height_in_pixels = self.entity_height_in_pixels;
        let beat_width_in_pixels = self.beat_width_in_pixels;
        let zoom_horizontal = self.zoom_horizontal;
        let zoom_vertical = self.zoom_vertical;
        let operation_mode = self.operation_mode.clone();
        let edit_drag_cycle = self.edit_drag_cycle.clone();
        // let tx_from_ui = self.tx_from_ui.clone();

        if let Some(horizontal_scale_painter_mut) = self.horizontal_scale_painter.as_mut() {
            let (entity_height, entity_width) = horizontal_scale_painter_mut.paint_custom(
                context,
                entity_height_in_pixels,
                beat_width_in_pixels,
                zoom_horizontal,
                zoom_vertical,
                None,
                0.0,
                0.0,
                0.0,
                0.0,
                false,
                DrawMode::Point,
                0.0,
                0.0,
                0.0,
                0.0,
                scene,
                &operation_mode,
                false,
                &edit_drag_cycle,
            );

            // self.entity_height_in_pixels = entity_height;
            // self.beat_width_in_pixels = entity_width;
        }
        else {
            let adjusted_beat_width_in_pixels = self.beat_width_in_pixels * self.zoom_horizontal;
            let bounds: Rect = context.size().to_rect();
            let clip_x1 = bounds.x0;
            let clip_y1 = bounds.y0;
            let clip_x2 = bounds.x1;
            let clip_y2 = bounds.y1;
            let clip_x1_in_beats = clip_x1 / adjusted_beat_width_in_pixels;
            let mut current_x = clip_x1_in_beats.floor() * adjusted_beat_width_in_pixels; // go to the first beat to the left of the view port e.g. bar 2 beat 3 = beat 2 * 4 + 3 = beat 11
            let mut beat_in_bar_index = (clip_x1_in_beats as i32 % self.beats_per_bar) + 1;

            while (bounds.x0 + current_x) < clip_x2 {
                let stroke_color = if beat_in_bar_index == 1 {
                    Color::from_rgba8((0.5 * 255.0) as u8, (0.5 * 255.0) as u8, (0.5 * 255.0) as u8, 255)
                } else {
                    Color::from_rgba8((0.5 * 255.0) as u8, (0.5 * 255.0) as u8, (0.5 * 255.0) as u8, 127)
                };

                let mut path = BezPath::new();
                path.move_to(Point{x: current_x, y: bounds.y0});
                path.line_to(Point{x: current_x, y: bounds.y1});

                scene.stroke(
                    &Stroke::new(0.3),
                    Affine::IDENTITY,
                    stroke_color,
                    None,
                    &path,
                );


                current_x += adjusted_beat_width_in_pixels;

                if beat_in_bar_index == self.beats_per_bar {
                    beat_in_bar_index = 1;
                } else {
                    beat_in_bar_index += 1;
                }
            }
        }
    }

    fn paint_custom(&mut self, height: f64, width: f64, drawing_area_widget_name: String, context: &mut PaintCtx<'_>, scene: &mut Scene) {
        let (x, y) = self.mouse_pointer_position;
        let (x_previous, y_previous) = self.mouse_pointer_previous_position;

        // let (x_selection_window_position, y_selection_window_position, x_selection_window_position2, y_selection_window_position2) = self.get_select_window();
        if let Some(custom_painter) = self.custom_painter.as_mut() {
            custom_painter.set_track_cursor_time_in_beats(self.track_cursor_time_in_beats);
            let (entity_height, entity_width) = custom_painter.paint_custom(
                context,
                self.entity_height_in_pixels,
                self.beat_width_in_pixels,
                self.zoom_horizontal,
                self.zoom_vertical,
                Some(drawing_area_widget_name),
                x,
                y,
                x_previous,
                y_previous,
                self.draw_mode_on,
                self.draw_mode.clone(),
                self.draw_mode_x_start,
                self.draw_mode_y_start,
                self.draw_mode_x_end,
                self.draw_mode_y_end,
                scene,
                &self.operation_mode,
                self.drag_started,
                &self.edit_drag_cycle.clone(),
            );

            // self.entity_height_in_pixels = entity_height;
            // self.beat_width_in_pixels = entity_width;

            match &self.edit_drag_cycle {
                DragCycle::MouseReleased => self.edit_drag_cycle = DragCycle::NotStarted,
                _ => {}
            }
        }
    }

    fn paint_select_window(&mut self, height: f64, width: f64, context: &mut PaintCtx<'_>, scene: &mut Scene) {
        let (top_left_x, top_left_y, bottom_right_x, bottom_right_y) = self.get_select_window();
        let rect = Rect::new(top_left_x, top_left_y, bottom_right_x, bottom_right_y);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::from_rgba8(0, 0, 255, 127),
            None,
            &rect,
        );
    }

    fn paint_zoom_window(&mut self, height: f64, width: f64, context: &mut PaintCtx<'_>, scene: &mut Scene) {
        let (top_left_x, top_left_y, bottom_right_x, bottom_right_y) = self.get_zoom_window();
        let rect = Rect::new(top_left_x, top_left_y, bottom_right_x, bottom_right_y);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::from_rgba8(0, 0, 0, 127),
            None,
            &rect,
        );
    }

    fn paint_loop_markers(&mut self, height: f64, width: f64, context: &mut PaintCtx<'_>, scene: &mut Scene) {
    }

    fn paint_play_cursor(&mut self, height: f64, width: f64, context: &mut PaintCtx<'_>, scene: &mut Scene) {
        let adjusted_beat_width_in_pixels = self.beat_width_in_pixels * self.zoom_horizontal;
        let x = self.track_cursor_time_in_beats * adjusted_beat_width_in_pixels;
        let clip_rect: Rect = context.size().to_rect();
        let clip_x1 = 0.0;
        let clip_y1 = 0.0;
        let clip_x2 = clip_rect.width();
        let clip_y2 = clip_rect.height();

        let mut path = BezPath::new();
        path.move_to(Point{x, y: 0.0});
        path.line_to(Point{x, y: clip_y2 - clip_y1});

        scene.stroke(
            &Stroke::new(1.0),
            Affine::IDENTITY,
            Color::from_rgba8(0, 0, 255, 255),
            None,
            &path,
        );
    }

    fn paint_edit_cursor(&mut self, height: f64, width: f64, context: &mut PaintCtx<'_>, scene: &mut Scene) {
        let adjusted_beat_width_in_pixels = self.beat_width_in_pixels * self.zoom_horizontal;
        let x = self.edit_cursor_time_in_beats * adjusted_beat_width_in_pixels;

        let mut path = BezPath::new();
        path.move_to(Point{x, y: 0.0});
        path.line_to(Point{x, y: height});

        scene.stroke(
            &Stroke::new(1.0),
            Affine::IDENTITY,
            Color::from_rgba8(255, 0, 255, 255),
            None,
            &path,
        );
    }

    pub fn get_select_window(&self) -> (f64, f64, f64, f64) {
        // find the top left x and y
        let top_left_x = self.x_selection_window_position.min(self.x_selection_window_position2);
        let top_left_y = self.y_selection_window_position.min(self.y_selection_window_position2);

        // find the bottom right x and y
        let bottom_right_x = self.x_selection_window_position.max(self.x_selection_window_position2);
        let bottom_right_y = self.y_selection_window_position.max(self.y_selection_window_position2);

        // println!("Window select: top_left_x={}, top_left_y={}, bottom_right_x={}, bottom_right_y={}", top_left_x, top_left_y, bottom_right_x, bottom_right_y);

        (top_left_x, top_left_y, bottom_right_x, bottom_right_y)
    }

    pub fn get_zoom_window(&self) -> (f64, f64, f64, f64) {
        // find the top left x and y
        let top_left_x = self.x_zoom_window_position.min(self.x_zoom_window_position2);
        let top_left_y = self.y_zoom_window_position.min(self.y_zoom_window_position2);

        // find the bottom right x and y
        let bottom_right_x = self.x_zoom_window_position.max(self.x_zoom_window_position2);
        let bottom_right_y = self.y_zoom_window_position.max(self.y_zoom_window_position2);

        (top_left_x, top_left_y, bottom_right_x, bottom_right_y)
    }

    pub fn zoom_horizontal_in(&mut self) {
        if self.zoom_horizontal < 7.0 {
            self.zoom_horizontal += self.zoom_factor;
        }
    }

    pub fn zoom_horizontal_out(&mut self) {
        if self.zoom_horizontal > (self.zoom_factor * 2.0) {
            self.zoom_horizontal -= self.zoom_factor;
        }
    }

    pub fn tempo(&self) -> f64 {
        self.tempo
    }

    pub fn tempo_mut(&mut self) -> &mut f64 {
        &mut self.tempo
    }

    pub fn set_tempo(&mut self, tempo: f64) {
        self.tempo = tempo;
    }

    pub fn set_snap_in_beats(&mut self, snap_in_beats: f64) {
        self.snap_in_beats = snap_in_beats;
    }

    pub fn set_new_entity_length_in_beats(&mut self, new_entity_length_in_beats: f64) {
        self.new_entity_length_in_beats = new_entity_length_in_beats;
    }

    pub fn set_entity_length_increment_in_beats(&mut self, entity_length_increment_in_beats: f64) {
        self.entity_length_increment_in_beats = entity_length_increment_in_beats;
    }

    pub fn snap_in_beats(&self) -> f64 {
        self.snap_in_beats
    }

    pub fn entity_length_in_beats(&self) -> f64 {
        self.new_entity_length_in_beats
    }

    pub fn entity_length_increment_in_beats(&self) -> f64 {
        self.entity_length_increment_in_beats
    }

    pub fn custom_painter(&mut self) -> &mut Option<Box<dyn CustomPainter>> {
        &mut self.custom_painter
    }

    pub fn beat_width_in_pixels(&self) -> f64 {
        self.beat_width_in_pixels
    }

    pub fn zoom_horizontal(&self) -> f64 {
        self.zoom_horizontal
    }

    pub fn turn_on_draw_point_mode(&mut self) {
        self.draw_mode = DrawMode::Point
    }

    pub fn turn_on_draw_line_mode(&mut self) {
        self.draw_mode = DrawMode::Line
    }

    pub fn turn_on_draw_curve_mode(&mut self) {
        self.draw_mode = DrawMode::Curve
    }

    pub fn zoom_vertical(&self) -> f64 {
        self.zoom_vertical
    }

    pub fn zoom_vertical_in(&mut self) {
        if self.zoom_vertical < 7.0 {
            self.zoom_vertical += self.zoom_factor;
        }
    }

    pub fn zoom_vertical_out(&mut self) {
        if self.zoom_vertical > (self.zoom_factor * 2.0) {
            self.zoom_vertical -= self.zoom_factor;
        }
    }

    pub fn set_horizontal_zoom(&mut self, zoom: f64) {
        // debug!("Horiz. zoom: {}", zoom);
        self.zoom_horizontal = zoom;
    }

    pub fn set_vertical_zoom(&mut self, zoom: f64) {
        self.zoom_vertical = zoom;
    }

    pub fn entity_height_in_pixels(&self) -> f64 {
        self.entity_height_in_pixels
    }

    /// Get the beat grid's track cursor time in beats.
    pub fn track_cursor_time_in_beats(&self) -> f64 {
        self.track_cursor_time_in_beats
    }

    /// Set the beat grid's track cursor time in beats.
    pub fn set_track_cursor_time_in_beats(&mut self, track_cursor_time_in_beats: f64) {
        self.track_cursor_time_in_beats = track_cursor_time_in_beats;
    }

    /// Get the beat grid's edit cursor time in beats.
    pub fn edit_cursor_time_in_beats(&self) -> f64 {
        self.edit_cursor_time_in_beats
    }

    /// Set the beat grid's edit cursor time in beats.
    pub fn set_edit_cursor_time_in_beats(&mut self, edit_cursor_time_in_beats: f64) {
        self.edit_cursor_time_in_beats = edit_cursor_time_in_beats;
    }

    pub fn drag_started_mut(&mut self) -> &mut bool {
        &mut self.drag_started
    }

    pub fn set_drag_started(&mut self, drag_started: bool) {
        self.drag_started = drag_started;
    }

    pub fn vertical_scale_painter_mut(&mut self) -> &mut Option<Box<dyn CustomPainter>> {
        &mut self.vertical_scale_painter
    }

    pub fn horizontal_scale_painter_mut(&mut self) -> &mut Option<Box<dyn CustomPainter>> {
        &mut self.horizontal_scale_painter
    }

    pub fn snap_strength(&self) -> f64 {
        self.snap_strength
    }

    pub fn set_snap_strength(&mut self, snap_strength: f64) {
        self.snap_strength = snap_strength;
    }

    pub fn snap_start(&self) -> bool {
        self.snap_start
    }

    pub fn set_snap_start(&mut self, snap_start: bool) {
        self.snap_start = snap_start;
    }

    pub fn snap_end(&self) -> bool {
        self.snap_end
    }

    pub fn set_snap_end(&mut self, snap_end: bool) {
        self.snap_end = snap_end;
    }

    pub fn set_draw_mode(&mut self, draw_mode: DrawMode) {
        self.draw_mode = draw_mode;
    }

    pub fn triplet_spacing_in_beats(&self) -> f64 {
        self.triplet_spacing_in_beats
    }

    pub fn set_triplet_spacing_in_beats(&mut self, triplet_spacing_in_beats: f64) {
        self.triplet_spacing_in_beats = triplet_spacing_in_beats;
    }

    pub fn set_beats_per_bar(&mut self, beats_per_bar: i32) {
        self.beats_per_bar = beats_per_bar;
    }
}

impl MouseHandler for BeatGridWidget {

    fn handle_mouse_motion(&mut self, cx: &mut EventCtx, x: f64, y: f64, mouse_button: MouseButton, control_key: bool, shift_key: bool, alt_key: bool) {
        self.control_key_active = control_key;
        self.shift_key_active = shift_key;
        self.alt_key_active = alt_key;
        self.mouse_pointer_position = (x, y);

        // println!("Mouse motion: x={}, y={}", x, y);

        match mouse_button {
            MouseButton::Button1 => {
                match self.operation_mode {
                    OperationModeType::PointMode => {
                        self.x_selection_window_position2 = x;
                        self.y_selection_window_position2 = y;
                        self.select_drag_cycle = DragCycle::Dragging;
                        cx.request_render();
                    },
                    OperationModeType::WindowedZoom => {
                        self.x_zoom_window_position2 = x;
                        self.y_zoom_window_position2 = y;
                        self.windowed_zoom_drag_cycle = DragCycle::Dragging;
                        cx.request_render();
                    },
                    OperationModeType::Add => {
                        self.draw_mode_x_end = x;
                        self.draw_mode_y_end = y;
                        cx.request_render();
                    },
                    OperationModeType::LoopPointMode => {

                    },
                    OperationModeType::Change => {
                        if control_key {
                            println!("Mouse motion: changed to EditDragCycle::CtrlDragging");
                            self.edit_drag_cycle = DragCycle::CtrlDragging;
                        }
                        else {
                            println!("Mouse motion: changed to EditDragCycle::Dragging");
                            self.edit_drag_cycle = DragCycle::Dragging;
                        }
                        cx.request_render();
                    },
                    _ => (),
                }
            },
            MouseButton::Button2 => {

            },
            MouseButton::Button3 => {

            },
        }
    }

    fn handle_mouse_press(&mut self, cx: &mut EventCtx, x: f64, y: f64, mouse_button: MouseButton, control_key: bool, shift_key: bool, alt_key: bool) {
        self.control_key_active = control_key;
        self.shift_key_active = shift_key;
        self.alt_key_active = alt_key;

        cx.request_focus();

        match mouse_button {
            MouseButton::Button1 => {
                match self.operation_mode {
                    OperationModeType::PointMode => {
                        self.x_selection_window_position = x;
                        self.y_selection_window_position = y;
                        self.draw_selection_window = true;
                        self.select_drag_cycle = DragCycle::MousePressed;
                    }
                    OperationModeType::WindowedZoom => {
                        self.x_zoom_window_position = x;
                        self.y_zoom_window_position = y;
                        self.draw_zoom_window = true;
                        self.windowed_zoom_drag_cycle = DragCycle::MousePressed;
                    }
                    OperationModeType::Change => {
                        if control_key {
                            println!("Mouse pressed: changed to EditDragCycle::CtrlMousePressed");
                            self.edit_drag_cycle = DragCycle::CtrlMousePressed;
                        }
                        else {
                            println!("Mouse pressed: changed to EditDragCycle::MousePressed");
                            self.edit_drag_cycle = DragCycle::MousePressed;
                        }
                        self.mouse_pointer_previous_position = (x, y);
                        cx.request_render();
                    }
                    OperationModeType::Add => {
                        self.draw_mode_x_start = x;
                        self.draw_mode_y_start = y;
                        self.draw_mode_on = true;
                    }
                    _ => (),
                }
            }
            MouseButton::Button2 => {

            }
            MouseButton::Button3 => {

            }
        }
    }

    fn handle_mouse_release(&mut self, cx: &mut EventCtx, x: f64, y: f64, mouse_button: MouseButton, control_key: bool, shift_key: bool, alt_key: bool, data: String) {
        self.control_key_active = control_key;
        self.shift_key_active = shift_key;
        self.alt_key_active = alt_key;

        match mouse_button {
            MouseButton::Button1 => {
                match self.operation_mode {
                    OperationModeType::Add => {
                        self.draw_mode_x_end = x;
                        self.draw_mode_y_end = y;
                        self.draw_mode_on = false;

                        match &self.mouse_coord_helper {
                            Some(mouse_coord_helper) => {
                                if let DrawMode::Point = self.draw_mode {
                                    let y_index = mouse_coord_helper.get_entity_vertical_value(y, self.entity_height_in_pixels, self.zoom_vertical);
                                    let position = mouse_coord_helper.get_time(x, self.beat_width_in_pixels, self.zoom_horizontal);
                                    let snap_position = mouse_coord_helper.get_snapped_to_time(self.snap_in_beats, position);
                                    let duration = self.new_entity_length_in_beats - 0.01; // take off just a little off so that the note off does not overlap the next note on

                                    mouse_coord_helper.add_entity(cx, y_index as i32, snap_position, duration, data);
                                }
                                else if let DrawMode::Line = self.draw_mode {
                                    let x_start_position = mouse_coord_helper.get_time(self.draw_mode_x_start, self.beat_width_in_pixels, self.zoom_horizontal);
                                    let y_start_index = mouse_coord_helper.get_entity_vertical_value(self.draw_mode_y_start, self.entity_height_in_pixels, self.zoom_vertical);
                                    let x_end_position = mouse_coord_helper.get_time(self.draw_mode_x_end, self.beat_width_in_pixels, self.zoom_horizontal);
                                    let y_end_index = mouse_coord_helper.get_entity_vertical_value(self.draw_mode_y_end, self.entity_height_in_pixels, self.zoom_vertical);
                                    let snap_position_start = mouse_coord_helper.get_snapped_to_time(self.snap_in_beats, x_start_position);
                                    let snap_position_end = mouse_coord_helper.get_snapped_to_time(self.snap_in_beats, x_end_position);

                                    let mut position = snap_position_start;
                                    let mut y_start = y_start_index;
                                    let mut number_of_events = 0;
                                    while position <= snap_position_end {
                                        position += self.snap_in_beats;
                                        number_of_events += 1;
                                    }

                                    let y_increment = (y_end_index - y_start_index) / (number_of_events - 1) as f64;
                                    position = snap_position_start;
                                    y_start = y_start_index;
                                    while position <= snap_position_end {
                                        mouse_coord_helper.add_entity(cx, y_start as i32, position, 0.0, data.clone());
                                        position += self.snap_in_beats;
                                        y_start += y_increment;
                                    }
                                }
                                else if let DrawMode::Triplet = self.draw_mode {
                                    let y_index = mouse_coord_helper.get_entity_vertical_value(y, self.entity_height_in_pixels, self.zoom_vertical);
                                    let position = mouse_coord_helper.get_time(x, self.beat_width_in_pixels, self.zoom_horizontal);
                                    let snap_position = mouse_coord_helper.get_snapped_to_time(self.snap_in_beats, position);
                                    let duration = self.new_entity_length_in_beats - 0.01; // take off just a little off so that the note off does not overlap the next note on

                                    mouse_coord_helper.add_entity(cx, y_index as i32, snap_position, duration, data.clone());
                                    mouse_coord_helper.add_entity(cx, y_index as i32, snap_position + self.triplet_spacing_in_beats, duration, data.clone());
                                    mouse_coord_helper.add_entity(cx, y_index as i32, snap_position + (self.triplet_spacing_in_beats * 2.0), duration, data);
                                }
                            },
                            None => (),
                        }
                    },
                    OperationModeType::Delete => {
                        match &self.mouse_coord_helper {
                            Some(mouse_coord_helper) => {
                                let y_index = mouse_coord_helper.get_entity_vertical_value(y, self.entity_height_in_pixels, self.zoom_vertical);
                                let position = mouse_coord_helper.get_time(x, self.beat_width_in_pixels, self.zoom_horizontal);
                                mouse_coord_helper.delete_entity(cx, y_index as i32, position, data);
                            },
                            None => (),
                        }
                    },
                    OperationModeType::Change => {
                        if control_key {
                            self.edit_drag_cycle = DragCycle::CtrlMouseReleased;
                            println!("Mouse release: changed to EditDragCycle::CtrlMouseReleased");
                        }
                        else {
                            self.edit_drag_cycle = DragCycle::MouseReleased;
                            println!("Mouse release: changed to EditDragCycle::MouseReleased");
                        }
                        cx.request_render();
                    },
                    OperationModeType::PointMode => {
                        self.draw_selection_window = false;
                        if let DrawingAreaType::Riff = self.drawing_area_type {
                            if let Some(custom_painter) = self.custom_painter.as_mut() {
                                if let Some(riff_set__track_custom_painter) = custom_painter.as_any().downcast_mut::<RiffSetTrackCustomPainter>() {
                                    cx.submit_action::<DAWEvents>(DAWEvents::RiffSetTrackIncrementRiff(
                                        riff_set__track_custom_painter.riff_set_uuid.clone(),
                                        riff_set__track_custom_painter.track_uuid.clone()));
                                }
                            }
                        }
                        else if let DragCycle::Dragging = self.select_drag_cycle {
                            self.select_drag_cycle = DragCycle::NotStarted;
                            // send an event to the ui via the mouse coord helper
                            if let Some(mouse_coord_helper) = self.mouse_coord_helper.as_ref() {
                                let select_window = self.get_select_window();

                                if shift_key {
                                    mouse_coord_helper.deselect_multiple(
                                        cx,
                                        mouse_coord_helper.get_time(select_window.0, self.beat_width_in_pixels, self.zoom_horizontal),
                                        mouse_coord_helper.get_entity_vertical_value(select_window.1, self.entity_height_in_pixels, self.zoom_vertical) as i32,
                                        mouse_coord_helper.get_time(select_window.2, self.beat_width_in_pixels, self.zoom_horizontal),
                                        mouse_coord_helper.get_entity_vertical_value(select_window.3, self.entity_height_in_pixels, self.zoom_vertical) as i32
                                    );
                                }
                                else {
                                    let add_to_select = control_key; // should this be the shift key???
                                    mouse_coord_helper.select_multiple(
                                        cx,
                                        mouse_coord_helper.get_time(select_window.0, self.beat_width_in_pixels, self.zoom_horizontal),
                                        mouse_coord_helper.get_entity_vertical_value(select_window.1, self.entity_height_in_pixels, self.zoom_vertical) as i32,
                                        mouse_coord_helper.get_time(select_window.2, self.beat_width_in_pixels, self.zoom_horizontal),
                                        mouse_coord_helper.get_entity_vertical_value(select_window.3, self.entity_height_in_pixels, self.zoom_vertical) as i32,
                                        add_to_select
                                    );
                                }
                            }
                        }
                        else if shift_key { // deselect single item
                            // send an event to the ui via the mouse coord helper
                            if let Some(mouse_coord_helper) = self.mouse_coord_helper.as_ref() {
                                mouse_coord_helper.deselect_single(
                                    cx,
                                    mouse_coord_helper.get_time(x, self.beat_width_in_pixels, self.zoom_horizontal),
                                    mouse_coord_helper.get_entity_vertical_value(y, self.entity_height_in_pixels, self.zoom_vertical) as i32,
                                );
                            }
                        }
                        else if control_key { // select single item
                            // send an event to the ui via the mouse coord helper
                            if let Some(mouse_coord_helper) = self.mouse_coord_helper.as_ref() {
                                mouse_coord_helper.select_single(
                                    cx,
                                    mouse_coord_helper.get_time(x, self.beat_width_in_pixels, self.zoom_horizontal),
                                    mouse_coord_helper.get_entity_vertical_value(y, self.entity_height_in_pixels, self.zoom_vertical) as i32,
                                    true
                                );
                            }
                        }
                        else /*if let DragCycle::NotStarted = self.select_drag_cycle*/ {
                            if let Some(mouse_coord_helper) = self.mouse_coord_helper.as_ref() {
                                let y_index = mouse_coord_helper.get_entity_vertical_value(y, self.entity_height_in_pixels, self.zoom_vertical);
                                let position = mouse_coord_helper.get_time(x, self.beat_width_in_pixels, self.zoom_horizontal);

                                mouse_coord_helper.cycle_entity_selection(cx, y_index as i32, position);
                            }
                        }
                        cx.request_render();
                    }
                    OperationModeType::WindowedZoom => {
                        self.draw_zoom_window = false;
                        self.windowed_zoom_drag_cycle = DragCycle::NotStarted;
                        // send an event to the ui via the mouse coord helper
                        if let Some(mouse_coord_helper) = self.mouse_coord_helper.as_ref() {
                            let (x1, y1, x2, y2) = self.get_zoom_window();
                            mouse_coord_helper.handle_windowed_zoom(cx, x1, y1, x2, y2);
                        }
                        cx.request_render();
                    }
                    OperationModeType::LoopPointMode => {
                        match &self.mouse_coord_helper {
                            Some(mouse_coord_helper) => {
                                let position = mouse_coord_helper.get_time(x, self.beat_width_in_pixels, self.zoom_horizontal);
                                let snap_position = mouse_coord_helper.get_snapped_to_time(self.snap_in_beats, position);
                                cx.submit_action::<DAWEvents>(DAWEvents::LoopChange(LoopChangeType::LoopLimitLeftChanged(snap_position as f64), Uuid::new_v4()));
                            }
                            None => (),
                        }
                    },
                    OperationModeType::SelectStartNote => {
                        match &self.mouse_coord_helper {
                            Some(mouse_coord_helper) => {
                                let y_index = mouse_coord_helper.get_entity_vertical_value(y, self.entity_height_in_pixels, self.zoom_vertical);
                                let position = mouse_coord_helper.get_time(x, self.beat_width_in_pixels, self.zoom_horizontal);
                                mouse_coord_helper.set_start_note(cx, y_index as i32, position);
                            },
                            None => (),
                        }
                    }
                    OperationModeType::SelectRiffReferenceMode => {
                        match &self.mouse_coord_helper {
                            Some(mouse_coord_helper) => {
                                let y_index = mouse_coord_helper.get_entity_vertical_value(y, self.entity_height_in_pixels, self.zoom_vertical);
                                let position = mouse_coord_helper.get_time(x, self.beat_width_in_pixels, self.zoom_horizontal);
                                mouse_coord_helper.set_riff_reference_play_mode(cx, y_index as i32, position);
                            },
                            None => (),
                        }
                    }
                }
            },
            MouseButton::Button2 => {
                match self.operation_mode {
                    OperationModeType::Add => {
                        if let Some(mouse_coord_helper) = self.mouse_coord_helper.as_ref() {
                            let y_index = mouse_coord_helper.get_entity_vertical_value(y, self.entity_height_in_pixels, self.zoom_vertical);
                            let position = mouse_coord_helper.get_time(x, self.beat_width_in_pixels, self.zoom_horizontal);
                            let snap_position = mouse_coord_helper.get_snapped_to_time(self.snap_in_beats, position);
                            let duration = self.new_entity_length_in_beats - 0.01; // take off just a little off so that the note off does not overlap the next note on
                            let new_riff_uuid = Uuid::new_v4();

                            mouse_coord_helper.add_entity_extra(cx, y_index as i32, snap_position, duration, new_riff_uuid.to_string());
                            mouse_coord_helper.add_entity(cx, y_index as i32, snap_position, duration, new_riff_uuid.to_string());
                        }
                    }
                    OperationModeType::Delete => println!("mouse button clicked=2, mode={:?}", self.operation_mode),
                    OperationModeType::Change => println!("mouse button clicked=2, mode={:?}", self.operation_mode),
                    OperationModeType::PointMode => {
                        if let DrawingAreaType::Riff = self.drawing_area_type {
                            if let Some(custom_painter) = self.custom_painter.as_mut() {
                                if let Some(riff_set__track_custom_painter) = custom_painter.as_any().downcast_mut::<RiffSetTrackCustomPainter>() {
                                    let new_riff_uuid = Uuid::new_v4().to_string();
                                    cx.submit_action::<DAWEvents>(DAWEvents::TrackChange(
                                        TrackChangeType::RiffAdd(
                                            new_riff_uuid.clone(),
                                            "New riff".to_string(),
                                            4.0
                                    ), Some(riff_set__track_custom_painter.track_uuid.clone())));
                                    cx.submit_action::<DAWEvents>(DAWEvents::RiffSetTrackSetRiff(
                                        riff_set__track_custom_painter.riff_set_uuid.clone(),
                                        riff_set__track_custom_painter.track_uuid.clone(),
                                        new_riff_uuid
                                    ));
                                }
                            }
                        }
                    }
                    OperationModeType::LoopPointMode => println!("mouse button clicked=2, mode={:?}", self.operation_mode),
                    OperationModeType::SelectStartNote => {}
                    OperationModeType::SelectRiffReferenceMode => {}
                    OperationModeType::WindowedZoom => {}
                }
            },
            MouseButton::Button3 => {
                match self.operation_mode {
                    OperationModeType::Add => println!("mouse button clicked=3, mode={:?}", self.operation_mode),
                    OperationModeType::Delete => println!("mouse button clicked=3, mode={:?}", self.operation_mode),
                    OperationModeType::Change => println!("mouse button clicked=3, mode={:?}", self.operation_mode),
                    OperationModeType::PointMode => {
                        if let DrawingAreaType::Riff = self.drawing_area_type {
                            if let Some(custom_painter) = self.custom_painter.as_mut() {
                                if let Some(riff_set__track_custom_painter) = custom_painter.as_any().downcast_mut::<RiffSetTrackCustomPainter>() {
                                    cx.submit_action::<DAWEvents>(DAWEvents::TrackChange(
                                        TrackChangeType::RiffSelect(riff_set__track_custom_painter.riff_set_uuid.clone()),
                                        Some(riff_set__track_custom_painter.track_uuid.clone()))
                                    );
                                }
                            }
                        }
                        else if shift_key {
                            match &self.mouse_coord_helper {
                                Some(mouse_coord_helper) => {
                                    let position = mouse_coord_helper.get_time(x, self.beat_width_in_pixels, self.zoom_horizontal);
                                    let snap_position = mouse_coord_helper.get_snapped_to_time(self.snap_in_beats, position);
                                    self.track_cursor_time_in_beats = snap_position;
                                    cx.submit_action::<DAWEvents>(DAWEvents::PlayPositionInBeats(snap_position as f64));
                                    match self.drawing_area_type {
                                        DrawingAreaType::PianoRoll => {
                                            cx.submit_action::<DAWEvents>(DAWEvents::RepaintPianoRollView);
                                        }
                                        DrawingAreaType::TrackGrid => {
                                            cx.submit_action::<DAWEvents>(DAWEvents::RepaintTrackGridView);
                                        }
                                        DrawingAreaType::Automation => {
                                            cx.submit_action::<DAWEvents>(DAWEvents::RepaintAutomationView);
                                        }
                                        _ => {}
                                    }
                                },
                                None => (),
                            }
                        }
                        else if control_key {
                            match &self.mouse_coord_helper {
                                Some(mouse_coord_helper) => {
                                    let position = mouse_coord_helper.get_time(x, self.beat_width_in_pixels, self.zoom_horizontal);
                                    let snap_position = mouse_coord_helper.get_snapped_to_time(self.snap_in_beats, position);
                                    self.edit_cursor_time_in_beats = snap_position;
                                    match self.drawing_area_type {
                                        DrawingAreaType::PianoRoll => {
                                            cx.submit_action::<DAWEvents>(DAWEvents::RepaintPianoRollView);
                                        }
                                        DrawingAreaType::TrackGrid => {
                                            cx.submit_action::<DAWEvents>(DAWEvents::RepaintTrackGridView);
                                            cx.submit_action::<DAWEvents>(DAWEvents::TrackGridEditCursorPositionChanged(self.edit_cursor_time_in_beats.clone()));
                                        }
                                        DrawingAreaType::Automation => {
                                            cx.submit_action::<DAWEvents>(DAWEvents::RepaintAutomationView);
                                        }
                                        _ => {}
                                    }
                                },
                                None => (),
                            }
                        }
                        else {
                            if let Some(mouse_coord_helper) = self.mouse_coord_helper.as_ref() {
                                let y_index = mouse_coord_helper.get_entity_vertical_value(y, self.entity_height_in_pixels, self.zoom_vertical);
                                let position = mouse_coord_helper.get_time(x, self.beat_width_in_pixels, self.zoom_horizontal);

                                mouse_coord_helper.select_underlying_entity(cx, y_index as i32, position);
                            }
                        }
                    }
                    OperationModeType::LoopPointMode => match &self.mouse_coord_helper {
                        Some(mouse_coord_helper) => {
                            let position = mouse_coord_helper.get_time(x, self.beat_width_in_pixels, self.zoom_horizontal);
                            let snap_position = mouse_coord_helper.get_snapped_to_time(self.snap_in_beats, position);
                            cx.submit_action::<DAWEvents>(DAWEvents::LoopChange(LoopChangeType::LoopLimitRightChanged(snap_position as f64), Uuid::new_v4()));
                        }
                        None => (),
                    }
                    OperationModeType::SelectStartNote => {}
                    OperationModeType::SelectRiffReferenceMode => {}
                    OperationModeType::WindowedZoom => {}
                }
            },
        }
    }
}
