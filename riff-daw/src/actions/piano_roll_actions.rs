use itertools::Itertools;
use log::debug;
use crate::domain::{DAWItemID, Track, TrackEvent};
use crate::event::OperationModeType;
use crate::state::{MidiPolyphonicExpressionNoteId, RiffDAWState};

pub fn daw_events_PianoRollMPENoteIdChange(state: &mut RiffDAWState, mpe_note_id_change: MidiPolyphonicExpressionNoteId) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let new_note_id = mpe_note_id_change.clone() as i32;
            state.set_piano_roll_mpe_note_id(mpe_note_id_change);

            // if a riff is selected and notes in the riff are selected then change there note id
            let selected_riff_events = state.selected_riff_events().iter().map(|id| id.clone()).collect_vec();
            if let Some(track_uuid) = state.selected_track() {
                if let Some(riff_uuid) = state.selected_riff_uuid(track_uuid.clone()) {
                    if let Some(track) = project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                        if let Some(riff) = track.riffs_mut().iter_mut().find(|riff| riff.id() == riff_uuid) {
                            for event in riff.events_mut().iter_mut().filter(|event| selected_riff_events.contains(&event.id())) {
                                if let TrackEvent::Note(note) = event {
                                    note.set_note_id(new_note_id);
                                }
                            }
                        }
                    }
                }
            }

            // gui.ui.piano_roll_drawing_area.queue_draw();
        }
        Err(_) => debug!("Main - rx_ui processing loop - PianoRollMPENoteIdChange - could not get lock on state"),
    }
}

pub fn daw_events_PianoRollWindowedZoom(state: &mut RiffDAWState, x1: f64, y1: f64, x2: f64, y2: f64) { // values are in pixels
    // if let Some(widget) = gui.ui.piano_roll_scrolled_window.child() {
    //     if let Some(view_port) = widget.dynamic_cast_ref::<Viewport>() {
    //         let width = view_port.allocated_width();
    //         let height = view_port.allocated_height();
    //         let window_width = x2 - x1;
    //         let window_height = y2 - y1;
    //         let horizontal_scale_up = width as f64 / window_width;
    //         let vertical_scale_up = height as f64 / window_height;
    //
    //         if let Some(grid_arc) = gui.piano_roll_grid.clone() {
    //             if let Ok(mut grid) = grid_arc.lock() {
    //                 let zoom_horizontal = grid.zoom_horizontal();
    //                 let zoom_vertical = grid.zoom_vertical();
    //                 let adjusted_horizontal_zoom = zoom_horizontal * horizontal_scale_up;
    //                 let adjusted_vertical_zoom = zoom_vertical * vertical_scale_up;
    //
    //                 grid.set_horizontal_zoom(zoom_horizontal * horizontal_scale_up);
    //                 grid.set_vertical_zoom(zoom_vertical * vertical_scale_up);
    //
    //
    //                 // need to adjust the gtk scale widget adjustments (ranges) - probably should do this rather than setting the zoom directly
    //
    //
    //                 // need to scroll the zoom window into view
    //
    //
    //             }
    //         }
    //     }
    // }
}

pub fn daw_events_PianoRollOperationModeChange(state: &mut RiffDAWState, mode: OperationModeType) {
    debug!("Event: PianoRollOperationModeChange");
}

pub fn daw_events_RepaintPianoRollView (state: &mut RiffDAWState) {
    // gui.ui.piano_roll_drawing_area.queue_draw();
}
