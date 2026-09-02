use std::any::{type_name, Any};
use std::sync::{Arc, Mutex};
use masonry_core::core::Widget;
use mlua::AsChunk;
use strum_macros::Display;
use vello::wgpu::hal::DynCommandEncoder;
use vello_svg::usvg::strict_num::ApproxEq;
use xilem::style::Style;
use xilem::view::{
    CrossAxisAlignment, GridExt, MainAxisAlignment, flex_col, flex_row, grid, label, portal,
    sized_box, text_button,
};
use xilem::{EventLoop, Pod, ViewCtx, WidgetView, WindowOptions, Xilem};
use xilem_core::one_of::Either;
use xilem_core::{MessageContext, MessageResult, Mut, View, ViewId, ViewMarker};
use crate::constants::{MUSICAL_ITEM_LENGTH_OPTIONS, NOTE_SUBDIVISIONS, TRIPLETS};
use crate::domain::{Project};
use crate::event::{DAWEvents, OperationModeType, TrackChangeType};
use crate::state::{AutomationViewState, MidiPolyphonicExpressionNoteId, PianoRollState, RiffDAWState, TrackGridState};
use crate::utils::DAWUtils;
use crate::views::widgets;
use crate::views::widgets::{BeatGridMouseCoordHelper, BeatGridWidget, CustomPainter, DrawingAreaType, AutomationMouseCoordHelper, PianoRollMouseCoordHelper, PianoRollCustomPainter, TrackGridCustomPainter, TrackGridMouseCoordHelper, RiffSetTrackCustomPainter, RiffGridCustomPainter, DrawMode, AutomationCustomPainter};



pub fn piano_roll<State: 'static, Action: 'static>(
    project: Arc<Mutex<Project>>,
    piano_roll_mpe_note_id: MidiPolyphonicExpressionNoteId,
    selected_track_uuid: String,
    selected_riff_uuid: String,
    selected_riff_events: Vec<String>,
    operation_mode: OperationModeType,
    piano_roll_state: PianoRollState
) -> BeatGrid<State, Action> {
    BeatGrid::new_piano_roll(
        project,
        60000.0,
        60000.0,
        piano_roll_mpe_note_id,
        selected_track_uuid,
        selected_riff_uuid,
        selected_riff_events,
        operation_mode,
        piano_roll_state
    )
}

pub fn piano_roll_with_size<State: 'static, Action: 'static>(
    project: Arc<Mutex<Project>>,
    height:f64,
    width: f64,
    piano_roll_mpe_note_id: MidiPolyphonicExpressionNoteId,
    selected_track_uuid: String,
    selected_riff_uuid: String,
    selected_riff_events: Vec<String>,
    operation_mode: OperationModeType,
    piano_roll_state: PianoRollState,
) -> BeatGrid<State, Action> {
    BeatGrid::new_piano_roll(
        project,
        height,
        width,
        piano_roll_mpe_note_id,
        selected_track_uuid,
        selected_riff_uuid,
        selected_riff_events,
        operation_mode,
        piano_roll_state
    )
}

pub fn automation_grid_with_size<State: 'static, Action: 'static>(
    project: Arc<Mutex<Project>>,
    height:f64,
    width: f64,
    piano_roll_mpe_note_id: MidiPolyphonicExpressionNoteId,
    selected_track_uuid: String,
    selected_riff_uuid: String,
    selected_riff_events: Vec<String>,
    operation_mode: OperationModeType,
    automation_view_state: AutomationViewState,
) -> BeatGrid<State, Action> {
    BeatGrid::new_automation_grid(
        project,
        height,
        width,
        piano_roll_mpe_note_id,
        selected_track_uuid,
        selected_riff_uuid,
        selected_riff_events,
        operation_mode,
        automation_view_state
    )
}

pub fn track_grid_with_size<State: 'static, Action: 'static>(
    project: Arc<Mutex<Project>>,
    height:f64,
    width: f64,
    selected_track_grid_riff_references: Vec<String>,
    operation_mode: OperationModeType,
    track_grid_state: TrackGridState,
) -> BeatGrid<State, Action> {
    BeatGrid::new_track_grid(
        project,
        height,
        width,
        selected_track_grid_riff_references,
        operation_mode,
        track_grid_state,
    )
}

pub fn riff_track_grid_with_size<State: 'static, Action: 'static>(
    project: Arc<Mutex<Project>>,
    height:f64,
    width: f64,
    selected_track_grid_riff_references: Vec<String>,
    operation_mode: OperationModeType,
    track_uuid: String,
    riff_set_uuid: String,
) -> BeatGrid<State, Action> {
    BeatGrid::new_riff_track_grid(
        project,
        height,
        width,
        selected_track_grid_riff_references,
        operation_mode,
        track_uuid,
        riff_set_uuid,
    )
}

