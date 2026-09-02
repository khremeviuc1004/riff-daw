use log::debug;
use uuid::Uuid;
use crate::domain::{AudioLayerInwardEvent, Loop, Track, TrackBackgroundProcessorInwardEvent};
use crate::event::AudioLayerEvent;
use crate::state::RiffDAWState;

pub fn loop_change_type_LoopOn(state: &mut RiffDAWState) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let mut start_block = 0;
            let mut end_block = 44;
            let sample_rate = state.configuration.audio.sample_rate as f64;
            let block_size = state.configuration.audio.block_size as f64;
            let song = project.song();
            let tracks = song.tracks();

            match state.active_loop() {
                Some(active_loop_uuid) => {
                    match song.loops().iter().find(|current_loop| current_loop.uuid().to_string() == active_loop_uuid.to_string()) {
                        Some(active_loop) => {
                            start_block = (active_loop.start_position() * sample_rate / block_size) as i32;
                            end_block = (active_loop.end_position() * sample_rate / block_size) as i32;
                        },
                        None => debug!("Could not find the active loop."),
                    }
                },
                None => debug!("No active loop found to set left position."),
            }
            if let Some(audio_layer_sender) = state.audio_layer_sender.as_ref() {
                match audio_layer_sender.send(AudioLayerEvent::AudioLayerInward(AudioLayerInwardEvent::ExtentsChange(end_block - start_block))) {
                    Ok(_) => (),
                    Err(error) => debug!("Problem using tx_to_audio to send message to jack layer when turning looping on: {}", error),
                }
            }
            for track in tracks {
                state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::LoopExtents(start_block, end_block));
                state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Loop(true));
            }

            {
                let mut state = state;
                state.set_looping(true);
            }
        },
        Err(_) => debug!("Main - rx_ui processing loop - set active loop - could not get lock on state"),
    }
    // gui.ui.track_drawing_area.queue_draw();
}

pub fn loop_change_type_LoopOff(state: &mut RiffDAWState) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            state.set_looping(false);
            let song = project.song();
            let tracks = song.tracks();
            for track in tracks {
                state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Loop(false));
            }
        },
        Err(_) => debug!("Main - rx_ui processing loop - set active loop - could not get lock on state"),
    }
    // gui.ui.track_drawing_area.queue_draw();
}

pub fn loop_change_type_ActiveLoopChanged(state: &mut RiffDAWState, uuid: Option<Uuid>) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let mut start_block = 0;
            let mut end_block = 44;
            let sample_rate = state.configuration.audio.sample_rate as f64;
            let block_size = state.configuration.audio.block_size as f64;
            let song = project.song();
            let tracks = song.tracks();

            match uuid {
                Some(active_loop_uuid) => {
                    match song.loops().iter().find(|current_loop| current_loop.uuid().to_string() == active_loop_uuid.to_string()) {
                        Some(active_loop) => {
                            start_block = (active_loop.start_position() * sample_rate / block_size) as i32;
                            end_block = (active_loop.end_position() * sample_rate / block_size) as i32;
                        },
                        None => debug!("Could not find the active loop."),
                    }
                },
                None => debug!("No loop found to mark as active."),
            }
            for track in tracks {
                state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::LoopExtents(start_block, end_block));
            }
            {
                let mut state = state;
                state.set_active_loop(uuid);
            }
        },
        Err(_) => debug!("Main - rx_ui processing loop - set active loop - could not get lock on state"),
    }
    // gui.ui.track_drawing_area.queue_draw();
}

pub fn loop_change_type_LoopLimitLeftChanged(state: &mut RiffDAWState, start_position: f64) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let sample_rate = state.configuration.audio.sample_rate as f64;
            let block_size = state.configuration.audio.block_size as f64;

            match state.active_loop() {
                Some(active_loop_uuid) => {
                    let song = project.song();
                    let tracks = song.tracks();
                    match song.loops().iter().find(|current_loop| current_loop.uuid().to_string() == active_loop_uuid.to_string()) {
                        Some(active_loop) => {
                            let start_block = (start_position * sample_rate / block_size) as i32;
                            let end_block = (active_loop.end_position() * sample_rate / block_size) as i32;
                            for track in tracks {
                                state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::LoopExtents(start_block, end_block));
                            }
                        },
                        None => debug!("Could not find the active loop."),
                    }
                    match project.song_mut().loops_mut().iter_mut().find(|current_loop| current_loop.uuid().to_string() == active_loop_uuid.to_string()) {
                        Some(active_loop) => {
                            active_loop.set_start_position(start_position);
                        },
                        None => debug!("Could not find the active loop."),
                    }
                },
                None => debug!("No active loop found to set left position."),
            }
        },
        Err(_) => debug!("Main - rx_ui processing loop - loop add - could not get lock on state"),
    }
    // gui.ui.track_drawing_area.queue_draw();
}

pub fn loop_change_type_LoopLimitRightChanged(state: &mut RiffDAWState, end_position: f64) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let sample_rate = state.configuration.audio.sample_rate as f64;
            let block_size = state.configuration.audio.block_size as f64;

            match state.active_loop() {
                Some(active_loop_uuid) => {
                    let song = project.song();
                    let tracks = song.tracks();
                    match song.loops().iter().find(|current_loop| current_loop.uuid().to_string() == active_loop_uuid.to_string()) {
                        Some(active_loop) => {
                            let start_block = (active_loop.start_position() * sample_rate / block_size) as i32;
                            let end_block = (end_position * sample_rate / block_size) as i32;
                            for track in tracks {
                                state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::LoopExtents(start_block, end_block));
                            }
                        },
                        None => debug!("Could not find the active loop."),
                    }
                    let mut state = state;
                    match project.song_mut().loops_mut().iter_mut().find(|current_loop| current_loop.uuid().to_string() == active_loop_uuid.to_string()) {
                        Some(active_loop) => {
                            active_loop.set_end_position(end_position);
                        },
                        None => debug!("Could not find the active loop."),
                    }
                },
                None => debug!("No active loop found to set right position."),
            }
        },
        Err(_) => debug!("Main - rx_ui processing loop - loop add - could not get lock on state"),
    }
    // gui.ui.track_drawing_area.queue_draw();
}

pub fn loop_change_type_Added(state: &mut RiffDAWState, loop_name: String, uuid: Uuid) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            project.song_mut().add_loop(Loop::new_with_uuid_and_name(uuid, loop_name));
        },
        Err(_) => debug!("Main - rx_ui processing loop - loop add - could not get lock on state"),
    }
    // gui.ui.track_drawing_area.queue_draw();
}

pub fn loop_change_type_Deleted(state: &mut RiffDAWState, uuid: Uuid) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            project.song_mut().delete_loop(uuid);
        },
        Err(_) => debug!("Main - rx_ui processing loop - loop delete - could not get lock on state"),
    }
    // gui.ui.track_drawing_area.queue_draw();
}

pub fn loop_change_type_NameChanged(state: &mut RiffDAWState, name: String, uuid: Uuid) {
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            project.song_mut().change_loop_name(uuid, name);
        },
        Err(_) => debug!("Main - rx_ui processing loop - loop name change - could not get lock on state"),
    }
}
