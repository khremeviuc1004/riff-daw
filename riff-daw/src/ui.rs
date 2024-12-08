
use std::collections::HashMap;
use std::io::Write;
use std::ptr::NonNull;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use cairo::glib::once_cell::unsync::Lazy;
use cairo::glib::{BindingFlags, BoolError, SignalHandlerId};
use crossbeam_channel::Sender;
use gdk::{EventType, RGBA, ScrollDirection};
use gladis::Gladis;
use gtk::{MessageDialogBuilder, ResponseType, TargetEntry, TargetFlags, DestDefaults, PolicyType};
use gtk::{AboutDialog, Adjustment, ApplicationWindow, Box, Button, ColorButton, ComboBoxText, CssProvider, Dialog, DrawingArea, Entry, EntryBuffer, FileChooserAction, FileChooserDialog, FileChooserWidget, FileFilter, Frame, gdk, glib, Grid, Label, ListStore, MenuItem, Orientation, Paned, prelude::*, prelude::Cast, ProgressBar, RadioToolButton, RecentChooserMenu, Scale, ScrolledWindow, SpinButton, Stack, TextView, ToggleButton, ToggleToolButton, ToolButton, TreeView, Viewport, Widget};
use indexmap::IndexMap;
use log::*;
use uuid::Uuid;

use crate::constants::{RIFF_ARRANGEMENT_VIEW_TRACK_PANEL_HEIGHT, RIFF_SEQUENCE_VIEW_TRACK_PANEL_HEIGHT, RIFF_SET_VIEW_TRACK_PANEL_HEIGHT, GTK_APPLICATION_ID};
use crate::{AudioEffectTrack, GeneralTrackType, RiffArrangement, RiffItemType};
use crate::domain::{NoteExpressionType, Track, TrackType, Note, TrackEvent, Riff, RiffItem};
use crate::event::{AutomationChangeData, CurrentView, DAWEvents, LoopChangeType, MasterChannelChangeType, NoteExpressionData, OperationModeType, ShowType, TrackChangeType, AutomationEditType};
use crate::grid::{AutomationCustomPainter, AutomationMouseCoordHelper, BeatGrid, BeatGridRuler, Grid as FreedomGrid, MouseButton, MouseHandler, Piano, PianoRollCustomPainter, PianoRollMouseCoordHelper, PianoRollVerticalScaleCustomPainter, RiffSetTrackCustomPainter, SampleRollCustomPainter, SampleRollMouseCoordHelper, TrackGridCustomPainter, TrackGridMouseCoordHelper, EditItemHandler, DrawingAreaType};
use crate::state::DAWState;
use crate::utils::DAWUtils;


const DRAG_N_DROP_TARGETS: Lazy<Vec<TargetEntry>> = Lazy::new(|| vec![
    TargetEntry::new("STRING", TargetFlags::SAME_APP, 0),
    TargetEntry::new("text/plain", TargetFlags::SAME_APP, 0)]
);

#[derive(Clone)]
pub enum RiffSetType {
    RiffSet,
    RiffSequence(String), // riff sequence uuid
    RiffArrangement(String), // riff arrangement uuid
}

#[derive(Clone)]
pub enum RiffSequenceType {
    RiffSequence,
    RiffArrangement(String), // riff arrangement uuid
}

#[derive(Gladis, Clone)]
pub struct Ui {
    pub wnd_main: ApplicationWindow,
    pub top_level_vbox: Box,
    pub sub_panel_stack: Stack,
    pub show_sub_panel_toggle_btn: ToggleButton,
    pub centre_split_pane: Paned,
    pub centre_panel_stack: Stack,

    pub progress_dialogue: Dialog,
    pub dialogue_progress_bar: ProgressBar,
    pub riff_name_dialogue: Dialog,
    pub riff_name_entry: Entry,

    pub configuration_dialogue: Dialog,

    pub about_dialogue: AboutDialog,

    pub recent_chooser_menu: RecentChooserMenu,

    // file menu
    pub menu_item_new: MenuItem,
    pub menu_item_open: MenuItem,
    pub menu_item_save: MenuItem,
    pub menu_item_save_as: MenuItem,
    pub menu_item_import_midi: MenuItem,
    pub menu_item_export_midi: MenuItem,
    pub menu_item_export_midi_riffs: MenuItem,
    pub menu_item_export_midi_riffs_separate: MenuItem,
    pub menu_item_export_wave: MenuItem,
    pub menu_item_quit: MenuItem,

    // edit menu
    pub menu_item_cut: MenuItem,
    pub menu_item_preferences: MenuItem,

    // help menu
    pub menu_item_about: MenuItem,

    pub toolbar_add_track_combobox: ComboBoxText,
    pub toolbar_add_track: ToolButton,
    pub toolbar_undo: ToolButton,
    pub toolbar_redo: ToolButton,

    pub track_split_pane: Paned,
    pub track_grid_vertical_adjustment: Adjustment,
    pub track_grid_horizontal_adjustment: Adjustment,
    pub track_grid_vertical_view_port: Viewport,

    pub track_grid_add_mode_btn: RadioToolButton,
    pub track_grid_delete_mode_btn: RadioToolButton,
    pub track_grid_edit_mode_btn: RadioToolButton,
    pub track_grid_select_mode_btn: RadioToolButton,
    pub track_grid_add_loop_mode_btn: RadioToolButton,

    pub track_grid_cut_btn: ToolButton,
    pub track_grid_copy_btn: ToolButton,
    pub track_grid_paste_btn: ToolButton,

    pub track_grid_horizontal_zoom_out: Button,
    pub track_grid_horizontal_zoom_scale: Scale,
    pub track_grid_horizontal_zoom_in: Button,
    pub track_grid_zoom_adjustment: Adjustment,
    
    pub track_grid_vertical_zoom_out: Button,
    pub track_grid_vertical_zoom_scale: Scale,
    pub track_grid_vertical_zoom_in: Button,
    pub track_grid_vertical_zoom_adjustment: Adjustment,

    pub track_grid_translate_left_btn: ToolButton,
    pub track_grid_translate_right_btn: ToolButton,
    pub track_grid_translate_up_btn: ToolButton,
    pub track_grid_translate_down_btn: ToolButton,

    pub track_grid_quantise_start_choice: ComboBoxText,
    pub track_grid_quantise_length_choice: ComboBoxText,
    pub track_grid_quantise_start_btn: ToggleToolButton,
    pub track_grid_quantise_end_btn: ToggleToolButton,

    pub track_grid_show_automation_btn: ToggleToolButton,
    pub track_grid_show_note_velocities_btn: ToggleToolButton,
    pub track_grid_show_notes_btn: ToggleToolButton,
    pub track_grid_show_pan_events_btn: ToggleToolButton,
    pub track_grid_cursor_follow: ToggleToolButton,

    pub track_panel_scrolled_window: ScrolledWindow,

    pub track_drawing_area: DrawingArea,
    pub track_ruler_drawing_area: DrawingArea,
    pub track_grid_scrolled_window: ScrolledWindow,

    pub piano_roll_component: Box,
    pub automation_component: Box,
    pub sample_library_component: Box,
    pub sample_roll_component: Box,
    pub mixer_component: Box,
    pub scripting_component: Box,

    pub piano_roll_scrolled_window: ScrolledWindow,

    pub piano_roll_piano_keyboard_drawing_area: DrawingArea,
    pub piano_roll_drawing_area: DrawingArea,
    pub piano_roll_ruler_drawing_area: DrawingArea,

    pub piano_roll_add_mode_btn: RadioToolButton,
    pub piano_roll_delete_mode_btn: RadioToolButton,
    pub piano_roll_edit_mode_btn: RadioToolButton,
    pub piano_roll_select_mode_btn: RadioToolButton,

    pub piano_roll_cut_btn: ToolButton,
    pub piano_roll_copy_btn: ToolButton,
    pub piano_roll_paste_btn: ToolButton,

    pub piano_roll_horizontal_zoom_out: Button,
    pub piano_roll_horizontal_zoom_scale: Scale,
    pub piano_roll_horizontal_zoom_in: Button,
    pub piano_roll_zoom_adjustment: Adjustment,

    pub piano_roll_vertical_zoom_out: Button,
    pub piano_roll_vertical_zoom_scale: Scale,
    pub piano_roll_vertical_zoom_in: Button,
    pub piano_roll_vertical_zoom_adjustment: Adjustment,

    pub piano_roll_translate_left_btn: ToolButton,
    pub piano_roll_translate_right_btn: ToolButton,
    pub piano_roll_translate_up_btn: ToolButton,
    pub piano_roll_translate_down_btn: ToolButton,

    pub piano_roll_quantise_start_choice: ComboBoxText,
    pub piano_roll_quantise_length_choice: ComboBoxText,
    pub piano_roll_quantise_start_checkbox: RadioToolButton,
    pub piano_roll_quantise_end_checkbox: RadioToolButton,
    pub piano_roll_quantise_btn: ToolButton,

    pub piano_roll_note_length_increment_choice: ComboBoxText,
    pub piano_roll_increase_note_length_btn: ToolButton,
    pub piano_roll_decrease_note_length_btn: ToolButton,

    pub piano_roll_dock_toggle_btn: ToggleToolButton,
    pub sample_roll_dock_toggle_btn: ToggleToolButton,
    pub sample_library_dock_toggle_btn: ToggleToolButton,
    pub automation_dock_toggle_btn: ToggleToolButton,
    pub scripting_dock_toggle_btn: ToggleToolButton,
    pub mixer_dock_toggle_btn: ToggleToolButton,

    pub piano_roll_track_name: Label,
    pub piano_roll_riff_name: Label,

    pub sample_library_file_chooser_widget: FileChooserWidget,
    pub sample_library_add_sample_to_song_btn: Button,

    pub sample_roll_drawing_area: DrawingArea,
    pub sample_roll_ruler_drawing_area: DrawingArea,

    pub sample_roll_add_mode_btn: ToolButton,
    pub sample_roll_delete_mode_btn: ToolButton,
    pub sample_roll_edit_mode_btn: ToolButton,
    pub sample_roll_select_mode_btn: ToolButton,

    pub sample_roll_cut_btn: ToolButton,
    pub sample_roll_copy_btn: ToolButton,
    pub sample_roll_paste_btn: ToolButton,

    pub sample_roll_zoom_out: ToolButton,
    pub sample_roll_zoom_scale: Scale,
    pub sample_roll_zoom_in: ToolButton,

    pub sample_roll_translate_left_btn: ToolButton,
    pub sample_roll_translate_right_btn: ToolButton,
    pub sample_roll_translate_up_btn: ToolButton,
    pub sample_roll_translate_down_btn: ToolButton,

    pub sample_roll_quantise_start_choice: ComboBoxText,
    pub sample_roll_quantise_length_choice: ComboBoxText,
    pub sample_roll_quantise_start_checkbox: RadioToolButton,
    pub sample_roll_quantise_end_checkbox: RadioToolButton,
    pub sample_roll_quantise_btn: ToolButton,

    pub sample_roll_increase_sample_length_btn: ToolButton,
    pub sample_roll_decrease_sample_length_btn: ToolButton,

    pub sample_roll_available_samples: TreeView,
    pub sample_roll_sample_browser_delete_btn: Button,

    pub sample_roll_track_name: Label,
    pub sample_roll_riff_name: Label,

    pub scripting_file_chooser_widget: FileChooserWidget,
    pub scripting_script_text_view: TextView,
    pub scripting_console_output_text_view: TextView,
    pub scripting_console_input_text_view: TextView,
    pub scripting_script_name_label: Label,
    pub scripting_new_script_btn: Button,
    pub scripting_save_script_btn: Button,
    pub scripting_save_script_as_btn: Button,
    pub scripting_run_script_btn: Button,
    pub scripting_console_run_btn: Button,

    pub mixer_box: Box,

    pub automation_ruler_drawing_area: DrawingArea,
    pub automation_drawing_area: DrawingArea,

    pub automation_add_mode_btn: ToolButton,
    pub automation_delete_mode_btn: ToolButton,
    pub automation_edit_mode_btn: ToolButton,
    pub automation_select_mode_btn: ToolButton,

    pub automation_cut_btn: ToolButton,
    pub automation_copy_btn: ToolButton,
    pub automation_paste_btn: ToolButton,

    pub automation_zoom_out: ToolButton,
    pub automation_zoom_scale: Scale,
    pub automation_zoom_in: ToolButton,

    pub automation_translate_left_btn: ToolButton,
    pub automation_translate_right_btn: ToolButton,
    pub automation_translate_up_btn: ToolButton,
    pub automation_translate_down_btn: ToolButton,

    pub automation_quantise_btn: ToolButton,
    pub automation_quantise_start_choice: ComboBoxText,

    pub automation_grid_edit_note_velocity: RadioToolButton,
    pub automation_grid_edit_note_expression: RadioToolButton,
    pub automation_grid_edit_controllers: RadioToolButton,
    pub automation_grid_edit_instrument_parameters: RadioToolButton,
    pub automation_grid_edit_effect_parameters: RadioToolButton,

    pub automation_grid_edit_track: RadioToolButton,
    pub automation_grid_edit_riff: RadioToolButton,

    pub automation_grid_edit_note_velocity_box: Box,
    pub automation_grid_edit_note_expression_box: Box,
    pub automation_grid_edit_controllers_box: Box,
    pub automation_grid_edit_instrument_parameters_box: Box,
    pub automation_grid_edit_effect_parameters_box: Box,
    pub automation_edit_panel_stack: Stack,

    pub automation_controller_combobox: ComboBoxText,

    pub automation_instrument_parameters_combobox: ComboBoxText,

    pub automation_effects_combobox: ComboBoxText,
    pub automation_effect_parameters_combobox: ComboBoxText,

    pub automation_note_expression_type: ComboBoxText,
    pub automation_note_expression_id: ComboBoxText,
    pub automation_note_expression_port_index: ComboBoxText,
    pub automation_note_expression_channel: ComboBoxText,
    pub automation_note_expression_key: ComboBoxText,

    pub automation_grid_mode_point: RadioToolButton,
    pub automation_grid_mode_line: RadioToolButton,
    pub automation_grid_mode_curve: RadioToolButton,

    pub riff_sets_track_panel_scrolled_window: ScrolledWindow,
    pub riff_sets_track_panel_view_port: Viewport,
    pub riff_sets_scrolled_window: ScrolledWindow,
    pub riff_sets_track_panel: Box,
    pub riff_set_heads_box: Box,
    pub riff_sets_box: Box,
    pub new_riff_set_name_entry: Entry,
    pub add_riff_set_btn: Button,
    pub riff_set_horizontal_adjustment: Adjustment,
    pub riff_set_vertical_adjustment: Adjustment,
    pub riff_sets_view_port: Viewport,

    pub riff_sequences_track_panel_scrolled_window: ScrolledWindow,
    pub riff_sequence_vertical_adjustment: Adjustment,
    pub riff_sequences_track_panel: Box,
    pub riff_sequences_tracks_view_port: Viewport,
    pub riff_sequences_box: Box,
    pub sequence_combobox: ComboBoxText,
    pub riff_sequence_name_entry: Entry,
    pub add_sequence_btn: Button,

    pub riff_arrangement_track_panel_scrolled_window: ScrolledWindow,
    pub riff_arrangement_track_panel: Box,
    pub riff_arrangement_box: Box,
    pub riff_arrangement_vertical_adjustment: Adjustment,
    pub riff_arrangement_tracks_view_port: Viewport,
    pub new_arrangement_name_entry: Entry,
    pub add_arrangement_btn: Button,
    pub arrangements_combobox: ComboBoxText,

    pub riffs_stack: Stack,

    pub song_position_txt_ctrl: Label,

    pub transport_goto_start_button: RadioToolButton,
    pub transport_move_back_button: RadioToolButton,
    pub transport_play_button: RadioToolButton,
    pub transport_record_button: ToggleToolButton,
    pub transport_stop_button: RadioToolButton,
    pub transport_loop_button: ToggleToolButton,
    pub transport_pause_button: RadioToolButton,
    pub transport_move_forward_button: RadioToolButton,
    pub transport_goto_end_button: RadioToolButton,

    pub song_tempo_spinner: SpinButton,

    pub loop_combobox_text: ComboBoxText,
    pub delete_loop_btn: ToolButton,
    pub save_loop_name_change_btn: ToolButton,
    pub loop_combobox_text_entry: Entry,

    pub panic_btn: ToolButton,

    pub track_grid_view_toggle_button: ToggleButton,
    pub riffs_view_toggle_button: ToggleButton,

    pub riff_sets_view_toggle_button: ToggleButton,
    pub riff_sequence_view_toggle_button: ToggleButton,
    pub riff_arrangement_view_toggle_button: ToggleButton,
}


impl Ui {
    pub fn get_wnd_main(&self) -> &ApplicationWindow {
        &self.wnd_main
    }
}


#[derive(Gladis, Clone)]
pub struct TrackPanel {
    pub track_panel: Frame,
    pub delete_button: Button,
    pub track_number_text: Button,
    pub track_name_text_ctrl: Entry,
    pub track_details_btn: Button,
    pub solo_toggle_btn: ToggleButton,
    pub mute_toggle_btn: ToggleButton,
    pub record_toggle_btn: ToggleButton,
    pub track_instrument_window_visibility_toggle_btn: Button,
    pub track_panel_copy_track_button: Button,
}

#[derive(Gladis, Clone)]
pub struct TrackDetailsDialogue {
    pub track_details_dialogue: Dialog,

    pub track_details_panel: Box,

    pub track_riff_choice: ComboBoxText,
    pub track_details_riff_choice_entry: Entry,
    pub track_riff_length_choice: ComboBoxText,
    pub track_delete_riff_btn: Button,
    pub track_copy_riff_btn: Button,
    pub track_details_riff_save_length: Button,
    pub track_details_riff_save_name_btn: Button,

    pub track_instrument_label: Label,
    pub track_instrument_choice: ComboBoxText,
    pub track_instrument_window_visibility_toggle_btn: Button,

    pub track_midi_device_label: Label,
    pub track_midi_device_choice: ComboBoxText,
    pub track_midi_channel_label: Label,
    pub track_midi_channel_choice: ComboBoxText,

    pub track_detail_track_colour_button: ColorButton,
    pub track_detail_riff_colour_button: ColorButton,

    pub track_send_midi_to_track_open_dialogue_button: Button,
    pub track_send_audio_to_track_open_dialogue_button: Button,

    pub track_effects_choice_label: Label,
    pub track_effects_choice: ComboBoxText,
    pub track_effects_btns_label: Label,
    pub track_add_effect_button: Button,
    pub track_effect_list: TreeView,
    pub track_effect_window_visibility_toggle_btn: Button,
    pub track_effect_delete_btn: Button,
    pub track_effects_scroll_window: ScrolledWindow,

    pub track_detail_close_button: Button,
}

#[derive(Gladis, Clone)]
pub struct MixerBlade {
    pub mixer_blade: Frame,
    pub mixer_blade_track_name_label: Label,
    pub track_details_btn: Button,
    pub mixer_blade_track_instrument_show_ui_btn: Button,
    pub mixer_blade_track_mute_toggle_btn: ToggleButton,
    pub mixer_blade_track_solo_toggle_btn: ToggleButton,
    pub mixer_blade_track_record_toggle_btn: ToggleButton,
    pub mixer_blade_track_pan_scale: Scale,
    pub mixer_blade_volume_scale: Scale,
    pub mixer_blade_right_channel_level_spin_button: SpinButton,
    pub mixer_blade_left_channel_level_spin_button: SpinButton,
    pub mixer_blade_channel_level_drawing_area: DrawingArea,
}

#[derive(Gladis, Clone)]
pub struct RiffSetBladeHead {
    pub riff_set_blade_play: Button,
    pub riff_set_blade_record: Button,
    pub riff_set_blade_copy: Button,
    pub riff_set_blade_delete: Button,
    pub riff_set_copy_to_track_view_btn: Button,
    pub riff_set_drag_btn: Button,
    pub riff_set_name_entry: Entry,
    pub riff_set_blade: Frame,
    pub riff_set_blade_box: Box,
    pub riff_set_blade_head_grid: Grid,
    pub riff_set_select_btn: Button,
}

#[derive(Gladis, Clone)]
pub struct RiffSetBlade {
    pub riff_set_box: Box,
}

#[derive(Gladis, Clone)]
pub struct RiffSequenceBlade {
    pub riff_sequence_blade_play: Button,
    pub riff_sequence_blade_copy: Button,
    pub riff_sequence_blade_delete: Button,
    pub riff_sequence_copy_to_track_view_btn: Button,
    pub riff_sequence_riff_set_combobox_label: Label,
    pub riff_sequence_name_entry: Entry,
    pub riff_sequence_blade: Frame,
    pub riff_sequence_riff_sets_scrolled_window: ScrolledWindow,
    pub riff_set_box: Box,
    pub riff_set_head_box: Box,
    pub riff_set_combobox: ComboBoxText,
    pub add_riff_set_btn: Button,
    pub riff_sequence_drag_btn: Button,
    pub riff_seq_horizontal_adjustment: Adjustment,
    pub riff_sets_view_port: Viewport,
    pub riff_sequence_select_btn: Button,
}

#[derive(Gladis, Clone)]
pub struct RiffArrangementBlade {
    pub riff_arrangement_blade_play: Button,
    pub riff_arrangement_blade_copy: Button,
    pub riff_arrangement_blade_delete: Button,
    pub riff_arrangement_copy_to_track_view_btn: Button,
    pub riff_arrangement_name_entry: Entry,
    pub riff_arrangement_blade: Frame,
    pub riff_set_box: Box,
    pub riff_set_combobox: ComboBoxText,
    pub add_riff_set_btn: Button,
    pub riff_sequence_combobox: ComboBoxText,
    pub add_riff_sequence_btn: Button,
    pub riff_arr_horizontal_adjustment: Adjustment,
    pub riff_items_view_port: Viewport,
}

#[derive(Gladis, Clone)]
pub struct RiffArrangementRiffSetBlade {
    pub local_riff_set_box: Box,
    pub riff_set_head_box: Box,
    pub riff_set_box: Box,
    pub riff_set_scrolled_window: ScrolledWindow,
}

#[derive(Gladis, Clone)]
pub struct TrackMidiRoutingDialogue {
    pub track_midi_routing_dialogue: Dialog,
    pub track_midi_routing_track_combobox_text: ComboBoxText,
    pub track_midi_routing_add_track_button: Button,
    pub track_midi_routing_scrolled_box: Box,
    pub track_midi_routing_close_button: Button,
}

#[derive(Gladis, Clone)]
pub struct TrackMidiRoutingPanel {
    pub track_midi_routing_panel: Frame,
    pub track_midi_routing_send_to_track_label: Label,
    pub track_midi_routing_midi_channel_combobox_text: ComboBoxText,
    pub track_midi_routing_note_from_combobox_text: ComboBoxText,
    pub track_midi_routing_note_to_combobox_text: ComboBoxText,
    pub track_midi_routing_delete_button: Button,
}

#[derive(Gladis, Clone)]
pub struct TrackAudioRoutingDialogue {
    pub track_audio_routing_dialogue: Dialog,
    pub track_audio_routing_track_combobox_text: ComboBoxText,
    pub track_audio_routing_add_track_button: Button,
    pub track_audio_routing_scrolled_box: Box,
    pub track_audio_routing_close_button: Button,
}

#[derive(Gladis, Clone)]
pub struct TrackAudioRoutingPanel {
    pub track_audio_routing_panel: Frame,
    pub track_audio_routing_send_to_track_label: Label,
    pub track_audio_routing_left_channel_input_index_combobox_text: ComboBoxText,
    pub track_audio_routing_right_channel_input_index_combobox_text: ComboBoxText,
    pub track_audio_routing_delete_button: Button,
}

pub struct MainWindow {
    pub ui: Ui,
    pub automation_window: gtk::Window,
    pub automation_window_stack: Stack,
    pub mixer_window: gtk::Window,
    pub mixer_window_stack: Stack,
    pub piano_roll_window: gtk::Window,
    pub piano_roll_window_stack: Stack,
    pub sample_library_window: gtk::Window,
    pub sample_library_window_stack: Stack,
    pub sample_roll_window: gtk::Window,
    pub sample_roll_window_stack: Stack,
    pub scripting_window: gtk::Window,
    pub scripting_window_stack: Stack,
    pub piano_roll_grid: Option<Arc<Mutex<BeatGrid>>>,
    pub piano_roll_grid_ruler: Option<Arc<Mutex<BeatGridRuler>>>,
    pub sample_roll_grid: Option<Arc<Mutex<BeatGrid>>>,
    pub sample_roll_grid_ruler: Option<Arc<Mutex<BeatGridRuler>>>,
    pub track_grid: Option<Arc<Mutex<BeatGrid>>>,
    pub track_grid_ruler: Option<Arc<Mutex<BeatGridRuler>>>,
    pub automation_grid: Option<Arc<Mutex<BeatGrid>>>,
    pub automation_grid_ruler: Option<Arc<Mutex<BeatGridRuler>>>,
    pub track_details_dialogues: HashMap<String, TrackDetailsDialogue>,
    pub track_midi_routing_dialogues: HashMap<String, TrackMidiRoutingDialogue>,
    pub track_audio_routing_dialogues: HashMap<String, TrackAudioRoutingDialogue>,
    pub selected_style_provider: CssProvider,
    pub track_details_dialogue_track_instrument_choice_signal_handlers: HashMap<String, SignalHandlerId>,
    pub automation_effects_choice_signal_handler_id: Option<SignalHandlerId>,

    pub widgets: Vec<Widget>,

    pub riff_set_view_riff_set_beat_grids: Arc<Mutex<HashMap<String, HashMap<String, Arc<Mutex<BeatGrid>>>>>>, // outer key = riff set uuid, inner key = track_uuid
    // outer key = riff sequence uuid, mid key = riff set uuid, inner key = track_uuid
    pub riff_sequence_view_riff_set_ref_beat_grids: Arc<Mutex<HashMap<String, Arc<Mutex<HashMap<String, HashMap<String, Arc<Mutex<BeatGrid>>>>>>>>>,
    // outer outer key = riff arrangement uuid, mid key = riff set uuid, inner key = track_uuid
    pub riff_arrangement_view_riff_set_ref_beat_grids: Arc<Mutex<HashMap<String, Arc<Mutex<HashMap<String, HashMap<String, Arc<Mutex<BeatGrid>>>>>>>>>,

    pub tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
}

impl MainWindow {

    pub fn new(
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        state: Arc<Mutex<DAWState>>
    ) -> MainWindow {
        let application = gtk::Application::new(
            Some(GTK_APPLICATION_ID),
            Default::default(),
        );

        let glade_src = include_str!("daw.glade");
        let ui = Ui::from_string(glade_src).unwrap();

        let wnd_main: ApplicationWindow = ui.wnd_main.clone();
        wnd_main.maximize();
        wnd_main.set_application(Some(&application));

        // setup drag and drop
        let _ = DRAG_N_DROP_TARGETS.len();

        MainWindow::setup_tracks_drag_and_drop(
            ui.top_level_vbox.clone(), 
            ui.track_grid_vertical_adjustment.clone(), 
            ui.track_grid_vertical_view_port.clone(), 
            tx_from_ui.clone());
        MainWindow::setup_tracks_drag_and_drop(
            ui.riff_sets_track_panel.clone(), 
            ui.riff_set_vertical_adjustment.clone(), 
            ui.riff_sets_track_panel_view_port.clone(), 
            tx_from_ui.clone());
        MainWindow::setup_tracks_drag_and_drop(
            ui.riff_sequences_track_panel.clone(), 
            ui.riff_sequence_vertical_adjustment.clone(), 
            ui.riff_sequences_tracks_view_port.clone(), 
            tx_from_ui.clone());
        MainWindow::setup_tracks_drag_and_drop(
            ui.riff_arrangement_track_panel.clone(), 
            ui.riff_arrangement_vertical_adjustment.clone(), 
            ui.riff_arrangement_tracks_view_port.clone(), 
            tx_from_ui.clone());

        {
            let state = state.clone();
            wnd_main.connect_delete_event(move |window, _| {
                let dirty = if let Ok(state) = state.lock() {
                    state.dirty
                }
                else {
                    true
                };

                if dirty {
                    let message_dialogue = MessageDialogBuilder::new()
                        .parent(window)
                        .message_type(gtk::MessageType::Question)
                        .buttons(gtk::ButtonsType::YesNo)
                        .text("There are unsaved changes - quit anyway?")
                        .title("Unsaved Changes")
                        .modal(true)
                        .build();

                    let result = message_dialogue.run();

                    message_dialogue.hide();

                    if result == ResponseType::Yes {
                        gtk::Inhibit(false)
                    }
                    else {
                        gtk::Inhibit(true)
                    }
                }
                else {
                    gtk::Inhibit(false)
                }
        });
        }

        let selected_style_provider = CssProvider::new();
        selected_style_provider.load_from_data("frame { background-color: #3f3f3f; }".as_bytes()).unwrap();
        let _ = selected_style_provider.set_property("selected", true);

        let sample_roll_window = gtk::Window::new(gtk::WindowType::Toplevel);
        sample_roll_window.set_title("Sample Roll".to_string().as_str());
        sample_roll_window.set_deletable(false);
        sample_roll_window.set_height_request(800);
        sample_roll_window.set_width_request(900);
        sample_roll_window.set_resizable(true);
        let sample_roll_window_stack = Stack::new();
        sample_roll_window.set_child(Some(&sample_roll_window_stack));

        let automation_window = gtk::Window::new(gtk::WindowType::Toplevel);
        automation_window.set_title("Automation".to_string().as_str());
        automation_window.set_deletable(false);
        automation_window.set_height_request(800);
        automation_window.set_width_request(900);
        automation_window.set_resizable(true);
        let automation_window_stack = Stack::new();
        automation_window.set_child(Some(&automation_window_stack));

        let sample_library_window = gtk::Window::new(gtk::WindowType::Toplevel);
        sample_library_window.set_title("Sample Library".to_string().as_str());
        sample_library_window.set_deletable(false);
        sample_library_window.set_height_request(800);
        sample_library_window.set_width_request(900);
        sample_library_window.set_resizable(true);
        let sample_library_window_stack = Stack::new();
        sample_library_window.set_child(Some(&sample_library_window_stack));

        let scripting_window = gtk::Window::new(gtk::WindowType::Toplevel);
        scripting_window.set_title("Scripting".to_string().as_str());
        scripting_window.set_deletable(false);
        scripting_window.set_height_request(800);
        scripting_window.set_width_request(900);
        scripting_window.set_resizable(true);
        let scripting_window_stack = Stack::new();
        scripting_window.set_child(Some(&scripting_window_stack));

        let mixer_window = gtk::Window::new(gtk::WindowType::Toplevel);
        mixer_window.set_title("Mixer".to_string().as_str());
        mixer_window.set_deletable(false);
        mixer_window.set_height_request(550);
        mixer_window.set_width_request(900);
        mixer_window.set_resizable(true);
        let mixer_window_stack = Stack::new();
        mixer_window.set_child(Some(&mixer_window_stack));

        let piano_roll_window = gtk::Window::new(gtk::WindowType::Toplevel);
        piano_roll_window.set_title("Piano Roll".to_string().as_str());
        piano_roll_window.set_deletable(false);
        piano_roll_window.set_height_request(800);
        piano_roll_window.set_width_request(900);
        piano_roll_window.set_resizable(true);
        let piano_roll_window_stack = Stack::new();
        piano_roll_window.set_child(Some(&piano_roll_window_stack));

        let mut main_window = MainWindow {
            ui: ui.clone(),
            piano_roll_grid: None,
            piano_roll_grid_ruler: None,
            sample_roll_grid: None,
            sample_roll_grid_ruler: None,
            track_grid: None,
            track_grid_ruler: None,
            automation_grid: None,
            automation_grid_ruler: None,
            track_details_dialogues: HashMap::new(),
            track_midi_routing_dialogues: HashMap::new(),
            track_audio_routing_dialogues: HashMap::new(),
            selected_style_provider,
            track_details_dialogue_track_instrument_choice_signal_handlers: HashMap::new(),
            automation_effects_choice_signal_handler_id: None,
            riff_set_view_riff_set_beat_grids: Arc::new(Mutex::new(HashMap::new())),
            riff_sequence_view_riff_set_ref_beat_grids:  Arc::new(Mutex::new(HashMap::new())),
            riff_arrangement_view_riff_set_ref_beat_grids: Arc::new(Mutex::new(HashMap::new())),
            tx_from_ui: tx_from_ui.clone(),
            piano_roll_window,
            piano_roll_window_stack,
            sample_library_window,
            sample_library_window_stack,
            sample_roll_window,
            sample_roll_window_stack,
            automation_window,
            automation_window_stack,
            mixer_window,
            mixer_window_stack,
            scripting_window,
            scripting_window_stack,
            widgets: vec![],
        };

        main_window.ui.configuration_dialogue.add_button("Cancel", gtk::ResponseType::Cancel);
        main_window.ui.configuration_dialogue.add_button("Ok", gtk::ResponseType::Ok);

        main_window.setup_menus(tx_from_ui.clone(), state.clone());
        main_window.setup_main_tool_bar(tx_from_ui.clone());
        main_window.setup_track_grid(tx_from_ui.clone(), state.clone());
        main_window.setup_mixer();
        main_window.setup_automation_grid(tx_from_ui.clone(), state.clone());
        let piano = main_window.setup_piano(tx_from_ui.clone());
        main_window.setup_piano_roll(piano, tx_from_ui.clone(), state.clone());
        main_window.setup_sample_library(tx_from_ui.clone());
        main_window.setup_scripting_view(tx_from_ui.clone());
        main_window.setup_sample_roll(tx_from_ui.clone(), state.clone());
        main_window.setup_riff_sets_view(tx_from_ui.clone(), state.clone());
        main_window.setup_riff_sequences_view(tx_from_ui.clone(), state.clone());
        main_window.setup_riff_arrangements_view(tx_from_ui.clone(), state.clone());
        main_window.setup_loops(tx_from_ui.clone(), state.clone());
        main_window.add_mixer_blade("Master", Uuid::nil(), tx_from_ui.clone(), 1.0, 0.0, GeneralTrackType::MasterTrack, ToggleButton::new(), ToggleButton::new());
        MainWindow::setup_riff_set_drag_and_drop(ui.riff_set_heads_box.clone(), ui.riff_sets_box.clone(), ui.riff_set_horizontal_adjustment.clone(), ui.riff_sets_view_port.clone(), RiffSetType::RiffSet, tx_from_ui.clone());

        {
            let centre_split_pane: Paned = ui.centre_split_pane.clone();
            let state = state.clone();
            ui.show_sub_panel_toggle_btn.connect_clicked(move |show_sub_panel_toggle_btn| {
                if show_sub_panel_toggle_btn.is_active() {
                    show_sub_panel_toggle_btn.set_label("v");
                    match state.lock() {
                        Ok(state) => {
                            centre_split_pane.set_position(state.centre_split_pane_position());
                        }
                        Err(_) => {
                            centre_split_pane.set_position(600);
                        }
                    }
                }
                else {
                    show_sub_panel_toggle_btn.set_label("^");
                    centre_split_pane.set_position(10000);
                }
            });
        }

        {
            let state = state.clone();
            ui.centre_split_pane.connect_position_notify(move |centre_split_pane| {
                {
                    let state = state.clone();
                    let mut centre_split_pane_position= centre_split_pane.position();
                    let centre_split_pane_max_position = centre_split_pane.max_position();

                    // debug!("Centre split pane position: {}", centre_split_pane_position);

                    if centre_split_pane_position < 230 {
                        centre_split_pane.set_position(230);
                        centre_split_pane_position = 230;
                    }

                    let _ = std::thread::Builder::new().name("Split position".into()).spawn(move || {
                        match state.lock() {
                            Ok(mut state) => {
                                if centre_split_pane_position != centre_split_pane_max_position
                                    && centre_split_pane_position != 10000
                                    && centre_split_pane_position != state.centre_split_pane_position() {
                                    state.set_centre_split_pane_position(centre_split_pane_position);
                                }
                            }
                            Err(_) => {}
                        }
                    });
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let recent_chooser_menu: RecentChooserMenu = ui.recent_chooser_menu.clone();
            let window = ui.wnd_main.clone();
            ui.recent_chooser_menu.connect_item_activated(move |_xxx| {
                {
                    let recent_info = recent_chooser_menu.current_item();
                    if let Some(recent_info) = recent_info {
                        let mut path = std::path::PathBuf::new();
                        if let Some(file_name) = recent_info.uri_display() {
                            window.set_title(format!("DAW - {}", file_name.as_str()).as_str());
                            path.set_file_name(&file_name);
                            {
                                let tx_from_ui = tx_from_ui.clone();
                                let _ = std::thread::Builder::new().name("Recent chooser menu".into()).spawn(move || {
                                    if let Err(error) = tx_from_ui.send(DAWEvents::OpenFile(path)) {
                                        debug!("Couldn't send open recent file from ui - failed to send with sender: {:?}", error)
                                    }
                                });
                            }
                        }
                    }
                }
            });
        }

        MainWindow::setup_transport(&mut main_window, tx_from_ui.clone());

        {
            let tx_from_ui = tx_from_ui;
            ui.song_tempo_spinner.connect_value_changed(move |tempo_spinner| {
                let tempo_value = tempo_spinner.value();
                if let Ok(tempo) = tempo_value.to_value().get() {
                    match tx_from_ui.send(DAWEvents::TempoChange(tempo)) {
                        Ok(_) => debug!(""),
                        Err(_) => debug!("Couldn't send tempo change from ui - failed to send with sender."),
                    }
                }
            });
        }

        {
            let state = state.clone();
            let riffs_view_toggle_button = ui.riffs_view_toggle_button.clone();
            let centre_panel_stack = ui.centre_panel_stack.clone();
            let automation_drawing_area = ui.automation_drawing_area.clone();
            ui.track_grid_view_toggle_button.connect_clicked(move |track_grid_view_toggle_button| {
                if track_grid_view_toggle_button.is_active() {
                    if let Ok(mut state) = state.lock() {
                        riffs_view_toggle_button.set_active(false);
                        state.set_current_view(CurrentView::Track);
                        centre_panel_stack.set_visible_child_name("Track Grid");
                        automation_drawing_area.queue_draw();
                    }
                }
            });
        }

        {
            let state = state.clone();
            let track_grid_view_toggle_button = ui.track_grid_view_toggle_button.clone();
            let centre_panel_stack = ui.centre_panel_stack.clone();
            let riff_sets_view_toggle_button = ui.riff_sets_view_toggle_button.clone();
            let riff_sequence_view_toggle_button = ui.riff_sequence_view_toggle_button.clone();
            let riff_arrangement_view_toggle_button = ui.riff_arrangement_view_toggle_button.clone();
            let automation_drawing_area = ui.automation_drawing_area.clone();
            ui.riffs_view_toggle_button.connect_clicked(move |riffs_view_toggle_button| {
                if riffs_view_toggle_button.is_active() {
                    if let Ok(mut state) = state.lock() {
                        track_grid_view_toggle_button.set_active(false);
                        centre_panel_stack.set_visible_child_name("Riffs");

                        if riff_sets_view_toggle_button.is_active() {
                            state.set_current_view(CurrentView::RiffSet);
                        }
                        else if riff_sequence_view_toggle_button.is_active() {
                            state.set_current_view(CurrentView::RiffSequence);
                        }
                        else if riff_arrangement_view_toggle_button.is_active() {
                            state.set_current_view(CurrentView::RiffArrangement);
                        }
                        automation_drawing_area.queue_draw();
                    }
                }
            });
        }

        {
            let state = state.clone();
            let riff_sequence_view_toggle_button = ui.riff_sequence_view_toggle_button.clone();
            let riff_arrangement_view_toggle_button = ui.riff_arrangement_view_toggle_button.clone();
            let riffs_stack = ui.riffs_stack.clone();
            let automation_drawing_area = ui.automation_drawing_area.clone();
            ui.riff_sets_view_toggle_button.connect_clicked(move |riff_sets_view_toggle_button| {
                if riff_sets_view_toggle_button.is_active() {
                    if let Ok(mut state) = state.lock() {
                        riff_sequence_view_toggle_button.set_active(false);
                        riff_arrangement_view_toggle_button.set_active(false);
                        state.set_current_view(CurrentView::RiffSet);
                        riffs_stack.set_visible_child_name("riff_sets");
                        automation_drawing_area.queue_draw();
                    }
                }
            });
        }

        {
            let state = state.clone();
            let riff_sets_view_toggle_button = ui.riff_sets_view_toggle_button.clone();
            let riff_arrangement_view_toggle_button = ui.riff_arrangement_view_toggle_button.clone();
            let riffs_stack = ui.riffs_stack.clone();
            let automation_drawing_area = ui.automation_drawing_area.clone();
            ui.riff_sequence_view_toggle_button.connect_clicked(move |riff_sequence_view_toggle_button| {
                if riff_sequence_view_toggle_button.is_active() {
                    if let Ok(mut state) = state.lock() {
                        riff_sets_view_toggle_button.set_active(false);
                        riff_arrangement_view_toggle_button.set_active(false);
                        state.set_current_view(CurrentView::RiffSequence);
                        riffs_stack.set_visible_child_name("riff_sequences");
                        automation_drawing_area.queue_draw();
                    }
                }
            });
        }

        {
            let state = state.clone();
            let riff_sets_view_toggle_button = ui.riff_sets_view_toggle_button.clone();
            let riff_sequence_view_toggle_button = ui.riff_sequence_view_toggle_button.clone();
            let riffs_stack = ui.riffs_stack.clone();
            let automation_drawing_area = ui.automation_drawing_area.clone();
            ui.riff_arrangement_view_toggle_button.connect_clicked(move |riff_arrangement_view_toggle_button| {
                if riff_arrangement_view_toggle_button.is_active() {
                    if let Ok(mut state) = state.lock() {
                        riff_sets_view_toggle_button.set_active(false);
                        riff_sequence_view_toggle_button.set_active(false);
                        state.set_current_view(CurrentView::RiffArrangement);
                        riffs_stack.set_visible_child_name("riff_arrangement");
                        automation_drawing_area.queue_draw();
                    }
                }
            });
        }

        main_window
    }

    pub fn clear_ui(&mut self) {
        self.track_details_dialogue_track_instrument_choice_signal_handlers.clear();

        // clear track detail panel references
        self.track_details_dialogues.clear();

        // remove track panels
        let vbox = self.ui.top_level_vbox.clone();
        self.ui.top_level_vbox.foreach(move |child| {
            vbox.remove(child);
        });

        // remove mixer blades
        let mixer_box = self.ui.mixer_box.clone();
        self.ui.mixer_box.foreach(move |child| {
            if child.widget_name() != Uuid::nil().to_string() {
                mixer_box.remove(child);
            }
        });

        // remove riff set track panels
        let children = &mut self.ui.riff_sets_track_panel.children();
        for child in children {
            self.ui.riff_sets_track_panel.remove(child);
        }

        // remove riff set blade heads
        let riff_set_heads_box = self.ui.riff_set_heads_box.clone();
        self.ui.riff_set_heads_box.foreach(move |child| {
            riff_set_heads_box.remove(child);
        });

        // remove riff set blades
        let riff_sets_box = self.ui.riff_sets_box.clone();
        self.ui.riff_sets_box.foreach(move |child| {
            riff_sets_box.remove(child);
        });

        // remove riff sequences from riff sequences combo
        self.ui.sequence_combobox.remove_all();

        // remove riff sequence track panels
        let children = &mut self.ui.riff_sequences_track_panel.children();
        for child in children {
            self.ui.riff_sequences_track_panel.remove(child);
        }

        // remove riff sequence blades
        let riff_sequences_box = self.ui.riff_sequences_box.clone();
        self.ui.riff_sequences_box.foreach(move |child| {
            riff_sequences_box.remove(child);
        });

        // remove riff arrangements from riff arrangements combo
        self.ui.arrangements_combobox.remove_all();

        // remove riff arrangement track panels
        let children = &mut self.ui.riff_arrangement_track_panel.children();
        for child in children {
            self.ui.riff_arrangement_track_panel.remove(child);
        }

        // remove riff arrangement blades
        let riff_arrangement_box = self.ui.riff_arrangement_box.clone();
        self.ui.riff_arrangement_box.foreach(move |child| {
            riff_arrangement_box.remove(child);
        });

        // reset loops
        self.ui.loop_combobox_text.remove_all();
        self.ui.loop_combobox_text_entry.set_text("");
    }

    pub fn change_track_name(&mut self, track_uuid: String, track_name: String) {
        // track from mixer blades
        let mut child_count = 1;
        let mixer_box = self.ui.mixer_box.clone();
        for child in mixer_box.children().iter_mut() {
            if child.widget_name() == track_uuid {
                if let Some(frame) =  child.dynamic_cast_ref::<Frame>() {
                    for frame_child in frame.children().iter_mut() {
                        if frame_child.widget_name() == "mixer_blade_track_name_label" {
                            if let Some(label) =  frame_child.dynamic_cast_ref::<Label>() {
                                let label_text = format!("{}. {}", child_count, track_name.clone());
                                label.set_text(label_text.as_str());
                                break;
                            }
                        }
                    }
                }
            }
            child_count += 1;
        }

        // track details dialogues
        for (_, dialogue) in self.track_details_dialogues.iter_mut() {
            if dialogue.track_details_panel.widget_name() == track_uuid {
                dialogue.track_details_dialogue.set_title(track_name.as_str());
                break;
            }
        }
    }

    pub fn delete_track_from_ui(&mut self, track_uuid: String) {
        // remove track panel
        let vbox = self.ui.top_level_vbox.clone();
        let mut child_count = 1;
        for child in vbox.children().iter_mut() {
            if child.widget_name() == track_uuid {
                vbox.remove(child);
            }
            else {
                if let Some(frame) =  child.dynamic_cast_ref::<Frame>() {
                    frame.children().iter_mut().for_each(|frame_child| {
                        if let Some(grid) =  frame_child.dynamic_cast_ref::<Grid>() {
                            grid.children().iter_mut().for_each(|grid_child| {
                                if grid_child.widget_name() == "track_number_text" {
                                    if let Some(track_number_text) =  grid_child.dynamic_cast_ref::<Button>() {
                                        let label = format!("   {}", child_count);
                                        track_number_text.set_label(label.as_str());
                                    }
                                }
                            });
                        }
                    });
                }
                child_count += 1;
            }
        }

        // remove track from mixer blades
        let mut child_count = 1;
        for child in self.ui.mixer_box.children().iter_mut() {
            if child.widget_name() == track_uuid {
                self.ui.mixer_box.remove(child);
            }
            else {
                if let Some(frame) =  child.dynamic_cast_ref::<Frame>() {
                    frame.children().iter_mut().for_each(|frame_child| {
                        if let Some(label) =  frame_child.dynamic_cast_ref::<Label>() {
                            let current_label = label.label().to_string();
                            let regex = regex::Regex::new(r"^\d+\.\s").unwrap();
                            let label_text = format!("{}. {}", child_count, regex.replace(current_label.as_str(), ""));
                            label.set_text(label_text.as_str());
                        }
                    });
                }
                child_count += 1;
            }
        }

        // remove track details dialogue
        for (_, dialogue) in self.track_details_dialogues.iter_mut() {
            if dialogue.track_details_panel.widget_name() == track_uuid {
                self.track_details_dialogues.remove(track_uuid.as_str());
                break;
            }
        }

        // remove riff set track panel
        let mut child_count = 1;
        for child in self.ui.riff_sets_track_panel.children().iter_mut() {
            if child.widget_name() == track_uuid {
                self.ui.riff_sets_track_panel.remove(child);
            }
            else {
                if let Some(frame) =  child.dynamic_cast_ref::<Frame>() {
                    frame.children().iter_mut().for_each(|frame_child| {
                        if let Some(grid) =  frame_child.dynamic_cast_ref::<Grid>() {
                            grid.children().iter_mut().for_each(|grid_child| {
                                if grid_child.widget_name() == "track_number_text" {
                                    if let Some(track_number_text) =  grid_child.dynamic_cast_ref::<Button>() {
                                        let label = format!("   {}", child_count);
                                        track_number_text.set_label(label.as_str());
                                    }
                                }
                            });
                        }
                    });
                }
                child_count += 1;
            }
        }

        // remove from riff set blades
        for riff_sets_box_child in self.ui.riff_sets_box.children().iter() {
            Self::delete_track_from_riff_set_blade2(&track_uuid, riff_sets_box_child);
        }

        // remove riff sequence track panel
        let mut child_count = 1;
        for child in self.ui.riff_sequences_track_panel.children().iter_mut() {
            if child.widget_name() == track_uuid {
                self.ui.riff_sequences_track_panel.remove(child);
            }
            else {
                if let Some(frame) =  child.dynamic_cast_ref::<Frame>() {
                    frame.children().iter_mut().for_each(|frame_child| {
                        if let Some(grid) =  frame_child.dynamic_cast_ref::<Grid>() {
                            grid.children().iter_mut().for_each(|grid_child| {
                                if grid_child.widget_name() == "track_number_text" {
                                    if let Some(track_number_text) =  grid_child.dynamic_cast_ref::<Button>() {
                                        let label = format!("   {}", child_count);
                                        track_number_text.set_label(label.as_str());
                                    }
                                }
                            });
                        }
                    });
                }
                child_count += 1;
            }
        }

        // remove from riff sequence blades
        for riff_sequence_box_child in self.ui.riff_sequences_box.children().iter() {
            Self::delete_track_from_blade(&track_uuid, riff_sequence_box_child);
        }

        // remove riff arrangement track panel
        let mut child_count = 1;
        for child in self.ui.riff_arrangement_track_panel.children().iter_mut() {
            if child.widget_name() == track_uuid {
                self.ui.riff_arrangement_track_panel.remove(child);
            }
            else {
                if let Some(frame) =  child.dynamic_cast_ref::<Frame>() {
                    frame.children().iter_mut().for_each(|frame_child| {
                        if let Some(grid) =  frame_child.dynamic_cast_ref::<Grid>() {
                            grid.children().iter_mut().for_each(|grid_child| {
                                if grid_child.widget_name() == "track_number_text" {
                                    if let Some(track_number_text) =  grid_child.dynamic_cast_ref::<Button>() {
                                        let label = format!("   {}", child_count);
                                        track_number_text.set_label(label.as_str());
                                    }
                                }
                            });
                        }
                    });
                }
                child_count += 1;
            }
        }

        // remove from riff arrangement blades
        for riff_arrangement_frame_widget in self.ui.riff_arrangement_box.children().iter() {
            Self::delete_track_from_blade(&track_uuid, riff_arrangement_frame_widget);
        }
    }

    fn delete_track_from_blade(track_uuid: &String, blade_frame_widget: &Widget) {
        if let Some(blade_frame) = blade_frame_widget.dynamic_cast_ref::<Frame>() {
            if let Some(blade_frame_child) = blade_frame.child() {
                if let Some(blade_box) = blade_frame_child.dynamic_cast_ref::<Box>() {
                    if let Some(blade_box_child) = blade_box.children().get(1) {
                        if let Some(blade_scrolled_window) = blade_box_child.dynamic_cast_ref::<ScrolledWindow>() {
                            if let Some(blade_scrolled_window_child) = blade_scrolled_window.children().get(0) {
                                if let Some(blade_viewport) = blade_scrolled_window_child.dynamic_cast_ref::<Viewport>() {
                                    if let Some(viewport_child) = blade_viewport.children().get(0) {
                                        if let Some(riff_set_box) = viewport_child.dynamic_cast_ref::<Box>() {
                                            for riff_set_frame_widget in riff_set_box.children().iter() {
                                                Self::delete_track_from_riff_set_blade(track_uuid, riff_set_frame_widget);
                                                Self::delete_track_from_blade(track_uuid, riff_set_frame_widget);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn delete_track_from_riff_set_blade(track_uuid: &String, riff_set_frame_widget: &Widget) {
        if let Some(riff_set_frame) = riff_set_frame_widget.dynamic_cast_ref::<Frame>() {
            for riff_set_frame_child in riff_set_frame.children().iter() {
                if riff_set_frame_child.widget_name() == "riff_set_blade_alignment_box" {
                    if let Some(riff_set_blade_alignment_box) = riff_set_frame_child.dynamic_cast_ref::<Box>() {
                        for riff_set_blade_alignment_box_child in riff_set_blade_alignment_box.children().iter() {
                            if riff_set_blade_alignment_box_child.widget_name() == "riff_set_box" {
                                if let Some(riff_set_box) = riff_set_blade_alignment_box_child.dynamic_cast_ref::<Box>() {
                                    let mut previous_child_option: Option<Widget> = None;
                                    for riff_set_box_child in riff_set_box.children().iter() {
                                        if let Some(previous_child) = &previous_child_option {
                                            if riff_set_box_child.widget_name().starts_with("riffset_") {
                                                previous_child.set_widget_name(riff_set_box_child.widget_name().to_string().as_str());
                                                riff_set_box_child.set_widget_name("");
                                                previous_child_option = Some(riff_set_box_child.clone());
                                            }
                                        } else if riff_set_box_child.widget_name().ends_with(track_uuid.as_str()) {
                                            riff_set_box_child.set_widget_name("");
                                            previous_child_option = Some(riff_set_box_child.clone());
                                        }
                                    }
                                    for riff_set_box_child in riff_set_box.children().iter() {
                                        if riff_set_box_child.widget_name() == "" {
                                            riff_set_box_child.set_visible(false);
                                        }
                                    }
                                }
                                break;
                            }
                        }
                    }
                    break;
                }
            }
        }
    }

    fn delete_track_from_riff_set_blade2(track_uuid: &String, riff_set_box: &Widget) {
        if let Some(riff_set_box) = riff_set_box.dynamic_cast_ref::<Box>() {
            let mut previous_child_option: Option<Widget> = None;
            for riff_set_box_child in riff_set_box.children().iter() {
                if let Some(previous_child) = &previous_child_option {
                    if riff_set_box_child.widget_name().starts_with("riffset_") {
                        previous_child.set_widget_name(riff_set_box_child.widget_name().to_string().as_str());
                        riff_set_box_child.set_widget_name("");
                        previous_child_option = Some(riff_set_box_child.clone());
                    }
                } else if riff_set_box_child.widget_name().ends_with(track_uuid.as_str()) {
                    riff_set_box_child.set_widget_name("");
                    previous_child_option = Some(riff_set_box_child.clone());
                }
            }
            for riff_set_box_child in riff_set_box.children().iter() {
                if riff_set_box_child.widget_name() == "" {
                    riff_set_box_child.set_visible(false);
                }
            }
        }
    }

    pub fn add_track(
        &mut self,
        track_name: &str,
        track_uuid: Uuid,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        state_arc: Arc<Mutex<DAWState>>,
        general_track_type: GeneralTrackType,
        midi_devices: Option<Vec<String>>,
        volume: f32,
        pan: f32,
        mute: bool,
        solo: bool,
    ) {
        let (entry_buffer, track_mute_toggle_state, track_solo_toggle_state) = {
            let tx_from_ui = tx_from_ui.clone();
            self.add_track_panel(track_name, track_uuid, tx_from_ui, general_track_type.clone())
        };

        track_mute_toggle_state.set_active(mute);
        track_solo_toggle_state.set_active(solo);

        {
            let tx_from_ui = tx_from_ui.clone();
            self.add_mixer_blade(track_name, track_uuid, tx_from_ui, volume, pan, general_track_type.clone(), track_mute_toggle_state.clone(), track_solo_toggle_state.clone());
        }
        {
            let tx_from_ui = tx_from_ui.clone();
            self.add_track_details_dialogue(track_name, track_uuid, tx_from_ui, general_track_type.clone(), midi_devices, state_arc.clone());
        }
        {
            let tx_from_ui = tx_from_ui.clone();
            self.add_riff_set_track_panel(track_name, track_uuid, tx_from_ui, general_track_type.clone(), entry_buffer.clone(), track_mute_toggle_state.clone(), track_solo_toggle_state.clone());
        }
        {
            let tx_from_ui = tx_from_ui.clone();
            self.add_riff_sequences_track_panel(track_name, track_uuid, tx_from_ui, general_track_type.clone(), entry_buffer.clone(), track_mute_toggle_state.clone(), track_solo_toggle_state.clone());
        }

        // activate the drawing area for the new track on every riff set blade
        {
            let riff_sets_box = self.ui.riff_sets_box.clone();
            Self::activate_riff_set_drawing_areas(track_uuid, &tx_from_ui, state_arc.clone(), riff_sets_box);

            let riff_sequences_box = self.ui.riff_sequences_box.clone();
            Self::navigate_to_riff_sets_and_activate(track_uuid, &tx_from_ui, &state_arc, riff_sequences_box,"riff_set_box".to_string());

            // handle the riff sequences in the arrangement riff blade
            let riff_arrangement_box = self.ui.riff_arrangement_box.clone();
            MainWindow::navigate_to_riff_sets_and_activate(track_uuid, &tx_from_ui, &state_arc, riff_arrangement_box, "GtkBox".to_string());
        }

        self.add_riff_arrangement_track_panel(track_name, track_uuid, tx_from_ui, general_track_type, entry_buffer, track_mute_toggle_state.clone(), track_solo_toggle_state.clone());
    }

    fn navigate_to_riff_sets_and_activate(track_uuid: Uuid, tx_from_ui: &crossbeam_channel::Sender<DAWEvents>, state_arc: &Arc<Mutex<DAWState>>, item_box: Box, inner_box_widget_name: String) {
        for riff_sequences_box_child in item_box.children().iter() {
            if let Some(frame) = riff_sequences_box_child.dynamic_cast_ref::<Frame>() {
                for frame_child in frame.children().iter() {
                    if let Some(top_level_box) = frame_child.dynamic_cast_ref::<Box>() {
                        for top_level_box_child in top_level_box.children().iter() {
                            if top_level_box_child.widget_name() == *"GtkScrolledWindow" {
                                if let Some(scrolled_window) = top_level_box_child.dynamic_cast_ref::<ScrolledWindow>() {
                                    for scrolled_window_child in scrolled_window.children().iter() {
                                        if scrolled_window_child.widget_name() == *"GtkViewport" {
                                            if let Some(view_port) = scrolled_window_child.dynamic_cast_ref::<Viewport>() {
                                                for view_port_child in view_port.children().iter() {
                                                    if view_port_child.widget_name() == inner_box_widget_name {
                                                        if let Some(inner_box) = view_port_child.dynamic_cast_ref::<Box>() {
                                                            MainWindow::activate_riff_set_drawing_areas_for_sequences_and_arrangements(track_uuid, tx_from_ui, state_arc.clone(), inner_box.clone());
                                                            // handle riff_sequences
                                                            MainWindow::navigate_to_riff_sets_and_activate(track_uuid, tx_from_ui, state_arc, inner_box.clone(), "riff_set_box".to_string());
                                                        }
                                                        break;
                                                    }
                                                }
                                            }
                                            break;
                                        }
                                    }
                                }
                                break;
                            }
                        }
                        break;
                    }
                }
            }
        }
    }

    fn activate_riff_set_drawing_areas(track_uuid: Uuid, tx_from_ui: &crossbeam_channel::Sender<DAWEvents>, state_arc: Arc<Mutex<DAWState>>, riff_sets_box: Box) {
        for riff_sets_box_child in riff_sets_box.children().iter() {
            if let Some(riff_set_box) = riff_sets_box_child.dynamic_cast_ref::<Box>() {
                let riff_set_uuid = riff_set_box.widget_name().to_string();

                for riff_set_box_child in riff_set_box.children().iter() {
                    // find the first widget that hasn't already been used
                    if !riff_set_box_child.widget_name().starts_with("riffset_") {
                        if let Some(drawing_area) = riff_set_box_child.dynamic_cast_ref::<DrawingArea>() {
                            drawing_area.set_visible(true);
                            drawing_area.set_widget_name(format!("riffset_{}_{}", riff_set_uuid.as_str(), track_uuid.to_string().as_str()).as_str());
                            MainWindow::setup_riff_set_rif_ref(/*riff_set_uuid.clone(), track_uuid.to_string(),*/ tx_from_ui.clone(), state_arc.clone(), drawing_area);
                        }
                        break;
                    }
                }
            }
        }
    }

    fn activate_riff_set_drawing_areas_for_sequences_and_arrangements(track_uuid: Uuid, tx_from_ui: &crossbeam_channel::Sender<DAWEvents>, state_arc: Arc<Mutex<DAWState>>, riff_sets_box: Box) {
        for riff_sets_box_child in riff_sets_box.children().iter() {
            if let Some(riff_set_frame) = riff_sets_box_child.dynamic_cast_ref::<Frame>() {
                let riff_set_uuid = riff_set_frame.widget_name().to_string();

                for riff_set_frame_child in riff_set_frame.children().iter() {
                    if riff_set_frame_child.widget_name() == "riff_set_blade_alignment_box" {
                        debug!("Found riff set blade widget: {}", riff_set_frame_child.widget_name());

                        if let Some(riff_set_blade_alignment_box) = riff_set_frame_child.dynamic_cast_ref::<Box>() {
                            for riff_set_blade_alignment_box_child in riff_set_blade_alignment_box.children().iter() {
                                debug!("Found widget: {}", riff_set_blade_alignment_box_child.widget_name());

                                if riff_set_blade_alignment_box_child.widget_name() == "riff_set_box" {
                                    if let Some(riff_set_box) = riff_set_blade_alignment_box_child.dynamic_cast_ref::<Box>() {
                                        for riff_set_box_child in riff_set_box.children().iter() {
                                            // find the first widget that hasn't already been used
                                            if !riff_set_box_child.widget_name().starts_with("riffset_") {
                                                if let Some(drawing_area) = riff_set_box_child.dynamic_cast_ref::<DrawingArea>() {
                                                    drawing_area.set_visible(true);
                                                    drawing_area.set_widget_name(format!("riffset_{}_{}", riff_set_uuid.as_str(), track_uuid.to_string().as_str()).as_str());
                                                    MainWindow::setup_riff_set_rif_ref(/*riff_set_uuid.clone(), track_uuid.to_string(),*/ tx_from_ui.clone(), state_arc.clone(), drawing_area);
                                                }
                                                break;
                                            }
                                        }
                                    }
                                    break;
                                }
                            }
                        }
                        break;
                    }
                }
            }
        }
    }

    pub fn add_track_panel(&mut self, track_name: &str, track_uuid: Uuid, tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
                           general_track_type: GeneralTrackType,
    ) -> (EntryBuffer, ToggleButton, ToggleButton)
    {
        let track_panel_glade_src = include_str!("track_panel.glade");

        let track_panel: TrackPanel = TrackPanel::from_string(track_panel_glade_src).unwrap();
        track_panel.track_panel.set_widget_name(track_uuid.to_string().as_str());

        self.ui.top_level_vbox.pack_start(&track_panel.track_panel, false, false, 0);
        let track_number_label_txt = format!("   {}", self.ui.top_level_vbox.children().len());
        track_panel.track_number_text.set_label(track_number_label_txt.as_str());
        track_panel.track_name_text_ctrl.set_text(track_name);

        debug!("$$$$$$$$$$$$$$$$$$$$$$$$$$$$ Track panel height: {}", track_panel.track_panel.allocation().height);
        
        track_panel.track_number_text.drag_source_set(
            gdk::ModifierType::BUTTON1_MASK, 
            DRAG_N_DROP_TARGETS.as_ref(), 
            gdk::DragAction::COPY);
    
        {
            let track_uuid = track_uuid.to_string();
            track_panel.track_number_text.connect_drag_data_get(move |_, _, selection_data, _, _| {
                debug!("Track drag data get called.");
                selection_data.set_text(track_uuid.as_str());
            });
        }

        match general_track_type {
            GeneralTrackType::AudioTrack => {
                track_panel.track_instrument_window_visibility_toggle_btn.set_sensitive(false);
            }
            GeneralTrackType::MidiTrack => {
                track_panel.track_instrument_window_visibility_toggle_btn.set_sensitive(false);
            }
            _ => {}
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            track_panel.delete_button.connect_clicked(move |_delete_button| {
                 match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::Deleted, Some(track_uuid.to_string()))) {
                    Err(_) => debug!("Failed to send message via tx when ui has deleted a track."),
                    _ => (),
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            track_panel.track_name_text_ctrl.connect_changed(move |entry| {
                let name = entry.text().to_string();
                let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::TrackNameChanged(name), Some(track_uuid.to_string())));
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            track_panel.solo_toggle_btn.connect_clicked(move |solo_toggle_btn| {
                if solo_toggle_btn.is_active() {
                    match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::SoloOn, Some(track_uuid.to_string()))) {
                        Err(_) => debug!("Failed to send message via tx when ui has changed solo for track."),
                        _ => (),
                    }
                }
                else {
                    let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::SoloOff, Some(track_uuid.to_string())));
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            track_panel.mute_toggle_btn.connect_clicked(move |mute_toggle_btn| {
                if mute_toggle_btn.is_active() {
                    match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::Mute, Some(track_uuid.to_string()))) {
                        Err(_) => debug!("Failed to send message via tx when ui has changed mute for track."),
                        _ => (),
                    }
                }
                else {
                    let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::Unmute, Some(track_uuid.to_string())));
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            track_panel.record_toggle_btn.connect_clicked(move |record_toggle_btn| {
                if record_toggle_btn.is_active() {
                    match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::Record(true), Some(track_uuid.to_string()))) {
                        Err(_) => debug!("Failed to send message via tx when ui has changed record state for track."),
                        _ => (),
                    }
                }
                else {
                    let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::Record(false), Some(track_uuid.to_string())));
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let track_panel_vertical_boxes = vec![self.ui.top_level_vbox.clone(), self.ui.riff_sets_track_panel.clone(), self.ui.riff_sequences_track_panel.clone(), self.ui.riff_arrangement_track_panel.clone()];
            let selected_track_style_provider = self.selected_style_provider.clone();
            track_panel.track_number_text.connect_clicked(move |_button| {
                debug!("Clicked on track: track number={}", track_uuid);
                // remove the style from all the other frames
                for riff_track_panel_vbox in track_panel_vertical_boxes.iter() {
                    for child in riff_track_panel_vbox.children() {
                        child.style_context().remove_provider(&selected_track_style_provider);
                        if child.widget_name() == track_uuid.to_string() {
                            child.style_context().add_provider(&selected_track_style_provider, gtk::STYLE_PROVIDER_PRIORITY_USER);
                        }
                    }
                }

                // notify that the track has been selected
                match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::Selected, Some(track_uuid.to_string()))) {
                    Err(_) => debug!("Failed to send message via tx when ui has selected a track."),
                    _ => {
                    },
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            track_panel.track_details_btn.connect_clicked(move |_| {
                match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::TrackDetails(true), Some(track_uuid.to_string()))) {
                    Err(_) => debug!("Problem sending message with tx from ui lock when showing track details dialog requested"),
                    _ => (),
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            track_panel.track_instrument_window_visibility_toggle_btn.connect_clicked(move |_| {
                match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::ShowInstrument, Some(track_uuid.to_string()))) {
                    Err(_) => debug!("Problem sending message with tx from ui lock when toggling instrument window visibility"),
                    _ => (),
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            track_panel.track_panel_copy_track_button.connect_clicked(move |_| {
                match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::CopyTrack, Some(track_uuid.to_string()))) {
                    Err(_) => debug!("Problem sending message with tx from ui lock when copying track"),
                    _ => (),
                }
            });
        }

        (track_panel.track_name_text_ctrl.buffer(),
        track_panel.mute_toggle_btn,
        track_panel.solo_toggle_btn)
    }

    pub fn add_riff_set_track_panel(&mut self,
        _track_name: &str,
        track_uuid: Uuid,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        general_track_type: GeneralTrackType,
        entry_buffer: EntryBuffer,
        track_mute_toggle_state: ToggleButton,
        track_solo_toggle_state: ToggleButton,
    ) {
        let track_panel_glade_src = include_str!("track_panel.glade");

        let riff_set_track_panel: TrackPanel = TrackPanel::from_string(track_panel_glade_src).unwrap();
        riff_set_track_panel.track_panel.set_widget_name(track_uuid.to_string().as_str());

        let track_panel: Frame = riff_set_track_panel.track_panel.clone();
        track_panel.set_height_request(RIFF_SET_VIEW_TRACK_PANEL_HEIGHT);

        self.ui.riff_sets_track_panel.pack_start(&riff_set_track_panel.track_panel, false, false, 0);
        let track_number_label_txt = format!("   {}", self.ui.riff_sets_track_panel.children().len());
        riff_set_track_panel.track_number_text.set_label(track_number_label_txt.as_str());
        riff_set_track_panel.track_name_text_ctrl.set_buffer(&entry_buffer);
        
        riff_set_track_panel.track_number_text.drag_source_set(
            gdk::ModifierType::BUTTON1_MASK, 
            DRAG_N_DROP_TARGETS.as_ref(), 
            gdk::DragAction::COPY);
    
        {
            let track_uuid = track_uuid.to_string();
            riff_set_track_panel.track_number_text.connect_drag_data_get(move |_, _, selection_data, _, _| {
                debug!("Track drag data get called.");
                selection_data.set_text(track_uuid.as_str());
            });
        }

        riff_set_track_panel.solo_toggle_btn.set_active(track_solo_toggle_state.is_active());
        riff_set_track_panel.mute_toggle_btn.set_active(track_mute_toggle_state.is_active());

        let _ = track_solo_toggle_state.bind_property("active", &riff_set_track_panel.solo_toggle_btn, "active").flags(BindingFlags::BIDIRECTIONAL).build();
        let _ = track_mute_toggle_state.bind_property("active", &riff_set_track_panel.mute_toggle_btn, "active").flags(BindingFlags::BIDIRECTIONAL).build();

        match general_track_type {
            GeneralTrackType::AudioTrack => {
                riff_set_track_panel.track_instrument_window_visibility_toggle_btn.set_sensitive(false);
            }
            GeneralTrackType::MidiTrack => {
                riff_set_track_panel.track_instrument_window_visibility_toggle_btn.set_sensitive(false);
            }
            _ => {}
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            riff_set_track_panel.delete_button.connect_clicked(move |_delete_button| {
                match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::Deleted, Some(track_uuid.to_string()))) {
                    Err(_) => debug!("Failed to send message via tx when ui has deleted a track."),
                    _ => (),
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            riff_set_track_panel.track_instrument_window_visibility_toggle_btn.connect_clicked(move |_| {
                match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::ShowInstrument, Some(track_uuid.to_string()))) {
                    Err(_) => debug!("Problem sending message with tx from ui lock when toggling instrument window visibility"),
                    _ => (),
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let track_panel_vertical_boxes = vec![self.ui.top_level_vbox.clone(), self.ui.riff_sets_track_panel.clone(), self.ui.riff_sequences_track_panel.clone(), self.ui.riff_arrangement_track_panel.clone()];
            let selected_track_style_provider = self.selected_style_provider.clone();
            riff_set_track_panel.track_number_text.connect_clicked(move |_button| {
                debug!("Clicked on track: track number={}", track_uuid);
                // remove the style from all the other frames
                for riff_track_panel_vbox in track_panel_vertical_boxes.iter() {
                    for child in riff_track_panel_vbox.children() {
                        child.style_context().remove_provider(&selected_track_style_provider);
                        if child.widget_name() == track_uuid.to_string() {
                            child.style_context().add_provider(&selected_track_style_provider, gtk::STYLE_PROVIDER_PRIORITY_USER);
                        }
                    }
                }

                // notify that the track has been selected
                match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::Selected, Some(track_uuid.to_string()))) {
                    Err(_) => debug!("Failed to send message via tx when ui has selected a track."),
                    _ => {
                    },
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            riff_set_track_panel.track_details_btn.connect_clicked(move |_| {
                match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::TrackDetails(true), Some(track_uuid.to_string()))) {
                    Err(_) => debug!("Problem sending message with tx from ui lock when showing track details dialog requested"),
                    _ => (),
                }
            });
        }
    }

    pub fn add_riff_sequences_track_panel(&mut self, _track_name: &str, track_uuid: Uuid, tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
                                          general_track_type: GeneralTrackType,
                                          entry_buffer: EntryBuffer,
                                          track_mute_toggle_state: ToggleButton,
                                          track_solo_toggle_state: ToggleButton,
    ) {
        let track_panel_glade_src = include_str!("track_panel.glade");

        let riff_sequence_track_panel: TrackPanel = TrackPanel::from_string(track_panel_glade_src).unwrap();
        riff_sequence_track_panel.track_panel.set_widget_name(track_uuid.to_string().as_str());
        riff_sequence_track_panel.delete_button.connect_clicked(|_item| {
            debug!("First delete button clicked.");
        });
        let track_panel: Frame = riff_sequence_track_panel.track_panel.clone();
        track_panel.set_height_request(RIFF_SEQUENCE_VIEW_TRACK_PANEL_HEIGHT);
        self.ui.riff_sequences_track_panel.pack_start(&riff_sequence_track_panel.track_panel, false, false, 0);
        let track_number_label_txt = format!("   {}", self.ui.riff_sequences_track_panel.children().len());
        riff_sequence_track_panel.track_number_text.set_label(track_number_label_txt.as_str());
        riff_sequence_track_panel.track_name_text_ctrl.set_buffer(&entry_buffer);
        
        riff_sequence_track_panel.track_number_text.drag_source_set(
            gdk::ModifierType::BUTTON1_MASK, 
            DRAG_N_DROP_TARGETS.as_ref(), 
            gdk::DragAction::COPY);
    
        {
            let track_uuid = track_uuid.to_string();
            riff_sequence_track_panel.track_number_text.connect_drag_data_get(move |_, _, selection_data, _, _| {
                debug!("Track drag data get called.");
                selection_data.set_text(track_uuid.as_str());
            });
        }

        riff_sequence_track_panel.solo_toggle_btn.set_active(track_solo_toggle_state.is_active());
        riff_sequence_track_panel.mute_toggle_btn.set_active(track_mute_toggle_state.is_active());

        let _ = track_solo_toggle_state.bind_property("active", &riff_sequence_track_panel.solo_toggle_btn, "active").flags(BindingFlags::BIDIRECTIONAL).build();
        let _ = track_mute_toggle_state.bind_property("active", &riff_sequence_track_panel.mute_toggle_btn, "active").flags(BindingFlags::BIDIRECTIONAL).build();

        match general_track_type {
            GeneralTrackType::AudioTrack => {
                riff_sequence_track_panel.track_instrument_window_visibility_toggle_btn.set_sensitive(false);
            }
            GeneralTrackType::MidiTrack => {
                riff_sequence_track_panel.track_instrument_window_visibility_toggle_btn.set_sensitive(false);
            }
            _ => {}
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            riff_sequence_track_panel.delete_button.connect_clicked(move |_delete_button| {
                match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::Deleted, Some(track_uuid.to_string()))) {
                    Err(_) => debug!("Failed to send message via tx when ui has deleted a track."),
                    _ => (),
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            riff_sequence_track_panel.track_instrument_window_visibility_toggle_btn.connect_clicked(move |_| {
                match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::ShowInstrument, Some(track_uuid.to_string()))) {
                    Err(_) => debug!("Problem sending message with tx from ui lock when toggling instrument window visibility"),
                    _ => (),
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let track_panel_vertical_boxes = vec![self.ui.top_level_vbox.clone(), self.ui.riff_sets_track_panel.clone(), self.ui.riff_sequences_track_panel.clone(), self.ui.riff_arrangement_track_panel.clone()];
            let selected_track_style_provider = self.selected_style_provider.clone();
            riff_sequence_track_panel.track_number_text.connect_clicked(move |_button| {
                debug!("Clicked on track: track number={}", track_uuid);
                // remove the style from all the other frames
                for riff_track_panel_vbox in track_panel_vertical_boxes.iter() {
                    for child in riff_track_panel_vbox.children() {
                        child.style_context().remove_provider(&selected_track_style_provider);
                        if child.widget_name() == track_uuid.to_string() {
                            child.style_context().add_provider(&selected_track_style_provider, gtk::STYLE_PROVIDER_PRIORITY_USER);
                        }
                    }
                }

                // notify that the track has been selected
                match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::Selected, Some(track_uuid.to_string()))) {
                    Err(_) => debug!("Failed to send message via tx when ui has selected a track."),
                    _ => {
                    },
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            riff_sequence_track_panel.track_details_btn.connect_clicked(move |_| {
                match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::TrackDetails(true), Some(track_uuid.to_string()))) {
                    Err(_) => debug!("Problem sending message with tx from ui lock when showing track details dialog requested"),
                    _ => (),
                }
            });
        }
    }

    pub fn add_riff_arrangement_track_panel(&mut self,
        _track_name: &str,
        track_uuid: Uuid,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        general_track_type: GeneralTrackType,
        entry_buffer: EntryBuffer,
        track_mute_toggle_state: ToggleButton,
        track_solo_toggle_state: ToggleButton,
    ) {
        let track_panel_glade_src = include_str!("track_panel.glade");

        let riff_arrangement_track_panel: TrackPanel = TrackPanel::from_string(track_panel_glade_src).unwrap();
        riff_arrangement_track_panel.track_panel.set_widget_name(track_uuid.to_string().as_str());

        riff_arrangement_track_panel.delete_button.connect_clicked(|_item| {
            debug!("First delete button clicked.");
        });
        let track_panel: Frame = riff_arrangement_track_panel.track_panel.clone();
        track_panel.set_height_request(RIFF_ARRANGEMENT_VIEW_TRACK_PANEL_HEIGHT);
        self.ui.riff_arrangement_track_panel.pack_start(&riff_arrangement_track_panel.track_panel, false, true, 0);
        let track_number_label_txt = format!("   {}", self.ui.riff_arrangement_track_panel.children().len());
        riff_arrangement_track_panel.track_number_text.set_label(track_number_label_txt.as_str());
        riff_arrangement_track_panel.track_name_text_ctrl.set_buffer(&entry_buffer);
        
        riff_arrangement_track_panel.track_number_text.drag_source_set(
            gdk::ModifierType::BUTTON1_MASK, 
            DRAG_N_DROP_TARGETS.as_ref(), 
            gdk::DragAction::COPY);
    
        {
            let track_uuid = track_uuid.to_string();
            riff_arrangement_track_panel.track_number_text.connect_drag_data_get(move |_, _, selection_data, _, _| {
                debug!("Track drag data get called.");
                selection_data.set_text(track_uuid.as_str());
            });
        }

        riff_arrangement_track_panel.solo_toggle_btn.set_active(track_solo_toggle_state.is_active());
        riff_arrangement_track_panel.mute_toggle_btn.set_active(track_mute_toggle_state.is_active());

        let _ = track_solo_toggle_state.bind_property("active", &riff_arrangement_track_panel.solo_toggle_btn, "active").flags(BindingFlags::BIDIRECTIONAL).build();
        let _ = track_mute_toggle_state.bind_property("active", &riff_arrangement_track_panel.mute_toggle_btn, "active").flags(BindingFlags::BIDIRECTIONAL).build();

        match general_track_type {
            GeneralTrackType::AudioTrack => {
                riff_arrangement_track_panel.track_instrument_window_visibility_toggle_btn.set_sensitive(false);
            }
            GeneralTrackType::MidiTrack => {
                riff_arrangement_track_panel.track_instrument_window_visibility_toggle_btn.set_sensitive(false);
            }
            _ => {}
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            riff_arrangement_track_panel.delete_button.connect_clicked(move |_delete_button| {
                match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::Deleted, Some(track_uuid.to_string()))) {
                    Err(_) => debug!("Failed to send message via tx when ui has deleted a track."),
                    _ => (),
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            riff_arrangement_track_panel.track_instrument_window_visibility_toggle_btn.connect_clicked(move |_| {
                match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::ShowInstrument, Some(track_uuid.to_string()))) {
                    Err(_) => debug!("Problem sending message with tx from ui lock when toggling instrument window visibility"),
                    _ => (),
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let track_panel_vertical_boxes = vec![self.ui.top_level_vbox.clone(), self.ui.riff_sets_track_panel.clone(), self.ui.riff_sequences_track_panel.clone(), self.ui.riff_arrangement_track_panel.clone()];
            let selected_track_style_provider = self.selected_style_provider.clone();
            riff_arrangement_track_panel.track_number_text.connect_clicked(move |_button| {
                debug!("Clicked on track: track number={}", track_uuid);
                // remove the style from all the other frames
                for riff_track_panel_vbox in track_panel_vertical_boxes.iter() {
                    for child in riff_track_panel_vbox.children() {
                        child.style_context().remove_provider(&selected_track_style_provider);
                        if child.widget_name() == track_uuid.to_string() {
                            child.style_context().add_provider(&selected_track_style_provider, gtk::STYLE_PROVIDER_PRIORITY_USER);
                        }
                    }
                }

                // notify that the track has been selected
                match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::Selected, Some(track_uuid.to_string()))) {
                    Err(_) => debug!("Failed to send message via tx when ui has selected a track."),
                    _ => {
                    },
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            riff_arrangement_track_panel.track_details_btn.connect_clicked(move |_| {
                match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::TrackDetails(true), Some(track_uuid.to_string()))) {
                    Err(_) => debug!("Problem sending message with tx from ui lock when showing track details dialog requested"),
                    _ => (),
                }
            });
        }
    }

    pub fn add_mixer_blade(
        &mut self,
        track_name: &str,
        track_uuid: Uuid,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        volume: f32,
        pan: f32,
        general_track_type: GeneralTrackType,
        track_mute_toggle_state: ToggleButton,
        track_solo_toggle_state: ToggleButton,
    ) {
        let mixer_blade_glade_src = include_str!("mixer_blade.glade");

        let mixer_blade: MixerBlade = MixerBlade::from_string(mixer_blade_glade_src).unwrap();
        mixer_blade.mixer_blade.set_widget_name(track_uuid.to_string().as_str());
        self.ui.mixer_box.pack_start(&mixer_blade.mixer_blade, false, false, 0);
        let mixer_blade_volume_scale: Scale = mixer_blade.mixer_blade_volume_scale.clone();
        mixer_blade_volume_scale.set_value((volume * 100.0) as f64);
        let mixer_blade_track_pan_scale: Scale = mixer_blade.mixer_blade_track_pan_scale.clone();
        mixer_blade_track_pan_scale.set_value((pan * 50.0) as f64);

        mixer_blade.mixer_blade_track_solo_toggle_btn.set_active(track_solo_toggle_state.is_active());
        mixer_blade.mixer_blade_track_mute_toggle_btn.set_active(track_mute_toggle_state.is_active());

        let _ = track_solo_toggle_state.bind_property("active", &mixer_blade.mixer_blade_track_solo_toggle_btn, "active").flags(BindingFlags::BIDIRECTIONAL).build();
        let _ = track_mute_toggle_state.bind_property("active", &mixer_blade.mixer_blade_track_mute_toggle_btn, "active").flags(BindingFlags::BIDIRECTIONAL).build();

        match general_track_type {
            GeneralTrackType::MasterTrack => {
                mixer_blade.mixer_blade_track_name_label.set_text(track_name.to_string().as_str());
                mixer_blade.mixer_blade_track_instrument_show_ui_btn.set_child_visible(false);
                mixer_blade.track_details_btn.set_child_visible(false);
            }
            GeneralTrackType::InstrumentTrack => {
                mixer_blade.mixer_blade_track_name_label.set_text(format!("{}. {}", self.ui.mixer_box.children().len() - 1, track_name).as_str());
            }
            GeneralTrackType::MidiTrack => {
                mixer_blade.mixer_blade_track_name_label.set_text(format!("{}. {}", self.ui.mixer_box.children().len() - 1, track_name).as_str());
                mixer_blade.mixer_blade_track_instrument_show_ui_btn.set_child_visible(false);
            }
            GeneralTrackType::AudioTrack => {
                mixer_blade.mixer_blade_track_name_label.set_text(format!("{}. {}", self.ui.mixer_box.children().len() - 1, track_name).as_str());
                mixer_blade.mixer_blade_track_instrument_show_ui_btn.set_child_visible(false);
            }
        }

        {
            let left_channel_level_spin_button = mixer_blade.mixer_blade_left_channel_level_spin_button.clone();
            let right_channel_level_spin_button = mixer_blade.mixer_blade_right_channel_level_spin_button.clone();
            let level_meter_width = 5.0;
            let gap_between_channel_levels = 5.0;
            let number_of_scale_graduations = 72.0;
            let graduations = vec![(0.0, "6"), (6.0, "0"), (12.0, "6"), (18.0, "12"), (24.0, "18"), (30.0, "24"), (36.0, "30"), (42.0, "36"), (48.0, "42"), (54.0, "48"), (60.0, "54"), (66.0, "60")];
            mixer_blade.mixer_blade_channel_level_drawing_area.connect_draw(move |drawing_area, context| {
                context.set_source_rgba(1.0, 1.0, 1.0, 1.0);
                let drawing_area_height = drawing_area.height_request() as f64;
                let graduation_height = drawing_area_height / number_of_scale_graduations;

                context.rectangle(2.0, 0.0, level_meter_width, drawing_area_height);
                context.rectangle(17.0 + level_meter_width + gap_between_channel_levels, 0.0, level_meter_width, drawing_area_height);
                context.set_line_width(0.1);
                let _ = context.stroke();

                context.set_font_size(9.0);
                for (position, text) in graduations.iter() {
                    context.move_to(11.0, *position * graduation_height + 9.0);
                    let _ = context.show_text(format!("{}", *text).as_str());
                }

                let mut left_channel = left_channel_level_spin_button.value();
                let mut right_channel = right_channel_level_spin_button.value();

                if left_channel < -66.0 {
                    left_channel = -66.0;
                }
                else if left_channel > 6.0 {
                    left_channel = 6.0;
                }

                left_channel = left_channel + 66.0;

                context.rectangle(2.0, (number_of_scale_graduations - left_channel) * graduation_height, level_meter_width, drawing_area_height);
                let _ = context.fill();

                if right_channel < -66.0 {
                    right_channel = -66.0;
                }
                else if right_channel > 6.0 {
                    right_channel = 6.0;
                }

                right_channel = right_channel + 66.0;

                context.rectangle(17.0 + level_meter_width + gap_between_channel_levels, (number_of_scale_graduations - right_channel) * graduation_height, level_meter_width, drawing_area_height);
                let _ = context.fill();

                gtk::Inhibit(false)
            });
        }

        mixer_blade.mixer_blade_channel_level_drawing_area.queue_draw();

        {
            let tx_from_ui = tx_from_ui.clone();
            let general_track_type = general_track_type.clone();
            mixer_blade.mixer_blade_volume_scale.connect_change_value(move |_a, _b, volume| {
                match general_track_type {
                    GeneralTrackType::MasterTrack => {
                        match tx_from_ui.send(DAWEvents::MasterChannelChange(MasterChannelChangeType::VolumeChange(volume / 100.0))) {
                            Ok(_) => {},
                            Err(_) => {},
                        }
                    }
                    _ => {
                        match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::Volume(None, (volume / 100.0) as f32), Some(track_uuid.to_string()))) {
                            Ok(_) => {},
                            Err(_) => {},
                        }
                    }
                }

                Inhibit(false)
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let general_track_type = general_track_type.clone();
            mixer_blade.mixer_blade_track_pan_scale.connect_change_value(move |_a, _b, pan| {
                match general_track_type {
                    GeneralTrackType::MasterTrack => {
                        match tx_from_ui.send(DAWEvents::MasterChannelChange(MasterChannelChangeType::PanChange(pan / 50.0))) {
                            Ok(_) => {},
                            Err(_) => {},
                        }
                    }
                    _ => {
                        match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::Pan(None, (pan / 50.0) as f32), Some(track_uuid.to_string()))) {
                            Ok(_) => {},
                            Err(_) => {},
                        }
                    }
                }

                Inhibit(false)
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let general_track_type = general_track_type;
            let track_uuid = track_uuid.to_string();
            mixer_blade.mixer_blade_track_instrument_show_ui_btn.connect_clicked(move |_| {
                match general_track_type {
                    GeneralTrackType::InstrumentTrack => {
                        match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::ShowInstrument, Some(track_uuid.clone()))) {
                            Ok(_) => {},
                            Err(_) => {},
                        }
                    }
                    _ => {}
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui;
            mixer_blade.track_details_btn.connect_clicked(move |_| {
                match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::TrackDetails(true), Some(track_uuid.to_string()))) {
                    Err(_) => debug!("Problem sending message with tx from ui lock when showing track details dialog requested"),
                    _ => (),
                }
            });
        }
    }

    pub fn add_track_details_dialogue(
        &mut self,
        track_name: &str,
        track_uuid: Uuid,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        general_track_type: GeneralTrackType,
        midi_devices: Option<Vec<String>>,
        state_arc: Arc<Mutex<DAWState>>,
    ) {
        let track_details_dialogue_glade_src = include_str!("track_details_dialogue.glade");

        let track_effects_list_store = ListStore::new(&[String::static_type(), String::static_type(), String::static_type(), RGBA::static_type(), RGBA::static_type()]);

        let track_details_dialogue: TrackDetailsDialogue = TrackDetailsDialogue::from_string(track_details_dialogue_glade_src).unwrap();
        track_details_dialogue.track_details_panel.set_widget_name(track_uuid.to_string().as_str());
        track_details_dialogue.track_effect_list.set_model(Some(&track_effects_list_store));


        match general_track_type {
            GeneralTrackType::InstrumentTrack => {
                track_details_dialogue.track_midi_device_label.set_child_visible(false);
                track_details_dialogue.track_midi_channel_choice.set_child_visible(false);
                track_details_dialogue.track_midi_channel_label.set_child_visible(false);
                track_details_dialogue.track_midi_device_choice.set_child_visible(false);
            }
            GeneralTrackType::AudioTrack => {
                track_details_dialogue.track_midi_device_label.set_child_visible(false);
                track_details_dialogue.track_midi_channel_choice.set_child_visible(false);
                track_details_dialogue.track_midi_channel_label.set_child_visible(false);
                track_details_dialogue.track_midi_device_choice.set_child_visible(false);
                track_details_dialogue.track_instrument_label.set_child_visible(false);
                track_details_dialogue.track_instrument_choice.set_child_visible(false);
                track_details_dialogue.track_instrument_window_visibility_toggle_btn.set_child_visible(false);
            }
            GeneralTrackType::MidiTrack => {
                track_details_dialogue.track_instrument_label.set_child_visible(false);
                track_details_dialogue.track_instrument_choice.set_child_visible(false);
                track_details_dialogue.track_instrument_window_visibility_toggle_btn.set_child_visible(false);
                track_details_dialogue.track_effects_choice_label.set_child_visible(false);
                track_details_dialogue.track_effects_choice.set_child_visible(false);
                track_details_dialogue.track_add_effect_button.set_child_visible(false);
                track_details_dialogue.track_effects_btns_label.set_child_visible(false);
                track_details_dialogue.track_effect_window_visibility_toggle_btn.set_child_visible(false);
                track_details_dialogue.track_effect_delete_btn.set_child_visible(false);
                track_details_dialogue.track_effects_scroll_window.set_child_visible(false);

                if let Some(midi_devices) = midi_devices {
                    for midi_device in midi_devices.iter() {
                        track_details_dialogue.track_midi_device_choice.append(Some(midi_device.as_str()), midi_device.as_str());
                    }
                }
            }
            _ => {}
        }

        track_details_dialogue.track_details_dialogue.set_title(track_name.to_string().as_str());

        {
            let tx_from_ui = tx_from_ui.clone();
            track_details_dialogue.track_riff_choice.connect_changed(move |track_riff_choice| {
                match track_riff_choice.active_id() {
                    Some(active_id) => {
                        debug!("Selected riff: id={:?}, text={:?}",
                        active_id.to_value(), track_riff_choice.active_text().unwrap().to_value());
                        match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffSelect(active_id.to_string()), Some(track_uuid.to_string()))) {
                            Err(_) => debug!("Problem sending message with tx from ui lock when a riff has been selected."),
                            _ => (),
                        }
                    },
                    None => (),
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            track_details_dialogue.track_midi_channel_choice.connect_changed(move |track_midi_channel_choice| {
                match track_midi_channel_choice.active_id() {
                    Some(active_id) => {
                        debug!("Selected midi channel: id={:?}, text={:?}",
                             active_id.to_value(), track_midi_channel_choice.active_text().unwrap().to_value());
                        match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::MidiOutputChannelChanged(active_id.to_string().parse::<i32>().unwrap()), Some(track_uuid.to_string()))) {
                            Err(_) => debug!("Problem sending message with tx from ui lock when a track midi channel has been selected."),
                            _ => (),
                        }
                    },
                    None => (),
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            track_details_dialogue.track_midi_device_choice.connect_changed(move |track_midi_device_choice| {
                match track_midi_device_choice.active_id() {
                    Some(active_id) => {
                        debug!("Selected midi device: id={:?}, text={:?}",
                                 active_id.to_value(), track_midi_device_choice.active_text().unwrap().to_value());
                        match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::MidiOutputDeviceChanged(active_id.to_string()), Some(track_uuid.to_string()))) {
                            Err(_) => debug!("Problem sending message with tx from ui lock when a track midi device has been selected."),
                            _ => (),
                        }
                    },
                    None => (),
                }
            });
        }

        // {
        //     let tx_from_ui = tx_from_ui.clone();
        //     let track_riff_choice = track_details_panel.track_riff_choice.clone();
        //     track_details_panel.track_riff_choice_entry.connect_activate(move |track_riff_choice_entry| {
        //         let new_text = String::from(track_riff_choice_entry.text().as_str());
        //         debug!("############################track riff choice entry enter hit: {}", new_text.as_str());
        //         let kev  = track_riff_choice.active_id();
        //         if let Some(id) = track_riff_choice.active_id() {
        //             // track_riff_choice.
        //             match tx_from_ui.try_lock() {
        //                 Ok(tx) => {
        //                     match tx.send(DAWEvents::TrackChange(TrackChangeType::RiffNameChange(id.to_string(), new_text), Some(track_uuid.to_string()))) {
        //                         Err(_) => debug!("Problem sending message with tx from ui lock when a riff has had a name change."),
        //                         _ => (),
        //                     }
        //                 },
        //                 Err(_) => debug!("Problem getting tx from ui lock when a track riff has had a name change"),
        //             }
        //         }
        //     });
        // }

        {
            // let tx_from_ui = tx_from_ui.clone();
            track_details_dialogue.track_riff_length_choice.connect_changed(move |_track_riff_length_choice| {
                // match track_riff_length_choice.active() {
                //     Some(selected_riff_index) => {
                //         debug!("Selected riff: index={}, id={:?}, text={:?}",
                //             selected_riff_index, track_riff_length_choice.active_id().unwrap().to_value(), track_riff_length_choice.active_text().unwrap().to_value());
                //         match tx_from_ui.try_lock() {
                //             Ok(tx) => {
                //                 match tx.send(DAWEvents::TrackChange(TrackChangeType::RIFF_LENGTH_CHANGED(1.0), None, Some(track_number - 1))) {
                //                     Err(_) => debug!("Problem sending message with tx from ui lock when riff length has changed."),
                //                     _ => (),
                //                 }
                //             },
                //             Err(_) => debug!("Problem getting tx from ui lock when riff length has changed"),
                //         }
                //     },
                //     None => (),
                // }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            track_details_dialogue.track_detail_track_colour_button.connect_color_set(move |track_detail_track_colour_button| {
                let selected_colour = track_detail_track_colour_button.rgba();
                match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::TrackColourChanged(selected_colour.red, selected_colour.green, selected_colour.blue, selected_colour.alpha), Some(track_uuid.to_string()))) {
                    Err(_) => debug!("Problem sending message with tx from ui lock when the track colour has been changed."),
                    _ => (),
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let track_riff_choice = track_details_dialogue.track_riff_choice.clone();
            track_details_dialogue.track_detail_riff_colour_button.connect_color_set(move |track_detail_riff_colour_button| {
                match track_riff_choice.active_id() {
                    Some(active_id) => {
                        let selected_colour = track_detail_riff_colour_button.rgba();
                        match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffColourChanged(active_id.to_string(),selected_colour.red, selected_colour.green, selected_colour.blue, selected_colour.alpha), Some(track_uuid.to_string()))) {
                            Err(_) => debug!("Problem sending message with tx from ui lock when a riff colour has been changed."),
                            _ => (),
                        }
                    },
                    None => (),
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let track_midi_routing_dialogue = self.add_track_midi_routing_dialogue(track_name.clone(), track_uuid.clone(), tx_from_ui.clone(), general_track_type.clone());
            let state_arc = state_arc.clone();
            let source_track_uuid = track_uuid.to_string();
            track_details_dialogue.track_send_midi_to_track_open_dialogue_button.connect_clicked(move |_| {
                track_midi_routing_dialogue.track_midi_routing_track_combobox_text.remove_all();
                
                // get all the tracks and instruments and effects
                if let Ok(state) = state_arc.lock() {
                    // find out if the track is an instrument track and get the instrument uuid
                    let source_track_instrument_details = {
                        state.project().song().tracks().iter().filter(|track| track.uuid().to_string() == source_track_uuid).map(|track| match track {
                            TrackType::InstrumentTrack(track) => Some((track.instrument().uuid().to_string(), track.instrument().name().to_string())),
                            TrackType::AudioTrack(_) => None,
                            TrackType::MidiTrack(_) => None,
                        }).nth(0).unwrap()
                    };

                    for track in state.project().song().tracks().iter() {
                        let destination_track_uuid = track.uuid().to_string();

                        if source_track_uuid != destination_track_uuid {
                            let desintation_track_name = track.name();

                            if let Some((source_track_instrument_uuid, source_track_instrument_name)) = &source_track_instrument_details {
                                let track_entry_id = format!("{}:{};{}:{}:{};{}", source_track_uuid, "instrument", source_track_instrument_uuid.clone(), destination_track_uuid, "none", "");
                                let track_entry_description = format!("Current track - instrument: {} to track: {}", source_track_instrument_name.clone(), desintation_track_name);
    
                                track_midi_routing_dialogue.track_midi_routing_track_combobox_text.append(
                                    Some(track_entry_id.as_str()), 
                                    track_entry_description.as_str());
                            }
            
                            let track_entry_id = format!("{}:{};{}:{}:{};{}", source_track_uuid, "none", "", destination_track_uuid, "none", "");
                            let track_entry_description = format!("Current track to track: {}", desintation_track_name);

                            track_midi_routing_dialogue.track_midi_routing_track_combobox_text.append(
                                Some(track_entry_id.as_str()), 
                                track_entry_description.as_str());

                            match track {
                                TrackType::InstrumentTrack(track) => {
                                    let instrument_uuid = track.instrument().uuid().to_string();
                                    let instrument_name = track.instrument().name().to_string();
    
                                    let track_instrument_entry_id = format!("{}:{};{}:{}:{};{}", source_track_uuid, "none", "", destination_track_uuid, "instrument", instrument_uuid);
                                    let track_instrument_entry_description = format!("Current track to track: {} - instrument: {}", desintation_track_name, instrument_name);
    
                                    track_midi_routing_dialogue.track_midi_routing_track_combobox_text.append(
                                        Some(track_instrument_entry_id.as_str()), 
                                        track_instrument_entry_description.as_str());

                                    if let Some((source_track_instrument_uuid, source_track_instrument_name)) = &source_track_instrument_details {
                                        let track_entry_id = format!("{}:{};{}:{}:{};{}", source_track_uuid, "instrument", source_track_instrument_uuid.clone(), destination_track_uuid, "none", "");
                                        let track_entry_description = format!("Current track - instrument: {} to track: {}", source_track_instrument_name.clone(), desintation_track_name);
            
                                        track_midi_routing_dialogue.track_midi_routing_track_combobox_text.append(
                                            Some(track_entry_id.as_str()), 
                                            track_entry_description.as_str());

                                        let track_entry_id = format!("{}:{};{}:{}:{};{}", source_track_uuid, "instrument", source_track_instrument_uuid.clone(), destination_track_uuid, "instrument", instrument_uuid);
                                        let track_entry_description = format!("Current track - instrument: {} to track: {} instrument: {}", source_track_instrument_name.clone(), desintation_track_name, instrument_name);
            
                                        track_midi_routing_dialogue.track_midi_routing_track_combobox_text.append(
                                            Some(track_entry_id.as_str()), 
                                            track_entry_description.as_str());
                                    }
                
                                    for effect in track.effects().iter() {
                                        let effect_uuid = effect.uuid().to_string();
                                        let effect_name = effect.name().to_string();
        
                                        let track_effect_entry_id = format!("{}:{};{}:{}:{};{}", source_track_uuid, "none", "", destination_track_uuid, "effect", effect_uuid);
                                        let track_effect_entry_description = format!("Current track to track: {} - effect: {}", desintation_track_name, effect_name);
        
                                        track_midi_routing_dialogue.track_midi_routing_track_combobox_text.append(
                                            Some(track_effect_entry_id.as_str()), 
                                            track_effect_entry_description.as_str());
        
                                        if let Some((source_track_instrument_uuid, source_track_instrument_name)) = &source_track_instrument_details {
                                            let track_effect_entry_id = format!("{}:{};{}:{}:{};{}", source_track_uuid, "instrument", source_track_instrument_uuid, destination_track_uuid, "effect", effect_uuid);
                                            let track_effect_entry_description = format!("Current track - instrument: {} to track: {} - effect: {}", source_track_instrument_name.clone(), desintation_track_name, effect_name);
            
                                            track_midi_routing_dialogue.track_midi_routing_track_combobox_text.append(
                                                Some(track_effect_entry_id.as_str()), 
                                                track_effect_entry_description.as_str());
                                        }
                                    }
                                }
                                TrackType::AudioTrack(_track) => {
    
                                }
                                TrackType::MidiTrack(_) => {}
                            }
                        }
                    }
                }

                let return_value =  track_midi_routing_dialogue.track_midi_routing_dialogue.run();

                track_midi_routing_dialogue.track_midi_routing_dialogue.hide();

                if return_value == gtk::ResponseType::Close {
                    debug!("track_midi_routing_dialogue Close on hide.");
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let track_audio_routing_dialogue = self.add_track_audio_routing_dialogue(track_name.clone(), track_uuid.clone(), tx_from_ui.clone(), general_track_type.clone());
            let state_arc = state_arc.clone();
            let source_track_uuid = track_uuid.to_string();
            track_details_dialogue.track_send_audio_to_track_open_dialogue_button.connect_clicked(move |_| {
                track_audio_routing_dialogue.track_audio_routing_track_combobox_text.remove_all();
                
                // get all the tracks and instruments and effects
                if let Ok(state) = state_arc.lock() {
                    // find out if the track is an instrument track and get the instrument uuid
                    let source_track_instrument_details = {
                        state.project().song().tracks().iter().filter(|track| track.uuid().to_string() == source_track_uuid).map(|track| match track {
                            TrackType::InstrumentTrack(track) => Some((track.instrument().uuid().to_string(), track.instrument().name().to_string())),
                            TrackType::AudioTrack(_) => None,
                            TrackType::MidiTrack(_) => None,
                        }).nth(0).unwrap()
                    };

                    for track in state.project().song().tracks().iter() {
                        let destination_track_uuid = track.uuid().to_string();

                        if source_track_uuid != destination_track_uuid {
                            let desintation_track_name = track.name();

                            if let Some((source_track_instrument_uuid, source_track_instrument_name)) = &source_track_instrument_details {
                                let track_entry_id = format!("{}:{};{}:{}:{};{}", source_track_uuid, "instrument", source_track_instrument_uuid.clone(), destination_track_uuid, "none", "");
                                let track_entry_description = format!("Current track - instrument: {} to track: {}", source_track_instrument_name.clone(), desintation_track_name);
    
                                track_audio_routing_dialogue.track_audio_routing_track_combobox_text.append(
                                    Some(track_entry_id.as_str()), 
                                    track_entry_description.as_str());
                            }
            
                            let track_entry_id = format!("{}:{};{}:{}:{};{}", source_track_uuid, "none", "", destination_track_uuid, "none", "");
                            let track_entry_description = format!("Current track to track: {}", desintation_track_name);

                            track_audio_routing_dialogue.track_audio_routing_track_combobox_text.append(
                                Some(track_entry_id.as_str()), 
                                track_entry_description.as_str());

                            match track {
                                TrackType::InstrumentTrack(track) => {
                                    let instrument_uuid = track.instrument().uuid().to_string();
                                    let instrument_name = track.instrument().name().to_string();
    
                                    let track_instrument_entry_id = format!("{}:{};{}:{}:{};{}", source_track_uuid, "none", "", destination_track_uuid, "instrument", instrument_uuid);
                                    let track_instrument_entry_description = format!("Current track to track: {} - instrument: {}", desintation_track_name, instrument_name);
    
                                    track_audio_routing_dialogue.track_audio_routing_track_combobox_text.append(
                                        Some(track_instrument_entry_id.as_str()), 
                                        track_instrument_entry_description.as_str());

                                    if let Some((source_track_instrument_uuid, source_track_instrument_name)) = &source_track_instrument_details {
                                        let track_entry_id = format!("{}:{};{}:{}:{};{}", source_track_uuid, "instrument", source_track_instrument_uuid.clone(), destination_track_uuid, "none", "");
                                        let track_entry_description = format!("Current track - instrument: {} to track: {}", source_track_instrument_name.clone(), desintation_track_name);
            
                                        track_audio_routing_dialogue.track_audio_routing_track_combobox_text.append(
                                            Some(track_entry_id.as_str()), 
                                            track_entry_description.as_str());

                                        let track_entry_id = format!("{}:{};{}:{}:{};{}", source_track_uuid, "instrument", source_track_instrument_uuid.clone(), destination_track_uuid, "instrument", instrument_uuid);
                                        let track_entry_description = format!("Current track - instrument: {} to track: {} instrument: {}", source_track_instrument_name.clone(), desintation_track_name, instrument_name);
            
                                        track_audio_routing_dialogue.track_audio_routing_track_combobox_text.append(
                                            Some(track_entry_id.as_str()), 
                                            track_entry_description.as_str());
                                    }
                
                                    for effect in track.effects().iter() {
                                        let effect_uuid = effect.uuid().to_string();
                                        let effect_name = effect.name().to_string();
        
                                        let track_effect_entry_id = format!("{}:{};{}:{}:{};{}", source_track_uuid, "none", "", destination_track_uuid, "effect", effect_uuid);
                                        let track_effect_entry_description = format!("Current track to track: {} - effect: {}", desintation_track_name, effect_name);
        
                                        track_audio_routing_dialogue.track_audio_routing_track_combobox_text.append(
                                            Some(track_effect_entry_id.as_str()), 
                                            track_effect_entry_description.as_str());
        
                                        if let Some((source_track_instrument_uuid, source_track_instrument_name)) = &source_track_instrument_details {
                                            let track_effect_entry_id = format!("{}:{};{}:{}:{};{}", source_track_uuid, "instrument", source_track_instrument_uuid, destination_track_uuid, "effect", effect_uuid);
                                            let track_effect_entry_description = format!("Current track - instrument: {} to track: {} - effect: {}", source_track_instrument_name.clone(), desintation_track_name, effect_name);
            
                                            track_audio_routing_dialogue.track_audio_routing_track_combobox_text.append(
                                                Some(track_effect_entry_id.as_str()), 
                                                track_effect_entry_description.as_str());
                                        }
                                    }
                                }
                                TrackType::AudioTrack(_track) => {
    
                                }
                                TrackType::MidiTrack(_) => {}
                            }
                        }
                    }
                }

                let return_value =  track_audio_routing_dialogue.track_audio_routing_dialogue.run();

                track_audio_routing_dialogue.track_audio_routing_dialogue.hide();

                if return_value == gtk::ResponseType::Close {
                    debug!("track_audio_routing_dialogue Close on hide.");
                }
            });
        }

        {
            let track_riff_choice = track_details_dialogue.track_riff_choice.clone();
            track_details_dialogue.track_details_riff_choice_entry.connect_focus_in_event(move |track_riff_choice_entry, _| {
                debug!("track_details_dialogue.track_details_riff_choice_entry.connect_focus_in_event...");

                if let Some(active_id) = track_riff_choice.active_id() {
                    unsafe {
                        track_riff_choice_entry.set_data("selected_riff_uuid", active_id.to_string());
                    }
                }

                gtk::Inhibit(false)
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let track_riff_choice = track_details_dialogue.track_riff_choice.clone();
            let track_riff_length_choice = track_details_dialogue.track_riff_length_choice.clone();
            track_details_dialogue.track_details_riff_choice_entry.connect_key_release_event(move |track_riff_choice_entry, event| {
                debug!("track_details_dialogue.track_details_riff_choice_entry.connect_key_release_event...");
                if event.event_type() == EventType::KeyRelease {
                    if let Some(key_name) = event.keyval().name() {
                        if key_name.as_str() == "Return" {
                            let uuid = Uuid::new_v4();
                            let id = uuid.to_string();
                            let mut riff_name = "Unknown".to_owned();
                            let riff_length = if let Some(selected_length) = track_riff_length_choice.active_text() {
                                DAWUtils::get_snap_quantise_value_in_beats_from_choice_text(selected_length.as_str(), 4.0)
                            }
                            else {
                                DAWUtils::get_snap_quantise_value_in_beats_from_choice_text("1.0", 4.0)
                            };

                            if track_riff_choice_entry.text().len() > 0 {
                                riff_name = track_riff_choice_entry.text().to_string();
                            }

                            match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffAdd(uuid, riff_name.clone(), riff_length), Some(track_uuid.to_string()))) {
                                Err(_) => debug!("Problem sending message with tx from ui lock when a riff has been added"),
                                _ => {
                                    track_riff_choice.append( Some(id.as_str()), &riff_name);
                                    track_riff_choice.set_active_id(Some(id.as_str()));

                                    unsafe {
                                        track_riff_choice_entry.set_data("selected_riff_uuid", id);
                                    }
                                },
                            }
                        }
                    }
                }
                else {
                    debug!("track_details_dialogue.track_details_riff_choice_entry.connect_key_release_event: no selected riff.");
                }

                gtk::Inhibit(false)
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let track_riff_choice = track_details_dialogue.track_riff_choice.clone();
            let track_details_riff_choice_entry = track_details_dialogue.track_details_riff_choice_entry.clone();
            track_details_dialogue.track_copy_riff_btn.connect_clicked(move |_| {
                debug!("track_details_dialogue.track_copy_riff_btn.connect_clicked...");
                if let Some(uuid_to_copy) = track_riff_choice.active_id() {
                    let uuid = Uuid::new_v4();
                    let id = uuid.to_string();
                    let mut riff_name = "Copy of ".to_owned();
                    if let Some(text) = track_riff_choice.active_text() {
                        riff_name.push_str(text.as_str());
                    }
                    match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffCopy(uuid_to_copy.to_string(), uuid, riff_name.clone()), Some(track_uuid.to_string()))) {
                        Err(_) => debug!("Problem sending message with tx from ui lock when a riff has been copied"),
                        _ => {
                            track_riff_choice.append( Some(id.as_str()), riff_name.as_str());
                            track_riff_choice.set_active_id(Some(id.as_str()));

                            unsafe {
                                track_details_riff_choice_entry.set_data("selected_riff_uuid", id.to_string());
                            }
                        },
                    }
                }
                else {
                    debug!("track_details_dialogue.track_copy_riff_btn.connect_clicked: no selected item.");
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let track_riff_choice = track_details_dialogue.track_riff_choice.clone();
            track_details_dialogue.track_delete_riff_btn.connect_clicked(move |_| {
                if let Some(riff_uuid) = track_riff_choice.active_id() {
                    match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffDelete(riff_uuid.to_string()), Some(track_uuid.to_string()))) {
                        Err(_) => debug!("Problem sending message with tx from ui lock when a riff has been deleted"),
                        _ => {},
                    }
                }
                else {
                    debug!("No riff selected delete.");
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let track_riff_choice = track_details_dialogue.track_riff_choice.clone();
            let track_riff_length_choice = track_details_dialogue.track_riff_length_choice.clone();
            track_details_dialogue.track_details_riff_save_length.connect_clicked(move |_| {
                let riff_length = if let Some(selected_length) = track_riff_length_choice.active_text() {
                    DAWUtils::get_snap_quantise_value_in_beats_from_choice_text(selected_length.as_str(), 4.0)
                }
                else {
                    DAWUtils::get_snap_quantise_value_in_beats_from_choice_text("1.0", 4.0)
                };
                match track_riff_choice.active_id() {
                    Some(riff_uuid) => match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffLengthChange(riff_uuid.to_string(), riff_length), Some(track_uuid.to_string()))) {
                        Err(_) => debug!("Problem sending message with tx from ui lock when nominating to edit a riff"),
                        _ => (),
                    },
                    None => debug!("No riff selected."),
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let track_riff_choice = track_details_dialogue.track_riff_choice.clone();
            let track_details_riff_choice_entry = track_details_dialogue.track_details_riff_choice_entry.clone();
            track_details_dialogue.track_details_riff_save_name_btn.connect_clicked(move |_| {
                debug!("track_details_dialogue.track_details_riff_save_name_btn.connect_clicked...");
                let selected_riff_uuid_option: Option<NonNull<String>> = unsafe {
                    track_details_riff_choice_entry.data("selected_riff_uuid")
                };

                if let Some(selected_riff_uuid_non_null) = selected_riff_uuid_option {
                    let selected_riff_uuid = selected_riff_uuid_non_null.as_ptr();

                    unsafe {
                        match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffNameChange((*selected_riff_uuid).clone(), track_details_riff_choice_entry.text().to_string()), Some(track_uuid.to_string()))) {
                            Err(_) => debug!("Problem sending message with tx from ui lock when changing a riff name"),
                            _ => (),
                        }
                    }

                    if let Some(tree_model) = track_riff_choice.model() {
                        if let Some(list_store) = tree_model.dynamic_cast_ref::<ListStore>() {
                            if let Some(list_store_iter) = list_store.iter_first() {
                                loop  {
                                    if let Ok(id_column_value) = list_store.value(&list_store_iter, 1).get::<String>() {
                                        unsafe {
                                            if (*selected_riff_uuid) == id_column_value {
                                                list_store.set_value(&list_store_iter, 0, &track_details_riff_choice_entry.text().to_value());
                                                break;
                                            }
                                        }
                                    }

                                    if !list_store.iter_next(&list_store_iter) {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                else {
                    debug!("track_details_dialogue.track_details_riff_save_name_btn.connect_clicked: no selected riff.");
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let track_instrument_choice_signal_handler_id = track_details_dialogue.track_instrument_choice.connect_changed(move |track_instrument_choice| {
                match track_instrument_choice.active_id() {
                    Some(instrument_shared_library_file) => {
                        let file = instrument_shared_library_file.to_string();
                        debug!("Selected instrument: id={:?}, text={:?}", file.as_str(), track_instrument_choice.active_text().unwrap().to_string());

                        match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::InstrumentChanged(file), Some(track_uuid.to_string()))) {
                            Err(_) => debug!("Problem sending message with tx from ui lock when a a track instrument has been selected."),
                            _ => (),
                        }
                    },
                    None => (),
                }
            });
            self.track_details_dialogue_track_instrument_choice_signal_handlers.insert(track_uuid.to_string(), track_instrument_choice_signal_handler_id);
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let track_effects_choice = track_details_dialogue.track_effects_choice.clone();
            let track_effects_list = track_details_dialogue.track_effect_list.clone();
            let track_effects_list_store = track_effects_list_store.clone();
            track_details_dialogue.track_add_effect_button.connect_clicked(move |_| {
                match track_effects_choice.active_id() {
                    Some(effect_shared_library_file) => {
                        let file = effect_shared_library_file.to_string();
                        let uuid = Uuid::new_v4();
                        let name = track_effects_choice.active_text().unwrap().to_string();

                        debug!("Add effect: id={}, text={}, uuid={}", file.as_str(), name.as_str(), uuid);

                        track_effects_list_store.insert_with_values(None, &[
                            (0, &name),
                            (1, &file),
                            (2, &uuid.to_string()),
                            (3, &(RGBA::black())),
                            (4, &(RGBA::white())),
                        ]);
                        track_effects_list.show_all();

                        match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::EffectAdded(uuid, name, file), Some(track_uuid.to_string()))) {
                            Err(_) => debug!("Problem sending message with tx from ui lock when a track effect is being added"),
                            _ => (),
                        }
                    },
                    None => debug!("No riff selected."),
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let track_effects_list = track_details_dialogue.track_effect_list.clone();
            let track_effects_list_store = track_effects_list_store;
            track_details_dialogue.track_effect_delete_btn.connect_clicked(move |_| {
                let selection = track_effects_list.selection();
                let (data_row_list, data_model) = selection.selected_rows();
                let mut tree_iterator_list = vec![];

                for tree_path in data_row_list.iter() {
                    let row = data_model.iter(tree_path);
                    if let Some(xxx) = row {
                        tree_iterator_list.push(xxx);
                    }
                }

                for tree_iterator in tree_iterator_list {
                    if let Some((model, iter)) = selection.selected() {
                        if let Ok(effect_uuid) = model.value(&iter, 2).get::<String>() {
                            match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::EffectDeleted(effect_uuid), Some(track_uuid.to_string()))) {
                                Ok(_) => {
                                    track_effects_list_store.remove(&tree_iterator);
                                    track_effects_list.set_model(Some(&track_effects_list_store));
                                },
                                Err(_) => debug!("Couldn't send effect delete message."),
                            }
                        }
                    }
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            track_details_dialogue.track_instrument_window_visibility_toggle_btn.connect_clicked(move |_| {
                match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::ShowInstrument, Some(track_uuid.to_string()))) {
                    Err(_) => debug!("Problem sending message with tx from ui lock when toggling instrument window visibility"),
                    _ => (),
                }
            });
        }


        {
            track_details_dialogue.track_effect_list.connect_cursor_changed(move |tree_view| {
                debug!("Track effect list row selected.");
                let selection = tree_view.selection();
                if let Some((model, iter)) = selection.selected() {
                    debug!("{}, {}, {}",
                        model.value(&iter, 0).get::<String>().expect("Tree view selection, column 0"),
                        model.value(&iter, 1).get::<String>().expect("Tree view selection, column 1"),
                        model.value(&iter, 2).get::<String>().expect("Tree view selection, column 2"),
                    );
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let track_effect_list = track_details_dialogue.track_effect_list.clone();
            track_details_dialogue.track_effect_window_visibility_toggle_btn.connect_clicked(move |_| {
                debug!("Track effect toggle visibility clicked.");
                let selection = track_effect_list.selection();
                if let Some((model, iter)) = selection.selected() {
                    debug!("{}, {}, {}",
                        model.value(&iter, 0).get::<String>().expect("Tree view selection, column 0"),
                        model.value(&iter, 1).get::<String>().expect("Tree view selection, column 1"),
                        model.value(&iter, 2).get::<String>().expect("Tree view selection, column 2"),
                    );
                    if let Ok(effect_uuid) = model.value(&iter, 2).get::<String>() {
                        match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::EffectToggleWindowVisibility(effect_uuid), Some(track_uuid.to_string()))) {
                            Err(_) => debug!("Problem sending message with tx from ui lock when toggling effect window visibility"),
                            _ => (),
                        }
                    }
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui;
            track_details_dialogue.track_detail_close_button.connect_clicked(move |_| {
                match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::TrackDetails(false), Some(track_uuid.to_string()))) {
                    Err(_) => debug!("Problem sending message with tx from ui lock when requesting hide track details"),
                    _ => (),
                }
            });
        }

        self.track_details_dialogues.insert(track_uuid.to_string(), track_details_dialogue);
    }

    pub fn add_track_midi_routing_dialogue(
        &mut self,
        _track_name: &str,
        track_uuid: Uuid,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        _general_track_type: GeneralTrackType,
    ) -> TrackMidiRoutingDialogue {
        let track_midi_routing_dialogue_glade_src = include_str!("track_midi_routing_dialogue.glade");
        let track_midi_routing_dialogue: TrackMidiRoutingDialogue = TrackMidiRoutingDialogue::from_string(track_midi_routing_dialogue_glade_src).unwrap();

        {
            let tx_from_ui = tx_from_ui.clone();
            let track_midi_routing_track_combobox_text = track_midi_routing_dialogue.track_midi_routing_track_combobox_text.clone();
            let track_midi_routing_scrolled_box = track_midi_routing_dialogue.track_midi_routing_scrolled_box.clone();
            track_midi_routing_dialogue.track_midi_routing_add_track_button.connect_clicked(move |_| {
                match track_midi_routing_track_combobox_text.active_id() {
                    Some(active_id) => {
                        let routing_description = track_midi_routing_track_combobox_text.active_text().unwrap().to_string();
                        debug!("Selected track: id={:?}, text={:?}", active_id.to_value(), routing_description.as_str());

                        let track_midi_routing_panel_glade_src = include_str!("track_midi_routing_panel.glade");
                        let track_midi_routing_panel: TrackMidiRoutingPanel = TrackMidiRoutingPanel::from_string(track_midi_routing_panel_glade_src).unwrap();

                        track_midi_routing_scrolled_box.add(&track_midi_routing_panel.track_midi_routing_panel);
                        
                        if let Some(midi_routing) = DAWUtils::parse_midi_routing_id(active_id.to_string(), routing_description.clone()) {
                            Self::setup_track_midi_routing_panel(track_midi_routing_panel, midi_routing, routing_description, tx_from_ui.clone(), track_midi_routing_scrolled_box.clone(), track_uuid);
                        }
                    },
                    None => (),
                }
            });
        }

        self.track_midi_routing_dialogues.insert(track_uuid.to_string(), track_midi_routing_dialogue.clone());

        track_midi_routing_dialogue
    }

    pub fn add_track_audio_routing_dialogue(
        &mut self,
        _track_name: &str,
        track_uuid: Uuid,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        _general_track_type: GeneralTrackType,
    ) -> TrackAudioRoutingDialogue {
        let track_audio_routing_dialogue_glade_src = include_str!("track_audio_routing_dialogue.glade");
        let track_audio_routing_dialogue: TrackAudioRoutingDialogue = TrackAudioRoutingDialogue::from_string(track_audio_routing_dialogue_glade_src).unwrap();

        {
            let tx_from_ui = tx_from_ui.clone();
            let track_audio_routing_track_combobox_text = track_audio_routing_dialogue.track_audio_routing_track_combobox_text.clone();
            let track_audio_routing_scrolled_box = track_audio_routing_dialogue.track_audio_routing_scrolled_box.clone();
            track_audio_routing_dialogue.track_audio_routing_add_track_button.connect_clicked(move |_| {
                match track_audio_routing_track_combobox_text.active_id() {
                    Some(active_id) => {
                        let routing_description = track_audio_routing_track_combobox_text.active_text().unwrap().to_string();
                        debug!("Selected track: id={:?}, text={:?}", active_id.to_value(), routing_description.as_str());

                        let track_audio_routing_panel_glade_src = include_str!("track_audio_routing_panel.glade");
                        let track_audio_routing_panel: TrackAudioRoutingPanel = TrackAudioRoutingPanel::from_string(track_audio_routing_panel_glade_src).unwrap();

                        track_audio_routing_scrolled_box.add(&track_audio_routing_panel.track_audio_routing_panel);
                        
                        if let Some(audio_routing) = DAWUtils::parse_audio_routing_id(active_id.to_string(), routing_description.clone()) {
                            Self::setup_track_audio_routing_panel(track_audio_routing_panel, audio_routing, routing_description, tx_from_ui.clone(), track_audio_routing_scrolled_box.clone(), track_uuid);
                        }
                    },
                    None => (),
                }
            });
        }

        self.track_audio_routing_dialogues.insert(track_uuid.to_string(), track_audio_routing_dialogue.clone());

        track_audio_routing_dialogue
    }

    pub fn setup_menus(
        &mut self,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        _state: Arc<Mutex<DAWState>>,
    ) {
        {
            let tx_from_ui = tx_from_ui.clone();
            let window = self.ui.wnd_main.clone();
            self.ui.menu_item_new.connect_button_press_event(move |_, _| {
                window.set_title("DAW - New");
                let _ = tx_from_ui.send(DAWEvents::NewFile);
                Inhibit(true)
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let window = self.ui.get_wnd_main().clone();
            self.ui.menu_item_open.connect_button_press_event(move |_, _|{
                let dialog = FileChooserDialog::new(Some("DAW project file"),     Some(&window), FileChooserAction::Open);
                let filter = FileFilter::new();
                filter.add_mime_type("application/json");
                filter.set_name(Some("DAW project file"));
                filter.add_pattern("*.fdaw");
                dialog.add_filter(&filter);
                dialog.add_button("Cancel", gtk::ResponseType::Cancel);
                dialog.add_button("Ok", gtk::ResponseType::Ok);
                let result = dialog.run();
                if result == gtk::ResponseType::Ok {
                    if let Some(filename) = dialog.filename() {
                        if let Some(filename_display) = filename.to_str() {
                            window.set_title(format!("DAW - {}", filename_display).as_str());
                        }
                        let _ = tx_from_ui.send(DAWEvents::OpenFile(filename));
                    }
                }
                dialog.hide();

                Inhibit(true)
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            self.ui.menu_item_save.connect_button_press_event(move |_, _| {
                debug!("Menu item save clicked!");
                let _ = tx_from_ui.send(DAWEvents::Save);
                Inhibit(true)
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let window = self.ui.get_wnd_main().clone();
            self.ui.menu_item_save_as.connect_button_press_event(move |_menu_item, _btn| {
                debug!("Menu item save as clicked!");
                let dialog = FileChooserDialog::new(Some("DAW save as project file"),     Some(&window), FileChooserAction::Save);
                let filter = FileFilter::new();
                filter.add_mime_type("application/json");
                filter.set_name(Some("DAW project file"));
                filter.add_pattern("*.fdaw");
                dialog.add_filter(&filter);
                dialog.add_button("Cancel", gtk::ResponseType::Cancel);
                dialog.add_button("Ok", gtk::ResponseType::Ok);
                let result = dialog.run();
                if result == gtk::ResponseType::Ok {
                    if let Some(filename) = dialog.filename() {
                        if let Some(filename_display) = filename.to_str() {
                            window.set_title(format!("DAW - {}", filename_display).as_str());
                        }
                        let _ = tx_from_ui.send(DAWEvents::SaveAs(filename));
                    }
                }
                dialog.hide();

                Inhibit(true)
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let window = self.ui.get_wnd_main().clone();
            self.ui.menu_item_import_midi.connect_button_press_event(move |_menu_item, _btn|{
                let dialog = FileChooserDialog::new(Some("Import midi file"),     Some(&window), FileChooserAction::Open);
                let filter = FileFilter::new();
                filter.add_mime_type("audio/midi");
                filter.set_name(Some("DAW project file"));
                filter.add_pattern("*.mid");
                dialog.add_filter(&filter);
                dialog.add_button("Cancel", gtk::ResponseType::Cancel);
                dialog.add_button("Ok", gtk::ResponseType::Ok);
                let result = dialog.run();
                if result == gtk::ResponseType::Ok {
                    let filename = dialog.filename();
                    let _ = tx_from_ui.send(DAWEvents::ImportMidiFile(filename.unwrap()));
                }
                dialog.hide();

                Inhibit(true)
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let window = self.ui.get_wnd_main().clone();
            self.ui.menu_item_export_midi.connect_button_press_event(move |_menu_item, _btn|{
                let dialog = FileChooserDialog::new(Some("Export to midi file..."),     Some(&window), FileChooserAction::Save);
                let filter = FileFilter::new();
                filter.add_mime_type("audio/midi");
                filter.set_name(Some("Midi file"));
                filter.add_pattern("*.mid");
                dialog.add_filter(&filter);
                dialog.add_button("Cancel", gtk::ResponseType::Cancel);
                dialog.add_button("Ok", gtk::ResponseType::Ok);
                let result = dialog.run();
                if result == gtk::ResponseType::Ok {
                    let filename = dialog.filename();
                    let _ = tx_from_ui.send(DAWEvents::ExportMidiFile(filename.unwrap()));
                }
                dialog.hide();

                Inhibit(true)
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let window = self.ui.get_wnd_main().clone();
            self.ui.menu_item_export_midi_riffs.connect_button_press_event(move |_menu_item, _btn|{
                let dialog = FileChooserDialog::new(Some("Export riffs to midi file..."),     Some(&window), FileChooserAction::Save);
                let filter = FileFilter::new();
                filter.add_mime_type("audio/midi");
                filter.set_name(Some("Midi file"));
                filter.add_pattern("*.mid");
                dialog.add_filter(&filter);
                dialog.add_button("Cancel", gtk::ResponseType::Cancel);
                dialog.add_button("Ok", gtk::ResponseType::Ok);
                let result = dialog.run();
                if result == gtk::ResponseType::Ok {
                    let filename = dialog.filename();
                    let _ = tx_from_ui.send(DAWEvents::ExportRiffsToMidiFile(filename.unwrap()));
                }
                dialog.hide();

                Inhibit(true)
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let window = self.ui.get_wnd_main().clone();
            self.ui.menu_item_export_midi_riffs_separate.connect_button_press_event(move |_menu_item, _btn|{
                let dialog = FileChooserDialog::new(Some("Export riffs to separate midi files in directory..."),     Some(&window), FileChooserAction::SelectFolder);
                dialog.add_button("Cancel", gtk::ResponseType::Cancel);
                dialog.add_button("Ok", gtk::ResponseType::Ok);
                let result = dialog.run();
                if result == gtk::ResponseType::Ok {
                    let directory = dialog.current_folder();
                    let _ = tx_from_ui.send(DAWEvents::ExportRiffsToSeparateMidiFiles(directory.unwrap()));
                }
                dialog.hide();

                Inhibit(true)
            });
        }

        {
            let tx_from_ui = tx_from_ui;
            let window = self.ui.get_wnd_main().clone();
            self.ui.menu_item_export_wave.connect_button_press_event(move |_menu_item, _btn|{
                let dialog = FileChooserDialog::new(Some("Export to wave file..."),     Some(&window), FileChooserAction::Save);
                let filter = FileFilter::new();
                filter.add_mime_type("audio/vnd.wav");
                filter.set_name(Some("Wave file"));
                filter.add_pattern("*.wav");
                dialog.add_filter(&filter);
                dialog.add_button("Cancel", gtk::ResponseType::Cancel);
                dialog.add_button("Ok", gtk::ResponseType::Ok);
                let result = dialog.run();
                if result == gtk::ResponseType::Ok {
                    let filename = dialog.filename();
                    let _ = tx_from_ui.send(DAWEvents::ExportWaveFile(filename.unwrap()));
                }
                dialog.hide();

                Inhibit(true)
            });
        }

        {
            let window = self.ui.get_wnd_main().clone();
            self.ui.menu_item_quit.connect_button_press_event(move |_menu_item, _btn|{
                let event = gdk::Event::new(EventType::Delete);
                let args = [event.to_value()];

                let _ = window.emit_by_name_with_values("delete_event", &args);

                Inhibit(false)
            });
        }

        {
            let about_dialogue = self.ui.about_dialogue.clone();
            self.ui.menu_item_about.connect_button_press_event(move |_menu_item, _btn|{
                about_dialogue.run();
                about_dialogue.hide();

                Inhibit(true)
            });
        }

        {
            let configuration_dialogue = self.ui.configuration_dialogue.clone();
            self.ui.menu_item_preferences.connect_button_press_event(move |_menu_item, _btn|{
                configuration_dialogue.run();
                configuration_dialogue.hide();

                Inhibit(true)
            });
        }
    }

    pub fn setup_main_tool_bar(
        &mut self,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
    ) {
        {
            let tx_from_ui = tx_from_ui.clone();
            let toolbar_add_track_combobox = self.ui.toolbar_add_track_combobox.clone();
            self.ui.toolbar_add_track.connect_clicked(move |_button|{
                // need to check the ui.toolbar_add_track_combobox for the track type and propagate it
                if let Some(track_type) = toolbar_add_track_combobox.active_id() {
                    debug!("ui.toolbar_add_track.connect_clicked: sending track add msg...");
                    match track_type.to_string().as_str() {
                        "audio_track" => {
                            let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::Added(GeneralTrackType::AudioTrack), None));
                        },
                        "midi_track" => {
                            let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::Added(GeneralTrackType::MidiTrack), None));
                        },
                        "instrument_track" => {
                            let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::Added(GeneralTrackType::InstrumentTrack), None));
                        },
                        _ => {}
                    }
                    debug!("ui.toolbar_add_track.connect_clicked: sent track add msg.");
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            self.ui.toolbar_undo.connect_clicked(move |_|{
                let _ = tx_from_ui.send(DAWEvents::Undo);
            });
        }

        {
            let tx_from_ui = tx_from_ui;
            self.ui.toolbar_redo.connect_clicked(move |_|{
                let _ = tx_from_ui.send(DAWEvents::Redo);
            });
        }
    }

    pub fn setup_loops(
        &mut self,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        _state: Arc<Mutex<DAWState>>,
    ) {
        {
            let loop_combobox_text = self.ui.loop_combobox_text.clone();
            self.ui.loop_combobox_text_entry.connect_focus_in_event(move |loop_combobox_text_entry, _| {
                debug!("loop_combobox_text_entry.connect_focus_in_event...");

                if let Some(active_id) = loop_combobox_text.active_id() {
                    unsafe {
                        loop_combobox_text_entry.set_data("selected_loop_uuid", active_id.to_string());
                    }
                }

                gtk::Inhibit(false)
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let loop_combobox_text = self.ui.loop_combobox_text.clone();
            self.ui.loop_combobox_text_entry.connect_key_release_event(move |loop_combobox_text_entry, event| {
                debug!("loop_combobox_text_entry.connect_key_release_event...");
                if event.event_type() == EventType::KeyRelease {
                    if let Some(key_name) = event.keyval().name() {
                        if key_name.as_str() == "Return" {
                            let uuid = Uuid::new_v4();
                            let id = uuid.to_string();
                            let mut loop_name = "Unknown".to_owned();

                            if loop_combobox_text_entry.text().len() > 0 {
                                loop_name = loop_combobox_text_entry.text().to_string();
                            }

                            match tx_from_ui.send(DAWEvents::LoopChange(LoopChangeType::Added(loop_name.clone()), uuid)) {
                                Err(_) => debug!("Problem sending message with tx from ui lock when a loop has been added"),
                                _ => {
                                    loop_combobox_text.append( Some(id.as_str()), &loop_name);
                                    loop_combobox_text.set_active_id(Some(id.as_str()));

                                    unsafe {
                                        loop_combobox_text_entry.set_data("selected_loop_uuid", id);
                                    }
                                },
                            }
                        }
                    }
                }
                else {
                    debug!("loop_combobox_text_entry.connect_key_release_event: no selected loop.");
                }

                gtk::Inhibit(false)
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let loop_combobox_text = self.ui.loop_combobox_text.clone();
            self.ui.delete_loop_btn.connect_clicked(move |_button| {
                debug!("ui.toolbar_delete_loop.connect_clicked: sending loop delete msg...");
                if let Some(id) = loop_combobox_text.active_id() {
                    match Uuid::parse_str(id.as_str()) {
                        Ok(uuid) => {
                            let _ = tx_from_ui.send(DAWEvents::LoopChange(LoopChangeType::Deleted, uuid));
                            if let Some(active_index) = loop_combobox_text.active() {
                                gtk::prelude::ComboBoxTextExt::remove(&loop_combobox_text, active_index as i32);
                            }
                            debug!("ui.toolbar_delete_loop.connect_clicked: sent delete loop msg.");

                        },
                        Err(error) => debug!("Could not parse loop uuid from combobox: {}", error),
                    }
                }
                else {
                    debug!("ui.toolbar_delete_loop.connect_clicked: no selected item.");
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            self.ui.loop_combobox_text.connect_changed(move |combo| {
                debug!("ui.toolbar_loop_combo.connect_changed: sending loop select msg...");
                if let Some(id) = combo.active_id() {
                    match Uuid::parse_str(id.as_str()) {
                        Ok(uuid) => {
                            let _ = tx_from_ui.send(DAWEvents::LoopChange(LoopChangeType::ActiveLoopChanged(Some(uuid)), uuid));
                            debug!("ui.toolbar_loop_combo.connect_changed: sent loop select msg.");
                        },
                        Err(error) => debug!("Could not parse loop uuid from combobox: {}", error),
                    }
                }
                else {
                    debug!("ui.toolbar_loop_combo.connect_changed: no selected item.");
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let loop_combobox_text = self.ui.loop_combobox_text.clone();
            let loop_combobox_text_entry = self.ui.loop_combobox_text_entry.clone();
            self.ui.save_loop_name_change_btn.connect_clicked(move |_| {
                debug!("ui.save_loop_name_change_btn.connect_clicked: sending loop name change msg...");
                let selected_loop_uuid_option: Option<NonNull<String>> = unsafe {
                    loop_combobox_text_entry.data("selected_loop_uuid")
                };

                if let Some(selected_loop_uuid_non_null) = selected_loop_uuid_option {
                    let selected_loop_uuid = selected_loop_uuid_non_null.as_ptr();

                    unsafe {
                        match Uuid::parse_str((*selected_loop_uuid).to_string().as_str()) {
                            Ok(uuid) => {
                                match tx_from_ui.send(DAWEvents::LoopChange(LoopChangeType::NameChanged(loop_combobox_text_entry.text().to_string()), uuid)) {
                                    Err(_) => debug!("Problem sending message with tx from ui lock when changing a loop name"),
                                    _ => (),
                                }
                            },
                            Err(_) => todo!(),
                        }
                    }

                    if let Some(tree_model) = loop_combobox_text.model() {
                        if let Some(list_store) = tree_model.dynamic_cast_ref::<ListStore>() {
                            if let Some(list_store_iter) = list_store.iter_first() {
                                loop  {
                                    if let Ok(id_column_value) = list_store.value(&list_store_iter, 1).get::<String>() {
                                        unsafe {
                                            if (*selected_loop_uuid) == id_column_value {
                                                list_store.set_value(&list_store_iter, 0, &loop_combobox_text_entry.text().to_value());
                                                break;
                                            }
                                        }
                                    }

                                    if !list_store.iter_next(&list_store_iter) {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                else {
                    debug!("ui.save_loop_name_change_btn.connect_clicked: no loop riff.");
                }
            });
        }


        {
            let tx_from_ui = tx_from_ui.clone();
            self.ui.panic_btn.connect_clicked(move |_| {
                debug!("ui.toolbar_panic_btn.connect_clicked: sending panic msg...");
                match tx_from_ui.send(DAWEvents::Panic) {
                    Ok(_) => (),
                    Err(error) => debug!("Problem sending panic message: {}", error),
                }
            });
        }
    }

    pub fn setup_track_grid(
        &mut self,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        state: Arc<Mutex<DAWState>>,
    ) {
        let event_sender = std::boxed::Box::new(|original_riff: Riff, changed_riff: Riff, track_uuid: String, tx_from_ui: Sender<DAWEvents>| {
            let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffReferenceChange(original_riff, changed_riff), Some(track_uuid)));
        });
        let track_grid_custom_painter = TrackGridCustomPainter::new_with_edit_item_handler(state.clone(), EditItemHandler::new(event_sender));
        let track_grid = BeatGrid::new_with_custom(
            0.04,
            1.0,
            18.0,
            50.0,
            4,
            Some(std::boxed::Box::new(track_grid_custom_painter)),
            Some(std::boxed::Box::new(TrackGridMouseCoordHelper)),
            tx_from_ui.clone(),
            true,
            Some(DrawingAreaType::TrackGrid),
        );
        let track_grid_arc = Arc::new( Mutex::new(track_grid));

        self.set_track_grid(Some(track_grid_arc.clone()));

        let track_grid_ruler = BeatGridRuler::new(0.04, 50.0, 4, tx_from_ui.clone());
        let track_grid_ruler_arc = Arc::new(Mutex::new(track_grid_ruler));

        self.set_piano_roll_grid_ruler(Some(track_grid_ruler_arc.clone()));

        {
            let grid = track_grid_arc.clone();
            self.ui.track_drawing_area.connect_draw(move |drawing_area, context|{
                match grid.lock() {
                    Ok(mut grid) => grid.paint(context, drawing_area),
                    Err(_) => (),
                }
                Inhibit(false)
            });
        }

        {
            let track_grid = track_grid_arc.clone();
            self.ui.track_drawing_area.connect_motion_notify_event(move |track_grid_drawing_area, motion_event| {
                let coords = motion_event.coords().unwrap();
                let control_key_pressed = motion_event.state().intersects(gdk::ModifierType::CONTROL_MASK);
                let shift_key_pressed = motion_event.state().intersects(gdk::ModifierType::SHIFT_MASK);
                let alt_key_pressed = motion_event.state().intersects(gdk::ModifierType::MOD1_MASK);
                let mouse_button = if motion_event.state().intersects(gdk::ModifierType::BUTTON1_MASK) {
                    MouseButton::Button1
                }
                else if motion_event.state().intersects(gdk::ModifierType::BUTTON2_MASK) {
                    MouseButton::Button2
                }
                else {
                    MouseButton::Button3
                };
                // debug!("Track grid mouse motion: x={}, y={}, Shift key: {}, Control key: {}", coords.0, coords.1, shift_key_pressed, control_key_pressed);
                match track_grid.lock() {
                    Ok(mut grid) => {
                        grid.handle_mouse_motion(coords.0, coords.1, track_grid_drawing_area, mouse_button, control_key_pressed, shift_key_pressed, alt_key_pressed);
                    },
                    Err(_) => (),
                }

                track_grid_drawing_area.queue_draw();

                Inhibit(false)
            });
        }

        {
            let track_grid = track_grid_arc.clone();
            self.ui.track_drawing_area.connect_button_press_event(move |track_grid_drawing_area, event_btn| {
                let coords = event_btn.coords().unwrap();
                let control_key_pressed = event_btn.state().intersects(gdk::ModifierType::CONTROL_MASK);
                let shift_key_pressed = event_btn.state().intersects(gdk::ModifierType::SHIFT_MASK);
                let alt_key_pressed = event_btn.state().intersects(gdk::ModifierType::MOD1_MASK);
                let mouse_button = if event_btn.state().intersects(gdk::ModifierType::BUTTON1_MASK) {
                    MouseButton::Button3
                }
                else if event_btn.state().intersects(gdk::ModifierType::BUTTON2_MASK) {
                    MouseButton::Button2
                }
                else {
                    MouseButton::Button1
                };
                debug!("Track grid mouse pressed coords: x={}, y={}, Shift key: {}, Control key: {}", coords.0, coords.1, shift_key_pressed, control_key_pressed);
                match track_grid.lock() {
                    Ok(mut grid) => {
                        grid.handle_mouse_press(coords.0, coords.1, track_grid_drawing_area, mouse_button, control_key_pressed, shift_key_pressed, alt_key_pressed);
                    },
                    Err(_) => (),
                }
                Inhibit(false)
            });
        }

        {
            let track_grid = track_grid_arc.clone();
            self.ui.track_drawing_area.connect_button_release_event(move |track_grid_drawing_area, event_btn| {
                let coords = event_btn.coords().unwrap();
                let control_key_pressed = event_btn.state().intersects(gdk::ModifierType::CONTROL_MASK);
                let shift_key_pressed = event_btn.state().intersects(gdk::ModifierType::SHIFT_MASK);
                let alt_key_pressed = event_btn.state().intersects(gdk::ModifierType::MOD1_MASK);
                let mouse_button = if event_btn.state().intersects(gdk::ModifierType::BUTTON1_MASK) {
                    MouseButton::Button1
                }
                else if event_btn.state().intersects(gdk::ModifierType::BUTTON2_MASK) {
                    MouseButton::Button2
                }
                else {
                    MouseButton::Button3
                };
                debug!("Track grid mouse released: x={}, y={}, Shift key: {}, Control key: {}", coords.0, coords.1, shift_key_pressed, control_key_pressed);

                track_grid_drawing_area.grab_focus();

                match track_grid.lock() {
                    Ok(mut grid) => grid.handle_mouse_release(coords.0, coords.1, track_grid_drawing_area, mouse_button, control_key_pressed, shift_key_pressed, alt_key_pressed, String::from("")),
                    Err(_) => (),
                }
                Inhibit(false)
            });
        }

        {
            let track_grid = track_grid_arc.clone();
            let track_grid_vertical_zoom_adjustment = self.ui.track_grid_vertical_zoom_adjustment.clone();
            let track_grid_zoom_adjustment = self.ui.track_grid_zoom_adjustment.clone();
            let track_grid_select_mode_btn = self.ui.track_grid_select_mode_btn.clone();
            let track_grid_add_mode_btn = self.ui.track_grid_add_mode_btn.clone();
            let track_grid_delete_mode_btn = self.ui.track_grid_delete_mode_btn.clone();
            let track_grid_edit_mode_btn = self.ui.track_grid_edit_mode_btn.clone();
            self.ui.track_drawing_area.connect_key_press_event(move |track_drawing_area, event_key| {
                let control_key_pressed = event_key.state().intersects(gdk::ModifierType::CONTROL_MASK);
                let shift_key_pressed = event_key.state().intersects(gdk::ModifierType::SHIFT_MASK);
                let alt_key_pressed = event_key.state().intersects(gdk::ModifierType::MOD1_MASK);
                let key_pressed_value = event_key.keyval().name();

                if let Some(key_name) = key_pressed_value {
                    debug!("Track grid key press: Key name={}, Shift key: {}, Control key: {}, Alt key: {}", key_name.as_str(), shift_key_pressed, control_key_pressed, alt_key_pressed);

                    if key_name == "c" && control_key_pressed && !shift_key_pressed && !alt_key_pressed {
                        // send the track grid copy message
                        match track_grid.lock() {
                            Ok(mut grid) => {
                                grid.handle_copy(track_drawing_area);
                            }
                            Err(_) => {}
                        }
                    }
                    else if key_name == "x" && control_key_pressed && !shift_key_pressed && !alt_key_pressed {
                        // send the track grid cut message
                        match track_grid.lock() {
                            Ok(mut grid) => {
                                grid.handle_cut(track_drawing_area);
                            }
                            Err(_) => {}
                        }
                    }
                    else if key_name == "v" && control_key_pressed && !shift_key_pressed && !alt_key_pressed {
                        // send the track grid paste message
                        match track_grid.lock() {
                            Ok(mut grid) => {
                                grid.handle_paste(track_drawing_area);
                            }
                            Err(_) => {}
                        }
                    }
                    else if key_name == "a" && control_key_pressed && !shift_key_pressed && !alt_key_pressed {
                        // send the track grid select all message
                        match track_grid.lock() {
                            Ok(_grid) => {
                                debug!("Not implemented yet!");
                            }
                            Err(_) => {}
                        }
                    }
                    else if key_name == "plus" && !control_key_pressed && shift_key_pressed && !alt_key_pressed {
                        track_grid_vertical_zoom_adjustment.set_value(track_grid_vertical_zoom_adjustment.value() + track_grid_vertical_zoom_adjustment.minimum_increment());
                        track_grid_zoom_adjustment.set_value(track_grid_zoom_adjustment.value() + track_grid_zoom_adjustment.minimum_increment());
                    }
                    else if key_name == "minus" && !control_key_pressed && !shift_key_pressed && !alt_key_pressed {
                        // send the track grid zoom out message
                        track_grid_vertical_zoom_adjustment.set_value(track_grid_vertical_zoom_adjustment.value() - track_grid_vertical_zoom_adjustment.minimum_increment());
                        track_grid_zoom_adjustment.set_value(track_grid_zoom_adjustment.value() - track_grid_zoom_adjustment.minimum_increment());
                    }
                    else if key_name == "s" && !control_key_pressed && !shift_key_pressed && !alt_key_pressed {
                        // send the track grid select mode message
                        track_grid_select_mode_btn.set_active(true);
                    }
                    else if key_name == "a" && !control_key_pressed && !shift_key_pressed && !alt_key_pressed {
                        // send the track grid add mode message
                        track_grid_add_mode_btn.set_active(true);
                    }
                    else if key_name == "d" && !control_key_pressed && !shift_key_pressed && !alt_key_pressed {
                        // send the track grid delete mode message
                        track_grid_delete_mode_btn.set_active(true);
                    }
                    else if key_name == "e" && !control_key_pressed && !shift_key_pressed && !alt_key_pressed {
                        // send the track grid edit mode message
                        track_grid_edit_mode_btn.set_active(true);
                    }
                }
                
                Inhibit(false)
            });
        }

        {
            let track_grid_vertical_zoom_adjustment = self.ui.track_grid_vertical_zoom_adjustment.clone();
            let track_grid_zoom_adjustment = self.ui.track_grid_zoom_adjustment.clone();
            self.ui.track_drawing_area.connect_scroll_event(move |_, event_scroll| {
                let control_key_pressed = event_scroll.state().intersects(gdk::ModifierType::CONTROL_MASK);
                let shift_key_pressed = event_scroll.state().intersects(gdk::ModifierType::SHIFT_MASK);
                let alt_key_pressed = event_scroll.state().intersects(gdk::ModifierType::MOD1_MASK);
                let scroll_direction = event_scroll.scroll_direction();

                if let Some(scroll_direction) = scroll_direction {
                    debug!("Track grid mouse scroll: scroll direction={}, Shift key: {}, Control key: {}, Alt key: {}", scroll_direction.to_string(), shift_key_pressed, control_key_pressed, alt_key_pressed);

                    if scroll_direction == ScrollDirection::Up && control_key_pressed && shift_key_pressed && !alt_key_pressed {
                        track_grid_vertical_zoom_adjustment.set_value(track_grid_vertical_zoom_adjustment.value() + track_grid_vertical_zoom_adjustment.minimum_increment());
                        return Inhibit(true);
                    }
                    else if scroll_direction == ScrollDirection::Down && control_key_pressed && shift_key_pressed && !alt_key_pressed {
                        track_grid_vertical_zoom_adjustment.set_value(track_grid_vertical_zoom_adjustment.value() - track_grid_vertical_zoom_adjustment.minimum_increment());
                        return Inhibit(true);
                    }
                    else if scroll_direction == ScrollDirection::Up && control_key_pressed && !shift_key_pressed && alt_key_pressed {
                        track_grid_zoom_adjustment.set_value(track_grid_zoom_adjustment.value() + track_grid_zoom_adjustment.minimum_increment());
                        return Inhibit(true);
                    }
                    else if scroll_direction == ScrollDirection::Down && control_key_pressed && !shift_key_pressed && alt_key_pressed {
                        track_grid_zoom_adjustment.set_value(track_grid_zoom_adjustment.value() - track_grid_zoom_adjustment.minimum_increment());
                        return Inhibit(true);
                    }
                }
                
                Inhibit(false)
            });
        }

        {
            let grid = track_grid_ruler_arc.clone();
            self.ui.track_ruler_drawing_area.connect_draw(move |drawing_area, context|{
                match grid.lock() {
                    Ok(mut grid) => grid.paint(context, drawing_area),
                    Err(_) => (),
                }
                Inhibit(false)
            });
        }

        {
            let grid = track_grid_arc.clone();
            self.ui.track_grid_add_mode_btn.connect_clicked(move |_| {
                match grid.lock() {
                    Ok(mut grid) => grid.set_operation_mode(OperationModeType::Add),
                    Err(_) => (),
                }
            });
        }

        {
            let grid = track_grid_arc.clone();
            self.ui.track_grid_delete_mode_btn.connect_clicked(move |_| {
                match grid.lock() {
                    Ok(mut grid) => grid.set_operation_mode(OperationModeType::Delete),
                    Err(_) => (),
                }
            });
        }

        {
            let grid = track_grid_arc.clone();
            self.ui.track_grid_edit_mode_btn.connect_clicked(move |_| {
                match grid.lock() {
                    Ok(mut grid) => grid.set_operation_mode(OperationModeType::Change),
                    Err(_) => (),
                }
            });
        }

        {
            let grid = track_grid_arc.clone();
            self.ui.track_grid_select_mode_btn.connect_clicked(move |_| {
                match grid.lock() {
                    Ok(mut grid) => grid.set_operation_mode(OperationModeType::PointMode),
                    Err(_) => (),
                }
            });
        }

        {
            let grid = track_grid_arc.clone();
            self.ui.track_grid_add_loop_mode_btn.connect_clicked(move |_| {
                match grid.lock() {
                    Ok(mut grid) => grid.set_operation_mode(OperationModeType::LoopPointMode),
                    Err(_) => (),
                }
            });
        }

        {
            let track_grid = track_grid_arc.clone();
            let track_drawing_area = self.ui.track_drawing_area.clone();
            self.ui.track_grid_cut_btn.connect_clicked(move |_| {
                match track_grid.lock() {
                    Ok(mut grid) => grid.handle_cut(&track_drawing_area),
                    Err(_) => (),
                }
            });
        }

        {
            let track_grid = track_grid_arc.clone();
            let track_drawing_area = self.ui.track_drawing_area.clone();
            self.ui.track_grid_copy_btn.connect_clicked(move |_| {
                match track_grid.lock() {
                    Ok(mut grid) => grid.handle_copy(&track_drawing_area),
                    Err(_) => (),
                }
            });
        }

        {
            let track_grid = track_grid_arc.clone();
            let track_drawing_area = self.ui.track_drawing_area.clone();
            self.ui.track_grid_paste_btn.connect_clicked(move |_| {
                match track_grid.lock() {
                    Ok(mut grid) => grid.handle_paste(&track_drawing_area),
                    Err(_) => (),
                }
            });
        }

        {
            // track_grid_zoom_scale
            let track_grid = track_grid_arc.clone();
            let track_drawing_area = self.ui.track_drawing_area.clone();
            let track_grid_ruler = track_grid_ruler_arc.clone();
            let track_grid_ruler_drawing_area = self.ui.track_ruler_drawing_area.clone();
            self.ui.track_grid_horizontal_zoom_scale.connect_value_changed(move |track_grid_zoom_scale| {
                match track_grid.lock() {
                    Ok(mut grid) => grid.set_horizontal_zoom(track_grid_zoom_scale.value()),
                    Err(_) => (),
                }
                track_drawing_area.queue_draw();
                match track_grid_ruler.lock() {
                    Ok(mut grid_ruler) => grid_ruler.set_horizontal_zoom(track_grid_zoom_scale.value()),
                    Err(_) => (),
                }
                track_grid_ruler_drawing_area.queue_draw();
            });
        }

        {
            let track_grid_zoom_scale = self.ui.track_grid_horizontal_zoom_scale.clone();
            self.ui.track_grid_horizontal_zoom_out.connect_clicked(move |_| {
                let minimum_increment = track_grid_zoom_scale.adjustment().minimum_increment();
                track_grid_zoom_scale.set_value(track_grid_zoom_scale.value() - minimum_increment);
            });
        }

        {
            let track_grid_zoom_scale = self.ui.track_grid_horizontal_zoom_scale.clone();
            self.ui.track_grid_horizontal_zoom_in.connect_clicked(move |_| {
                let minimum_increment = track_grid_zoom_scale.adjustment().minimum_increment();
                track_grid_zoom_scale.set_value(track_grid_zoom_scale.value() + minimum_increment);
            });
        }

        {
            // track_grid_vertical_zoom_scale
            let track_grid = track_grid_arc.clone();
            let _track_drawing_area = self.ui.track_drawing_area.clone();
            let tx_from_ui = tx_from_ui.clone();
            self.ui.track_grid_vertical_zoom_scale.connect_value_changed(move |track_grid_vertical_zoom_scale| {
                let scale = track_grid_vertical_zoom_scale.value();
                match track_grid.lock() {
                    Ok(mut grid) => {
                        grid.set_vertical_zoom(scale);
                        let _ = tx_from_ui.send(DAWEvents::TrackGridVerticalScaleChanged(scale));
                    }
                    Err(_) => (),
                }
            });
        }

        {
            let track_grid_vertical_zoom_scale = self.ui.track_grid_vertical_zoom_scale.clone();
            self.ui.track_grid_vertical_zoom_out.connect_clicked(move |_| {
                let minimum_increment = track_grid_vertical_zoom_scale.adjustment().minimum_increment();
                track_grid_vertical_zoom_scale.set_value(track_grid_vertical_zoom_scale.value() - minimum_increment);
            });
        }

        {
            let track_grid_vertical_zoom_scale = self.ui.track_grid_vertical_zoom_scale.clone();
            self.ui.track_grid_vertical_zoom_in.connect_clicked(move |_| {
                let minimum_increment = track_grid_vertical_zoom_scale.adjustment().minimum_increment();
                track_grid_vertical_zoom_scale.set_value(track_grid_vertical_zoom_scale.value() + minimum_increment);
            });
        }

        {
            let track_grid = track_grid_arc.clone();
            let track_drawing_area = self.ui.track_drawing_area.clone();
            self.ui.track_grid_translate_left_btn.connect_clicked(move |_| {
                match track_grid.lock() {
                    Ok(mut grid) => grid.handle_translate_left(&track_drawing_area),
                    Err(_) => (),
                }
            });
        }

        {
            let track_grid = track_grid_arc.clone();
            let track_drawing_area = self.ui.track_drawing_area.clone();
            self.ui.track_grid_translate_right_btn.connect_clicked(move |_| {
                match track_grid.lock() {
                    Ok(mut grid) => grid.handle_translate_right(&track_drawing_area),
                    Err(_) => (),
                }
            });
        }

        {
            let track_grid = track_grid_arc.clone();
            let track_drawing_area = self.ui.track_drawing_area.clone();
            self.ui.track_grid_translate_up_btn.connect_clicked(move |_| {
                match track_grid.lock() {
                    Ok(mut grid) => grid.handle_translate_up(&track_drawing_area),
                    Err(_) => (),
                }
            });
        }

        {
            let track_grid = track_grid_arc.clone();
            let track_drawing_area = self.ui.track_drawing_area.clone();
            self.ui.track_grid_translate_down_btn.connect_clicked(move |_| {
                match track_grid.lock() {
                    Ok(mut grid) => grid.handle_translate_down(&track_drawing_area),
                    Err(_) => (),
                }
            });
        }

        {
            let track_grid = track_grid_arc.clone();
            let track_grid_quantise_start_choice = self.ui.track_grid_quantise_start_choice.clone();
            self.ui.track_grid_quantise_start_choice.connect_changed(move |_| {
                match track_grid_quantise_start_choice.active_text() {
                    Some(quantise_start_to_text) => {
                        let snap_position_in_beats = DAWUtils::get_snap_quantise_value_in_beats_from_choice_text(quantise_start_to_text.as_str(), 4.0);
                        match track_grid.try_lock() {
                            Ok(mut track_grid) => track_grid.set_snap_position_in_beats(snap_position_in_beats),
                            Err(_) => debug!("Unable to lock the track grid in order to set the snap in beats."),
                        };
                    },
                    None => debug!("Track grid: Unable to extract a quantise start value from the ComboBox - is there an active item?"),
                };
            });
        }

        {
            let track_grid = track_grid_arc.clone();
            let track_grid_quantise_length_choice = self.ui.track_grid_quantise_length_choice.clone();
            self.ui.track_grid_quantise_length_choice.connect_changed(move |_| {
                match track_grid_quantise_length_choice.active_text() {
                    Some(quantise_length_to_text) => {
                        let snap_length_in_beats = DAWUtils::get_snap_quantise_value_in_beats_from_choice_text(quantise_length_to_text.as_str(), 4.0);
                        match track_grid.try_lock() {
                            Ok(mut track_grid) => track_grid.set_new_entity_length_in_beats(snap_length_in_beats),
                            Err(_) => debug!("Unable to lock the track grid in order to set the new entity length in beats."),
                        };
                    },
                    None => debug!("Track grid: Unable to extract a quantise length value from the ComboBox - is there an active item?"),
                };
            });
        }

        {
            // let tx_from_ui = tx_from_ui.clone();
            // let track_grid = track_grid.clone();
            self.ui.track_grid_quantise_start_btn.connect_clicked(move |_toggle_btn| {

            });
        }

        {
            // let tx_from_ui = tx_from_ui.clone();
            // let track_grid = track_grid.clone();
            self.ui.track_grid_quantise_end_btn.connect_clicked(move |_toggle_btn| {

            });
        }

        {
            let track_grid = track_grid_arc.clone();
            let track_grid_drawing_area = self.ui.track_drawing_area.clone();
            self.ui.track_grid_show_automation_btn.connect_clicked(move |toggle_btn| {
                if let Ok(mut beat_grid) = track_grid.lock() {
                    if let Some(custom_painter) = beat_grid.custom_painter() {
                        if let Some(track_grid_custom_painter) = custom_painter.as_any().downcast_mut::<TrackGridCustomPainter>() {
                            track_grid_custom_painter.set_show_automation(toggle_btn.is_active());
                            track_grid_drawing_area.queue_draw();
                        }
                    }
                }
            });
        }

        {
            let track_grid = track_grid_arc.clone();
            let track_grid_drawing_area = self.ui.track_drawing_area.clone();
            self.ui.track_grid_show_note_velocities_btn.connect_clicked(move |toggle_btn| {
                if let Ok(mut beat_grid) = track_grid.lock() {
                    if let Some(custom_painter) = beat_grid.custom_painter() {
                        if let Some(track_grid_custom_painter) = custom_painter.as_any().downcast_mut::<TrackGridCustomPainter>() {
                            track_grid_custom_painter.set_show_note_velocity(toggle_btn.is_active());
                            track_grid_drawing_area.queue_draw();
                        }
                    }
                }
            });
        }

        {
            let track_grid = track_grid_arc.clone();
            let track_grid_drawing_area = self.ui.track_drawing_area.clone();
            self.ui.track_grid_show_notes_btn.connect_clicked(move |toggle_btn| {
                if let Ok(mut beat_grid) = track_grid.lock() {
                    if let Some(custom_painter) = beat_grid.custom_painter() {
                        if let Some(track_grid_custom_painter) = custom_painter.as_any().downcast_mut::<TrackGridCustomPainter>() {
                            track_grid_custom_painter.set_show_note(toggle_btn.is_active());
                            track_grid_drawing_area.queue_draw();
                        }
                    }
                }
            });
        }

        {
            let track_grid = track_grid_arc;
            let track_grid_drawing_area = self.ui.track_drawing_area.clone();
            self.ui.track_grid_show_pan_events_btn.connect_clicked(move |toggle_btn| {
                if let Ok(mut beat_grid) = track_grid.lock() {
                    if let Some(custom_painter) = beat_grid.custom_painter() {
                        if let Some(track_grid_custom_painter) = custom_painter.as_any().downcast_mut::<TrackGridCustomPainter>() {
                            track_grid_custom_painter.set_show_pan(toggle_btn.is_active());
                            track_grid_drawing_area.queue_draw();
                        }
                    }
                }
            });
        }

        {
            let state = state;
            self.ui.track_grid_cursor_follow.connect_clicked(move |toggle_btn| {
                if let Ok(mut state) = state.lock() {
                    state.set_track_grid_cursor_follow(toggle_btn.is_active());
                }
            });
        }
    }

    pub fn setup_automation_grid(
        &mut self,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        state: Arc<Mutex<DAWState>>
    ) {
        let state = state;
        let automation_custom_painter = AutomationCustomPainter::new(state);
        let automation_grid = BeatGrid::new_with_custom(
            1.0,
            1.0,
            3.0,
            50.0,
            4,
            Some(std::boxed::Box::new(automation_custom_painter)),
            Some(std::boxed::Box::new(AutomationMouseCoordHelper)),
            tx_from_ui.clone(),
            true,
            Some(DrawingAreaType::Automation),
        );
        let automation_grid_arc = Arc::new( Mutex::new(automation_grid));
        self.set_automation_grid(Some(automation_grid_arc.clone()));

        {
            let automation_grid = automation_grid_arc.clone();
            self.ui.automation_drawing_area.connect_draw(move |drawing_area, context|{
                match automation_grid.lock() {
                    Ok(mut grid) => grid.paint(context, drawing_area),
                    Err(_) => (),
                }
                Inhibit(false)
            });
        }

        let automation_grid_ruler = BeatGridRuler::new(1.0, 50.0, 4, tx_from_ui.clone());
        let automation_grid_ruler_arc = Arc::new(Mutex::new(automation_grid_ruler));
        self.set_automation_grid_ruler(Some(automation_grid_ruler_arc.clone()));

        {
            let automation_grid_ruler = automation_grid_ruler_arc.clone();
            self.ui.automation_ruler_drawing_area.connect_draw(move |drawing_area, context|{
                match automation_grid_ruler.lock() {
                    Ok(mut grid) => grid.paint(context, drawing_area),
                    Err(_) => (),
                }
                Inhibit(false)
            });
        }

        {
            let automation_grid = automation_grid_arc.clone();
            self.ui.automation_add_mode_btn.connect_clicked(move |_|{
                match automation_grid.lock() {
                    Ok(mut grid) => grid.set_operation_mode(OperationModeType::Add),
                    Err(_) => (),
                }
            });
        }

        {
            let automation_grid = automation_grid_arc.clone();
            self.ui.automation_delete_mode_btn.connect_clicked(move |_|{
                match automation_grid.lock() {
                    Ok(mut grid) => grid.set_operation_mode(OperationModeType::Delete),
                    Err(_) => (),
                }
            });
        }

        {
            let automation_grid = automation_grid_arc.clone();
            self.ui.automation_edit_mode_btn.connect_clicked(move |_|{
                match automation_grid.lock() {
                    Ok(mut grid) => grid.set_operation_mode(OperationModeType::Change),
                    Err(_) => (),
                }
            });
        }

        {
            let automation_grid = automation_grid_arc.clone();
            self.ui.automation_select_mode_btn.connect_clicked(move |_|{
                match automation_grid.lock() {
                    Ok(mut grid) => grid.set_operation_mode(OperationModeType::PointMode),
                    Err(_) => (),
                }
            });
        }

        {
            let automation_grid = automation_grid_arc.clone();
            let automation_drawing_area = self.ui.automation_drawing_area.clone();
            self.ui.automation_cut_btn.connect_clicked(move |_| {
                match automation_grid.lock() {
                    Ok(mut grid) => grid.handle_cut(&automation_drawing_area),
                    Err(_) => (),
                }
            });
        }

        {
            let automation_grid = automation_grid_arc.clone();
            let automation_drawing_area = self.ui.automation_drawing_area.clone();
            self.ui.automation_copy_btn.connect_clicked(move |_| {
                match automation_grid.lock() {
                    Ok(mut grid) => grid.handle_copy(&automation_drawing_area),
                    Err(_) => (),
                }
            });
        }

        {
            let automation_grid = automation_grid_arc.clone();
            let automation_drawing_area = self.ui.automation_drawing_area.clone();
            self.ui.automation_paste_btn.connect_clicked(move |_| {
                match automation_grid.lock() {
                    Ok(mut grid) => grid.handle_paste(&automation_drawing_area),
                    Err(_) => (),
                }
            });
        }

        {
            let automation_grid = automation_grid_arc.clone();
            let automation_drawing_area = self.ui.automation_drawing_area.clone();
            let automation_grid_ruler = automation_grid_ruler_arc;
            let automation_ruler_drawing_area = self.ui.automation_ruler_drawing_area.clone();
            self.ui.automation_zoom_scale.connect_value_changed(move |automation_zoom_scale| {
                match automation_grid.lock() {
                    Ok(mut grid) => grid.set_horizontal_zoom(automation_zoom_scale.value()),
                    Err(_) => (),
                }
                automation_drawing_area.queue_draw();
                match automation_grid_ruler.lock() {
                    Ok(mut grid_ruler) => grid_ruler.set_horizontal_zoom(automation_zoom_scale.value()),
                    Err(_) => (),
                }
                automation_ruler_drawing_area.queue_draw();
            });
        }

        {
            let automation_zoom_scale = self.ui.automation_zoom_scale.clone();
            self.ui.automation_zoom_out.connect_clicked(move |_| {
                let minimum_increment = automation_zoom_scale.adjustment().minimum_increment();
                automation_zoom_scale.set_value(automation_zoom_scale.value() - minimum_increment);
            });
        }

        {
            let automation_zoom_scale = self.ui.automation_zoom_scale.clone();
            self.ui.automation_zoom_in.connect_clicked(move |_| {
                let minimum_increment = automation_zoom_scale.adjustment().minimum_increment();
                automation_zoom_scale.set_value(automation_zoom_scale.value() + minimum_increment);
            });
        }

        {
            let automation_grid = automation_grid_arc.clone();
            self.ui.automation_drawing_area.connect_motion_notify_event(move |automation_drawing_area, motion_event| {
                let coords = motion_event.coords().unwrap();
                let control_key_pressed = motion_event.state().intersects(gdk::ModifierType::CONTROL_MASK);
                let shift_key_pressed = motion_event.state().intersects(gdk::ModifierType::SHIFT_MASK);
                let alt_key_pressed = motion_event.state().intersects(gdk::ModifierType::MOD1_MASK);
                let mouse_button = if motion_event.state().intersects(gdk::ModifierType::BUTTON1_MASK) {
                    MouseButton::Button1
                }
                else if motion_event.state().intersects(gdk::ModifierType::BUTTON2_MASK) {
                    MouseButton::Button2
                }
                else {
                    MouseButton::Button3
                };
                // debug!("Controller mouse motion: x={}, y={}, Shift key: {}, Control key: {}", coords.0, coords.1, shift_key_pressed, control_key_pressed);
                match automation_grid.lock() {
                    Ok(mut grid) => {
                        grid.handle_mouse_motion(coords.0, coords.1, automation_drawing_area, mouse_button, control_key_pressed, shift_key_pressed, alt_key_pressed);
                    },
                    Err(_) => (),
                }
                Inhibit(false)
            });
        }

        {
            let automation_grid = automation_grid_arc.clone();
            self.ui.automation_drawing_area.connect_button_press_event(move |automation_drawing_area, event_btn| {
                let coords = event_btn.coords().unwrap();
                let control_key_pressed = event_btn.state().intersects(gdk::ModifierType::CONTROL_MASK);
                let shift_key_pressed = event_btn.state().intersects(gdk::ModifierType::SHIFT_MASK);
                let alt_key_pressed = event_btn.state().intersects(gdk::ModifierType::MOD1_MASK);
                let mouse_button = if event_btn.state().intersects(gdk::ModifierType::BUTTON1_MASK) {
                    MouseButton::Button3
                }
                else if event_btn.state().intersects(gdk::ModifierType::BUTTON2_MASK) {
                    MouseButton::Button2
                }
                else {
                    MouseButton::Button1
                };
                debug!("Controller mouse pressed coords: x={}, y={}, Shift key: {}, Control key: {}", coords.0, coords.1, shift_key_pressed, control_key_pressed);
                // let event_state = event_btn.state();
                // debug!("Event modifier: {:?}", event_state);
                match automation_grid.lock() {
                    Ok(mut grid) => {
                        grid.handle_mouse_press(coords.0, coords.1, automation_drawing_area, mouse_button, control_key_pressed, shift_key_pressed, alt_key_pressed);
                    },
                    Err(_) => (),
                }
                Inhibit(false)
            });
        }

        {
            let automation_grid = automation_grid_arc.clone();
            self.ui.automation_drawing_area.connect_button_release_event(move |automation_drawing_area, event_btn| {
                let coords = event_btn.coords().unwrap();
                let control_key_pressed = event_btn.state().intersects(gdk::ModifierType::CONTROL_MASK);
                let shift_key_pressed = event_btn.state().intersects(gdk::ModifierType::SHIFT_MASK);
                let alt_key_pressed = event_btn.state().intersects(gdk::ModifierType::MOD1_MASK);
                let mouse_button = if event_btn.state().intersects(gdk::ModifierType::BUTTON1_MASK) {
                    MouseButton::Button1
                }
                else if event_btn.state().intersects(gdk::ModifierType::BUTTON2_MASK) {
                    MouseButton::Button2
                }
                else {
                    MouseButton::Button3
                };
                debug!("Controller mouse released: x={}, y={}, Shift key: {}, Control key: {}", coords.0, coords.1, shift_key_pressed, control_key_pressed);
                match automation_grid.lock() {
                    Ok(mut grid) => grid.handle_mouse_release(coords.0, coords.1, automation_drawing_area, mouse_button, control_key_pressed, shift_key_pressed, alt_key_pressed, String::from("")),
                    Err(_) => (),
                }
                Inhibit(false)
            });
        }

        {
            let automation_grid = automation_grid_arc.clone();
            let automation_drawing_area = self.ui.automation_drawing_area.clone();
            self.ui.automation_translate_left_btn.connect_clicked(move |_| {
                match automation_grid.lock() {
                    Ok(mut grid) => grid.handle_translate_left(&automation_drawing_area),
                    Err(_) => (),
                }
            });
        }

        {
            let automation_grid = automation_grid_arc.clone();
            let automation_drawing_area = self.ui.automation_drawing_area.clone();
            self.ui.automation_translate_right_btn.connect_clicked(move |_| {
                match automation_grid.lock() {
                    Ok(mut grid) => grid.handle_translate_right(&automation_drawing_area),
                    Err(_) => (),
                }
            });
        }

        {
            let automation_grid = automation_grid_arc.clone();
            let automation_drawing_area = self.ui.automation_drawing_area.clone();
            self.ui.automation_translate_up_btn.connect_clicked(move |_| {
                match automation_grid.lock() {
                    Ok(mut grid) => grid.handle_translate_up(&automation_drawing_area),
                    Err(_) => (),
                }
            });
        }

        {
            let automation_grid = automation_grid_arc.clone();
            let automation_drawing_area = self.ui.automation_drawing_area.clone();
            self.ui.automation_translate_down_btn.connect_clicked(move |_| {
                match automation_grid.lock() {
                    Ok(mut grid) => grid.handle_translate_down(&automation_drawing_area),
                    Err(_) => (),
                }
            });
        }

        {
            let automation_grid = automation_grid_arc.clone();
            let automation_drawing_area = self.ui.automation_drawing_area.clone();
            self.ui.automation_quantise_btn.connect_clicked(move |_| {
                match automation_grid.lock() {
                    Ok(mut grid) => grid.handle_quantise(&automation_drawing_area),
                    Err(_) => (),
                }
            });
        }

        {
            let automation_grid = automation_grid_arc.clone();
            self.ui.automation_quantise_start_choice.connect_changed(move |automation_quantise_start_choice| {
                match automation_quantise_start_choice.active_text() {
                    Some(quantise_start_to_text) => {
                        let snap_position_in_beats = DAWUtils::get_snap_quantise_value_in_beats_from_choice_text(quantise_start_to_text.as_str(), 4.0);
                        match automation_grid.try_lock() {
                            Ok(mut grid) => grid.set_snap_position_in_beats(snap_position_in_beats),
                            Err(_) => debug!("Unable to lock the controller grid in order to set the snap in beats."),
                        };
                    },
                    None => debug!("Unable to extract a quantise start value from the ComboBox - is there an active item?"),
                };
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let automation_edit_panel_stack = self.ui.automation_edit_panel_stack.clone();
            let automation_grid_edit_note_velocity_box = self.ui.automation_grid_edit_note_velocity_box.clone();
            self.ui.automation_grid_edit_note_velocity.connect_clicked(move |_|{
                automation_edit_panel_stack.set_visible_child(&automation_grid_edit_note_velocity_box);
                automation_edit_panel_stack.set_visible(false);

                match tx_from_ui.send(DAWEvents::AutomationViewShowTypeChange(ShowType::Velocity)) {
                    Ok(_) => (),
                    Err(error) => debug!("Error: {}", error),
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let automation_edit_panel_stack = self.ui.automation_edit_panel_stack.clone();
            let automation_grid_edit_note_expression_box = self.ui.automation_grid_edit_note_expression_box.clone();
            self.ui.automation_grid_edit_note_expression.connect_clicked(move |_|{
                automation_edit_panel_stack.set_visible_child(&automation_grid_edit_note_expression_box);
                automation_edit_panel_stack.set_visible(true);

                match tx_from_ui.send(DAWEvents::AutomationViewShowTypeChange(ShowType::NoteExpression)) {
                    Ok(_) => (),
                    Err(error) => debug!("Error: {}", error),
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let automation_edit_panel_stack = self.ui.automation_edit_panel_stack.clone();
            let automation_grid_edit_controllers_box = self.ui.automation_grid_edit_controllers_box.clone();
            self.ui.automation_grid_edit_controllers.connect_clicked(move |_|{
                automation_edit_panel_stack.set_visible_child(&automation_grid_edit_controllers_box);
                automation_edit_panel_stack.set_visible(true);

                match tx_from_ui.send(DAWEvents::AutomationViewShowTypeChange(ShowType::Controller)) {
                    Ok(_) => (),
                    Err(error) => debug!("Error: {}", error),
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let automation_edit_panel_stack = self.ui.automation_edit_panel_stack.clone();
            let automation_grid_edit_instrument_parameters_box = self.ui.automation_grid_edit_instrument_parameters_box.clone();
            self.ui.automation_grid_edit_instrument_parameters.connect_clicked(move |_|{
                automation_edit_panel_stack.set_visible_child(&automation_grid_edit_instrument_parameters_box);
                automation_edit_panel_stack.set_visible(true);

                match tx_from_ui.send(DAWEvents::AutomationViewShowTypeChange(ShowType::InstrumentParameter)) {
                    Ok(_) => (),
                    Err(error) => debug!("Error: {}", error),
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let automation_edit_panel_stack = self.ui.automation_edit_panel_stack.clone();
            let automation_grid_edit_effect_parameters_box = self.ui.automation_grid_edit_effect_parameters_box.clone();
            self.ui.automation_grid_edit_effect_parameters.connect_clicked(move |_|{
                automation_edit_panel_stack.set_visible_child(&automation_grid_edit_effect_parameters_box);
                automation_edit_panel_stack.set_visible(true);

                match tx_from_ui.send(DAWEvents::AutomationViewShowTypeChange(ShowType::EffectParameter)) {
                    Ok(_) => (),
                    Err(error) => debug!("Error: {}", error),
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            self.ui.automation_grid_edit_track.connect_clicked(move |_|{
                match tx_from_ui.send(DAWEvents::AutomationEditTypeChange(AutomationEditType::Track)) {
                    Ok(_) => (),
                    Err(error) => debug!("Error: {}", error),
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            self.ui.automation_grid_edit_riff.connect_clicked(move |_|{
                match tx_from_ui.send(DAWEvents::AutomationEditTypeChange(AutomationEditType::Riff)) {
                    Ok(_) => (),
                    Err(error) => debug!("Error: {}", error),
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            self.ui.automation_controller_combobox.connect_changed(move |automation_controller_combobox|{
                match automation_controller_combobox.active_id() {
                    Some(controller_type_text) => {
                        let controller_type: i32 = controller_type_text.as_str().parse().unwrap();
                        match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::AutomationTypeChange(AutomationChangeData::ParameterType(controller_type)), None)) {
                            Ok(_) => (),
                            Err(_) => (),
                        }
                    },
                    None => debug!("Unable to extract a controller type value from the ComboBox - is there an active item?"),
                };
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            self.ui.automation_instrument_parameters_combobox.connect_changed(move |automation_instrument_parameters_combobox|{
                match automation_instrument_parameters_combobox.active_id() {
                    Some(instrument_parameter_index_text) => {
                        let instrument_parameter_index: i32 = instrument_parameter_index_text.as_str().parse().unwrap();
                        match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::AutomationTypeChange(AutomationChangeData::ParameterType(instrument_parameter_index)), None)) {
                            Ok(_) => (),
                            Err(_) => (),
                        }
                    },
                    None => ()/* debug!("Unable to extract an instrument parameter index value from the ComboBox - is there an active item?") */,
                };
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            self.ui.automation_effect_parameters_combobox.connect_changed(move |automation_effect_parameters_combobox|{
                match automation_effect_parameters_combobox.active_id() {
                    Some(effect_parameter_index_text) => {
                        let effect_parameter_index: i32 = effect_parameter_index_text.as_str().parse().unwrap();
                        match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::AutomationTypeChange(AutomationChangeData::ParameterType(effect_parameter_index)), None)) {
                            Ok(_) => (),
                            Err(_) => (),
                        }
                    },
                    None => debug!("Unable to extract an effect parameter index value from the ComboBox - is there an active item?"),
                };
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let sig_handler_id = self.ui.automation_effects_combobox.connect_changed(move |automation_effects_combobox|{
                match automation_effects_combobox.active_id() {
                    Some(effect_uuid_text) => {
                        let effect_uuid = effect_uuid_text.as_str().to_string();
                        match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::EffectSelected(effect_uuid), None)) {
                            Ok(_) => (),
                            Err(_) => (),
                        }
                    },
                    None => debug!("Unable to extract an effect uuid value from the ComboBox - is there an active item?"),
                };
            });
            self.automation_effects_choice_signal_handler_id = Some(sig_handler_id);
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            self.ui.automation_note_expression_type.connect_changed(move |automation_note_expression_type_combobox|{
                match automation_note_expression_type_combobox.active_id() {
                    Some(note_expression_type_text) => {
                        let note_expression_type = NoteExpressionType::from_str(note_expression_type_text.as_str()).unwrap();
                        match tx_from_ui.send(DAWEvents::TrackChange(
                            TrackChangeType::AutomationTypeChange(AutomationChangeData::NoteExpression(NoteExpressionData::Type(note_expression_type))), None)) {
                            Ok(_) => (),
                            Err(_) => (),
                        }
                    },
                    None => debug!("Unable to extract a note expression type from the ComboBox - is there an active item?"),
                };
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            self.ui.automation_note_expression_id.connect_changed(move |automation_note_expression_id_combobox|{
                match automation_note_expression_id_combobox.active_id() {
                    Some(note_expression_id_text) => {
                        let note_expression_id: i32 = note_expression_id_text.as_str().parse().unwrap();
                        match tx_from_ui.send(DAWEvents::TrackChange(
                            TrackChangeType::AutomationTypeChange(AutomationChangeData::NoteExpression(NoteExpressionData::NoteId(note_expression_id))), None)) {
                            Ok(_) => (),
                            Err(_) => (),
                        }
                    },
                    None => debug!("Unable to extract a note expression id from the ComboBox - is there an active item?"),
                };
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            self.ui.automation_note_expression_port_index.connect_changed(move |automation_note_expression_port_index_combobox|{
                match automation_note_expression_port_index_combobox.active_id() {
                    Some(automation_note_expression_port_index_text) => {
                        let automation_note_expression_port_index: i32 = automation_note_expression_port_index_text.as_str().parse().unwrap();
                        match tx_from_ui.send(DAWEvents::TrackChange(
                            TrackChangeType::AutomationTypeChange(AutomationChangeData::NoteExpression(NoteExpressionData::PortIndex(automation_note_expression_port_index))), None)) {
                            Ok(_) => (),
                            Err(_) => (),
                        }
                    },
                    None => debug!("Unable to extract a note expression port index from the ComboBox - is there an active item?"),
                };
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            self.ui.automation_note_expression_channel.connect_changed(move |automation_note_expression_channel_combobox|{
                match automation_note_expression_channel_combobox.active_id() {
                    Some(automation_note_expression_channel_text) => {
                        let automation_note_expression_channel: i32 = automation_note_expression_channel_text.as_str().parse().unwrap();
                        match tx_from_ui.send(DAWEvents::TrackChange(
                            TrackChangeType::AutomationTypeChange(AutomationChangeData::NoteExpression(NoteExpressionData::Channel(automation_note_expression_channel))), None)) {
                            Ok(_) => (),
                            Err(_) => (),
                        }
                    },
                    None => debug!("Unable to extract a note expression channel from the ComboBox - is there an active item?"),
                };
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            self.ui.automation_note_expression_key.connect_changed(move |automation_note_expression_key_combobox|{
                match automation_note_expression_key_combobox.active_id() {
                    Some(automation_note_expression_key_text) => {
                        let automation_note_expression_key: i32 = automation_note_expression_key_text.as_str().parse().unwrap();
                        match tx_from_ui.send(DAWEvents::TrackChange(
                            TrackChangeType::AutomationTypeChange(AutomationChangeData::NoteExpression(NoteExpressionData::Key(automation_note_expression_key))), None)) {
                            Ok(_) => (),
                            Err(_) => (),
                        }
                    },
                    None => debug!("Unable to extract a note expression key from the ComboBox - is there an active item?"),
                };
            });
        }

        {
            let automation_grid = automation_grid_arc.clone();
            self.ui.automation_grid_mode_point.connect_clicked(move |_|{
                if let Ok(mut grid) = automation_grid.lock() {
                    grid.turn_on_draw_point_mode();
                }
            });
        }

        {
            let automation_grid = automation_grid_arc.clone();
            self.ui.automation_grid_mode_line.connect_clicked(move |_|{
                if let Ok(mut grid) = automation_grid.lock() {
                    grid.turn_on_draw_line_mode();
                }
            });
        }

        {
            let automation_grid = automation_grid_arc;
            self.ui.automation_grid_mode_curve.connect_clicked(move |_|{
                if let Ok(mut grid) = automation_grid.lock() {
                    grid.turn_on_draw_curve_mode();
                }
            });
        }

        {
            let automation_component = self.ui.automation_component.clone();
            let automation_window = self.automation_window.clone();
            let automation_window_stack = self.automation_window_stack.clone();
            let sub_panel_stack = self.ui.sub_panel_stack.clone();
            self.ui.automation_dock_toggle_btn.connect_clicked(move |toggle_button| {
                if toggle_button.is_active() {
                    sub_panel_stack.remove(&automation_component);
                    automation_window_stack.add_titled(&automation_component, "automation", "Automation");
                    automation_window.show_all();
                }
                else {
                    automation_window_stack.remove(&automation_component);
                    sub_panel_stack.add_titled(&automation_component, "automation", "Automation");
                    automation_window.hide();
                }
            });
        }
    }

    pub fn setup_mixer(
        &mut self,
    ) {
        {
            let mixer_component = self.ui.mixer_component.clone();
            let mixer_window = self.mixer_window.clone();
            let mixer_window_stack = self.mixer_window_stack.clone();
            let sub_panel_stack = self.ui.sub_panel_stack.clone();
            self.ui.mixer_dock_toggle_btn.connect_clicked(move |toggle_button| {
                if toggle_button.is_active() {
                    sub_panel_stack.remove(&mixer_component);
                    mixer_window_stack.add_titled(&mixer_component, "mixer", "Mixer");
                    mixer_window.show_all();
                }
                else {
                    mixer_window_stack.remove(&mixer_component);
                    sub_panel_stack.add_titled(&mixer_component, "mixer", "Mixer");
                    mixer_window.hide();
                }
            });
        }
    }

    pub fn setup_piano(
        &mut self,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>
    ) -> Arc<Mutex<Piano>> {
        let tx_from_ui = tx_from_ui;
        let piano = Piano::new(1.0, self.ui.piano_roll_piano_keyboard_drawing_area.height_request() as f64 / 127.0, tx_from_ui);
        let piano_ref = Arc::new( Mutex::new(piano));
        {
            let piano_ref = piano_ref.clone();
            self.ui.piano_roll_piano_keyboard_drawing_area.connect_draw(move |drawing_area, context| {
                match piano_ref.lock() {
                    Ok(piano_ref) => piano_ref.paint(context, drawing_area),
                    Err(error) => debug!("Could not lock piano for drawing: {}", error),
                }
                Inhibit(false)
            });
        }

        {
            let piano_ref = piano_ref.clone();
            self.ui.piano_roll_piano_keyboard_drawing_area.connect_button_press_event(move |drawing_area, event_btn| {
                match piano_ref.lock() {
                    Ok(mut piano_ref) => {
                        let coords = event_btn.coords().unwrap();
                        let control_key_pressed = event_btn.state().intersects(gdk::ModifierType::CONTROL_MASK);
                        let shift_key_pressed = event_btn.state().intersects(gdk::ModifierType::SHIFT_MASK);
                        let alt_key_pressed = event_btn.state().intersects(gdk::ModifierType::MOD1_MASK);
                        let mouse_button = if event_btn.state().intersects(gdk::ModifierType::BUTTON1_MASK) {
                            MouseButton::Button1
                        }
                        else if event_btn.state().intersects(gdk::ModifierType::BUTTON2_MASK) {
                            MouseButton::Button2
                        }
                        else {
                            MouseButton::Button3
                        };
                        // debug!("Piano keyboard mouse press: x={}, y={}, Shift key: {}, Control key: {}", coords.0, coords.1, shift_key_pressed, control_key_pressed);
                        piano_ref.handle_mouse_press(coords.0, coords.1, drawing_area, mouse_button, control_key_pressed, shift_key_pressed, alt_key_pressed);
                    },
                    Err(error) => debug!("Could not lock piano keyboard for drawing: {}", error),
                }
                Inhibit(false)
            });
        }

        {
            let piano_ref = piano_ref.clone();
            self.ui.piano_roll_piano_keyboard_drawing_area.connect_button_release_event(move |drawing_area, event_btn| {
                match piano_ref.lock() {
                    Ok(mut piano_ref) => {
                        let coords = event_btn.coords().unwrap();
                        let control_key_pressed = event_btn.state().intersects(gdk::ModifierType::CONTROL_MASK);
                        let shift_key_pressed = event_btn.state().intersects(gdk::ModifierType::SHIFT_MASK);
                        let alt_key_pressed = event_btn.state().intersects(gdk::ModifierType::MOD1_MASK);
                        let mouse_button = if event_btn.state().intersects(gdk::ModifierType::BUTTON1_MASK) {
                            MouseButton::Button1
                        }
                        else if event_btn.state().intersects(gdk::ModifierType::BUTTON2_MASK) {
                            MouseButton::Button2
                        }
                        else {
                            MouseButton::Button3
                        };
                        // debug!("Piano keyboard mouse release: x={}, y={}, Shift key: {}, Control key: {}", coords.0, coords.1, shift_key_pressed, control_key_pressed);
                        piano_ref.handle_mouse_release(coords.0, coords.1, drawing_area, mouse_button, control_key_pressed, shift_key_pressed, alt_key_pressed, String::from(""));
                    },
                    Err(error) => debug!("Could not lock piano keyboard for drawing: {}", error),
                }
                Inhibit(false)
            });
        }

        piano_ref
    }

    pub fn setup_piano_roll(
        &mut self,
        piano: Arc<Mutex<Piano>>,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        state: Arc<Mutex<DAWState>>
    ) {
        {
            let state = state;
            let event_sender = std::boxed::Box::new(|original_note: Note, changed_note: Note, track_uuid: String, tx_from_ui: Sender<DAWEvents>| {
                    let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RiffEventChange(TrackEvent::Note(original_note), TrackEvent::Note(changed_note)), Some(track_uuid)));
            });
            let piano_roll_custom_painter: PianoRollCustomPainter = PianoRollCustomPainter::new_with_edit_item_handler(state.clone(), EditItemHandler::new(event_sender));
            let piano_roll_custom_vertical_scale_painter = PianoRollVerticalScaleCustomPainter::new(state);
            let piano_roll_grid = BeatGrid::new_with_painters(
                2.0,
                1.0,
                self.ui.piano_roll_drawing_area.height_request() as f64 / 127.0,
                50.0,
                4,
                Some(std::boxed::Box::new(piano_roll_custom_painter)),
                Some(std::boxed::Box::new(piano_roll_custom_vertical_scale_painter)),
                Some(std::boxed::Box::new(PianoRollMouseCoordHelper)),
                tx_from_ui.clone(),
                true,
                Some(DrawingAreaType::PianoRoll),
            );
            let piano_roll_grid_arc = Arc::new( Mutex::new(piano_roll_grid));

            self.set_piano_roll_grid(Some(piano_roll_grid_arc.clone()));

            let piano_roll_grid_ruler = BeatGridRuler::new(2.0, 50.0, 4, tx_from_ui);
            let piano_roll_grid_ruler_arc = Arc::new(Mutex::new(piano_roll_grid_ruler));

            self.set_piano_roll_grid_ruler(Some(piano_roll_grid_ruler_arc.clone()));

            {
                let piano_roll_grid = piano_roll_grid_arc.clone();
                self.ui.piano_roll_drawing_area.connect_draw(move |drawing_area, context| {
                    match piano_roll_grid.lock() {
                        Ok(mut grid) => grid.paint(context, drawing_area),
                        Err(_) => (),
                    }
                    Inhibit(false)
                });
            }

            {
                let piano_roll_grid = piano_roll_grid_arc.clone();
                self.ui.piano_roll_drawing_area.connect_motion_notify_event(move |piano_roll_drawing_area, motion_event| {
                    let coords = motion_event.coords().unwrap();
                    let control_key_pressed = motion_event.state().intersects(gdk::ModifierType::CONTROL_MASK);
                    let shift_key_pressed = motion_event.state().intersects(gdk::ModifierType::SHIFT_MASK);
                    let alt_key_pressed = motion_event.state().intersects(gdk::ModifierType::MOD1_MASK);
                    let mouse_button = if motion_event.state().intersects(gdk::ModifierType::BUTTON1_MASK) {
                        MouseButton::Button1
                    }
                    else if motion_event.state().intersects(gdk::ModifierType::BUTTON2_MASK) {
                        MouseButton::Button2
                    }
                    else {
                        MouseButton::Button3
                    };
                    // debug!("Piano roll mouse motion: x={}, y={}, Shift key: {}, Control key: {}", coords.0, coords.1, shift_key_pressed, control_key_pressed);
                    match piano_roll_grid.lock() {
                        Ok(mut grid) => {
                            grid.handle_mouse_motion(coords.0, coords.1, piano_roll_drawing_area, mouse_button, control_key_pressed, shift_key_pressed, alt_key_pressed);
                        },
                        Err(_) => (),
                    }

                    piano_roll_drawing_area.queue_draw();

                    Inhibit(false)
                });
            }

            {
                let piano_roll_grid = piano_roll_grid_arc.clone();
                self.ui.piano_roll_drawing_area.connect_button_press_event(move |piano_roll_drawing_area, event_btn| {
                    let coords = event_btn.coords().unwrap();
                    let control_key_pressed = event_btn.state().intersects(gdk::ModifierType::CONTROL_MASK);
                    let shift_key_pressed = event_btn.state().intersects(gdk::ModifierType::SHIFT_MASK);
                    let alt_key_pressed = event_btn.state().intersects(gdk::ModifierType::MOD1_MASK);
                    let mouse_button = if event_btn.state().intersects(gdk::ModifierType::BUTTON1_MASK) {
                        MouseButton::Button3
                    }
                    else if event_btn.state().intersects(gdk::ModifierType::BUTTON2_MASK) {
                        MouseButton::Button2
                    }
                    else {
                        MouseButton::Button1
                    };
                    debug!("Piano roll mouse pressed coords: x={}, y={}, Shift key: {}, Control key: {}", coords.0, coords.1, shift_key_pressed, control_key_pressed);
                    // let event_state = event_btn.state();
                    // debug!("Event modifier: {:?}", event_state);
                    match piano_roll_grid.lock() {
                        Ok(mut grid) => {
                            grid.handle_mouse_press(coords.0, coords.1, piano_roll_drawing_area, mouse_button, control_key_pressed, shift_key_pressed, alt_key_pressed);
                        },
                        Err(_) => (),
                    }
                    Inhibit(false)
                });
            }

            {
                let piano_roll_grid = piano_roll_grid_arc.clone();
                self.ui.piano_roll_drawing_area.connect_button_release_event(move |piano_roll_drawing_area, event_btn| {
                    let coords = event_btn.coords().unwrap();
                    let control_key_pressed = event_btn.state().intersects(gdk::ModifierType::CONTROL_MASK);
                    let shift_key_pressed = event_btn.state().intersects(gdk::ModifierType::SHIFT_MASK);
                    let alt_key_pressed = event_btn.state().intersects(gdk::ModifierType::MOD1_MASK);
                    let mouse_button = if event_btn.state().intersects(gdk::ModifierType::BUTTON1_MASK) {
                        MouseButton::Button1
                    }
                    else if event_btn.state().intersects(gdk::ModifierType::BUTTON2_MASK) {
                        MouseButton::Button2
                    }
                    else {
                        MouseButton::Button3
                    };

                    piano_roll_drawing_area.grab_focus();

                    debug!("Piano roll mouse released: x={}, y={}, Shift key: {}, Control key: {}", coords.0, coords.1, shift_key_pressed, control_key_pressed);
                    match piano_roll_grid.lock() {
                        Ok(mut grid) => grid.handle_mouse_release(coords.0, coords.1, piano_roll_drawing_area, mouse_button, control_key_pressed, shift_key_pressed, alt_key_pressed, String::from("")),
                        Err(_) => (),
                    }
                    Inhibit(false)
                });
            }

            {
                let piano_roll_grid = piano_roll_grid_arc.clone();
                let piano_roll_vertical_zoom_adjustment = self.ui.piano_roll_vertical_zoom_adjustment.clone();
                let piano_roll_zoom_adjustment = self.ui.piano_roll_zoom_adjustment.clone();
                let piano_roll_select_mode_btn = self.ui.piano_roll_select_mode_btn.clone();
                let piano_roll_add_mode_btn = self.ui.piano_roll_add_mode_btn.clone();
                let piano_roll_delete_mode_btn = self.ui.piano_roll_delete_mode_btn.clone();
                let piano_roll_edit_mode_btn = self.ui.piano_roll_edit_mode_btn.clone();
                self.ui.piano_roll_drawing_area.connect_key_press_event(move |piano_roll_drawing_area, event_key| {
                    let control_key_pressed = event_key.state().intersects(gdk::ModifierType::CONTROL_MASK);
                    let shift_key_pressed = event_key.state().intersects(gdk::ModifierType::SHIFT_MASK);
                    let alt_key_pressed = event_key.state().intersects(gdk::ModifierType::MOD1_MASK);
                    let key_pressed_value = event_key.keyval().name();

                    if let Some(key_name) = key_pressed_value {
                        debug!("Piano roll key press: Key name={}, Shift key: {}, Control key: {}, Alt key: {}", key_name.as_str(), shift_key_pressed, control_key_pressed, alt_key_pressed);

                        if key_name == "c" && control_key_pressed && !shift_key_pressed && !alt_key_pressed {
                            // send the piano roll copy message
                            match piano_roll_grid.lock() {
                                Ok(mut grid) => {
                                    grid.handle_copy(piano_roll_drawing_area);
                                }
                                Err(_) => {}
                            }
                        }
                        else if key_name == "x" && control_key_pressed && !shift_key_pressed && !alt_key_pressed {
                            // send the piano roll cut message
                            match piano_roll_grid.lock() {
                                Ok(mut grid) => {
                                    grid.handle_cut(piano_roll_drawing_area);
                                }
                                Err(_) => {}
                            }
                        }
                        else if key_name == "v" && control_key_pressed && !shift_key_pressed && !alt_key_pressed {
                            // send the piano roll paste message
                            match piano_roll_grid.lock() {
                                Ok(mut grid) => {
                                    grid.handle_paste(piano_roll_drawing_area);
                                }
                                Err(_) => {}
                            }
                        }
                        else if key_name == "a" && control_key_pressed && !shift_key_pressed && !alt_key_pressed {
                            // send the piano roll select all message
                            match piano_roll_grid.lock() {
                                Ok(_grid) => {
                                    debug!("Not implemented yet!");
                                }
                                Err(_) => {}
                            }
                        }
                        else if key_name == "plus" && !control_key_pressed && shift_key_pressed && !alt_key_pressed {
                            piano_roll_vertical_zoom_adjustment.set_value(piano_roll_vertical_zoom_adjustment.value() + piano_roll_vertical_zoom_adjustment.minimum_increment());
                            piano_roll_zoom_adjustment.set_value(piano_roll_zoom_adjustment.value() + piano_roll_zoom_adjustment.minimum_increment());
                        }
                        else if key_name == "minus" && !control_key_pressed && !shift_key_pressed && !alt_key_pressed {
                            // send the piano roll zoom out message
                            piano_roll_vertical_zoom_adjustment.set_value(piano_roll_vertical_zoom_adjustment.value() - piano_roll_vertical_zoom_adjustment.minimum_increment());
                            piano_roll_zoom_adjustment.set_value(piano_roll_zoom_adjustment.value() - piano_roll_zoom_adjustment.minimum_increment());
                        }
                        else if key_name == "s" && !control_key_pressed && !shift_key_pressed && !alt_key_pressed {
                            // send the piano roll select mode message
                            piano_roll_select_mode_btn.set_active(true);
                        }
                        else if key_name == "a" && !control_key_pressed && !shift_key_pressed && !alt_key_pressed {
                            // send the piano roll add mode message
                            piano_roll_add_mode_btn.set_active(true);
                        }
                        else if key_name == "d" && !control_key_pressed && !shift_key_pressed && !alt_key_pressed {
                            // send the piano roll delete mode message
                            piano_roll_delete_mode_btn.set_active(true);
                        }
                        else if key_name == "e" && !control_key_pressed && !shift_key_pressed && !alt_key_pressed {
                            // send the piano roll edit mode message
                            piano_roll_edit_mode_btn.set_active(true);
                        }
                    }
                    
                    Inhibit(false)
                });
            }

            {
                let piano_roll_vertical_zoom_adjustment = self.ui.piano_roll_vertical_zoom_adjustment.clone();
                let piano_roll_zoom_adjustment = self.ui.piano_roll_zoom_adjustment.clone();
                self.ui.piano_roll_drawing_area.connect_scroll_event(move |_, event_scroll| {
                    let control_key_pressed = event_scroll.state().intersects(gdk::ModifierType::CONTROL_MASK);
                    let shift_key_pressed = event_scroll.state().intersects(gdk::ModifierType::SHIFT_MASK);
                    let alt_key_pressed = event_scroll.state().intersects(gdk::ModifierType::MOD1_MASK);
                    let scroll_direction = event_scroll.scroll_direction();

                    if let Some(scroll_direction) = scroll_direction {
                        debug!("Piano roll mouse scroll: scroll direction={}, Shift key: {}, Control key: {}, Alt key: {}", scroll_direction.to_string(), shift_key_pressed, control_key_pressed, alt_key_pressed);

                        if scroll_direction == ScrollDirection::Up && control_key_pressed && shift_key_pressed && !alt_key_pressed {
                            piano_roll_vertical_zoom_adjustment.set_value(piano_roll_vertical_zoom_adjustment.value() + piano_roll_vertical_zoom_adjustment.minimum_increment());
                            return Inhibit(true);
                        }
                        else if scroll_direction == ScrollDirection::Down && control_key_pressed && shift_key_pressed && !alt_key_pressed {
                            piano_roll_vertical_zoom_adjustment.set_value(piano_roll_vertical_zoom_adjustment.value() - piano_roll_vertical_zoom_adjustment.minimum_increment());
                            return Inhibit(true);
                        }
                        else if scroll_direction == ScrollDirection::Up && control_key_pressed && !shift_key_pressed && alt_key_pressed {
                            piano_roll_zoom_adjustment.set_value(piano_roll_zoom_adjustment.value() + piano_roll_zoom_adjustment.minimum_increment());
                            return Inhibit(true);
                        }
                        else if scroll_direction == ScrollDirection::Down && control_key_pressed && !shift_key_pressed && alt_key_pressed {
                            piano_roll_zoom_adjustment.set_value(piano_roll_zoom_adjustment.value() - piano_roll_zoom_adjustment.minimum_increment());
                            return Inhibit(true);
                        }
                    }
                    
                    Inhibit(false)
                });
            }

            {
                let piano_roll_grid = piano_roll_grid_arc.clone();
                let piano_roll_drawing_area = self.ui.piano_roll_drawing_area.clone();
                let piano_roll_grid_ruler = piano_roll_grid_ruler_arc.clone();
                let piano_roll_ruler_drawing_area = self.ui.piano_roll_ruler_drawing_area.clone();
                self.ui.piano_roll_horizontal_zoom_scale.connect_value_changed(move |piano_roll_horizontal_zoom_scale| {
                    match piano_roll_grid.lock() {
                        Ok(mut grid) => grid.set_horizontal_zoom(piano_roll_horizontal_zoom_scale.value()),
                        Err(_) => (),
                    }
                    piano_roll_drawing_area.queue_draw();
                    match piano_roll_grid_ruler.lock() {
                        Ok(mut grid_ruler) => grid_ruler.set_horizontal_zoom(piano_roll_horizontal_zoom_scale.value()),
                        Err(_) => (),
                    }
                    piano_roll_ruler_drawing_area.queue_draw();
                });
            }

            {
                let piano_roll_horizontal_zoom_scale = self.ui.piano_roll_horizontal_zoom_scale.clone();
                self.ui.piano_roll_horizontal_zoom_out.connect_clicked(move |_| {
                    let minimum_increment = piano_roll_horizontal_zoom_scale.adjustment().minimum_increment();
                    piano_roll_horizontal_zoom_scale.set_value(piano_roll_horizontal_zoom_scale.value() - minimum_increment);
                });
            }

            {
                let piano_roll_horizontal_zoom_scale = self.ui.piano_roll_horizontal_zoom_scale.clone();
                self.ui.piano_roll_horizontal_zoom_in.connect_clicked(move |_| {
                    let minimum_increment = piano_roll_horizontal_zoom_scale.adjustment().minimum_increment();
                    piano_roll_horizontal_zoom_scale.set_value(piano_roll_horizontal_zoom_scale.value() + minimum_increment);
                });
            }

            {
                let piano_roll_grid = piano_roll_grid_arc.clone();
                let piano_roll_drawing_area = self.ui.piano_roll_drawing_area.clone();
                let piano_keyboard_drawing_area = self.ui.piano_roll_piano_keyboard_drawing_area.clone();
                self.ui.piano_roll_vertical_zoom_scale.connect_value_changed(move |piano_roll_vertical_zoom_scale| {
                    match piano_roll_grid.lock() {
                        Ok(mut grid) => {
                            grid.set_vertical_zoom(piano_roll_vertical_zoom_scale.value());
                            piano_roll_drawing_area.set_height_request((grid.entity_height_in_pixels() * grid.zoom_vertical() * 127.0) as i32);
                        }
                        Err(_) => (),
                    }
                    piano_roll_drawing_area.queue_draw();
                    if let Ok(mut piano) =  piano.lock() {
                        piano.set_vertical_zoom(piano_roll_vertical_zoom_scale.value());
                        debug!("Piano keyboard vertical zoom level; {}", piano.zoom_vertical());
                        piano_keyboard_drawing_area.set_height_request((piano.entity_height_in_pixels() * piano.zoom_vertical() * 127.0) as i32);
                    }
                    piano_keyboard_drawing_area.queue_draw();
                });
            }

            {
                let piano_roll_vertical_zoom_scale = self.ui.piano_roll_vertical_zoom_scale.clone();
                self.ui.piano_roll_vertical_zoom_out.connect_clicked(move |_| {
                    let minimum_increment = piano_roll_vertical_zoom_scale.adjustment().minimum_increment();
                    piano_roll_vertical_zoom_scale.set_value(piano_roll_vertical_zoom_scale.value() - minimum_increment);
                });
            }

            {
                let piano_roll_vertical_zoom_scale = self.ui.piano_roll_vertical_zoom_scale.clone();
                self.ui.piano_roll_vertical_zoom_in.connect_clicked(move |_| {
                    let minimum_increment = piano_roll_vertical_zoom_scale.adjustment().minimum_increment();
                    piano_roll_vertical_zoom_scale.set_value(piano_roll_vertical_zoom_scale.value() + minimum_increment);
                });
            }

            {
                let piano_roll_grid_ruler = piano_roll_grid_ruler_arc;
                self.ui.piano_roll_ruler_drawing_area.connect_draw(move |drawing_area, context| {
                    match piano_roll_grid_ruler.lock() {
                        Ok(mut grid_ruler) => grid_ruler.paint(context, drawing_area),
                        Err(_) => (),
                    }
                    Inhibit(false)
                });
            }

            {
                let piano_roll_grid = piano_roll_grid_arc.clone();
                self.ui.piano_roll_add_mode_btn.connect_clicked(move |_| {
                    match piano_roll_grid.lock() {
                        Ok(mut grid) => grid.set_operation_mode(OperationModeType::Add),
                        Err(_) => (),
                    }
                });
            }

            {
                let piano_roll_grid = piano_roll_grid_arc.clone();
                self.ui.piano_roll_delete_mode_btn.connect_clicked(move |_| {
                    match piano_roll_grid.lock() {
                        Ok(mut grid) => grid.set_operation_mode(OperationModeType::Delete),
                        Err(_) => (),
                    }
                });
            }

            {
                let piano_roll_grid = piano_roll_grid_arc.clone();
                self.ui.piano_roll_edit_mode_btn.connect_clicked(move |_| {
                    match piano_roll_grid.lock() {
                        Ok(mut grid) => grid.set_operation_mode(OperationModeType::Change),
                        Err(_) => (),
                    }
                });
            }

            {
                let piano_roll_grid = piano_roll_grid_arc.clone();
                self.ui.piano_roll_select_mode_btn.connect_clicked(move |_| {
                    match piano_roll_grid.lock() {
                        Ok(mut grid) => grid.set_operation_mode(OperationModeType::PointMode),
                        Err(_) => (),
                    }
                });
            }

            {
                let piano_roll_grid = piano_roll_grid_arc.clone();
                let piano_roll_drawing_area = self.ui.piano_roll_drawing_area.clone();
                self.ui.piano_roll_cut_btn.connect_clicked(move |_| {
                    match piano_roll_grid.lock() {
                        Ok(mut grid) => grid.handle_cut(&piano_roll_drawing_area),
                        Err(_) => (),
                    }
                });
            }

            {
                let piano_roll_grid = piano_roll_grid_arc.clone();
                let piano_roll_drawing_area = self.ui.piano_roll_drawing_area.clone();
                self.ui.piano_roll_copy_btn.connect_clicked(move |_| {
                    match piano_roll_grid.lock() {
                        Ok(mut grid) => grid.handle_copy(&piano_roll_drawing_area),
                        Err(_) => (),
                    }
                });
            }

            {
                let piano_roll_grid = piano_roll_grid_arc.clone();
                let piano_roll_drawing_area = self.ui.piano_roll_drawing_area.clone();
                self.ui.piano_roll_paste_btn.connect_clicked(move |_| {
                    match piano_roll_grid.lock() {
                        Ok(mut grid) => grid.handle_paste(&piano_roll_drawing_area),
                        Err(_) => (),
                    }
                });
            }

            {
                let piano_roll_grid = piano_roll_grid_arc.clone();
                let piano_roll_drawing_area = self.ui.piano_roll_drawing_area.clone();
                self.ui.piano_roll_translate_left_btn.connect_clicked(move |_| {
                    match piano_roll_grid.lock() {
                        Ok(mut grid) => grid.handle_translate_left(&piano_roll_drawing_area),
                        Err(_) => (),
                    }
                });
            }

            {
                let piano_roll_grid = piano_roll_grid_arc.clone();
                let piano_roll_drawing_area = self.ui.piano_roll_drawing_area.clone();
                self.ui.piano_roll_translate_right_btn.connect_clicked(move |_| {
                    match piano_roll_grid.lock() {
                        Ok(mut grid) => grid.handle_translate_right(&piano_roll_drawing_area),
                        Err(_) => (),
                    }
                });
            }

            {
                let piano_roll_grid = piano_roll_grid_arc.clone();
                let piano_roll_drawing_area = self.ui.piano_roll_drawing_area.clone();
                self.ui.piano_roll_translate_up_btn.connect_clicked(move |_| {
                    match piano_roll_grid.lock() {
                        Ok(mut grid) => grid.handle_translate_up(&piano_roll_drawing_area),
                        Err(_) => (),
                    }
                });
            }

            {
                let piano_roll_grid = piano_roll_grid_arc.clone();
                let piano_roll_drawing_area = self.ui.piano_roll_drawing_area.clone();
                self.ui.piano_roll_translate_down_btn.connect_clicked(move |_| {
                    match piano_roll_grid.lock() {
                        Ok(mut grid) => grid.handle_translate_down(&piano_roll_drawing_area),
                        Err(_) => (),
                    }
                });
            }

            {
                let piano_roll_grid = piano_roll_grid_arc.clone();
                let piano_roll_quantise_start_choice = self.ui.piano_roll_quantise_start_choice.clone();
                self.ui.piano_roll_quantise_start_choice.connect_changed(move |_| {
                    match piano_roll_quantise_start_choice.active_text() {
                        Some(quantise_start_to_text) => {
                            let snap_position_in_beats = DAWUtils::get_snap_quantise_value_in_beats_from_choice_text(quantise_start_to_text.as_str(), 4.0);
                            match piano_roll_grid.try_lock() {
                                Ok(mut piano_roll_grid) => piano_roll_grid.set_snap_position_in_beats(snap_position_in_beats),
                                Err(_) => debug!("Unable to lock the piano grid in order to set the snap in beats."),
                            };
                        },
                        None => debug!("Unable to extract a quantise start value from the ComboBox - is there an active item?"),
                    };
                });
            }

            {
                let piano_roll_grid = piano_roll_grid_arc.clone();
                let piano_roll_quantise_length_choice = self.ui.piano_roll_quantise_length_choice.clone();
                self.ui.piano_roll_quantise_length_choice.connect_changed(move |_| {
                    match piano_roll_quantise_length_choice.active_text() {
                        Some(quantise_length_to_text) => {
                            let snap_length_in_beats = DAWUtils::get_snap_quantise_value_in_beats_from_choice_text(quantise_length_to_text.as_str(), 4.0);
                            match piano_roll_grid.try_lock() {
                                Ok(mut piano_roll_grid) => piano_roll_grid.set_new_entity_length_in_beats(snap_length_in_beats),
                                Err(_) => debug!("Unable to lock the piano grid in order to set the new entity length in beats."),
                            };
                        },
                        None => debug!("Unable to extract a quantise length value from the ComboBox - is there an active item?"),
                    };
                });
            }

            {
                let piano_roll_grid = piano_roll_grid_arc.clone();
                self.ui.piano_roll_note_length_increment_choice.connect_changed(move |piano_roll_note_length_increment_choice| {
                    match piano_roll_note_length_increment_choice.active_text() {
                        Some(note_increment_length_to_text) => {
                            let length_increment_in_beats = DAWUtils::get_snap_quantise_value_in_beats_from_choice_text(note_increment_length_to_text.as_str(), 4.0);
                            match piano_roll_grid.try_lock() {
                                Ok(mut piano_roll_grid) => piano_roll_grid.set_entity_length_increment_in_beats(length_increment_in_beats),
                                Err(_) => debug!("Unable to lock the piano grid in order to set the new length increment in beats."),
                            };
                        }
                        None => debug!("Unable to extract a length increment value from the ComboBox - is there an active item?"),
                    };
                });
            }

            {
                self.ui.piano_roll_quantise_start_checkbox.connect_clicked(move |_| {

                });
            }

            {
                self.ui.piano_roll_quantise_end_checkbox.connect_clicked(move |_| {

                });
            }

            {
                let piano_roll_grid = piano_roll_grid_arc.clone();
                let piano_roll_drawing_area = self.ui.piano_roll_drawing_area.clone();
                self.ui.piano_roll_quantise_btn.connect_clicked(move |_| {
                    match piano_roll_grid.lock() {
                        Ok(mut grid) => grid.handle_quantise(&piano_roll_drawing_area),
                        Err(_) => (),
                    }
                });
            }

            {
                let piano_roll_grid = piano_roll_grid_arc.clone();
                let piano_roll_drawing_area = self.ui.piano_roll_drawing_area.clone();
                self.ui.piano_roll_increase_note_length_btn.connect_clicked(move |_| {
                    match piano_roll_grid.lock() {
                        Ok(mut grid) => grid.handle_increase_entity_length(&piano_roll_drawing_area),
                        Err(_) => (),
                    }
                });
            }

            {
                let piano_roll_grid = piano_roll_grid_arc;
                let piano_roll_drawing_area = self.ui.piano_roll_drawing_area.clone();
                self.ui.piano_roll_decrease_note_length_btn.connect_clicked(move |_| {
                    match piano_roll_grid.lock() {
                        Ok(mut grid) => grid.handle_decrease_entity_length(&piano_roll_drawing_area),
                        Err(_) => (),
                    }
                });
            }

            {
                let piano_roll_component = self.ui.piano_roll_component.clone();
                let piano_roll_window = self.piano_roll_window.clone();
                let piano_roll_window_stack = self.piano_roll_window_stack.clone();
                let sub_panel_stack = self.ui.sub_panel_stack.clone();
                self.ui.piano_roll_dock_toggle_btn.connect_clicked(move |toggle_button| {
                    if toggle_button.is_active() {
                        sub_panel_stack.remove(&piano_roll_component);
                        piano_roll_window_stack.add_titled(&piano_roll_component, "piano_roll", "Piano Roll");
                        piano_roll_window.show_all();
                    }
                    else {
                        piano_roll_window_stack.remove(&piano_roll_component);
                        sub_panel_stack.add_titled(&piano_roll_component, "piano_roll", "Piano Roll");
                        piano_roll_window.hide();
                    }
                });
            }
        }
    }

    pub fn setup_sample_library(
        &mut self,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>
    ) {
        {
            let tx_from_ui = tx_from_ui.clone();
            self.ui.sample_library_file_chooser_widget.connect_selection_changed(move |file_chooser_widget| {
                if let Some(file_name) = file_chooser_widget.filename() {
                    if let Ok(file_meta_data) = std::fs::metadata(file_name.clone()) {
                        let file_meta_data: std::fs::Metadata = file_meta_data;
                        if file_meta_data.is_file() {
                            debug!("Sample library file choose: selected file name={:?}", file_name);
                            match tx_from_ui.send(DAWEvents::PreviewSample(file_name.to_str().unwrap().to_string())) {
                                Ok(_) => {}
                                Err(_) => {}
                            }
                        }
                    }

                }
            });
        }

        {
            let tx_from_ui = tx_from_ui;
            let sample_library_file_chooser_widget = self.ui.sample_library_file_chooser_widget.clone();
            self.ui.sample_library_add_sample_to_song_btn.connect_clicked(move |_| {
                if let Some(file_name) = sample_library_file_chooser_widget.filename() {
                    if let Ok(file_meta_data) = std::fs::metadata(file_name.clone()) {
                        let file_meta_data: std::fs::Metadata = file_meta_data;
                        if file_meta_data.is_file() {
                            debug!("Sample library file choose: selected file name={:?}", file_name);
                            match tx_from_ui.send(DAWEvents::SampleAdd(file_name.to_str().unwrap().to_string())) {
                                Ok(_) => {}
                                Err(_) => {}
                            }
                        }
                    }

                }
            });
        }

        {
            let sample_library_component = self.ui.sample_library_component.clone();
            let sample_library_window = self.sample_library_window.clone();
            let sample_library_window_stack = self.sample_library_window_stack.clone();
            let sub_panel_stack = self.ui.sub_panel_stack.clone();
            self.ui.sample_library_dock_toggle_btn.connect_clicked(move |toggle_button| {
                if toggle_button.is_active() {
                    sub_panel_stack.remove(&sample_library_component);
                    sample_library_window_stack.add_titled(&sample_library_component, "sample_library", "Sample Library");
                    sample_library_window.show_all();
                }
                else {
                    sample_library_window_stack.remove(&sample_library_component);
                    sub_panel_stack.add_titled(&sample_library_component, "sample_library", "Sample Library");
                    sample_library_window.hide();
                }
            });
        }
    }

    pub fn setup_scripting_view(
        &mut self,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>
    ) {
        {
            let scripting_script_text_view = self.ui.scripting_script_text_view.clone();
            let scripting_script_name_label = self.ui.scripting_script_name_label.clone();
            self.ui.scripting_file_chooser_widget.connect_selection_changed(move |file_chooser_widget| {
                if let Some(file_name) = file_chooser_widget.filename() {
                    if let Ok(file_meta_data) = std::fs::metadata(file_name.clone()) {
                        let file_meta_data: std::fs::Metadata = file_meta_data;
                        if file_meta_data.is_file() {
                            debug!("Scripting file chooser: selected file name={:?}", file_name);
                            match std::fs::read_to_string(file_name.clone()) {
                                Ok(script_text) => {
                                    scripting_script_name_label.set_label(file_name.to_str().expect("Could not get file name."));
                                    scripting_script_text_view.buffer().expect("Couldn't get text view buffer.").set_text(script_text.as_str());
                                }
                                Err(_) => {}
                            }
                        }
                    }

                }
            });
        }

        {
            let tx_from_ui = tx_from_ui.clone();
            let scripting_script_text_view = self.ui.scripting_script_text_view.clone();
            let scripting_script_name_label = self.ui.scripting_script_name_label.clone();
            self.ui.scripting_run_script_btn.connect_clicked(move |_| {
                let file_name = scripting_script_name_label.label().to_string();
                let text_buffer = scripting_script_text_view.buffer().expect("Couldn't get text view buffer.");
                let script_text = text_buffer.text(&text_buffer.start_iter(), &text_buffer.end_iter(), true);

                if let Some(script) = script_text {
                    if script.len() > 0 {
                        debug!("Scripting - running: selected file name={:?}", file_name);

                        match tx_from_ui.send(DAWEvents::RunLuaScript(script.to_string())) {
                            Ok(_) => {}
                            Err(_) => {}
                        }
                    }
                    else {
                        debug!("Script is empty.");
                    }
                }
            });
        }

        {
            let tx_from_ui = tx_from_ui;
            let scripting_console_input_text_view = self.ui.scripting_console_input_text_view.clone();
            let scripting_console_output_text_view = self.ui.scripting_console_output_text_view.clone();
            self.ui.scripting_console_run_btn.connect_clicked(move |_| {
                let console_input_text_buffer = scripting_console_input_text_view.buffer().expect("Couldn't get text view buffer.");
                let script_text = console_input_text_buffer.text(&console_input_text_buffer.start_iter(), &console_input_text_buffer.end_iter(), true);

                if let Some(script) = script_text {
                    if script.len() > 0 {
                        debug!("Scripting - running console input");
                        if let Some(console_output_text_buffer) = scripting_console_output_text_view.buffer() {
                            let console_input_text = format!(">> {}\n", script.as_str());
                            console_output_text_buffer.insert(&mut console_output_text_buffer.end_iter(), console_input_text.as_str());
                        }

                        match tx_from_ui.send(DAWEvents::RunLuaScript(script.to_string())) {
                            Ok(_) => {}
                            Err(_) => {}
                        }

                        console_input_text_buffer.set_text("");
                    }
                    else {
                        debug!("Script is empty.");
                    }
                }
            });
        }

        {
            let scripting_script_text_view = self.ui.scripting_script_text_view.clone();
            let scripting_script_name_label = self.ui.scripting_script_name_label.clone();
            self.ui.scripting_save_script_btn.connect_clicked(move |_| {
                let file_name = scripting_script_name_label.label().to_string();
                if !file_name.is_empty() {
                    if let Ok(file_meta_data) = std::fs::metadata(file_name.clone()) {
                        let file_meta_data: std::fs::Metadata = file_meta_data;
                        if file_meta_data.is_file() {
                            if let Some(text_buffer) = scripting_script_text_view.buffer() {
                                let script_text = text_buffer.text(&text_buffer.start_iter(), &text_buffer.end_iter(), true);

                                if let Some(script) = script_text {
                                    debug!("Saving file name={}", file_name);

                                    if let Ok(mut file) = std::fs::File::create(file_name) {
                                        if let Err(error) = file.write_all(script.as_bytes()) {
                                            debug!("Could not write script to a file: {}", error);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                else {
                    debug!("Could not save the script because there is no file name.");
                }
            });
        }

        {
            let scripting_script_text_view = self.ui.scripting_script_text_view.clone();
            let scripting_script_name_label = self.ui.scripting_script_name_label.clone();
            let window = self.ui.get_wnd_main().clone();
            self.ui.scripting_save_script_as_btn.connect_clicked(move |_| {
                let dialog = FileChooserDialog::new(Some("DAW save as script file"), Some(&window), FileChooserAction::Save);
                let filter = FileFilter::new();
                filter.set_name(Some("DAW Lua script file"));
                filter.add_pattern("*.lua");
                dialog.add_filter(&filter);
                dialog.add_button("Cancel", gtk::ResponseType::Cancel);
                dialog.add_button("Ok", gtk::ResponseType::Ok);
                let result = dialog.run();
                if result == gtk::ResponseType::Ok {
                    if let Some(path) = dialog.filename() {
                        if let Some(file_name) = path.to_str() {
                            if let Some(text_buffer) = scripting_script_text_view.buffer() {
                                let script_text = text_buffer.text(&text_buffer.start_iter(), &text_buffer.end_iter(), true);

                                if let Some(script) = script_text {
                                    debug!("Saving as file name={}", file_name);

                                    if let Ok(mut file) = std::fs::File::create(file_name) {
                                        if let Err(error) = file.write_all(script.as_bytes()) {
                                            debug!("Could not write script to a file: {}", error);
                                        }
                                        else {
                                            scripting_script_name_label.set_label(file_name);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                dialog.hide();
            });
        }

        {
            let scripting_script_text_view = self.ui.scripting_script_text_view.clone();
            let scripting_script_name_label = self.ui.scripting_script_name_label.clone();
            self.ui.scripting_new_script_btn.connect_clicked(move |_| {
                scripting_script_name_label.set_label("");
                if let Some(text_buffer) = scripting_script_text_view.buffer() {
                    text_buffer.set_text("");
                }
            });
        }

        {
            let scripting_component = self.ui.scripting_component.clone();
            let scripting_window = self.scripting_window.clone();
            let scripting_window_stack = self.scripting_window_stack.clone();
            let sub_panel_stack = self.ui.sub_panel_stack.clone();
            self.ui.scripting_dock_toggle_btn.connect_clicked(move |toggle_button| {
                if toggle_button.is_active() {
                    sub_panel_stack.remove(&scripting_component);
                    scripting_window_stack.add_titled(&scripting_component, "scripting", "Scripting");
                    scripting_window.show_all();
                }
                else {
                    scripting_window_stack.remove(&scripting_component);
                    sub_panel_stack.add_titled(&scripting_component, "scripting", "Scripting");
                    scripting_window.hide();
                }
            });
        }
    }

    pub fn update_sample_roll_sample_browser(&mut self, sample_uuid: String, sample_name: String) {
        if let Some(sample_roll_available_samples_list_store) = self.ui.sample_roll_available_samples.model() {
            if let Some(list_store) = sample_roll_available_samples_list_store.dynamic_cast_ref::<ListStore>() {
                debug!("Updating the sample roll sample browser...");
                list_store.insert_with_values(None, &[
                    (0, &sample_name),
                    (1, &sample_uuid),
                ]);
                self.ui.sample_roll_available_samples.show_all();
            }
        }
    }

    pub fn setup_sample_roll(
        &mut self,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        state: Arc<Mutex<DAWState>>
    ) {
        let sample_roll_available_samples_list_store = ListStore::new(&[String::static_type(), String::static_type()]);
        self.ui.sample_roll_available_samples.set_model(Some(&sample_roll_available_samples_list_store));

        {
            let state = state;
            let sample_roll_custom_painter = SampleRollCustomPainter::new(state);
            let sample_roll_grid = BeatGrid::new_with_custom(
                2.0,
                1.0,
                self.ui.sample_roll_drawing_area.height_request() as f64 / 10.0,
                50.0,
                4,
                Some(std::boxed::Box::new(sample_roll_custom_painter)),
                Some(std::boxed::Box::new(SampleRollMouseCoordHelper)),
                tx_from_ui.clone(),
                true,
                None,
            );
            let sample_roll_grid_arc = Arc::new( Mutex::new(sample_roll_grid));

            self.set_sample_roll_grid(Some(sample_roll_grid_arc.clone()));

            let sample_roll_grid_ruler = BeatGridRuler::new(2.0, 50.0, 4, tx_from_ui);
            let sample_roll_grid_ruler_arc = Arc::new(Mutex::new(sample_roll_grid_ruler));

            self.set_sample_roll_grid_ruler(Some(sample_roll_grid_ruler_arc.clone()));

            {
                let sample_roll_grid = sample_roll_grid_arc.clone();
                self.ui.sample_roll_drawing_area.connect_draw(move |drawing_area, context| {
                    match sample_roll_grid.lock() {
                        Ok(mut grid) => grid.paint(context, drawing_area),
                        Err(_) => (),
                    }
                    Inhibit(false)
                });
            }

            {
                let sample_roll_grid = sample_roll_grid_arc.clone();
                let sample_roll_drawing_area = self.ui.sample_roll_drawing_area.clone();
                self.ui.sample_roll_drawing_area.connect_motion_notify_event(move |_, motion_event| {
                    let coords = motion_event.coords().unwrap();
                    let control_key_pressed = motion_event.state().intersects(gdk::ModifierType::CONTROL_MASK);
                    let shift_key_pressed = motion_event.state().intersects(gdk::ModifierType::SHIFT_MASK);
                    let alt_key_pressed = motion_event.state().intersects(gdk::ModifierType::MOD1_MASK);
                    let mouse_button = if motion_event.state().intersects(gdk::ModifierType::BUTTON1_MASK) {
                        MouseButton::Button1
                    }
                    else if motion_event.state().intersects(gdk::ModifierType::BUTTON2_MASK) {
                        MouseButton::Button2
                    }
                    else {
                        MouseButton::Button3
                    };
                    match sample_roll_grid.lock() {
                        Ok(mut grid) => {
                            grid.handle_mouse_motion(coords.0, coords.1, &sample_roll_drawing_area, mouse_button, control_key_pressed, shift_key_pressed, alt_key_pressed);
                        },
                        Err(_) => (),
                    }
                    Inhibit(false)
                });
            }

            {
                let sample_roll_grid = sample_roll_grid_arc.clone();
                let sample_roll_drawing_area = self.ui.sample_roll_drawing_area.clone();
                self.ui.sample_roll_drawing_area.connect_button_press_event(move |_, event_btn| {
                    let coords = event_btn.coords().unwrap();
                    let control_key_pressed = event_btn.state().intersects(gdk::ModifierType::CONTROL_MASK);
                    let shift_key_pressed = event_btn.state().intersects(gdk::ModifierType::SHIFT_MASK);
                    let alt_key_pressed = event_btn.state().intersects(gdk::ModifierType::MOD1_MASK);
                    let mouse_button = if event_btn.state().intersects(gdk::ModifierType::BUTTON1_MASK) {
                        MouseButton::Button3
                    }
                    else if event_btn.state().intersects(gdk::ModifierType::BUTTON2_MASK) {
                        MouseButton::Button2
                    }
                    else {
                        MouseButton::Button1
                    };
                    debug!("Sample roll mouse pressed coords: x={}, y={}, Shift key: {}, Control key: {}", coords.0, coords.1, shift_key_pressed, control_key_pressed);
                    // let event_state = event_btn.state();
                    // debug!("Event modifier: {:?}", event_state);
                    match sample_roll_grid.lock() {
                        Ok(mut grid) => {
                            grid.handle_mouse_press(coords.0, coords.1, &sample_roll_drawing_area, mouse_button, control_key_pressed, shift_key_pressed, alt_key_pressed);
                        },
                        Err(_) => (),
                    }
                    Inhibit(false)
                });
            }

            {
                let sample_roll_grid = sample_roll_grid_arc.clone();
                let sample_roll_drawing_area = self.ui.sample_roll_drawing_area.clone();
                let sample_roll_available_samples = self.ui.sample_roll_available_samples.clone();
                self.ui.sample_roll_drawing_area.connect_button_release_event(move |_, event_btn| {
                    let coords = event_btn.coords().unwrap();
                    let control_key_pressed = event_btn.state().intersects(gdk::ModifierType::CONTROL_MASK);
                    let shift_key_pressed = event_btn.state().intersects(gdk::ModifierType::SHIFT_MASK);
                    let alt_key_pressed = event_btn.state().intersects(gdk::ModifierType::MOD1_MASK);
                    let mouse_button = if event_btn.state().intersects(gdk::ModifierType::BUTTON1_MASK) {
                        MouseButton::Button1
                    }
                    else if event_btn.state().intersects(gdk::ModifierType::BUTTON2_MASK) {
                        MouseButton::Button2
                    }
                    else {
                        MouseButton::Button3
                    };
                    debug!("Sample roll mouse released: x={}, y={}, Shift key: {}, Control key: {}", coords.0, coords.1, shift_key_pressed, control_key_pressed);

                    // get the selected sample
                    let selection = sample_roll_available_samples.selection();
                    if let Some((model, iter)) = selection.selected() {
                        debug!("{}, {}",
                                 model.value(&iter, 0).get::<String>().expect("Tree view selection, column 0"),
                                 model.value(&iter, 1).get::<String>().expect("Tree view selection, column 1"),
                        );
                        if let Ok(sample_uuid) = model.value(&iter, 1).get::<String>() {
                            // call the mouse event handler
                            match sample_roll_grid.lock() {
                                Ok(mut grid) => grid.handle_mouse_release(coords.0, coords.1, &sample_roll_drawing_area, mouse_button, control_key_pressed, shift_key_pressed, alt_key_pressed, sample_uuid),
                                Err(_) => {
                                    // call the mouse event handler
                                    match sample_roll_grid.lock() {
                                        Ok(mut grid) => grid.handle_mouse_release(coords.0, coords.1, &sample_roll_drawing_area, mouse_button, control_key_pressed, shift_key_pressed, alt_key_pressed, "".to_string()),
                                        Err(_) => (),
                                    }
                                },
                            }
                        }
                        else {
                            // call the mouse event handler
                            match sample_roll_grid.lock() {
                                Ok(mut grid) => grid.handle_mouse_release(coords.0, coords.1, &sample_roll_drawing_area, mouse_button, control_key_pressed, shift_key_pressed, alt_key_pressed, "".to_string()),
                                Err(_) => (),
                            }
                        }
                    }
                    else {
                        // call the mouse event handler
                        match sample_roll_grid.lock() {
                            Ok(mut grid) => grid.handle_mouse_release(coords.0, coords.1, &sample_roll_drawing_area, mouse_button, control_key_pressed, shift_key_pressed, alt_key_pressed, "".to_string()),
                            Err(_) => (),
                        }
                    }
                    Inhibit(false)
                });
            }

            {
                let sample_roll_grid = sample_roll_grid_arc.clone();
                let sample_roll_drawing_area = self.ui.sample_roll_drawing_area.clone();
                let sample_roll_grid_ruler = sample_roll_grid_ruler_arc.clone();
                let sample_roll_ruler_drawing_area = self.ui.sample_roll_ruler_drawing_area.clone();
                self.ui.sample_roll_zoom_scale.connect_value_changed(move |sample_roll_zoom_scale| {
                    match sample_roll_grid.lock() {
                        Ok(mut grid) => grid.set_horizontal_zoom(sample_roll_zoom_scale.value()),
                        Err(_) => (),
                    }
                    sample_roll_drawing_area.queue_draw();
                    match sample_roll_grid_ruler.lock() {
                        Ok(mut grid_ruler) => grid_ruler.set_horizontal_zoom(sample_roll_zoom_scale.value()),
                        Err(_) => (),
                    }
                    sample_roll_ruler_drawing_area.queue_draw();
                });
            }

            {
                let sample_roll_zoom_scale = self.ui.sample_roll_zoom_scale.clone();
                self.ui.sample_roll_zoom_out.connect_clicked(move |_| {
                    let minimum_increment = sample_roll_zoom_scale.adjustment().minimum_increment();
                    sample_roll_zoom_scale.set_value(sample_roll_zoom_scale.value() - minimum_increment);
                });
            }

            {
                let sample_roll_zoom_scale = self.ui.sample_roll_zoom_scale.clone();
                self.ui.sample_roll_zoom_in.connect_clicked(move |_| {
                    let minimum_increment = sample_roll_zoom_scale.adjustment().minimum_increment();
                    sample_roll_zoom_scale.set_value(sample_roll_zoom_scale.value() + minimum_increment);
                });
            }

            {
                let sample_roll_grid_ruler = sample_roll_grid_ruler_arc;
                self.ui.sample_roll_ruler_drawing_area.connect_draw(move |drawing_area, context| {
                    match sample_roll_grid_ruler.lock() {
                        Ok(mut grid_ruler) => grid_ruler.paint(context, drawing_area),
                        Err(_) => (),
                    }
                    Inhibit(false)
                });
            }

            {
                // let sample_roll_grid = sample_roll_grid.clone();
                // let sample_roll_available_samples_list_store = sample_roll_available_samples_list_store.clone();
                self.ui.sample_roll_sample_browser_delete_btn.connect_clicked(move |_| {
                    // TODO implement delete sample
                });
            }

            {
                let sample_roll_grid = sample_roll_grid_arc.clone();
                self.ui.sample_roll_add_mode_btn.connect_clicked(move |_| {
                    match sample_roll_grid.lock() {
                        Ok(mut grid) => grid.set_operation_mode(OperationModeType::Add),
                        Err(_) => (),
                    }
                });
            }

            {
                let sample_roll_grid = sample_roll_grid_arc.clone();
                self.ui.sample_roll_delete_mode_btn.connect_clicked(move |_| {
                    match sample_roll_grid.lock() {
                        Ok(mut grid) => grid.set_operation_mode(OperationModeType::Delete),
                        Err(_) => (),
                    }
                });
            }

            {
                let sample_roll_grid = sample_roll_grid_arc.clone();
                self.ui.sample_roll_edit_mode_btn.connect_clicked(move |_| {
                    match sample_roll_grid.lock() {
                        Ok(mut grid) => grid.set_operation_mode(OperationModeType::Change),
                        Err(_) => (),
                    }
                });
            }

            {
                let sample_roll_grid = sample_roll_grid_arc.clone();
                self.ui.sample_roll_select_mode_btn.connect_clicked(move |_| {
                    match sample_roll_grid.lock() {
                        Ok(mut grid) => grid.set_operation_mode(OperationModeType::PointMode),
                        Err(_) => (),
                    }
                });
            }

            {
                let sample_roll_grid = sample_roll_grid_arc.clone();
                let sample_roll_drawing_area = self.ui.sample_roll_drawing_area.clone();
                self.ui.sample_roll_cut_btn.connect_clicked(move |_| {
                    match sample_roll_grid.lock() {
                        Ok(mut grid) => grid.handle_cut(&sample_roll_drawing_area),
                        Err(_) => (),
                    }
                });
            }

            {
                let sample_roll_grid = sample_roll_grid_arc.clone();
                let sample_roll_drawing_area = self.ui.sample_roll_drawing_area.clone();
                self.ui.sample_roll_copy_btn.connect_clicked(move |_| {
                    match sample_roll_grid.lock() {
                        Ok(mut grid) => grid.handle_copy(&sample_roll_drawing_area),
                        Err(_) => (),
                    }
                });
            }

            {
                let sample_roll_grid = sample_roll_grid_arc.clone();
                let sample_roll_drawing_area = self.ui.sample_roll_drawing_area.clone();
                self.ui.sample_roll_paste_btn.connect_clicked(move |_| {
                    match sample_roll_grid.lock() {
                        Ok(mut grid) => grid.handle_paste(&sample_roll_drawing_area),
                        Err(_) => (),
                    }
                });
            }

            {
                let sample_roll_grid = sample_roll_grid_arc.clone();
                let sample_roll_drawing_area = self.ui.sample_roll_drawing_area.clone();
                self.ui.sample_roll_translate_left_btn.connect_clicked(move |_| {
                    match sample_roll_grid.lock() {
                        Ok(mut grid) => grid.handle_translate_left(&sample_roll_drawing_area),
                        Err(_) => (),
                    }
                });
            }

            {
                let sample_roll_grid = sample_roll_grid_arc.clone();
                let sample_roll_drawing_area = self.ui.sample_roll_drawing_area.clone();
                self.ui.sample_roll_translate_right_btn.connect_clicked(move |_| {
                    match sample_roll_grid.lock() {
                        Ok(mut grid) => grid.handle_translate_right(&sample_roll_drawing_area),
                        Err(_) => (),
                    }
                });
            }

            {
                let sample_roll_grid = sample_roll_grid_arc.clone();
                let sample_roll_drawing_area = self.ui.sample_roll_drawing_area.clone();
                self.ui.sample_roll_translate_up_btn.connect_clicked(move |_| {
                    match sample_roll_grid.lock() {
                        Ok(mut grid) => grid.handle_translate_up(&sample_roll_drawing_area),
                        Err(_) => (),
                    }
                });
            }

            {
                let sample_roll_grid = sample_roll_grid_arc.clone();
                let sample_roll_drawing_area = self.ui.sample_roll_drawing_area.clone();
                self.ui.sample_roll_translate_down_btn.connect_clicked(move |_| {
                    match sample_roll_grid.lock() {
                        Ok(mut grid) => grid.handle_translate_down(&sample_roll_drawing_area),
                        Err(_) => (),
                    }
                });
            }

            {
                let sample_roll_grid = sample_roll_grid_arc.clone();
                let sample_roll_quantise_start_choice = self.ui.sample_roll_quantise_start_choice.clone();
                self.ui.sample_roll_quantise_start_choice.connect_changed(move |_| {
                    match sample_roll_quantise_start_choice.active_text() {
                        Some(quantise_start_to_text) => {
                            let snap_position_in_beats = DAWUtils::get_snap_quantise_value_in_beats_from_choice_text(quantise_start_to_text.as_str(), 4.0);
                            match sample_roll_grid.try_lock() {
                                Ok(mut sample_roll_grid) => sample_roll_grid.set_snap_position_in_beats(snap_position_in_beats),
                                Err(_) => debug!("Unable to lock the piano grid in order to set the snap in beats."),
                            };
                        },
                        None => debug!("Unable to extract a quantise start value from the ComboBox - is there an active item?"),
                    };
                });
            }

            {
                let sample_roll_grid = sample_roll_grid_arc.clone();
                let sample_roll_quantise_length_choice = self.ui.sample_roll_quantise_length_choice.clone();
                self.ui.sample_roll_quantise_length_choice.connect_changed(move |_| {
                    match sample_roll_quantise_length_choice.active_text() {
                        Some(quantise_length_to_text) => {
                            let snap_length_in_beats = DAWUtils::get_snap_quantise_value_in_beats_from_choice_text(quantise_length_to_text.as_str(), 4.0);
                            match sample_roll_grid.try_lock() {
                                Ok(mut sample_roll_grid) => sample_roll_grid.set_new_entity_length_in_beats(snap_length_in_beats),
                                Err(_) => debug!("Unable to lock the piano grid in order to set the new entity length in beats."),
                            };
                        },
                        None => debug!("Unable to extract a quantise length value from the ComboBox - is there an active item?"),
                    };
                });
            }

            {
                self.ui.sample_roll_quantise_start_checkbox.connect_clicked(move |_| {

                });
            }

            {
                self.ui.sample_roll_quantise_end_checkbox.connect_clicked(move |_| {

                });
            }

            {
                let sample_roll_grid = sample_roll_grid_arc.clone();
                let sample_roll_drawing_area = self.ui.sample_roll_drawing_area.clone();
                self.ui.sample_roll_quantise_btn.connect_clicked(move |_| {
                    match sample_roll_grid.lock() {
                        Ok(mut grid) => grid.handle_quantise(&sample_roll_drawing_area),
                        Err(_) => (),
                    }
                });
            }

            {
                let sample_roll_grid = sample_roll_grid_arc.clone();
                let sample_roll_drawing_area = self.ui.sample_roll_drawing_area.clone();
                self.ui.sample_roll_increase_sample_length_btn.connect_clicked(move |_| {
                    match sample_roll_grid.lock() {
                        Ok(mut grid) => grid.handle_increase_entity_length(&sample_roll_drawing_area),
                        Err(_) => (),
                    }
                });
            }

            {
                let sample_roll_grid = sample_roll_grid_arc;
                let sample_roll_drawing_area = self.ui.sample_roll_drawing_area.clone();
                self.ui.sample_roll_decrease_sample_length_btn.connect_clicked(move |_| {
                    match sample_roll_grid.lock() {
                        Ok(mut grid) => grid.handle_decrease_entity_length(&sample_roll_drawing_area),
                        Err(_) => (),
                    }
                });
            }
        }

        {
            let sample_roll_component = self.ui.sample_roll_component.clone();
            let sample_roll_window = self.sample_roll_window.clone();
            let sample_roll_window_stack = self.sample_roll_window_stack.clone();
            let sub_panel_stack = self.ui.sub_panel_stack.clone();
            self.ui.sample_roll_dock_toggle_btn.connect_clicked(move |toggle_button| {
                if toggle_button.is_active() {
                    sub_panel_stack.remove(&sample_roll_component);
                    sample_roll_window_stack.add_titled(&sample_roll_component, "Sample_roll", "Sample Roll");
                    sample_roll_window.show_all();
                }
                else {
                    sample_roll_window_stack.remove(&sample_roll_component);
                    sub_panel_stack.add_titled(&sample_roll_component, "sample_roll", "Sample Roll");
                    sample_roll_window.hide();
                }
            });
        }
    }

    pub fn setup_riff_set_rif_ref(
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        state_arc: Arc<Mutex<DAWState>>,
        drawing_area: &DrawingArea,
    ) -> Arc<Mutex<BeatGrid>> {
        let state_arc = state_arc;
        let custom_painter = RiffSetTrackCustomPainter::new(state_arc);
        let mut beat_grid = BeatGrid::new_with_custom(
            10.0,
            1.0,
            drawing_area.height_request() as f64 / 127.0,
            1.0,
            4,
            Some(std::boxed::Box::new(custom_painter)),
            Some(std::boxed::Box::new(PianoRollMouseCoordHelper)),
            tx_from_ui,
            false,
            Some(DrawingAreaType::Riff),
        );
        beat_grid.draw_play_cursor = false;
        let beat_grid_arc = Arc::new( Mutex::new(beat_grid));

        {
            let grid = beat_grid_arc.clone();
            drawing_area.connect_draw(move |drawing_area, context| {
                match grid.lock() {
                    Ok(mut grid) => grid.paint(context, drawing_area),
                    Err(_) => (),
                }
                Inhibit(false)
            });
            drawing_area.queue_draw();
        }

        {
            let grid = beat_grid_arc.clone();
            let drawing_area = drawing_area.clone();
            drawing_area.clone().connect_button_release_event(move |_, event_btn| {
                let coords = event_btn.coords().unwrap();
                let control_key_pressed = event_btn.state().intersects(gdk::ModifierType::CONTROL_MASK);
                let shift_key_pressed = event_btn.state().intersects(gdk::ModifierType::SHIFT_MASK);
                let alt_key_pressed = event_btn.state().intersects(gdk::ModifierType::MOD1_MASK);
                let mouse_button = if event_btn.state().intersects(gdk::ModifierType::BUTTON1_MASK) {
                    MouseButton::Button1
                }
                else if event_btn.state().intersects(gdk::ModifierType::BUTTON2_MASK) {
                    MouseButton::Button2
                }
                else {
                    MouseButton::Button3
                };
                debug!("Piano roll mouse released: x={}, y={}, Shift key: {}, Control key: {}", coords.0, coords.1, shift_key_pressed, control_key_pressed);
                match grid.lock() {
                    Ok(mut grid) => grid.handle_mouse_release(coords.0, coords.1, &drawing_area, mouse_button, control_key_pressed, shift_key_pressed, alt_key_pressed, String::from("")),
                    Err(_) => (),
                }
                Inhibit(false)
            });
        }

        beat_grid_arc
    }

    pub fn setup_riff_sets_view(
        &mut self,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        state_arc: Arc<Mutex<DAWState>>
    ) {
        {
            let riff_sets_box = self.ui.riff_sets_box.clone();
            let riff_set_heads_box = self.ui.riff_set_heads_box.clone();
            let new_riff_set_name_entry = self.ui.new_riff_set_name_entry.clone();
            let tx_from_ui = tx_from_ui;
            let state_arc = state_arc;
            let selected_track_style_provider = self.selected_style_provider.clone();
            let mut riff_set_view_riff_set_beat_grids = self.riff_set_view_riff_set_beat_grids.clone();
            self.ui.add_riff_set_btn.connect_clicked(move |_| {
                if new_riff_set_name_entry.text().len() > 0 {
                    let riff_set_uuid = Uuid::new_v4();
                    let mut track_uuids = vec![];

                    if let Ok(locked_state) = state_arc.lock() {
                        for track in locked_state.project().song().tracks().iter() {
                            track_uuids.push(track.uuid().to_string())
                        }
                    }

                    let (riff_set_blade_head, _riff_set_blade, _) = MainWindow::add_riff_set_blade(
                        tx_from_ui.clone(),
                        riff_sets_box.clone(),
                        riff_set_heads_box.clone(),
                        riff_set_uuid.to_string(),
                        track_uuids,
                        state_arc.clone(),
                        new_riff_set_name_entry.text().to_string(),
                        RiffSetType::RiffSet,
                        selected_track_style_provider.clone(),
                        Some(riff_set_view_riff_set_beat_grids.clone()),
                        "".to_string(),
                        None,
                    );

                    riff_set_blade_head.riff_set_blade.set_margin_top(20);
                    riff_set_blade_head.riff_set_blade.set_height_request(100);
                    riff_set_blade_head.riff_set_blade_delete.hide();
                    riff_set_blade_head.riff_set_drag_btn.hide();

                    // move the new blade to the right position if there is a selection
                    let mut selected_child_position = None;
                    for (index, child) in riff_set_heads_box.children().iter().enumerate() {
                        unsafe {
                            if let Some(selected) = child.data::<u32>("selected") {
                                if *(selected.cast::<u32>().as_ptr()) == 1 {
                                    selected_child_position = Some(index);
                                    break;
                                }
                            }
                        }
                    }

                    if let Some(selected_child_position) = selected_child_position {
                        riff_set_heads_box.set_child_position(&riff_set_blade_head.riff_set_blade, selected_child_position as i32 + 1);
                        riff_sets_box.set_child_position(&_riff_set_blade.riff_set_box, selected_child_position as i32 + 1);
                    }

                    match tx_from_ui.send(DAWEvents::RiffSetAdd(riff_set_uuid, new_riff_set_name_entry.text().to_string())) {
                        Ok(_) => (),
                        Err(error) => debug!("Failed to send add riff set event: {}", error),
                    }

                    riff_set_blade_head.riff_set_name_entry.set_text(new_riff_set_name_entry.text().as_str());
                    new_riff_set_name_entry.set_text("");
                }
                else {
                    let dialogue = gtk::MessageDialog::builder()
                        .modal(true)
                        .text("Need a riff set name.")
                        .buttons(gtk::ButtonsType::Close)
                        .title("Problem")
                        .build();
                    dialogue.run();
                    dialogue.hide();
                }
            });
        }
    }

    pub fn add_riff_set_blade(
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        riff_sets_box: Box,
        riff_set_heads_box: Box,
        riff_set_uuid: String,
        track_uuids: Vec<String>,
        state_arc: Arc<Mutex<DAWState>>,
        riff_set_name: String,
        riff_set_type: RiffSetType,
        selected_style_provider: CssProvider,
        mut riff_set_beat_grids: Option<Arc<Mutex<HashMap<String, HashMap<String, Arc<Mutex<BeatGrid>>>>>>>,
        riff_set_instance_id: String,
        vertical_adjustment: Option<&Adjustment>,
    ) -> (RiffSetBladeHead, RiffSetBlade, Box) {
        let riff_set_blade_head_glade_src = include_str!("riff_set_blade_head.glade");
        let riff_set_blade_head: RiffSetBladeHead = RiffSetBladeHead::from_string(riff_set_blade_head_glade_src).unwrap();
        riff_set_blade_head.riff_set_blade_play.set_widget_name(riff_set_uuid.as_str());
        riff_set_blade_head.riff_set_drag_btn.drag_source_set(
            gdk::ModifierType::BUTTON1_MASK, 
            DRAG_N_DROP_TARGETS.as_ref(), 
            gdk::DragAction::COPY);
    
        {
            let riff_set_uuid = riff_set_uuid.clone();
            let riff_set_type = riff_set_type.clone();
            let riff_set_instance_id = riff_set_instance_id.clone();
            riff_set_blade_head.riff_set_drag_btn.connect_drag_data_get(move |_riff_set_drag_btn, _drag_context, selection_data, _info, _time| {
                debug!("Riff set drag data get called.");
                if let RiffSetType::RiffSet = riff_set_type {
                    selection_data.set_text(riff_set_uuid.as_str());
                }
                else {
                    selection_data.set_text(format!("{}_{}", riff_set_instance_id.as_str(), riff_set_uuid.as_str()).as_str());
                }
            });
        }


        let riff_set_blade_glade_src = include_str!("riff_set_blade.glade");
        let riff_set_blade: RiffSetBlade = RiffSetBlade::from_string(riff_set_blade_glade_src).unwrap();
        let riff_set_box: Box = riff_set_blade.riff_set_box.clone();

        if riff_set_instance_id.len() > 0 {
            riff_set_blade_head.riff_set_blade.set_widget_name(riff_set_instance_id.as_str());
            riff_set_box.set_widget_name(riff_set_instance_id.as_str());
        }
        else {
            riff_set_blade_head.riff_set_blade.set_widget_name(riff_set_uuid.as_str());
            riff_set_box.set_widget_name(riff_set_uuid.as_str());
        }

        let mut blade_box = Box::new(Orientation::Vertical, 0);
        if let RiffSetType::RiffArrangement(_) = riff_set_type { // A riff arrangement is the parent
            let riff_arrangement_riff_set_blade_glade_src = include_str!("riff_arrangement_riff_set_blade.glade");
            let riff_arrangement_riff_set_blade: RiffArrangementRiffSetBlade = RiffArrangementRiffSetBlade::from_string(riff_arrangement_riff_set_blade_glade_src).unwrap();
            let local_riff_set_box = riff_arrangement_riff_set_blade.local_riff_set_box.clone();
            let riff_set_head_box = riff_arrangement_riff_set_blade.riff_set_head_box.clone();
            let riff_set_box = riff_arrangement_riff_set_blade.riff_set_box.clone();

            riff_arrangement_riff_set_blade.riff_set_scrolled_window.set_vadjustment(vertical_adjustment);
            riff_arrangement_riff_set_blade.riff_set_scrolled_window.set_vscrollbar_policy(PolicyType::Never);
            riff_set_head_box.pack_start(&riff_set_blade_head.riff_set_blade, false, false, 2);
            riff_set_box.pack_start(&riff_set_blade.riff_set_box, false, false, 2);
            riff_sets_box.pack_start(&local_riff_set_box, false, false, 0);
            blade_box = local_riff_set_box;

            blade_box.set_widget_name(riff_set_instance_id.as_str());

            // riff_arrangement_riff_set_blade.riff_set_scrolled_window.
        }
        else if let RiffSetType::RiffSet = riff_set_type {
            for child in riff_set_box.children().iter() {
                child.set_width_request(69 + 15);
            }
            riff_set_heads_box.pack_start(&riff_set_blade_head.riff_set_blade, false, false, 2);
            riff_sets_box.pack_start(&riff_set_blade.riff_set_box, false, false, 2);
        }
        else {
            riff_set_heads_box.pack_start(&riff_set_blade_head.riff_set_blade, false, false, 2);
            riff_sets_box.pack_start(&riff_set_blade.riff_set_box, false, false, 2);
        }

        // for each track_uuid set the drawing area widget name using riff set uuid and track uuid and create a riff ref beat grid
        let mut count = 0;
        let mut riff_set_track_beat_grids = HashMap::new();
        for track_uuid in track_uuids.iter() {
            if let Some(child)  = riff_set_box.children().get(count) {
                if let Some(drawing_area) = child.dynamic_cast_ref::<DrawingArea>() {
                    drawing_area.set_visible(true);
                    drawing_area.set_widget_name(format!("riffset_{}_{}", riff_set_uuid.as_str(), track_uuid.as_str()).as_str());
                    let beat_grid = MainWindow::setup_riff_set_rif_ref(/*riff_set_uuid.clone(), track_uuid.clone(),*/ tx_from_ui.clone(),state_arc.clone(), drawing_area);
                    riff_set_track_beat_grids.insert(track_uuid.clone(), beat_grid);
                }
            }
            count += 1;
        }

        if let Some(riff_sets_tracks_beat_grids) = riff_set_beat_grids.as_mut() {
            let riff_set_key = if riff_set_instance_id.len() > 0 {
                if let RiffSetType::RiffSequence(riff_sequence_uuid) = riff_set_type.clone() {
                    format!("{}_{}", riff_sequence_uuid.as_str(), riff_set_instance_id.as_str())
                }
                else {
                    format!("{}_{}", riff_set_instance_id.as_str(), riff_set_uuid.as_str())
                }
            }
            else {
                riff_set_uuid.to_string()
            };

            if let Ok(mut riff_sets_tracks_beat_grids) = riff_sets_tracks_beat_grids.lock() {
                riff_sets_tracks_beat_grids.insert(riff_set_key, riff_set_track_beat_grids);
            }
        }

        match riff_set_type.clone() {
            RiffSetType::RiffSet => {
            }
            RiffSetType::RiffSequence(_) => {
                riff_set_blade_head.riff_set_blade_record.hide();
                riff_set_blade_head.riff_set_blade_copy.hide();
                riff_set_blade_head.riff_set_copy_to_track_view_btn.hide();
                // riff_set_blade_head.riff_set_select_btn.hide();
                // riff_set_blade_head.riff_set_blade_delete.hide();
                // riff_set_blade_head.riff_set_drag_btn.hide();
            }
            RiffSetType::RiffArrangement(_) => {
                riff_set_blade_head.riff_set_blade_record.hide();
                riff_set_blade_head.riff_set_blade_copy.hide();
                riff_set_blade_head.riff_set_copy_to_track_view_btn.hide();
            }
        }

        {
            let selected_style_provider = selected_style_provider.clone();
            let riff_set_type = riff_set_type.clone();
            let tx_from_ui = tx_from_ui.clone();
            let riff_set_blade = riff_set_blade_head.riff_set_blade.clone();
            let riff_set_heads_box = riff_set_heads_box.clone();
            let riff_set_instance_id = riff_set_instance_id.clone();
            let riff_set_uuid = riff_set_uuid.clone();
            unsafe {
                riff_set_blade.set_data("selected", 0u32);
            }
            riff_set_blade_head.riff_set_select_btn.connect_button_press_event(move |_, _| {
                unsafe  {
                    if let RiffSetType::RiffArrangement(_) = riff_set_type {
                        for child in riff_set_heads_box.children().iter() {
                            let actual_child = if let Some(local_riff_set_box) = child.dynamic_cast_ref::<Box>() {
                                if let Some(riff_set_head_box_widget) = local_riff_set_box.children().get(1) {
                                    if let Some(riff_set_head_box) = riff_set_head_box_widget.dynamic_cast_ref::<Box>() {
                                        riff_set_head_box.children().get(0).unwrap().clone()
                                    }
                                    else {
                                        child.clone()
                                    }
                                }
                                else {
                                    child.clone()
                                }
                            }
                            else {
                                child.clone()
                            };
                            if actual_child.widget_name().to_string() != riff_set_blade.widget_name().to_string() {
                                actual_child.style_context().remove_provider(&selected_style_provider);
                                actual_child.set_data("selected", 0u32);
                            }
                        }
                    }
                    else {
                        for child in riff_set_heads_box.children() {
                            if child.widget_name().to_string() != riff_set_blade.widget_name().to_string() {
                                child.style_context().remove_provider(&selected_style_provider);
                                child.set_data("selected", 0u32);
                            }
                        }
                    }

                    if let Some(selected) = riff_set_blade.data::<u32>("selected") {
                        let selected = selected.cast::<u32>().as_ptr();
                        let mut selected_bool = false;
                        if *selected == 0 {
                            riff_set_blade.style_context().add_provider(&selected_style_provider, gtk::STYLE_PROVIDER_PRIORITY_USER);
                            riff_set_blade.set_data("selected", 1u32);
                            selected_bool = true;
                        }
                        else {
                            riff_set_blade.style_context().remove_provider(&selected_style_provider);
                            riff_set_blade.set_data("selected", 0u32);
                        }

                        match &riff_set_type {
                            RiffSetType::RiffSet => {
                                let _ = tx_from_ui.send(DAWEvents::RiffSetSelect(riff_set_uuid.clone(), selected_bool));
                            }
                            RiffSetType::RiffSequence(riff_sequence_uuid) => {
                                let _ = tx_from_ui.send(DAWEvents::RiffSequenceRiffSetSelect(riff_sequence_uuid.to_string(), riff_set_instance_id.to_string(), selected_bool));
                            }
                            RiffSetType::RiffArrangement(riff_arrangement_uuid) => {
                                let _ = tx_from_ui.send(DAWEvents::RiffArrangementRiffItemSelect(riff_arrangement_uuid.to_string(), riff_set_instance_id.to_string(), selected_bool));
                            }
                        }
                    }
                }

                gtk::Inhibit(true)
            });
        }

        {
            let rs_box = riff_sets_box.clone();
            let rs_heads_box = riff_set_heads_box.clone();
            let blade_head = riff_set_blade_head.riff_set_blade.clone();
            let blade = riff_set_blade.riff_set_box.clone();
            let tx_from_ui = tx_from_ui.clone();
            let riff_set_type = riff_set_type.clone();
            let blade_box = blade_box.clone();
            let riff_set_instance_id = riff_set_instance_id.clone();
            riff_set_blade_head.riff_set_blade_delete.connect_clicked(move |_| {
                let riff_set_uuid = blade_head.widget_name().to_string();

                // if let RiffSetType::RiffArrangement(_) = riff_set_type {
                //     rs_box.remove(&blade_box)
                // }
                // else {
                //     rs_heads_box.remove(&blade_head);
                //     rs_box.remove(&blade)
                // }

                match riff_set_type.clone() {
                    RiffSetType::RiffSet => {
                        match tx_from_ui.send(DAWEvents::RiffSetDelete(riff_set_uuid)) {
                            Ok(_) => (),
                            Err(error) => debug!("Failed to send delete riff set event: {}", error),
                        }
                    }
                    RiffSetType::RiffSequence(riff_set_sequence_uuid) => {
                        let event_to_send = DAWEvents::RiffSequenceRiffSetDelete(riff_set_sequence_uuid, riff_set_instance_id.clone());
                        match tx_from_ui.send(event_to_send) {
                            Ok(_) => (),
                            Err(error) => debug!("Failed to send delete riff set from riff sequence event: {}", error),
                        }
                    }
                    RiffSetType::RiffArrangement(riff_set_arrangement_uuid) => {
                        rs_box.remove(&blade_box);
                        let event_to_send = DAWEvents::RiffArrangementRiffItemDelete(riff_set_arrangement_uuid, riff_set_instance_id.clone());
                        match tx_from_ui.send(event_to_send) {
                            Ok(_) => (),
                            Err(error) => debug!("Failed to send delete riff set from riff arrangement event: {}", error),
                        }
                    }
                };
            });
        }

        {
            let rs_box = riff_sets_box.clone();
            let blade_head = riff_set_blade_head.riff_set_blade.clone();
            let tx_from_ui = tx_from_ui.clone();
            // let selected_track_style_provider= selected_track_style_provider.clone();
            riff_set_blade_head.riff_set_blade_play.connect_clicked(move |_| {
                let uuid = blade_head.widget_name().to_string();

                // for child in rs_box.children() {
                //     child.style_context().remove_provider(&selected_track_style_provider);
                //     if child.widget_name() == uuid {
                //         child.style_context().add_provider(&selected_track_style_provider, gtk::STYLE_PROVIDER_PRIORITY_USER);
                //     }
                // }

                match tx_from_ui.send(DAWEvents::RiffSetPlay(uuid)) {
                    Ok(_) => (),
                    Err(error) => debug!("Failed to send play riff set event: {}", error),
                }
            });
        }

        {
            let rs_box = riff_sets_box.clone();
            let blade_head = riff_set_blade_head.riff_set_blade.clone();
            let tx_from_ui = tx_from_ui.clone();
            let selected_track_style_provider= selected_style_provider.clone();
            riff_set_blade_head.riff_set_copy_to_track_view_btn.connect_clicked(move |_| {
                let uuid = blade_head.widget_name().to_string();

                for child in rs_box.children() {
                    child.style_context().remove_provider(&selected_track_style_provider);
                    if child.widget_name() == uuid {
                        child.style_context().add_provider(&selected_track_style_provider, gtk::STYLE_PROVIDER_PRIORITY_USER);
                    }
                }

                match tx_from_ui.send(DAWEvents::RiffSetCopySelectedToTrackViewCursorPosition(uuid)) {
                    Err(error) => debug!("Failed to send riff sets view copy selected riff set contents to track view cursor position event: {}", error),
                    _ => (),
                }
            });
        }

        {
            let blade_head = riff_set_blade_head.riff_set_blade.clone();
            let tx_from_ui = tx_from_ui.clone();
            riff_set_blade_head.riff_set_name_entry.set_text(riff_set_name.as_str());
            riff_set_blade_head.riff_set_name_entry.connect_key_release_event(move |entry, event_key| {
                let name = entry.text().to_string();
                let uuid = blade_head.widget_name().to_string();

                if event_key.keyval() == gdk::keys::constants::Return {
                    match tx_from_ui.send(DAWEvents::RiffSetNameChange(uuid, name)) {
                        Ok(_) => (),
                        Err(error) => debug!("Failed to send riff set name change: {}", error),
                    }
                }

                gtk::Inhibit(true)
            });
        }

        if let RiffSetType::RiffSet = riff_set_type {
            {
                let riff_sets_box = riff_sets_box;
                let riff_set_heads_box = riff_set_heads_box.clone();
                let blade_head = riff_set_blade_head.riff_set_blade.clone();
                let blade = riff_set_blade.riff_set_box.clone();
                let tx_from_ui = tx_from_ui;
                let state = state_arc.clone();
                let selected_track_style_provider = selected_style_provider;
                let riff_set_beat_grids = riff_set_beat_grids;
                riff_set_blade_head.riff_set_blade_copy.connect_clicked(move |_| {
                    let riff_set_uuid_for_copy = Uuid::new_v4();
                    let riff_set_uuid = blade_head.widget_name().to_string();
                    let position = riff_sets_box.child_position(&blade);

                    match state.lock() {
                        Ok(state) => {
                            let track_uuids: Vec<String> = state.project().song().tracks().iter().map(|track| track.uuid().to_string()).collect();

                            MainWindow::add_riff_set_blade(
                                tx_from_ui.clone(),
                                riff_sets_box.clone(),
                                riff_set_heads_box.clone(),
                                riff_set_uuid_for_copy.to_string(),
                                track_uuids,
                                state_arc.clone(),
                                "Copy".to_string(),
                                RiffSetType::RiffSet,
                                selected_track_style_provider.clone(),
                                riff_set_beat_grids.clone(),
                                "".to_string(),
                                None
                            );

                            let copy_position = riff_set_heads_box.children().len() - 1;
                            if let Some(copy_blade) = riff_set_heads_box.children().get(copy_position) {
                                riff_set_heads_box.set_child_position(copy_blade, position + 1);
                            }

                            let copy_position = riff_sets_box.children().len() - 1;
                            if let Some(copy_blade) = riff_sets_box.children().get(copy_position) {
                                riff_sets_box.set_child_position(copy_blade, position + 1);
                            }

                            match tx_from_ui.send(DAWEvents::RiffSetCopy(riff_set_uuid, riff_set_uuid_for_copy)) {
                                Ok(_) => (),
                                Err(error) => debug!("Failed to send copy riff set event: {}", error),
                            }
                        }
                        Err(error) => {
                            debug!("Problem copying riff set: {}", error);
                        }
                    }
                });
            }
        }

        riff_set_blade_head.riff_set_blade.queue_draw();
        riff_set_blade.riff_set_box.queue_draw();

        (riff_set_blade_head, riff_set_blade, blade_box.clone())
    }

    pub fn delete_riff_set_blade(&mut self, riff_set_uuid: String) {
        for child in self.ui.riff_set_heads_box.children().iter() {
            if child.widget_name().to_string() == riff_set_uuid {
                self.ui.riff_set_heads_box.remove(child);
                break;
            }
        }
        for child in self.ui.riff_sets_box.children().iter() {
            if child.widget_name().to_string() == riff_set_uuid {
                self.ui.riff_sets_box.remove(child);
                break;
            }
        }
    }

    pub fn setup_riff_sequences_view(
        &mut self,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        state_arc: Arc<Mutex<DAWState>>
    ) {
        {
            let riff_sequences_box = self.ui.riff_sequences_box.clone();
            let riff_sequence_name_entry = self.ui.riff_sequence_name_entry.clone();
            let sequence_combobox = self.ui.sequence_combobox.clone();
            let tx_from_ui = tx_from_ui;
            let state_arc = state_arc;
            let selected_track_style_provider = self.selected_style_provider.clone();
            let riff_sequence_vertical_adjustment = self.ui.riff_sequence_vertical_adjustment.clone();
            self.ui.add_sequence_btn.connect_clicked(move |_| {
                riff_sequences_box.children().iter_mut().for_each(|child| child.hide());
                if riff_sequence_name_entry.text().len() > 0 {
                    let riff_sequence_blade = MainWindow::add_riff_sequence_blade(
                        riff_sequences_box.clone(),
                        tx_from_ui.clone(),
                        state_arc.clone(),
                        None,
                        None,
                        None,
                        true,
                        RiffSequenceType::RiffSequence,
                        selected_track_style_provider.clone(),
                        Some(&riff_sequence_vertical_adjustment),
                    );
                    if let Some(last_child) = riff_sequences_box.children().last() {
                        sequence_combobox.append(Some(last_child.widget_name().as_str()), riff_sequence_name_entry.text().as_str());
                        sequence_combobox.set_active_id(Some(last_child.widget_name().as_str()));
                        riff_sequence_blade.riff_sequence_name_entry.set_text(riff_sequence_name_entry.text().as_str());
                        riff_sequence_name_entry.set_text("");
                    }
                }
                else {
                    let dialogue = gtk::MessageDialog::builder()
                        .modal(true)
                        .text("Need a sequence name.")
                        .buttons(gtk::ButtonsType::Close)
                        .title("Problem")
                        .build();
                    dialogue.run();
                    dialogue.hide();
                }
            });
        }



        {
            let riff_sequences_box = self.ui.riff_sequences_box.clone();
            self.ui.sequence_combobox.connect_changed(move |sequence_combobox| {
                if let Some(uuid) = sequence_combobox.active_id() {
                    debug!("ui.sequence_combobox.active_id={}", uuid);
                    riff_sequences_box.children().iter_mut().for_each(|child| child.hide());
                    riff_sequences_box.children().iter_mut().for_each(|child| {
                        debug!("Found sequence uuid={}", child.widget_name());
                        if child.widget_name() == uuid {
                            child.show();
                        }
                    });
                }
            });
        }
    }

    fn add_riff_sequence_blade(
        riff_sequences_box: Box,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        state_arc: Arc<Mutex<DAWState>>,
        riff_sets_data: Option<Vec<(String, String)>>,
        riff_sequence_uuid: Option<String>,
        reference_item_uuid: Option<String>,
        send_riff_sequence_add_message: bool,
        riff_sequence_type: RiffSequenceType,
        selected_style_provider: CssProvider,
        riff_sequence_vertical_adjustment: Option<&Adjustment>,
    ) -> RiffSequenceBlade {
        let uuid = if let Some(existing_uuid) = riff_sequence_uuid {
            match Uuid::parse_str(existing_uuid.as_str()) {
                Ok(parsed_uuid) => parsed_uuid,
                Err(_) => Uuid::new_v4()
            }
        }
        else {
            Uuid::new_v4()
        };
        let riff_sequence_blade_glade_src = include_str!("riff_sequence_blade.glade");

        let riff_sequence_blade = RiffSequenceBlade::from_string(riff_sequence_blade_glade_src).unwrap();
        riff_sequences_box.pack_start(&riff_sequence_blade.riff_sequence_blade, true, true, 0);

        if let Some(riff_item_uuid) = reference_item_uuid.clone() {
            riff_sequence_blade.riff_sequence_blade.set_widget_name(riff_item_uuid.as_str());
        }
        else {
            riff_sequence_blade.riff_sequence_blade.set_widget_name(uuid.to_string().as_str());
        }

        unsafe {
            riff_sequence_blade.riff_sequence_blade.set_data("selected", 0u32);
        }

        riff_sequence_blade.riff_sequence_blade_play.set_widget_name(uuid.to_string().as_str());
        riff_sequence_blade.riff_sequence_riff_sets_scrolled_window.set_vadjustment(riff_sequence_vertical_adjustment);
        riff_sequence_blade.riff_sequence_drag_btn.drag_source_set(
            gdk::ModifierType::BUTTON1_MASK, 
            DRAG_N_DROP_TARGETS.as_ref(), 
            gdk::DragAction::COPY);

        let riff_set_type = match riff_sequence_type.clone() {
            RiffSequenceType::RiffSequence => {
                riff_sequence_blade.riff_sequence_drag_btn.hide();
                riff_sequence_blade.riff_sequence_select_btn.hide();
                RiffSetType::RiffSequence(uuid.to_string())
            },
            RiffSequenceType::RiffArrangement(riff_sequence_uuid) => {
                riff_sequence_blade.riff_sequence_blade_copy.hide();
                riff_sequence_blade.riff_sequence_riff_set_combobox_label.hide();
                riff_sequence_blade.riff_sequence_copy_to_track_view_btn.hide();
                riff_sequence_blade.riff_set_combobox.hide();
                riff_sequence_blade.add_riff_set_btn.hide();
                RiffSetType::RiffArrangement(riff_sequence_uuid)
            },
        };
        MainWindow::setup_riff_set_drag_and_drop(
            riff_sequence_blade.riff_set_head_box.clone(), 
            riff_sequence_blade.riff_set_box.clone(), 
            riff_sequence_blade.riff_seq_horizontal_adjustment.clone(), 
            riff_sequence_blade.riff_sets_view_port.clone(),
            riff_set_type,
            tx_from_ui.clone());
    
        {
            let riff_sequence_uuid = uuid.to_string();
            let reference_item_uuid = reference_item_uuid.clone();
            riff_sequence_blade.riff_sequence_drag_btn.connect_drag_data_get(move |_, _, selection_data, _, _| {
                debug!("Riff sequence drag data get called.");
                if let Some(uuid) = reference_item_uuid.clone() {
                    selection_data.set_text(uuid.as_str());
                }
                else {
                    selection_data.set_text(riff_sequence_uuid.as_str());
                }
            });
        }

        if send_riff_sequence_add_message {
            match tx_from_ui.send(DAWEvents::RiffSequenceAdd(uuid)) {
                Ok(_) => {}
                Err(_) => {}
            }
        }

        {
            let selected_style_provider = selected_style_provider.clone();
            let riff_sequence_type = riff_sequence_type.clone();
            let tx_from_ui = tx_from_ui.clone();
            let riff_sequence_blade_frame = riff_sequence_blade.riff_sequence_blade.clone();
            let riff_sequences_box = riff_sequences_box.clone();
            let reference_item_uuid = reference_item_uuid.clone();
            riff_sequence_blade.riff_sequence_select_btn.connect_button_press_event(move |_, _| {
                unsafe  {
                    for child in riff_sequences_box.children() {
                        let actual_child = if let Some(local_riff_set_box) = child.dynamic_cast_ref::<Box>() {
                            if let Some(riff_set_head_box_widget) = local_riff_set_box.children().get(1) {
                                if let Some(riff_set_head_box) = riff_set_head_box_widget.dynamic_cast_ref::<Box>() {
                                    riff_set_head_box.children().get(0).unwrap().clone()
                                }
                                else {
                                    child
                                }
                            }
                            else {
                                child
                            }
                        }
                        else {
                            child
                        };
                        if actual_child.widget_name().to_string() != riff_sequence_blade_frame.widget_name().to_string() {
                            actual_child.style_context().remove_provider(&selected_style_provider);
                            actual_child.set_data("selected", 0u32);
                        }
                    }

                    if let Some(mut selected) = riff_sequence_blade_frame.data::<u32>("selected") {
                        let selected = selected.cast::<u32>().as_ptr();
                        let mut selected_bool = false;
                        if *selected == 0 {
                            riff_sequence_blade_frame.style_context().add_provider(&selected_style_provider, gtk::STYLE_PROVIDER_PRIORITY_USER);
                            riff_sequence_blade_frame.set_data("selected", 1u32);
                            selected_bool = true;
                        }
                        else {
                            riff_sequence_blade_frame.style_context().remove_provider(&selected_style_provider);
                            riff_sequence_blade_frame.set_data("selected", 0u32);
                        }

                        if let RiffSequenceType::RiffArrangement(riff_arrangement_uuid) = &riff_sequence_type {
                            if let Some(riff_item_uuid) = &reference_item_uuid {
                                let _ = tx_from_ui.send(DAWEvents::RiffArrangementRiffItemSelect(riff_arrangement_uuid.to_string(), riff_item_uuid.to_string(), selected_bool));
                            }
                        }
                    }
                }

                gtk::Inhibit(true)
            });
        }

        {
            let blade = riff_sequence_blade.riff_sequence_blade.clone();
            let tx_from_ui = tx_from_ui.clone();
            riff_sequence_blade.riff_sequence_name_entry.set_text("");
            riff_sequence_blade.riff_sequence_name_entry.connect_changed(move |entry| {
                let name = entry.text().to_string();
                let uuid = blade.widget_name().to_string();

                if let Ok(name) = name.to_value().get() {
                    match tx_from_ui.send(DAWEvents::RiffSequenceNameChange(uuid, name)) {
                        Ok(_) => (),
                        Err(error) => debug!("Failed to send riff sequence name change: {}", error),
                    }
                }
            });
        }

        // populate the riff_set_combobox
        if let Some(combobox_riff_sets_data) = riff_sets_data {
            let riff_set_combobox: ComboBoxText = riff_sequence_blade.riff_set_combobox.clone();
            for (riff_set_uuid, riff_set_name) in combobox_riff_sets_data {
                riff_set_combobox.append(Some(riff_set_uuid.as_str()), riff_set_name.as_str());
            }
        }
        else {
            match state_arc.lock() {
                Ok(state) => {
                    let riff_set_combobox: ComboBoxText = riff_sequence_blade.riff_set_combobox.clone();
                    for riff_set in state.project().song().riff_sets().iter() {
                        riff_set_combobox.append(Some(riff_set.uuid().as_str()), riff_set.name());
                    }
                }
                Err(error) => {
                    debug!("Problem populating riff sequence blade riff sets combobox: {}", error);
                }
            }
        }

        // handle adding a riff set to the riff sequence
        {
            let blade = riff_sequence_blade.riff_sequence_blade.clone();
            let tx_from_ui = tx_from_ui.clone();
            let state_arc = state_arc;
            let riff_set_combobox: ComboBoxText = riff_sequence_blade.riff_set_combobox.clone();
            let riff_set_box: Box = riff_sequence_blade.riff_set_box.clone();
            let riff_set_head_box: Box = riff_sequence_blade.riff_set_head_box.clone();
            let selected_track_style_provider = selected_style_provider.clone();
            riff_sequence_blade.add_riff_set_btn.connect_clicked(move |_| {
                if let Some(riff_set_uuid) = riff_set_combobox.active_id() {
                    let riff_sequence_uuid = blade.widget_name().to_string();
                    let riff_set_reference_uuid = Uuid::new_v4();

                    match state_arc.try_lock() {
                        Ok(state) => {
                            let mut track_uuids = vec![];
                            for track in state.project().song().tracks().iter() {
                                track_uuids.push(track.uuid().to_string());
                            }
                            let (riff_set_blade_head, riff_set_blade, _) = MainWindow::add_riff_set_blade(
                                tx_from_ui.clone(),
                                riff_set_box.clone(),
                                riff_set_head_box.clone(),
                                riff_set_uuid.to_string(),
                                track_uuids,
                                state_arc.clone(),
                                riff_set_combobox.active_text().unwrap().to_string(),
                                RiffSetType::RiffSequence(riff_sequence_uuid.clone()),
                                selected_track_style_provider.clone(),
                                None,
                                riff_set_reference_uuid.to_string(),
                                None,
                            );

                            // move the new blade to the right position if there is a selection
                            let mut selected_child_position = None;
                            for (index, child) in riff_set_head_box.children().iter().enumerate() {
                                unsafe {
                                    if let Some(selected) = child.data::<u32>("selected") {
                                        if *(selected.cast::<u32>().as_ptr()) == 1 {
                                            selected_child_position = Some(index);
                                            break;
                                        }
                                    }
                                }
                            }

                            if let Some(selected_child_position) = selected_child_position {
                                riff_set_head_box.set_child_position(&riff_set_blade_head.riff_set_blade, selected_child_position as i32 + 1);
                                riff_set_box.set_child_position(&riff_set_blade.riff_set_box, selected_child_position as i32 + 1);
                            }

                            match tx_from_ui.send(DAWEvents::RiffSequenceRiffSetAdd(riff_sequence_uuid, riff_set_uuid.to_string(), riff_set_reference_uuid)) {
                                Ok(_) => (),
                                Err(error) => debug!("Failed to send riff sequence add riff set: {}", error),
                            }
                        }
                        Err(error) => {
                            debug!("Problem getting lock on state when adding a riff set to a rff sequence in the ui: {}", error);
                        }
                    }
                }
            });
        }

        {
            let blade = riff_sequence_blade.riff_sequence_blade.clone();
            let tx_from_ui = tx_from_ui.clone();
            let riff_sequences_box = riff_sequences_box.clone();
            let riff_sequence_type = riff_sequence_type.clone();
            let reference_item_uuid = reference_item_uuid.clone();
            riff_sequence_blade.riff_sequence_blade_delete.connect_clicked(move |_| {
                let riff_sequence_uuid = blade.widget_name().to_string();

                let event_to_send = match riff_sequence_type.clone() {
                    RiffSequenceType::RiffSequence => DAWEvents::RiffSequenceDelete(riff_sequence_uuid),
                    RiffSequenceType::RiffArrangement(riff_arrangement_uuid) => {
                        riff_sequences_box.remove(&blade);
                        let reference_item_uuid = if let Some(uuid) = reference_item_uuid.clone() {
                            uuid
                        }
                        else {
                            riff_sequence_uuid // not correct but need to return a dummy value which will prevent anything from being deleted
                        };
                        DAWEvents::RiffArrangementRiffItemDelete(riff_arrangement_uuid, reference_item_uuid)
                    },
                };

                match tx_from_ui.send(event_to_send) {
                    Ok(_) => (),
                    Err(error) => debug!("Failed to send delete riff sequence: {}", error),
                }
            });
        }

        {
            let blade = riff_sequence_blade.riff_sequence_blade.clone();
            let tx_from_ui = tx_from_ui.clone();
            let riff_sequences_box = riff_sequences_box.clone();
            // let selected_track_style_provider = selected_track_style_provider.clone();
            riff_sequence_blade.riff_sequence_blade_play.connect_clicked(move |_| {
                let riff_sequence_uuid = blade.widget_name().to_string();

                // for child in riff_sequences_box.children() {
                //     child.style_context().remove_provider(&selected_track_style_provider);
                //     if child.widget_name() == riff_sequence_uuid {
                //         child.style_context().add_provider(&selected_track_style_provider, gtk::STYLE_PROVIDER_PRIORITY_USER);
                //     }
                // }

                match tx_from_ui.send(DAWEvents::RiffSequencePlay(riff_sequence_uuid)) {
                    Ok(_) => (),
                    Err(error) => debug!("Failed to send riff sequence play: {}", error),
                }
            });
        }

        {
            let blade = riff_sequence_blade.riff_sequence_blade.clone();
            let tx_from_ui = tx_from_ui;
            let riff_sequences_box = riff_sequences_box;
            let selected_track_style_provider = selected_style_provider;
            riff_sequence_blade.riff_sequence_copy_to_track_view_btn.connect_clicked(move |_| {
                let riff_sequence_uuid = blade.widget_name().to_string();

                for child in riff_sequences_box.children() {
                    child.style_context().remove_provider(&selected_track_style_provider);
                    if child.widget_name() == riff_sequence_uuid {
                        child.style_context().add_provider(&selected_track_style_provider, gtk::STYLE_PROVIDER_PRIORITY_USER);
                    }
                }

                match tx_from_ui.send(DAWEvents::RiffSequenceCopySelectedToTrackViewCursorPosition(riff_sequence_uuid)) {
                    Err(error) => debug!("Failed to send riff sets view copy selected riff sequence contents to track view cursor position event: {}", error),
                    _ => (),
                }
            });
        }

        riff_sequence_blade
    }


    pub fn update_riff_sequences_combobox_in_riff_sequence_view(
        &mut self,
        state: &DAWState,
    ) {
        // get the available riff sequences
        let riff_sequences: Vec<(String, String)> = state.project().song().riff_sequences().iter().map(|riff_sequence| (riff_sequence.uuid(), riff_sequence.name().to_string())).collect();

        // update the riff sequences in the update_riff_sequences_combobox_in_riff_sequence_view the riff sequence view
        self.ui.sequence_combobox.remove_all();
        for riff_sequence_details in riff_sequences.iter() {
            self.ui.sequence_combobox.append(Some(riff_sequence_details.0.as_str()), riff_sequence_details.1.as_str());
        }
        if self.ui.sequence_combobox.children().len() > 0 {
            self.ui.sequence_combobox.set_active(Some(0));
        }
    }


    pub fn delete_riff_sequence_blade(&mut self, riff_sequence_uuid: String) {
        for child in self.ui.riff_sequences_box.children().iter() {
            if child.widget_name().to_string() == riff_sequence_uuid {
                self.ui.riff_sequences_box.remove(child);
                break;
            }
        }
    }

    pub fn setup_riff_arrangements_view(
        &mut self,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        state_arc: Arc<Mutex<DAWState>>
    ) {
        {
            let riff_arrangement_box = self.ui.riff_arrangement_box.clone();
            let riff_arrangements_combobox = self.ui.arrangements_combobox.clone();
            let new_arrangement_name_entry = self.ui.new_arrangement_name_entry.clone();
            let arrangement_sequence_combobox = self.ui.arrangements_combobox.clone();
            let state_arc = state_arc.clone();
            let tx_from_ui = tx_from_ui.clone();
            let selected_track_style_provider = self.selected_style_provider.clone();
            let riff_arrangement_vertical_adjustment = self.ui.riff_arrangement_vertical_adjustment.clone();
            let _arrangement_vertical_adjustment = self.ui.riff_arrangement_vertical_adjustment.clone();
            self.ui.add_arrangement_btn.connect_clicked(move |_| {
                riff_arrangement_box.children().iter_mut().for_each(|child| child.hide());
                if new_arrangement_name_entry.text().len() > 0 {
                    let riff_arrangement_blade = MainWindow::add_riff_arrangement_blade(
                        riff_arrangement_box.clone(),
                        riff_arrangements_combobox.clone(),
                        tx_from_ui.clone(),
                        state_arc.clone(),
                        None,
                        None,
                        None,
                        true,
                        true,
                        selected_track_style_provider.clone(),
                        riff_arrangement_vertical_adjustment.clone(),
                    );
                    if let Some(last_child) = riff_arrangement_box.children().last() {
                        arrangement_sequence_combobox.append(Some(last_child.widget_name().as_str()), new_arrangement_name_entry.text().as_str());
                        arrangement_sequence_combobox.set_active_id(Some(last_child.widget_name().as_str()));
                        riff_arrangement_blade.riff_arrangement_name_entry.set_text(new_arrangement_name_entry.text().as_str());
                        new_arrangement_name_entry.set_text("");
                    }
                }
                else {
                    let dialogue = gtk::MessageDialog::builder()
                        .modal(true)
                        .text("Need an arrangement name.")
                        .buttons(gtk::ButtonsType::Close)
                        .title("Problem")
                        .build();
                    dialogue.run();
                    dialogue.hide();
                }
            });
        }

        {
            let riff_arrangement_box = self.ui.riff_arrangement_box.clone();
            let state = state_arc.clone();
            let tx_from_ui = tx_from_ui.clone();
            self.ui.arrangements_combobox.connect_changed(move |arrangements_combobox| {
                if let Some(uuid) = arrangements_combobox.active_id() {
                    debug!("ui.arrangements_combobox.active_id={}", uuid);
                    riff_arrangement_box.children().iter_mut().for_each(|child| child.hide());
                    riff_arrangement_box.children().iter_mut().for_each(|child| {
                        debug!("Found arrangement uuid={}", child.widget_name());
                        if child.widget_name() == uuid {
                            child.show();
                        }
                    });

                    {
                        let state = state.clone();
                        let tx_from_ui = tx_from_ui.clone();
                        let _ = std::thread::Builder::new().name("Set sel riffarr".into()).spawn(move || {
                            // update the state
                            if let Ok(mut state) = state.lock() {
                                state.set_selected_riff_arrangement_uuid(Some(uuid.to_string()));
                                let _ = tx_from_ui.send(DAWEvents::RepaintAutomationView);
                            }
                        });
                    }
                }
            });
        }
    }

    fn add_riff_arrangement_blade(
        riff_arrangements_box: Box,
        riff_arrangements_combobox: ComboBoxText,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        state_arc: Arc<Mutex<DAWState>>,
        riff_sets_data: Option<Vec<(String, String)>>,
        riff_sequences_data: Option<Vec<(String, String)>>,
        riff_arrangement_uuid: Option<String>,
        send_riff_arrangement_add_message: bool,
        visible: bool,
        selected_track_style_provider: CssProvider,
        riff_arrangement_vertical_adjustment: Adjustment,
    ) -> RiffArrangementBlade {
        let uuid = if let Some(existing_uuid) = riff_arrangement_uuid {
            match Uuid::parse_str(existing_uuid.as_str()) {
                Ok(parsed_uuid) => parsed_uuid,
                Err(_) => Uuid::new_v4()
            }
        }
        else {
            Uuid::new_v4()
        };
        let riff_arrangement_blade_glade_src = include_str!("riff_arrangement_blade.glade");


        let riff_arrangement_blade = RiffArrangementBlade::from_string(riff_arrangement_blade_glade_src).unwrap();
        riff_arrangements_box.pack_start(&riff_arrangement_blade.riff_arrangement_blade, true, true, 0);
        riff_arrangement_blade.riff_arrangement_blade.set_widget_name(uuid.to_string().as_str());
        riff_arrangement_blade.riff_arrangement_blade_play.set_widget_name(uuid.to_string().as_str());

        MainWindow::setup_riff_arrangement_riff_item_drag_and_drop(
            riff_arrangement_blade.riff_set_box.clone(),
            riff_arrangement_blade.riff_arr_horizontal_adjustment.clone(),
            riff_arrangement_blade.riff_items_view_port.clone(),
            tx_from_ui.clone(),
            RiffSetType::RiffArrangement(uuid.to_string()),
        );

        // MainWindow::setup_riff_view_drag_and_drop(riff_arrangement_blade.riff_set_box.clone(), tx_from_ui.clone());

        if send_riff_arrangement_add_message {
            match tx_from_ui.send(DAWEvents::RiffArrangementAdd(uuid)) {
                Ok(_) => {}
                Err(_) => {}
            }
        }

        {
            let blade = riff_arrangement_blade.riff_arrangement_blade.clone();
            let tx_from_ui = tx_from_ui.clone();
            riff_arrangement_blade.riff_arrangement_name_entry.set_text("");
            riff_arrangement_blade.riff_arrangement_name_entry.connect_changed(move |entry| {
                let name = entry.text().to_string();
                let uuid = blade.widget_name().to_string();

                if let Ok(name) = name.to_value().get() {
                    match tx_from_ui.send(DAWEvents::RiffArrangementNameChange(uuid, name)) {
                        Ok(_) => (),
                        Err(error) => debug!("Failed to send riff arrangement name change: {}", error),
                    }
                }
            });
        }

        // populate the riff_set_combobox
        if let Some(combobox_riff_sets_data) = riff_sets_data {
            let riff_set_combobox: ComboBoxText = riff_arrangement_blade.riff_set_combobox.clone();
            for (riff_set_uuid, riff_set_name) in combobox_riff_sets_data {
                riff_set_combobox.append(Some(riff_set_uuid.as_str()), riff_set_name.as_str());
            }
        }
        else {
            match state_arc.lock() {
                Ok(state) => {
                    let riff_set_combobox: ComboBoxText = riff_arrangement_blade.riff_set_combobox.clone();
                    for riff_set in state.project().song().riff_sets().iter() {
                        riff_set_combobox.append(Some(riff_set.uuid().as_str()), riff_set.name());
                    }
                }
                Err(error) => {
                    debug!("Problem populating riff arrangement blade riff sets combobox: {}", error);
                }
            }
        }

        // populate the riff_sequence_combobox
        if let Some(combobox_riff_sequences_data) = riff_sequences_data {
            let riff_sequence_combobox: ComboBoxText = riff_arrangement_blade.riff_sequence_combobox.clone();
            for (riff_sequence_uuid, riff_sequence_name) in combobox_riff_sequences_data {
                riff_sequence_combobox.append(Some(riff_sequence_uuid.as_str()), riff_sequence_name.as_str());
            }
        }
        else {
            match state_arc.lock() {
                Ok(state) => {
                    let riff_sequence_combobox: ComboBoxText = riff_arrangement_blade.riff_sequence_combobox.clone();
                    for riff_sequence in state.project().song().riff_sequences().iter() {
                        riff_sequence_combobox.append(Some(riff_sequence.uuid().as_str()), riff_sequence.name());
                    }
                }
                Err(error) => {
                    debug!("Problem populating riff arrangement blade riff sequences combobox: {}", error);
                }
            }
        }

        // handle adding a riff set to the riff arrangement
        {
            let blade = riff_arrangement_blade.riff_arrangement_blade.clone();
            let tx_from_ui = tx_from_ui.clone();
            let riff_set_combobox: ComboBoxText = riff_arrangement_blade.riff_set_combobox.clone();
            riff_arrangement_blade.add_riff_set_btn.connect_clicked(move |_| {
                if let Some(riff_set_uuid) = riff_set_combobox.active_id() {
                    let riff_arrangement_uuid = blade.widget_name().to_string();

                    match tx_from_ui.send(DAWEvents::RiffArrangementRiffItemAdd(riff_arrangement_uuid, riff_set_uuid.to_string(), RiffItemType::RiffSet)) {
                        Ok(_) => (),
                        Err(error) => debug!("Failed to send riff arrangement add riff set: {}", error),
                    }
                }
            });
        }

        // handle adding a riff sequence to the riff arrangement
        {
            let blade = riff_arrangement_blade.riff_arrangement_blade.clone();
            let tx_from_ui = tx_from_ui.clone();
            let riff_sequence_combobox: ComboBoxText = riff_arrangement_blade.riff_sequence_combobox.clone();
            riff_arrangement_blade.add_riff_sequence_btn.connect_clicked(move |_| {
                if let Some(riff_sequence_uuid) = riff_sequence_combobox.active_id().as_ref() {
                    let riff_arrangement_uuid = blade.widget_name().to_string();

                    match tx_from_ui.send(DAWEvents::RiffArrangementRiffItemAdd(riff_arrangement_uuid, riff_sequence_uuid.to_string(), RiffItemType::RiffSequence)) {
                        Ok(_) => (),
                        Err(error) => debug!("Failed to send riff arrangement add riff sequence: {}", error),
                    }
                }
            });
        }

        {
            let blade = riff_arrangement_blade.riff_arrangement_blade.clone();
            let tx_from_ui = tx_from_ui.clone();
            let riff_arrangements_box = riff_arrangements_box.clone();
            let riff_arrangements_combobox = riff_arrangements_combobox.clone();
            riff_arrangement_blade.riff_arrangement_blade_delete.connect_clicked(move |_| {
                let riff_arrangement_uuid = blade.widget_name().to_string();

                // delete riff arrangement blade from the UI
                if let Some(riff_arrangement_blade) = riff_arrangements_box.children().iter().find(|widget| widget.widget_name() == riff_arrangement_uuid) {
                    if let Some(active_id) = riff_arrangements_combobox.active_id() {
                        if active_id.to_string() == riff_arrangement_uuid {
                            if let Some(active_position) = riff_arrangements_combobox.active() {
                                // remove the riff arrangement from the arrangements_combobox
                                gtk::prelude::ComboBoxTextExt::remove(&riff_arrangements_combobox, active_position as i32);
                                // set the first riff arrangement in the arrangements_combobox as visible
                                if riff_arrangements_combobox.children().len() > 0 {
                                    riff_arrangements_combobox.set_active(Some(0));
                                }
                            }
                        }
                    }
                    // remove the riff arrangement blade from the riff_arrangement_box
                    riff_arrangements_box.remove(riff_arrangement_blade);
                }

                match tx_from_ui.send(DAWEvents::RiffArrangementDelete(riff_arrangement_uuid)) {
                    Ok(_) => (),
                    Err(error) => debug!("Failed to send riff arrangement delete riff arrangement: {}", error),
                }
            });
        }

        {
            let blade = riff_arrangement_blade.riff_arrangement_blade.clone();
            let tx_from_ui = tx_from_ui.clone();
            let riff_arrangements_box = riff_arrangements_box.clone();
            // let selected_track_style_provider = selected_track_style_provider.clone();
            riff_arrangement_blade.riff_arrangement_blade_play.connect_clicked(move |_| {
                let riff_arrangement_uuid = blade.widget_name().to_string();

                // for child in riff_arrangements_box.children() {
                //     child.style_context().remove_provider(&selected_track_style_provider);
                //     if child.widget_name() == riff_arrangement_uuid {
                //         child.style_context().add_provider(&selected_track_style_provider, gtk::STYLE_PROVIDER_PRIORITY_USER);
                //     }
                // }

                match tx_from_ui.send(DAWEvents::RiffArrangementPlay(riff_arrangement_uuid)) {
                    Ok(_) => (),
                    Err(error) => debug!("Failed to send riff arrangement play: {}", error),
                }
            });
        }

        {
            let blade = riff_arrangement_blade.riff_arrangement_blade.clone();
            let tx_from_ui = tx_from_ui.clone();
            let riff_arrangements_box = riff_arrangements_box.clone();
            let selected_track_style_provider = selected_track_style_provider.clone();
            riff_arrangement_blade.riff_arrangement_blade_copy.connect_clicked(move |_| {
                let riff_arrangement_uuid = blade.widget_name().to_string();

                for child in riff_arrangements_box.children() {
                    child.style_context().remove_provider(&selected_track_style_provider);
                    if child.widget_name() == riff_arrangement_uuid {
                        child.style_context().add_provider(&selected_track_style_provider, gtk::STYLE_PROVIDER_PRIORITY_USER);
                    }
                }

                match tx_from_ui.send(DAWEvents::RiffArrangementCopy(riff_arrangement_uuid)) {
                    Err(error) => debug!("Failed to send riff arrangements view copy selected riff arrangement event: {}", error),
                    _ => (),
                }
            });
        }

        {
            let blade = riff_arrangement_blade.riff_arrangement_blade.clone();
            let tx_from_ui = tx_from_ui;
            let riff_arrangements_box = riff_arrangements_box;
            let selected_track_style_provider = selected_track_style_provider;
            riff_arrangement_blade.riff_arrangement_copy_to_track_view_btn.connect_clicked(move |_| {
                let riff_arrangement_uuid = blade.widget_name().to_string();

                for child in riff_arrangements_box.children() {
                    child.style_context().remove_provider(&selected_track_style_provider);
                    if child.widget_name() == riff_arrangement_uuid {
                        child.style_context().add_provider(&selected_track_style_provider, gtk::STYLE_PROVIDER_PRIORITY_USER);
                    }
                }

                match tx_from_ui.send(DAWEvents::RiffArrangementCopySelectedToTrackViewCursorPosition(riff_arrangement_uuid)) {
                    Err(error) => debug!("Failed to send riff arrangements view copy selected riff arrangement contents to track view cursor position event: {}", error),
                    _ => (),
                }
            });
        }

        if !visible {
            riff_arrangement_blade.riff_arrangement_blade.hide();
        }

        riff_arrangement_blade
    }

    pub fn start(&self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        let css_provider = CssProvider::new();
        let freedom_daw_style = include_bytes!("daw_style.css");
        css_provider.load_from_data(freedom_daw_style).expect("Couldn't load CSS");
        gtk::StyleContext::add_provider_for_screen(
            &gdk::Screen::default().expect("Error adding css provider."),
            &css_provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        glib::set_application_name("DAW");
        self.ui.wnd_main.set_wmclass("DAW", "DAW");
        {
            self.ui.wnd_main.connect_delete_event(move |_, _| {
                match tx_from_ui.send(DAWEvents::Shutdown) {
                    Ok(_) => {}
                    Err(_) => {}
                }

                gtk::main_quit();
                Inhibit(false)
            });
        }
        self.ui.wnd_main.show();
    }

    pub fn add_riff_arrangement_riff_set_blade(
        &self,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        riff_arrangement_uuid: String,
        item_instance_uuid: String,
        riff_set_uuid: String,
        track_uuids: Vec<String>,
        selected_track_style_provider: CssProvider,
        riff_arrangement_vertical_adjustment: Adjustment,
        riff_set_name: String,
        state_arc: Arc<Mutex<DAWState>>,
    ) {
        // find the riff item box
        let mut riff_item_box = self.find_riff_arrangement_riff_item_box();

        // if there is a riff set box add a new blade to it and populate a map of beat grids
        if let Some(riff_item_box) = riff_item_box {
            let riff_item_beat_grids: Arc<Mutex<HashMap<String, HashMap<String, Arc<Mutex<BeatGrid>>>>>> = Arc::new(Mutex::new(HashMap::new()));
            let (riff_set_blade_head, riff_set_blade_drawing_areas, blade_box) = MainWindow::add_riff_set_blade(
                tx_from_ui.clone(),
                riff_item_box.clone(),
                riff_item_box.clone(),
                riff_set_uuid.to_string(),
                track_uuids,
                state_arc.clone(),
                riff_set_name,
                RiffSetType::RiffArrangement(riff_arrangement_uuid.clone()),
                selected_track_style_provider.clone(),
                Some(riff_item_beat_grids.clone()),
                item_instance_uuid.to_string(),
                Some(&riff_arrangement_vertical_adjustment),
            );

            Self::style_riff_arrangement_riff_set(&riff_set_blade_head, &riff_set_blade_drawing_areas);

            // move the new blade to the right position if there is a selection - find a selected blade if there is one and add it after that or just add it to the end of the list
            let mut selected_child_position = Self::get_selected_riff_item_position(&riff_item_box);
            if let Some(selected_child_position) = selected_child_position {
                riff_item_box.set_child_position(&blade_box, selected_child_position as i32 + 1);
            }

            // add the riff set(s) beat grids map to self.riff_arrangement_view_riff_set_ref_beat_grids
            self.add_riff_set_beat_grids_to_beat_grid_collection(riff_arrangement_uuid.clone(), riff_item_beat_grids);
        }
    }

    pub fn add_riff_arrangement_riff_sequence_blade(
        &self,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        riff_arrangement_uuid: String,
        item_instance_uuid: String,
        item_uuid: String,
        track_uuids: Vec<String>,
        selected_track_style_provider: CssProvider,
        riff_arrangement_vertical_adjustment: Adjustment,
        riff_sequence_name: String,
        state_arc: Arc<Mutex<DAWState>>,
        state: &DAWState,
    ) {
        // find the riff item box
        let mut riff_item_box = self.find_riff_arrangement_riff_item_box();

        // if there is a riff set box add a new blade to it and populate a map of beat grids
        if let Some(riff_item_box) = riff_item_box {
            let mut riff_sets = vec![];
            for riff_set in state.project().song().riff_sets().iter() {
                riff_sets.push((riff_set.uuid(), riff_set.name().to_string()));
            }

            let riff_sequence_blade = MainWindow::add_riff_sequence_blade(
                riff_item_box.clone(),
                tx_from_ui.clone(),
                state_arc.clone(),
                Some(riff_sets),
                Some(item_instance_uuid.to_string()),
                Some(item_uuid.to_string()),
                false,
                RiffSequenceType::RiffArrangement(riff_arrangement_uuid.clone()),
                selected_track_style_provider.clone(),
                Some(&riff_arrangement_vertical_adjustment),
            );

            // move the new blade to the right position if there is a selection - find a selected blade if there is one and add it after that or just add it to the end of the list
            let mut selected_child_position = Self::get_selected_riff_item_position(&riff_item_box);
            if let Some(selected_child_position) = selected_child_position {
                riff_item_box.set_child_position(&riff_sequence_blade.riff_sequence_blade, selected_child_position as i32 + 1);
            }

            let mut riff_sequence_blade_width = 0;
            let mut riff_item_beat_grids  = Arc::new(Mutex::new(HashMap::new()));
            if let Some(riff_sequence) = state.project().song().riff_sequences().iter().find(|current_riff_sequence| current_riff_sequence.uuid() == item_uuid.to_string()) {
                riff_sequence_blade.riff_sequence_name_entry.set_text(riff_sequence.name());

                for riff_set_reference in riff_sequence.riff_sets().iter() {
                    if let Some(riff_set) = state.project().song().riff_sets().iter().find(|current_riff_set| current_riff_set.uuid() == riff_set_reference.item_uuid().to_string()) {
                        let (riff_set_blade_head, riff_set_blade_drawing_areas, _) = MainWindow::add_riff_set_blade(
                            tx_from_ui.clone(),
                            riff_sequence_blade.riff_set_box.clone(),
                            riff_sequence_blade.riff_set_head_box.clone(),
                            riff_set_reference.item_uuid().to_string(),
                            track_uuids.clone(),
                            state_arc.clone(),
                            riff_set.name().to_string(),
                            RiffSetType::RiffSequence(riff_sequence.uuid()),
                            selected_track_style_provider.clone(),
                            Some(riff_item_beat_grids.clone()),
                            riff_set_reference.uuid(),
                            None,
                        );

                        Self::style_riff_arrangement_riff_seq_riff_set(&riff_set_blade_head);

                        riff_sequence_blade_width = riff_sequence_blade_width + 75; // FIXME magic number - 69 should be a constant plus the right/left margin
                    }
                }
            }

            &riff_sequence_blade.riff_sequence_blade.set_width_request(riff_sequence_blade_width);
            self.add_riff_set_beat_grids_to_beat_grid_collection(riff_arrangement_uuid.clone(), riff_item_beat_grids);
        }
    }
    pub fn style_riff_arrangement_riff_set(riff_set_blade_head: &RiffSetBladeHead, riff_set_blade_drawing_areas: &RiffSetBlade) {
        riff_set_blade_head.riff_set_blade.set_margin_top(15);
        riff_set_blade_head.riff_set_blade.set_margin_bottom(13);
        riff_set_blade_head.riff_set_blade.set_height_request(riff_set_blade_head.riff_set_blade.height_request() - 20);
        riff_set_blade_head.riff_set_blade.set_width_request(69);
        riff_set_blade_drawing_areas.riff_set_box.set_width_request(69);
    }

    pub fn style_riff_arrangement_riff_seq_riff_set(riff_set_blade_head: &RiffSetBladeHead) {
        riff_set_blade_head.riff_set_blade.set_margin_bottom(50);
        riff_set_blade_head.riff_set_blade_play.hide();
        riff_set_blade_head.riff_set_select_btn.hide();
        riff_set_blade_head.riff_set_blade_delete.hide();
        riff_set_blade_head.riff_set_drag_btn.hide();
    }

    pub fn get_selected_riff_item_position(riff_item_box: &Box) -> Option<usize> {
        for (index, child) in riff_item_box.children().iter().enumerate() {
            let actual_child = if let Some(local_riff_set_box) = child.dynamic_cast_ref::<Box>() {
                // look for the riff set head if this is a riff set otherwise it is a riff sequence
                if let Some(riff_set_head_box_widget) = local_riff_set_box.children().get(1) {
                    if let Some(riff_set_head_box) = riff_set_head_box_widget.dynamic_cast_ref::<Box>() {
                        riff_set_head_box.children().get(0).unwrap().clone()
                    } else {
                        child.clone()
                    }
                } else {
                    child.clone()
                }
            } else {
                child.clone()
            };
            unsafe {
                if let Some(selected) = actual_child.data::<u32>("selected") {
                    if *(selected.cast::<u32>().as_ptr()) == 1 {
                        return Some(index);
                    }
                }
            }
        }

        None
    }

    pub fn add_riff_set_beat_grids_to_beat_grid_collection(&self, riff_arrangement_uuid: String, riff_item_beat_grids:  Arc<Mutex<HashMap<String, HashMap<String, Arc<Mutex<BeatGrid>>>>>>) {
        if let Ok(mut riff_arrangement_view_riff_set_ref_beat_grids) = self.riff_arrangement_view_riff_set_ref_beat_grids.lock() {
            // get the riff arr beat grids
            let riff_set_beat_grids = if let Some(riff_arrangement_riff_set_ref_beat_grids) = riff_arrangement_view_riff_set_ref_beat_grids.get(riff_arrangement_uuid.as_str()) {
                riff_arrangement_riff_set_ref_beat_grids.clone()
            } else {
                // Arc<Mutex<HashMap<String, HashMap<String, Arc<Mutex<BeatGrid>>>>>>
                let data = Arc::new(Mutex::new(HashMap::new()));
                riff_arrangement_view_riff_set_ref_beat_grids.insert(riff_arrangement_uuid, data.clone());
                data
            };

            // add the new stuff to it
            if let Ok(mut riff_item_beat_grids) = riff_item_beat_grids.lock() {
                if let Ok(mut riff_set_beat_grids) = riff_set_beat_grids.lock() {
                    riff_set_beat_grids.extend(riff_item_beat_grids.drain());
                }
            }
        }
    }

    pub fn find_riff_arrangement_riff_item_box(
        &self
    ) -> Option<Box> {
        if let Some(active_riff_arrangement_index) = self.ui.arrangements_combobox.active() {
            if let Some(riff_arrangement_blade_widget) = self.ui.riff_arrangement_box.children().get(active_riff_arrangement_index as usize) {
                if let Some(riff_arrangement_blade) = riff_arrangement_blade_widget.dynamic_cast_ref::<Frame>() {
                    // navigate down to the riff_set_box child of a child of a child... widget
                    'outer:
                    for widget in riff_arrangement_blade.children().iter() {
                        if let Some(top_level_box) = widget.dynamic_cast_ref::<Box>() {
                            for widget in top_level_box.children().iter() {
                                if widget.widget_name().to_string() == "riff_arrangement_riff_items_scrolled_window" {
                                    if let Some(riff_items_scrolled_window) = widget.dynamic_cast_ref::<ScrolledWindow>() {
                                        if let Some(widget) = riff_items_scrolled_window.child() {
                                            if let Some(view_port) = widget.dynamic_cast_ref::<Viewport>() {
                                                if let Some(riff_sets_box) = view_port.child() {
                                                    if let Some(riff_sets_box) = riff_sets_box.dynamic_cast_ref::<Box>() {
                                                        return Some(riff_sets_box.clone());
                                                        break 'outer;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        None
    }

    /// Set the main window's piano roll grid.
    pub fn set_piano_roll_grid(&mut self, piano_roll_grid: Option<Arc<Mutex<BeatGrid>>>) {
        self.piano_roll_grid = piano_roll_grid;
    }

    /// Set the main window's piano roll grid ruler.
    pub fn set_piano_roll_grid_ruler(&mut self, piano_roll_grid_ruler: Option<Arc<Mutex<BeatGridRuler>>>) {
        self.piano_roll_grid_ruler = piano_roll_grid_ruler;
    }

    /// Set the main window's sample roll grid.
    pub fn set_sample_roll_grid(&mut self, sample_roll_grid: Option<Arc<Mutex<BeatGrid>>>) {
        self.sample_roll_grid = sample_roll_grid;
    }

    /// Set the main window's piano roll grid ruler.
    pub fn set_sample_roll_grid_ruler(&mut self, sample_roll_grid_ruler: Option<Arc<Mutex<BeatGridRuler>>>) {
        self.sample_roll_grid_ruler = sample_roll_grid_ruler;
    }

    /// Set the main window's track grid.
    pub fn set_track_grid(&mut self, track_grid: Option<Arc<Mutex<BeatGrid>>>) {
        self.track_grid = track_grid;
    }

    /// Set the main window's track grid ruler.
    pub fn set_track_grid_ruler(&mut self, track_grid_ruler: Option<Arc<Mutex<BeatGridRuler>>>) {
        self.track_grid_ruler = track_grid_ruler;
    }

    /// Set the main window's automation grid.
    pub fn set_automation_grid(&mut self, automation_grid: Option<Arc<Mutex<BeatGrid>>>) {
        self.automation_grid = automation_grid;
    }

    /// Set the main window's controller grid ruler.
    pub fn set_automation_grid_ruler(&mut self, automation_grid_ruler: Option<Arc<Mutex<BeatGridRuler>>>) {
        self.automation_grid_ruler = automation_grid_ruler;
    }

    pub fn setup_transport(
        main_window: &mut MainWindow,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>) {
        {
            let tx_from_ui = tx_from_ui.clone();
            main_window.ui.transport_goto_start_button.connect_clicked(move |button| {
                if button.is_active() && button.is_active() {
                    match tx_from_ui.send(DAWEvents::TransportGotoStart) {
                        Ok(_) => (),
                        Err(error) => debug!("{:?}", error),
                    }
                }
            });
        }
        {
            let tx_from_ui = tx_from_ui.clone();
            main_window.ui.transport_move_back_button.connect_clicked(move |button| {
                if button.is_active() && button.is_active() {
                    match tx_from_ui.send(DAWEvents::TransportMoveBack) {
                        Ok(_) => (),
                        Err(error) => debug!("{:?}", error),
                    }
                }
            });
        }
        {
            let tx_from_ui = tx_from_ui.clone();
            main_window.ui.transport_stop_button.connect_clicked(move |button| {
                if button.is_active() && button.is_active() {
                    match tx_from_ui.send(DAWEvents::TransportStop) {
                        Ok(_) => (),
                        Err(error) => debug!("{:?}", error),
                    }
                }
            });
        }
        {
            let tx_from_ui = tx_from_ui.clone();
            main_window.ui.transport_play_button.connect_clicked(move |button| {
                if button.is_active() {
                    match tx_from_ui.send(DAWEvents::TransportPlay) {
                        Ok(_) => (),
                        Err(error) => debug!("{:?}", error),
                    }
                }
            });
        }
        {
            let tx_from_ui = tx_from_ui.clone();
            main_window.ui.transport_record_button.connect_clicked(move |button| {
                if button.is_active() {
                    match tx_from_ui.send(DAWEvents::TransportRecordOn) {
                        Ok(_) => (),
                        Err(error) => debug!("{:?}", error),
                    }
                }
                else {
                    match tx_from_ui.send(DAWEvents::TransportRecordOff) {
                        Ok(_) => (),
                        Err(error) => debug!("{:?}", error),
                    }
                }
            });
        }
        {
            let tx_from_ui = tx_from_ui.clone();
            main_window.ui.transport_loop_button.connect_clicked(move |button| {
                let loop_change = if button.is_active() {
                    LoopChangeType::LoopOn
                }
                else {
                    LoopChangeType::LoopOff
                };
                match tx_from_ui.send(DAWEvents::LoopChange(loop_change, Uuid::new_v4())) {
                    Ok(_) => (),
                    Err(error) => debug!("Problem sending loop on/off change: {:?}", error),
                }
            });
        }
        {
            let tx_from_ui = tx_from_ui.clone();
            main_window.ui.transport_pause_button.connect_clicked(move |button| {
                if button.is_active() && button.is_active() {
                    match tx_from_ui.send(DAWEvents::TransportPause) {
                        Ok(_) => (),
                        Err(error) => debug!("{:?}", error),
                    }
                }
            });
        }
        {
            let tx_from_ui = tx_from_ui.clone();
            main_window.ui.transport_move_forward_button.connect_clicked(move |button| {
                if button.is_active() && button.is_active() {
                    match tx_from_ui.send(DAWEvents::TransportMoveForward) {
                        Ok(_) => (),
                        Err(error) => debug!("{:?}", error),
                    }
                }
            });
        }
        {
            let tx_from_ui = tx_from_ui;
            main_window.ui.transport_goto_end_button.connect_clicked(move |button| {
                if button.is_active() && button.is_active() {
                    match tx_from_ui.send(DAWEvents::TransportGotoEnd) {
                        Ok(_) => (),
                        Err(error) => debug!("{:?}", error),
                    }
                }
            });
        }
    }

    pub fn update_ui_from_state(&mut self, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, state: &mut DAWState, state_arc: Arc<Mutex<DAWState>>) {
        let midi_input_devices: Vec<String> = state.midi_devices();
        let mut instrument_plugins: IndexMap<String, String> = IndexMap::new();

        for (key, value) in state.vst_instrument_plugins().iter() {
            instrument_plugins.insert(key.clone(), value.clone());
        }

        let project = state.get_project();
        let song = project.song_mut();
        let mut track_number = 0;


        self.ui.song_tempo_spinner.set_value(song.tempo());

        debug!("main_window.update_ui_from_state() start - number of riff sequences={}", song.riff_sequences_mut().len());

        let mut track_details = HashMap::new();
        for track in song.tracks_mut() {
            track_details.insert(track.uuid().to_string(), track.name_mut().to_string());
        }

        for track in song.tracks_mut() {
            match track {
                TrackType::InstrumentTrack(track) => {
                    let tx_ui = tx_from_ui.clone();
                    self.add_track(
                        track.name(),
                        track.uuid(),
                        tx_ui,
                        state_arc.clone(),
                        GeneralTrackType::InstrumentTrack,
                        None,
                        track.volume(),
                        track.pan(),
                        track.mute(),
                        track.solo()
                    );
                }
                TrackType::AudioTrack(track) => {
                    let tx_ui = tx_from_ui.clone();
                    self.add_track(
                        track.name(),
                        track.uuid(),
                        tx_ui,
                        state_arc.clone(),
                        GeneralTrackType::AudioTrack,
                        None,
                        track.volume(),
                        track.pan(),
                        track.mute(),
                        track.solo()
                    );
                }
                TrackType::MidiTrack(track) => {
                    let tx_ui = tx_from_ui.clone();
                    self.add_track(
                        track.name(),
                        track.uuid(),
                        tx_ui,
                        state_arc.clone(),
                        GeneralTrackType::MidiTrack,
                        None,
                        track.volume(),
                        track.pan(),
                        track.mute(),
                        track.solo()
                    );
                }
            }
            self.update_track_details_dialogue(&midi_input_devices, &mut instrument_plugins, &mut track_number, &track);
            match self.track_midi_routing_dialogues.get_mut(&track.uuid_string()) {
                Some(midi_routing_dialogue) => {
                    for route in track.midi_routings().iter() {
                        let track_midi_routing_panel_glade_src = include_str!("track_midi_routing_panel.glade");
                        let track_midi_routing_panel: TrackMidiRoutingPanel = TrackMidiRoutingPanel::from_string(track_midi_routing_panel_glade_src).unwrap();
                        let track_midi_routing_scrolled_box = midi_routing_dialogue.track_midi_routing_scrolled_box.clone();

                        track_midi_routing_scrolled_box.add(&track_midi_routing_panel.track_midi_routing_panel);
                        Self::setup_track_midi_routing_panel(track_midi_routing_panel, route.clone(), route.description.clone(), tx_from_ui.clone(), track_midi_routing_scrolled_box, track.uuid())
                    }
                }
                None => {}
            }
            match self.track_audio_routing_dialogues.get_mut(&track.uuid_string()) {
                Some(audio_routing_dialogue) => {
                    for route in track.audio_routings().iter() {
                        let track_audio_routing_panel_glade_src = include_str!("track_audio_routing_panel.glade");
                        let track_audio_routing_panel: TrackAudioRoutingPanel = TrackAudioRoutingPanel::from_string(track_audio_routing_panel_glade_src).unwrap();
                        let track_audio_routing_scrolled_box = audio_routing_dialogue.track_audio_routing_scrolled_box.clone();

                        track_audio_routing_scrolled_box.add(&track_audio_routing_panel.track_audio_routing_panel);
                        Self::setup_track_audio_routing_panel(track_audio_routing_panel, route.clone(), route.description.clone(), tx_from_ui.clone(), track_audio_routing_scrolled_box, track.uuid())
                    }
                }
                None => {}
            }
            track_number += 1;
        }

        // clear and add the loops
        let loop_combo = self.ui.loop_combobox_text.clone();
        let mut first = true;
        loop_combo.remove_all();
        song.loops().iter().for_each(move |current_loop| {
            loop_combo.append(Some(current_loop.uuid().to_string().as_str()), current_loop.name());
            if first {
                loop_combo.set_active_id(Some(current_loop.uuid().to_string().as_str()));
                first = false;
            }
        });

        // update the controller view
        // get the selected track
        let plugins_params = state.audio_plugin_parameters();
        if let Some(track_uuid) = state.selected_track() {
            if let Some(track_type) = state.project().song().tracks().iter().find(|track_type| {
                match track_type {
                    TrackType::InstrumentTrack(track) => track.uuid().to_string() == track_uuid,
                    TrackType::AudioTrack(_) => false,
                    TrackType::MidiTrack(_) => false,
                }
            }) {
                match track_type {
                    TrackType::InstrumentTrack(track) => {
                        // update the controller view instrument params list based on the selected track
                        self.ui.automation_instrument_parameters_combobox.remove_all();
                        let instrument_uuid = track.instrument().uuid().to_string();
                        let key = track.uuid().to_string();
                        if let Some(track_plugins) = plugins_params.get(&key) {
                            if let Some(plugin_params) = track_plugins.get(&instrument_uuid) {
                                debug!("instrument plugin param count={}", plugin_params.len());
                                plugin_params.iter().for_each(|param| {
                                    self.ui.automation_instrument_parameters_combobox.append(Some(param.index.to_string().as_str()), param.name());
                                });
                            }
                        }

                        // update the controller view effect list based on the selected track
                        self.ui.automation_effects_combobox.remove_all();
                        for effect in track.effects() {
                            let effect_uuid = effect.uuid().to_string();
                            self.ui.automation_effects_combobox.append(Some(effect_uuid.as_str()), effect.name());

                            if let Some(selected_effect_uuid) = state.selected_effect_plugin_uuid() {
                                if *selected_effect_uuid == effect_uuid {
                                    if let Some(signal_handler_id) = &self.automation_effects_choice_signal_handler_id {
                                        self.ui.automation_effects_combobox.block_signal(signal_handler_id);
                                        self.ui.automation_effects_combobox.set_active_id(Some(selected_effect_uuid));
                                        self.ui.automation_effects_combobox.unblock_signal(signal_handler_id);
                                    }

                                    // update the effect parameters based on the selected effect
                                    self.ui.automation_effect_parameters_combobox.remove_all();
                                    if let Some(track_plugins) = plugins_params.get(&key) {
                                        if let Some(plugin_params) = track_plugins.get(effect_uuid.as_str()) {
                                            debug!("effect plugin param count={}", plugin_params.len());
                                            plugin_params.iter().for_each(|param| {
                                                self.ui.automation_effect_parameters_combobox.append(Some(param.index.to_string().as_str()), param.name());
                                            });
                                        }
                                    }
                                }
                            }
                        }

                    },
                    TrackType::AudioTrack(_) => (),
                    TrackType::MidiTrack(_) => (),
                }
            }
        }

        if let Some(track_grid) = self.track_grid() {
            if let Ok(track) = track_grid.lock() {
                let mut grid = track;
                grid.set_tempo(state.project().song().tempo());
            }
        }
        if let Some(piano_roll_grid) = self.piano_roll_grid() {
            if let Ok(piano_roll) = piano_roll_grid.lock() {
                let mut grid = piano_roll;
                grid.set_tempo(state.project().song().tempo());
            }
        }
        if let Some(automation_grid) = self.automation_grid() {
            if let Ok(controllers) = automation_grid.lock() {
                let mut grid = controllers;
                grid.set_tempo(state.project().song().tempo());
            }
        }

        // populate the sample roll available samples list
        if let Some(model) = self.ui.sample_roll_available_samples.model() {
            if let Some(available_samples_list_store) =  model.dynamic_cast_ref::<ListStore>() {
                available_samples_list_store.clear();
                for (sample_uuid, sample) in state.project().song().samples().iter() {
                    available_samples_list_store.insert_with_values(None, &[
                        (0, &sample.name().to_string()),
                        (1, &sample_uuid),
                    ]);
                    self.ui.sample_roll_available_samples.show_all();
                }
            }
        }

        // collect the track uuids
        let mut track_uuids = Self::collect_track_uuids(state);

        self.update_riff_sets(&tx_from_ui, state, &state_arc, &mut track_uuids);
        self.update_riff_sequences(&tx_from_ui, state, &state_arc, &mut track_uuids, false);
        self.update_riff_arrangements(tx_from_ui, state, state_arc, track_uuids, false);
        self.update_available_audio_plugins_in_ui(state.vst_instrument_plugins(), state.vst_effect_plugins());

        debug!("main_window.update_ui_from_state() end - number of riff sequences={}", state.project().song().riff_sequences().len());
    }

    pub fn collect_track_uuids(state: &mut DAWState) -> Vec<String> {
        let mut track_uuids = vec![];
        for track in state.project().song().tracks().iter() {
            track_uuids.push(track.uuid().to_string());
        }
        track_uuids
    }

    pub fn update_riff_arrangements(
        &mut self,
        tx_from_ui: Sender<DAWEvents>,
        state: &mut DAWState,
        state_arc: Arc<Mutex<DAWState>>,
        mut track_uuids: Vec<String>,
        restore_selected: bool
    ) {
        let selected_index = self.ui.arrangements_combobox.active();

        // remove the current riff arrangements
        for widget in self.ui.riff_arrangement_box.children().iter() {
            self.ui.riff_arrangement_box.remove(widget);
        }

        if let Ok(mut riff_arrangement_view_riff_set_ref_beat_grids) = self.riff_arrangement_view_riff_set_ref_beat_grids.lock() {
            riff_arrangement_view_riff_set_ref_beat_grids.clear();
        }

        // clear the arrangements combo
        self.ui.arrangements_combobox.remove_all();

        // populate the arrangements
        let mut first = true;
        for riff_arrangement in state.project().song().riff_arrangements().iter() {
            self.ui.arrangements_combobox.append(Some(riff_arrangement.uuid().as_str()), riff_arrangement.name());

            // populate the riff arrangements
            let riff_sets: Vec<(String, String)> = state.project().song().riff_sets().iter().map(|riff_set| (riff_set.uuid(), riff_set.name().to_string())).collect();
            let riff_sequences: Vec<(String, String)> = state.project().song().riff_sequences().iter().map(|riff_sequence| (riff_sequence.uuid(), riff_sequence.name().to_string())).collect();
            let mut riff_item_beat_grids  = Arc::new(Mutex::new(HashMap::new()));
            self.setup_riff_arrangement(
                riff_arrangement,
                self.ui.clone(),
                tx_from_ui.clone(),
                state,
                state_arc.clone(),
                track_uuids.clone(),
                riff_sets,
                riff_sequences,
                first,
                self.selected_style_provider.clone(),
                riff_item_beat_grids.clone()
            );

            if first {
                self.ui.arrangements_combobox.set_active_id(Some(riff_arrangement.uuid().as_str()));
                first = false;
            }

            if let Ok(mut riff_arrangement_view_riff_set_ref_beat_grids) = self.riff_arrangement_view_riff_set_ref_beat_grids.lock() {
                riff_arrangement_view_riff_set_ref_beat_grids.insert(riff_arrangement.uuid(), riff_item_beat_grids);
            }
        }

        if restore_selected {
            self.ui.arrangements_combobox.set_active(selected_index);
        }
    }

    pub fn update_riff_sequences(
        &mut self,
        tx_from_ui: &Sender<DAWEvents>,
        state: &mut DAWState,
        state_arc: &Arc<Mutex<DAWState>>,
        mut track_uuids: &mut Vec<String>,
        restore_selected: bool
    ) {
        let selected_index = self.ui.sequence_combobox.active();

        // remove the current riff sequences
        for widget in self.ui.riff_sequences_box.children().iter() {
            self.ui.riff_sequences_box.remove(widget);
        }

        if let Ok(mut riff_sequence_view_riff_set_ref_beat_grids) = self.riff_sequence_view_riff_set_ref_beat_grids.lock() {
            riff_sequence_view_riff_set_ref_beat_grids.clear();
        }

        // clear the sequences combo
        self.ui.sequence_combobox.remove_all();

        let riff_sets: Vec<(String, String)> = state.project().song().riff_sets().iter().map(|riff_set| (riff_set.uuid(), riff_set.name().to_string())).collect();
        let mut first = true;
        for riff_sequence in state.project().song().riff_sequences().iter() {
            let riff_sequence_blade = MainWindow::add_riff_sequence_blade(
                self.ui.riff_sequences_box.clone(),
                tx_from_ui.clone(),
                state_arc.clone(),
                Some(riff_sets.clone()),
                Some(riff_sequence.uuid()),
                None,
                false,
                RiffSequenceType::RiffSequence,
                self.selected_style_provider.clone(),
                Some(&self.ui.riff_sequence_vertical_adjustment),
            );

            riff_sequence_blade.riff_sequence_blade.hide();

            self.ui.sequence_combobox.append(Some(riff_sequence.uuid().as_str()), riff_sequence.name());

            if !restore_selected && first {
                self.ui.sequence_combobox.set_active_id(Some(riff_sequence.uuid().as_str()));
                first = false;
            }

            // set the name
            riff_sequence_blade.riff_sequence_name_entry.set_text(riff_sequence.name());

            // add the sequence to the sequences data
            // state.
            let mut riff_sequence_riff_set_beat_grids = Arc::new(Mutex::new(HashMap::new()));
            for riff_set_reference in riff_sequence.riff_sets().iter() {
                let (_, riff_set_blade, _) = MainWindow::add_riff_set_blade(
                    tx_from_ui.clone(),
                    riff_sequence_blade.riff_set_box.clone(),
                    riff_sequence_blade.riff_set_head_box.clone(),
                    riff_set_reference.item_uuid().to_string(),
                    track_uuids.clone(),
                    state_arc.clone(),
                    riff_sets.iter().find(|riff_set_details_tuple| riff_set_details_tuple.0 == riff_set_reference.item_uuid().to_string()).unwrap().1.clone(),
                    RiffSetType::RiffSequence(riff_sequence.uuid()),
                    self.selected_style_provider.clone(),
                    Some(riff_sequence_riff_set_beat_grids.clone()),
                    riff_set_reference.uuid(),
                    None,
                );
            }

            if let Ok(mut riff_sequence_view_riff_set_ref_beat_grids) = self.riff_sequence_view_riff_set_ref_beat_grids.lock() {
                riff_sequence_view_riff_set_ref_beat_grids.insert(riff_sequence.uuid(), riff_sequence_riff_set_beat_grids);
            }
        }

        if restore_selected {
            self.ui.sequence_combobox.set_active(selected_index);
        }
    }

    pub fn update_riff_sets(&mut self, tx_from_ui: &Sender<DAWEvents>, state: &mut DAWState, state_arc: &Arc<Mutex<DAWState>>, mut track_uuids: &mut Vec<String>) {
        // remove the current riff sets
        for widget in self.ui.riff_sets_box.children().iter() {
            self.ui.riff_sets_box.remove(widget);
        }

        if let Ok(mut riff_set_view_riff_set_beat_grids) = self.riff_set_view_riff_set_beat_grids.lock() {
            riff_set_view_riff_set_beat_grids.clear();
        }

        // populate the riff sets
        for riff_set in state.project().song().riff_sets().iter() {
            MainWindow::add_riff_set_blade(
                tx_from_ui.clone(),
                self.ui.riff_sets_box.clone(),
                self.ui.riff_set_heads_box.clone(),
                riff_set.uuid(),
                track_uuids.clone(),
                state_arc.clone(),
                riff_set.name().to_owned(),
                RiffSetType::RiffSet,
                self.selected_style_provider.clone(),
                Some(self.riff_set_view_riff_set_beat_grids.clone()),
                "".to_string(),
                None,
            );
        }
    }

    pub fn update_track_details_dialogue(&mut self, midi_input_devices: &Vec<String>, instrument_plugins: &mut IndexMap<String, String>, track_number: &mut i32, track: &&mut TrackType) {
        let track_uuid = track.uuid().to_string();
        
        match self.track_details_dialogues.get_mut(&track_uuid) {
            Some(track_details_dialogue) => {
                // clear and add the riffs
                let track_riff_choice = track_details_dialogue.track_riff_choice.clone();
                track_riff_choice.remove_all();
                let mut first_riff = true;
                for riff in track.riffs().iter() {
                    if riff.name() != "empty" {
                        let uuid = riff.uuid().to_string();
                        track_riff_choice.append(Some(uuid.as_str()), riff.name());

                        if first_riff {
                            track_riff_choice.set_active_id(Some(uuid.as_str()));
                            first_riff = false;
                        }
                    }
                }

                match track {
                    TrackType::InstrumentTrack(track) => {
                        // select the instrument
                        let track_instrument_choice = track_details_dialogue.track_instrument_choice.clone();
                        let mut instrument_id = track.instrument().file().to_string();
                        instrument_id.push(':');
                        if let Some(sub_plugin_id) = track.instrument().sub_plugin_id() {
                            instrument_id.push_str(sub_plugin_id.as_str());
                        }
                        instrument_id.push(':');
                        instrument_id.push_str(track.instrument().plugin_type());

                        // re-populate the track instrument choice
                        track_instrument_choice.remove_all();
                        for (key, value) in instrument_plugins.iter() {
                            let adjusted_key = key.replace(char::from(0), "");
                            let adjusted_value = value.replace(char::from(0), "");
                            // debug!("Add instrument to choice: key={}, value={}", adjusted_key.as_str(), adjusted_value.as_str());
                            track_instrument_choice.append(Some(adjusted_key.as_str()), adjusted_value.as_str());
                        }

                        // set the active element without generating an event
                        if instrument_id.ends_with(".so") || instrument_id.contains(".so:") || instrument_id.ends_with(".clap") || instrument_id.contains(".clap:") {
                            if let Some(signal_handler_id) = self.track_details_dialogue_track_instrument_choice_signal_handlers.get(&track_uuid) {
                                track_instrument_choice.block_signal(signal_handler_id);

                                if !track_instrument_choice.set_active_id(Some(instrument_id.as_str())) {
                                    debug!("failed to set the active id for: track={}, instrument={} taking the shell plugin id off and trying again", track_number, instrument_id.as_str());
                                    let adjusted_instrument_id = instrument_id.replace(char::from(0), "");
                                    let tokens: Vec<&str> = adjusted_instrument_id.split(':').collect();

                                    if tokens.len() == 2 {
                                        let library_file_only = tokens.first().unwrap().to_string();

                                        if !track_instrument_choice.set_active_id(Some(library_file_only.as_str())) {
                                            debug!("Fallback failed to set the active id for: track={}, instrument={}", track_number, library_file_only.as_str());
                                        }
                                    }
                                }

                                track_instrument_choice.unblock_signal(signal_handler_id);
                                track_instrument_choice.show_all();
                                track_instrument_choice.activate();
                            }
                        }

                        // clear and add the effects
                        let track_effects_list = track_details_dialogue.track_effect_list.clone();
                        if let Some(track_effects_list_store) = track_effects_list.model() {
                            if let Some(model) = track_effects_list_store.dynamic_cast_ref::<ListStore>() {
                                model.clear();
                                for effect in track.effects().iter() {
                                    model.insert_with_values(None, &[
                                        (0, &effect.name().to_string()),
                                        (1, &effect.file().to_string()),
                                        (2, &effect.uuid().to_string()),
                                        (3, &(RGBA::black())),
                                        (4, &(RGBA::white())),
                                    ]);
                                }
                                track_effects_list.set_model(Some(model));
                            }
                        }
                    },
                    TrackType::AudioTrack(_track) => {},
                    TrackType::MidiTrack(track) => {
                        track_details_dialogue.track_midi_channel_choice.set_active_id(Some(track.midi_device().midi_channel().to_string().as_str()));

                        // populate the possible midi connections
                        track_details_dialogue.track_midi_device_choice.remove_all();
                        for midi_in_port in midi_input_devices.iter() {
                            track_details_dialogue.track_midi_device_choice.append(Some(midi_in_port.as_str()), midi_in_port.as_str());
                        }

                        track_details_dialogue.track_midi_device_choice.set_active_id(Some(track.midi_device().name()));
                    },
                };
            }
            None => (),
        }
    }

    pub fn update_available_audio_plugins_in_ui(&self, instrument_plugins: &IndexMap<String, String>, effect_plugins: &IndexMap<String, String>) {
        self.track_details_dialogues.iter().for_each(|(track_uuid, panel)| {
            let active_instrument_id = panel.track_instrument_choice.active_id();
            panel.track_instrument_choice.remove_all();
            for (key, value) in instrument_plugins.iter() {
                let adjusted_key = key.replace(char::from(0), "");
                let adjusted_value = value.replace(char::from(0), "");
                // debug!("Add instrument to choice: key={}, value={}", adjusted_key.as_str(), adjusted_value.as_str());
                panel.track_instrument_choice.append(Some(adjusted_key.as_str()), adjusted_value.as_str());
            }
            if let Some(active_instrument_id) = active_instrument_id {
                if let Some(signal_handler_id) = self.track_details_dialogue_track_instrument_choice_signal_handlers.get(track_uuid) {
                    panel.track_instrument_choice.block_signal(signal_handler_id);
                    if !panel.track_instrument_choice.set_active_id(Some(active_instrument_id.as_str())) {
                        debug!("Failed to set the active id for: track={}, instrument={}", track_uuid, active_instrument_id.as_str());
                    }
                    panel.track_instrument_choice.unblock_signal(signal_handler_id);
                    panel.track_instrument_choice.show_all();
                    panel.track_instrument_choice.activate();
                }
            }

            panel.track_effects_choice.remove_all();
            for (key, value) in effect_plugins.iter() {
                panel.track_effects_choice.append(Some(key.replace(char::from(0), "").as_str()), value.replace(char::from(0), "").as_str());
            }
        });
    }

    pub fn update_riff_set_name_in_riff_views(
        &mut self,
        riff_set_uuid: String,
        riff_set_name: String,
    ) {
        // update the riff sets - name change originated in a different view
        let riff_set_heads_box = self.ui.riff_set_heads_box.clone();
        self.update_riff_set_name_in_riff_set_head_box(riff_set_uuid.clone(), riff_set_name.clone(), &riff_set_heads_box);

        // update the riff seq blades - riff sets combobox
        for riff_sequences_box_child in self.ui.riff_sequences_box.children().iter() {
            if let Some(riff_seq_blade_frame) = riff_sequences_box_child.dynamic_cast_ref::<Frame>() {
                if let Some(riff_seq_blade_frame_child) = riff_seq_blade_frame.child() {
                    if let Some(top_level_box) = riff_seq_blade_frame_child.dynamic_cast_ref::<Box>() {
                        if let Some(riff_set_blade_box_widget) = top_level_box.children().get_mut(1) {
                            if let Some(riff_set_blade_box) = riff_set_blade_box_widget.dynamic_cast_ref::<Box>() {
                                if let Some(riff_set_head_scrolled_window_widget) = riff_set_blade_box.children().get_mut(0) {
                                    if let Some(riff_set_head_scrolled_window) = riff_set_head_scrolled_window_widget.dynamic_cast_ref::<ScrolledWindow>() {
                                        if let Some(scrolled_window_viewport_widget) = riff_set_head_scrolled_window.child() {
                                            if let Some(viewport) = scrolled_window_viewport_widget.dynamic_cast_ref::<Viewport>() {
                                                if let Some(riff_set_head_box_widget) = viewport.child() {
                                                    if let Some(riff_set_head_box) = riff_set_head_box_widget.dynamic_cast_ref::<Box>() {
                                                        self.update_riff_set_name_in_riff_set_head_box(riff_set_uuid.clone(), riff_set_name.clone(), riff_set_head_box);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn update_riff_set_name_in_riff_set_head_box(
        &mut self,
        riff_set_uuid: String,
        riff_set_name: String,
        riff_set_head_box: &Box,
    ) {
        for riff_set_head_widget in riff_set_head_box.children() {
            if riff_set_head_widget.widget_name() == riff_set_uuid {
                if let Some(riff_set_head) = riff_set_head_widget.dynamic_cast_ref::<Frame>() {
                    if let Some(riff_set_blade_box_widget) = riff_set_head.child() {
                        if let Some(riff_set_blade_box) = riff_set_blade_box_widget.dynamic_cast_ref::<Box>() {
                            if let Some(grid_widget) = riff_set_blade_box.children().first() {
                                if let Some(grid) = grid_widget.dynamic_cast_ref::<Grid>() {
                                    for widget in grid.children().iter_mut() {
                                        if let Some(name_entry) = widget.dynamic_cast_ref::<Entry>() {
                                            if name_entry.text().to_string() != riff_set_name {
                                                name_entry.set_text(riff_set_name.as_str());
                                            }
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn update_available_riff_sets(
        &mut self,
        state: &DAWState,
    ) {
        // get the available riff sets
        let riff_sets: Vec<(String, String)> = state.project().song().riff_sets().iter().map(|riff_set| (riff_set.uuid(), riff_set.name().to_string())).collect();

        // update the riff seq blades
        self.update_available_riff_sets_in_riff_seq_blades(state, &riff_sets);

        // update the riff arrangement blades
        self.update_available_riff_sets_in_riff_arrangement_blades(state, &riff_sets);
    }

    pub fn update_available_riff_sets_in_riff_seq_blades(
        &mut self,
        state: &DAWState,
        riff_sets: &Vec<(String, String)>,
    ) {
        // update the riff seq blades - riff sets combobox
        for riff_sequences_box_child in self.ui.riff_sequences_box.children().iter() {
            if let Some(riff_seq_blade_frame) = riff_sequences_box_child.dynamic_cast_ref::<Frame>() {
                self.update_available_items_in_blade_combobox(state, riff_sets, riff_seq_blade_frame.clone(), "riff_seq_riff_set_combo");
            }
        }
    }

    pub fn update_available_items_in_blade_combobox(
        &mut self,
        _state: &DAWState,
        items: &Vec<(String, String)>,
        blade: Frame,
        combo_box_widget_name: &str,
    ) {
        // update the blade
        if let Some(blade_child) = blade.child() {
            if let Some(blade_box) = blade_child.dynamic_cast_ref::<Box>() {
                if let Some(blade_box_child) = blade_box.children().get(0) {
                    if let Some(blade_box_grid) = blade_box_child.dynamic_cast_ref::<Grid>() {
                        for child in blade_box_grid.children().iter() {
                            if child.widget_name() == combo_box_widget_name {
                                if let Some(item_combobox) = child.dynamic_cast_ref::<ComboBoxText>() {
                                    let item_combobox: &ComboBoxText = item_combobox;
                                    item_combobox.remove_all();
                                    for (uuid, name) in items.iter() {
                                        item_combobox.append(Some(uuid), name);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn update_available_riff_sets_in_riff_arrangement_blades(
        &mut self,
        state: &DAWState,
        riff_sets: &Vec<(String, String)>,
    ) {
        // update the riff arr blades
        for riff_arr_box_child in self.ui.riff_arrangement_box.children().iter() {
            if let Some(blade) = riff_arr_box_child.dynamic_cast_ref::<Frame>() {
                self.update_available_items_in_blade_combobox(state, riff_sets, blade.clone(), "riff_arr_riff_set_combo");

                if let Some(blade_child) = blade.child() {
                    if let Some(blade_box) = blade_child.dynamic_cast_ref::<Box>() {
                        if let Some(blade_box_child) = blade_box.children().get(1) {
                            if let Some(blade_scrolled_window) = blade_box_child.dynamic_cast_ref::<ScrolledWindow>() {
                                for child in blade_scrolled_window.children().iter() {
                                    if let Some(riff_arr_blade_viewport) = child.dynamic_cast_ref::<Viewport>() {
                                        if let Some(viewport_child) = riff_arr_blade_viewport.children().get(0) {
                                            if let Some(riff_arr_box) = viewport_child.dynamic_cast_ref::<Box>() {
                                                for riff_arr_child in riff_arr_box.children().iter() {
                                                    if let Some(blade) = riff_arr_child.dynamic_cast_ref::<Frame>() {
                                                        self.update_available_items_in_blade_combobox(state, riff_sets, blade.clone(), "riff_seq_riff_set_combo");
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn update_available_riff_sequences_in_riff_arrangement_blades(
        &mut self,
        state: &DAWState,
    ) {
        // get the available riff sequences
        let riff_sequences: Vec<(String, String)> = state.project().song().riff_sequences().iter().map(|riff_sequence| (riff_sequence.uuid(), riff_sequence.name().to_string())).collect();

        // update the riff arrangement blades
        for riff_arr_box_child in self.ui.riff_arrangement_box.children().iter() {
            if let Some(blade) = riff_arr_box_child.dynamic_cast_ref::<Frame>() {
                self.update_available_items_in_blade_combobox(state, &riff_sequences, blade.clone(), "riff_arr_riff_seq_combo");
            }
        }
    }

    fn setup_riff_arrangement(
        &mut self,
        riff_arrangement: &RiffArrangement,
        ui: Ui,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        state: &DAWState,
        state_arc: Arc<Mutex<DAWState>>,
        track_uuids: Vec<String>,
        riff_sets: Vec<(String, String)>,
        riff_sequences: Vec<(String, String)>,
        visible: bool,
        selected_track_style_provider: CssProvider,
        riff_item_riff_set_blades_beat_grids: Arc<Mutex<HashMap<String, HashMap<String, Arc<Mutex<BeatGrid>>>>>>,
    ) {
        let riff_arrangement_blade = MainWindow::add_riff_arrangement_blade(
            ui.riff_arrangement_box,
            ui.arrangements_combobox.clone(),
            tx_from_ui.clone(),
            state_arc.clone(),
            Some(riff_sets.clone()),
            Some(riff_sequences),
            Some(riff_arrangement.uuid()),
            false,
                visible,
            selected_track_style_provider.clone(),
            ui.riff_arrangement_vertical_adjustment.clone(),
        );

        // set the name
        riff_arrangement_blade.riff_arrangement_name_entry.set_text(riff_arrangement.name());

        for item in riff_arrangement.items().iter() {
            match item.item_type() {
                RiffItemType::RiffSet => {
                    let riff_set_uuid = item.item_uuid().to_string();
                    let riff_set_box = riff_arrangement_blade.riff_set_box.clone();

                    let (riff_set_blade_head, riff_set_blade_drawing_areas, _) = MainWindow::add_riff_set_blade(
                        tx_from_ui.clone(),
                        riff_set_box.clone(),
                        riff_set_box.clone(),
                        riff_set_uuid.to_string(),
                        track_uuids.clone(),
                        state_arc.clone(),
                        riff_sets.iter().find(|(riff_set_uuid, _)| riff_set_uuid == riff_set_uuid).unwrap().1.clone(),
                        RiffSetType::RiffArrangement(riff_arrangement.uuid()),
                        selected_track_style_provider.clone(),
                        Some(riff_item_riff_set_blades_beat_grids.clone()),
                        item.uuid(),
                        Some(&ui.riff_arrangement_vertical_adjustment),
                    );

                    Self::style_riff_arrangement_riff_set(&riff_set_blade_head, &riff_set_blade_drawing_areas);
                }
                RiffItemType::RiffSequence => {
                    let riff_sequence_uuid = item.item_uuid().to_string();
                    let riff_sequence_blade = MainWindow::add_riff_sequence_blade(
                        riff_arrangement_blade.riff_set_box.clone(),
                        tx_from_ui.clone(),
                        state_arc.clone(),
                        Some(riff_sets.clone()),
                        Some(riff_sequence_uuid.to_string()),
                        Some(item.uuid()),
                        false,
                        RiffSequenceType::RiffArrangement(riff_arrangement.uuid()),
                        selected_track_style_provider.clone(),
                        Some(&ui.riff_arrangement_vertical_adjustment),
                    );

                    if let Some(riff_sequence) = state.project().song().riff_sequences().iter().find(|current_riff_sequence| current_riff_sequence.uuid() == riff_sequence_uuid) {
                        riff_sequence_blade.riff_sequence_name_entry.set_text(riff_sequence.name());
                        riff_sequence_blade.riff_sequence_blade.set_width_request(riff_sequence.riff_sets().iter().count() as i32 * (69 + 2 * 2 + 1));

                        for riff_set_reference in riff_sequence.riff_sets().iter() {
                            if let Some(riff_set) = state.project().song().riff_sets().iter().find(|current_riff_set| current_riff_set.uuid() == riff_set_reference.item_uuid().to_string()) {
                                let (riff_set_blade_head, _, _) = MainWindow::add_riff_set_blade(
                                    tx_from_ui.clone(),
                                    riff_sequence_blade.riff_set_box.clone(),
                                    riff_sequence_blade.riff_set_head_box.clone(),
                                    riff_set_reference.item_uuid().to_string(),
                                    track_uuids.clone(),
                                    state_arc.clone(),
                                    riff_set.name().to_string(),
                                    RiffSetType::RiffSequence(item.uuid()),
                                    selected_track_style_provider.clone(),
                                    Some(riff_item_riff_set_blades_beat_grids.clone()),
                                    riff_set_reference.uuid(),
                                    None,
                                );

                                Self::style_riff_arrangement_riff_seq_riff_set(&riff_set_blade_head);
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn update_automation_effect_parameters_combo(&mut self, state: &mut DAWState, track_uuid: String, effect_uuid: String) {
        let plugins_params = state.audio_plugin_parameters();
        if let Some(selected_effect_uuid) = state.selected_effect_plugin_uuid() {
            if *selected_effect_uuid == effect_uuid {
                if let Some(signal_handler_id) = &self.automation_effects_choice_signal_handler_id {
                    self.ui.automation_effects_combobox.block_signal(signal_handler_id);
                    self.ui.automation_effects_combobox.set_active_id(Some(selected_effect_uuid));
                    self.ui.automation_effects_combobox.unblock_signal(signal_handler_id);
                }

                // update the effect parameters based on the selected effect
                self.ui.automation_effect_parameters_combobox.remove_all();
                if let Some(track_plugins) = plugins_params.get(&track_uuid) {
                    if let Some(plugin_params) = track_plugins.get(effect_uuid.as_str()) {
                        debug!("effect plugin param count={}", plugin_params.len());
                        plugin_params.iter().for_each(|param| {
                            self.ui.automation_effect_parameters_combobox.append(Some(param.index.to_string().as_str()), param.name());
                        });
                    }
                }
            }
        }
    }

    pub fn update_automation_ui_from_state(&mut self, state: &mut DAWState) {
        // update the automation view
        // get the selected track
        let plugins_params = state.audio_plugin_parameters();
        if let Some(track_uuid) = state.selected_track() {
            if let Some(track_type) = state.project().song().tracks().iter().find(|track_type| {
                match track_type {
                    TrackType::InstrumentTrack(track) => track.uuid().to_string() == track_uuid,
                    TrackType::AudioTrack(_) => false,
                    TrackType::MidiTrack(_) => false,
                }
            }) {
                match track_type {
                    TrackType::InstrumentTrack(track) => {
                        // update the automation view instrument params list based on the selected track
                        self.ui.automation_instrument_parameters_combobox.remove_all();
                        let instrument_uuid = track.instrument().uuid().to_string();
                        let key = track.uuid().to_string();
                        if let Some(track_plugins) = plugins_params.get(&key) {
                            if let Some(plugin_params) = track_plugins.get(&instrument_uuid) {
                                debug!("Instrument plugin param count={}", plugin_params.len());
                                plugin_params.iter().for_each(|param| {
                                    self.ui.automation_instrument_parameters_combobox.append(Some(param.index.to_string().as_str()), param.name());
                                });
                            }
                            else {
                                debug!("Instrument plugin param count={}", 0);
                            }
                        }
                        else {
                            debug!("No track instrument plugin found.");
                        }

                        // update the automation view effect list based on the selected track
                        self.ui.automation_effects_combobox.remove_all();
                        let mut found_effects = false;
                        for effect in track.effects() {
                            found_effects = true;
                            let effect_uuid = effect.uuid().to_string();
                            self.ui.automation_effects_combobox.append(Some(effect_uuid.as_str()), effect.name());

                            if let Some(selected_effect_uuid) = state.selected_effect_plugin_uuid() {
                                if *selected_effect_uuid == effect_uuid {
                                    if let Some(signal_handler_id) = &self.automation_effects_choice_signal_handler_id {
                                        self.ui.automation_effects_combobox.block_signal(signal_handler_id);
                                        self.ui.automation_effects_combobox.set_active_id(Some(selected_effect_uuid));
                                        self.ui.automation_effects_combobox.unblock_signal(signal_handler_id);
                                    }

                                    // update the effect parameters based on the selected effect
                                    self.ui.automation_effect_parameters_combobox.remove_all();
                                    if let Some(track_plugins) = plugins_params.get(&key) {
                                        if let Some(plugin_params) = track_plugins.get(effect_uuid.as_str()) {
                                            debug!("effect plugin param count={}", plugin_params.len());
                                            plugin_params.iter().for_each(|param| {
                                                self.ui.automation_effect_parameters_combobox.append(Some(param.index.to_string().as_str()), param.name());
                                            });
                                        }
                                        else {
                                            debug!("Effect plugin param count={}", 0);
                                        }
                                    }
                                    else {
                                        debug!("No track effect plugins found.");
                                    }
                                }
                            }
                        }

                        // clear the effect params combo of anything left over from another track (trips if this track does not have any effects)
                        if !found_effects {
                            self.ui.automation_effect_parameters_combobox.remove_all();
                        }
                    },
                    TrackType::AudioTrack(_) => (),
                    TrackType::MidiTrack(_) => (),
                }
            }
        }
    }

    /// Get a mutable reference to the main window's piano roll grid.
    pub fn piano_roll_grid_mut(&mut self) -> &mut Option<Arc<Mutex<BeatGrid>>> {
        &mut self.piano_roll_grid
    }

    /// Get a reference to the main window's piano roll grid.
    pub fn piano_roll_grid(&self) -> Option<&Arc<Mutex<BeatGrid>>> {
        self.piano_roll_grid.as_ref()
    }

    /// Get a mutable reference to the main window's sample roll grid.
    pub fn sample_roll_grid_mut(&mut self) -> &mut Option<Arc<Mutex<BeatGrid>>> {
        &mut self.sample_roll_grid
    }

    /// Get a reference to the main window's sample roll grid.
    pub fn sample_roll_grid(&self) -> Option<&Arc<Mutex<BeatGrid>>> {
        self.sample_roll_grid.as_ref()
    }

    /// Get a reference to the main window's track grid.
    pub fn track_grid(&self) -> Option<&Arc<Mutex<BeatGrid>>> {
        self.track_grid.as_ref()
    }

    /// Get a reference to the main window's controller grid.
    #[must_use]
    pub fn automation_grid(&self) -> Option<&Arc<Mutex<BeatGrid>>> {
        self.automation_grid.as_ref()
    }

    pub fn get_track_riffs_stack_visible_name(&self) -> String {
        self.ui.centre_panel_stack.visible_child_name().unwrap().to_string()
    }

    pub fn get_riffs_stack_visible_name(&self) -> String {
        self.ui.riffs_stack.visible_child_name().unwrap().to_string()
    }

    pub fn set_piano_roll_selected_track_name_label(&self, track_name: &str) {
        self.ui.piano_roll_track_name.set_text(track_name);
    }

    pub fn set_piano_roll_selected_riff_name_label(&self, riff_name: &str) {
        self.ui.piano_roll_riff_name.set_text(riff_name);
    }

    pub fn set_sample_roll_selected_track_name_label(&self, track_name: &str) {
        self.ui.sample_roll_track_name.set_text(track_name);
    }

    pub fn set_sample_roll_selected_riff_name_label(&self, riff_name: &str) {
        self.ui.sample_roll_riff_name.set_text(riff_name);
    }

    pub fn repaint_riff_set_view_riff_set_active_drawing_areas(&self, riff_set_uuid: &str, play_position_in_beats: f64) {
        // update the cursor position for the grids
        if let Ok(riff_set_view_riff_set_beat_grids) = self.riff_set_view_riff_set_beat_grids.lock() {
            if let Some(riff_set_tracks_beat_grids) = riff_set_view_riff_set_beat_grids.get(&riff_set_uuid.to_string()) {
                for (_, beat_grid_mutex) in riff_set_tracks_beat_grids.iter() {
                    if let Ok(mut beat_grid) = beat_grid_mutex.lock() {
                        beat_grid.set_track_cursor_time_in_beats(play_position_in_beats);
                    }
                }
            }
        }

        // queue a redraw of the DrawingAreas
        for riff_set_drawing_areas_box in self.ui.riff_sets_box.children().iter() {
            if riff_set_drawing_areas_box.widget_name() == riff_set_uuid {
                if let Some(riff_drawing_areas_box) = riff_set_drawing_areas_box.dynamic_cast_ref::<Box>() {
                    for riff_drawing_area in riff_drawing_areas_box.children().iter() {
                        if riff_drawing_area.widget_name().starts_with("riffset_") {
                            riff_drawing_area.queue_draw();
                        }
                        else {
                            break;
                        }
                    }
                }
                break;
            }
        }
   }

    pub fn repaint_riff_sequence_view_riff_sequence_active_drawing_areas(&self, riff_sequence_uuid: &str, play_position_in_beats: f64, playing_riff_sequence_summary_data: &(f64, Vec<(f64, String, String)>)) {
        // get the playing riff sequence details
        let riff_sequence_actual_playing_length = playing_riff_sequence_summary_data.0;
        if play_position_in_beats <= riff_sequence_actual_playing_length {
            let riff_set_actual_playing_lengths = playing_riff_sequence_summary_data.1.clone();

            // find the playing riff set
            let mut running_playing_position = 0.0;
            let mut playing_riff_set_reference_uuid = None;
            let mut playing_before_riff_set_reference_uuid = None;
            for riff_set_details in riff_set_actual_playing_lengths.iter() {
                let riff_set_actual_playing_length = riff_set_details.0;
                let riff_set_reference_uuid = riff_set_details.1.clone();
                let riff_set_end_position = running_playing_position + riff_set_actual_playing_length;
                if running_playing_position <= play_position_in_beats && play_position_in_beats <= riff_set_end_position {
                    playing_riff_set_reference_uuid = Some(riff_set_reference_uuid);
                    break;
                }
                else {
                    playing_before_riff_set_reference_uuid = Some(riff_set_reference_uuid);
                }
                running_playing_position += riff_set_actual_playing_length;
            }

            let playing_before_riff_set_reference_uuid = if let Some(playing_before_riff_set_reference_uuid) = &playing_before_riff_set_reference_uuid {
                self.set_riff_sequence_riff_set_track_beat_grid_cursor_time_in_beats(riff_sequence_uuid.to_string(), playing_before_riff_set_reference_uuid.to_string(), 0.0);
                playing_before_riff_set_reference_uuid.to_string()
            }
            else {
                "".to_string()
            };

            if let Some(playing_riff_set_reference_uuid) = playing_riff_set_reference_uuid {
                // update the cursor position for the grids
                self.set_riff_sequence_riff_set_track_beat_grid_cursor_time_in_beats(riff_sequence_uuid.to_string(), playing_riff_set_reference_uuid.to_string(), play_position_in_beats);
                // self.repaint_riff_sequence_riff_set_drawing_areas(playing_riff_set_reference_uuid.to_string());
                if let Some(active_riff_sequence_index) = self.ui.sequence_combobox.active() {
                    if let Some(riff_sequence_blade_widget) = self.ui.riff_sequences_box.children().get(active_riff_sequence_index as usize) {
                        if let Some(riff_sequence_blade) = riff_sequence_blade_widget.dynamic_cast_ref::<Frame>() {
                            self.repaint_riff_sequence_riff_set_drawing_areas(riff_sequence_blade, playing_riff_set_reference_uuid.to_string(), playing_before_riff_set_reference_uuid.to_string());
                        }
                    }
                }
            }
        }
    }

    pub fn set_riff_sequence_riff_set_track_beat_grid_cursor_time_in_beats(&self, riff_sequence_uuid: String, playing_riff_set_reference_uuid: String, play_position_in_beats: f64) {
        if let Ok(riff_sequence_view_riff_set_ref_beat_grids) = self.riff_sequence_view_riff_set_ref_beat_grids.lock() {
            if let Some(riff_sequence_riff_set_beat_grids) = riff_sequence_view_riff_set_ref_beat_grids.get(&riff_sequence_uuid.to_string()) {
                if let Ok(riff_sequence_riff_set_beat_grids) = riff_sequence_riff_set_beat_grids.lock() {
                    for (riff_set_ref_uuid, tracks_beat_grids) in riff_sequence_riff_set_beat_grids.iter() {
                        if riff_set_ref_uuid.contains(&playing_riff_set_reference_uuid) {
                            for (_, beat_grid_arc) in tracks_beat_grids.iter() {
                                if let Ok(mut beat_grid) = beat_grid_arc.lock() {
                                    beat_grid.set_track_cursor_time_in_beats(play_position_in_beats);
                                }
                            }
                            break;
                        }
                    }
                }
            }
        }
    }


    pub fn repaint_riff_sequence_riff_set_drawing_areas(&self, riff_sequence_blade: &Frame, playing_riff_set_reference_uuid: String, playing_before_riff_set_reference_uuid: String) {
        // navigate down to the riff_set_box child of a child of a child... widget
        for widget in riff_sequence_blade.children().iter() {
            if let Some(top_level_box) = widget.dynamic_cast_ref::<Box>() {
                for widget in top_level_box.children().iter() {
                    if let Some(mid_level_box) = widget.dynamic_cast_ref::<Box>() {
                        for widget in mid_level_box.children().iter() {
                            if widget.widget_name().to_string() == "riff_sequence_riff_sets_scrolled_window" {
                                if let Some(riff_sets_scrolled_window) = widget.dynamic_cast_ref::<ScrolledWindow>() {
                                    if let Some(widget) = riff_sets_scrolled_window.child() {
                                        if let Some(view_port) = widget.dynamic_cast_ref::<Viewport>() {
                                            if let Some(riff_sets_box) = view_port.child() {
                                                if let Some(riff_sets_box) = riff_sets_box.dynamic_cast_ref::<Box>() {
                                                    for widget in riff_sets_box.children().iter() {
                                                        if let Some(riff_set_box) = widget.dynamic_cast_ref::<Box>() {
                                                            let riff_set_ref_id = riff_set_box.widget_name().to_string();
                                                            let (update_beat_grids, break_from_loop) = if riff_set_ref_id.contains(&playing_riff_set_reference_uuid) {
                                                                (true, true)
                                                            }
                                                            else if riff_set_ref_id.contains(&playing_before_riff_set_reference_uuid) {
                                                                (true, false)
                                                            }
                                                            else {
                                                                continue;
                                                            };

                                                            if update_beat_grids {
                                                                for widget in riff_set_box.children().iter() {
                                                                    if let Some(drawing_area) = widget.dynamic_cast_ref::<DrawingArea>() {
                                                                        // need to check that the drawing area is in use by checking its widget name starts with the riffset_ prefix
                                                                        if drawing_area.widget_name().to_string().starts_with("riffset_") {
                                                                            drawing_area.queue_draw();
                                                                        }
                                                                    }
                                                                }
                                                            }

                                                            if break_from_loop {
                                                                break;
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn repaint_riff_arrangement_view_riff_arrangement_active_drawing_areas(
        &self,
        riff_arrangement_uuid: &str,
        play_position_in_beats: f64,
        playing_riff_arrangement_summary_data: &(f64, Vec<(f64, RiffItem, Vec<(f64, RiffItem)>)>)) {
        // get the playing riff arrangement details
        let riff_arrangement_actual_playing_length = playing_riff_arrangement_summary_data.0;
        let mut adjusted_play_position_in_beats = 0.0;
        let mut playing_riff_set_pixel_position = 0.0;
        if play_position_in_beats <= riff_arrangement_actual_playing_length {
            let riff_items_actual_playing_lengths = playing_riff_arrangement_summary_data.1.clone();

            // find the playing riff item
            let mut running_playing_position = 0.0;
            let mut playing_riff_sequence = None;
            // let mut playing_before_riff_sequence = None;
            let mut playing_riff_item = None;
            let mut playing_before_riff_item = None;
            for riff_item_details in riff_items_actual_playing_lengths.iter() {
                let riff_item_actual_playing_length = riff_item_details.0;
                let riff_item_end_position = running_playing_position + riff_item_actual_playing_length;
                if running_playing_position <= play_position_in_beats && play_position_in_beats <= riff_item_end_position {
                    if let RiffItemType::RiffSequence = riff_item_details.1.item_type() {
                        for riff_set_data in riff_item_details.2.iter() {
                            let riff_set_actual_playing_length = riff_set_data.0;
                            let riff_set_end_position = running_playing_position + riff_set_actual_playing_length;
                            if running_playing_position <= play_position_in_beats && play_position_in_beats <= riff_set_end_position {
                                playing_riff_item = Some(riff_set_data.1.clone());
                                playing_riff_sequence = Some(riff_item_details.1.uuid());
                                adjusted_play_position_in_beats = play_position_in_beats - running_playing_position;
                                break;
                            }
                            else {
                                playing_riff_set_pixel_position += 69.0;
                                playing_before_riff_item = Some(riff_set_data.1.clone());
                            }
                            running_playing_position += riff_set_actual_playing_length;
                        }
                    }
                    else {
                        playing_riff_item = Some(riff_item_details.1.clone());
                        playing_riff_sequence = None;
                        adjusted_play_position_in_beats = play_position_in_beats - running_playing_position;
                    }
                    break;
                }
                else {
                    if let RiffItemType::RiffSequence = riff_item_details.1.item_type() {
                        playing_riff_set_pixel_position += (69.0 * (riff_item_details.2.len() as f64));
                    }
                    else {
                        playing_riff_set_pixel_position += 69.0;
                    }
                    playing_before_riff_item = Some(riff_item_details.1.clone());
                }
                running_playing_position += riff_item_actual_playing_length;
            }

            if let Some(playing_riff_item) = playing_riff_item {
                let before_riff_item = if let Some(riff_item_ref) = &playing_before_riff_item {
                    riff_item_ref.clone()
                }
                else {
                    RiffItem::new(RiffItemType::RiffSet, "".to_string())
                };

                // update the cursor position for the grids
                if let Ok(riff_arrangement_view_riff_set_ref_beat_grids) = self.riff_arrangement_view_riff_set_ref_beat_grids.lock() {
                    if let Some(riff_arrangement_riff_item_beat_grids) = riff_arrangement_view_riff_set_ref_beat_grids.get(&riff_arrangement_uuid.to_string()) {
                        if let Ok(riff_arrangement_riff_item_beat_grids) = riff_arrangement_riff_item_beat_grids.lock() {
                            for (key, thing) in riff_arrangement_riff_item_beat_grids.iter() {
                                debug!("riff_arrangement_riff_item_beat_grids key={}", key);
                            }
                            let playing_key_to_match = if let Some(playing_riff_sequence) = playing_riff_sequence.clone() {
                                format!("{}_{}", playing_riff_sequence.as_str(), playing_riff_item.uuid())
                            }
                            else {
                                playing_riff_item.uuid()
                            };
                            if let Some((key, playing_riff_item_beat_grids)) = riff_arrangement_riff_item_beat_grids.iter().find(|(key, value)| key.contains(playing_key_to_match.as_str())) {
                                for (_, track_beat_grid) in playing_riff_item_beat_grids.iter() {
                                    if let Ok(mut beat_grid) = track_beat_grid.lock() {
                                        beat_grid.set_track_cursor_time_in_beats(adjusted_play_position_in_beats);
                                    }
                                }
                            }
                            let before_key_to_match = if let Some(playing_riff_sequence) = playing_riff_sequence.clone() {
                                format!("{}_{}", playing_riff_sequence.as_str(), before_riff_item.uuid())
                            }
                            else {
                                before_riff_item.uuid()
                            };
                            if let Some((key, playing_before_riff_item_beat_grids)) = riff_arrangement_riff_item_beat_grids.iter().find(|(key, value)| key.contains(before_key_to_match.as_str())) {
                                for (_, track_beat_grid) in playing_before_riff_item_beat_grids.iter() {
                                    if let Ok(mut beat_grid) = track_beat_grid.lock() {
                                        beat_grid.set_track_cursor_time_in_beats(0.0);
                                    }
                                }
                            }
                        }
                    }
                }

                // queue a redraw of the DrawingAreas
                if let Some(active_riff_arrangement_index) = self.ui.arrangements_combobox.active() {
                    if let Some(riff_arrangement_blade_widget) = self.ui.riff_arrangement_box.children().get(active_riff_arrangement_index as usize) {
                        if let Some(riff_arrangement_blade) = riff_arrangement_blade_widget.dynamic_cast_ref::<Frame>() {
                            // navigate down to the riff_set_box child of a child of a child... widget
                            for widget in riff_arrangement_blade.children().iter() {
                                if let Some(top_level_box) = widget.dynamic_cast_ref::<Box>() {
                                    for widget in top_level_box.children().iter() {
                                        if widget.widget_name().to_string() == "riff_arrangement_riff_items_scrolled_window" {
                                            if let Some(riff_items_scrolled_window) = widget.dynamic_cast_ref::<ScrolledWindow>() {
                                                // set the scroll position
                                                if riff_items_scrolled_window.hadjustment().value() != playing_riff_set_pixel_position {
                                                    riff_items_scrolled_window.hadjustment().set_value(playing_riff_set_pixel_position);
                                                }

                                                if let Some(widget) = riff_items_scrolled_window.child() {
                                                    if let Some(view_port) = widget.dynamic_cast_ref::<Viewport>() {
                                                        if let Some(riff_sets_box) = view_port.child() {
                                                            if let Some(riff_sets_box) = riff_sets_box.dynamic_cast_ref::<Box>() {
                                                                for widget in riff_sets_box.children().iter() {
                                                                    if let Some(riff_set_box) = widget.dynamic_cast_ref::<Box>() {
                                                                        // handle a riff set
                                                                        if let Some(widget) = riff_set_box.children().get(2) {
                                                                            if let Some(scrolled_window) = widget.dynamic_cast_ref::<ScrolledWindow>() {
                                                                                if let Some(widget) = scrolled_window.child() {
                                                                                    if let Some(view_port) = widget.dynamic_cast_ref::<Viewport>() {
                                                                                        if let Some(riff_sets_box) = view_port.child() {
                                                                                            if let Some(riff_set_box) = riff_sets_box.dynamic_cast_ref::<Box>() {
                                                                                                for widget in riff_set_box.children().iter() {
                                                                                                    // debug!("widget.widget_name().to_string()={}, playing_riff_item.uuid()={}, before_riff_item.uuid()={}", widget.widget_name().to_string(), playing_riff_item.uuid(), before_riff_item.uuid());
                                                                                                    if widget.widget_name().to_string().contains(&playing_riff_item.uuid()) || widget.widget_name().to_string().contains(&before_riff_item.uuid()) {
                                                                                                        if let Some(riff_set) = widget.dynamic_cast_ref::<Box>() {
                                                                                                            for widget in riff_set.children().iter() {
                                                                                                                if let Some(drawing_area) = widget.dynamic_cast_ref::<DrawingArea>() {
                                                                                                                    // need to check that the drawing area is in use by checking its widget name starts with the riffset_ prefix
                                                                                                                    if drawing_area.widget_name().to_string().starts_with("riffset_") {
                                                                                                                        drawing_area.queue_draw();
                                                                                                                    }
                                                                                                                }
                                                                                                            }
                                                                                                        }
                                                                                                    }
                                                                                                }
                                                                                            }
                                                                                        }
                                                                                    }
                                                                                }
                                                                            }
                                                                        }
                                                                    } else if let Some(riff_sequence_blade) = widget.dynamic_cast_ref::<Frame>() {
                                                                        // handle a riff sequence
                                                                        // self.repaint_riff_sequence_riff_set_drawing_areas(&riff_sequence_blade.clone(), before_riff_item.uuid());
                                                                        if let Some(playing_riff_sequence_uuid) = playing_riff_sequence.clone() {
                                                                            if riff_sequence_blade.widget_name().to_string().contains(playing_riff_sequence_uuid.as_str()) {
                                                                                self.repaint_riff_sequence_riff_set_drawing_areas(riff_sequence_blade, playing_riff_item.uuid(), before_riff_item.uuid());
                                                                            }
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn get_selected_riff_arrangement_play_position(&self) -> usize {
        // find the riff item box
        if let Some(riff_item_box) = self.find_riff_arrangement_riff_item_box() {
            if let Some(selected_child_position) = Self::get_selected_riff_item_position(&riff_item_box) {
                return selected_child_position;
            }
        }

        return 0
    }

   pub fn setup_track_midi_routing_panel(track_midi_routing_panel: TrackMidiRoutingPanel, midi_routing: crate::domain::TrackEventRouting, routing_description: String, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, track_midi_routing_scrolled_box: Box, track_uuid: Uuid) {
       track_midi_routing_panel.track_midi_routing_panel.set_widget_name(midi_routing.uuid().as_str());
       track_midi_routing_panel.track_midi_routing_send_to_track_label.set_text(routing_description.as_str());
       track_midi_routing_panel.track_midi_routing_midi_channel_combobox_text.set_active(Some((midi_routing.channel - 1) as u32));
       track_midi_routing_panel.track_midi_routing_note_from_combobox_text.set_active(Some(midi_routing.note_range.0 as u32));
       track_midi_routing_panel.track_midi_routing_note_to_combobox_text.set_active(Some(midi_routing.note_range.1 as u32));

       // need to add listeners to handle changes
       {
           let tx_from_ui = tx_from_ui.clone();
           let track_midi_routing_scrolled_box = track_midi_routing_scrolled_box.clone();
           let track_midi_routing_frame = track_midi_routing_panel.track_midi_routing_panel.clone();
           track_midi_routing_panel.track_midi_routing_delete_button.connect_clicked(move |_| {
               let route_uuid = track_midi_routing_frame.widget_name().to_string();
   
               track_midi_routing_scrolled_box.remove(&track_midi_routing_frame);
               track_midi_routing_scrolled_box.queue_draw();
   
               match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RemoveMidiRouting(route_uuid), Some(track_uuid.to_string()))) {
                   Err(_) => debug!("Problem sending message with tx from ui lock when removing a midi routing."),
                   _ => (),
               }
           });
       }
       {
           let tx_from_ui = tx_from_ui.clone();
           let track_midi_routing_frame = track_midi_routing_panel.track_midi_routing_panel.clone();
           let track_midi_routing_note_from_combobox_text = track_midi_routing_panel.track_midi_routing_note_from_combobox_text.clone();
           let track_midi_routing_note_to_combobox_text = track_midi_routing_panel.track_midi_routing_note_to_combobox_text.clone();
           track_midi_routing_panel.track_midi_routing_midi_channel_combobox_text.connect_changed(move |midi_channel_combobox| {
               let route_uuid = track_midi_routing_frame.widget_name().to_string();
   
               if let Some(midi_channel) = midi_channel_combobox.active_id() {
                   if let Some(start_note) = track_midi_routing_note_from_combobox_text.active_id() {
                       if let Some(end_note) = track_midi_routing_note_to_combobox_text.active_id() {
                           match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::UpdateMidiRouting(
                               route_uuid, 
                               midi_channel.as_str().parse::<i32>().unwrap(), 
                               start_note.as_str().parse::<i32>().unwrap(), 
                               end_note.as_str().parse::<i32>().unwrap()
                           ), Some(track_uuid.to_string()))) {
                               Err(_) => debug!("Problem sending message with tx from ui lock when updating a midi routing."),
                               _ => (),
                           }
                       }
                   }
               }
           });
       }
       {
           let tx_from_ui = tx_from_ui.clone();
           let track_midi_routing_frame = track_midi_routing_panel.track_midi_routing_panel.clone();
           let midi_channel_combobox = track_midi_routing_panel.track_midi_routing_midi_channel_combobox_text.clone();
           let track_midi_routing_note_to_combobox_text = track_midi_routing_panel.track_midi_routing_note_to_combobox_text.clone();
           track_midi_routing_panel.track_midi_routing_note_from_combobox_text.connect_changed(move |note_from_combobox| {
               let route_uuid = track_midi_routing_frame.widget_name().to_string();
   
               if let Some(midi_channel) = midi_channel_combobox.active_id() {
                   if let Some(start_note) = note_from_combobox.active_id() {
                       if let Some(end_note) = track_midi_routing_note_to_combobox_text.active_id() {
                           match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::UpdateMidiRouting(
                               route_uuid, 
                               midi_channel.as_str().parse::<i32>().unwrap(), 
                               start_note.as_str().parse::<i32>().unwrap(), 
                               end_note.as_str().parse::<i32>().unwrap()
                           ), Some(track_uuid.to_string()))) {
                               Err(_) => debug!("Problem sending message with tx from ui lock when updating a midi routing."),
                               _ => (),
                           }
                       }
                   }
               }
           });
       }
       {
           let tx_from_ui = tx_from_ui.clone();
           let track_midi_routing_frame = track_midi_routing_panel.track_midi_routing_panel.clone();
           let midi_channel_combobox= track_midi_routing_panel.track_midi_routing_midi_channel_combobox_text.clone();
           let track_midi_routing_note_from_combobox_text = track_midi_routing_panel.track_midi_routing_note_from_combobox_text.clone();
           track_midi_routing_panel.track_midi_routing_note_to_combobox_text.connect_changed(move |note_to_combobox| {
               let route_uuid = track_midi_routing_frame.widget_name().to_string();
   
               if let Some(midi_channel) = midi_channel_combobox.active_id() {
                   if let Some(start_note) = track_midi_routing_note_from_combobox_text.active_id() {
                       if let Some(end_note) = note_to_combobox.active_id() {
                           match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::UpdateMidiRouting(
                               route_uuid, 
                               midi_channel.as_str().parse::<i32>().unwrap(), 
                               start_note.as_str().parse::<i32>().unwrap(), 
                               end_note.as_str().parse::<i32>().unwrap()
                           ), Some(track_uuid.to_string()))) {
                               Err(_) => debug!("Problem sending message with tx from ui lock when updating a midi routing."),
                               _ => (),
                           }
                       }
                   }
               }
           });
       }
       match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RouteMidiTo(midi_routing), Some(track_uuid.to_string()))) {
           Err(_) => debug!("Problem sending message with tx from ui lock when routing midi to a track and plugin has been selected."),
           _ => (),
       }
   }
      
   pub fn setup_track_audio_routing_panel(track_audio_routing_panel: TrackAudioRoutingPanel, audio_routing: crate::domain::AudioRouting, routing_description: String, tx_from_ui: crossbeam_channel::Sender<DAWEvents>, track_audio_routing_scrolled_box: Box, track_uuid: Uuid) {
       track_audio_routing_panel.track_audio_routing_panel.set_widget_name(audio_routing.uuid().as_str());
       track_audio_routing_panel.track_audio_routing_send_to_track_label.set_text(routing_description.as_str());

       match &audio_routing.destination {
        crate::domain::AudioRoutingNodeType::Track(_) => {
            // track_audio_routing_panel.track_audio_routing_left_channel_input_index_combobox_text.;
            // track_audio_routing_panel.track_audio_routing_right_channel_input_index_combobox_text.set_active(Some(audio_routing.note_range.1 as u32));
        }
        crate::domain::AudioRoutingNodeType::Instrument(_, _, left_channel_index, right_channel_index) => {
            track_audio_routing_panel.track_audio_routing_left_channel_input_index_combobox_text.set_active(Some(*left_channel_index as u32));
            track_audio_routing_panel.track_audio_routing_right_channel_input_index_combobox_text.set_active(Some(*right_channel_index as u32));
        },
        crate::domain::AudioRoutingNodeType::Effect(_, _, left_channel_index, right_channel_index) => {
            track_audio_routing_panel.track_audio_routing_left_channel_input_index_combobox_text.set_active(Some(*left_channel_index as u32));
            track_audio_routing_panel.track_audio_routing_right_channel_input_index_combobox_text.set_active(Some(*right_channel_index as u32));
        },
       }

       // need to add listeners to handle changes
       {
           let tx_from_ui = tx_from_ui.clone();
           let track_audio_routing_scrolled_box = track_audio_routing_scrolled_box.clone();
           let track_audio_routing_frame = track_audio_routing_panel.track_audio_routing_panel.clone();
           track_audio_routing_panel.track_audio_routing_delete_button.connect_clicked(move |_| {
               let route_uuid = track_audio_routing_frame.widget_name().to_string();
   
               track_audio_routing_scrolled_box.remove(&track_audio_routing_frame);
               track_audio_routing_scrolled_box.queue_draw();
   
               match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RemoveAudioRouting(route_uuid), Some(track_uuid.to_string()))) {
                   Err(_) => debug!("Problem sending message with tx from ui lock when removing an audio routing."),
                   _ => (),
               }
           });
       }

       {
           let _tx_from_ui = tx_from_ui.clone();
           let track_audio_routing_frame = track_audio_routing_panel.track_audio_routing_panel.clone();
           let track_audio_routing_right_channel_input_index_combobox_text = track_audio_routing_panel.track_audio_routing_right_channel_input_index_combobox_text.clone();
           track_audio_routing_panel.track_audio_routing_left_channel_input_index_combobox_text.connect_changed(move |left_channel_input_index_combobox| {
               let _route_uuid = track_audio_routing_frame.widget_name().to_string();
   
                if let Some(_left_channel_input_index) = left_channel_input_index_combobox.active_id() {
                    if let Some(_right_channel_input_index) = track_audio_routing_right_channel_input_index_combobox_text.active_id() {
                        // match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::UpdateAudioRouting(
                        //     route_uuid, 
                        //     left_channel_input_index.as_str().parse::<i32>().unwrap(), 
                        //     right_channel_input_index.as_str().parse::<i32>().unwrap()
                        // ), Some(track_uuid.to_string()))) {
                        //     Err(_) => debug!("Problem sending message with tx from ui lock when updating an audio routing."),
                        //     _ => (),
                        // }
                    }
                }
           });
       }
       {
           let _tx_from_ui = tx_from_ui.clone();
           let track_audio_routing_frame = track_audio_routing_panel.track_audio_routing_panel.clone();
           let track_audio_routing_left_channel_input_index_combobox_text = track_audio_routing_panel.track_audio_routing_left_channel_input_index_combobox_text.clone();
           track_audio_routing_panel.track_audio_routing_right_channel_input_index_combobox_text.connect_changed(move |right_channel_input_index_combobox| {
               let _route_uuid = track_audio_routing_frame.widget_name().to_string();
   
                if let Some(_left_channel_input_index) = track_audio_routing_left_channel_input_index_combobox_text.active_id() {
                    if let Some(_right_channel_input_index) = right_channel_input_index_combobox.active_id() {
                        // match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::UpdateAudioRouting(
                        //     route_uuid, 
                        //     left_channel_input_index.as_str().parse::<i32>().unwrap(), 
                        //     right_channel_input_index.as_str().parse::<i32>().unwrap()
                        // ), Some(track_uuid.to_string()))) {
                        //     Err(_) => debug!("Problem sending message with tx from ui lock when updating an audio routing."),
                        //     _ => (),
                        // }
                    }
                }
           });
       }
       match tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::RouteAudioTo(audio_routing), Some(track_uuid.to_string()))) {
           Err(_) => debug!("Problem sending message with tx from ui lock when routing audio to a track and plugin has been selected."),
           _ => (),
       }
    }

    pub fn setup_riff_set_drag_and_drop(
        riff_set_heads_box: Box, 
        riff_set_bodies_box: Box, 
        riff_set_horizontal_adjustment: Adjustment,
        riff_sets_view_port: Viewport,
        riff_set_type: RiffSetType,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
    ) {
        riff_set_heads_box.drag_dest_set(
            DestDefaults::ALL, 
            DRAG_N_DROP_TARGETS.as_ref(), 
            gdk::DragAction::COPY);

        riff_set_heads_box.connect_drag_motion(move |_, _ , x , _y, _| {
            if let Some(window) = riff_sets_view_port.window() {
                let view_port_width = window.width();
                let horizontal_adjustment_position = riff_set_horizontal_adjustment.value() as i32;

                // debug!("Dragging a riff set: view_port_width={}, horizon_adjustment_position={}, x={}, y={}, horizontal_adjustment_position + view_port_width - x={}, x - horizontal_adjustment_position={}", view_port_width, riff_set_horizontal_adjustment.value(), x, y, horizontal_adjustment_position + view_port_width - x, x - horizontal_adjustment_position);
    
                if (horizontal_adjustment_position + view_port_width - x) <= 50 {
                    riff_set_horizontal_adjustment.set_value((horizontal_adjustment_position + 50) as f64);
                }
                else if (x - horizontal_adjustment_position) < 50 && horizontal_adjustment_position >= 50 {
                    riff_set_horizontal_adjustment.set_value((horizontal_adjustment_position - 50) as f64);
                }
            }

            true
        });
    
        {
            riff_set_heads_box.connect_drag_data_received(move |riff_set_heads_box, _, x, y, selection_data, _, _| {
                // debug!("drag data received: x={}, y={}, info={}, time={}", x, y, info, time);
                if let Some(riff_set_uuid) = selection_data.text() {
                    let riff_set_uuid = riff_set_uuid.to_string();
                    // get the child at x and y
                    for child in riff_set_heads_box.children().iter() {
                        if child.allocation().x <= x && 
                            x <= (child.allocation().x + child.allocation().width) &&
                            child.allocation().y <= y && 
                            y <= (child.allocation().y + child.allocation().height) {
                            let drop_zone_child_position = riff_set_heads_box.child_position(child);
                            
                            // move the dropped child to the found position
                            for child in riff_set_heads_box.children().iter() {
                                let child_widget_name = child.widget_name().to_string();
                                if riff_set_uuid.contains(child_widget_name.as_str()) {
                                    let dragged_riff_set_position = riff_set_heads_box.child_position(child);
    
                                    
                                    let mut body_index = 0;
                                    for riff_sets_box_child in riff_set_bodies_box.children().iter() {
                                        if body_index == dragged_riff_set_position {
                                            riff_set_heads_box.set_child_position(child, drop_zone_child_position);
                                            riff_set_bodies_box.set_child_position(riff_sets_box_child, drop_zone_child_position);

                                            match &riff_set_type {
                                                RiffSetType::RiffSet => {
                                                    let _ = tx_from_ui.send(DAWEvents::RiffSetMoveToPosition(riff_set_uuid.to_string(), drop_zone_child_position as usize));
                                                }
                                                RiffSetType::RiffSequence(riff_sequence_uuid) => {
                                                    let _ = tx_from_ui.send(DAWEvents::RiffSequenceRiffSetMoveToPosition(riff_sequence_uuid.clone(), riff_set_uuid.to_string(), drop_zone_child_position as usize));
                                                }
                                                RiffSetType::RiffArrangement(riff_arrangement_uuid) => {
                                                    let _ = tx_from_ui.send(DAWEvents::RiffArrangementMoveRiffItemToPosition(riff_arrangement_uuid.clone(), riff_set_uuid.to_string(), drop_zone_child_position as usize));
                                                }
                                            }
                                            break;
                                        }
                                        body_index += 1;
                                    }
                                    break;
                                }
                            }
                            break;
                        }
                    }
                }
            });
        }
    }

    pub fn setup_riff_arrangement_riff_item_drag_and_drop(
        riff_items_box: Box,
        riff_items_horizontal_adjustment: Adjustment,
        riff_items_view_port: Viewport,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
        riff_set_type: RiffSetType,
    ) {
        riff_items_box.drag_dest_set(
            DestDefaults::ALL,
            DRAG_N_DROP_TARGETS.as_ref(),
            gdk::DragAction::COPY);

        riff_items_box.connect_drag_motion(move |_, _, x, _y, _| {
            if let Some(window) = riff_items_view_port.window() {
                let view_port_width = window.width();
                let horizontal_adjustment_position = riff_items_horizontal_adjustment.value() as i32;

                // debug!("Dragging a riff set: view_port_width={}, horizon_adjustment_position={}, x={}, y={}, horizontal_adjustment_position + view_port_width - x={}, x - horizontal_adjustment_position={}", view_port_width, riff_set_horizontal_adjustment.value(), x, y, horizontal_adjustment_position + view_port_width - x, x - horizontal_adjustment_position);

                if (horizontal_adjustment_position + view_port_width - x) <= 50 {
                    riff_items_horizontal_adjustment.set_value((horizontal_adjustment_position + 50) as f64);
                }
                else if (x - horizontal_adjustment_position) < 50 && horizontal_adjustment_position >= 50 {
                    riff_items_horizontal_adjustment.set_value((horizontal_adjustment_position - 50) as f64);
                }
            }

            true
        });

        {
            riff_items_box.connect_drag_data_received(move |riff_items_box, _, x, y, selection_data, _, _| {
                // debug!("drag data received: x={}, y={}, info={}, time={}", x, y, info, time);
                if let Some(riff_item_uuid) = selection_data.text() {
                    let riff_item_uuid = riff_item_uuid.to_string();
                    // get the child at x and y
                    for drop_zone_child in riff_items_box.children().iter() {
                        if drop_zone_child.allocation().x <= x &&
                            x <= (drop_zone_child.allocation().x + drop_zone_child.allocation().width) &&
                            drop_zone_child.allocation().y <= y &&
                            y <= (drop_zone_child.allocation().y + drop_zone_child.allocation().height) {
                            let drop_zone_child_position = riff_items_box.child_position(drop_zone_child);

                            // move the dropped child to the found position
                            for dragged_child in riff_items_box.children().iter() {
                                let child_widget_name = dragged_child.widget_name().to_string();
                                if riff_item_uuid.contains(child_widget_name.as_str()) {
                                    riff_items_box.set_child_position(dragged_child, drop_zone_child_position);
                                    if let RiffSetType::RiffArrangement(riff_arrangement_uuid) = riff_set_type.clone() {
                                        let _ = tx_from_ui.send(DAWEvents::RiffArrangementMoveRiffItemToPosition(riff_arrangement_uuid.clone(), riff_item_uuid.to_string(), drop_zone_child_position as usize));
                                    }
                                    break;
                                }
                            }
                            break;
                        }
                    }
                }
            });
        }
    }

    pub fn setup_riff_view_drag_and_drop(
        item_box: Box, 
        horizontal_adjustment: Adjustment,
        view_port: Viewport,
        _tx_from_ui: crossbeam_channel::Sender<DAWEvents>
    ){
        item_box.drag_dest_set(
            DestDefaults::ALL, 
            DRAG_N_DROP_TARGETS.as_ref(), 
            gdk::DragAction::COPY
        );

        item_box.connect_drag_motion(move |_, _ , x , _y, _| {
            if let Some(window) = view_port.window() {
                let view_port_width = window.width();
                let horizontal_adjustment_position = horizontal_adjustment.value() as i32;

                // debug!("Dragging a riff set: view_port_width={}, horizon_adjustment_position={}, x={}, y={}, horizontal_adjustment_position + view_port_width - x={}, x - horizontal_adjustment_position={}", view_port_width, riff_set_horizontal_adjustment.value(), x, y, horizontal_adjustment_position + view_port_width - x, x - horizontal_adjustment_position);
    
                if (horizontal_adjustment_position + view_port_width - x) <= 50 {
                    horizontal_adjustment.set_value((horizontal_adjustment_position + 50) as f64);
                }
                else if (x - horizontal_adjustment_position) < 50 && horizontal_adjustment_position >= 50 {
                    horizontal_adjustment.set_value((horizontal_adjustment_position - 50) as f64);
                }
            }

            true
        });

        item_box.connect_drag_data_received(move |item_box, _, x, y, selection_data, _, _| {
            debug!("riff view drag data received: x={}, y={}", x, y);
            if let Some(track_uuid) = selection_data.text() {
                // get the child at x and y
                for child in item_box.children().iter() {
                    if child.allocation().x <= x && 
                        x <= (child.allocation().x + child.allocation().width) &&
                        child.allocation().y <= y && 
                        y <= (child.allocation().y + child.allocation().height) {
                        let drop_zone_child_position = item_box.child_position(child);
                        
                        // move the dropped child to the found position
                        for child in item_box.children().iter() {
                            if child.widget_name() == track_uuid {
                                item_box.set_child_position(child, drop_zone_child_position);
                                // let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::TrackMoveToPosition(drop_zone_child_position as usize), Some(track_uuid.to_string())));
                                break;
                            }
                        }
                        break;
                    }
                }
            }
        });
    }

    pub fn setup_tracks_drag_and_drop(
        vertical_box: Box, 
        vertical_adjustment: Adjustment,
        view_port: Viewport,
        tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
    ){
        vertical_box.drag_dest_set(
            DestDefaults::ALL, 
            DRAG_N_DROP_TARGETS.as_ref(), 
            gdk::DragAction::COPY
        );

        vertical_box.connect_drag_motion(move |_, _ , x , y, _| {
            if let Some(window) = view_port.window() {
                let view_port_width = window.width();
                let vertical_adjustment_position = vertical_adjustment.value() as i32;

                debug!("Dragging a track: view_port_width={}, tracks_vertical_adjustment_position={}, x={}, y={}, tracks_vertical_adjustment_position + view_port_width - y={}, y - tracks_vertical_adjustment_position={}", view_port_width, vertical_adjustment.value(), x, y, vertical_adjustment_position + view_port_width - y, y - vertical_adjustment_position);
    
                if (vertical_adjustment_position + view_port_width - y) <= 50 {
                    vertical_adjustment.set_value((vertical_adjustment_position + 50) as f64);
                }
                else if (y - vertical_adjustment_position) < 50 && vertical_adjustment_position >= 50 {
                    vertical_adjustment.set_value((vertical_adjustment_position - 50) as f64);
                }
            }

            true
        });

        let tx_from_ui = tx_from_ui.clone();
        vertical_box.connect_drag_data_received(move |vertical_box, _, x, y, selection_data, _, _| {
            debug!("track drag data received: x={}, y={}", x, y);
            if let Some(track_uuid) = selection_data.text() {
                // get the child at x and y
                for child in vertical_box.children().iter() {
                    if child.allocation().x <= x && 
                        x <= (child.allocation().x + child.allocation().width) &&
                        child.allocation().y <= y && 
                        y <= (child.allocation().y + child.allocation().height) {
                        let drop_zone_child_position = vertical_box.child_position(child);
                        
                        // move the dropped child to the found position
                        for child in vertical_box.children().iter() {
                            if child.widget_name() == track_uuid {
                                // top_level_vbox.set_child_position(child, drop_zone_child_position);
                                let _ = tx_from_ui.send(DAWEvents::TrackChange(TrackChangeType::TrackMoveToPosition(drop_zone_child_position as usize), Some(track_uuid.to_string())));
                                break;
                            }
                        }
                        break;
                    }
                }
            }
        });
    }
}