pub fn riff_grid_with_size<State: 'static, Action: 'static>(
    project: Arc<Mutex<Project>>,
    height:f64,
    width: f64,
    selected_track_grid_riff_references: Vec<String>,
    operation_mode: OperationModeType,
    riff_grid_uuid: Option<String>,
) -> BeatGrid<State, Action> {
    BeatGrid::new_riff_grid_grid(
        project,
        height,
        width,
        selected_track_grid_riff_references,
        operation_mode,
        riff_grid_uuid,
    )
}

const BEAT_GRID_CONTENT_VIEW_ID: ViewId = ViewId::new(0);

pub type CallbackSelectMultiple<State, Action> = Box<dyn Fn(&mut State, &f64, &i32, &f64, &i32, &bool) -> Action + Send + Sync + 'static>;

pub struct BeatGrid<State, Action> {
    height: f64,
    width: f64,
    project: Arc<Mutex<Project>>,
    piano_roll_mpe_note_id: MidiPolyphonicExpressionNoteId,
    selected_track_uuid: String,
    selected_riff_uuid: String,
    selected_riff_events: Vec<String>,
    grid_type: DrawingAreaType,
    on_select_multiple: Option<CallbackSelectMultiple<State, Action>>,
    on_add_riff_reference: Option<Box<dyn Fn(&mut State, &f64, &i32) -> Action + Send + Sync + 'static>>,
    on_delete_riff_reference: Option<Box<dyn Fn(&mut State, &f64, &i32) -> Action + Send + Sync + 'static>>,
    on_add_riff_note: Option<Box<dyn Fn(&mut State, Vec<(i32, f64, f64)>) -> Action + Send + Sync + 'static>>,
    on_delete_riff_note: Option<Box<dyn Fn(&mut State, i32, f64) -> Action + Send + Sync + 'static>>,
    on_cut: Option<Box<dyn Fn(&mut State) -> Action + Send + Sync + 'static>>,
    on_copy: Option<Box<dyn Fn(&mut State) -> Action + Send + Sync + 'static>>,
    on_paste: Option<Box<dyn Fn(&mut State) -> Action + Send + Sync + 'static>>,
    on_edit_cursor_position_change: Option<Box<dyn Fn(&mut State, f64) -> Action + Send + Sync + 'static>>,
    on_riff_set_track_increment_riff: Option<Box<dyn Fn(&mut State, String, String) -> Action + Send + Sync + 'static>>,
    on_riff_select: Option<Box<dyn Fn(&mut State, String, String) -> Action + Send + Sync + 'static>>,
    on_riff_add: Option<Box<dyn Fn(&mut State, String, String, String, f64) -> Action + Send + Sync + 'static>>, // new riff uuid, new riff name, track_uuid, new riff duration
    on_riff_set_track_set_riff: Option<Box<dyn Fn(&mut State, String, String, String) -> Action + Send + Sync + 'static>>,
    selected_track_grid_riff_references: Vec<String>,
    operation_mode: OperationModeType,
    track_uuid: String,
    riff_set_uuid: String,
    riff_grid_uuid: Option<String>,
    piano_roll_state: Option<PianoRollState>,
    track_grid_state: Option<TrackGridState>,
    automation_view_state: Option<AutomationViewState>,
}

// impl<State, Action> BeatGrid<State, Action> {
// }

impl<State: 'static, Action: 'static> BeatGrid<State, Action> {
    pub fn new_piano_roll(
        project: Arc<Mutex<Project>>,
        height:f64,
        width: f64,
        piano_roll_mpe_note_id: MidiPolyphonicExpressionNoteId,
        selected_track_uuid: String,
        selected_riff_uuid: String,
        selected_riff_events: Vec<String>,
        operation_mode: OperationModeType,
        piano_roll_state: PianoRollState,
    ) -> BeatGrid<State, Action> {
        Self {
            height,
            width,
            project,
            piano_roll_mpe_note_id,
            selected_track_uuid,
            selected_riff_uuid,
            selected_riff_events,
            grid_type: DrawingAreaType::PianoRoll,
            on_select_multiple: None,
            on_add_riff_reference: None,
            on_delete_riff_reference: None,
            on_add_riff_note: None,
            on_delete_riff_note: None,
            on_cut: None,
            on_copy: None,
            on_paste: None,
            on_edit_cursor_position_change: None,
            on_riff_set_track_increment_riff: None,
            on_riff_select: None,
            on_riff_add: None,
            on_riff_set_track_set_riff: None,
            selected_track_grid_riff_references: vec![],
            operation_mode,
            track_uuid: "".to_string(),
            riff_set_uuid: "".to_string(),
            riff_grid_uuid: None,
            piano_roll_state: Some(piano_roll_state),
            track_grid_state: None,
            automation_view_state: None,
        }
    }
    pub fn new_automation_grid(
        project: Arc<Mutex<Project>>,
        height:f64,
        width: f64,
        piano_roll_mpe_note_id: MidiPolyphonicExpressionNoteId,
        selected_track_uuid: String,
        selected_riff_uuid: String,
        selected_riff_events: Vec<String>,
        operation_mode: OperationModeType,
        automation_view_state: AutomationViewState,
    ) -> BeatGrid<State, Action> {
        Self {
            height,
            width,
            project,
            piano_roll_mpe_note_id,
            selected_track_uuid,
            selected_riff_uuid,
            selected_riff_events,
            grid_type: DrawingAreaType::PianoRoll,
            on_select_multiple: None,
            on_add_riff_reference: None,
            on_delete_riff_reference: None,
            on_add_riff_note: None,
            on_delete_riff_note: None,
            on_cut: None,
            on_copy: None,
            on_paste: None,
            on_edit_cursor_position_change: None,
            on_riff_set_track_increment_riff: None,
            on_riff_select: None,
            on_riff_add: None,
            on_riff_set_track_set_riff: None,
            selected_track_grid_riff_references: vec![],
            operation_mode,
            track_uuid: "".to_string(),
            riff_set_uuid: "".to_string(),
            riff_grid_uuid: None,
            piano_roll_state: None,
            track_grid_state: None,
            automation_view_state: Some(automation_view_state),
        }
    }

