use crate::domain::Track;
use crate::event::{AudioLayerEvent, AudioLayerInwardEvent, CurrentView, TrackBackgroundProcessorInwardEvent};
use crate::state::RiffDAWState;

pub fn transport_goto_start_action(state: &mut RiffDAWState) {
    match state.get_project().lock() {
        Ok(mut project) => {
            let bpm = project.song().tempo();
            let time_signature_numerator = project.song().time_signature_numerator();
            let sample_rate = state.configuration.audio.sample_rate as f64;;
            let play_position_in_frames = 0.0;
            let play_position_in_beats = play_position_in_frames / sample_rate * bpm / 60.0;
            let current_bar = play_position_in_beats as i32 / time_signature_numerator as i32 + 1;
            let current_beat_in_bar = play_position_in_beats as i32 % time_signature_numerator as i32 + 1;

            state.set_play_position_in_frames(play_position_in_frames as u32);

            // gui.ui.song_position_txt_ctrl.set_label(format!("{:03}:{:03}:000", current_bar, current_beat_in_bar).as_str());

            let time_in_secs = play_position_in_frames / sample_rate;
            let minutes = time_in_secs as i32 / 60;
            let seconds = time_in_secs as i32 % 60;
            let milli_seconds = ((time_in_secs - (time_in_secs as u64) as f64) * 1000.0) as u64;
            // gui.ui.song_time_txt_ctrl.set_label(format!("{:03}:{:02}:{:03}", minutes, seconds, milli_seconds).as_str());

            // if let Some(piano_roll_grid) = gui.piano_roll_grid() {
            //     match piano_roll_grid.lock() {
            //         Ok(mut grid) => grid.set_track_cursor_time_in_beats(play_position_in_beats),
            //         Err(_) => (),
            //     }
            // }
            // if let Some(track_grid) = gui.track_grid() {
            //     match track_grid.lock() {
            //         Ok(mut grid) => grid.set_track_cursor_time_in_beats(play_position_in_beats),
            //         Err(_) => (),
            //     }
            // }
            // if let Some(sample_roll_grid) = gui.sample_roll_grid() {
            //     match sample_roll_grid.lock() {
            //         Ok(mut grid) => grid.set_track_cursor_time_in_beats(play_position_in_beats),
            //         Err(_) => (),
            //     }
            // }
            // if let Some(automation_grid) = gui.automation_grid() {
            //     match automation_grid.lock() {
            //         Ok(mut grid) => grid.set_track_cursor_time_in_beats(play_position_in_beats),
            //         Err(_) => (),
            //     }
            // }

            let song = project.song();
            let tracks = song.tracks();
            for track in tracks {
                state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::GotoStart);
            }
        },
        Err(_) => println!("Main - rx_ui processing loop - transport goto start - could not get lock on state"),
    }
}


pub fn transport_move_back_action(state: &mut RiffDAWState) {
    println!("Main - rx_ui processing loop - transport move back - received");
    match state.get_project().lock() {
        Ok(mut project) => {
            let bpm = project.song().tempo();
            let sample_rate = state.configuration.audio.sample_rate as f64;;
            let block_size = state.configuration.audio.block_size as f64;
            let time_signature_numerator = project.song().time_signature_numerator();
            let beats_per_bar = time_signature_numerator;
            let mut play_position_in_frames = state.play_position_in_frames();
            let frames_per_beat = sample_rate * 60.0 /bpm;
            let frames_in_measure = (frames_per_beat * beats_per_bar) as u32;

            if play_position_in_frames >= frames_in_measure {
                play_position_in_frames -= frames_in_measure;
            }
            else {
                play_position_in_frames = 0;
            }
            state.set_play_position_in_frames(play_position_in_frames);

            {
                let state = state;
                for track in project.song().tracks().iter() {
                    state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetBlockPosition((play_position_in_frames / (block_size as u32)) as i32));
                }
            }

            let play_position_in_beats = play_position_in_frames as f64 / sample_rate * bpm / 60.0;
            let current_bar = play_position_in_beats as i32 / time_signature_numerator as i32 + 1;
            let current_beat_in_bar = play_position_in_beats as i32 % time_signature_numerator as i32 + 1;
            // gui.ui.song_position_txt_ctrl.set_label(format!("{:03}:{:03}:000", current_bar, current_beat_in_bar).as_str());

            let time_in_secs = play_position_in_frames as f64 / sample_rate;
            let minutes = time_in_secs as i32 / 60;
            let seconds = time_in_secs as i32 % 60;
            let milli_seconds = ((time_in_secs - (time_in_secs as u64) as f64) * 1000.0) as u64;
            // gui.ui.song_time_txt_ctrl.set_label(format!("{:03}:{:02}:{:03}", minutes, seconds, milli_seconds).as_str());

            // if let Some(piano_roll_grid) = gui.piano_roll_grid() {
            //     match piano_roll_grid.lock() {
            //         Ok(mut grid) => grid.set_track_cursor_time_in_beats(play_position_in_beats),
            //         Err(_) => (),
            //     }
            // }
            // if let Some(track_grid) = gui.track_grid() {
            //     match track_grid.lock() {
            //         Ok(mut grid) => grid.set_track_cursor_time_in_beats(play_position_in_beats),
            //         Err(_) => (),
            //     }
            // }
            // if let Some(sample_roll_grid) = gui.sample_roll_grid() {
            //     match sample_roll_grid.lock() {
            //         Ok(mut grid) => grid.set_track_cursor_time_in_beats(play_position_in_beats),
            //         Err(_) => (),
            //     }
            // }
            // if let Some(automation_grid) = gui.automation_grid() {
            //     match automation_grid.lock() {
            //         Ok(mut grid) => grid.set_track_cursor_time_in_beats(play_position_in_beats),
            //         Err(_) => (),
            //     }
            // }
        },
        Err(_) => println!("Main - rx_ui processing loop - play position in beats - could not get lock on state"),
    }
}


