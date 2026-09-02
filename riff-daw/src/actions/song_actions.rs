use log::debug;
use crate::domain::{AudioLayerInwardEvent, Track, TrackBackgroundProcessorInwardEvent};
use crate::event::AudioLayerEvent;
use crate::state::RiffDAWState;

pub fn daw_events_TempoChange(state: &mut RiffDAWState, tempo: f64) {
    match state.get_project().lock().as_mut() {
        Ok(mut project) => {
            project.song_mut().set_tempo(tempo);

            if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
                match audio_layer_sender.send(AudioLayerEvent::AudioLayerInward(AudioLayerInwardEvent::Tempo(project.song().tempo()))) {
                    Ok(_) => (),
                    Err(error) => debug!("Problem using tx_to_audio to send tempo message to jack layer: {}", error),
                }
            }
            for track in project.song().tracks().iter() {
                state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Tempo(tempo));
            }
        },
        Err(_) => debug!("Main - rx_ui processing loop - tempo change - could not get lock on state"),
    };
}

pub fn daw_events_TimeSignatureNumeratorChange(state: &mut RiffDAWState, time_signature_numerator: f64) {
    match state.get_project().lock().as_mut() {
        Ok(mut project) => {
            let denominator = project.song_mut().time_signature_denominator();
            project.song_mut().set_time_signature_numerator(time_signature_numerator);

            for track in project.song().tracks().iter() {
                state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::TimeSignatureChange(time_signature_numerator as u32, denominator as u32));
            }

            // gui.ui.piano_roll_drawing_area.queue_draw();
            // gui.ui.piano_roll_ruler_drawing_area.queue_draw();
            // gui.ui.track_drawing_area.queue_draw();
            // gui.ui.track_ruler_drawing_area.queue_draw();
            // gui.ui.automation_drawing_area.queue_draw();
            // gui.ui.automation_ruler_drawing_area.queue_draw();
            // gui.ui.riff_grid_drawing_area.queue_draw();
            // gui.ui.riff_grid_ruler_drawing_area.queue_draw();
        },
        Err(_) => debug!("Main - rx_ui processing loop - time signature numerator change - could not get lock on state"),
    };
}

pub fn daw_events_TimeSignatureDenominatorChange(state: &mut RiffDAWState, time_signature_denominator: f64) {
    match state.get_project().lock().as_mut() {
        Ok(mut project) => {
            let numerator = project.song_mut().time_signature_numerator();
            project.song_mut().set_time_signature_denominator(time_signature_denominator);

            for track in project.song().tracks().iter() {
                state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::TimeSignatureChange(numerator as u32, time_signature_denominator as u32));
            }
        },
        Err(_) => debug!("Main - rx_ui processing loop - time signature denominator change - could not get lock on state"),
    };
}
