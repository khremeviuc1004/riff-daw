use std::{sync::{Arc, Mutex}, vec::Vec};
use std::any::Any;
use std::collections::HashMap;
use cairo::{Context};
use crossbeam_channel::Sender;
use gtk::{DrawingArea, prelude::*};
use itertools::Itertools;
use log::*;
use strum_macros::Display;
use uuid::Uuid;
use geo::{coord, Intersects, Rect};

use crate::{domain::*, event::{DAWEvents, LoopChangeType, OperationModeType, TrackChangeType, TranslateDirection, TranslationEntityType, AutomationEditType}, state::DAWState, constants::NOTE_NAMES};
use crate::event::{CurrentView, RiffGridChangeType};
use crate::event::TrackChangeType::RiffReferencePlayMode;

#[derive(Debug)]
pub enum MouseButton {
    Button1,
    Button2,
    Button3,
}

pub trait MouseHandler {
    fn handle_mouse_motion(&mut self, x: f64, y: f64, drawing_area: &DrawingArea, mouse_button: MouseButton, control_key: bool, shift_key: bool, alt_key: bool);
    fn handle_mouse_press(&mut self, x: f64, y: f64, drawing_area: &DrawingArea, mouse_button: MouseButton, control_key: bool, shift_key: bool, alt_key: bool);
    fn handle_mouse_release(&mut self, x: f64, y: f64, drawing_area: &DrawingArea, mouse_button: MouseButton, control_key: bool, shift_key: bool, alt_key: bool, data: String);
}

#[derive(Clone)]
pub enum DrawMode {
    Point,
    Line,
    Curve,
}

#[derive(Clone)]
pub enum DrawingAreaType {
    PianoRoll,
    TrackGrid,
    Automation,
    Riff,
    RiffGrid,
}

pub trait Grid : MouseHandler {
    fn paint(&mut self, context: &Context, drawing_area: &DrawingArea);
    fn paint_vertical_scale(&mut self, context: &Context, height: f64, width: f64, drawing_area: &DrawingArea);
    fn paint_horizontal_scale(&mut self, context: &Context, height: f64, width: f64);
    fn paint_custom(&mut self, context: &Context, height: f64, width: f64, drawing_area_widget_name: String, drawing_area: &DrawingArea);
    fn paint_select_window(&mut self, context: &Context, height: f64, width: f64);
    fn paint_loop_markers(&mut self, context: &Context, height: f64, width: f64);
    fn paint_play_cursor(&mut self, context: &Context, height: f64, width: f64);
    fn paint_edit_cursor(&mut self, context: &Context, height: f64, width: f64);
    // fn handle_mouse_motion(&mut self, x: f64, y: f64, drawing_area: &DrawingArea, mouse_button: MouseButton, control_key: bool, shift_key: bool, alt_key: bool);
    // fn handle_mouse_press(&mut self, x: f64, y: f64, drawing_area: &DrawingArea, mouse_button: MouseButton, control_key: bool, shift_key: bool, alt_key: bool);
    // fn handle_mouse_release(&mut self, x: f64, y: f64, drawing_area: &DrawingArea, mouse_button: MouseButton, control_key: bool, shift_key: bool, alt_key: bool);
    fn handle_cut(&mut self, drawing_area: &DrawingArea);
    fn handle_copy(&mut self, drawing_area: &DrawingArea);
    fn handle_paste(&mut self, drawing_area: &DrawingArea);
    fn handle_translate_up(&mut self, drawing_area: &DrawingArea);
    fn handle_translate_down(&mut self, drawing_area: &DrawingArea);
    fn handle_translate_left(&mut self, drawing_area: &DrawingArea);
    fn handle_translate_right(&mut self, drawing_area: &DrawingArea);
    fn handle_quantise(&mut self, drawing_area: &DrawingArea);
    fn handle_increase_entity_length(&mut self, drawing_area: &DrawingArea);
    fn handle_decrease_entity_length(&mut self, drawing_area: &DrawingArea);
    fn set_tempo(&mut self, tempo: f64);
    fn set_snap_position_in_beats(&mut self, snap_position_in_beats: f64);
    fn set_new_entity_length_in_beats(&mut self, new_entity_length_in_beats: f64);
    fn set_entity_length_increment_in_beats(&mut self, entity_length_increment_in_beats: f64);
    fn snap_position_in_beats(&self) -> f64;
    fn entity_length_in_beats(&self) -> f64;
    fn entity_height_in_pixels(&self) -> f64;
    fn entity_length_increment_in_beats(&self) -> f64;
    fn custom_painter(&mut self) -> &mut Option<Box<dyn CustomPainter>>;
    fn beat_width_in_pixels(&self) -> f64;
    fn zoom_horizontal_in(&mut self);
    fn zoom_horizontal_out(&mut self);
    fn zoom_horizontal(&self) -> f64;
    fn zoom_vertical_in(&mut self);
    fn zoom_vertical_out(&mut self);
    fn zoom_vertical(&self) -> f64;
    fn set_horizontal_zoom(&mut self, zoom: f64);
    fn set_vertical_zoom(&mut self, zoom: f64);
    fn turn_on_draw_point_mode(&mut self);
    fn turn_on_draw_line_mode(&mut self);
    fn turn_on_draw_curve_mode(&mut self);
}

pub trait CustomPainter {
    fn paint_custom(&mut self, context: &Context, height: f64, width: f64, entity_height_in_pixels: f64,
                    beat_width_in_pixels: f64,
                    zoom_horizontal: f64,
                    zoom_vertical: f64,
                    select_window_top_left_x: f64, 
                    select_window_top_left_y: f64, 
                    select_window_bottom_right_x: f64, 
                    select_window_bottom_right_y: f64,
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
                    drawing_area: &DrawingArea,
                    operation_mode: &OperationModeType,
                    drag_started: bool,
                    edit_drag_cycle: &DragCycle,
                    tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
                );
    fn track_cursor_time_in_beats(&self) -> f64;
    fn set_track_cursor_time_in_beats(&mut self, track_cursor_time_in_beats: f64);

    fn as_any(&mut self) -> &mut dyn Any;
}

pub trait BeatGridMouseCoordHelper {
    fn get_entity_vertical_value(&self, y: f64, entity_height_in_pixels: f64, zoom_vertical: f64) -> f64;
    fn get_snapped_to_time(&self, snap: f64, time: f64) -> f64 {
        let mut snapped_to_time = time;
        snapped_to_time -= time % snap;
        snapped_to_time
    }

    fn get_time(&self, x: f64, beat_width_in_pixels: f64, zoom_horizontal: f64) -> f64 {
        x / (beat_width_in_pixels * zoom_horizontal)
    }
    fn select(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, x: f64, y: i32, x2: f64, y2: i32, add_to_select: bool);
    fn add_entity(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, y_index: i32, time: f64, duration: f64, entity_uuid: String);
    fn delete_entity(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, y_index: i32, time: f64, entity_uuid: String);
    fn cut_selected(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>);
    fn copy_selected(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>);
    fn paste_selected(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>);
    fn handle_translate_up(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>);
    fn handle_translate_down(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>);
    fn handle_translate_left(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>);
    fn handle_translate_right(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>);
    fn handle_quantise(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>);
    fn handle_increase_entity_length(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>);
    fn handle_decrease_entity_length(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>);
    fn set_start_note(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, y_index: i32, time: f64);
    fn set_riff_reference_play_mode(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, y_index: i32, time: f64);

}

pub struct PianoRollMouseCoordHelper;

impl BeatGridMouseCoordHelper for PianoRollMouseCoordHelper {
    fn get_entity_vertical_value(&self, y: f64, entity_height_in_pixels: f64, zoom_vertical: f64) -> f64 {
        ((127.0 * entity_height_in_pixels * zoom_vertical) - y) / (entity_height_in_pixels * zoom_vertical)
    }

    fn add_entity(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, y_index: i32, time: f64, duration: f64, _entity_uuid: String) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffAddNote(vec![(y_index, time, duration)]), None));
    }

    fn delete_entity(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, y_index: i32, time: f64, _entity_uuid: String) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffDeleteNote(y_index, time), None));
    }

    fn cut_selected(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffCutSelected, None));
    }

    fn copy_selected(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffCopySelected, None));
    }

    fn paste_selected(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffPasteSelected, None));
    }

    fn handle_translate_up(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffTranslateSelected(TranslationEntityType::Note, TranslateDirection::Up), None));
    }

    fn handle_translate_down(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffTranslateSelected(TranslationEntityType::Note, TranslateDirection::Down), None));
    }

    fn handle_translate_left(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffTranslateSelected(TranslationEntityType::Note, TranslateDirection::Left), None));
    }

    fn handle_translate_right(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffTranslateSelected(TranslationEntityType::Note, TranslateDirection::Right), None));
    }

    fn handle_quantise(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffQuantiseSelected, None));
    }

    fn handle_increase_entity_length(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffChangeLengthOfSelected(true), None));
    }

    fn handle_decrease_entity_length(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffChangeLengthOfSelected(false), None));
    }

    fn select(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, x: f64, y: i32, x2: f64, y2: i32, add_to_select: bool) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffEventsSelected(x, y2, x2, y, add_to_select), None));
    }

    fn set_start_note(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffSetStartNote(y_index, time), None));
    }

    fn set_riff_reference_play_mode(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffReferencePlayMode(y_index, time), None));
    }
}

pub struct SampleRollMouseCoordHelper;

impl BeatGridMouseCoordHelper for SampleRollMouseCoordHelper {
    fn get_entity_vertical_value(&self, y: f64, entity_height_in_pixels: f64, zoom_vertical: f64) -> f64 {
        ((127.0 * entity_height_in_pixels * zoom_vertical) - y) / (entity_height_in_pixels * zoom_vertical)
    }

    fn add_entity(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, _y_index: i32, time: f64, _duration: f64, entity_uuid: String) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffAddSample(entity_uuid, time), None));
    }

    fn delete_entity(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, _y_index: i32, time: f64, entity_uuid: String) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffDeleteSample(entity_uuid, time), None));
    }

    fn cut_selected(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffCutSelected, None));
    }

    fn copy_selected(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffCopySelected, None));
    }

    fn paste_selected(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffPasteSelected, None));
    }

    fn handle_translate_up(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffTranslateSelected(TranslationEntityType::Note, TranslateDirection::Up), None));
    }

    fn handle_translate_down(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffTranslateSelected(TranslationEntityType::Note, TranslateDirection::Down), None));
    }

    fn handle_translate_left(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffTranslateSelected(TranslationEntityType::Note, TranslateDirection::Left), None));
    }

    fn handle_translate_right(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffTranslateSelected(TranslationEntityType::Note, TranslateDirection::Right), None));
    }

    fn handle_quantise(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffQuantiseSelected, None));
    }

    fn handle_increase_entity_length(&self, _tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {

    }

    fn handle_decrease_entity_length(&self, _tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {

    }

    fn select(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, x: f64, y: i32, x2: f64, y2: i32, add_to_select: bool) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::AutomationSelected(x, y2, x2, y, add_to_select), None));
    }

    fn set_start_note(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64) {
    }

    fn set_riff_reference_play_mode(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64) {
    }
}

pub struct TrackGridMouseCoordHelper;

impl BeatGridMouseCoordHelper for TrackGridMouseCoordHelper {
    fn get_entity_vertical_value(&self, y: f64, entity_height_in_pixels: f64, zoom_vertical: f64) -> f64 {
        y / entity_height_in_pixels * zoom_vertical
    }

    fn add_entity(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, y_index: i32, time: f64, _duration: f64, _entity_uuid: String) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffReferenceAdd(y_index, time), None));
    }

    fn delete_entity(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, y_index: i32, time: f64, _entity_uuid: String) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffReferenceDelete(y_index, time), None));
    }

    fn cut_selected(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffReferenceCutSelected, None));
    }

    fn copy_selected(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffReferenceCopySelected, None));
    }

    fn paste_selected(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffReferencePaste, None));
    }

    fn handle_translate_up(&self, _tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {

    }

    fn handle_translate_down(&self, _tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {

    }

    fn handle_translate_left(&self, _tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {

    }

    fn handle_translate_right(&self, _tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {

    }

    fn handle_quantise(&self, _tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {

    }

    fn handle_increase_entity_length(&self, _tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
    }

    fn handle_decrease_entity_length(&self, _tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
    }

    fn select(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, x: f64, y: i32, x2: f64, y2: i32, add_to_select: bool) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffReferencesSelected(x, y, x2, y2, add_to_select), None));
    }

    fn set_start_note(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64) {
    }

    fn set_riff_reference_play_mode(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffReferencePlayMode(y_index, time), None));
    }
}

pub struct RiffGridMouseCoordHelper;

impl BeatGridMouseCoordHelper for RiffGridMouseCoordHelper {
    fn get_entity_vertical_value(&self, y: f64, entity_height_in_pixels: f64, zoom_vertical: f64) -> f64 {
        y / entity_height_in_pixels * zoom_vertical
    }

    fn add_entity(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, y_index: i32, time: f64, _duration: f64, _entity_uuid: String) {
        let _ = tx_from_ui.send(DAWEvents::RiffGridChange(RiffGridChangeType::RiffReferenceAdd{ track_index: y_index, position: time }, None));
    }

    fn delete_entity(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, y_index: i32, time: f64, _entity_uuid: String) {
        let _ = tx_from_ui.send(DAWEvents::RiffGridChange(RiffGridChangeType::RiffReferenceDelete{ track_index: y_index, position: time }, None));
    }