    pub fn new_track_grid(
        project: Arc<Mutex<Project>>,
        height:f64,
        width: f64,
        selected_track_grid_riff_references: Vec<String>,
        operation_mode: OperationModeType,
        track_grid_state: TrackGridState,
    ) -> BeatGrid<State, Action> {
        Self {
            height,
            width,
            project,
            piano_roll_mpe_note_id: crate::state::MidiPolyphonicExpressionNoteId::ALL,
            selected_track_uuid: "".to_string(),
            selected_riff_uuid: "".to_string(),
            selected_riff_events: vec![],
            grid_type: DrawingAreaType::TrackGrid,
            on_select_multiple: None,
            on_add_riff_reference: None,
            on_delete_riff_reference: None,
            on_add_riff_note: None,
            on_delete_riff_note: None,
            on_cut: None,
            on_copy: None,
            on_paste: None,
            on_edit_cursor_position_change: None,
            on_riff_set_track_increment_riff: None,
            on_riff_select: None,
            on_riff_add: None,
            on_riff_set_track_set_riff: None,
            selected_track_grid_riff_references,
            operation_mode,
            track_uuid: "".to_string(),
            riff_set_uuid: "".to_string(),
            riff_grid_uuid: None,
            piano_roll_state: None,
            track_grid_state: Some(track_grid_state),
            automation_view_state: None,
        }
    }

    pub fn new_riff_track_grid(
        project: Arc<Mutex<Project>>,
        height:f64,
        width: f64,
        selected_track_grid_riff_references: Vec<String>,
        operation_mode: OperationModeType,
        track_uuid: String,
        riff_set_uuid: String,
    ) -> BeatGrid<State, Action> {
        Self {
            height,
            width,
            project,
            piano_roll_mpe_note_id: crate::state::MidiPolyphonicExpressionNoteId::ALL,
            selected_track_uuid: "".to_string(),
            selected_riff_uuid: "".to_string(),
            selected_riff_events: vec![],
            grid_type: DrawingAreaType::Riff,
            on_select_multiple: None,
            on_add_riff_reference: None,
            on_delete_riff_reference: None,
            on_add_riff_note: None,
            on_delete_riff_note: None,
            on_cut: None,
            on_copy: None,
            on_paste: None,
            on_edit_cursor_position_change: None,
            on_riff_set_track_increment_riff: None,
            on_riff_select: None,
            on_riff_add: None,
            on_riff_set_track_set_riff: None,
            selected_track_grid_riff_references,
            operation_mode,
            track_uuid,
            riff_set_uuid,
            riff_grid_uuid: None,
            piano_roll_state: None,
            track_grid_state: None,
            automation_view_state: None,
        }
    }

    pub fn new_riff_grid_grid(
        project: Arc<Mutex<Project>>,
        height:f64,
        width: f64,
        selected_track_grid_riff_references: Vec<String>,
        operation_mode: OperationModeType,
        riff_grid_uuid: Option<String>,
    ) -> BeatGrid<State, Action> {
        Self {
            height,
            width,
            project,
            piano_roll_mpe_note_id: crate::state::MidiPolyphonicExpressionNoteId::ALL,
            selected_track_uuid: "".to_string(),
            selected_riff_uuid: "".to_string(),
            selected_riff_events: vec![],
            grid_type: DrawingAreaType::RiffGrid,
            on_select_multiple: None,
            on_add_riff_reference: None,
            on_delete_riff_reference: None,
            on_add_riff_note: None,
            on_delete_riff_note: None,
            on_cut: None,
            on_copy: None,
            on_paste: None,
            on_edit_cursor_position_change: None,
            on_riff_set_track_increment_riff: None,
            on_riff_select: None,
            on_riff_add: None,
            on_riff_set_track_set_riff: None,
            selected_track_grid_riff_references,
            operation_mode,
            track_uuid: "".to_string(),
            riff_set_uuid: "".to_string(),
            riff_grid_uuid,
            piano_roll_state: None,
            track_grid_state: None,
            automation_view_state: None,
        }
    }