pub fn transport_stop_action(state: &mut RiffDAWState) {
state.set_playing(false);
    if let Some(playing_riff_set_uuid) = state.playing_riff_set() {
        // gui.repaint_riff_set_view_riff_set_active_drawing_areas(playing_riff_set_uuid, 0.0);
        state.set_playing_riff_set(None);
    }
    if let Some(playing_riff_sequence_uuid) = state.playing_riff_sequence() {
        // let playing_riff_sequence_summary_data = (0.0, vec![]);
        // gui.repaint_riff_sequence_view_riff_sequence_active_drawing_areas(playing_riff_sequence_uuid, 0.0, &playing_riff_sequence_summary_data);
        state.set_playing_riff_sequence(None);
    }
    if let Some(_) = state.playing_riff_grid() {
        // gui.repaint_riff_grid_view_drawing_area(0.0);
        state.set_playing_riff_grid(None);
    }
    if let Some(playing_riff_arrangement_uuid) = state.playing_riff_arrangement() {
        // let playing_riff_arrangement_summary_data = (0.0, vec![]);
        // gui.repaint_riff_arrangement_view_active_drawing_areas(playing_riff_arrangement_uuid, 0.0, &playing_riff_arrangement_summary_data);
        state.set_playing_riff_arrangement(None);
    }
    match state.project.lock() {
        Ok(project) => {
            let song = project.song();
            let song_length_in_beats = song.length_in_beats() as f64;
            let tracks = song.tracks();
            let bpm = song.tempo();
            let sample_rate = state.configuration.audio.sample_rate as f64;
            let block_size = state.configuration.audio.block_size as f64;
            for track in tracks {
                state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Stop);
            }
            let number_of_blocks = (song_length_in_beats / bpm * 60.0 * sample_rate / block_size) as i32;
            if let Some(audio_layer_sender) = state.audio_layer_sender.as_ref() {
                match audio_layer_sender.send(AudioLayerEvent::AudioLayerInward(AudioLayerInwardEvent::Play(false, number_of_blocks, 0))) {
                    Ok(_) => (),
                    Err(error) => println!("Problem using tx_to_audio to send message to jack layer when stopping play: {}", error),
                }
            }
        },
        Err(_) => println!("Main - rx_ui processing loop - transport stop - could not get lock on state"),
    };
}


pub fn transport_play_action(state: &mut RiffDAWState) {

    // {
    //     let mut time_info = vst_host_time_info.write();
    //     time_info.sample_pos = 0.0;
    // }

    match state.current_view() {
        CurrentView::Track => {
            state.play_song();
        }
        CurrentView::RiffSet => {
            let riff_set_uuid = if let Some(playing_riff_set_uuid) = state.playing_riff_set() {
                playing_riff_set_uuid.to_string()
            }
            else if let Some(riff_set) = state.project.lock().unwrap().song_mut().riff_sets_mut().get_mut(0) {
                riff_set.uuid()
            } else {
                "".to_string()
            };
            state.set_playing_riff_set(Some(riff_set_uuid.clone()));
            state.play_riff_set(riff_set_uuid);
        }
        CurrentView::RiffSequence => {
            let riff_sequence_uuid = if let Some(selected_riff_sequence_uuid) = state.riff_sequence_view_state.selected_riff_sequence_uuid() {
                selected_riff_sequence_uuid.to_string()
            }
            else if let Some(riff_sequence) = state.project.lock().unwrap().song_mut().riff_sequences_mut().get_mut(0) {
                riff_sequence.uuid()
            } else {
                "".to_string()
            };
            state.set_playing_riff_sequence(Some(riff_sequence_uuid.clone()));
            state.play_riff_sequence(riff_sequence_uuid);
        }
        CurrentView::RiffGrid => {
            let riff_grid_uuid = if let Some(riff_grid_uuid) = state.riff_grid_view_state.selected_riff_grid_uuid() {
                riff_grid_uuid.to_string()
            }
            else if let Some(riff_grid) = state.project.lock().unwrap().song_mut().riff_grids_mut().get_mut(0) {
                riff_grid.uuid()
            } else {
                "".to_string()
            };
            state.set_playing_riff_grid(Some(riff_grid_uuid.clone()));
            state.play_riff_grid(riff_grid_uuid);
        }
        CurrentView::RiffArrangement => {
            let riff_arrangement_uuid = if let Some(selected_riff_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                selected_riff_arrangement_uuid.to_string()
            }
            else if let Some(riff_arrangement) = state.project.lock().unwrap().song_mut().riff_arrangements_mut().get_mut(0) {
                riff_arrangement.uuid()
            } else {
                "".to_string()
            };
            state.set_playing_riff_arrangement(Some(riff_arrangement_uuid.clone()));
            state.play_riff_arrangement(riff_arrangement_uuid, 0.0);
        }
    }
}