    fn cut_selected(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::RiffGridChange(RiffGridChangeType::RiffReferenceCutSelected, None));
    }

    fn copy_selected(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::RiffGridChange(RiffGridChangeType::RiffReferenceCopySelected, None));
    }

    fn paste_selected(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::RiffGridChange(RiffGridChangeType::RiffReferencePaste, None));
    }

    fn handle_translate_up(&self, _tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {

    }

    fn handle_translate_down(&self, _tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {

    }

    fn handle_translate_left(&self, _tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {

    }

    fn handle_translate_right(&self, _tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {

    }

    fn handle_quantise(&self, _tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {

    }

    fn handle_increase_entity_length(&self, _tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
    }

    fn handle_decrease_entity_length(&self, _tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
    }

    fn select(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, x: f64, y: i32, x2: f64, y2: i32, add_to_select: bool) {
        let _ = tx_from_ui.send(DAWEvents::RiffGridChange(RiffGridChangeType::RiffReferencesSelected(x, y, x2, y2, add_to_select), None));
    }

    fn set_start_note(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64) {
    }

    fn set_riff_reference_play_mode(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffReferencePlayMode(y_index, time), None));
    }
}

#[derive(Clone, Display)]
pub enum EditMode {
    Inactive,
    ChangeStart,
    Move,
    ChangeEnd,
}

#[derive(Clone)]
pub enum DragCycle {
    NotStarted,
    MousePressed,
    Dragging,
    MouseReleased,
    CtrlMousePressed,
    CtrlDragging,
    CtrlMouseReleased,
}

pub struct BeatGrid {
    entity_height_in_pixels: f64,
    beat_width_in_pixels: f64,

    // zoom
    zoom_horizontal: f64,
    zoom_vertical: f64,
    zoom_factor: f64,

    beats_per_bar: i32,

    custom_painter: Option<Box<dyn CustomPainter>>,

    operation_mode: OperationModeType,

    mouse_coord_helper: Option<Box<dyn BeatGridMouseCoordHelper>>,

    show_notes: bool,
    show_volume: bool,
    show_pan: bool,
    show_automation: bool,

	control_key_active: bool,
	shift_key_active: bool,
	alt_key_active: bool,

	drag_started: bool,

    snap_position_in_beats: f64,
    new_entity_length_in_beats: f64,
    entity_length_increment_in_beats: f64,
    tempo: f64,

	//selection window
    draw_selection_window: bool,
	x_selection_window_position: f64,
	y_selection_window_position: f64,
	x_selection_window_position2: f64,
	y_selection_window_position2: f64,

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

    tx_from_ui: crossbeam_channel::Sender<DAWEvents>,

    resize_drawing_area: bool,

    vertical_scale_painter: Option<Box<dyn CustomPainter>>,

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

    pub draw_play_cursor: bool,

    pub drawing_area_type: Option<DrawingAreaType>,
}

impl BeatGrid {
    pub fn new(zoom_horizontal: f64, zoom_vertical: f64, entity_height_in_pixels: f64, beat_width_in_pixels: f64, beats_per_bar: i32, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, drawing_area_type: Option<DrawingAreaType>) -> BeatGrid {
        BeatGrid {
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

            snap_position_in_beats: 1.0,
            new_entity_length_in_beats: 1.0,
            entity_length_increment_in_beats: 0.03125,
            tempo: 140.0,

            //selection window
            draw_selection_window: false,
            x_selection_window_position: 0.0,
            y_selection_window_position: 0.0,
            x_selection_window_position2: 0.0,
            y_selection_window_position2: 0.0,

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

            tx_from_ui,

            resize_drawing_area: true,

            vertical_scale_painter: None,

            draw_mode_on: false,
            draw_mode: DrawMode::Point,
            draw_mode_x_start: 0.0,
            draw_mode_y_start: 0.0,
            draw_mode_x_end: 0.0,
            draw_mode_y_end: 0.0,

            edit_drag_cycle: DragCycle::NotStarted,
            select_drag_cycle: DragCycle::NotStarted,

            draw_play_cursor: true,

            drawing_area_type,
        }
    }

    pub fn new_with_custom(
        zoom_horizontal: f64, 
        zoom_vertical: f64,
        entity_height_in_pixels: f64,
        beat_width_in_pixels: f64,
        beats_per_bar: i32,
        custom_painter: Option<Box<dyn CustomPainter>>,
        mouse_coord_helper: Option<Box<dyn BeatGridMouseCoordHelper>>,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        resize_drawing_area: bool,
        drawing_area_type: Option<DrawingAreaType>,
    ) -> BeatGrid {
        BeatGrid {
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

            snap_position_in_beats: 1.0,
            new_entity_length_in_beats: 1.0,
            entity_length_increment_in_beats: 0.03125,
            tempo: 140.0,

            //selection window
            draw_selection_window: false,
            x_selection_window_position: 0.0,
            y_selection_window_position: 0.0,
            x_selection_window_position2: 0.0,
            y_selection_window_position2: 0.0,

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

            tx_from_ui,

            resize_drawing_area,

            vertical_scale_painter: None,

            draw_mode_on: false,
            draw_mode: DrawMode::Point,
            draw_mode_x_start: 0.0,
            draw_mode_y_start: 0.0,
            draw_mode_x_end: 0.0,
            draw_mode_y_end: 0.0,

            edit_drag_cycle: DragCycle::NotStarted,
            select_drag_cycle: DragCycle::NotStarted,

            draw_play_cursor: true,

            drawing_area_type,
        }
    }

    pub fn new_with_painters(
        zoom_horizontal: f64, 
        zoom_vertical: f64,
        entity_height_in_pixels: f64,
        beat_width_in_pixels: f64,
        beats_per_bar: i32,
        custom_painter: Option<Box<dyn CustomPainter>>,
        vertical_scale_painter: Option<Box<dyn CustomPainter>>,
        mouse_coord_helper: Option<Box<dyn BeatGridMouseCoordHelper>>,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        resize_drawing_area: bool,
        drawing_area_type: Option<DrawingAreaType>,
    ) -> BeatGrid {
        BeatGrid {
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

            snap_position_in_beats: 1.0,
            new_entity_length_in_beats: 1.0,
            entity_length_increment_in_beats: 0.03125,
            tempo: 140.0,

            //selection window
            draw_selection_window: false,
            x_selection_window_position: 0.0,
            y_selection_window_position: 0.0,
            x_selection_window_position2: 0.0,
            y_selection_window_position2: 0.0,

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

            tx_from_ui,

            resize_drawing_area,

            vertical_scale_painter,

            draw_mode_on: false,
            draw_mode: DrawMode::Point,
            draw_mode_x_start: 0.0,
            draw_mode_y_start: 0.0,
            draw_mode_x_end: 0.0,
            draw_mode_y_end: 0.0,

            edit_drag_cycle: DragCycle::NotStarted,
            select_drag_cycle: DragCycle::NotStarted,

            draw_play_cursor: true,

            drawing_area_type,
        }
    }

    pub fn set_operation_mode(&mut self, operation_mode: OperationModeType) {
        self.operation_mode = operation_mode;
    }

    pub fn add_entity(&mut self) {
    }

    pub fn delete_entity(&mut self) {
    }

    pub fn change_entity(&mut self) {
    }

    pub fn delete_selected(&mut self) {
    }

    pub fn copy_selected(&mut self) {
    }

    pub fn paste_selected(&mut self) {
    }

    pub fn translate_entities_vertically(_amount: i32) {

    }

    pub fn translate_entities_horizontally(_amount: i32) {

    }

    pub fn translate_horizontal(_amount: i32) {

    }

    pub fn translate_vertical(_amount: i32) {

    }

    pub fn snap_changed(_value: i32) {

    }

    pub fn snap_length_changed(_value: i32) {

    }

    pub fn copy_changed() {

    }

    pub fn cut_changed() {

    }

    pub fn paste_changed() {

    }

    pub fn quantize_selected() {

    }

    pub fn undo() {

    }

    pub fn redo() {

    }

    pub fn timer_callback() {

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

    pub fn get_select_window(&self) -> (f64, f64, f64, f64) {
        // find the top left x and y
        let top_left_x = self.x_selection_window_position.min(self.x_selection_window_position2);
        let top_left_y = self.y_selection_window_position.min(self.y_selection_window_position2);

        // find the bottom right x and y
        let bottom_right_x = self.x_selection_window_position.max(self.x_selection_window_position2);
        let bottom_right_y = self.y_selection_window_position.max(self.y_selection_window_position2);

        // debug!("Window select: top_left_x={}, top_left_y={}, bottom_right_x={}, bottom_right_y={}", top_left_x, top_left_y, bottom_right_x, bottom_right_y);

        (top_left_x, top_left_y, bottom_right_x, bottom_right_y)
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

    pub fn drag_started(&self) -> bool {
        self.drag_started
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
}

impl MouseHandler for BeatGrid {

    fn handle_mouse_motion(&mut self, x: f64, y: f64, drawing_area: &DrawingArea, mouse_button: MouseButton, control_key: bool, shift_key: bool, alt_key: bool) {
        self.control_key_active = control_key;
        self.shift_key_active = shift_key;
        self.alt_key_active = alt_key;
        self.mouse_pointer_position = (x, y);

        // debug!("Mouse motion: x={}, y={}", x, y);

        match mouse_button {
            MouseButton::Button1 => {
                match self.operation_mode {
                    OperationModeType::PointMode => {
                        self.x_selection_window_position2 = x;
                        self.y_selection_window_position2 = y;
                        self.draw_selection_window = true;
                        self.select_drag_cycle = DragCycle::Dragging;
                        drawing_area.queue_draw();
                    },
                    OperationModeType::Add => {
                        self.draw_mode_x_end = x;
                        self.draw_mode_y_end = y;
                        drawing_area.queue_draw();
                    },
                    OperationModeType::LoopPointMode => {

                    },
                    OperationModeType::Change => {
                        if control_key {
                            debug!("Mouse motion: changed to EditDragCycle::CtrlDragging");
                            self.edit_drag_cycle = DragCycle::CtrlDragging;
                        }
                        else {
                            debug!("Mouse motion: changed to EditDragCycle::Dragging");
                            self.edit_drag_cycle = DragCycle::Dragging;
                        }
                        drawing_area.queue_draw();
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

    fn handle_mouse_press(&mut self, x: f64, y: f64, drawing_area: &DrawingArea, mouse_button: MouseButton, control_key: bool, shift_key: bool, alt_key: bool) {
        self.control_key_active = control_key;
        self.shift_key_active = shift_key;
        self.alt_key_active = alt_key;

        match mouse_button {
            MouseButton::Button1 => {
                match self.operation_mode {
                    OperationModeType::PointMode => {
                        self.x_selection_window_position = x;
                        self.y_selection_window_position = y;
                        self.select_drag_cycle = DragCycle::MousePressed;
                    }
                    OperationModeType::Change => {
                        if control_key {
                            debug!("Mouse pressed: changed to EditDragCycle::CtrlMousePressed");
                            self.edit_drag_cycle = DragCycle::CtrlMousePressed;
                        }
                        else {
                            debug!("Mouse pressed: changed to EditDragCycle::MousePressed");
                            self.edit_drag_cycle = DragCycle::MousePressed;
                        }
                        self.mouse_pointer_previous_position = (x, y);
                        drawing_area.queue_draw();
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

    fn handle_mouse_release(&mut self, x: f64, y: f64, drawing_area: &DrawingArea, mouse_button: MouseButton, control_key: bool, shift_key: bool, alt_key: bool, data: String) {
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
                                    let snap_position = mouse_coord_helper.get_snapped_to_time(self.snap_position_in_beats, position);
                                    let duration = self.new_entity_length_in_beats - 0.01; // take off just a little off so that the note off does not overlap the next note on

                                    mouse_coord_helper.add_entity(self.tx_from_ui.clone(), y_index as i32, snap_position, duration, data);
                                }
                                else if let DrawMode::Line = self.draw_mode {
                                    let x_start_position = mouse_coord_helper.get_time(self.draw_mode_x_start, self.beat_width_in_pixels, self.zoom_horizontal);
                                    let y_start_index = mouse_coord_helper.get_entity_vertical_value(self.draw_mode_y_start, self.entity_height_in_pixels, self.zoom_vertical);
                                    let x_end_position = mouse_coord_helper.get_time(self.draw_mode_x_end, self.beat_width_in_pixels, self.zoom_horizontal);
                                    let y_end_index = mouse_coord_helper.get_entity_vertical_value(self.draw_mode_y_end, self.entity_height_in_pixels, self.zoom_vertical);
                                    let snap_position_start = mouse_coord_helper.get_snapped_to_time(self.snap_position_in_beats, x_start_position);
                                    let snap_position_end = mouse_coord_helper.get_snapped_to_time(self.snap_position_in_beats, x_end_position);

                                    let mut position = snap_position_start;
                                    let mut y_start = y_start_index;
                                    let mut number_of_events = 0;
                                    while position <= snap_position_end {
                                        position += self.snap_position_in_beats;
                                        number_of_events += 1;
                                    }

                                    let y_increment = (y_end_index - y_start_index) / (number_of_events - 1) as f64;
                                    position = snap_position_start;
                                    y_start = y_start_index;
                                    while position <= snap_position_end {
                                        mouse_coord_helper.add_entity(self.tx_from_ui.clone(), y_start as i32, position, 0.0, data.clone());
                                        position += self.snap_position_in_beats;
                                        y_start += y_increment;
                                    }
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
                                mouse_coord_helper.delete_entity(self.tx_from_ui.clone(), y_index as i32, position, data);
                            },
                            None => (),
                        }
                    },
                    OperationModeType::Change => {
                        if (control_key) {
                            self.edit_drag_cycle = DragCycle::CtrlMouseReleased;
                            debug!("Mouse release: changed to EditDragCycle::CtrlMouseReleased");
                        }
                        else {
                            self.edit_drag_cycle = DragCycle::MouseReleased;
                            debug!("Mouse release: changed to EditDragCycle::MouseReleased");
                        }
                        drawing_area.queue_draw();
                    },
                    OperationModeType::PointMode => {
                        self.draw_selection_window = false;
                        if drawing_area.widget_name().to_string().starts_with("riffset_") {
                            let widget_name = drawing_area.widget_name().to_string();
                            let segments = widget_name.split('_').collect_vec();
                            if segments.len() == 3 {
                                let riff_set_uuid = *segments.get(1).unwrap();
                                let track_uuid = *segments.get(2).unwrap();
                                match self.tx_from_ui.send(DAWEvents::RiffSetTrackIncrementRiff(riff_set_uuid.to_owned(), track_uuid.to_owned())) {
                                    Ok(_) => (),
                                    Err(_) => (),
                                }
                            }
                            else {
                                debug!("Not enough elements to extract riff set and track uuids.")
                            }
                        }
                        else if let DragCycle::Dragging = self.select_drag_cycle {
                            self.select_drag_cycle = DragCycle::NotStarted;
                            // send an event to the ui via the mouse coord helper
                            if let Some(mouse_coord_helper) = self.mouse_coord_helper.as_ref() {
                                let select_window = self.get_select_window();
                                let add_to_select = control_key;
                                mouse_coord_helper.select(
                                    self.tx_from_ui.clone(), 
                                    mouse_coord_helper.get_time(select_window.0, self.beat_width_in_pixels, self.zoom_horizontal), 
                                    mouse_coord_helper.get_entity_vertical_value(select_window.1, self.entity_height_in_pixels, self.zoom_vertical) as i32, 
                                    mouse_coord_helper.get_time(select_window.2, self.beat_width_in_pixels, self.zoom_horizontal), 
                                    mouse_coord_helper.get_entity_vertical_value(select_window.3, self.entity_height_in_pixels, self.zoom_vertical) as i32,
                                    add_to_select
                                );
                            }
                        }
                        drawing_area.queue_draw();
                    },
                    OperationModeType::LoopPointMode => {
                        match &self.mouse_coord_helper {
                            Some(mouse_coord_helper) => {
                                let position = mouse_coord_helper.get_time(x, self.beat_width_in_pixels, self.zoom_horizontal);
                                let snap_position = mouse_coord_helper.get_snapped_to_time(self.snap_position_in_beats, position);
                                match self.tx_from_ui.send(DAWEvents::LoopChange(LoopChangeType::LoopLimitLeftChanged(snap_position), Uuid::new_v4())) {
                                    Ok(_) => (),
                                    Err(error) => debug!("Problem setting loop left - could sender event: {}", error),
                                }
                            },
                            None => (),
                        }
                    },
                    OperationModeType::DeleteSelected => {
                    },
                    OperationModeType::CopySelected => {

                    },
                    OperationModeType::PasteSelected => {

                    },
                    OperationModeType::SelectAll => {

                    },
                    OperationModeType::DeselectAll => {

                    },
                    OperationModeType::Undo => {

                    },
                    OperationModeType::Redo => {

                    },
                    OperationModeType::SelectStartNote => {
                        match &self.mouse_coord_helper {
                            Some(mouse_coord_helper) => {
                                let y_index = mouse_coord_helper.get_entity_vertical_value(y, self.entity_height_in_pixels, self.zoom_vertical);
                                let position = mouse_coord_helper.get_time(x, self.beat_width_in_pixels, self.zoom_horizontal);
                                mouse_coord_helper.set_start_note(self.tx_from_ui.clone(), y_index as i32, position);
                            },
                            None => (),
                        }
                    }
                    OperationModeType::SelectRiffReferenceMode => {
                        match &self.mouse_coord_helper {
                            Some(mouse_coord_helper) => {
                                let y_index = mouse_coord_helper.get_entity_vertical_value(y, self.entity_height_in_pixels, self.zoom_vertical);
                                let position = mouse_coord_helper.get_time(x, self.beat_width_in_pixels, self.zoom_horizontal);
                                mouse_coord_helper.set_riff_reference_play_mode(self.tx_from_ui.clone(), y_index as i32, position);
                            },
                            None => (),
                        }
                    }
                }
            },
            MouseButton::Button2 => {
                match self.operation_mode {
                    OperationModeType::Add => debug!("mouse button clicked=2, mode={:?}", self.operation_mode),
                    OperationModeType::Delete => debug!("mouse button clicked=2, mode={:?}", self.operation_mode),
                    OperationModeType::Change => debug!("mouse button clicked=2, mode={:?}", self.operation_mode),
                    OperationModeType::PointMode => {
                        if drawing_area.widget_name().to_string().starts_with("riffset_") {
                            let widget_name = drawing_area.widget_name().to_string();
                            let segments = widget_name.split('_').collect_vec();
                            if segments.len() == 3 {
                                let riff_set_uuid = *segments.get(1).unwrap();
                                let track_uuid = *segments.get(2).unwrap();
                                let new_riff_uuid = Uuid::new_v4();
                                match self.tx_from_ui.send(
                                    DAWEvents::TrackChange(TrackChangeType::RiffAdd(new_riff_uuid, "".to_string(), 4.0), Some(track_uuid.to_owned()))) {
                                    Ok(_) => (),
                                    Err(_) => (),
                                }
                                match self.tx_from_ui.send(
                                    DAWEvents::RiffSetTrackSetRiff(riff_set_uuid.to_string(), track_uuid.to_string(), new_riff_uuid.to_string())) {
                                    Ok(_) => (),
                                    Err(_) => (),
                                }
                            }
                            else {
                                debug!("Not enough elements to extract riff set and track uuids.")
                            }
                        }

                    },
                    OperationModeType::LoopPointMode => debug!("mouse button clicked=2, mode={:?}", self.operation_mode),
                    OperationModeType::DeleteSelected => debug!("mouse button clicked=2, mode={:?}", self.operation_mode),
                    OperationModeType::CopySelected => debug!("mouse button clicked=2, mode={:?}", self.operation_mode),
                    OperationModeType::PasteSelected => debug!("mouse button clicked=2, mode={:?}", self.operation_mode),
                    OperationModeType::SelectAll => debug!("mouse button clicked=2, mode={:?}", self.operation_mode),
                    OperationModeType::DeselectAll => debug!("mouse button clicked=2, mode={:?}", self.operation_mode),
                    OperationModeType::Undo => debug!("mouse button clicked=2, mode={:?}", self.operation_mode),
                    OperationModeType::Redo => debug!("mouse button clicked=2, mode={:?}", self.operation_mode),
                    OperationModeType::SelectStartNote => {}
                    OperationModeType::SelectRiffReferenceMode => {}
                }
            },
            MouseButton::Button3 => {
                match self.operation_mode {
                    OperationModeType::Add => debug!("mouse button clicked=3, mode={:?}", self.operation_mode),
                    OperationModeType::Delete => debug!("mouse button clicked=3, mode={:?}", self.operation_mode),
                    OperationModeType::Change => debug!("mouse button clicked=3, mode={:?}", self.operation_mode),
                    OperationModeType::PointMode => {
                        if drawing_area.widget_name().to_string().starts_with("riffset_") {
                            let widget_name = drawing_area.widget_name().to_string();
                            let segments = widget_name.split('_').collect_vec();
                            if segments.len() == 3 {
                                let riff_set_uuid = *segments.get(1).unwrap();
                                let track_uuid = *segments.get(2).unwrap();
                                match self.tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffSelect(riff_set_uuid.to_owned()), Some(track_uuid.to_owned()))) {
                                    Ok(_) => (),
                                    Err(_) => (),
                                }
                            }
                            else {
                                debug!("Not enough elements to extract riff set and track uuids.")
                            }
                        }
                        else if shift_key {
                            match &self.mouse_coord_helper {
                                Some(mouse_coord_helper) => {
                                    let position = mouse_coord_helper.get_time(x, self.beat_width_in_pixels, self.zoom_horizontal);
                                    let snap_position = mouse_coord_helper.get_snapped_to_time(self.snap_position_in_beats, position);
                                    self.track_cursor_time_in_beats = snap_position;
                                    let _ = self.tx_from_ui.send(DAWEvents::PlayPositionInBeats(snap_position));
                                    if let Some(drawing_area_type) = &self.drawing_area_type {
                                        match drawing_area_type {
                                            DrawingAreaType::PianoRoll => {
                                                let _ = self.tx_from_ui.send(DAWEvents::RepaintPianoRollView);
                                            }
                                            DrawingAreaType::TrackGrid => {
                                                let _ = self.tx_from_ui.send(DAWEvents::RepaintTrackGridView);
                                            }
                                            DrawingAreaType::Automation => {
                                                let _ = self.tx_from_ui.send(DAWEvents::RepaintAutomationView);
                                            }
                                            _ => {}
                                        }
                                    }
                                },
                                None => (),
                            }
                        }
                        else if control_key {
                            match &self.mouse_coord_helper {
                                Some(mouse_coord_helper) => {
                                    let position = mouse_coord_helper.get_time(x, self.beat_width_in_pixels, self.zoom_horizontal);
                                    let snap_position = mouse_coord_helper.get_snapped_to_time(self.snap_position_in_beats, position);
                                    self.edit_cursor_time_in_beats = snap_position;
                                    if let Some(drawing_area_type) = &self.drawing_area_type {
                                        match drawing_area_type {
                                            DrawingAreaType::PianoRoll => {
                                                let _ = self.tx_from_ui.send(DAWEvents::RepaintPianoRollView);
                                            }
                                            DrawingAreaType::TrackGrid => {
                                                let _ = self.tx_from_ui.send(DAWEvents::RepaintTrackGridView);
                                            }
                                            DrawingAreaType::Automation => {
                                                let _ = self.tx_from_ui.send(DAWEvents::RepaintAutomationView);
                                            }
                                            _ => {}
                                        }
                                    }
                                },
                                None => (),
                            }
                        }
                    },
                    OperationModeType::LoopPointMode =>                         match &self.mouse_coord_helper {
                        Some(mouse_coord_helper) => {
                            let position = mouse_coord_helper.get_time(x, self.beat_width_in_pixels, self.zoom_horizontal);
                            let snap_position = mouse_coord_helper.get_snapped_to_time(self.snap_position_in_beats, position);
                            match self.tx_from_ui.send(DAWEvents::LoopChange(LoopChangeType::LoopLimitRightChanged(snap_position), Uuid::new_v4())) {
                                Ok(_) => (),
                                Err(error) => debug!("Problem setting loop left - could sender event: {}", error),
                            }
                        },
                        None => (),
                    },
                    OperationModeType::DeleteSelected => debug!("mouse button clicked=3, mode={:?}", self.operation_mode),
                    OperationModeType::CopySelected => debug!("mouse button clicked=3, mode={:?}", self.operation_mode),
                    OperationModeType::PasteSelected => debug!("mouse button clicked=3, mode={:?}", self.operation_mode),
                    OperationModeType::SelectAll => debug!("mouse button clicked=3, mode={:?}", self.operation_mode),
                    OperationModeType::DeselectAll => debug!("mouse button clicked=3, mode={:?}", self.operation_mode),
                    OperationModeType::Undo => debug!("mouse button clicked=3, mode={:?}", self.operation_mode),
                    OperationModeType::Redo => debug!("mouse button clicked=3, mode={:?}", self.operation_mode),
                    OperationModeType::SelectStartNote => {}
                    OperationModeType::SelectRiffReferenceMode => {}
                }
            },
        }
    }
}

impl Grid for BeatGrid {
    fn paint(&mut self, context: &Context, drawing_area: &DrawingArea) {
        let (clip_x1, clip_y1, clip_x2, clip_y2) = context.clip_extents().unwrap();
        
        // debug!("painting beatgrid - {} is visible={}, x={}, y={}, width={}, height={} - clip_x1={}, clip_y1={}, clip_x2={}, clip_y2={}",
        //     drawing_area.widget_name(), 
        //     drawing_area.is_visible(), 
        //     drawing_area.allocation().x,
        //     drawing_area.allocation().y,
        //     drawing_area.allocation().width,
        //     drawing_area.allocation().height,
        //     clip_x1, 
        //     clip_y1, 
        //     clip_x2, 
        //     clip_y2);

        // if self.resize_drawing_area {
        //     drawing_area.set_width_request((self.beat_width_in_pixels * self.zoom) as i32 * 400 * 4);
        // }
        context.set_source_rgb(1.0, 1.0, 1.0);
        context.rectangle(clip_x1, clip_y1, clip_x2 - clip_x1, clip_y2 - clip_y1);
        let _ = context.fill();

        let height = drawing_area.height_request() as f64;
        let width = drawing_area.width_request() as f64;

        if let Some(window) = drawing_area.window() {
            window.set_cursor(Some(&gdk::Cursor::for_display(&window.display(), gdk::CursorType::Cross)));
        }

        self.paint_vertical_scale(context, height, width, drawing_area);
        self.paint_horizontal_scale(context, height, width);
        self.paint_custom(context, height, width, drawing_area.widget_name().to_string(), drawing_area);
        self.paint_loop_markers(context, height, width);
        if self.draw_selection_window {
            self.paint_select_window(context, height, width);
        }
        if self.draw_play_cursor {
            self.paint_play_cursor(context, height, width);
        }
        self.paint_edit_cursor(context, height, width);
    }

    fn paint_vertical_scale(&mut self, context: &Context, height: f64, width: f64, drawing_area: &DrawingArea) {
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
        let tx_from_ui = self.tx_from_ui.clone();
        let (clip_x1, clip_y1, clip_x2, clip_y2) = context.clip_extents().unwrap();

        if let Some(vertical_scale_painter) = self.vertical_scale_painter_mut() {
            vertical_scale_painter.paint_custom(context, 
                                                height, 
                                                width, 
                                                entity_height_in_pixels,
                                                beat_width_in_pixels,
                                                zoom_horizontal,
                                                zoom_vertical,
                                                x_selection_window_position,
                                                y_selection_window_position,
                                                x_selection_window_position2,
                                                x_selection_window_position2,
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
                                                drawing_area,
                                                &operation_mode,
                                                false,
                                                &edit_drag_cycle,
                                                tx_from_ui,
                                            );
        }
        else {
            context.set_source_rgba(0.9, 0.9, 0.9, 0.5);
            let adjusted_entity_height_in_pixels = self.entity_height_in_pixels * self.zoom_vertical;

            let mut current_y = if adjusted_entity_height_in_pixels >= 1.0 {
                clip_y1 - (clip_y1 as i32 % adjusted_entity_height_in_pixels as i32) as f64
            }
            else {
                1.0
            };
            while current_y < clip_y2 {
                let row_number = current_y / adjusted_entity_height_in_pixels;

                if row_number as i32 % 2 == 0 {
                    context.rectangle(clip_x1, current_y, clip_x2 - clip_x1, adjusted_entity_height_in_pixels);
                    let _ = context.fill();
                }

                current_y += adjusted_entity_height_in_pixels;
            }
        }
    }

    fn paint_horizontal_scale(&mut self, context: &Context, height: f64, width: f64) {
        let adjusted_beat_width_in_pixels = self.beat_width_in_pixels * self.zoom_horizontal;
        let (clip_x1, clip_y1, clip_x2, clip_y2) = context.clip_extents().unwrap();
        let clip_x1_in_beats = clip_x1 / adjusted_beat_width_in_pixels;
        let mut current_x = clip_x1_in_beats.floor() * adjusted_beat_width_in_pixels; // go to the first beat to the left of the view port e.g. bar 2 beat 3 = beat 2 * 4 + 3 = beat 11
        let mut beat_in_bar_index = (clip_x1_in_beats as i32 % 4) + 1;

        while current_x < clip_x2 {
            if beat_in_bar_index == 1 {
                context.set_source_rgba(0.5, 0.5, 0.5, 1.0);
            }
            else {
                context.set_source_rgba(0.5, 0.5, 0.5, 0.5);
            }

            context.move_to(current_x, clip_y1);
            context.line_to(current_x, clip_y2);
            context.set_line_width(0.3);
            let _ = context.stroke();
            current_x += adjusted_beat_width_in_pixels;

            if beat_in_bar_index == 4 {
                beat_in_bar_index = 1;
            }
            else {
                beat_in_bar_index += 1;
            }
        }
    }

    fn paint_custom(&mut self, context: &Context, height: f64, width: f64, drawing_area_widget_name: String, drawing_area: &DrawingArea) {
        let (x, y) = self.mouse_pointer_position;
        let (x_previous, y_previous) = self.mouse_pointer_previous_position;

        let (x_selection_window_position, y_selection_window_position, x_selection_window_position2, y_selection_window_position2) = self.get_select_window();
        if let Some(custom_painter) = self.custom_painter.as_mut() {
            custom_painter.set_track_cursor_time_in_beats(self.track_cursor_time_in_beats);
            custom_painter.paint_custom(
                context,
                height,
                width,
                self.entity_height_in_pixels,
                self.beat_width_in_pixels,
                self.zoom_horizontal,
                self.zoom_vertical,
                x_selection_window_position,
                y_selection_window_position,
                x_selection_window_position2,
                y_selection_window_position2,
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
                drawing_area,
                &self.operation_mode,
                self.drag_started,
                &self.edit_drag_cycle.clone(),
                self.tx_from_ui.clone(),
            );

            match &self.edit_drag_cycle {
                DragCycle::MouseReleased => self.edit_drag_cycle = DragCycle::NotStarted,
                _ => {}
            }
        }
    }

    fn paint_select_window(&mut self, context: &Context, _height: f64, _width: f64) {
        context.set_source_rgba(0.0, 0.0, 1.0, 0.5);
        let (top_left_x, top_left_y, bottom_right_x, bottom_right_y) = self.get_select_window();
        context.rectangle(top_left_x, top_left_y, bottom_right_x - top_left_x, bottom_right_y - top_left_y);
        let _ = context.fill();
    }

    fn paint_loop_markers(&mut self, _context: &Context, _height: f64, _width: f64) {
    }

    fn paint_play_cursor(&mut self, context: &Context, height: f64, _width: f64) {
        let adjusted_beat_width_in_pixels = self.beat_width_in_pixels * self.zoom_horizontal;
        let x = self.track_cursor_time_in_beats * adjusted_beat_width_in_pixels;
        let (_, clip_y1, _, clip_y2) = context.clip_extents().unwrap();

        context.set_source_rgba(0.0, 0.0, 1.0, 1.0);
        context.move_to(x, 0.0);
        context.line_to(x, clip_y2 - clip_y1);
        let _ = context.stroke();
    }

    fn paint_edit_cursor(&mut self, context: &Context, height: f64, _width: f64) {
        let adjusted_beat_width_in_pixels = self.beat_width_in_pixels * self.zoom_horizontal;
        let x = self.edit_cursor_time_in_beats * adjusted_beat_width_in_pixels;

        context.set_source_rgba(1.0, 0.0, 1.0, 1.0);
        context.move_to(x, 0.0);
        context.line_to(x, height);
        let _ = context.stroke();
    }

    fn handle_cut(&mut self, _drawing_area: &DrawingArea) {
        match self.operation_mode {
            OperationModeType::PointMode => {
                match &self.mouse_coord_helper {
                    Some(mouse_coord_helper) => {
                        mouse_coord_helper.cut_selected(self.tx_from_ui.clone());
                    },
                    None => (),
                }
            },
            _ => (),
        }
    }

    fn handle_copy(&mut self, _drawing_area: &DrawingArea) {
        match self.operation_mode {
            OperationModeType::PointMode => {
                match &self.mouse_coord_helper {
                    Some(mouse_coord_helper) => {
                        mouse_coord_helper.copy_selected(self.tx_from_ui.clone());
                    },
                    None => (),
                }
            },
            _ => (),
        }
    }

    fn handle_paste(&mut self, _drawing_area: &DrawingArea) {
        match self.operation_mode {
            OperationModeType::PointMode => {
                match &self.mouse_coord_helper {
                    Some(mouse_coord_helper) => {
                        mouse_coord_helper.paste_selected(self.tx_from_ui.clone());
                    },
                    None => (),
                }
            },
            _ => (),
        }
    }

    fn handle_translate_up(&mut self, _drawing_area: &DrawingArea) {
        match self.operation_mode {
            OperationModeType::PointMode => {
                match &self.mouse_coord_helper {
                    Some(mouse_coord_helper) => {
                        mouse_coord_helper.handle_translate_up(self.tx_from_ui.clone());
                    },
                    None => (),
                }
            },
            _ => (),
        }
    }

    fn handle_translate_down(&mut self, _drawing_area: &DrawingArea) {
        match self.operation_mode {
            OperationModeType::PointMode => {
                match &self.mouse_coord_helper {
                    Some(mouse_coord_helper) => {
                        mouse_coord_helper.handle_translate_down(self.tx_from_ui.clone());
                    },
                    None => (),
                }
            },
            _ => (),
        }
    }

    fn handle_translate_left(&mut self, _drawing_area: &DrawingArea) {
        match self.operation_mode {
            OperationModeType::PointMode => {
                match &self.mouse_coord_helper {
                    Some(mouse_coord_helper) => {
                        mouse_coord_helper.handle_translate_left(self.tx_from_ui.clone());
                    },
                    None => (),
                }
            },
            _ => (),
        }
    }

    fn handle_translate_right(&mut self, _drawing_area: &DrawingArea) {
        match self.operation_mode {
            OperationModeType::PointMode => {
                match &self.mouse_coord_helper {
                    Some(mouse_coord_helper) => {
                        mouse_coord_helper.handle_translate_right(self.tx_from_ui.clone());
                    },
                    None => (),
                }
            },
            _ => (),
        }
    }

    fn handle_quantise(&mut self, _drawing_area: &DrawingArea) {
        match self.operation_mode {
            OperationModeType::PointMode => {
                match &self.mouse_coord_helper {
                    Some(mouse_coord_helper) => {
                        mouse_coord_helper.handle_quantise(self.tx_from_ui.clone());
                    },
                    None => (),
                }
            },
            _ => (),
        }
    }

    fn handle_increase_entity_length(&mut self, _drawing_area: &DrawingArea) {
        match self.operation_mode {
            OperationModeType::PointMode => {
                match &self.mouse_coord_helper {
                    Some(mouse_coord_helper) => {
                        mouse_coord_helper.handle_increase_entity_length(self.tx_from_ui.clone());
                    },
                    None => (),
                }
            },
            _ => (),
        }
    }

    fn handle_decrease_entity_length(&mut self, _drawing_area: &DrawingArea) {
        match self.operation_mode {
            OperationModeType::PointMode => {
                match &self.mouse_coord_helper {
                    Some(mouse_coord_helper) => {
                        mouse_coord_helper.handle_decrease_entity_length(self.tx_from_ui.clone());
                    },
                    None => (),
                }
            },
            _ => (),
        }
    }

    fn zoom_horizontal_in(&mut self) {
        if self.zoom_horizontal < 7.0 {
            self.zoom_horizontal += self.zoom_factor;
        }
    }

    fn zoom_horizontal_out(&mut self) {
        if self.zoom_horizontal > (self.zoom_factor * 2.0) {
            self.zoom_horizontal -= self.zoom_factor;
        }
    }

    fn set_tempo(&mut self, tempo: f64) {
        self.tempo = tempo;
    }

    fn set_snap_position_in_beats(&mut self, snap_position_in_beats: f64) {
        self.snap_position_in_beats = snap_position_in_beats;
    }

    fn set_new_entity_length_in_beats(&mut self, new_entity_length_in_beats: f64) {
        self.new_entity_length_in_beats = new_entity_length_in_beats;
    }

    fn set_entity_length_increment_in_beats(&mut self, entity_length_increment_in_beats: f64) {
        self.entity_length_increment_in_beats = entity_length_increment_in_beats;
    }

    fn snap_position_in_beats(&self) -> f64 {
        self.snap_position_in_beats
    }

    fn entity_length_in_beats(&self) -> f64 {
        self.new_entity_length_in_beats
    }

    fn entity_length_increment_in_beats(&self) -> f64 {
        self.entity_length_increment_in_beats
    }

    fn custom_painter(&mut self) -> &mut Option<Box<dyn CustomPainter>> {
        &mut self.custom_painter
    }

    fn beat_width_in_pixels(&self) -> f64 {
        self.beat_width_in_pixels
    }

    fn zoom_horizontal(&self) -> f64 {
        self.zoom_horizontal
    }

    fn turn_on_draw_point_mode(&mut self) {
        self.draw_mode = DrawMode::Point
    }

    fn turn_on_draw_line_mode(&mut self) {
        self.draw_mode = DrawMode::Line
    }

    fn turn_on_draw_curve_mode(&mut self) {
        self.draw_mode = DrawMode::Curve
    }

    fn zoom_vertical(&self) -> f64 {
        self.zoom_vertical
    }

    fn zoom_vertical_in(&mut self) {
        if self.zoom_vertical < 7.0 {
            self.zoom_vertical += self.zoom_factor;
        }
    }

    fn zoom_vertical_out(&mut self) {
        if self.zoom_vertical > (self.zoom_factor * 2.0) {
            self.zoom_vertical -= self.zoom_factor;
        }
    }

    fn set_horizontal_zoom(&mut self, zoom: f64) {
        // debug!("Horiz. zoom: {}", zoom);
        self.zoom_horizontal = zoom;
    }

    fn set_vertical_zoom(&mut self, zoom: f64) {
        self.zoom_vertical = zoom;
    }

    fn entity_height_in_pixels(&self) -> f64 {
        self.entity_height_in_pixels
    }
}

pub struct BeatGridRuler {
    beat_width_in_pixels: f64,
    zoom_horizontal: f64,
    zoom_vertical: f64,
    zoom_factor: f64,
    beats_per_bar: i32,
    tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
    custom_painter: Option<Box<dyn CustomPainter>>,
}

impl BeatGridRuler {
    pub fn new(zoom: f64, beat_width_in_pixels: f64, beats_per_bar: i32, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) -> Self {
        Self {
            beat_width_in_pixels,
            zoom_horizontal: zoom,
            zoom_vertical: zoom,
            zoom_factor: 0.01,
            beats_per_bar,
            tx_from_ui,
            custom_painter: None,
        }
    }

    pub fn new_with_individual_zoom_level(zoom_horizontal: f64, zoom_vertical: f64, beat_width_in_pixels: f64, beats_per_bar: i32, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) -> Self {
        Self {
            beat_width_in_pixels,
            zoom_horizontal,
            zoom_vertical,
            zoom_factor: 0.01,
            beats_per_bar,
            tx_from_ui,
            custom_painter: None,
        }
    }
}

impl MouseHandler for BeatGridRuler {
    fn handle_mouse_motion(&mut self, _x: f64, _y: f64, _drawing_area: &DrawingArea, _mouse_button: MouseButton, _control_key: bool, _shift_key: bool, _alt_key: bool) {
    }

    fn handle_mouse_press(&mut self, _x: f64, _y: f64, _drawing_area: &DrawingArea, _mouse_button: MouseButton, _control_key: bool, _shift_key: bool, _alt_key: bool) {
    }

    fn handle_mouse_release(&mut self, _x: f64, _y: f64, _drawing_area: &DrawingArea, _mouse_button: MouseButton, _control_key: bool, _shift_key: bool, _alt_key: bool, _data: String) {
    }
}

impl Grid for BeatGridRuler {
    fn paint(&mut self, context: &Context, drawing_area: &DrawingArea) {
        // drawing_area.set_width_request((self.beat_width_in_pixels * self.zoom) as i32 * 400 * 4);
        context.set_source_rgb(1.0, 1.0, 1.0);
        context.rectangle(0.0, 0.0, drawing_area.width_request() as f64, drawing_area.height_request() as f64);
        let _ = context.fill();

        let height = drawing_area.height_request() as f64;
        let width = drawing_area.width_request() as f64;

        self.paint_horizontal_scale(context, height, width);
    }

    fn paint_vertical_scale(&mut self, _context: &Context, _height: f64, _width: f64, _drawing_area: &DrawingArea) {
    }

    fn paint_horizontal_scale(&mut self, context: &Context, height: f64, width: f64) {
        let adjusted_beat_width_in_pixels = self.beat_width_in_pixels * self.zoom_horizontal;
        let (clip_x1, _, clip_x2, _) = context.clip_extents().unwrap();
        let clip_x1_in_beats = clip_x1 / adjusted_beat_width_in_pixels;
        let mut current_x = clip_x1_in_beats.floor() * adjusted_beat_width_in_pixels; // go to the first beat to the left of the view port e.g. bar 2 beat 3 = beat 2 * 4 + 3 = beat 11
        let mut bar_index = (clip_x1_in_beats / 4.0) as i32 + 1; // get the bar
        let mut beat_in_bar_index = (clip_x1_in_beats as i32 % 4) + 1;

        while current_x < clip_x2 {
            if beat_in_bar_index == 1 {
                context.move_to(current_x, height - 20.0);
                context.set_source_rgba(0.5, 0.5, 0.5, 1.0);
                if self.zoom_horizontal < 0.08 {
                    context.set_font_size(7.0);
                }
                else {
                    context.set_font_size(10.0);
                }
                let _ = context.show_text(format!("{}", bar_index).as_str());
            }

            if self.zoom_horizontal > 0.11 {
                context.move_to(current_x, height - 5.0);
                context.set_source_rgba(0.5, 0.5, 0.5, 0.5);
                context.set_font_size(8.0);
                let _ = context.show_text(format!("{}", beat_in_bar_index).as_str());
            }

            if self.zoom_horizontal > 0.11 || beat_in_bar_index == 1 {
                context.move_to(current_x, 0.0);
                context.line_to(current_x, height);
                context.set_line_width(0.3);
                let _ = context.stroke();
            }

            current_x += adjusted_beat_width_in_pixels;

            if beat_in_bar_index == 4 {
                beat_in_bar_index = 1;
                bar_index += 1;
            }
            else {
                beat_in_bar_index += 1;
            }
        }
    }

    fn paint_custom(&mut self, _context: &Context, _height: f64, _width: f64, _drawing_area_widget_name: String, _drawing_area: &DrawingArea) {
    }

    fn paint_select_window(&mut self, _context: &Context, _height: f64, _width: f64) {
    }

    fn paint_loop_markers(&mut self, _context: &Context, _height: f64, _width: f64) {
    }

    fn paint_play_cursor(&mut self, _context: &Context, _height: f64, _width: f64) {
    }

    fn paint_edit_cursor(&mut self, _context: &Context, _height: f64, _width: f64) {
    }

    fn handle_cut(&mut self, _drawing_area: &DrawingArea) {

    }

    fn handle_copy(&mut self, _drawing_area: &DrawingArea) {

    }

    fn handle_paste(&mut self, _drawing_area: &DrawingArea) {
        todo!()
    }

    fn handle_translate_up(&mut self, _drawing_area: &DrawingArea) {
        todo!()
    }

    fn handle_translate_down(&mut self, _drawing_area: &DrawingArea) {
        todo!()
    }

    fn handle_translate_left(&mut self, _drawing_area: &DrawingArea) {

    }

    fn handle_translate_right(&mut self, _drawing_area: &DrawingArea) {

    }

    fn handle_quantise(&mut self, _drawing_area: &DrawingArea) {

    }

    fn handle_increase_entity_length(&mut self, _drawing_area: &DrawingArea) {

    }

    fn handle_decrease_entity_length(&mut self, _drawing_area: &DrawingArea) {

    }

    fn zoom_horizontal_in(&mut self) {
        if self.zoom_horizontal < 7.0 {
            self.zoom_horizontal += self.zoom_factor;
        }
    }

    fn zoom_horizontal_out(&mut self) {
        if self.zoom_horizontal > (self.zoom_factor * 2.0) {
            self.zoom_horizontal -= self.zoom_factor;
        }
    }

    fn set_tempo(&mut self, _tempo: f64) {

    }

    fn set_snap_position_in_beats(&mut self, _snap_position_in_beats: f64) {

    }

    fn set_new_entity_length_in_beats(&mut self, _new_entity_length_in_beats: f64) {

    }

    fn set_entity_length_increment_in_beats(&mut self, _entity_length_increment_in_beats: f64) {

    }

    fn snap_position_in_beats(&self) -> f64 {
        0.0
    }

    fn entity_length_in_beats(&self) -> f64 {
        0.0
    }

    fn entity_length_increment_in_beats(&self) -> f64 {
        0.03125
    }

    fn custom_painter(&mut self) -> &mut Option<Box<dyn CustomPainter>> {
        &mut self.custom_painter
    }

    fn beat_width_in_pixels(&self) -> f64 {
        self.beat_width_in_pixels
    }

    fn zoom_horizontal(&self) -> f64 {
        self.zoom_horizontal
    }

    fn turn_on_draw_point_mode(&mut self) {
    }

    fn turn_on_draw_line_mode(&mut self) {
    }

    fn turn_on_draw_curve_mode(&mut self) {
    }

    fn zoom_vertical(&self) -> f64 {
        self.zoom_vertical
    }

    fn zoom_vertical_in(&mut self) {
        if self.zoom_vertical < 7.0 {
            self.zoom_vertical += self.zoom_factor;
        }
    }

    fn zoom_vertical_out(&mut self) {
        if self.zoom_vertical > (self.zoom_factor * 2.0) {
            self.zoom_vertical -= self.zoom_factor;
        }
    }

    fn set_horizontal_zoom(&mut self, zoom: f64) {
        self.zoom_horizontal = zoom;
    }

    fn set_vertical_zoom(&mut self, zoom: f64) {
        self.zoom_vertical = zoom;
    }

    fn entity_height_in_pixels(&self) -> f64 {
        0.0
    }
}

pub struct Piano {
    entity_height_in_pixels: f64,
    white_key_length: f64,
    black_key_length: f64,
    tx_from_ui: crossbeam_channel::Sender<DAWEvents>,

    // zoom
    zoom_vertical: f64,
    zoom_factor: f64,
}

impl MouseHandler for Piano {
    fn handle_mouse_motion(&mut self, _x: f64, _y: f64, _drawing_area: &DrawingArea, _mouse_button: MouseButton, _control_key: bool, _shift_key: bool, _alt_key: bool) {
    }

    fn handle_mouse_press(&mut self, _x: f64, y: f64, drawing_area: &DrawingArea, _mouse_button: MouseButton, _control_key: bool, _shift_key: bool, _alt_key: bool) {
        let height = drawing_area.height_request() as f64;
        let note_height_in_pixels = height / 127.0;
        let note_number = 127.0 - y /note_height_in_pixels;
        let _ = self.tx_from_ui.send(DAWEvents::PlayNoteImmediate(note_number as i32));
    }

    fn handle_mouse_release(&mut self, _x: f64, y: f64, drawing_area: &DrawingArea, _mouse_button: MouseButton, _control_key: bool, _shift_key: bool, _alt_key: bool, _data: String) {
        let height = drawing_area.height_request() as f64;
        let note_height_in_pixels = height / 127.0;
        let note_number = 127.0 - y /note_height_in_pixels;
        let _ = self.tx_from_ui.send(DAWEvents::StopNoteImmediate(note_number as i32));
    }
}

impl Piano {
    pub fn new(zoom_vertical: f64, entity_height_in_pixels: f64, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) -> Self {
        Self {
            entity_height_in_pixels,
            white_key_length: 100.0,
            black_key_length: 50.0,
            tx_from_ui,
            zoom_vertical,
            zoom_factor: 0.01,
        }
    }

    pub fn paint(&self, context: &Context, drawing_area: &DrawingArea) {
        context.set_source_rgb(1.0, 1.0, 1.0);
        context.rectangle(0.0, 0.0, drawing_area.width_request() as f64, drawing_area.height_request() as f64);
        let _ = context.fill();

        self.paint_keyboard(context, drawing_area);
    }

    fn paint_keyboard(&self, context: &Context, drawing_area: &DrawingArea) {
        let x_start = 0.0;
        let mut y_start = drawing_area.height_request() as f64;
        let adjusted_entity_height_in_pixels = self.entity_height_in_pixels * self.zoom_vertical;

        for index in 0..12 {
            self.paint_white_left_key(context, x_start, y_start, adjusted_entity_height_in_pixels);
            context.move_to(self.white_key_length - 25.0, y_start - 2.0);
            context.set_font_size(12.0);
            context.set_line_width(2.0);
            let octave = format!("C{}", index - 2);
            let _ = context.show_text(octave.as_str());
            context.set_source_rgba(0.0, 0.0, 0.0, 1.0);
            let _ = context.stroke();
            context.set_line_width(1.0);
            y_start -= adjusted_entity_height_in_pixels;
            self.paint_black_key(context, x_start, y_start, adjusted_entity_height_in_pixels);
            y_start -= adjusted_entity_height_in_pixels;
            self.paint_white_t_key(context, x_start, y_start, adjusted_entity_height_in_pixels);
            y_start -= adjusted_entity_height_in_pixels;
            self.paint_black_key(context, x_start, y_start, adjusted_entity_height_in_pixels);
            y_start -= adjusted_entity_height_in_pixels;
            self.paint_white_right_key(context, x_start, y_start, adjusted_entity_height_in_pixels);
            y_start -= adjusted_entity_height_in_pixels;
            self.paint_white_left_key(context, x_start, y_start, adjusted_entity_height_in_pixels);
            y_start -= adjusted_entity_height_in_pixels;
            self.paint_black_key(context, x_start, y_start, adjusted_entity_height_in_pixels);
            y_start -= adjusted_entity_height_in_pixels;
            self.paint_white_t_key(context, x_start, y_start, adjusted_entity_height_in_pixels);
            y_start -= adjusted_entity_height_in_pixels;
            self.paint_black_key(context, x_start, y_start, adjusted_entity_height_in_pixels);
            y_start -= adjusted_entity_height_in_pixels;
            self.paint_white_t_key(context, x_start, y_start, adjusted_entity_height_in_pixels);
            y_start -= adjusted_entity_height_in_pixels;
            self.paint_black_key(context, x_start, y_start, adjusted_entity_height_in_pixels);
            y_start -= adjusted_entity_height_in_pixels;
            self.paint_white_right_key(context, x_start, y_start, adjusted_entity_height_in_pixels);
            y_start -= adjusted_entity_height_in_pixels;
        }
    }

    fn paint_white_left_key(&self, context: &Context, x: f64, y: f64, adjusted_entity_height_in_pixels: f64) {
        context.move_to(x, y);
        context.line_to(x + self.white_key_length, y);
        context.line_to(x + self.white_key_length, y - (adjusted_entity_height_in_pixels * 1.5));
        context.line_to(x + self.black_key_length, y - (adjusted_entity_height_in_pixels * 1.5));
        context.set_source_rgb(0.0, 0.0, 0.0);
        context.set_line_width(0.3);
        let _ = context.stroke();
    }

    fn paint_white_t_key(&self, context: &Context, x: f64, y: f64, adjusted_entity_height_in_pixels: f64) {
        context.move_to(x + self.black_key_length, y + (adjusted_entity_height_in_pixels / 2.0));
        context.line_to(x + self.white_key_length, y + (adjusted_entity_height_in_pixels / 2.0));
        context.line_to(x + self.white_key_length, y - (adjusted_entity_height_in_pixels * 1.5));
        context.line_to(x + self.black_key_length, y - (adjusted_entity_height_in_pixels * 1.5));
        context.set_source_rgb(0.0, 0.0, 0.0);
        context.set_line_width(0.3);
        let _ = context.stroke();
    }

    fn paint_white_right_key(&self, context: &Context, x: f64, y: f64, adjusted_entity_height_in_pixels: f64) {
        context.move_to(x + self.black_key_length, y + (adjusted_entity_height_in_pixels / 2.0));
        context.line_to(x + self.white_key_length, y + (adjusted_entity_height_in_pixels / 2.0));
        context.line_to(x + self.white_key_length, y - adjusted_entity_height_in_pixels);
        context.line_to(x, y - adjusted_entity_height_in_pixels);
        context.set_source_rgb(0.0, 0.0, 0.0);
        context.set_line_width(0.3);
        let _ = context.stroke();
    }

    fn paint_black_key(&self, context: &Context, x: f64, y: f64, adjusted_entity_height_in_pixels: f64) {
        context.set_source_rgb(0.0, 0.0, 0.0);
        context.rectangle(x, y - adjusted_entity_height_in_pixels, x + self.black_key_length, adjusted_entity_height_in_pixels);
        let _ = context.fill();
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

    pub fn set_vertical_zoom(&mut self, zoom: f64) {
        self.zoom_vertical = zoom;
    }

    pub fn entity_height_in_pixels(&self) -> f64 {
        self.entity_height_in_pixels
    }
}

pub struct PianoRollCustomPainter {
    state: Arc<Mutex<DAWState>>,
    pub original_track_event_copy: Option<TrackEvent>,
    pub dragged_track_event: Option<TrackEvent>,
    pub edit_item_handler: EditItemHandler<Note, Note>,
}

impl PianoRollCustomPainter {
    pub fn new_with_edit_item_handler(state: Arc<Mutex<DAWState>>, edit_item_handler: EditItemHandler<Note, Note>) -> PianoRollCustomPainter {
        PianoRollCustomPainter {
            state,
            original_track_event_copy: None,
            dragged_track_event: None,
            edit_item_handler,
        }
    }
}

impl CustomPainter for PianoRollCustomPainter {
    fn paint_custom(&mut self, 
                    context: &Context, 
                    canvas_height: f64, 
                    _canvas_width: f64, 
                    entity_height_in_pixels: f64,
                    beat_width_in_pixels: f64, 
                    zoom_horizontal: f64, 
                    zoom_vertical: f64, 
                    select_window_top_left_x: f64, 
                    select_window_top_left_y: f64, 
                    select_window_bottom_right_x: f64, 
                    select_window_bottom_right_y: f64,
                    _drawing_area_widget_name: Option<String>,
                    mouse_pointer_x: f64,
                    mouse_pointer_y: f64,
                    mouse_pointer_previous_x: f64,
                    mouse_pointer_previous_y: f64,
                    _draw_mode_on: bool,
                    _draw_mode: DrawMode,
                    _draw_mode_start_x: f64,
                    _draw_mode_start_y: f64,
                    _draw_mode_end_x: f64,
                    _draw_mode_end_y: f64,
                    drawing_area: &DrawingArea,
                    operation_mode: &OperationModeType,
                    _drag_started: bool,
                    edit_drag_cycle: &DragCycle,
                    tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
                ) {
        let adjusted_entity_height_in_pixels = entity_height_in_pixels * zoom_vertical;

        match self.state.lock() {
            Ok(state) => {
                let note_expression_note_id = state.piano_roll_mpe_note_id().clone() as i32;
                let adjusted_beat_width_in_pixels = beat_width_in_pixels * zoom_horizontal;
                // let mut edit_mode = EditMode::Inactive;

                match state.selected_track() {
                    Some(track_uuid) => match state.selected_riff_uuid(track_uuid.clone()) {
                        Some(riff_uuid) => match state.project().song().tracks().iter().find(|track| track.uuid().to_string() == track_uuid) {
                            Some(track) => {
                                let (red, green, blue, _) = track.colour();

                                if let Some((track_index, _0)) = state.project().song().tracks().iter().find_position(|track| track.uuid().to_string() == track_uuid) {
                                    for riff in track.riffs().iter() {
                                        if riff.uuid().to_string() == riff_uuid {
                                            let unselected_event_colour = if let Some((red, green, blue, _)) = riff.colour() {
                                                (*red, *green, *blue, 1.0)
                                            }
                                            else {
                                                (red, green, blue, 1.0)
                                            };

                                            // find all the selected notes
                                            let selected_riff_events = state.selected_riff_events().clone();
                                            let mut selected_notes = vec![];
                                            for event in riff.events().iter().filter(|event| {
                                                if let TrackEvent::Note(note) = event {
                                                    selected_riff_events.contains(&note.id())
                                                } else {
                                                    false
                                                }
                                            }) {
                                                if let TrackEvent::Note(note) = event {
                                                    selected_notes.push(note.clone());
                                                }
                                            }

                                            for track_event in riff.events() {
                                                let mut event_colour = unselected_event_colour.clone();

                                                match track_event {
                                                    TrackEvent::Note(note) => {
                                                        if note_expression_note_id == -1 || note_expression_note_id == note.note_id()  {
                                                            let note_number = note.note();
                                                            let note_y_pos_inverted = note_number as f64 * adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels;
                                                            let x = note.position() * adjusted_beat_width_in_pixels;
                                                            // let x_original = x;
                                                            let y = canvas_height - note_y_pos_inverted;
                                                            // let y_original = y;
                                                            let width = note.length() * adjusted_beat_width_in_pixels;

                                                            let is_selected = state.selected_riff_events().iter().any(|id| id.as_str() == note.id().as_str());
                                                            if is_selected {
                                                                event_colour = (0.0, 0.0, 1.0, 1.0);
                                                            }
                                                            context.set_source_rgba(event_colour.0, event_colour.1, event_colour.2, event_colour.3);

                                                            self.edit_item_handler.handle_item_edit(
                                                                context,
                                                                note,
                                                                operation_mode,
                                                                mouse_pointer_x,
                                                                mouse_pointer_y,
                                                                mouse_pointer_previous_x,
                                                                mouse_pointer_previous_y,
                                                                adjusted_entity_height_in_pixels,
                                                                adjusted_beat_width_in_pixels,
                                                                x,
                                                                y,
                                                                width,
                                                                canvas_height,
                                                                drawing_area,
                                                                edit_drag_cycle,
                                                                tx_from_ui.clone(),
                                                                true,
                                                                track_uuid.clone(),
                                                                note,
                                                                true,
                                                                track_index as f64,
                                                                is_selected,
                                                                selected_notes.clone()
                                                            );

                                                            context.rectangle(x, y, width, adjusted_entity_height_in_pixels);
                                                            if note.note_id() > -1 {
                                                                context.set_font_size(9.0);
                                                                let _ = context.show_text(format!("{}", note.note_id()).as_str());
                                                            }
                                                            let _ = context.fill();

                                                            if note.riff_start_note() {
                                                                context.set_source_rgba(0.0, 0.0, 0.0, 1.0);
                                                                context.rectangle(x, y, 2.0, adjusted_entity_height_in_pixels);
                                                                let _ = context.fill();
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
                            },
                            None => (),
                        },
                        None => (),
                    },
                    None => (),
                }
            },
            Err(_) => debug!("Piano Roll custom painter could not get state lock."),
        }

        context.set_source_rgba(0.0, 0.0, 0.0, 1.0);
        context.move_to(mouse_pointer_x, mouse_pointer_y);
        context.set_font_size(12.0);
        let note_number = ((canvas_height - mouse_pointer_y) / adjusted_entity_height_in_pixels) as i32;
        let note_name_index = (note_number % 12) as usize;

        if note_name_index >=0 && note_name_index < NOTE_NAMES.len() {
            let note_name = NOTE_NAMES[note_name_index];
            let octave_number = note_number / 12 - 2;
            let _ = context.show_text(format!("{}{}", note_name, octave_number).as_str());
        }
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

pub struct EditItemHandler<T: DAWItemID + DAWItemPosition + DAWItemLength + DAWItemVerticalIndex + Clone, U: DAWItemID + DAWItemPosition + DAWItemLength + DAWItemVerticalIndex + Clone> {
    pub original_item: Option<T>,
    pub original_item_is_selected: bool,
    pub original_selected_items: Vec<T>,
    pub selected_item_ids: Vec<String>,
    pub dragged_item: Option<T>,
    pub referenced_item: Option<U>,
    pub changed_event_sender: Box<dyn Fn(Vec<(T, T)>, String, crossbeam_channel::Sender<DAWEvents>)>,
    pub copied_event_sender: Box<dyn Fn(Vec<T>, String, crossbeam_channel::Sender<DAWEvents>)>,
    pub can_change_start: bool,
    pub can_change_position: bool,
    pub can_change_end: bool,
    pub can_drag_copy: bool,
}

impl<T: DAWItemID + DAWItemPosition + DAWItemLength + DAWItemVerticalIndex + Clone, U: DAWItemID + DAWItemPosition + DAWItemLength + DAWItemVerticalIndex + Clone> EditItemHandler<T, U> {
    pub fn new(
        changed_event_sender: Box<dyn Fn(Vec<(T, T)>, String, crossbeam_channel::Sender<DAWEvents>)>,
        copied_event_sender: Box<dyn Fn(Vec<T>, String, crossbeam_channel::Sender<DAWEvents>)>,
        can_change_start: bool,
        can_change_position: bool,
        can_change_end: bool,
        can_drag_copy: bool
    ) -> Self {
        Self { 
            original_item: None,
            original_item_is_selected: false,
            original_selected_items: vec![],
            selected_item_ids: vec![],
            dragged_item: None,
            referenced_item: None,
            changed_event_sender,
            copied_event_sender,
            can_change_start,
            can_change_position,
            can_change_end,
            can_drag_copy,
        }
    }    
}

impl<T: DAWItemID + DAWItemPosition + DAWItemLength + DAWItemVerticalIndex + Clone, U: DAWItemID + DAWItemPosition + DAWItemLength + DAWItemVerticalIndex + Clone> EditItemHandler<T, U> {
    pub fn handle_item_edit(
        &mut self,
        context: &Context, 
        item: &T,
        operation_mode: &OperationModeType,
        mouse_pointer_x: f64,
        mouse_pointer_y: f64,
        mouse_pointer_previous_x: f64,
        mouse_pointer_previous_y: f64,
        adjusted_entity_height_in_pixels: f64,
        adjusted_beat_width_in_pixels: f64,
        x_original: f64,
        y_original: f64,
        width_original: f64,
        canvas_height: f64, 
        drawing_area: &DrawingArea,
        edit_drag_cycle: &DragCycle,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        invert_vertically: bool,
        track_uuid: String,
        referencing_item: &U,
        allow_vertical_drag: bool,
        track_index: f64,
        item_is_selected: bool,
        selected_items: Vec<T>,
    ) {
        let mut edit_mode = EditMode::Inactive;

        match operation_mode {
            OperationModeType::Change => {
                let mut x = x_original;
                let mut y = y_original;
                let mut width = width_original;
                let mut found_item_being_changed = false;
                // calculate the mouse position deltas
                let delta_x = mouse_pointer_x - mouse_pointer_previous_x;
                let delta_y = mouse_pointer_y - mouse_pointer_previous_y;
                let mut use_this_item = false;

                if (item.position() - referencing_item.position()).abs() > 1e-10 {
                    x = referencing_item.position() * adjusted_beat_width_in_pixels;
                }

                if let DragCycle::NotStarted = edit_drag_cycle {
                    self.original_item = None;
                    self.original_selected_items.clear();
                    self.selected_item_ids.clear();
                    self.dragged_item = None;
                }

                // make sure original item matches the iterated item
                if let Some(original_item) = self.original_item.as_ref() {
                    if (original_item.position() - item.position()).abs() < 1e-10 &&
                        original_item.id() == item.id() &&
                        original_item.vertical_index() == item.vertical_index() &&
                        (original_item.length() - item.length()).abs() < 1e-10 {
                        if let Some(dragged_item) = self.dragged_item.as_ref() {
                            if dragged_item.id() == referencing_item.id() {
                                found_item_being_changed = true;
                            }
                        }
                    }
                }

                if found_item_being_changed {
                    if let Some(dragged_item) = self.dragged_item.as_ref() {
                        debug!("Dragged item found.");
                        let vertical_index = if allow_vertical_drag {
                            dragged_item.vertical_index() as f64
                        }
                        else {
                            track_index
                        };
                        let vertical_y_position = vertical_index * adjusted_entity_height_in_pixels;
                        width = dragged_item.length() * adjusted_beat_width_in_pixels;
                        x = dragged_item.position() * adjusted_beat_width_in_pixels + delta_x;

                        if invert_vertically {
                            y = canvas_height - vertical_y_position + delta_y - adjusted_entity_height_in_pixels;
                        }
                        else {
                            y = vertical_y_position + delta_y;
                        }
                    }
                }

                // draw left length adjust handle if required
                if self.can_change_start &&
                    mouse_pointer_x as f64 >= x &&
                    mouse_pointer_x <= (x + 5.0) &&
                    width >= 10.0 &&
                    mouse_pointer_y >= y &&
                    mouse_pointer_y <= (y + adjusted_entity_height_in_pixels) {
                    //change the mode
                    edit_mode = EditMode::ChangeStart;
                    use_this_item = true;

                    // change the prompt
                    if let Some(window) = drawing_area.window() {
                        window.set_cursor(Some(&gdk::Cursor::for_display(&window.display(), gdk::CursorType::LeftSide)));
                        // debug!("drawing left side prompt.");
                    }
                }
                // draw drag position adjust handle if required
                else if (self.can_change_position || self.can_drag_copy) &&
                    mouse_pointer_x as f64 >= (x + 5.0) &&
                    mouse_pointer_x <= (x + width - 5.0) &&
                    width >= 10.0 &&
                    mouse_pointer_y >= y &&
                    mouse_pointer_y <= (y + adjusted_entity_height_in_pixels) {
                    //change the mode
                    edit_mode = EditMode::Move;
                    use_this_item = true;

                    // change the prompt
                    if let Some(window) = drawing_area.window() {
                        window.set_cursor(Some(&gdk::Cursor::for_display(&window.display(), gdk::CursorType::Hand1)));
                        // debug!("drawing hand prompt.");
                    }
                }
                // draw right length adjust handle if required
                else if self.can_change_end &&
                    mouse_pointer_x as f64 <= (x + width) &&
                    mouse_pointer_x >= (x + width - 5.0) &&
                    width >= 10.0 &&
                    mouse_pointer_y >= y &&
                    mouse_pointer_y <= (y + adjusted_entity_height_in_pixels) {
                    //change the mode
                    edit_mode = EditMode::ChangeEnd;
                    use_this_item = true;

                    // change the prompt
                    if let Some(window) = drawing_area.window() {
                        window.set_cursor(Some(&gdk::Cursor::for_display(&window.display(), gdk::CursorType::RightSide)));
                        // debug!("drawing right side prompt.");
                    }
                }

                match edit_mode {
                    EditMode::Inactive => {
                        // debug!("EditMode::Inactive");
                    }
                    _ => {
                        match edit_drag_cycle {
                            DragCycle::MousePressed => {
                                debug!("handle_item_edit EditDragCycle::MousePressed");
                                if use_this_item {
                                    debug!("handle_item_edit EditDragCycle::MousePressed - set original and dragged items.");
                                    self.original_item = Some(item.clone());
                                    self.original_item_is_selected = item_is_selected;
                                    self.original_selected_items = selected_items;
                                    if item.id() != referencing_item.id() {
                                        let mut dragged_item = item.clone();

                                        dragged_item.set_id(referencing_item.id());
                                        dragged_item.set_position(referencing_item.position());
                                        self.dragged_item = Some(dragged_item);
                                    }
                                    else {
                                        self.dragged_item = Some(item.clone());
                                    }
                                }
                            }
                            DragCycle::Dragging => {
                                debug!("handle_item_edit EditDragCycle::Dragging");

                                if found_item_being_changed {
                                    match edit_mode {
                                        EditMode::ChangeStart => {
                                            if let Some(dragged_item) = self.dragged_item.as_mut() {
                                                // calculate the new item width
                                                let new_width = width - delta_x;

                                                // draw the dragged item
                                                context.rectangle(x, y_original, new_width, adjusted_entity_height_in_pixels);
                                                let _ = context.fill();

                                                // draw the other selected items
                                                if self.original_item_is_selected {
                                                    let delta_x = new_width - dragged_item.length() * adjusted_beat_width_in_pixels;
                                                    for item in self.original_selected_items.iter() {
                                                        if item.id() != dragged_item.id() {
                                                            let x = item.position() * adjusted_beat_width_in_pixels - delta_x;
                                                            let y_pos_inverted = item.vertical_index() as f64 * adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels;
                                                            let y = canvas_height - y_pos_inverted;
                                                            let new_width = item.length() * adjusted_beat_width_in_pixels + delta_x;
                                                            context.rectangle(x, y, new_width, adjusted_entity_height_in_pixels);
                                                            let _ = context.fill();
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        EditMode::Move => {
                                            if let Some(dragged_item) = self.dragged_item.as_mut() {
                                                // draw the dragged item
                                                if allow_vertical_drag {
                                                    context.rectangle(x, y, width, adjusted_entity_height_in_pixels);
                                                }
                                                else {
                                                    context.rectangle(x, y_original, width, adjusted_entity_height_in_pixels);
                                                }
                                                let _ = context.fill();

                                                // draw the other selected items
                                                if self.original_item_is_selected {
                                                    let delta_x = x - dragged_item.position() * adjusted_beat_width_in_pixels;
                                                    let delta_y = y - (canvas_height - dragged_item.vertical_index() as f64 * adjusted_entity_height_in_pixels);
                                                    for item in self.original_selected_items.iter() {
                                                        if item.id() != dragged_item.id() {
                                                            let x = item.position() * adjusted_beat_width_in_pixels + delta_x;
                                                            let mut y = if allow_vertical_drag {
                                                                if invert_vertically {
                                                                    canvas_height - item.vertical_index() as f64 * adjusted_entity_height_in_pixels
                                                                }
                                                                else {
                                                                    item.vertical_index() as f64 * adjusted_entity_height_in_pixels
                                                                }
                                                            }
                                                            else {
                                                                // track y is not inverted
                                                                debug!("Track riff ref vertical index={}", item.vertical_index());
                                                                item.vertical_index() as f64 * adjusted_entity_height_in_pixels
                                                            };
                                                            let width = item.length() * adjusted_beat_width_in_pixels;

                                                            if allow_vertical_drag {
                                                                context.rectangle(x, y + delta_y, width, adjusted_entity_height_in_pixels);
                                                            }
                                                            else {
                                                                context.rectangle(x, y, width, adjusted_entity_height_in_pixels);
                                                            }
                                                            let _ = context.fill();
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        EditMode::ChangeEnd => {
                                            if let Some(dragged_item) = self.dragged_item.as_mut() {
                                                // calculate the new item width
                                                let new_width = width + delta_x;

                                                // draw the dragged item
                                                context.rectangle(x_original, y_original, new_width, adjusted_entity_height_in_pixels);
                                                let _ = context.fill();

                                                // draw the other selected items
                                                if self.original_item_is_selected {
                                                    let delta_x = new_width - dragged_item.length() * adjusted_beat_width_in_pixels;
                                                    for item in self.original_selected_items.iter() {
                                                        if item.id() != dragged_item.id() {
                                                            let x = item.position() * adjusted_beat_width_in_pixels;
                                                            let y = if invert_vertically {
                                                                let y_pos_inverted = item.vertical_index() as f64 * adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels;
                                                                canvas_height - y_pos_inverted
                                                            }
                                                            else {
                                                                item.vertical_index() as f64 * adjusted_entity_height_in_pixels
                                                            };
                                                            let new_width = item.length() * adjusted_beat_width_in_pixels + delta_x;
                                                            context.rectangle(x, y, new_width, adjusted_entity_height_in_pixels);
                                                            let _ = context.fill();
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            DragCycle::MouseReleased => {
                                debug!("handle_item_edit EditDragCycle::MouseReleased");

                                if found_item_being_changed {
                                    if let Some(original_item) = self.original_item.as_ref() {
                                        if let Some(dragged_item) = self.dragged_item.as_mut() {
                                            match edit_mode {
                                                EditMode::Inactive => {}
                                                EditMode::ChangeStart => {
                                                    let mut change = vec![];
                                                    // calculate and set the position
                                                    let position_in_beats = x /adjusted_beat_width_in_pixels;

                                                    // calculate and set the width
                                                    let new_width = width - delta_x;
                                                    let duration = new_width / adjusted_beat_width_in_pixels;

                                                    dragged_item.set_position(position_in_beats);
                                                    dragged_item.set_length(duration);

                                                    // (self.changed_event_sender)(original_item.clone(), dragged_item.clone(), track_uuid.clone(), tx_from_ui.clone());
                                                    change.push((original_item.clone(), dragged_item.clone()));

                                                    // handle the other selected items
                                                    if self.original_item_is_selected {
                                                        let delta_x = new_width - original_item.length() * adjusted_beat_width_in_pixels;
                                                        for item in self.original_selected_items.iter() {
                                                            if item.id() != dragged_item.id() {
                                                                let x = item.position() * adjusted_beat_width_in_pixels - delta_x;
                                                                let position_in_beats = x /adjusted_beat_width_in_pixels;

                                                                let new_width = item.length() * adjusted_beat_width_in_pixels + delta_x;
                                                                let duration = new_width / adjusted_beat_width_in_pixels;

                                                                let mut changed_item = item.clone();
                                                                changed_item.set_position(position_in_beats);
                                                                changed_item.set_length(duration);

                                                                // (self.changed_event_sender)(item.clone(), changed_item, track_uuid.clone(), tx_from_ui.clone());
                                                                change.push((item.clone(), changed_item.clone()));
                                                            }
                                                        }
                                                    }

                                                    if !change.is_empty() {
                                                        (self.changed_event_sender)(change, track_uuid.clone(), tx_from_ui.clone());
                                                    }
                                                }
                                                EditMode::Move => {
                                                    let mut change = vec![];
                                                    // calculate and set the position
                                                    let delta_x = x - dragged_item.position() * adjusted_beat_width_in_pixels;
                                                    let position_in_beats = x /adjusted_beat_width_in_pixels;
                                                    dragged_item.set_position(position_in_beats);

                                                    // calculate and set the vertical index
                                                    if allow_vertical_drag {
                                                        let vertical_index = if invert_vertically {
                                                            let y_pos_inverted = canvas_height - y;
                                                            ((y_pos_inverted - adjusted_entity_height_in_pixels) / adjusted_entity_height_in_pixels) as i32
                                                        }
                                                        else {
                                                            y as i32
                                                        };

                                                        debug!("Setting dragged item vertical index to: {}", vertical_index);
                                                        dragged_item.set_vertical_index(vertical_index + 1);
                                                    }

                                                    // (self.changed_event_sender)(original_item.clone(), dragged_item.clone(), track_uuid.clone(), tx_from_ui.clone());
                                                    change.push((original_item.clone(), dragged_item.clone()));

                                                    // handle the other selected items
                                                    if self.original_item_is_selected {
                                                        let delta_y = y - (canvas_height - original_item.vertical_index() as f64 * adjusted_entity_height_in_pixels);
                                                        for item in self.original_selected_items.iter() {
                                                            if item.id() != dragged_item.id() {
                                                                let x = item.position() * adjusted_beat_width_in_pixels + delta_x;
                                                                let y_pos_inverted = item.vertical_index() as f64 * adjusted_entity_height_in_pixels;
                                                                let y = if invert_vertically {
                                                                    canvas_height - y_pos_inverted
                                                                }
                                                                else {
                                                                    item.vertical_index() as f64 * adjusted_entity_height_in_pixels
                                                                };

                                                                // calculate and set the position
                                                                let position_in_beats = x /adjusted_beat_width_in_pixels;
                                                                let mut changed_item = item.clone();
                                                                changed_item.set_position(position_in_beats);

                                                                // calculate and set the vertical index
                                                                if allow_vertical_drag {
                                                                    let vertical_index = if invert_vertically {
                                                                        let y_pos_uninverted = canvas_height - y;
                                                                        ((y_pos_uninverted - adjusted_entity_height_in_pixels - delta_y) / adjusted_entity_height_in_pixels) as i32
                                                                    }
                                                                    else {
                                                                        ((y - delta_y) / adjusted_entity_height_in_pixels) as i32
                                                                    };

                                                                    debug!("Setting selected item vertical index to: {}", vertical_index);
                                                                    changed_item.set_vertical_index(vertical_index + 1);
                                                                }

                                                                // (self.changed_event_sender)(item.clone(), changed_item.clone(), track_uuid.clone(), tx_from_ui.clone());
                                                                change.push((item.clone(), changed_item.clone()));
                                                            }
                                                        }
                                                    }

                                                    if !change.is_empty() {
                                                        (self.changed_event_sender)(change, track_uuid.clone(), tx_from_ui.clone());
                                                    }
                                                }
                                                EditMode::ChangeEnd => {
                                                    let mut change = vec![];
                                                    // calculate and set the width
                                                    let new_width = width + delta_x;
                                                    let duration = new_width / adjusted_beat_width_in_pixels;

                                                    dragged_item.set_length(duration);

                                                    change.push((original_item.clone(), dragged_item.clone()));

                                                    // handle the other selected items
                                                    if self.original_item_is_selected {
                                                        let delta_x = new_width - original_item.length() * adjusted_beat_width_in_pixels;
                                                        for item in self.original_selected_items.iter() {
                                                            if item.id() != dragged_item.id() {
                                                                let new_width = item.length() * adjusted_beat_width_in_pixels + delta_x;
                                                                let mut changed_item = item.clone();

                                                                changed_item.set_length(new_width / adjusted_beat_width_in_pixels);

                                                                change.push((item.clone(), changed_item.clone()));
                                                            }
                                                        }
                                                    }

                                                    if !change.is_empty() {
                                                        (self.changed_event_sender)(change, track_uuid.clone(), tx_from_ui.clone());
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    debug!("handle_item_edit EditDragCycle::MouseReleased - unset original and dragged items.");
                                    self.original_item = None;
                                    self.original_selected_items.clear();
                                    self.selected_item_ids.clear();
                                    self.dragged_item = None;
                                }
                            }
                            DragCycle::CtrlMousePressed => {
                                debug!("handle_item_edit EditDragCycle::CtrlMousePressed");
                                if use_this_item {
                                    debug!("handle_item_edit EditDragCycle::CtrlMousePressed - set original and dragged items.");
                                    self.original_item = Some(item.clone());
                                    self.original_item_is_selected = item_is_selected;
                                    self.original_selected_items = selected_items;
                                    if item.id() != referencing_item.id() {
                                        let mut dragged_item = item.clone();

                                        dragged_item.set_id(referencing_item.id());
                                        dragged_item.set_position(referencing_item.position());
                                        self.dragged_item = Some(dragged_item);
                                    }
                                    else {
                                        self.dragged_item = Some(item.clone());
                                    }
                                }
                            }
                            DragCycle::CtrlDragging => {
                                debug!("handle_item_edit EditDragCycle::CtrlDragging");

                                if found_item_being_changed {
                                    if let EditMode::Move = edit_mode {
                                        if let Some(dragged_item) = self.dragged_item.as_mut() {
                                            // draw the dragged item
                                            if allow_vertical_drag {
                                                context.rectangle(x, y, width, adjusted_entity_height_in_pixels);
                                            }
                                            else {
                                                context.rectangle(x, y_original, width, adjusted_entity_height_in_pixels);
                                            }
                                            let _ = context.fill();

                                            // draw the other selected items
                                            if self.original_item_is_selected {
                                                let delta_x = x - dragged_item.position() * adjusted_beat_width_in_pixels;
                                                let delta_y = y - (canvas_height - dragged_item.vertical_index() as f64 * adjusted_entity_height_in_pixels);
                                                for item in self.original_selected_items.iter() {
                                                    if item.id() != dragged_item.id() {
                                                        let x = item.position() * adjusted_beat_width_in_pixels + delta_x;
                                                        let mut y = if allow_vertical_drag {
                                                            if invert_vertically {
                                                                canvas_height - item.vertical_index() as f64 * adjusted_entity_height_in_pixels
                                                            }
                                                            else {
                                                                item.vertical_index() as f64 * adjusted_entity_height_in_pixels
                                                            }
                                                        }
                                                        else {
                                                            // track y is not inverted
                                                            item.vertical_index() as f64 * adjusted_entity_height_in_pixels
                                                        };
                                                        let width = item.length() * adjusted_beat_width_in_pixels;

                                                        if allow_vertical_drag {
                                                            context.rectangle(x, y + delta_y, width, adjusted_entity_height_in_pixels);
                                                        }
                                                        else {
                                                            context.rectangle(x, y, width, adjusted_entity_height_in_pixels);
                                                        }
                                                        let _ = context.fill();
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            DragCycle::CtrlMouseReleased => {
                                debug!("handle_item_edit EditDragCycle::CtrlMouseReleased");

                                if found_item_being_changed {
                                    if let Some(original_item) = self.original_item.as_ref() {
                                        if let Some(dragged_item) = self.dragged_item.as_mut() {
                                            if let EditMode::Move = edit_mode {
                                                let mut change = vec![];
                                                // calculate and set the position
                                                let delta_x = x - dragged_item.position() * adjusted_beat_width_in_pixels;
                                                let position_in_beats = x /adjusted_beat_width_in_pixels;
                                                dragged_item.set_position(position_in_beats);

                                                // calculate and set the vertical index
                                                if allow_vertical_drag {
                                                    let vertical_index = if invert_vertically {
                                                        let y_pos_inverted = canvas_height - y;
                                                        ((y_pos_inverted - adjusted_entity_height_in_pixels) / adjusted_entity_height_in_pixels) as i32
                                                    }
                                                    else {
                                                        y as i32
                                                    };

                                                    debug!("Setting dragged item vertical index to: {}", vertical_index);
                                                    dragged_item.set_vertical_index(vertical_index + 1);
                                                }

                                                change.push(dragged_item.clone());

                                                // handle the other selected items
                                                if self.original_item_is_selected {
                                                    // let delta_x = x - original_item.position() * adjusted_beat_width_in_pixels;
                                                    let delta_y = y - (canvas_height - original_item.vertical_index() as f64 * adjusted_entity_height_in_pixels);
                                                    for item in self.original_selected_items.iter() {
                                                        if item.id() != dragged_item.id() {
                                                            let x = item.position() * adjusted_beat_width_in_pixels + delta_x;
                                                            let y_pos_inverted = item.vertical_index() as f64 * adjusted_entity_height_in_pixels;
                                                            let y = if invert_vertically {
                                                                canvas_height - y_pos_inverted
                                                            }
                                                            else {
                                                                item.vertical_index() as f64 * adjusted_entity_height_in_pixels
                                                            };

                                                            // calculate and set the position
                                                            let position_in_beats = x /adjusted_beat_width_in_pixels;
                                                            let mut copied_item = item.clone();
                                                            copied_item.set_position(position_in_beats);

                                                            // calculate and set the vertical index
                                                            if allow_vertical_drag {
                                                                let vertical_index = if invert_vertically {
                                                                    let y_pos_uninverted = canvas_height - y;
                                                                    ((y_pos_uninverted - adjusted_entity_height_in_pixels - delta_y) / adjusted_entity_height_in_pixels) as i32
                                                                }
                                                                else {
                                                                    ((y - delta_y) / adjusted_entity_height_in_pixels) as i32
                                                                };

                                                                debug!("Setting selected item vertical index to: {}", vertical_index);
                                                                copied_item.set_vertical_index(vertical_index + 1);
                                                            }

                                                            change.push(copied_item);
                                                        }
                                                    }
                                                }

                                                if !change.is_empty() {
                                                    (self.copied_event_sender)(change, track_uuid.clone(), tx_from_ui.clone());
                                                }
                                            }
                                        }
                                    }

                                    debug!("handle_item_edit EditDragCycle::CtrlMouseReleased - unset original and dragged items.");
                                    self.original_item = None;
                                    self.original_selected_items.clear();
                                    self.selected_item_ids.clear();
                                    self.dragged_item = None;
                                }
                            }
                            DragCycle::NotStarted => {
                                debug!("handle_item_edit EditDragCycle::NotStarted");
                            }
                        }
                    }
                }
            }
            _ => {
            }
        }
    }
}

pub struct SampleRollCustomPainter {
    state: Arc<Mutex<DAWState>>,
}

impl SampleRollCustomPainter {
    pub fn new(state: Arc<Mutex<DAWState>>) -> Self {
        Self {
            state,
        }
    }
}

impl CustomPainter for SampleRollCustomPainter {
    fn paint_custom(&mut self, context: &Context, _height: f64, _width: f64, entity_height_in_pixels: f64,
                    beat_width_in_pixels: f64, 
                    zoom_horizontal: f64, 
                    zoom_vertical: f64, 
                    select_window_top_left_x: f64, 
                    select_window_top_left_y: f64, 
                    select_window_bottom_right_x: f64, 
                    select_window_bottom_right_y: f64,
                    _drawing_area_widget_name: Option<String>,
                    _mouse_pointer_x: f64,
                    _mouse_pointer_y: f64,
                    _mouse_pointer_previous_x: f64,
                    _mouse_pointer_previous_y: f64,
                    _draw_mode_on: bool,
                    _draw_mode: DrawMode,
                    _draw_mode_start_x: f64,
                    _draw_mode_start_y: f64,
                    _draw_mode_end_x: f64,
                    _draw_mode_end_y: f64,
                    _drawing_area: &DrawingArea,
                    _operation_mode: &OperationModeType,
                    _drag_started: bool,
                    _edit_drag_cycle: &DragCycle,
                    _tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
                ) {
        match self.state.lock() {
            Ok( state) => {
                // let bpm = state.get_project().song().tempo();
                // let beats_per_second = bpm / 60.0;
                let adjusted_beat_width_in_pixels = /* beats_per_second * */ beat_width_in_pixels * zoom_horizontal;
                let _adjusted_entity_height_in_pixels = entity_height_in_pixels * zoom_vertical;

                match state.selected_track() {
                    Some(track_uuid) => match state.selected_riff_uuid(track_uuid.clone()) {
                        Some(riff_uuid) => match state.project().song().tracks().iter().find(|track| track.uuid().to_string() == track_uuid) {
                            Some(track) => {
                                let (red, green, blue, alpha) = track.colour();

                                for riff in track.riffs().iter() {
                                    if riff.uuid().to_string() == riff_uuid {
                                        for track_event in riff.events() {
                                            if let Some((red, green, blue, alpha)) = riff.colour() {
                                                context.set_source_rgba(*red, *green, *blue, *alpha);
                                            }
                                            else {
                                                context.set_source_rgba(red, green, blue, alpha);
                                            }

                                            match track_event {
                                                TrackEvent::Sample(sample) => {
                                                    let sample_y_pos = 0.0;
                                                    let x = sample.position() * adjusted_beat_width_in_pixels;
                                                    let width = 1.0 * adjusted_beat_width_in_pixels;
                                                    if select_window_top_left_x <= x && (x + width) <= select_window_bottom_right_x &&
                                                        select_window_top_left_y <= sample_y_pos && (sample_y_pos + entity_height_in_pixels) <= select_window_bottom_right_y {
                                                        context.set_source_rgb(0.0, 0.0, 1.0);
                                                    }
                                                    context.rectangle(x, sample_y_pos, width, entity_height_in_pixels);
                                                    let _ = context.fill();
                                                }
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
            },
            Err(_) => debug!("Piano Roll custom painter could not get state lock."),
        }
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

pub struct RiffSetTrackCustomPainter {
    state: Arc<Mutex<DAWState>>,
    track_cursor_time_in_beats: f64,
}

impl RiffSetTrackCustomPainter {
    pub fn new(state: Arc<Mutex<DAWState>>) -> RiffSetTrackCustomPainter {
        RiffSetTrackCustomPainter {
            state,
            track_cursor_time_in_beats: 0.0,
        }
    }
}

impl CustomPainter for RiffSetTrackCustomPainter {
    fn paint_custom(&mut self, context: &Context, height: f64, width: f64, entity_height_in_pixels: f64,
                    beat_width_in_pixels: f64, 
                    zoom_horizontal: f64,
                    zoom_vertical: f64,
                    _select_window_top_left_x: f64, 
                    _select_window_top_left_y: f64, 
                    _select_window_bottom_right_x: f64, 
                    _select_window_bottom_right_y: f64,
                    drawing_area_widget_name: Option<String>,
                    _mouse_pointer_x: f64,
                    _mouse_pointer_y: f64,
                    _mouse_pointer_previous_x: f64,
                    _mouse_pointer_previous_y: f64,
                    _draw_mode_on: bool,
                    _draw_mode: DrawMode,
                    _draw_mode_start_x: f64,
                    _draw_mode_start_y: f64,
                    _draw_mode_end_x: f64,
                    _draw_mode_end_y: f64,
                    _drawing_area: &DrawingArea,
                    _operation_mode: &OperationModeType,
                    _drag_started: bool,
                    _edit_drag_cycle: &DragCycle,
                    _tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
                ) {
        // debug!("RiffSetTrackCustomPainter::paint_custom - entered");
        match self.state.lock() {
            Ok( state) => {
                let mut adjusted_beat_width_in_pixels = beat_width_in_pixels * zoom_horizontal;
                let _adjusted_entity_height_in_pixels = entity_height_in_pixels * zoom_vertical;

                if let Some(drawing_area_widget_name) = drawing_area_widget_name {
                    if drawing_area_widget_name.starts_with("riffset_") {
                        let segments = drawing_area_widget_name.split('_').collect_vec();

                        if segments.len() == 3 {
                            let riff_set_uuid = *segments.get(1).unwrap();
                            let riff_set_number = if let Some((position, _)) = state.project().song().riff_sets().iter().find_position(|riff_set| riff_set.uuid() == riff_set_uuid.to_string()) {
                                position as i32 + 1
                            }
                            else {
                                1
                            };
                            let track_uuid = *segments.get(2).unwrap();
                            let (riff_ref_linked_to, mode) = {
                                if let Some(riff_set) = state.project().song().riff_set(riff_set_uuid.to_string()) {
                                    if let Some(riff_ref) = riff_set.riff_refs().get(track_uuid.clone()) {
                                        (riff_ref.linked_to(), riff_ref.mode().clone())
                                    }
                                    else {
                                        ("".to_string(), RiffReferenceMode::Normal)
                                    }
                                }
                                else {
                                    ("".to_string(), RiffReferenceMode::Normal)
                                }
                            };

                            let number_of_beats_in_bar = state.project().song().time_signature_denominator();
                            let mut state = state;
                            let mut track = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid);

                            // get the track
                            match track {
                                Some(track) => {
                                    let track_colour = track.colour_mut();

                                    // get the riff
                                    if let Some(riff) = track.riffs_mut().iter_mut().find(|current_riff| current_riff.uuid().to_string() == riff_ref_linked_to) {
                                        if riff.name() != "empty" {
                                            if let Some((red, green, blue, alpha)) = riff.colour() {
                                                context.set_source_rgba(*red, *green, *blue, *alpha);
                                            }
                                            else {
                                                let (red, green, blue, alpha) = track_colour;
                                                context.set_source_rgba(red, green, blue, alpha);
                                            }

                                            // also zoom out to fit the entire riff
                                            let riff_width_in_pixels = riff.length() * adjusted_beat_width_in_pixels;
                                            if riff_width_in_pixels > width {
                                                let zoom_factor = riff_width_in_pixels / width;
                                                adjusted_beat_width_in_pixels = beat_width_in_pixels * zoom_horizontal / zoom_factor;
                                            }
                                        }
                                        else {
                                            context.set_source_rgba(0.5, 0.5, 0.5, 1.0);
                                        }

                                        context.rectangle(0.0, 0.0, width, height);
                                        let _ = context.fill();

                                        let mut use_note = match mode {
                                            RiffReferenceMode::Normal => true,
                                            RiffReferenceMode::Start => false,
                                            RiffReferenceMode::End => true,
                                        };
                                        context.set_source_rgba(0.0, 0.0, 0.0, 1.0);
                                        for track_event in riff.events() {

                                            match track_event {
                                                TrackEvent::Note(note) => {
                                                    use_note = match &mode {
                                                        RiffReferenceMode::Start => {
                                                            if !use_note && note.riff_start_note() { true }
                                                            else if use_note { true }
                                                            else { false }
                                                        }
                                                        RiffReferenceMode::End => {
                                                            if use_note && note.riff_start_note() { false }
                                                            else if !use_note { false }
                                                            else { true }
                                                        }
                                                        RiffReferenceMode::Normal => true,
                                                    };

                                                    if use_note {
                                                        let note_number = note.note();
                                                        let note_y_pos_inverted = note_number as f64 * entity_height_in_pixels + entity_height_in_pixels;
                                                        // let duration_in_beats = note.duration() * adjusted_beat_width_in_pixels;
                                                        let x = note.position() * adjusted_beat_width_in_pixels;
                                                        let y = height - note_y_pos_inverted;
                                                        let width = note.length() * adjusted_beat_width_in_pixels;
                                                        context.rectangle(x, y, width, entity_height_in_pixels);
                                                        let _ = context.fill();
                                                    }
                                                },
                                                TrackEvent::Sample(sample) => {
                                                    context.set_source_rgba(1.0, 0.0, 0.0, 1.0);
                                                    let sample_y_pos = 0.0;
                                                    let x = sample.position() * adjusted_beat_width_in_pixels;
                                                    let width = 1.0 * adjusted_beat_width_in_pixels;
                                                    context.rectangle(x, sample_y_pos, width, height);
                                                    let _ = context.fill();
                                                }
                                                _ => (),
                                            }
                                        }

                                        // draw the riff name
                                        context.set_source_rgba(0.0, 0.0, 0.0, 1.0);
                                        context.move_to(1.0, 15.0);
                                        context.set_font_size(9.0);
                                        if riff.name() == "empty" {
                                            let _ = context.show_text("e");
                                            let _ = context.stroke();
                                        }
                                        else {
                                            let _ = context.show_text(riff.name());
                                            let _ = context.stroke();

                                            // draw the track cursor
                                            let x = (self.track_cursor_time_in_beats as i32 % riff.length() as i32) as f64 * adjusted_beat_width_in_pixels;
                                            context.set_source_rgba(0.0, 0.0, 1.0, 1.0);
                                            context.move_to(x, 0.0);
                                            context.line_to(x, height);
                                            let _ = context.stroke();

                                            // draw the riff length
                                            context.move_to(width - 20.0, height - 1.0);
                                            context.set_source_rgba(0.0, 0.0, 0.0, 1.0);
                                            let _ = context.show_text(format!("{}", riff.length() / number_of_beats_in_bar).as_str());
                                            let _ = context.stroke();

                                            // draw the riff set number
                                            context.move_to(width / 2.0 - 18.0, height / 2.0 + 14.0);
                                            context.set_source_rgba(0.0, 0.0, 0.0, 0.1);
                                            context.set_font_size(36.0);
                                            let _ = context.show_text(format!("{}", riff_set_number).as_str());
                                            let _ = context.stroke();

                                            // draw the riff reference play mode
                                            context.set_font_size(9.0);
                                            context.move_to(1.0, height - 1.0);
                                            context.set_source_rgba(0.0, 0.0, 0.0, 1.0);
                                            if let RiffReferenceMode::Start = mode {
                                                let _ = context.show_text("start");
                                                let _ = context.stroke();
                                            }
                                            else if let RiffReferenceMode::End = mode {
                                                let _ = context.show_text("end");
                                                let _ = context.stroke();
                                            }
                                        }

                                    }
                                },
                                None => (),
                            }
                        }
                    }
                }
            },
            Err(_) => debug!("Riff set track custom painter could not get state lock."),
        }

        // debug!("RiffSetTrackCustomPainter::paint_custom - entered");
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

pub struct PianoRollVerticalScaleCustomPainter {
    state: Arc<Mutex<DAWState>>,
}

impl PianoRollVerticalScaleCustomPainter {
    pub fn new(state: Arc<Mutex<DAWState>>) -> Self {
        Self {
            state,
        }
    }
}

impl CustomPainter for PianoRollVerticalScaleCustomPainter {
    fn paint_custom(&mut self, context: &Context, height: f64, width: f64, entity_height_in_pixels: f64,
                    _beat_width_in_pixels: f64,
                    _zoom_horizontal: f64,
                    zoom_vertical: f64,
                    _select_window_top_left_x: f64, 
                    _select_window_top_left_y: f64, 
                    _select_window_bottom_right_x: f64, 
                    _select_window_bottom_right_y: f64,
                    _drawing_area_widget_name: Option<String>,
                    _mouse_pointer_x: f64,
                    _mouse_pointer_y: f64,
                    _mouse_pointer_previous_x: f64,
                    _mouse_pointer_previous_y: f64,
                    _draw_mode_on: bool,
                    _draw_mode: DrawMode,
                    _draw_mode_start_x: f64,
                    _draw_mode_start_y: f64,
                    _draw_mode_end_x: f64,
                    _draw_mode_end_y: f64,
                    _drawing_area: &DrawingArea,
                    _operation_mode: &OperationModeType,
                    _drag_started: bool,
                    _edit_drag_cycle: &DragCycle,
                    _tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
                ) {
        context.set_source_rgba(0.9, 0.9, 0.9, 0.5);
        let adjusted_entity_height_in_pixels = entity_height_in_pixels * zoom_vertical;

        let mut current_y = 0.0;
        let mut row_number = 7;
        while current_y < height {
            if row_number == 2 || row_number == 4 || row_number == 7 || row_number == 9 || row_number == 11 {
                context.rectangle(0.0, current_y, width, adjusted_entity_height_in_pixels);
                let _ = context.fill();
            }
            else {
                context.move_to(0.0, current_y);
                context.line_to(width, current_y);
                let _ = context.stroke();
            }

            current_y += adjusted_entity_height_in_pixels;

            if row_number == 1 {
                row_number = 12;
            }
            else {
                row_number -= 1;
            }
        }
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


pub struct TrackGridCustomPainter {
    state: Arc<Mutex<DAWState>>,
    show_automation: bool,
    show_note: bool,
    show_note_velocity: bool,
    show_pan: bool,
    pub original_riff: Option<Riff>,
    pub dragged_riff: Option<Riff>,
    pub edit_item_handler: EditItemHandler<Riff, RiffReference>,
}

impl TrackGridCustomPainter {
    pub fn new_with_edit_item_handler(state: Arc<Mutex<DAWState>>, edit_item_handler: EditItemHandler<Riff, RiffReference>) -> TrackGridCustomPainter {
        TrackGridCustomPainter {
            state,
            show_automation: false,
            show_note: true,
            show_note_velocity: false,
            show_pan: false,
            original_riff: None,
            dragged_riff: None,
            edit_item_handler,
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
                    context: &Context, 
                    canvas_height: f64, 
                    _canvas_width: f64, 
                    entity_height_in_pixels: f64,
                    beat_width_in_pixels: f64, 
                    zoom_horizontal: f64, 
                    zoom_vertical: f64, 
                    select_window_top_left_x: f64, 
                    select_window_top_left_y: f64, 
                    select_window_bottom_right_x: f64, 
                    select_window_bottom_right_y: f64,
                    _drawing_area_widget_name: Option<String>,
                    mouse_pointer_x: f64,
                    mouse_pointer_y: f64,
                    mouse_pointer_previous_x: f64,
                    mouse_pointer_previous_y: f64,
                    _draw_mode_on: bool,
                    _draw_mode: DrawMode,
                    _draw_mode_start_x: f64,
                    _draw_mode_start_y: f64,
                    _draw_mode_end_x: f64,
                    _draw_mode_end_y: f64,
                    drawing_area: &DrawingArea,
                    operation_mode: &OperationModeType,
                    _drag_started: bool,
                    edit_drag_cycle: &DragCycle,
                    tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
                ) {
        let (clip_x1, clip_y1, clip_x2, clip_y2) = context.clip_extents().unwrap();
        // debug!("TrackGridCustomPainter::paint_custom - entered...: clip_x1={}, clip_y1={}, clip_x2={}, clip_y2={},", clip_x1, clip_y1, clip_x2, clip_y2);
        let clip_rectangle = Rect::new(
            coord!{x: clip_x1, y: clip_y1}, 
            coord!{x: clip_x2, y: clip_y2}
        );

        match self.state.lock() {
            Ok(mut state) => {
                let adjusted_beat_width_in_pixels = beat_width_in_pixels * zoom_horizontal;
                let adjusted_entity_height_in_pixels = entity_height_in_pixels * zoom_vertical;

                // find all the selected notes
                let selected_riff_ref_ids = state.selected_riff_references().clone();
                let mut selected_riff_references = vec![];

                for (index, track) in state.get_project().song_mut().tracks_mut().iter_mut().enumerate() {
                    for riff_reference in track.riff_refs().iter().filter(|riff_ref| selected_riff_ref_ids.clone().contains(&riff_ref.uuid().to_string())) {
                        if let Some(riff) = track.riffs().iter().find(|riff| riff.id() == riff_reference.linked_to()) {
                            let mut riff = riff.clone();
                            riff.set_id(riff_reference.id());
                            riff.set_position(riff_reference.position());
                            riff.set_vertical_index(index as i32);
                            selected_riff_references.push(riff);
                        }
                    }
                }

                for (track_number, track) in state.get_project().song_mut().tracks_mut().iter_mut().enumerate() {
                    let riff_refs = track.riff_refs();
                    let automation = track.automation().events();
                    let (red, green, blue, alpha) = track.colour();

                    for riff_ref in riff_refs.iter() {
                        let linked_to_riff_uuid = riff_ref.linked_to();

                        let is_selected = selected_riff_ref_ids.iter().any(|id| *id == riff_ref.uuid().to_string());
                        if is_selected {
                            context.set_source_rgba(0.0, 0.0, 1.0, 1.0);
                        }
                        else {
                            context.set_source_rgba(red, green, blue, alpha);
                        }

                        for riff in track.riffs().iter() {
                            if riff.uuid().to_string() == linked_to_riff_uuid {
                                let mut riff = riff.clone();
                                riff.set_id(riff_ref.id());
                                riff.set_position(riff_ref.position());
                                let mut use_notes = match riff_ref.mode() {
                                    RiffReferenceMode::Normal => true,
                                    RiffReferenceMode::Start => false,
                                    RiffReferenceMode::End => true,
                                };
                                if let Some((red, green, blue, alpha)) = riff.colour() {
                                    context.set_source_rgba(*red, *green, *blue, *alpha);
                                }

                                let x = riff_ref.position() * adjusted_beat_width_in_pixels;
                                let y = track_number as f64 * adjusted_entity_height_in_pixels;
                                let duration_in_beats = riff.length();
                                let width = duration_in_beats * beat_width_in_pixels * zoom_horizontal;

                                let riff_rect = Rect::new(
                                    coord!{x: x, y: y}, 
                                    coord!{x: x + width, y: y + adjusted_entity_height_in_pixels}
                                );
                        
                                // debug!("Part: x1={}, y1={}, x2={}, y2={},", x, y, x + width, y + adjusted_entity_height_in_pixels);

                                // if x >= clip_x1 && x <= clip_x2 && y >= clip_y1 && y <= clip_y2 {
                                if riff_rect.intersects(&clip_rectangle) {
                                    // debug!("Part in clip region");

                                    self.edit_item_handler.handle_item_edit(
                                        context, 
                                        &riff,
                                        operation_mode, 
                                        mouse_pointer_x, 
                                        mouse_pointer_y, 
                                        mouse_pointer_previous_x, 
                                        mouse_pointer_previous_y, 
                                        adjusted_entity_height_in_pixels, 
                                        adjusted_beat_width_in_pixels, 
                                        x, 
                                        y, 
                                        width, 
                                        canvas_height, 
                                        drawing_area, 
                                        edit_drag_cycle, 
                                        tx_from_ui.clone(), 
                                        false,
                                        track.uuid().to_string(),
                                        riff_ref,
                                        false,
                                        track_number as f64,
                                        is_selected,
                                        selected_riff_references.clone()
                                    );

                                    context.rectangle(x - 1.0, y + 1.0, width - 2.0, adjusted_entity_height_in_pixels - 2.0);
                                    let _ = context.fill();
                                    context.set_source_rgba(0.0, 0.0, 0.0, 1.0);
                                    context.rectangle(x - 1.0, y + 1.0, width - 2.0, adjusted_entity_height_in_pixels - 2.0);
                                    context.move_to(x + 5.0, y + 15.0);
                                    context.set_font_size(9.0);
                                    let mut name = riff.name().to_string();
                                    let mut name_fits = false;
                                    while !name_fits {
                                        if let Ok(text_extents) = context.text_extents(name.as_str()) {
                                            if (width - 2.0) < (text_extents.width as f64 + 10.0) {
                                                if !name.is_empty() {
                                                    name = name.as_str()[0..name.len() - 1].to_string();
                                                }
                                                else {
                                                    name_fits = true;
                                                    break;
                                                }
                                            }
                                            else {
                                                name_fits = true;
                                                break;
                                            }
                                        }
                                    }
                                    let _ = context.show_text(name.as_str());
                                    context.set_source_rgba(0.0, 0.0, 0.0, 1.0);
                                    let _ = context.stroke();

                                    context.move_to(x, y + 8.0);
                                    if let RiffReferenceMode::Start = riff_ref.mode() {
                                        let _ = context.show_text("s");
                                        let _ = context.stroke();
                                    }
                                    else if let RiffReferenceMode::End = riff_ref.mode() {
                                        let _ = context.show_text("e");
                                        let _ = context.stroke();
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
                                                        let note_y = track_number as f64 * adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels - (adjusted_entity_height_in_pixels / 127.0 * note.note() as f64);
                                                        // debug!("Note: x={}, y={}, width={}, height={}", note_x, note_y, note.duration() * adjusted_beat_width_in_pixels, entity_height_in_pixels / 127.0);
                                                        context.set_source_rgba(0.0, 0.0, 0.0, 1.0);
                                                        context.rectangle(note_x, note_y, note.length() * adjusted_beat_width_in_pixels, adjusted_entity_height_in_pixels / 127.0);
                                                        let _ = context.fill();
                                                    }

                                                    // draw velocity
                                                    if self.show_note_velocity {
                                                        context.set_source_rgba(0.0, 0.0, 0.0, 1.0);
                                                        let velocity_y_start = track_number as f64 * adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels;
                                                        // debug!("Note velocity: x={}, y={}, height={}", note_x, velocity_y_start, velocity_y_start - (entity_height_in_pixels / 127.0 * note.velocity() as f64));
                                                        context.move_to(note_x, velocity_y_start);
                                                        context.line_to(note_x, velocity_y_start - (adjusted_entity_height_in_pixels / 127.0 * note.velocity() as f64));
                                                        let _ = context.stroke();
                                                    }
                                                }
                                            },
                                            TrackEvent::NoteOn(_) => (),
                                            TrackEvent::NoteOff(_) => (),
                                            TrackEvent::Controller(controller) => {
                                                let x_position = (riff_ref.position() + controller.position()) * adjusted_beat_width_in_pixels;
                                                let y_start = track_number as f64 * adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels;

                                                context.set_source_rgba(0.0, 0.0, 0.0, 1.0);
                                                context.move_to(x_position, y_start);
                                                context.line_to(x_position, y_start - (adjusted_entity_height_in_pixels / 127.0 * (controller.value() as f64) as f64));
                                                let _ = context.stroke();
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
                                //     debug!("Part not in clip region");
                                // }
                                break;
                            }
                        }
                    }

                    if self.show_automation {
                        for track_event in automation.iter() {
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
                                        context.set_source_rgba(0.0, 0.0, 0.0, 1.0);
                                        let y_start = track_number as f64 * adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels;
                                        context.move_to(x_position, y_start);
                                        context.line_to(x_position, y_start - (adjusted_entity_height_in_pixels / 127.0 * (controller.value() as f64) as f64));
                                        let _ = context.stroke();
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

                if state.looping() {
                    if let Some(active_loop_uuid) =  state.active_loop() {
                        if let Some(active_loop) = state.project().song().loops().iter().find(|current_loop| current_loop.uuid().to_string() == active_loop_uuid.to_string()) {
                            let start_x = active_loop.start_position() * adjusted_beat_width_in_pixels;
                            let end_x = active_loop.end_position() * adjusted_beat_width_in_pixels;
                            context.set_source_rgba(0.0, 1.0, 0.0, 0.1);
                            context.rectangle(start_x, 0.0, end_x - start_x, canvas_height);
                            match context.fill() {
                                Ok(_) => (),
                                Err(_) => (),
                            }
                        }
                    }
                }
            },
            Err(_) => debug!("Track grid custom painter could not get state lock."),
        }
        // debug!("TrackGridCustomPainter::paint_custom - exited.");
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
    state: Arc<Mutex<DAWState>>,
    show_automation: bool,
    show_note: bool,
    show_note_velocity: bool,
    show_pan: bool,
    pub original_riff: Option<Riff>,
    pub dragged_riff: Option<Riff>,
    pub edit_item_handler: EditItemHandler<Riff, RiffReference>,
    use_globally_selected_riff_grid: bool,
    riff_grid_uuid: Option<String>,
    track_cursor_time_in_beats: f64,
}

impl RiffGridCustomPainter {
    pub fn new_with_edit_item_handler(
        state: Arc<Mutex<DAWState>>,
        edit_item_handler: EditItemHandler<Riff, RiffReference>,
        use_globally_selected_riff_grid: bool,
        riff_grid_uuid: Option<String>,
    ) -> RiffGridCustomPainter {
        RiffGridCustomPainter {
            state,
            show_automation: false,
            show_note: true,
            show_note_velocity: false,
            show_pan: false,
            original_riff: None,
            dragged_riff: None,
            edit_item_handler,
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
                    context: &Context,
                    canvas_height: f64,
                    _canvas_width: f64,
                    entity_height_in_pixels: f64,
                    beat_width_in_pixels: f64,
                    zoom_horizontal: f64,
                    zoom_vertical: f64,
                    select_window_top_left_x: f64,
                    select_window_top_left_y: f64,
                    select_window_bottom_right_x: f64,
                    select_window_bottom_right_y: f64,
                    _drawing_area_widget_name: Option<String>,
                    mouse_pointer_x: f64,
                    mouse_pointer_y: f64,
                    mouse_pointer_previous_x: f64,
                    mouse_pointer_previous_y: f64,
                    _draw_mode_on: bool,
                    _draw_mode: DrawMode,
                    _draw_mode_start_x: f64,
                    _draw_mode_start_y: f64,
                    _draw_mode_end_x: f64,
                    _draw_mode_end_y: f64,
                    drawing_area: &DrawingArea,
                    operation_mode: &OperationModeType,
                    _drag_started: bool,
                    edit_drag_cycle: &DragCycle,
                    tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
    ) {
        let (clip_x1, clip_y1, clip_x2, clip_y2) = context.clip_extents().unwrap();
        // debug!("RiffGridCustomPainter::paint_custom - entered...: clip_x1={}, clip_y1={}, clip_x2={}, clip_y2={},", clip_x1, clip_y1, clip_x2, clip_y2);
        let clip_rectangle = Rect::new(
            coord!{x: clip_x1, y: clip_y1},
            coord!{x: clip_x2, y: clip_y2}
        );

        match self.state.lock() {
            Ok(mut state) => {
                let adjusted_beat_width_in_pixels = beat_width_in_pixels * zoom_horizontal;
                let adjusted_entity_height_in_pixels = entity_height_in_pixels * zoom_vertical;
                let riff_grid_uuid_to_paint = if !self.use_globally_selected_riff_grid {
                    if let Some(riff_grid_uuid) = &self.riff_grid_uuid {
                        riff_grid_uuid.to_string()
                    }
                    else {
                        "".to_string()
                    }
                }
                else if let Some(selected_riff_grid_uuid) = state.selected_riff_grid_uuid() {
                    selected_riff_grid_uuid.to_string()
                }
                else {
                    "".to_string()
                };

                // find all the selected riff refs
                let selected_riff_ref_ids = state.selected_riff_references().clone();
                let mut riff_lengths = HashMap::new();
                for track in state.project().song().tracks().iter() {
                    for riff in track.riffs().iter() {
                        riff_lengths.insert(riff.id(), riff.length());
                    }
                }

                if let Some(riff_grid) = state.project().song().riff_grid(riff_grid_uuid_to_paint) {
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

                    for (index, track) in state.project().song().tracks().iter().enumerate() {
                        let track_number = index as f64;
                        let (red, green, blue, alpha) = track.colour();

                        if let Some(riff_refs) = riff_grid.track_riff_references(track.uuid().to_string()) {
                            for riff_ref in riff_refs.iter() {
                                let linked_to_riff_uuid = riff_ref.linked_to();
                                let is_selected = selected_riff_ref_ids.iter().any(|id| *id == riff_ref.uuid().to_string());
                                if is_selected {
                                    context.set_source_rgba(0.0, 0.0, 1.0, 1.0);
                                }
                                else {
                                    context.set_source_rgba(red, green, blue, alpha);
                                }

                                if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == linked_to_riff_uuid) {
                                    let mut use_notes = match riff_ref.mode() {
                                        RiffReferenceMode::Normal => true,
                                        RiffReferenceMode::Start => false,
                                        RiffReferenceMode::End => true,
                                    };
                                    if let Some((red, green, blue, alpha)) = riff.colour() {
                                        context.set_source_rgba(*red, *green, *blue, *alpha);
                                    }

                                    let x = riff_ref.position() * adjusted_beat_width_in_pixels;
                                    let y = track_number * adjusted_entity_height_in_pixels;
                                    let duration_in_beats = riff.length();
                                    let width = duration_in_beats * beat_width_in_pixels * zoom_horizontal;

                                    let riff_rect = Rect::new(
                                        coord! {x: x, y: y},
                                        coord! {x: x + width, y: y + adjusted_entity_height_in_pixels}
                                    );

                                    // debug!("Part: uuid={}, x1={}, y1={}, x2={}, y2={},", riff_ref.uuid().to_string(), x, y, x + width, y + adjusted_entity_height_in_pixels);

                                    // if x >= clip_x1 && x <= clip_x2 && y >= clip_y1 && y <= clip_y2 {
                                    if riff_rect.intersects(&clip_rectangle) {
                                        // debug!("Part in clip region");

                                        self.edit_item_handler.handle_item_edit(
                                            context,
                                            riff,
                                            operation_mode,
                                            mouse_pointer_x,
                                            mouse_pointer_y,
                                            mouse_pointer_previous_x,
                                            mouse_pointer_previous_y,
                                            adjusted_entity_height_in_pixels,
                                            adjusted_beat_width_in_pixels,
                                            x,
                                            y,
                                            width,
                                            canvas_height,
                                            drawing_area,
                                            edit_drag_cycle,
                                            tx_from_ui.clone(),
                                            false,
                                            track.uuid().to_string(),
                                            riff_ref,
                                            false,
                                            track_number,
                                            is_selected,
                                            selected_riff_references.clone()
                                        );

                                        context.rectangle(x - 1.0, y + 1.0, width - 2.0, adjusted_entity_height_in_pixels - 2.0);
                                        let _ = context.fill();
                                        context.set_source_rgba(0.0, 0.0, 0.0, 1.0);
                                        context.rectangle(x - 1.0, y + 1.0, width - 2.0, adjusted_entity_height_in_pixels - 2.0);
                                        context.move_to(x + 5.0, y + 15.0);
                                        context.set_font_size(9.0);
                                        let mut name = riff.name().to_string();
                                        let mut name_fits = false;
                                        while !name_fits {
                                            if let Ok(text_extents) = context.text_extents(name.as_str()) {
                                                if (width - 2.0) < (text_extents.width as f64 + 10.0) {
                                                    if !name.is_empty() {
                                                        name = name.as_str()[0..name.len() - 1].to_string();
                                                    } else {
                                                        name_fits = true;
                                                        break;
                                                    }
                                                } else {
                                                    name_fits = true;
                                                    break;
                                                }
                                            }
                                        }
                                        let _ = context.show_text(name.as_str());
                                        context.set_source_rgba(0.0, 0.0, 0.0, 1.0);
                                        let _ = context.stroke();

                                        context.move_to(x, y + 8.0);
                                        if let RiffReferenceMode::Start = riff_ref.mode() {
                                            let _ = context.show_text("s");
                                            let _ = context.stroke();
                                        }
                                        else if let RiffReferenceMode::End = riff_ref.mode() {
                                            let _ = context.show_text("e");
                                            let _ = context.stroke();
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
                                                            let note_y = track_number * adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels - (adjusted_entity_height_in_pixels / 127.0 * note.note() as f64);
                                                            // debug!("Note: x={}, y={}, width={}, height={}", note_x, note_y, note.duration() * adjusted_beat_width_in_pixels, entity_height_in_pixels / 127.0);
                                                            context.set_source_rgba(0.0, 0.0, 0.0, 1.0);
                                                            context.rectangle(note_x, note_y, note.length() * adjusted_beat_width_in_pixels, adjusted_entity_height_in_pixels / 127.0);
                                                            let _ = context.fill();
                                                        }

                                                        // draw velocity
                                                        if self.show_note_velocity {
                                                            context.set_source_rgba(0.0, 0.0, 0.0, 1.0);
                                                            let velocity_y_start = track_number * adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels;
                                                            // debug!("Note velocity: x={}, y={}, height={}", note_x, velocity_y_start, velocity_y_start - (entity_height_in_pixels / 127.0 * note.velocity() as f64));
                                                            context.move_to(note_x, velocity_y_start);
                                                            context.line_to(note_x, velocity_y_start - (adjusted_entity_height_in_pixels / 127.0 * note.velocity() as f64));
                                                            let _ = context.stroke();
                                                        }
                                                    }
                                                },
                                                TrackEvent::NoteOn(_) => (),
                                                TrackEvent::NoteOff(_) => (),
                                                TrackEvent::Controller(controller) => {
                                                    let x_position = (riff_ref.position() + controller.position()) * adjusted_beat_width_in_pixels;
                                                    let y_start = track_number * adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels;

                                                    context.set_source_rgba(0.0, 0.0, 0.0, 1.0);
                                                    context.move_to(x_position, y_start);
                                                    context.line_to(x_position, y_start - (adjusted_entity_height_in_pixels / 127.0 * (controller.value() as f64) as f64));
                                                    let _ = context.stroke();
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
            },
            Err(_) => debug!("Riff grid custom painter could not get state lock."),
        }
        // debug!("RiffGridCustomPainter::paint_custom - exited.");
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

pub struct AutomationCustomPainter {
    state: Arc<Mutex<DAWState>>,
}

impl AutomationCustomPainter {
    pub fn new(state: Arc<Mutex<DAWState>>) -> AutomationCustomPainter {
        AutomationCustomPainter {
            state,
        }
    }

    fn draw_track(context: &Context, height: f64, entity_height_in_pixels: f64, beat_width_in_pixels: f64, zoom: f64, adjusted_beat_width_in_pixels: f64, track: &TrackType, riff_refs: &Vec<RiffReference>) {
        let (red, green, blue, _) = track.colour();

        // draw the track name

        for riff_ref in riff_refs.iter() {
            let linked_to_riff_uuid = riff_ref.linked_to();

            for riff in track.riffs().iter() {
                if let Some((red, green, blue, _)) = riff.colour() {
                    context.set_source_rgba(*red, *green, *blue, 1.0);
                }
                else {
                    context.set_source_rgba(red, green, blue, 1.0);
                }

                if riff.uuid().to_string() == linked_to_riff_uuid {
                    let duration_in_beats = riff.length();
                    let x = riff_ref.position() * adjusted_beat_width_in_pixels;
                    let y = height / 2.0;
                    let width = duration_in_beats * beat_width_in_pixels * zoom;

                    // draw the riff ref rectangle
                    context.rectangle(x + 1.0, y + 1.0, width - 2.0, entity_height_in_pixels * 15.0 - 2.0);
                    let _ = context.fill();

                    // draw the riff name
                    context.set_source_rgba(0.0, 0.0, 0.0, 1.0);
                    context.move_to(x + 5.0, y + 10.0);
                    context.set_font_size(9.0);
                    let mut name = riff.name().to_string();
                    let mut name_fits = false;
                    while !name_fits {
                        if let Ok(text_extents) = context.text_extents(name.as_str()) {
                            if (width - 2.0) < (text_extents.width as f64 + 10.0) {
                                if !name.is_empty() {
                                    name = name.as_str()[0..name.len() - 1].to_string();
                                } else {
                                    name_fits = true;
                                    break;
                                }
                            } else {
                                name_fits = true;
                                break;
                            }
                        }
                    }
                    let _ = context.show_text(name.as_str());

                    // draw the notes
                    for track_event in riff.events() {
                        if let TrackEvent::Note(note) = track_event {
                            let note_x = (riff_ref.position() + note.position()) * adjusted_beat_width_in_pixels;

                            // draw note
                            let note_y = height / 2.0 + entity_height_in_pixels * 15.0 - (entity_height_in_pixels * 15.0 / 127.0 * note.note() as f64);
                            context.move_to(note_x, note_y);
                            context.line_to(note_x + note.length() * adjusted_beat_width_in_pixels, note_y);
                            let _ = context.stroke();
                        }
                    }
                }
            }
        }
    }

    fn draw_track_name(context: &Context, name: &str) {
        context.set_source_rgba(0.0, 0.0, 0.0, 1.0);
        context.move_to(5.0, 10.0);
        context.set_font_size(9.0);
        let _ = context.show_text(format!("Track: {}", name).as_str());
    }

    fn draw_riff_name(context: &Context, name: &str) {
        context.set_source_rgba(0.0, 0.0, 0.0, 1.0);
        context.move_to(5.0, 20.0);
        context.set_font_size(9.0);
        let _ = context.show_text(format!("Riff: {}", name).as_str());
    }

    fn draw_line(context: &Context, x_start: f64, y_start: f64, x_end: f64, y_end: f64) {
        context.set_source_rgba(0.0, 0.0, 0.0, 1.0);
        context.move_to(x_start, y_start);
        context.line_to(x_end, y_end);
        let _ = context.stroke();
    }
}

impl CustomPainter for AutomationCustomPainter {
    fn paint_custom(&mut self,
                    context: &Context,
                    height: f64,
                    _width: f64,
                    entity_height_in_pixels: f64,
                    beat_width_in_pixels: f64,
                    zoom_horizontal: f64,
                    zoom_vertical: f64,
                    select_window_top_left_x: f64, 
                    select_window_top_left_y: f64, 
                    select_window_bottom_right_x: f64, 
                    select_window_bottom_right_y: f64,
                    _drawing_area_widget_name: Option<String>,
                    _mouse_pointer_x: f64,
                    _mouse_pointer_y: f64,
                    _mouse_pointer_previous_x: f64,
                    _mouse_pointer_previous_y: f64,
                    draw_mode_on: bool,
                    draw_mode: DrawMode,
                    draw_mode_start_x: f64,
                    draw_mode_start_y: f64,
                    draw_mode_end_x: f64,
                    draw_mode_end_y: f64,
                    _drawing_area: &DrawingArea,
                    _operation_mode: &OperationModeType,
                    _drag_started: bool,
                    _edit_drag_cycle: &DragCycle,
                    _tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
                ) {
        match self.state.lock() {
            Ok(mut state) => {
                let automation_type = *state.automation_type_mut();
                let note_expression_type = state.note_expression_type_mut().clone();
                let note_expression_note_id = state.note_expression_id();
                let adjusted_beat_width_in_pixels = beat_width_in_pixels * zoom_horizontal;
                let adjusted_entity_height_in_pixels = entity_height_in_pixels * zoom_vertical;
                let type_to_show = state.automation_view_mode();
                let current_view = state.current_view();

                if let crate::state::AutomationViewMode::NoteVelocities = type_to_show {
                    match state.selected_track() {
                        Some(track_uuid) => match state.selected_riff_uuid(track_uuid.clone()) {
                            Some(riff_uuid) => match state.project().song().tracks().iter().find(|track| track.uuid().to_string() == track_uuid) {
                                Some(track) => {
                                    // let (red, green, blue, _) = track.colour();
                                    let red = 0.0;
                                    let green = 0.0;
                                    let blue = 0.0;
                                    let name = track.name().to_string();

                                    // draw the track name
                                    Self::draw_track_name(context, name.as_str());

                                    for riff in track.riffs().iter() {
                                        if riff.uuid().to_string() == riff_uuid {
                                            Self::draw_riff_name(context, riff.name());

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
                                                        let note_number = note.note();
                                                        let note_velocity = note.velocity();
                                                        let note_velocity_y_pos_inverted = note_velocity as f64 * adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels;
                                                        let note_y_pos_inverted = note_number as f64 * adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels;
                                                        let x = note.position() * adjusted_beat_width_in_pixels;
                                                        let y_velocity = height - note_velocity_y_pos_inverted;
                                                        let y_note = height - note_y_pos_inverted;
                                                        let note_width = note.length() * adjusted_beat_width_in_pixels;

                                                        let is_selected = state.selected_automation().iter().any(|id| id.as_str() == note.id().as_str());
                                                        if is_selected {
                                                            event_colour = (0.0, 0.0, 1.0, 1.0);
                                                        }
                                                        context.set_source_rgba(event_colour.0, event_colour.1, event_colour.2, event_colour.3);
                                                        context.move_to(x, height);
                                                        context.line_to(x, y_velocity + 5.0);
                                                        match context.stroke() {
                                                            Ok(_) => (),
                                                            Err(error) => debug!("Problem drawing note velocity in controller view: {:?}", error),
                                                        }
                                                        context.arc(x, y_velocity, 5.0, 0.0, 6.3 /* 2 * PI */);
                                                        match context.fill() {
                                                            Ok(_) => (),
                                                            Err(error) => debug!("Problem drawing note velocity circle in controller view: {:?}", error),
                                                        }
                                                        context.rectangle(x, y_note, note_width, adjusted_entity_height_in_pixels);
                                                        let _ = context.fill();
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
                    if let Some(track_uuid) = state.selected_track() {
                        if let Some(track) = state.project().song().tracks().iter().find(|track| track.uuid().to_string() == track_uuid) {
                            // let (red, green, blue, _) = track.colour();
                            let red = 0.0;
                            let green = 0.0;
                            let blue = 0.0;
                            let name = track.name().to_string();

                            // draw the track name
                            Self::draw_track_name(context, name.as_str());

                            let events = if let CurrentView::RiffArrangement = current_view {
                                // get the arrangement
                                if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                                    if let Some(riff_arrangement) = state.project().song().riff_arrangement(selected_arrangement_uuid.clone()){
                                        if let Some(riff_arr_automation) = riff_arrangement.automation(&track_uuid) {
                                            Some(riff_arr_automation.events())
                                        }
                                        else {
                                            None
                                        }
                                    }
                                    else {
                                        None
                                    }
                                }
                                else {
                                    None
                                }
                            }
                            else {
                                match state.automation_edit_type() {
                                    AutomationEditType::Track => {
                                        Some(track.automation().events())
                                    }
                                    AutomationEditType::Riff => {
                                        if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
                                            if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == selected_riff_uuid) {
                                                Self::draw_riff_name(context, riff.name());
                                                Some(riff.events_vec())
                                            }
                                            else {
                                                None
                                            }
                                        }
                                        else {
                                            None
                                        }
                                    }
                                }
                            };
                
                            if let Some(events) = events {
                                if let CurrentView::Track = current_view {
                                    let riff_refs = track.riff_refs();
                                    Self::draw_track(context, height, adjusted_entity_height_in_pixels, beat_width_in_pixels, zoom_horizontal, adjusted_beat_width_in_pixels, track, riff_refs);
                                }

                                let unselected_event_colour = (red, green, blue, 1.0);

                                for track_event in events.iter() {
                                    let mut event_colour = unselected_event_colour.clone();

                                    match type_to_show {
                                        crate::state::AutomationViewMode::Controllers => {
                                            if let TrackEvent::Controller(controller) = track_event {
                                                if let Some(automation_type_value) = automation_type {
                                                    if controller.controller() == automation_type_value {
                                                        let controller_value = controller.value();
                                                        let note_y_pos_inverted = controller_value as f64 * adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels;
                                                        let x = controller.position() * adjusted_beat_width_in_pixels;
                                                        let y = height - note_y_pos_inverted;

                                                        let is_selected = state.selected_automation().iter().any(|id| {
                                                            id.as_str() == controller.id().as_str()
                                                        });
                                                        if is_selected {
                                                            event_colour = (0.0, 0.0, 1.0, 1.0);
                                                        }
                                                        context.set_source_rgba(event_colour.0, event_colour.1, event_colour.2, event_colour.3);

                                                        context.move_to(x, height);
                                                        context.line_to(x, y);

                                                        match context.stroke() {
                                                            Ok(_) => (),
                                                            Err(error) => debug!("Problem drawing not controller in the automation view: {:?}", error),
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        crate::state::AutomationViewMode::PitchBend => {
                                            if let TrackEvent::PitchBend(pitch_bend) = track_event {
                                                let pitch_bend_value = pitch_bend.value();
                                                let note_y_pos_inverted = ((pitch_bend_value as f64 + 8192.0) / 16384.0 * 127.0) * adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels;
                                                let x = pitch_bend.position() * adjusted_beat_width_in_pixels;
                                                let y = height - note_y_pos_inverted;

                                                let is_selected = state.selected_automation().iter().any(|id| {
                                                    id.as_str() == pitch_bend.id().as_str()
                                                });
                                                if is_selected {
                                                    event_colour = (0.0, 0.0, 1.0, 1.0);
                                                }
                                                context.set_source_rgba(event_colour.0, event_colour.1, event_colour.2, event_colour.3);

                                                context.move_to(x, height / 2.0);
                                                context.line_to(x, y);

                                                match context.stroke() {
                                                    Ok(_) => (),
                                                    Err(error) => debug!("Problem drawing not pitch bend in the automation view: {:?}", error),
                                                }
                                            }
                                        }
                                        crate::state::AutomationViewMode::Instrument => {
                                            if let TrackEvent::AudioPluginParameter(audio_plugin_parameter) = track_event {
                                                if let Some(automation_type_value) = automation_type {
                                                    if audio_plugin_parameter.index == automation_type_value && audio_plugin_parameter.instrument() {
                                                        let parameter_value = audio_plugin_parameter.value();
                                                        let note_y_pos_inverted = parameter_value as f64 * 127.0 *  adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels;
                                                        let x = audio_plugin_parameter.position() * adjusted_beat_width_in_pixels;
                                                        let y = height - note_y_pos_inverted;
                                                        let width = 1.0;

                                                        let is_selected = state.selected_automation().iter().any(|id| id.as_str() == audio_plugin_parameter.id().as_str());
                                                        if is_selected {
                                                            event_colour = (0.0, 0.0, 1.0, 1.0);
                                                        }
                                                        context.set_source_rgba(event_colour.0, event_colour.1, event_colour.2, event_colour.3);

                                                        context.move_to(x, height);
                                                        context.line_to(x, y);

                                                        match context.stroke() {
                                                            Ok(_) => (),
                                                            Err(error) => debug!("Problem drawing instrument plugin parameter in the automation view: {:?}", error),
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        crate::state::AutomationViewMode::Effect => {
                                            if let TrackEvent::AudioPluginParameter(audio_plugin_parameter) = track_event {
                                                if let Some(selected_effect_plugin_uuid) = state.selected_effect_plugin_uuid() {
                                                    if let Some(automation_type_value) = automation_type {
                                                        if audio_plugin_parameter.index == automation_type_value &&
                                                            !audio_plugin_parameter.instrument() &&
                                                            audio_plugin_parameter.plugin_uuid() == *selected_effect_plugin_uuid {
                                                            let parameter_value = audio_plugin_parameter.value();
                                                            let note_y_pos_inverted = parameter_value as f64 * 127.0 *  adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels;
                                                            let x = audio_plugin_parameter.position() * adjusted_beat_width_in_pixels;
                                                            let y = height - note_y_pos_inverted;
                                                            let width = 1.0;

                                                            let is_selected = state.selected_automation().iter().any(|id| id.as_str() == audio_plugin_parameter.id().as_str());
                                                            if is_selected {
                                                                event_colour = (0.0, 0.0, 1.0, 1.0);
                                                            }
                                                            context.set_source_rgba(event_colour.0, event_colour.1, event_colour.2, event_colour.3);
    
                                                            context.move_to(x, height);
                                                            context.line_to(x, y);

                                                            match context.stroke() {
                                                                Ok(_) => (),
                                                                Err(error) => debug!("Problem drawing effect plugin parameter in the automation view: {:?}", error),
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        crate::state::AutomationViewMode::NoteExpression => {
                                            if let TrackEvent::NoteExpression(note_expression) = track_event {
                                                if note_expression_type as i32 == *(note_expression.expression_type()) as i32 && (note_expression_note_id == -1 || note_expression_note_id == note_expression.note_id())  {
                                                    let note_expression_value = note_expression.value();
                                                    let note_y_pos_inverted = note_expression_value as f64 * 127.0 *  adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels;
                                                    let x = note_expression.position() * adjusted_beat_width_in_pixels;
                                                    let y = height - note_y_pos_inverted;
                                                    let width = 1.0;

                                                    let is_selected = state.selected_automation().iter().any(|id| id.as_str() == note_expression.id().as_str());
                                                    if is_selected {
                                                        event_colour = (0.0, 0.0, 1.0, 1.0);
                                                    }
                                                    context.set_source_rgba(event_colour.0, event_colour.1, event_colour.2, event_colour.3);

                                                    context.move_to(x, height);
                                                    context.line_to(x, y);

                                                    match context.stroke() {
                                                        Ok(_) => (),
                                                        Err(error) => debug!("Problem drawing note expression value in the automation view: {:?}", error),
                                                    }
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                }
            },
            Err(_) => debug!("Piano Roll custom painter could not get state lock."),
        }

        if draw_mode_on {
            if let DrawMode::Line = draw_mode {
                context.set_source_rgba(0.0, 0.0, 0.0, 1.0);
                Self::draw_line(context, draw_mode_start_x, draw_mode_start_y, draw_mode_end_x, draw_mode_end_y);
            }
        }
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

pub struct AutomationMouseCoordHelper;

impl BeatGridMouseCoordHelper for AutomationMouseCoordHelper {
    fn get_entity_vertical_value(&self, y: f64, entity_height_in_pixels: f64, zoom_vertical: f64) -> f64 {
        let value = ((127.0 * entity_height_in_pixels * zoom_vertical) - y) / (entity_height_in_pixels * zoom_vertical);
        if value < 0.0 {
            0.0
        }
        else {
            value
        }
    }

    fn add_entity(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, y_index: i32, time: f64, _duration: f64, _entity_uuid: String) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::AutomationAdd(time, y_index), None));
    }

    fn delete_entity(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, _y_index: i32, time: f64, _entity_uuid: String) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::AutomationDelete(time), None));
    }

    fn cut_selected(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::AutomationCut, None));
    }

    fn copy_selected(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::AutomationCopy, None));
    }

    fn paste_selected(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::AutomationPaste, None));
    }

    fn handle_translate_up(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::AutomationTranslateSelected(TranslationEntityType::Any, TranslateDirection::Up), None));
    }

    fn handle_translate_down(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::AutomationTranslateSelected(TranslationEntityType::Any, TranslateDirection::Down), None));
    }

    fn handle_translate_left(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::AutomationTranslateSelected(TranslationEntityType::Any, TranslateDirection::Left), None));
    }

    fn handle_translate_right(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::AutomationTranslateSelected(TranslationEntityType::Any, TranslateDirection::Right), None));
    }

    fn handle_quantise(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::AutomationQuantiseSelected, None));
    }

    fn handle_increase_entity_length(&self, _tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {

    }

    fn handle_decrease_entity_length(&self, _tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {

    }

    fn select(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, x: f64, y: i32, x2: f64, y2: i32, add_to_select: bool) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::AutomationSelected(x, y2, x2, y, add_to_select), None));
    }

    fn set_start_note(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64) {
    }

    fn set_riff_reference_play_mode(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64) {
    }
}