    pub fn on_select_multiple(mut self, on_select_multiple: CallbackSelectMultiple<State, Action>) -> Self {
        self.on_select_multiple = Some(on_select_multiple);
        self
    }

    pub fn on_add_riff_reference(mut self, on_add: Box<dyn Fn(&mut State, &f64, &i32) -> Action + Send + Sync + 'static>) -> Self {
        self.on_add_riff_reference = Some(on_add);
        self
    }

    pub fn on_delete_riff_reference(mut self, on_delete: Box<dyn Fn(&mut State, &f64, &i32) -> Action + Send + Sync + 'static>) -> Self {
        self.on_delete_riff_reference = Some(on_delete);
        self
    }

    pub fn on_add_riff_note(mut self, on_add_riff_note: Box<dyn Fn(&mut State, Vec<(i32, f64, f64)>) -> Action + Send + Sync + 'static>) -> Self {
        self.on_add_riff_note = Some(on_add_riff_note);
        self
    }

    pub fn on_delete_riff_note(mut self, on_delete_riff_note: Box<dyn Fn(&mut State, i32, f64) -> Action + Send + Sync + 'static>) -> Self {
        self.on_delete_riff_note = Some(on_delete_riff_note);
        self
    }

    pub fn on_cut(mut self, on_cut: Box<dyn Fn(&mut State) -> Action + Send + Sync + 'static>) -> Self {
        self.on_cut = Some(on_cut);
        self
    }

    pub fn on_copy(mut self, on_copy: Box<dyn Fn(&mut State) -> Action + Send + Sync + 'static>) -> Self {
        self.on_copy = Some(on_copy);
        self
    }

    pub fn on_paste(mut self, on_paste: Box<dyn Fn(&mut State) -> Action + Send + Sync + 'static>) -> Self {
        self.on_paste = Some(on_paste);
        self
    }

    pub fn on_edit_cursor_position_change(mut self, on_edit_cursor_position_change: Box<dyn Fn(&mut State, f64) -> Action + Send + Sync + 'static>) -> Self {
        self.on_edit_cursor_position_change = Some(on_edit_cursor_position_change);
        self
    }

    pub fn on_riff_set_track_increment_riff(mut self, on_riff_set_track_increment_riff: Box<dyn Fn(&mut State, String, String) -> Action + Send + Sync + 'static>) -> Self {
        self.on_riff_set_track_increment_riff = Some(on_riff_set_track_increment_riff);
        self
    }

    pub fn on_riff_select(mut self, on_riff_select: Box<dyn Fn(&mut State, String, String) -> Action + Send + Sync + 'static>) -> Self {
        self.on_riff_select = Some(on_riff_select);
        self
    }

    pub fn on_riff_add(mut self, on_riff_add: Box<dyn Fn(&mut State, String, String, String, f64) -> Action + Send + Sync + 'static>) -> Self {
        self.on_riff_add = Some(on_riff_add);
        self
    }

    pub fn on_riff_set_track_set_riff(mut self, on_riff_set_track_set_riff: Box<dyn Fn(&mut State, String, String, String) -> Action + Send + Sync + 'static>) -> Self {
        self.on_riff_set_track_set_riff = Some(on_riff_set_track_set_riff);
        self
    }
}

impl<State, Action> ViewMarker for BeatGrid<State, Action> {}


impl<State: 'static, Action: 'static> View<State, Action, ViewCtx> for BeatGrid<State, Action> {
    type Element = Pod<BeatGridWidget>;
    type ViewState = ();

