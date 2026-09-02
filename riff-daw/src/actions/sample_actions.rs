use log::debug;
use crate::domain::{AudioLayerInwardEvent, Sample, SampleData};
use crate::event::AudioLayerEvent;
use crate::state::RiffDAWState;

pub fn daw_events_PreviewSample(state: &mut RiffDAWState, file_name: String) {
    if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
        match audio_layer_sender.send(AudioLayerEvent::AudioLayerInward(AudioLayerInwardEvent::PreviewSample(file_name))) {
            Ok(_) => {}
            Err(_) => {}
        }
    }
}

pub fn daw_events_SampleAdd(state: &mut RiffDAWState, file_name: String) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            // create the sample object and store it
            let sample_data = SampleData::new(
                file_name.clone(),
                state.configuration.audio.sample_rate,
            );
            let sample = Sample::new(
                file_name.clone(),
                file_name,
                sample_data.uuid().to_string(),
            );

            project.song_mut().samples_mut().insert(sample.uuid().to_string(), sample.clone());
            state.sample_data_mut().insert(sample_data.uuid().to_string(), sample_data);
            debug!("Added sample: id={}, text={}, uuid={}", sample.file_name(), sample.name(), sample.uuid());

            // update the sample roll browser list store
            // gui.update_sample_roll_sample_browser(sample.uuid().to_string(), sample.name().to_string());
        }
        Err(_) => {}
    }
}

pub fn daw_events_SampleDelete(state: &mut RiffDAWState, _uuid: String) {}