pub fn transport_record_on_action(state: &mut RiffDAWState) {
state.set_recording(true);
}


pub fn transport_record_off_action(state: &mut RiffDAWState) {
state.set_recording(false);
}


pub fn transport_pause_action(state: &mut RiffDAWState) {
    match state.project.lock() {
        Ok(project) => {
            let song = project.song();
            let tracks = song.tracks();
            for track in tracks {
                state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Pause);
            }
        },
        Err(_) => println!("Main - rx_ui processing loop - transport pause - could not get lock on state"),
    };
}


pub fn transport_move_forward_action(state: &mut RiffDAWState) {
    println!("Main - rx_ui processing loop - transport move forward - received");
    match state.get_project().lock() {
        Ok(mut project) => {
            let bpm = project.song().tempo();
            let sample_rate = state.configuration.audio.sample_rate as f64;;
            let block_size = state.configuration.audio.block_size as f64;
            let time_signature_numerator = project.song().time_signature_numerator();
            let beats_per_bar = time_signature_numerator;
            let mut play_position_in_frames = state.play_position_in_frames();
            let frames_per_beat = sample_rate * 60.0 /bpm;
            let frames_in_measure = (frames_per_beat * beats_per_bar) as u32;

            play_position_in_frames += frames_in_measure;
            state.set_play_position_in_frames(play_position_in_frames);

            {
                let state = state;
                for track in project.song().tracks().iter() {
                    state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::SetBlockPosition((play_position_in_frames / (block_size as u32)) as i32));
                }
            }
            let play_position_in_beats = play_position_in_frames as f64 / sample_rate * bpm / 60.0;
            let current_bar = play_position_in_beats as i32 / time_signature_numerator as i32 + 1;
            let current_beat_in_bar = play_position_in_beats as i32 % time_signature_numerator as i32 + 1;
            // gui.ui.song_position_txt_ctrl.set_label(format!("{:03}:{:03}:000", current_bar, current_beat_in_bar).as_str());

            let time_in_secs = play_position_in_frames as f64 / sample_rate;
            let minutes = time_in_secs as i32 / 60;
            let seconds = time_in_secs as i32 % 60;
            let milli_seconds = ((time_in_secs - (time_in_secs as u64) as f64) * 1000.0) as u64;
            // gui.ui.song_time_txt_ctrl.set_label(format!("{:03}:{:02}:{:03}", minutes, seconds, milli_seconds).as_str());

            // if let Some(piano_roll_grid) = gui.piano_roll_grid() {
            //     match piano_roll_grid.lock() {
            //         Ok(mut grid) => grid.set_track_cursor_time_in_beats(play_position_in_beats),
            //         Err(_) => (),
            //     }
            // }
            // if let Some(track_grid) = gui.track_grid() {
            //     match track_grid.lock() {
            //         Ok(mut grid) => grid.set_track_cursor_time_in_beats(play_position_in_beats),
            //         Err(_) => (),
            //     }
            // }
            // if let Some(sample_roll_grid) = gui.sample_roll_grid() {
            //     match sample_roll_grid.lock() {
            //         Ok(mut grid) => grid.set_track_cursor_time_in_beats(play_position_in_beats),
            //         Err(_) => (),
            //     }
            // }
            // if let Some(automation_grid) = gui.automation_grid() {
            //     match automation_grid.lock() {
            //         Ok(mut grid) => grid.set_track_cursor_time_in_beats(play_position_in_beats),
            //         Err(_) => (),
            //     }
            // }
        },
        Err(_) => println!("Main - rx_ui processing loop - play position in beats - could not get lock on state"),
    }
}


pub fn transport_goto_end_action(state: &mut RiffDAWState) {
    match state.project.lock() {
        Ok(project) => {
            let song = project.song();
            let tracks = song.tracks();
            for track in tracks {
                state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::GotoEnd);
            }
        },
        Err(_) => println!("Main - rx_ui processing loop - transport goto end - could not get lock on state"),
    };
}


