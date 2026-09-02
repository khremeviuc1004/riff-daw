use crate::state::RiffDAWState;

pub fn daw_events_SampleRollSetTrackName(state: &mut RiffDAWState, name: String, track_uuid: String) {
    // gui.set_sample_roll_selected_track_name_label(name.as_str());
    // gui.ui.sample_roll_drawing_area.queue_draw();
}

pub fn daw_events_SampleRollSetRiffName(state: &mut RiffDAWState, name: String, track_uuid: String) {
    // gui.set_sample_roll_selected_riff_name_label(name.as_str());
    // gui.ui.sample_roll_drawing_area.queue_draw();
}

pub fn daw_events_RepaintSampleRollDrawingArea (state: &mut RiffDAWState) {
    // gui.ui.sample_roll_drawing_area.queue_draw();
}