    fn build(&self, ctx: &mut ViewCtx, app_state: &mut State) -> (Self::Element, Self::ViewState) {
        let custom_painter: Box<dyn CustomPainter> = match self.grid_type {
            DrawingAreaType::PianoRoll => Box::new(PianoRollCustomPainter {
                track_cursor_time_in_beats: 0.0,
                project: self.project.clone(),
                piano_roll_mpe_note_id: self.piano_roll_mpe_note_id.clone(),
                selected_track_uuid: self.selected_track_uuid.clone(),
                selected_riff_uuid: self.selected_riff_uuid.clone(),
                selected_riff_events: self.selected_riff_events.clone(),
            }),
            DrawingAreaType::TrackGrid => {
                let (show_automation, show_note, show_note_velocity, show_pan) = if let Some(track_grid_state) = self.track_grid_state.as_ref() {
                    (track_grid_state.show_automation, track_grid_state.show_notes, track_grid_state.show_note_velocities, track_grid_state.show_pan)
                }

                else {
                    (false, false, false, false)
                };
                Box::new(TrackGridCustomPainter {
                    project: self.project.clone(),
                    show_automation,
                    show_note,
                    show_note_velocity,
                    show_pan,
                    looping: false,
                    active_loop: None,
                    selected_track_grid_riff_references: vec![],
                })
            },
            DrawingAreaType::Riff => Box::new(RiffSetTrackCustomPainter {
                project: self.project.clone(),
                track_cursor_time_in_beats: 0.0,
                track_uuid: self.track_uuid.clone(),
                riff_set_uuid: self.riff_set_uuid.clone(),
            }),
            DrawingAreaType::RiffGrid => Box::new(RiffGridCustomPainter::new_with_edit_item_handler (
                self.project.clone(),
                false,
                self.riff_grid_uuid.clone(),
            )),
            DrawingAreaType::Automation => Box::new(AutomationCustomPainter::new_with_edit_item_handler(self.project.clone())  ),
            _ => unreachable!()
        };

        let beat_grid_mouse_coord_helper: Option<Box<dyn BeatGridMouseCoordHelper<Action=DAWEvents>>> = match self.grid_type {
            DrawingAreaType::TrackGrid => Some(Box::new(TrackGridMouseCoordHelper {})),
            DrawingAreaType::PianoRoll => Some(Box::new(PianoRollMouseCoordHelper {})),
            DrawingAreaType::Automation => Some(Box::new(AutomationMouseCoordHelper {})),
            _ => None,
        };

        let entity_height_in_pixels = match self.grid_type {
            DrawingAreaType::TrackGrid => 41.0,
            DrawingAreaType::PianoRoll => self.height / 127.0,
            DrawingAreaType::Riff => self.height / 127.0,
            DrawingAreaType::RiffGrid => 41.0,
            DrawingAreaType::Automation => self.height / 127.0,
            _ => unreachable!()
        };

        let beat_width_in_pixels = match self.grid_type {
            DrawingAreaType::TrackGrid => 50.0,
            DrawingAreaType::PianoRoll => 50.0,
            DrawingAreaType::Riff => 1.0,
            DrawingAreaType::RiffGrid => 50.0,
            DrawingAreaType::Automation => 50.0,
            _ => unreachable!()
        };

        let zoom_horizontal = match self.grid_type {
            DrawingAreaType::TrackGrid => 1.0,
            DrawingAreaType::PianoRoll => 1.0,
            DrawingAreaType::Riff => 10.0,
            DrawingAreaType::RiffGrid => 1.0,
            DrawingAreaType::Automation => 1.0,
            _ => unreachable!()
        };

        let zoom_vertical = match self.grid_type {
            DrawingAreaType::TrackGrid => 1.0,
            DrawingAreaType::PianoRoll => 1.0,
            DrawingAreaType::Riff => 1.0,
            DrawingAreaType::RiffGrid => 1.0,
            DrawingAreaType::Automation => 1.0,
            _ => unreachable!()
        };

        let pod = ctx.with_action_widget(|ctx| ctx.create_pod(
            BeatGridWidget::new_with_custom(
                self.height,
                self.width,
                zoom_horizontal,
                zoom_vertical,
                entity_height_in_pixels,
                beat_width_in_pixels,
                4,
                Some(custom_painter),
                beat_grid_mouse_coord_helper,
                false,
                self.grid_type.clone(),
                self.operation_mode.clone(),
            )
        )
        );
        (pod, ())
    }

