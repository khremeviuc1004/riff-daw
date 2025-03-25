use std::{sync::{Arc, Mutex}, vec::Vec};
use std::any::Any;
use std::collections::HashMap;
use std::sync::MutexGuard;
use cairo::{Context};
use crossbeam_channel::Sender;
use gtk::{DrawingArea, prelude::*};
use itertools::Itertools;
use log::*;
use strum_macros::Display;
use uuid::Uuid;
use geo::{coord, Intersects, Rect};
use gtk::glib::ffi::G_PI;
use crate::{domain::*, event::{DAWEvents, LoopChangeType, OperationModeType, TrackChangeType, TranslateDirection, TranslationEntityType, AutomationEditType}, state::DAWState, constants::NOTE_NAMES};
use crate::event::{CurrentView, RiffGridChangeType};
use crate::event::TrackChangeType::RiffReferencePlayMode;
use crate::state::AutomationViewMode;
use crate::utils::DAWUtils;

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
    Triplet,
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
    fn paint_zoom_window(&mut self, context: &Context, height: f64, width: f64);
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
        let calculated_snap = DAWUtils::quantise(time, snap, 1.0, false);
        if calculated_snap.snapped {
            calculated_snap.snapped_value
        }
        else {
            time
        }
    }

    fn get_time(&self, x: f64, beat_width_in_pixels: f64, zoom_horizontal: f64) -> f64 {
        x / (beat_width_in_pixels * zoom_horizontal)
    }
    fn select_single(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, x: f64, y: i32, add_to_select: bool);
    fn select_multiple(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, x: f64, y: i32, x2: f64, y2: i32, add_to_select: bool);
    fn deselect_single(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, x: f64, y: i32);
    fn deselect_multiple(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, x: f64, y: i32, x2: f64, y2: i32);
    fn add_entity(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, y_index: i32, time: f64, duration: f64, entity_uuid: String);
    fn add_entity_extra(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, y_index: i32, time: f64, duration: f64, entity_uuid: String);
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
    fn handle_windowed_zoom(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, x1: f64, y1: f64, x2: f64, y2: f64);
    fn cycle_entity_selection(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, y_index: i32, time: f64);
    fn select_underlying_entity(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, y_index: i32, time: f64);
}

pub struct PianoRollMouseCoordHelper;

impl BeatGridMouseCoordHelper for PianoRollMouseCoordHelper {
    fn get_entity_vertical_value(&self, y: f64, entity_height_in_pixels: f64, zoom_vertical: f64) -> f64 {
        ((127.0 * entity_height_in_pixels * zoom_vertical) - y) / (entity_height_in_pixels * zoom_vertical)
    }

    fn select_single(&self, tx_from_ui: Sender<DAWEvents>, x: f64, y: i32, add_to_select: bool) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffEventsSelectSingle(x, y, add_to_select), None));
    }

    fn select_multiple(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, x: f64, y: i32, x2: f64, y2: i32, add_to_select: bool) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffEventsSelectMultiple(x, y2, x2, y, add_to_select), None));
    }

    fn deselect_single(&self, tx_from_ui: Sender<DAWEvents>, x: f64, y: i32) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffEventsDeselectSingle(x, y), None));
    }

    fn deselect_multiple(&self, tx_from_ui: Sender<DAWEvents>, x: f64, y: i32, x2: f64, y2: i32) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffEventsDeselectMultiple(x, y2, x2, y), None));
    }

    fn add_entity(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, y_index: i32, time: f64, duration: f64, _entity_uuid: String) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffAddNote(vec![(y_index, time, duration)]), None));
    }

    fn add_entity_extra(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64, duration: f64, entity_uuid: String) {
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

    fn set_start_note(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffSetStartNote(y_index, time), None));
    }

    fn set_riff_reference_play_mode(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffReferencePlayMode(y_index, time), None));
    }

    fn handle_windowed_zoom(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, x1: f64, y1: f64, x2: f64, y2: f64) {
        let _ = tx_from_ui.send(DAWEvents::PianoRollWindowedZoom {x1, y1, x2, y2});
    }

    fn cycle_entity_selection(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64) {
    }

    fn select_underlying_entity(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64) {
    }
}

pub struct SampleRollMouseCoordHelper;

impl BeatGridMouseCoordHelper for SampleRollMouseCoordHelper {
    fn get_entity_vertical_value(&self, y: f64, entity_height_in_pixels: f64, zoom_vertical: f64) -> f64 {
        ((127.0 * entity_height_in_pixels * zoom_vertical) - y) / (entity_height_in_pixels * zoom_vertical)
    }

    fn select_single(&self, tx_from_ui: Sender<DAWEvents>, x: f64, y: i32, add_to_select: bool) {
    }

    fn select_multiple(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, x: f64, y: i32, x2: f64, y2: i32, add_to_select: bool) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::AutomationSelectMultiple(x, y2, x2, y, add_to_select), None));
    }

    fn deselect_single(&self, tx_from_ui: Sender<DAWEvents>, x: f64, y: i32) {
        todo!()
    }

    fn deselect_multiple(&self, tx_from_ui: Sender<DAWEvents>, x: f64, y: i32, x2: f64, y2: i32) {
    }

    fn add_entity(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, _y_index: i32, time: f64, _duration: f64, entity_uuid: String) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffAddSample(entity_uuid, time), None));
    }

    fn add_entity_extra(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64, duration: f64, entity_uuid: String) {
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

    fn set_start_note(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64) {
    }

    fn set_riff_reference_play_mode(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64) {
    }

    fn handle_windowed_zoom(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, x1: f64, y1: f64, x2: f64, y2: f64) {
    }

    fn cycle_entity_selection(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64) {
    }

    fn select_underlying_entity(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64) {
    }
}

pub struct TrackGridMouseCoordHelper;

impl BeatGridMouseCoordHelper for TrackGridMouseCoordHelper {
    fn get_entity_vertical_value(&self, y: f64, entity_height_in_pixels: f64, zoom_vertical: f64) -> f64 {
        y / entity_height_in_pixels * zoom_vertical
    }

