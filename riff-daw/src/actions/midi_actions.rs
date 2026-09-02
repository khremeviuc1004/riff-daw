use log::debug;
use vst::event::MidiEvent;
use crate::actions::{transport_move_back_action, transport_move_forward_action, transport_play_action, transport_stop_action};
use crate::domain::{Controller, DAWItemLength, DAWItemPosition, Note, PitchBend, PlayMode, Track, TrackEvent, TrackType};
use crate::event::{CurrentView, DAWEvents, TrackBackgroundProcessorInwardEvent, TrackChangeType};
use crate::state::{MidiPolyphonicExpressionNoteId, RiffDAWState};

pub fn midi_AudioLayerTimeCriticalOutwardEvent_MidiEvent(state: &mut RiffDAWState, jack_midi_event: MidiEvent) {
    let midi_msg_type = jack_midi_event.data[0] as i32;

    // match state.get_project().lock() {
    //     Ok(project) => {
    //         match state.selected_track() {
    //             Some(track_uuid) => {
    //                 match project.song().tracks().iter().find(|track| track.uuid().to_string() == track_uuid) {
    //                     Some(track) => {
    //                         let midi_channel = if let TrackType::MidiTrack(midi_track) = track {
    //                             midi_track.midi_device().midi_channel()
    //                         } else {
    //                             0
    //                         };
    //                         if (144..=159).contains(&midi_msg_type) {
    //                             state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::PlayNoteImmediate(jack_midi_event.data[1] as i32, midi_channel));
    //                         } else if (128..=143).contains(&midi_msg_type) {
    //                             state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::StopNoteImmediate(jack_midi_event.data[1] as i32, midi_channel));
    //                         } else if (176..=191).contains(&midi_msg_type) {
    //                             state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::PlayControllerImmediate(jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32, midi_channel));
    //                         } else if (224..=239).contains(&midi_msg_type) {
    //                             state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::PlayPitchBendImmediate(jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32, midi_channel));
    //                         } else {
    //                             debug!("Unknown jack midi event: ");
    //                             for event_byte in jack_midi_event.data.iter() {
    //                                 debug!(" {}", event_byte);
    //                             }
    //                             debug!("");
    //                         }
    //                     },
    //                     None => (),
    //                 };
    //             },
    //             None => debug!("Play note immediate: no track number given."),
    //         }
    //     },
    //     Err(_) => debug!("Main - jack_event_prcessing_thread processing loop - play note immediate - could not get lock on state"),
    // }
    let mut selected_riff_uuid = None;
    let mut selected_riff_track_uuid = None;

    selected_riff_track_uuid = state.selected_track();

    match selected_riff_track_uuid {
        Some(track_uuid) => {
            selected_riff_uuid = state.selected_riff_uuid(track_uuid.clone());
            selected_riff_track_uuid = Some(track_uuid);
        },
        None => (),
    }

    match state.get_project().lock().as_mut() {
        Ok(project) => {
            let tempo = project.song().tempo();
            let sample_rate = state.configuration.audio.sample_rate as f64;;
            let current_view = state.current_view().clone();
            let selected_riff_arrangement_uuid = if let Some(selected_riff_arrangement_uuid) = state.selected_riff_arrangement_uuid() {
                Some(selected_riff_arrangement_uuid.to_string())
            }
            else {
                None
            };
            let playing = state.playing();
            let recording = state.recording();

            if playing && recording {
                let play_mode = state.play_mode();
                let playing_riff_set = state.playing_riff_set().clone();
                let mut riff_changed = false;

                match selected_riff_track_uuid {
                    Some(track_uuid) => {
                        match project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == *track_uuid) {
                            Some(track_type) => match track_type {
                                TrackType::InstrumentTrack(track) => {
                                    match current_view {
                                        CurrentView::Track => {
                                            match selected_riff_uuid {
                                                Some(uuid) => {
                                                    for riff in track.riffs_mut().iter_mut() {
                                                        if riff.uuid().to_string() == *uuid {
                                                            if (144..=159).contains(&midi_msg_type) { //note on
                                                                let actual_position = tempo / 60.0 * jack_midi_event.delta_frames as f64 / sample_rate;
                                                                let adjusted_position = ((actual_position * 1000.0) as i32 % ((riff.length() * 1000.0) as i32)) as f64 / 1000.0;
                                                                debug!(
                                                                                                    "Adding note to riff: delta frames={}, actual_position={}, adjusted_position={}, note={}, velocity={}",
                                                                                                    jack_midi_event.delta_frames,
                                                                                                    actual_position,
                                                                                                    adjusted_position,
                                                                                                    jack_midi_event.data[1] as i32,
                                                                                                    jack_midi_event.data[2] as i32);
                                                                let note = Note::new_with_params(
                                                                    MidiPolyphonicExpressionNoteId::ALL as i32, adjusted_position, jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32, 0.2);
                                                                state.recorded_playing_notes.insert(note.note(), note.position());
                                                                riff.events_mut().push(TrackEvent::Note(note));
                                                                riff.events_mut().sort_by(|a, b| a.position().partial_cmp(&b.position()).unwrap());
                                                            } else if (128..=143).contains(&midi_msg_type) { // note off
                                                                let note_number = jack_midi_event.data[1] as i32;
                                                                if let Some(note_position) = state.recorded_playing_notes.get_mut(&note_number) {
                                                                    let actual_position = tempo / 60.0 * jack_midi_event.delta_frames as f64 / sample_rate;
                                                                    let adjusted_position = ((actual_position * 1000.0) as i32 % ((riff.length() * 1000.0) as i32)) as f64 / 1000.0;
                                                                    // find the note in the riff
                                                                    for track_event in riff.events_mut().iter_mut() {
                                                                        if track_event.position() == *note_position {
                                                                            if let TrackEvent::Note(note) = track_event {
                                                                                if note.note() == note_number {
                                                                                    note.set_length(adjusted_position - note.position());
                                                                                    riff_changed = true;
                                                                                    break;
                                                                                }
                                                                            }
                                                                        }
                                                                    }
                                                                }
                                                                state.recorded_playing_notes.remove(&note_number);
                                                            }

                                                            break;
                                                        }
                                                    }
                                                },
                                                None => debug!("Jack midi receiver - no selected riff."),
                                            }
                                            // add the controller events to the track automation
                                            if (176..=191).contains(&midi_msg_type) { // Controller - including modulation wheel
                                                debug!("Adding controller to track automation: delta frames={}, controller={}, value={}", jack_midi_event.delta_frames, jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32);
                                                track.automation_mut().events_mut().push(
                                                    TrackEvent::Controller(
                                                        Controller::new(
                                                            tempo / 60.0 * jack_midi_event.delta_frames as f64 / sample_rate, jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32)));
                                                track.automation_mut().events_mut().sort_by(|a, b| a.position().partial_cmp(&b.position()).unwrap());
                                            } else if (224..=239).contains(&midi_msg_type) {
                                                debug!("Adding pitch bend to track_automation: delta frames={}, lsb={}, msb={}", jack_midi_event.delta_frames, jack_midi_event.data[1], jack_midi_event.data[2]);
                                                track.automation_mut().events_mut().push(
                                                    TrackEvent::PitchBend(
                                                        PitchBend::new_from_midi_bytes(
                                                            tempo / 60.0 * jack_midi_event.delta_frames as f64 / sample_rate, jack_midi_event.data[1], jack_midi_event.data[2])));
                                                track.automation_mut().events_mut().sort_by(|a, b| a.position().partial_cmp(&b.position()).unwrap());
                                            }
                                        }
                                        CurrentView::RiffSet => {
                                            match selected_riff_uuid {
                                                Some(uuid) => {
                                                    for riff in track.riffs_mut().iter_mut() {
                                                        if riff.uuid().to_string() == *uuid {
                                                            if (144..=159).contains(&midi_msg_type) { //note on
                                                                let actual_position = tempo / 60.0 * jack_midi_event.delta_frames as f64 / sample_rate;
                                                                let adjusted_position = ((actual_position * 1000.0) as i32 % ((riff.length() * 1000.0) as i32)) as f64 / 1000.0;
                                                                debug!(
                                                                                                    "Adding note to riff: delta frames={}, actual_position={}, adjusted_position={}, note={}, velocity={}",
                                                                                                    jack_midi_event.delta_frames,
                                                                                                    actual_position,
                                                                                                    adjusted_position,
                                                                                                    jack_midi_event.data[1] as i32,
                                                                                                    jack_midi_event.data[2] as i32);
                                                                let note = Note::new_with_params(
                                                                    MidiPolyphonicExpressionNoteId::ALL as i32, adjusted_position, jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32, 0.2);
                                                                state.recorded_playing_notes.insert(note.note(), note.position());
                                                                riff.events_mut().push(TrackEvent::Note(note));
                                                                riff.events_mut().sort_by(|a, b| a.position().partial_cmp(&b.position()).unwrap());
                                                            } else if (128..=143).contains(&midi_msg_type) { // note off
                                                                let note_number = jack_midi_event.data[1] as i32;
                                                                if let Some(note_position) = state.recorded_playing_notes.get_mut(&note_number) {
                                                                    let actual_position = tempo / 60.0 * jack_midi_event.delta_frames as f64 / sample_rate;
                                                                    let adjusted_position = ((actual_position * 1000.0) as i32 % ((riff.length() * 1000.0) as i32)) as f64 / 1000.0;
                                                                    // find the note in the riff
                                                                    for track_event in riff.events_mut().iter_mut() {
                                                                        if track_event.position() == *note_position {
                                                                            if let TrackEvent::Note(note) = track_event {
                                                                                if note.note() == note_number {
                                                                                    note.set_length(adjusted_position - note.position());
                                                                                    riff_changed = true;
                                                                                    break;
                                                                                }
                                                                            }
                                                                        }
                                                                    }
                                                                }
                                                                state.recorded_playing_notes.remove(&note_number);
                                                            } else if (176..=191).contains(&midi_msg_type) { // Controller - including modulation wheel
                                                                debug!("Adding controller to riff: delta frames={}, controller={}, value={}", jack_midi_event.delta_frames, jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32);
                                                                riff.events_mut().push(
                                                                    TrackEvent::Controller(
                                                                        Controller::new(
                                                                            tempo / 60.0 * jack_midi_event.delta_frames as f64 / sample_rate, jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32)));
                                                                riff.events_mut().sort_by(|a, b| a.position().partial_cmp(&b.position()).unwrap());
                                                            } else if (224..=239).contains(&midi_msg_type) {
                                                                debug!("Adding pitch bend to riff: delta frames={}, lsb={}, msb={}", jack_midi_event.delta_frames, jack_midi_event.data[1], jack_midi_event.data[2]);
                                                                riff.events_mut().push(
                                                                    TrackEvent::PitchBend(
                                                                        PitchBend::new_from_midi_bytes(
                                                                            tempo / 60.0 * jack_midi_event.delta_frames as f64 / sample_rate, jack_midi_event.data[1], jack_midi_event.data[2])));
                                                                riff.events_mut().sort_by(|a, b| a.position().partial_cmp(&b.position()).unwrap());
                                                            }

                                                            break;
                                                        }
                                                    }
                                                },
                                                None => debug!("Jack midi receiver - no selected riff."),
                                            }
                                        }
                                        CurrentView::RiffSequence => {
                                            // not doing anything for sequences at this point in time
                                        }
                                        CurrentView::RiffArrangement => {
                                            if let Some(selected_riff_arrangement_uuid) = selected_riff_arrangement_uuid {
                                                if let Some(riff_arrangement) = project.song_mut().riff_arrangements_mut().iter_mut().find(|riff_arrangement| riff_arrangement.uuid() == selected_riff_arrangement_uuid.to_string()) {
                                                    let track_automation = if let Some(track_automation) = riff_arrangement.automation_mut(&track_uuid) {
                                                        track_automation
                                                    }
                                                    else {
                                                        riff_arrangement.add_track_automation(track_uuid.clone());
                                                        riff_arrangement.automation_mut(&track_uuid).unwrap()
                                                    };

                                                    if (176..=191).contains(&midi_msg_type) { // Controller - including modulation wheel
                                                        debug!("Adding controller to riff arrangement: delta frames={}, controller={}, value={}", jack_midi_event.delta_frames, jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32);
                                                        track_automation.events_mut().push(
                                                            TrackEvent::Controller(
                                                                Controller::new(
                                                                    tempo / 60.0 * jack_midi_event.delta_frames as f64 / sample_rate, jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32)));
                                                        track_automation.events_mut().sort_by(|a, b| a.position().partial_cmp(&b.position()).unwrap());
                                                    } else if (224..=239).contains(&midi_msg_type) {
                                                        debug!("Adding pitch bend to riff arrangement: delta frames={}, lsb={}, msb={}", jack_midi_event.delta_frames, jack_midi_event.data[1], jack_midi_event.data[2]);
                                                        track_automation.events_mut().push(
                                                            TrackEvent::PitchBend(
                                                                PitchBend::new_from_midi_bytes(
                                                                    tempo / 60.0 * jack_midi_event.delta_frames as f64 / sample_rate, jack_midi_event.data[1], jack_midi_event.data[2])));
                                                        track_automation.events_mut().sort_by(|a, b| a.position().partial_cmp(&b.position()).unwrap());
                                                    }
                                                }
                                            }
                                        }
                                        CurrentView::RiffGrid => {
                                            // not doing anything for riff grids at this point in time
                                        }
                                    }
                                },
                                TrackType::AudioTrack(_) => (),
                                TrackType::MidiTrack(_) => (),
                            },
                            None => (),
                        }

                        if play_mode == PlayMode::RiffSet && riff_changed {
                            if let Some(playing_riff_set) = playing_riff_set {
                                debug!("RiffSet riff updated - now calling state.play_riff_set_update_track");
                                state.play_riff_set_update_track_as_riff(playing_riff_set, track_uuid);
                            }
                        }
                    },
                    None => debug!("Record: no track number given."),
                }
            }
        },
        Err(_) => debug!("Main - jack_event_prcessing_thread processing loop - Record - could not get lock on state"),
    }
}