    fn rebuild(&self, prev: &Self, view_state: &mut Self::ViewState, ctx: &mut ViewCtx, element: Mut<'_, Self::Element>, app_state: &mut State) {
        // println!("Beat grid widget rebuild requested.");

        if prev.selected_riff_events != self.selected_riff_events {
            if let Some(custom_painter) = element.widget.custom_painter.as_mut() {
                if let Some(piano_roll_custom_painter) = custom_painter.as_any().downcast_mut::<PianoRollCustomPainter>() {
                    piano_roll_custom_painter.selected_riff_events = self.selected_riff_events.clone();
                }
            }
        }

        if let Some(current_piano_roll_state) = self.piano_roll_state.as_ref() {
            if let Some(previous_piano_roll_state) = prev.piano_roll_state.as_ref() {
                if previous_piano_roll_state.piano_roll_selected_snap != current_piano_roll_state.piano_roll_selected_snap {
                    element.widget.set_snap_in_beats(
                        DAWUtils::get_snap_quantise_value_in_beats_from_choice_text(
                            MUSICAL_ITEM_LENGTH_OPTIONS.get(current_piano_roll_state.piano_roll_selected_snap).unwrap(), 4.0)
                    );
                }
                if previous_piano_roll_state.selected_piano_roll_note_length_option != current_piano_roll_state.selected_piano_roll_note_length_option {
                    element.widget.set_new_entity_length_in_beats(
                        DAWUtils::get_snap_quantise_value_in_beats_from_choice_text(
                            MUSICAL_ITEM_LENGTH_OPTIONS.get(current_piano_roll_state.selected_piano_roll_note_length_option).unwrap(), 4.0)
                    );
                }
                if previous_piano_roll_state.selected_piano_roll_triplet != current_piano_roll_state.selected_piano_roll_triplet {
                    let triplet_name = TRIPLETS.get(current_piano_roll_state.selected_piano_roll_triplet).unwrap();
                    element.widget.set_triplet_spacing_in_beats(
                        DAWUtils::get_snap_quantise_value_in_beats_from_choice_text(
                            *triplet_name, 4.0)
                    );
                }
                if previous_piano_roll_state.selected_piano_roll_subdivision != current_piano_roll_state.selected_piano_roll_subdivision {
                    let note_subdivision_name = NOTE_SUBDIVISIONS.get(current_piano_roll_state.selected_piano_roll_subdivision).unwrap();
                    let note_subdivision = if *note_subdivision_name == "Normal" {
                        DrawMode::Point
                    }
                    else {
                        DrawMode::Triplet
                    };
                    element.widget.set_draw_mode(note_subdivision);
                }
                if previous_piano_roll_state.piano_roll_edit_cursor_time_in_beats != current_piano_roll_state.piano_roll_edit_cursor_time_in_beats {
                    element.widget.set_edit_cursor_time_in_beats(current_piano_roll_state.piano_roll_edit_cursor_time_in_beats);
                }
                if previous_piano_roll_state.piano_roll_quantise_end != current_piano_roll_state.piano_roll_quantise_end {
                    element.widget.set_snap_end(current_piano_roll_state.piano_roll_quantise_end);
                }
                if previous_piano_roll_state.piano_roll_quantise_quantise_strength != current_piano_roll_state.piano_roll_quantise_quantise_strength {
                    element.widget.set_snap_strength(current_piano_roll_state.piano_roll_quantise_quantise_strength as f64 / 100.0);
                }
                if previous_piano_roll_state.piano_roll_quantise_start != current_piano_roll_state.piano_roll_quantise_start {
                    element.widget.set_snap_start(current_piano_roll_state.piano_roll_quantise_start);
                }
                // pub piano_roll_scroll_y: f32,
            }
        }

        if let Some(track_grid_state) = self.track_grid_state.as_ref() {
            if let Some(previous_track_grid_state) = prev.track_grid_state.as_ref() {
                if previous_track_grid_state.show_automation != track_grid_state.show_automation {
                    if let Some(custom_painter) = element.widget.custom_painter.as_mut() {
                        if let Some(track_grid_custom_painter) = custom_painter.as_any().downcast_mut::<TrackGridCustomPainter>() {
                            track_grid_custom_painter.show_automation = track_grid_state.show_automation;
                        }
                    }
                }
                if previous_track_grid_state.show_note_velocities != track_grid_state.show_note_velocities {
                    if let Some(custom_painter) = element.widget.custom_painter.as_mut() {
                        if let Some(track_grid_custom_painter) = custom_painter.as_any().downcast_mut::<TrackGridCustomPainter>() {
                            track_grid_custom_painter.show_note_velocity = track_grid_state.show_note_velocities;
                        }
                    }
                }
                if previous_track_grid_state.show_notes != track_grid_state.show_notes {
                    if let Some(custom_painter) = element.widget.custom_painter.as_mut() {
                        if let Some(track_grid_custom_painter) = custom_painter.as_any().downcast_mut::<TrackGridCustomPainter>() {
                            track_grid_custom_painter.show_note = track_grid_state.show_notes;
                        }
                    }
                }
                if previous_track_grid_state.show_pan != track_grid_state.show_pan {
                    if let Some(custom_painter) = element.widget.custom_painter.as_mut() {
                        if let Some(track_grid_custom_painter) = custom_painter.as_any().downcast_mut::<TrackGridCustomPainter>() {
                            track_grid_custom_painter.show_pan = track_grid_state.show_pan;
                        }
                    }
                }
            }
        }

        if prev.height != self.height {

        }
        if prev.width != self.width {

        }
        if prev.selected_track_uuid != self.selected_track_uuid {
            if let Some(custom_painter) = element.widget.custom_painter.as_mut() {
                if let Some(piano_roll_custom_painter) = custom_painter.as_any().downcast_mut::<PianoRollCustomPainter>() {
                    piano_roll_custom_painter.selected_track_uuid = self.selected_track_uuid.clone();
                }
            }
        }
        if prev.selected_riff_uuid != self.selected_riff_uuid {
            if let Some(custom_painter) = element.widget.custom_painter.as_mut() {
                if let Some(piano_roll_custom_painter) = custom_painter.as_any().downcast_mut::<PianoRollCustomPainter>() {
                    piano_roll_custom_painter.selected_riff_uuid = self.selected_riff_uuid.clone();
                }
            }
        }
        if prev.selected_riff_events != self.selected_riff_events {
            if let Some(custom_painter) = element.widget.custom_painter.as_mut() {
                if let Some(piano_roll_custom_painter) = custom_painter.as_any().downcast_mut::<PianoRollCustomPainter>() {
                    piano_roll_custom_painter.selected_riff_uuid = self.selected_riff_uuid.clone();
                }
            }
        }
        if prev.piano_roll_mpe_note_id != self.piano_roll_mpe_note_id {
            if let Some(custom_painter) = element.widget.custom_painter.as_mut() {
                if let Some(piano_roll_custom_painter) = custom_painter.as_any().downcast_mut::<PianoRollCustomPainter>() {
                    piano_roll_custom_painter.piano_roll_mpe_note_id = self.piano_roll_mpe_note_id.clone();
                }
            }
        }

        if prev.selected_track_grid_riff_references != self.selected_track_grid_riff_references {
            if let Some(custom_painter) = element.widget.custom_painter.as_mut() {
                if let Some(track_grid_custom_painter) = custom_painter.as_any().downcast_mut::<TrackGridCustomPainter>() {
                    track_grid_custom_painter.selected_track_grid_riff_references = self.selected_track_grid_riff_references.clone();
                }
            }
        }

        if prev.operation_mode != self.operation_mode {
            element.widget.operation_mode = self.operation_mode.clone();
        }
    }