    fn select_single(&self, tx_from_ui: Sender<DAWEvents>, x: f64, y: i32, add_to_select: bool) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffReferencesSelectSingle(x, y, add_to_select), None));
    }

    fn select_multiple(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, x: f64, y: i32, x2: f64, y2: i32, add_to_select: bool) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffReferencesSelectMultiple(x, y, x2, y2, add_to_select), None));
    }

    fn deselect_single(&self, tx_from_ui: Sender<DAWEvents>, x: f64, y: i32) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffReferencesDeselectSingle(x, y), None));
    }

    fn deselect_multiple(&self, tx_from_ui: Sender<DAWEvents>, x: f64, y: i32, x2: f64, y2: i32) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffReferencesDeselectMultiple(x, y, x2, y2), None));
    }

    fn add_entity(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, y_index: i32, time: f64, _duration: f64, _entity_uuid: String) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffReferenceAdd(y_index, time), None));
    }

    fn add_entity_extra(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64, duration: f64, entity_uuid: String) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffAddWithTrackIndex(entity_uuid, duration, y_index), None));
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

    fn set_start_note(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64) {
    }

    fn set_riff_reference_play_mode(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffReferencePlayMode(y_index, time), None));
    }

    fn handle_windowed_zoom(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, x1: f64, y1: f64, x2: f64, y2: f64) {
    }

    fn cycle_entity_selection(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffReferenceIncrementRiff{track_index: y_index, position: time}, None));
    }

    fn select_underlying_entity(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffSelectWithTrackIndex{track_index: y_index, position: time}, None));
    }
}

pub struct RiffGridMouseCoordHelper;

impl BeatGridMouseCoordHelper for RiffGridMouseCoordHelper {
    fn get_entity_vertical_value(&self, y: f64, entity_height_in_pixels: f64, zoom_vertical: f64) -> f64 {
        y / entity_height_in_pixels * zoom_vertical
    }

    fn select_single(&self, tx_from_ui: Sender<DAWEvents>, x: f64, y: i32, add_to_select: bool) {
        let _ = tx_from_ui.send(DAWEvents::RiffGridChange(RiffGridChangeType::RiffReferencesSelectSingle(x, y, add_to_select), None));
    }

    fn select_multiple(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, x: f64, y: i32, x2: f64, y2: i32, add_to_select: bool) {
        let _ = tx_from_ui.send(DAWEvents::RiffGridChange(RiffGridChangeType::RiffReferencesSelectMultiple(x, y, x2, y2, add_to_select), None));
    }

    fn deselect_single(&self, tx_from_ui: Sender<DAWEvents>, x: f64, y: i32) {
        let _ = tx_from_ui.send(DAWEvents::RiffGridChange(RiffGridChangeType::RiffReferencesDeselectSingle(x, y), None));
    }

    fn deselect_multiple(&self, tx_from_ui: Sender<DAWEvents>, x: f64, y: i32, x2: f64, y2: i32) {
        let _ = tx_from_ui.send(DAWEvents::RiffGridChange(RiffGridChangeType::RiffReferencesDeselectMultiple(x, y, x2, y2), None));
    }

    fn add_entity(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, y_index: i32, time: f64, _duration: f64, _entity_uuid: String) {
        let _ = tx_from_ui.send(DAWEvents::RiffGridChange(RiffGridChangeType::RiffReferenceAdd{ track_index: y_index, position: time }, None));
    }