pub fn midi_AudioLayerOutwardEvent_PlayPositionInFrames(state: &mut RiffDAWState, play_position_in_frames: u32) {
    match state.get_project().lock() {
        Ok(project) => {
            let bpm = project.song().tempo();
            let time_signature_numerator = project.song().time_signature_numerator();
            let sample_rate = state.configuration.audio.sample_rate as f64;
            let play_position_in_beats = play_position_in_frames as f64 / sample_rate * bpm / 60.0;

            let current_bar = play_position_in_beats as i32 / time_signature_numerator as i32 + 1;
            let current_beat_in_bar = play_position_in_beats as i32 % time_signature_numerator as i32 + 1;
            // gui.ui.song_position_txt_ctrl.set_label(format!("{:03}:{:03}:000", current_bar, current_beat_in_bar).as_str());

            let time_in_secs = play_position_in_frames as f64 / sample_rate;
            let minutes = time_in_secs as i32 / 60;
            let seconds = time_in_secs as i32 % 60;
            let milli_seconds = ((time_in_secs - (time_in_secs as u64) as f64) * 1000.0) as u64;
            // gui.ui.song_time_txt_ctrl.set_label(format!("{:03}:{:02}:{:03}", minutes, seconds, milli_seconds).as_str());

            // debug!("Play position in frames: {}", play_position_in_frames);
            state.set_play_position_in_frames(play_position_in_frames);
        },
        Err(_) => debug!("Main - rx_ui processing loop - play position - could not get lock on state"),
    }
}