    fn teardown(&self, view_state: &mut Self::ViewState, ctx: &mut ViewCtx, element: Mut<'_, Self::Element>) {
        // println!("Beat grid widget tear down requested.")
    }

    fn message(&self, view_state: &mut Self::ViewState, message: &mut MessageContext, element: Mut<'_, Self::Element>, app_state: &mut State) -> MessageResult<Action> {
        let mut message_result = MessageResult::Stale;
        match message.take_message::<DAWEvents>() {
            Some(event) => {
                match event.as_ref() {
                    DAWEvents::TrackChange(change,track_uuid) => {
                        match change {
                            TrackChangeType::Added(_) => {}
                            TrackChangeType::Deleted => {}
                            TrackChangeType::Modified => {}
                            TrackChangeType::Selected => {}
                            TrackChangeType::Volume(_, _) => {}
                            TrackChangeType::Pan(_, _) => {}
                            TrackChangeType::RiffAdd(uuid, name, duration) => {
                                println!("^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ BeatGrid - TrackChangeType::RiffAdd");
                                if let Some(on_riff_add) = self.on_riff_add.as_ref() {
                                    if let Some(track_uuid) = track_uuid {
                                        message_result = MessageResult::Action(on_riff_add(app_state, uuid.to_string(), name.clone(), track_uuid.clone(), duration.clone()));
                                    }
                                }
                            }
                            TrackChangeType::RiffAddWithTrackIndex(_, _, _) => {}
                            TrackChangeType::RiffCopy(_, _, _) => {}
                            TrackChangeType::RiffDelete(_) => {}
                            TrackChangeType::RiffNameChange(_, _) => {}
                            TrackChangeType::RiffLengthChange(_, _) => {}
                            TrackChangeType::RiffSelect(riff_set_uuid) => {
                                println!("^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ BeatGrid - TrackChangeType::RiffSelect");
                                if let Some(on_riff_select) = self.on_riff_select.as_ref() {
                                    if let Some(track_uuid) = track_uuid {
                                        message_result = MessageResult::Action(on_riff_select(app_state, riff_set_uuid.to_string(), track_uuid.clone()));
                                    }
                                }
                            }
                            TrackChangeType::RiffSelectWithTrackIndex { .. } => {}
                            TrackChangeType::RiffReferenceAdd(track_index, position) => {
                                println!("^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ BeatGrid - TrackChangeType::RiffReferenceAdd");
                                if let Some(on_add_riff_reference) = self.on_add_riff_reference.as_ref() {
                                    message_result = MessageResult::Action(on_add_riff_reference(app_state, position, track_index));
                                }
                            }
                            TrackChangeType::RiffReferenceDragCopy(_) => {}
                            TrackChangeType::RiffReferenceDelete(track_index, position) => {
                                println!("^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ BeatGrid - TrackChangeType::RiffReferenceDelete");
                                if let Some(on_delete_riff_reference) = self.on_delete_riff_reference.as_ref() {
                                    message_result = MessageResult::Action(on_delete_riff_reference(app_state, position, track_index));
                                }
                            }
                            TrackChangeType::RiffReferencesSelectMultiple(x, y, x2, y2, add_to_select) => {
                                if let Some(on_select_multiple) = self.on_select_multiple.as_ref() {
                                    message_result = MessageResult::Action(on_select_multiple(app_state, x, y, x2, y2, add_to_select));
                                }
                            }
                            TrackChangeType::RiffReferencesSelectSingle(_, _, _) => {}
                            TrackChangeType::RiffReferencesDeselectMultiple(_, _, _, _) => {}
                            TrackChangeType::RiffReferencesDeselectSingle(_, _) => {}
                            TrackChangeType::RiffReferenceCutSelected => {
                                if let Some(on_cut) = self.on_cut.as_ref() {
                                    message_result = MessageResult::Action(on_cut(app_state));
                                }
                            }
                            TrackChangeType::RiffReferenceCopySelected => {
                                if let Some(on_copy) = self.on_copy.as_ref() {
                                    message_result = MessageResult::Action(on_copy(app_state));
                                }
                            }
                            TrackChangeType::RiffReferencePaste => {
                                if let Some(on_paste) = self.on_paste.as_ref() {
                                    message_result = MessageResult::Action(on_paste(app_state));
                                }
                            }
                            TrackChangeType::RiffReferenceChange(_) => {}
                            TrackChangeType::RiffReferencesSelectAll => {}
                            TrackChangeType::RiffReferencesDeselectAll => {}
                            TrackChangeType::RiffReferenceIncrementRiff { .. } => {}
                            TrackChangeType::RiffAddNote(data) => {
                                if let Some(on_add_riff_note) = self.on_add_riff_note.as_ref() {
                                    message_result = MessageResult::Action(on_add_riff_note(app_state, data.clone()));
                                }
                            }
                            TrackChangeType::RiffDeleteNote(note, position) => {
                                if let Some(on_delete_riff_note) = self.on_delete_riff_note.as_ref() {
                                    message_result = MessageResult::Action(on_delete_riff_note(app_state, *note, *position));
                                }
                            }
                            TrackChangeType::RiffAddSample(_, _) => {}
                            TrackChangeType::RiffDeleteSample(_, _) => {}
                            TrackChangeType::RiffTranslateSelected(_, _) => {}
                            TrackChangeType::RiffEventChange(_) => {}
                            TrackChangeType::RiffQuantiseSelected => {}
                            TrackChangeType::RiffCutSelected => {
                                if let Some(on_cut) = self.on_cut.as_ref() {
                                    message_result = MessageResult::Action(on_cut(app_state));
                                }
                            }
                            TrackChangeType::RiffCopySelected => {
                                if let Some(on_copy) = self.on_copy.as_ref() {
                                    message_result = MessageResult::Action(on_copy(app_state));
                                }
                            }
                            TrackChangeType::RiffPasteSelected => {
                                if let Some(on_paste) = self.on_paste.as_ref() {
                                    message_result = MessageResult::Action(on_paste(app_state));
                                }
                            }
                            TrackChangeType::RiffChangeLengthOfSelected(_) => {}
                            TrackChangeType::RiffEventsSelectMultiple(x, y, x2, y2, add_to_select) => {
                                if let Some(on_select_multiple) = self.on_select_multiple.as_ref() {
                                    message_result = MessageResult::Action(on_select_multiple(app_state, x, y, x2, y2, add_to_select));
                                }
                            }
                            TrackChangeType::RiffEventsSelectSingle(_, _, _) => {}
                            TrackChangeType::RiffEventsDeselectMultiple(_, _, _, _) => {}
                            TrackChangeType::RiffEventsDeselectSingle(_, _) => {}
                            TrackChangeType::RiffEventsSelectAll => {}
                            TrackChangeType::RiffEventsDeselectAll => {}
                            TrackChangeType::RiffSetStartNote(_, _) => {}
                            TrackChangeType::RiffReferencePlayMode(_, _) => {}
                            TrackChangeType::AutomationAdd(_) => {}
                            TrackChangeType::AutomationDelete(_) => {}
                            TrackChangeType::AutomationTranslateSelected(_, _) => {}
                            TrackChangeType::AutomationChange(_) => {}
                            TrackChangeType::AutomationQuantiseSelected => {}
                            TrackChangeType::AutomationCut => {}
                            TrackChangeType::AutomationCopy => {}
                            TrackChangeType::AutomationPaste => {}
                            TrackChangeType::AutomationTypeChange(_) => {}
                            TrackChangeType::AutomationSelectMultiple(_, _, _, _, _) => {}
                            TrackChangeType::AutomationDeselectMultiple(_, _, _, _) => {}
                            TrackChangeType::AutomationSelectAll => {}
                            TrackChangeType::AutomationDeselectAll => {}
                            _ => println!("Unknown beat grid event received.")
                        }
                    }
                    DAWEvents::RepaintPianoRollView => {}
                    DAWEvents::PianoRollMPENoteIdChange(midiPolyphonicExpressionNodeId) => {}
                    DAWEvents::TrackGridEditCursorPositionChanged(position) => {
                        if let Some(on_edit_cursor_position_change) = self.on_edit_cursor_position_change.as_ref() {
                            message_result = MessageResult::Action(on_edit_cursor_position_change(app_state, *position));
                        }
                    }
                    DAWEvents::RiffSetTrackIncrementRiff(riff_set_uuid, track_uuid) => {
                        if let Some(on_riff_set_track_increment_riff) = self.on_riff_set_track_increment_riff.as_ref() {
                            message_result = MessageResult::Action(on_riff_set_track_increment_riff(app_state, riff_set_uuid.clone(), track_uuid.clone()));
                        }
                    }
                    DAWEvents::RiffSetTrackSetRiff(riff_set_uuid, track_uuid, new_riff_uuid) => {
                        if let Some(on_riff_set_track_set_riff) = self.on_riff_set_track_set_riff.as_ref() {
                            message_result = MessageResult::Action(on_riff_set_track_set_riff(app_state, riff_set_uuid.clone(), track_uuid.clone(), new_riff_uuid.clone()));
                        }
                    }
                    _ => ()
                }

                message_result
            }
            None => {
                tracing::error!(
                    "Wrong message type in Button::message: {message:?} expected {}",
                    type_name::<DAWEvents>()
                );
                MessageResult::Stale
            }
        }
    }
}