    fn add_entity_extra(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64, duration: f64, entity_uuid: String) {
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

    fn set_start_note(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64) {
    }

    fn set_riff_reference_play_mode(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffReferencePlayMode(y_index, time), None));
    }

    fn handle_windowed_zoom(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, x1: f64, y1: f64, x2: f64, y2: f64) {
    }

    fn cycle_entity_selection(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64) {
        let _ = tx_from_ui.send(DAWEvents::RiffGridChange(RiffGridChangeType::RiffReferenceIncrementRiff{track_index: y_index, position: time}, None));
    }

    fn select_underlying_entity(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64) {
        let _ = tx_from_ui.send(DAWEvents::RiffGridChange(RiffGridChangeType::RiffSelectWithTrackIndex{track_index: y_index, position: time}, None));
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
    pub windowed_zoom_drag_cycle: DragCycle,

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
            windowed_zoom_drag_cycle: DragCycle::NotStarted,

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
            windowed_zoom_drag_cycle: DragCycle::NotStarted,

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
            windowed_zoom_drag_cycle: DragCycle::NotStarted,

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

    pub fn get_zoom_window(&self) -> (f64, f64, f64, f64) {
        // find the top left x and y
        let top_left_x = self.x_zoom_window_position.min(self.x_zoom_window_position2);
        let top_left_y = self.y_zoom_window_position.min(self.y_zoom_window_position2);

        // find the bottom right x and y
        let bottom_right_x = self.x_zoom_window_position.max(self.x_zoom_window_position2);
        let bottom_right_y = self.y_zoom_window_position.max(self.y_zoom_window_position2);

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

    pub fn snap_strength(&self) -> f64 {
        self.snap_strength
    }

    pub fn set_snap_strength(&mut self, snap_strength: f64) {
        self.snap_strength = snap_strength;
    }

    pub fn set_beats_per_bar(&mut self, beats_per_bar: i32) {
        self.beats_per_bar = beats_per_bar;
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
                    OperationModeType::WindowedZoom => {
                        self.x_zoom_window_position2 = x;
                        self.y_zoom_window_position2 = y;
                        self.draw_zoom_window = true;
                        self.windowed_zoom_drag_cycle = DragCycle::Dragging;
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
                    OperationModeType::WindowedZoom => {
                        self.x_zoom_window_position = x;
                        self.y_zoom_window_position = y;
                        self.windowed_zoom_drag_cycle = DragCycle::MousePressed;
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
                                else if let DrawMode::Triplet = self.draw_mode {
                                    let y_index = mouse_coord_helper.get_entity_vertical_value(y, self.entity_height_in_pixels, self.zoom_vertical);
                                    let position = mouse_coord_helper.get_time(x, self.beat_width_in_pixels, self.zoom_horizontal);
                                    let snap_position = mouse_coord_helper.get_snapped_to_time(self.snap_position_in_beats, position);
                                    let duration = self.new_entity_length_in_beats - 0.01; // take off just a little off so that the note off does not overlap the next note on

                                    mouse_coord_helper.add_entity(self.tx_from_ui.clone(), y_index as i32, snap_position, duration, data.clone());
                                    mouse_coord_helper.add_entity(self.tx_from_ui.clone(), y_index as i32, snap_position + self.triplet_spacing_in_beats, duration, data.clone());
                                    mouse_coord_helper.add_entity(self.tx_from_ui.clone(), y_index as i32, snap_position + (self.triplet_spacing_in_beats * 2.0), duration, data);
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
                        if control_key {
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

                                if shift_key {
                                    mouse_coord_helper.deselect_multiple(
                                        self.tx_from_ui.clone(),
                                        mouse_coord_helper.get_time(select_window.0, self.beat_width_in_pixels, self.zoom_horizontal),
                                        mouse_coord_helper.get_entity_vertical_value(select_window.1, self.entity_height_in_pixels, self.zoom_vertical) as i32,
                                        mouse_coord_helper.get_time(select_window.2, self.beat_width_in_pixels, self.zoom_horizontal),
                                        mouse_coord_helper.get_entity_vertical_value(select_window.3, self.entity_height_in_pixels, self.zoom_vertical) as i32
                                    );
                                }
                                else {
                                    let add_to_select = control_key; // should this be the shift key???
                                    mouse_coord_helper.select_multiple(
                                        self.tx_from_ui.clone(),
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
                                    self.tx_from_ui.clone(),
                                    mouse_coord_helper.get_time(x, self.beat_width_in_pixels, self.zoom_horizontal),
                                    mouse_coord_helper.get_entity_vertical_value(y, self.entity_height_in_pixels, self.zoom_vertical) as i32,
                                );
                            }
                        }
                        else if control_key { // select single item
                            // send an event to the ui via the mouse coord helper
                            if let Some(mouse_coord_helper) = self.mouse_coord_helper.as_ref() {
                                mouse_coord_helper.select_single(
                                    self.tx_from_ui.clone(),
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

                                mouse_coord_helper.cycle_entity_selection(self.tx_from_ui.clone(), y_index as i32, position);
                            }
                        }
                        drawing_area.queue_draw();
                    }
                    OperationModeType::WindowedZoom => {
                        self.draw_zoom_window = false;
                        self.windowed_zoom_drag_cycle = DragCycle::NotStarted;
                        // send an event to the ui via the mouse coord helper
                        if let Some(mouse_coord_helper) = self.mouse_coord_helper.as_ref() {
                            let (x1, y1, x2, y2) = self.get_zoom_window();
                            mouse_coord_helper.handle_windowed_zoom(self.tx_from_ui.clone(), x1, y1, x2, y2);
                        }
                        drawing_area.queue_draw();
                    }
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
                    OperationModeType::Add => {
                        if let Some(mouse_coord_helper) = self.mouse_coord_helper.as_ref() {
                            let y_index = mouse_coord_helper.get_entity_vertical_value(y, self.entity_height_in_pixels, self.zoom_vertical);
                            let position = mouse_coord_helper.get_time(x, self.beat_width_in_pixels, self.zoom_horizontal);
                            let snap_position = mouse_coord_helper.get_snapped_to_time(self.snap_position_in_beats, position);
                            let duration = self.new_entity_length_in_beats - 0.01; // take off just a little off so that the note off does not overlap the next note on
                            let new_riff_uuid = Uuid::new_v4();

                            mouse_coord_helper.add_entity_extra(self.tx_from_ui.clone(), y_index as i32, snap_position, duration, new_riff_uuid.to_string());
                            mouse_coord_helper.add_entity(self.tx_from_ui.clone(), y_index as i32, snap_position, duration, new_riff_uuid.to_string());
                        }
                    }
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
                    }
                    OperationModeType::LoopPointMode => debug!("mouse button clicked=2, mode={:?}", self.operation_mode),
                    OperationModeType::SelectStartNote => {}
                    OperationModeType::SelectRiffReferenceMode => {}
                    OperationModeType::WindowedZoom => {}
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
                                let riff_uuid = *segments.get(1).unwrap();
                                let track_uuid = *segments.get(2).unwrap();
                                match self.tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffSelect(riff_uuid.to_owned()), Some(track_uuid.to_owned()))) {
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
                        else {
                            if let Some(mouse_coord_helper) = self.mouse_coord_helper.as_ref() {
                                let y_index = mouse_coord_helper.get_entity_vertical_value(y, self.entity_height_in_pixels, self.zoom_vertical);
                                let position = mouse_coord_helper.get_time(x, self.beat_width_in_pixels, self.zoom_horizontal);

                                mouse_coord_helper.select_underlying_entity(self.tx_from_ui.clone(), y_index as i32, position);
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
                    OperationModeType::SelectStartNote => {}
                    OperationModeType::SelectRiffReferenceMode => {}
                    OperationModeType::WindowedZoom => {}
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
        if self.draw_zoom_window {
            self.paint_zoom_window(context, height, width);
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
        let mut beat_in_bar_index = (clip_x1_in_beats as i32 % self.beats_per_bar) + 1;

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

            if beat_in_bar_index == self.beats_per_bar {
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

    fn paint_zoom_window(&mut self, context: &Context, _height: f64, _width: f64) {
        context.set_source_rgba(0.0, 0.0, 0.0, 0.5);
        let (top_left_x, top_left_y, bottom_right_x, bottom_right_y) = self.get_zoom_window();
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

    pub fn set_beats_per_bar(&mut self, beats_per_bar: i32) {
        self.beats_per_bar = beats_per_bar;
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
        let mut bar_index = (clip_x1_in_beats / (self.beats_per_bar as f64)) as i32 + 1; // get the bar
        let mut beat_in_bar_index = (clip_x1_in_beats as i32 % self.beats_per_bar) + 1;

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

            if beat_in_bar_index == self.beats_per_bar {
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

    fn paint_zoom_window(&mut self, _context: &Context, _height: f64, _width: f64) {
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
    pub edit_item_handler: EditItemHandler<Note, Note>,
}

impl PianoRollCustomPainter {
    pub fn new_with_edit_item_handler(state: Arc<Mutex<DAWState>>, edit_item_handler: EditItemHandler<Note, Note>) -> PianoRollCustomPainter {
        PianoRollCustomPainter {
            state,
            edit_item_handler,
        }
    }
}

impl CustomPainter for PianoRollCustomPainter {
    fn paint_custom(&mut self,
                    context: &Context,
                    height: f64,
                    width: f64,
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
                    drawing_area: &DrawingArea,
                    operation_mode: &OperationModeType,
                    drag_started: bool,
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
                                                            let y = height - note_y_pos_inverted;
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
                                                                height,
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
        let note_number = ((height - mouse_pointer_y) / adjusted_entity_height_in_pixels) as i32;
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
                    mouse_pointer_x >= x &&
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
                    mouse_pointer_x >= (x + 5.0) &&
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
                    mouse_pointer_x <= (x + width) &&
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


pub struct AutomationEditItemHandler {
    pub original_item: Option<TrackEvent>,
    pub original_item_is_selected: bool,
    pub original_selected_items: Vec<TrackEvent>,
    pub selected_item_ids: Vec<String>,
    pub dragged_item: Option<TrackEvent>,
    pub referenced_item: Option<TrackEvent>,
    pub changed_event_sender: Box<dyn Fn(Vec<(TrackEvent, TrackEvent)>, String, crossbeam_channel::Sender<DAWEvents>)>,
    pub copied_event_sender: Box<dyn Fn(Vec<TrackEvent>, String, crossbeam_channel::Sender<DAWEvents>)>,
    pub can_change_position: bool,
    pub can_drag_copy: bool,
}

impl AutomationEditItemHandler {
    pub fn new(
        changed_event_sender: Box<dyn Fn(Vec<(TrackEvent, TrackEvent)>, String, crossbeam_channel::Sender<DAWEvents>)>,
        copied_event_sender: Box<dyn Fn(Vec<TrackEvent>, String, crossbeam_channel::Sender<DAWEvents>)>,
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
            can_change_position: true,
            can_drag_copy,
        }
    }
}

impl AutomationEditItemHandler {
    pub fn handle_item_edit(
        &mut self,
        context: &Context,
        item: &TrackEvent,
        operation_mode: &OperationModeType,
        mouse_pointer_x: f64,
        mouse_pointer_y: f64,
        mouse_pointer_previous_x: f64,
        mouse_pointer_previous_y: f64,
        adjusted_entity_height_in_pixels: f64,
        adjusted_beat_width_in_pixels: f64,
        x_original: f64,
        y_original: f64,
        canvas_height: f64,
        drawing_area: &DrawingArea,
        edit_drag_cycle: &DragCycle,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        invert_vertically: bool,
        track_uuid: String,
        referencing_item: &TrackEvent,
        item_is_selected: bool,
        selected_items: Vec<TrackEvent>,
        previous_point_x: f64,
        previous_point_y: f64,
        automation_discrete: bool,
        selected_automation_b4_after_points: &HashMap<String, ((String, f64, f64), (String, f64, f64))>
    ) {
        let mut edit_mode = EditMode::Inactive;

        match operation_mode {
            OperationModeType::Change => {
                let mut x = x_original;
                let mut y = y_original;
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
                        (original_item.value() - item.value()).abs() < 1e-10 {
                        if let Some(dragged_item) = self.dragged_item.as_ref() {
                            if dragged_item.id() == referencing_item.id() {
                                found_item_being_changed = true;
                            }
                        }
                    }
                }

                if found_item_being_changed {
                    if let Some(dragged_item) = self.dragged_item.as_ref() {
                        debug!("Automation - Dragged item found.");
                        let vertical_y_position = dragged_item.value() * 127.0 * adjusted_entity_height_in_pixels;
                        x = dragged_item.position() * adjusted_beat_width_in_pixels + delta_x;

                        if invert_vertically {
                            y = canvas_height - vertical_y_position + delta_y - adjusted_entity_height_in_pixels;
                        }
                        else {
                            y = vertical_y_position + delta_y;
                        }
                    }
                }

                // draw drag position adjust handle if required
                if found_item_being_changed || ((self.can_change_position || self.can_drag_copy) &&
                    mouse_pointer_x >= (x - 5.0) &&
                    mouse_pointer_x <= (x + 5.0) &&
                    (canvas_height - mouse_pointer_y) >= (y - 5.0) &&
                    (canvas_height - mouse_pointer_y) <= (y + 5.0)) {
                    //change the mode
                    edit_mode = EditMode::Move;
                    use_this_item = true;

                    // change the prompt
                    if let Some(window) = drawing_area.window() {
                        window.set_cursor(Some(&gdk::Cursor::for_display(&window.display(), gdk::CursorType::Hand1)));
                        // debug!("Automation - drawing hand prompt.");
                    }
                }

                match edit_mode {
                    EditMode::Inactive => {
                        // debug!("Automation - EditMode::Inactive");
                    }
                    _ => {
                        match edit_drag_cycle {
                            DragCycle::MousePressed => {
                                debug!("Automation - handle_item_edit EditDragCycle::MousePressed");
                                if use_this_item {
                                    debug!("Automation - handle_item_edit EditDragCycle::MousePressed - set original and dragged items.");
                                    self.original_item = Some(item.clone());
                                    self.original_item_is_selected = item_is_selected;
                                    self.original_selected_items = selected_items;
                                    if item.id() != referencing_item.id() {
                                        let mut dragged_item = item.clone();

                                        dragged_item.set_id(referencing_item.id());
                                        dragged_item.set_position(referencing_item.position());
                                        dragged_item.set_value(referencing_item.value());
                                        self.dragged_item = Some(dragged_item);
                                    }
                                    else {
                                        self.dragged_item = Some(item.clone());
                                    }
                                }
                            }
                            DragCycle::Dragging => {
                                debug!("Automation - handle_item_edit EditDragCycle::Dragging");

                                if found_item_being_changed {
                                    if let EditMode::Move =  edit_mode {
                                        if let Some(dragged_item) = self.dragged_item.as_mut() {
                                            let delta_x = x - dragged_item.position() * adjusted_beat_width_in_pixels;
                                            let delta_y = y - (canvas_height - dragged_item.value() * 127.0 * adjusted_entity_height_in_pixels);
                                            let mut updated_selected_items: HashMap<String, (f64, f64)> = HashMap::new();

                                            // draw the dragged item
                                            if automation_discrete {
                                                context.move_to(x, canvas_height);
                                            }
                                            else {
                                                if let Some((b4_point, after_point)) = selected_automation_b4_after_points.get(&dragged_item.id()) {
                                                    // need a look ahead check for another selected item that positionally comes before this one and use it's new position as the b4 point
                                                    debug!("dragged id={}, x_b4_point={}, b4_point_y={}", dragged_item.id(), b4_point.0, b4_point.1);
                                                    if self.original_item_is_selected {
                                                        if let Some(b4_item) = self.original_selected_items.iter().find(|event| event.id() == b4_point.0.clone()) {
                                                            context.move_to(
                                                                b4_item.position() * adjusted_beat_width_in_pixels + delta_x,
                                                                if invert_vertically {
                                                                    canvas_height - b4_item.value() * 127.0 * adjusted_entity_height_in_pixels + delta_y
                                                                }
                                                                else {
                                                                    b4_item.value() * 127.0 * adjusted_entity_height_in_pixels + delta_y
                                                                }
                                                            );
                                                        }
                                                        else {
                                                            context.move_to(b4_point.1, b4_point.2);
                                                        }

                                                        updated_selected_items.insert(dragged_item.id(), (x, y));
                                                    }
                                                    else {
                                                        context.move_to(b4_point.1, b4_point.2);
                                                    }
                                                }
                                                else {
                                                    context.move_to(previous_point_x, previous_point_y);
                                                }
                                            }

                                            context.line_to(x, y);
                                            let _ = context.stroke();

                                            if ! automation_discrete {
                                                context.rectangle(x - 5.0, y - 5.0, 10.0, 10.0);
                                                let _ = context.fill();
                                            }

                                            // if the after point does not come from a selected item then draw its line - the square denoting the position will be drawn by the normal code
                                            if let Some((_, after_point)) = selected_automation_b4_after_points.get(&dragged_item.id()) {
                                                if !self.original_selected_items.iter().any(|event| event.id() == after_point.0.clone()) {
                                                    context.move_to(x, y);
                                                    context.line_to(after_point.1, after_point.2);
                                                    let _ = context.stroke();
                                                }
                                            }

                                            // draw the other selected items
                                            if self.original_item_is_selected {
                                                for item in self.original_selected_items.iter() {
                                                    if item.id() != dragged_item.id() {
                                                        let x = item.position() * adjusted_beat_width_in_pixels + delta_x;
                                                        let y = if invert_vertically {
                                                            canvas_height - item.value() * 127.0 * adjusted_entity_height_in_pixels + delta_y
                                                        }
                                                        else {
                                                            item.value() * 127.0 * adjusted_entity_height_in_pixels + delta_y
                                                        };

                                                        if automation_discrete {
                                                            context.move_to(x, canvas_height);
                                                        }
                                                        else {
                                                            if let Some((b4_point, after_point)) = selected_automation_b4_after_points.get(&item.id()) {
                                                                debug!("selected id={}, b4_point_id={}, b4_point_x={}, b4_point_y={}, after_point_id={}, after_point_x={}, after_point_y={}", item.id(), b4_point.0, b4_point.1, b4_point.2, after_point.0, after_point.1, after_point.2);
                                                                if let Some((b4_updated_x, b4_updated_y)) = updated_selected_items.get(&b4_point.0.clone()) {
                                                                    context.move_to(*b4_updated_x, *b4_updated_y);
                                                                }
                                                                else {
                                                                    context.move_to(b4_point.1, b4_point.2);
                                                                }

                                                                updated_selected_items.insert(item.id(), (x, y));

                                                            }
                                                            else {
                                                                context.move_to(previous_point_x, previous_point_y);
                                                            }
                                                        }

                                                        context.line_to(x, y);
                                                        let _ = context.stroke();

                                                        if ! automation_discrete {
                                                            context.rectangle(x - 5.0, y - 5.0, 10.0, 10.0);
                                                            let _ = context.fill();
                                                        }

                                                        // if the after point does not come from a selected item then draw its line - the square denoting the position will be drawn by the normal code
                                                        if let Some((_, after_point)) = selected_automation_b4_after_points.get(&item.id()) {
                                                            if !self.original_selected_items.iter().any(|event| event.id() == after_point.0.clone()) {
                                                                context.move_to(x, y);
                                                                context.line_to(after_point.1, after_point.2);
                                                                let _ = context.stroke();
                                                            }
                                                        }
                                                    }
                                                }
                                                debug!("updated_selected_items count={}", updated_selected_items.iter().count());
                                            }
                                        }
                                    }
                                }
                            }
                            DragCycle::MouseReleased => {
                                debug!("Automation - handle_item_edit EditDragCycle::MouseReleased");

                                if found_item_being_changed {
                                    if let Some(original_item) = self.original_item.as_ref() {
                                        if let Some(dragged_item) = self.dragged_item.as_mut() {
                                            if let EditMode::Move =  edit_mode {
                                                let mut change = vec![];
                                                // calculate and set the position
                                                let position_in_beats = x /adjusted_beat_width_in_pixels;
                                                dragged_item.set_position(position_in_beats);

                                                // calculate and set the value
                                                let mut value = if invert_vertically {
                                                    let y_pos_inverted = canvas_height - y;
                                                    ((y_pos_inverted - adjusted_entity_height_in_pixels) / adjusted_entity_height_in_pixels) / 127.0
                                                }
                                                else {
                                                    y / 127.0
                                                };

                                                if value < 0.0 {
                                                    value = 0.0;
                                                }

                                                debug!("Automation - Setting dragged item value to: {}", value);
                                                dragged_item.set_value(value);

                                                change.push((original_item.clone(), dragged_item.clone()));

                                                // handle the other selected items
                                                if self.original_item_is_selected {
                                                    let delta_x = position_in_beats - original_item.position();
                                                    let delta_y = value - original_item.value();
                                                    for item in self.original_selected_items.iter() {
                                                        if item.id() != dragged_item.id() {
                                                            let mut changed_item = item.clone();
                                                            let mut changed_item_value = changed_item.value() + delta_y;

                                                            if changed_item_value < 0.0 {
                                                                changed_item_value = 0.0;
                                                            }

                                                            changed_item.set_position(changed_item.position() + delta_x);
                                                            changed_item.set_value(changed_item.value() + delta_y);
                                                            change.push((item.clone(), changed_item));
                                                        }
                                                    }
                                                }

                                                if !change.is_empty() {
                                                    (self.changed_event_sender)(change, track_uuid.clone(), tx_from_ui.clone());
                                                }
                                            }
                                        }
                                    }

                                    debug!("Automation - handle_item_edit EditDragCycle::MouseReleased - unset original and dragged items.");
                                    self.original_item = None;
                                    self.original_selected_items.clear();
                                    self.selected_item_ids.clear();
                                    self.dragged_item = None;
                                }
                            }
                            DragCycle::CtrlMousePressed => {
                                debug!("Automation - handle_item_edit EditDragCycle::CtrlMousePressed");
                                if use_this_item {
                                    debug!("Automation - handle_item_edit EditDragCycle::CtrlMousePressed - set original and dragged items.");
                                    self.original_item = Some(item.clone());
                                    self.original_item_is_selected = item_is_selected;
                                    self.original_selected_items = selected_items;
                                    if item.id() != referencing_item.id() {
                                        let mut dragged_item = item.clone();

                                        dragged_item.set_id(referencing_item.id());
                                        dragged_item.set_position(referencing_item.position());
                                        dragged_item.set_value(referencing_item.value());
                                        self.dragged_item = Some(dragged_item);
                                    }
                                    else {
                                        self.dragged_item = Some(item.clone());
                                    }
                                }
                            }
                            DragCycle::CtrlDragging => {
                                debug!("Automation - handle_item_edit EditDragCycle::CtrlDragging");

                                if found_item_being_changed {
                                    if let EditMode::Move = edit_mode {
                                        if let Some(dragged_item) = self.dragged_item.as_mut() {
                                            // draw the dragged item
                                            if automation_discrete {
                                                context.move_to(x, canvas_height);
                                            }
                                            else {
                                                context.move_to(previous_point_x, previous_point_y);
                                            }
                                            context.line_to(x, y);
                                            let _ = context.stroke();

                                            if ! automation_discrete {
                                                context.rectangle(x - 5.0, y - 5.0, 10.0, 10.0);
                                                let _ = context.fill();
                                            }

                                            // draw the other selected items
                                            if self.original_item_is_selected {
                                                let delta_x = x - dragged_item.position() * adjusted_beat_width_in_pixels;
                                                let delta_y = y - (canvas_height - dragged_item.value() * 127.0 * adjusted_entity_height_in_pixels);
                                                for item in self.original_selected_items.iter() {
                                                    if item.id() != dragged_item.id() {
                                                        let x = item.position() * adjusted_beat_width_in_pixels + delta_x;
                                                        let y = if invert_vertically {
                                                            canvas_height - item.value() * 127.0 * adjusted_entity_height_in_pixels + delta_y
                                                        }
                                                        else {
                                                            item.value() * 127.0 * adjusted_entity_height_in_pixels + delta_y
                                                        };

                                                        if automation_discrete {
                                                            context.move_to(x, canvas_height);
                                                        }
                                                        else {
                                                            context.move_to(previous_point_x, previous_point_y);
                                                        }

                                                        context.line_to(x, y);
                                                        let _ = context.stroke();

                                                        if ! automation_discrete {
                                                            context.rectangle(x - 5.0, y - 5.0, 10.0, 10.0);
                                                            let _ = context.fill();
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            DragCycle::CtrlMouseReleased => {
                                debug!("Automation - handle_item_edit EditDragCycle::CtrlMouseReleased");

                                if found_item_being_changed {
                                    if let Some(original_item) = self.original_item.as_ref() {
                                        if let Some(dragged_item) = self.dragged_item.as_mut() {
                                            if let EditMode::Move = edit_mode {
                                                let mut copied = vec![];
                                                // calculate and set the position
                                                let position_in_beats = x /adjusted_beat_width_in_pixels;
                                                dragged_item.set_position(position_in_beats);

                                                // calculate and set the vertical index
                                                let value = if invert_vertically {
                                                    let y_pos_inverted = canvas_height - y;
                                                    ((y_pos_inverted - adjusted_entity_height_in_pixels) / adjusted_entity_height_in_pixels) / 127.0
                                                }
                                                else {
                                                    y / 127.0
                                                };

                                                debug!("Automation - Setting dragged item vertical index to: {}", value);
                                                dragged_item.set_value(value * 127.0);

                                                copied.push(dragged_item.clone());

                                                // handle the other selected items
                                                if self.original_item_is_selected {
                                                    let delta_x = position_in_beats - original_item.position();
                                                    let delta_y = value - original_item.value();
                                                    for item in self.original_selected_items.iter() {
                                                        if item.id() != dragged_item.id() {
                                                            let mut copied_item = item.clone();

                                                            copied_item.set_position(copied_item.position() + delta_x);
                                                            copied_item.set_value((copied_item.value() + delta_y) * 127.0);
                                                            copied.push(copied_item);
                                                        }
                                                    }
                                                }

                                                if !copied.is_empty() {
                                                    (self.copied_event_sender)(copied, track_uuid.clone(), tx_from_ui.clone());
                                                }
                                            }
                                        }
                                    }

                                    debug!("Automation - handle_item_edit EditDragCycle::CtrlMouseReleased - unset original and dragged items.");
                                    self.original_item = None;
                                    self.original_selected_items.clear();
                                    self.selected_item_ids.clear();
                                    self.dragged_item = None;
                                }
                            }
                            DragCycle::NotStarted => {
                                debug!("Automation - handle_item_edit EditDragCycle::NotStarted");
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
    fn paint_custom(&mut self, context: &Context, height: f64, width: f64, entity_height_in_pixels: f64,
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
                    drawing_area: &DrawingArea,
                    operation_mode: &OperationModeType,
                    drag_started: bool,
                    edit_drag_cycle: &DragCycle,
                    tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
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
                                                    // if select_window_top_left_x <= x && (x + width) <= select_window_bottom_right_x &&
                                                    //     select_window_top_left_y <= sample_y_pos && (sample_y_pos + entity_height_in_pixels) <= select_window_bottom_right_y {
                                                    //     context.set_source_rgb(0.0, 0.0, 1.0);
                                                    // }
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
                    drawing_area: &DrawingArea,
                    operation_mode: &OperationModeType,
                    drag_started: bool,
                    edit_drag_cycle: &DragCycle,
                    tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
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
                    height: f64,
                    width: f64,
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
                    drawing_area: &DrawingArea,
                    operation_mode: &OperationModeType,
                    drag_started: bool,
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
                let selected_riff_ref_ids = state.selected_track_grid_riff_references().clone();
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
                                        height,
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
                                                context.line_to(x_position, y_start - (adjusted_entity_height_in_pixels / 127.0 * (controller.value() as f64)));
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
                                        context.line_to(x_position, y_start - (adjusted_entity_height_in_pixels / 127.0 * (controller.value() as f64)));
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
                            context.rectangle(start_x, 0.0, end_x - start_x, height);
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
                    height: f64,
                    width: f64,
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
                    drawing_area: &DrawingArea,
                    operation_mode: &OperationModeType,
                    drag_started: bool,
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
                let selected_riff_ref_ids = state.selected_riff_grid_riff_references().clone();
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
                                            height,
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
                                                    context.line_to(x_position, y_start - (adjusted_entity_height_in_pixels / 127.0 * (controller.value() as f64)));
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
    pub edit_item_handler: AutomationEditItemHandler,
}

impl AutomationCustomPainter {
    pub fn new_with_edit_item_handler(state: Arc<Mutex<DAWState>>, edit_item_handler: AutomationEditItemHandler) -> AutomationCustomPainter {
        AutomationCustomPainter {
            state,
            edit_item_handler,
        }
    }

    fn draw_riff(context: &Context, height: f64, entity_height_in_pixels: f64, beat_width_in_pixels: f64, zoom: f64, adjusted_beat_width_in_pixels: f64, riff: &Riff, track: &TrackType) {
        let duration_in_beats = riff.length();
        let x = riff.position() * adjusted_beat_width_in_pixels;
        let y = height / 2.0;
        let width = duration_in_beats * beat_width_in_pixels * zoom;
        let (red, green, blue, alpha) = track.colour();

        // draw the riff ref rectangle
        context.set_source_rgba(red, green, blue, alpha);
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
                let note_x = (riff.position() + note.position()) * adjusted_beat_width_in_pixels;

                // draw note
                let note_y = height / 2.0 + entity_height_in_pixels * 15.0 - (entity_height_in_pixels * 15.0 / 127.0 * note.note() as f64);
                context.move_to(note_x, note_y);
                context.line_to(note_x + note.length() * adjusted_beat_width_in_pixels, note_y);
                let _ = context.stroke();
            }
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

    fn draw_automation(context: &Context, height: f64, automation_discrete: bool, mut previous_point_x: &mut f64, mut previous_point_y: &mut f64, default_line_width: f64, x: f64, y: f64, automation_value: f64) {
        if automation_discrete {
            context.move_to(x, height);
        } else {
            context.move_to(*previous_point_x, *previous_point_y);
            context.set_line_width(0.75);
        }

        context.line_to(x, y);
        let _ = context.stroke();
        context.set_line_width(default_line_width);

        if !automation_discrete {
            context.rectangle(x - 5.0, y - 5.0, 10.0, 10.0);
            let _ = context.fill();
            let _ = context.stroke();
        }

        context.move_to(x + 10.0, y);
        context.set_font_size(9.0);
        let _ = context.show_text(format!("{:.3}", automation_value).as_str());

        if !automation_discrete {
            *previous_point_x = x;
            *previous_point_y = y;
        }
    }

    fn draw_riff_set_riff_refs(
        context: &Context, height: f64,
        entity_height_in_pixels: f64,
        beat_width_in_pixels: f64,
        zoom_horizontal: f64,
        state: &MutexGuard<DAWState>,
        adjusted_beat_width_in_pixels: f64,
        track_uuid: &String,
        track: &&TrackType,
        running_position: &mut f64,
        riff_set_uuid: String
    ) {
        if let Some(riff_set) = state.project().song().riff_set(riff_set_uuid) {
            let mut riff_lengths = vec![];
            let mut riff_to_draw = Riff::new_with_position_length_and_colour(Uuid::new_v4(), 0.0, 4.0, None);

            // get the number of repeats
            for track in state.project().song().tracks().iter() {
                // get the riff_ref
                if let Some(riff_ref) = riff_set.get_riff_ref_for_track(track.uuid().to_string()) {
                    // get the riff
                    if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_ref.linked_to()) {
                        riff_lengths.push(riff.length() as i32);

                        if track_uuid.to_string() == track.uuid().to_string() {
                            riff_to_draw = riff.clone();
                        }
                    }
                }
            }

            let (product, unique_riff_lengths) = DAWState::get_length_product(riff_lengths);

            let lowest_common_factor_in_beats = DAWState::get_lowest_common_factor(unique_riff_lengths, product);

            // draw the riff reference x number of times
            for x in 0..((lowest_common_factor_in_beats as f64 / riff_to_draw.length()) as i32) {
                riff_to_draw.set_position(*running_position);
                if riff_to_draw.name() != "empty" {
                    Self::draw_riff(context, height, entity_height_in_pixels, beat_width_in_pixels, zoom_horizontal, adjusted_beat_width_in_pixels, &riff_to_draw, &track);
                }
                *running_position += riff_to_draw.length();
            }
        }
    }
}

impl CustomPainter for AutomationCustomPainter {
    fn paint_custom(&mut self,
                    context: &Context,
                    height: f64,
                    width: f64,
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
                    drawing_area: &DrawingArea,
                    operation_mode: &OperationModeType,
                    drag_started: bool,
                    edit_drag_cycle: &DragCycle,
                    tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
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
                let automation_discrete = state.automation_discrete();
                let selected_effect_uuid = if let Some(selected_effect_uuid) = state.selected_effect_plugin_uuid() {
                    selected_effect_uuid.clone()
                }
                else {
                    "".to_string()
                };

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

                            let events = match current_view {
                                CurrentView::Track => if let AutomationEditType::Track = state.automation_edit_type() {
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
                                                            if controller.controller() == automation_type_value {
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
                                                            instrument_track.instrument().uuid().to_string()
                                                        }
                                                        else {
                                                            "".to_string()
                                                        };
                                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                            if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid {
                                                                found = true;
                                                            }
                                                        }
                                                    }
                                                    AutomationViewMode::Effect => {
                                                        if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                            if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
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
                                CurrentView::RiffSet => if let AutomationEditType::Riff = state.automation_edit_type() {
                                    if let Some(selected_riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
                                        if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == selected_riff_uuid) {
                                            Self::draw_riff_name(context, riff.name());
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
                                    if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                                        if let Some(riff_arrangement) = state.project().song().riff_arrangement(selected_arrangement_uuid.clone()){
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
                                                                        if controller.controller() == automation_type_value {
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
                                                                        instrument_track.instrument().uuid().to_string()
                                                                    }
                                                                    else {
                                                                        "".to_string()
                                                                    };
                                                                    if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                                        if param.index == automation_type_value && param.plugin_uuid() == plugin_uuid {
                                                                            found = true;
                                                                        }
                                                                    }
                                                                }
                                                                AutomationViewMode::Effect => {
                                                                    if let TrackEvent::AudioPluginParameter(param) = envelope.event_details() {
                                                                        if param.index == automation_type_value && param.plugin_uuid() == selected_effect_uuid {
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
                                Self::draw_track(context, height, adjusted_entity_height_in_pixels, beat_width_in_pixels, zoom_horizontal, adjusted_beat_width_in_pixels, track, riff_refs);
                            }
                            else if let CurrentView::RiffArrangement = current_view { // draw the riff arrangement track riff refs if relevant so that the user can see where to place their automation
                                // get the current riff arrangement
                                if let Some(selected_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                                    if let Some(riff_arrangement) = state.project().song().riff_arrangement(selected_arrangement_uuid.clone()){
                                        // loop through all the riff sets, riff seqs and riff grids fetching the riff refs using the LCF on riff sets to get the number of iterations for a riff ref for a track
                                        let mut running_position = 0.0;
                                        for item in riff_arrangement.items().iter() {
                                            // instantiate and pack new riffs to represent the riff ref and the underlying riff all packed into one.
                                            match item.item_type() {
                                                RiffItemType::RiffSet => {
                                                    Self::draw_riff_set_riff_refs(
                                                        context,
                                                        height,
                                                        entity_height_in_pixels,
                                                        beat_width_in_pixels,
                                                        zoom_horizontal,
                                                        &state,
                                                        adjusted_beat_width_in_pixels,
                                                        &track_uuid,
                                                        &track,
                                                        &mut running_position,
                                                        item.item_uuid().to_string());
                                                }
                                                RiffItemType::RiffSequence => {
                                                    if let Some(riff_sequence) = state.project().song().riff_sequence(item.item_uuid().to_string()) {
                                                        for riff_set in riff_sequence.riff_sets().iter() {
                                                            Self::draw_riff_set_riff_refs(
                                                                context,
                                                                height,
                                                                entity_height_in_pixels,
                                                                beat_width_in_pixels,
                                                                zoom_horizontal,
                                                                &state,
                                                                adjusted_beat_width_in_pixels,
                                                                &track_uuid,
                                                                &track,
                                                                &mut running_position,
                                                                riff_set.item_uuid().to_string());
                                                        }
                                                    }
                                                }
                                                RiffItemType::RiffGrid => {
                                                    if let Some(riff_grid) = state.project().song().riff_grid(item.item_uuid().to_string()) {
                                                        let mut riff_grid_length = 0.0;
                                                        for track_uuid in riff_grid.tracks() {
                                                            if let Some(track) = state.project().song().tracks().iter().find(|track| track.uuid().to_string() == track_uuid.to_string()) {
                                                                if let Some(riff_refs) = riff_grid.track_riff_references(track_uuid.clone()) {
                                                                    for riff_ref in riff_refs.iter() {
                                                                        if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_ref.linked_to()) {
                                                                            let max_rightward_position = riff_ref.position() + riff.length();
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
                                                                    riff_to_draw.set_position(running_position + riff_ref.position());
                                                                    if riff_to_draw.name() != "empty" {
                                                                        Self::draw_riff(context, height, entity_height_in_pixels, beat_width_in_pixels, zoom_horizontal, adjusted_beat_width_in_pixels, &riff_to_draw, &track);
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
                                let default_line_width = context.line_width();
                                let selected_automation  = events.iter().filter(|event| state.selected_automation().contains(&event.id())).map(|event| event.clone()).collect_vec();
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
                                        if state.selected_automation().contains(&event.id()) {
                                            current_id = event.id();
                                            selected_automation_b4_after_points.insert(current_id.clone(), ((previous_point.0, previous_point.1, previous_point.2), ("".to_string(), 0.0, 0.0)));
                                            // debug!("id={}, x_pre={}, y_pre={}", current_id.as_str(), previous_point.0, previous_point.1);
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
                                // debug!("selected_automation_b4_after_points entry count={}", selected_automation_b4_after_points.iter().count());
                                // selected_automation_b4_after_points.iter().for_each(|(key, (pre, post))| debug!("id={}, x_pre={}, y_pre={}, x_post={}, y_post={}", key, pre.0, pre.1, post.0, post.1));
                                for track_event in events.iter() {
                                    let mut event_colour = unselected_event_colour.clone();
                                    let is_selected = state.selected_automation().iter().any(|id| {
                                        id.as_str() == track_event.id().as_str()
                                    });

                                    if is_selected {
                                        event_colour = (0.0, 0.0, 1.0, 1.0);
                                    }
                                    context.set_source_rgba(event_colour.0, event_colour.1, event_colour.2, event_colour.3);

                                    self.edit_item_handler.handle_item_edit(
                                        context,
                                        track_event,
                                        operation_mode,
                                        mouse_pointer_x,
                                        mouse_pointer_y,
                                        mouse_pointer_previous_x,
                                        mouse_pointer_previous_y,
                                        adjusted_entity_height_in_pixels,
                                        adjusted_beat_width_in_pixels,
                                        track_event.position() * adjusted_beat_width_in_pixels,
                                        track_event.value() * 127.0 * adjusted_entity_height_in_pixels,
                                        height,
                                        drawing_area,
                                        edit_drag_cycle,
                                        tx_from_ui.clone(),
                                        true,
                                        track_uuid.clone(),
                                        track_event,
                                        is_selected,
                                        selected_automation.clone(),
                                        previous_point_x,
                                        previous_point_y,
                                        automation_discrete,
                                        &selected_automation_b4_after_points
                                    );

                                    match type_to_show {
                                        crate::state::AutomationViewMode::Controllers => {
                                            if let TrackEvent::Controller(controller) = track_event {
                                                if let Some(automation_type_value) = automation_type {
                                                    if controller.controller() == automation_type_value {
                                                        let controller_value = controller.value();
                                                        let note_y_pos_inverted = controller_value as f64 * adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels;
                                                        let x = controller.position() * adjusted_beat_width_in_pixels;
                                                        let y = height - note_y_pos_inverted;

                                                        Self::draw_automation(context, height, automation_discrete, &mut previous_point_x, &mut previous_point_y, default_line_width, x, y, controller_value as f64);

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

                                                if automation_discrete {
                                                    context.move_to(x, height / 2.0);
                                                } else {
                                                    context.move_to(previous_point_x, previous_point_y);
                                                    context.set_line_width(0.75);
                                                }

                                                context.line_to(x, y);
                                                let _ = context.stroke();
                                                context.set_line_width(default_line_width);

                                                if !automation_discrete {
                                                    context.rectangle(x - 5.0, y - 5.0, 10.0, 10.0);
                                                    let _ = context.fill();
                                                    let _ = context.stroke();
                                                }

                                                if !automation_discrete {
                                                    previous_point_x = x;
                                                    previous_point_y = y;
                                                }

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

                                                        Self::draw_automation(context, height, automation_discrete, &mut previous_point_x, &mut previous_point_y, default_line_width, x, y, parameter_value as f64);
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

                                                            Self::draw_automation(context, height, automation_discrete, &mut previous_point_x, &mut previous_point_y, default_line_width, x, y, parameter_value as f64);

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
                                                    let note_y_pos_inverted = note_expression_value * 127.0 *  adjusted_entity_height_in_pixels + adjusted_entity_height_in_pixels;
                                                    let x = note_expression.position() * adjusted_beat_width_in_pixels;
                                                    let y = height - note_y_pos_inverted;

                                                    Self::draw_automation(context, height, automation_discrete, &mut previous_point_x, &mut previous_point_y, default_line_width, x, y, note_expression_value);

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
                                if !automation_discrete {
                                    context.set_line_width(0.75);
                                    context.move_to(previous_point_x, previous_point_y);
                                    context.line_to(width, previous_point_y);
                                    let _ = context.stroke();
                                    context.set_line_width(default_line_width);
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

    fn select_single(&self, tx_from_ui: Sender<DAWEvents>, x: f64, y: i32, add_to_select: bool) {
    }

    fn select_multiple(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, x: f64, y: i32, x2: f64, y2: i32, add_to_select: bool) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::AutomationSelectMultiple(x, y2, x2, y, add_to_select), None));
    }

    fn deselect_single(&self, tx_from_ui: Sender<DAWEvents>, x: f64, y: i32) {
    }

    fn deselect_multiple(&self, tx_from_ui: Sender<DAWEvents>, x: f64, y: i32, x2: f64, y2: i32) {
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::AutomationDeselectMultiple(x, y2, x2, y), None));
    }

    fn add_entity(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, y_index: i32, time: f64, _duration: f64, _entity_uuid: String) {
        let mut new_entities = vec![];
        new_entities.push((time, y_index));
        let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::AutomationAdd(new_entities), None));
    }

    fn add_entity_extra(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64, duration: f64, entity_uuid: String) {
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

    fn set_start_note(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64) {
    }

    fn set_riff_reference_play_mode(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64) {
    }

    fn handle_windowed_zoom(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, x1: f64, y1: f64, x2: f64, y2: f64) {
    }

    fn cycle_entity_selection(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64) {
    }

    fn select_underlying_entity(&self, tx_from_ui: Sender<DAWEvents>, y_index: i32, time: f64) {
    }
}