pub fn midi_AudioLayerOutwardEvent_GeneralMMCEvent(state: &mut RiffDAWState, mmc_sysex_bytes: [u8; 6]) {
    debug!("Midi generic MMC event: ");
    let command_byte = mmc_sysex_bytes[4];
    match command_byte {
        1 => { transport_stop_action(state); }
        2 => { transport_play_action(state); }
        4 => { transport_move_forward_action(state); }
        5 => { transport_move_back_action(state); }
        6 => { state.set_recording(!state.recording()); }
        _ => {}
    }
}

pub fn midi_AudioLayerOutwardEvent_MidiControlEvent(state: &mut RiffDAWState, jack_midi_event: MidiEvent) {
    match state.project().lock().as_mut() {
        Ok(project) => {
            if jack_midi_event.data[0] as i32 == 144 && jack_midi_event.data[1] as usize >= 36_usize {
                let riff_thing_index = jack_midi_event.data[1] as usize - 36_usize;
                // let track_riffs_stack_visible_name = gui.get_track_riffs_stack_visible_name();
                // if track_riffs_stack_visible_name == "Track Grid" {
                //     state.play_song();
                // } else if track_riffs_stack_visible_name == "Riffs" {
                //     let riffs_stack_visible_name = gui.get_riffs_stack_visible_name();
                //     if riffs_stack_visible_name == "riff_sets" {
                //         let riff_set_uuid = if let Some(riff_set) = project.song_mut().riff_sets_mut().get_mut(riff_thing_index) {
                //             riff_set.uuid()
                //         } else {
                //             "".to_string()
                //         };
                //         state.play_riff_set(riff_set_uuid);
                //     } else if riffs_stack_visible_name == "riff_sequences" {
                //         let riff_sequence_uuid = if let Some(riff_sequence) = project.song_mut().riff_sequences_mut().get_mut(riff_thing_index) {
                //             riff_sequence.uuid()
                //         } else {
                //             "".to_string()
                //         };
                //         state.play_riff_sequence(riff_sequence_uuid);
                //     } else if riffs_stack_visible_name == "riff_arrangement" {
                //         let riff_arrangement_uuid = if let Some(riff_arrangement) = project.song_mut().riff_arrangements_mut().get_mut(riff_thing_index) {
                //             riff_arrangement.uuid()
                //         } else {
                //             "".to_string()
                //         };
                //         state.play_riff_arrangement(riff_arrangement_uuid, 0.0);
                //     }
                // }
            } else {
                debug!("Main - rx_ui processing loop - jack AudioLayerOutwardEvent::MidiControlEvent - received a unknown message: {} {} {}", jack_midi_event.data[0], jack_midi_event.data[1], jack_midi_event.data[2]);
            }
        }
        Err(_) => {}
    }
}