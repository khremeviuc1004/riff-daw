use log::debug;
use crate::constants::TRACK_VIEW_TRACK_PANEL_HEIGHT;
use crate::event::OperationModeType;
use crate::state::RiffDAWState;

pub fn daw_events_TrackGridOperationModeChange(state: &mut RiffDAWState, mode: OperationModeType) {
    debug!("Event: TrackGridOperationModeChange");
}

pub fn daw_events_TrackGridVerticalScaleChanged(state: &mut RiffDAWState, vertical_scale: f64) {

    // let widget_height = (TRACK_VIEW_TRACK_PANEL_HEIGHT as f64 * vertical_scale) as i32;
    // for track_panel in gui.ui.top_level_vbox.children().iter_mut() {
    //     debug!("Track grid - Track panel height: {}", track_panel.allocation().height);
    //     track_panel.set_height_request(widget_height);
    // }
    // gui.ui.track_panel_scrolled_window.queue_draw();
    // gui.ui.top_level_vbox.queue_draw();
    // gui.ui.track_drawing_area.queue_draw();
}

pub fn daw_events_RepaintTrackGridView (state: &mut RiffDAWState) {
    // gui.ui.track_drawing_area.queue_draw();
}
