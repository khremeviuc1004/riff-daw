use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::ops::DerefMut;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use apres::MIDI;
use dawproject::{zip, DawprojectReader};
use dawproject::project::{AuPluginType, ChannelDevicesElementTypeContent, ClipTypeContent, DeviceRoleType, LanesType, LanesTypeContent, ProjectStructureElementTypeContent};
use jack::MidiOut;
use log::debug;
use regex::Regex;
use uuid::Uuid;
use crate::constants::{CLAP, VST24, VST3};
use crate::domain::{AudioLayerInwardEvent, DAWItemLength, InstrumentTrack, Note, Project, Riff, RiffReference, Track, TrackBackgroundProcessorInwardEvent, AudioMode, TrackEvent, TrackType, AudioPlugin, AudioEffectTrack};
use crate::event::{AudioLayerEvent, DAWEvents, NotificationType};
use crate::state::{MidiPolyphonicExpressionNoteId, RiffDAWState};

pub fn daw_events_NewFile (state: &mut RiffDAWState) {
    // gui.clear_ui();
    // history.clear();
    state.close_all_tracks();
    state.reset_state();
    match state.get_project().lock().as_mut() {
        Ok(project) => {

            let mut new_project = Project::new();

            new_project.song_mut().set_tempo(project.song_mut().tempo());

            *(project.deref_mut()) = new_project;
            let mut instrument_track_senders2 = HashMap::new();
            let mut instrument_track_receivers2 = HashMap::new();
            let mut sample_references = HashMap::new();
            let mut samples_data = HashMap::new();
            let sample_rate = state.configuration.audio.sample_rate as f64;
            let block_size = state.configuration.audio.block_size as f64;
            let tempo = project.song().tempo();
            let time_signature_numerator = project.song().time_signature_numerator();
            let time_signature_denominator = project.song().time_signature_denominator();
            for track in project.song_mut().tracks_mut().iter_mut() {
                state.init_track(
                    track,
                    Some(&sample_references),
                    Some(&samples_data),
                    sample_rate,
                    block_size,
                    tempo,
                    time_signature_numerator as i32,
                    time_signature_denominator as i32,
                );
            }
            state.update_track_senders_and_receivers(instrument_track_senders2, instrument_track_receivers2);

            // gui.update_ui_from_state(tx_from_ui, &mut state, state_arc);
            if let Some(audio_layer_sender) = state.audio_layer_sender.as_ref() {
                match audio_layer_sender.send(AudioLayerEvent::AudioLayerInward(AudioLayerInwardEvent::BlockSize(state.configuration.audio.block_size as f64))) {
                    Ok(_) => (),
                    Err(error) => debug!("Problem using tx_to_audio to send block size message to jack layer: {}", error),
                }
                match audio_layer_sender.send(AudioLayerEvent::AudioLayerInward(AudioLayerInwardEvent::Tempo(project.song().tempo()))) {
                    Ok(_) => (),
                    Err(error) => debug!("Problem using tx_to_audio to send tempo message to jack layer: {}", error),
                }
                match audio_layer_sender.send(AudioLayerEvent::AudioLayerInward(AudioLayerInwardEvent::SampleRate(state.configuration.audio.sample_rate as f64))) {
                    Ok(_) => (),
                    Err(error) => debug!("Problem using tx_to_audio to send sample rate message to jack layer: {}", error),
                }
            }
        },
        Err(_) => debug!("Main - rx_ui processing loop - New File - could not get lock on state"),
    }
}
pub fn daw_events_OpenFile(state: &mut RiffDAWState, path: String) {
    // gui.clear_ui();
    // gui.ui.dialogue_progress_bar.set_text(Some(format!("Opening {}...", path.as_str().unwrap()).as_str()));
    // gui.ui.progress_dialogue.set_title("Open");
    // gui.ui.progress_dialogue.show_all();

    // THREAD_POOL.with_borrow(|thread_pool| thread_pool.spawn(move || {
        if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
            let _ = audio_layer_sender.send(AudioLayerEvent::AudioMode(AudioMode::Coast));
        }
        thread::sleep(Duration::from_millis(1000));
        // history.clear();
        let mut midi_tracks = HashMap::new();

        // get and release locks on state regularly (parking lot promises round robin locking to prevent starvation) otherwise it locks up the UI and causes jack under runs
            // let _ = tx_from_ui.send(DAWEvents::UpdateProgressBarMessage("Closing all tracks...".to_string()));
            state.close_all_tracks();
            // let _ = tx_from_ui.send(DAWEvents::UpdateProgressBarMessage("Resetting state...".to_string()));
            state.reset_state();
            // let _ = tx_from_ui.send(DAWEvents::UpdateProgressBarMessage("Loading file...".to_string()));
            state.load_from_file(path.as_str());
        if let Ok(project) = state.get_project().lock().as_mut() {
            // let _ = tx_from_ui.send(DAWEvents::UpdateProgressBarMessage("Setting up VST24 time info...".to_string()));
            let tempo = project.song().tempo();

            // {
            //     let mut time_info = vst_host_time_info.write();
            //     time_info.sample_pos = 0.0;
            //     time_info.sample_rate = state.configuration.audio.sample_rate as f64;; // FIXME is sample rate and block size part of a song or should it be part of configuration???
            //     time_info.nanoseconds = 0.0;
            //     time_info.ppq_pos = 0.0;
            //     time_info.tempo = tempo;
            //     time_info.bar_start_pos = 0.0;
            //     time_info.cycle_start_pos = 0.0;
            //     time_info.cycle_end_pos = 0.0;
            //     time_info.time_sig_numerator = project.song().time_signature_numerator() as i32;
            //     time_info.time_sig_denominator = project.song().time_signature_denominator() as i32;
            //     time_info.smpte_offset = 0;
            //     time_info.smpte_frame_rate = vst::api::SmpteFrameRate::Smpte24fps;
            //     time_info.samples_to_next_clock = 0;
            //     time_info.flags = 3;
            // }
        }
        match state.get_project().lock().as_mut() {
            Ok(project) => {
                // let _ = tx_from_ui.send(DAWEvents::UpdateProgressBarMessage("Sending tempo to track background processor...".to_string()));
                let tempo = project.song().tempo();
                for track in project.song().tracks() {
                    match track {
                        TrackType::MidiTrack(track) => {
                            midi_tracks.insert(track.uuid().to_string(), track.name().to_string());
                        }
                        _ => {
                            state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Tempo(tempo));
                        }
                    }
                }
            },
            Err(_) => debug!("Main - rx_ui processing loop - Open File - could not get lock on state"),
        }
        // let _ = tx_from_ui.send(DAWEvents::UpdateProgressBarMessage("Sending block size to the audio layer...".to_string()));
        if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
            match audio_layer_sender.send(AudioLayerEvent::AudioLayerInward(AudioLayerInwardEvent::BlockSize(state.configuration.audio.block_size as f64))) {
                Ok(_) => (),
                Err(error) => debug!("Problem using tx_to_audio to send block size message to jack layer: {}", error),
            }
        }
        if let Ok(project) = state.project.lock().as_ref() {
            // let _ = tx_from_ui.send(DAWEvents::UpdateProgressBarMessage("Sending tempo to the audio layer...".to_string()));
            if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
                match audio_layer_sender.send(AudioLayerEvent::AudioLayerInward(AudioLayerInwardEvent::Tempo(project.song().tempo()))) {
                    Ok(_) => (),
                    Err(error) => debug!("Problem using tx_to_audio to send block size message to jack layer: {}", error),
                }
            }
        }
        // let _ = tx_from_ui.send(DAWEvents::UpdateProgressBarMessage("Sending sample rate to the audio layer...".to_string()));
        if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
            match audio_layer_sender.send(AudioLayerEvent::AudioLayerInward(AudioLayerInwardEvent::SampleRate(state.configuration.audio.sample_rate as f64))) {
                Ok(_) => (),
                Err(error) => debug!("Problem using tx_to_audio to send block size message to jack layer: {}", error),
            }
        }

        match state.get_project().lock().as_mut() {
            Ok(project) => {
                // let _ = tx_from_ui.send(DAWEvents::UpdateProgressBarMessage("Creating track midi ports...".to_string()));
                // add midi track ports
                for (track_uuid, _) in midi_tracks {
                    if let Some(jack_client) = state.jack_client() {
                        if let Ok(midi_out_port) = jack_client.register_port(track_uuid.as_str(), MidiOut::default()) {
                            if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
                                if let Err(error) = audio_layer_sender.send(AudioLayerEvent::AudioLayerInward(AudioLayerInwardEvent::NewMidiOutPortForTrack(track_uuid.clone(), midi_out_port))) {
                                    debug!("Problem using tx_to_audio to send new midi out port message to jack layer: {}", error);
                                }
                            }
                        }
                    }
                }
            }
            Err(_) => {}
        }

        if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
            let _ = audio_layer_sender.send(AudioLayerEvent::AudioMode(AudioMode::AudioOut));
        }
    // }));
}

pub fn daw_events_Save (state: &mut RiffDAWState) {
    // gui.ui.dialogue_progress_bar.set_text(Some("Saving..."));
    // gui.ui.progress_dialogue.set_title("Save");
    // gui.ui.progress_dialogue.show_all();

    {
        // let _ = THREAD_POOL.with_borrow(|thread_pool| thread_pool.spawn(move || {
            if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
                let _ = audio_layer_sender.send(AudioLayerEvent::AudioMode(AudioMode::Coast));
            }
            thread::sleep(Duration::from_millis(1000));
            match state.get_project().lock().as_mut() {
                Ok(project) => {
                    debug!("main - DAWEvents::Save - number of riff sequences={}", project.song().riff_sequences().len());
                },
                Err(_) => debug!("Main - rx_ui processing loop - Save File - could not get lock on state"),
            }
            state.save();
            if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
                let _ = audio_layer_sender.send(AudioLayerEvent::AudioMode(AudioMode::AudioOut));
            }

            // let _ = tx_from_ui.send(DAWEvents::HideProgressDialogue);
        // }));
    }
}

pub fn daw_events_SaveAs(state: &mut RiffDAWState, path: String) {
    // gui.ui.dialogue_progress_bar.set_text(Some(format!("Saving as {}...", path.as_str().unwrap()).as_str()));
    // gui.ui.progress_dialogue.set_title("Save As");
    // gui.ui.progress_dialogue.show_all();

    {
        // let _ = THREAD_POOL.with_borrow(|thread_pool| thread_pool.spawn(move || {
            if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
                let _ = audio_layer_sender.send(AudioLayerEvent::AudioMode(AudioMode::Coast));
            }

            state.save_as(path.as_str());

            if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
                let _ = audio_layer_sender.send(AudioLayerEvent::AudioMode(AudioMode::AudioOut));
            }
            state.set_current_file_path(Some(path));

            // let _ = tx_from_ui.send(DAWEvents::HideProgressDialogue);
        // }));
    }
}

pub fn daw_events_ImportDAWProjectFile(state: &mut RiffDAWState, path: String) {
    if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
        let _ = audio_layer_sender.send(AudioLayerEvent::AudioMode(AudioMode::Coast));
    }
    thread::sleep(Duration::from_millis(1000));
    // history.clear();
    let mut midi_tracks = HashMap::new();
    state.close_all_tracks();
    state.reset_state();

    let mut new_project = Project::new();

    if let Ok(mut reader) = DawprojectReader::open(path) {
        let mut track_map = HashMap::new();
        let mut plugin_presets = HashMap::new();
        let file_names = reader.file_names().map(|file_name| file_name.to_string()).collect::<HashSet<String>>();

        for file_name in file_names {
            if file_name.starts_with("plugins/") {
                if let Ok(mut file) = reader.by_name(file_name.as_str()) {
                    let mut file_data = vec![];
                    if let Ok(read_bytes) = file.read_to_end(file_data.as_mut()) {
                        let mut plugin_preset = base64::encode(file_data);
                        println!("{}", file_name.as_str());
                        println!("{}", plugin_preset.as_str());
                        plugin_presets.insert(file_name.clone(), plugin_preset);
                    }
                    else {
                        println!("{} - can not read data from file", file_name.as_str());
                    }
                }
                else {
                    println!("{} - can not extract file from zip", file_name.as_str());
                }
            }
        }
        if let Ok(_) =reader.read_dawproject() {
            if let Some(daw_project) = reader.build_dawproject() {
                let application = &daw_project.project().application;

                new_project.song_mut().set_name(application.name.clone());

                if let Some(transport) = daw_project.project().transport.as_ref() {
                    if let Some(time_signature) = transport.time_signature.as_ref() {
                        let numerator = time_signature.numerator;
                        let denominator = time_signature.denominator;
                        new_project.song_mut().set_time_signature_numerator(numerator as f64);
                        new_project.song_mut().set_time_signature_denominator(denominator as f64);
                    }

                    if let Some(tempo) = transport.tempo.as_ref() {
                        if let Some(tempo) = tempo.value.as_ref() {
                            let song_tempo = tempo.clone();
                            if let Ok(tempo) = song_tempo.parse::<f64>() {
                                new_project.song_mut().set_tempo(tempo);
                            }
                            else {
                            new_project.song_mut().set_tempo(140.0);
                            }
                        }
                    }
                }

                if let Some(structure) = daw_project.project().structure.as_ref() {
                    for content in structure.content.iter() {
                        match content {
                            ProjectStructureElementTypeContent::Track(track_type) => {
                                let mut new_track = InstrumentTrack::new();

                                if let Some(track_id) = track_type.id.as_ref() {
                                track_map.insert(track_id.clone(), new_track.uuid());
                                }

                                if let Some(track_name) = track_type.name.as_ref() {
                                    new_track.name = track_name.clone();
                                }
                                if let Some(colour) = track_type.color.as_ref() {

                                }
                                if let Some(content_types) = track_type.content_types.as_ref() {

                                }
                                if let Some(channel) = track_type.channel.as_ref() {
                                    if let Some(channel_name) = channel.name.as_ref() {

                                    }
                                    if let Some(mute) = channel.mute.as_ref() {
                                        if let Some(mute) = mute.value.as_ref() {
                                            new_track.mute = *mute;
                                        }
                                    }
                                    if let Some(pan) = channel.pan.as_ref() {
                                        if let Some(pan) = pan.value.as_ref() {
                                            if let Ok(pan) = pan.parse::<f64>() {
                                                new_track.pan = pan as f32;
                                            }
                                            else {
                                                new_track.pan = 0.0;
                                            }
                                        }
                                    }
                                    if let Some(solo) = channel.solo.as_ref() {
                                        new_track.solo = *solo;
                                    }
                                    if let Some(devices) = channel.devices.as_ref() {
                                        for device in devices.content.iter() {
                                            match device {
                                                ChannelDevicesElementTypeContent::Device(_) => {

                                                }
                                                ChannelDevicesElementTypeContent::Vst2Plugin(plugin) => {
                                                    if plugin.device_role == DeviceRoleType::Instrument {
                                                        process_instrument_audio_plugin(state, &mut plugin_presets, &mut new_track, plugin, VST24.as_ref());
                                                    }
                                                    else {
                                                        process_effect_audio_plugin(state, &mut plugin_presets, &mut new_track, plugin, VST24.as_ref());
                                                    }
                                                }
                                                ChannelDevicesElementTypeContent::Vst3Plugin(plugin) => {
                                                    if plugin.device_role == DeviceRoleType::Instrument {
                                                        process_instrument_audio_plugin(state, &mut plugin_presets, &mut new_track, plugin, VST3.as_ref());
                                                    }
                                                    else {
                                                        process_effect_audio_plugin(state, &mut plugin_presets, &mut new_track, plugin, VST3.as_ref());
                                                    }
                                                }
                                                ChannelDevicesElementTypeContent::ClapPlugin(plugin) => {
                                                    if plugin.device_role == DeviceRoleType::Instrument {
                                                        process_instrument_audio_plugin(state, &mut plugin_presets, &mut new_track, plugin, CLAP.as_ref());
                                                    }
                                                    else {
                                                        process_effect_audio_plugin(state, &mut plugin_presets, &mut new_track, plugin, CLAP.as_ref());
                                                    }
                                                }
                                                ChannelDevicesElementTypeContent::BuiltinDevice(_) => {

                                                }
                                                ChannelDevicesElementTypeContent::Equalizer(_) => {

                                                }
                                                ChannelDevicesElementTypeContent::Compressor(_) => {

                                                }
                                                ChannelDevicesElementTypeContent::NoiseGate(_) => {

                                                }
                                                ChannelDevicesElementTypeContent::Limiter(_) => {

                                                }
                                                ChannelDevicesElementTypeContent::AuPlugin(_) => {

                                                }
                                            }
                                        }
                                    }
                                }

                                new_project.song_mut().add_track(TrackType::InstrumentTrack(new_track));
                            }
                            ProjectStructureElementTypeContent::Channel(channel_type) => {

                            }
                        }
                    }

                    // {
                    //     let sample_references = HashMap::new();
                    //     let samples_data = HashMap::new();
                    //     let sample_rate = state.configuration.audio.sample_rate as f64;
                    //     let block_size = state.configuration.audio.block_size as f64;
                    //     let tempo = new_project.song().tempo();
                    //     let time_signature_numerator = new_project.song().time_signature_numerator();
                    //     let time_signature_denominator = new_project.song().time_signature_denominator();
                    //     for track in new_project.song_mut().tracks_mut().iter_mut() {
                    //         state.init_track(
                    //             track,
                    //             Some(&sample_references),
                    //             Some(&samples_data),
                    //             // vst_host_time_info.clone(),
                    //             sample_rate,
                    //             block_size,
                    //             tempo,
                    //             time_signature_numerator as i32,
                    //             time_signature_denominator as i32,
                    //         );
                    //     }
                    // }
                }

                // now rummage for the track clips
                if let Some(arrangement) = daw_project.project().arrangement.as_ref() {
                    for lanes in arrangement.lanes.iter() {
                        for lane_content in lanes.content.iter() {
                            match lane_content {
                                LanesTypeContent::Timeline(_) => {}
                                LanesTypeContent::Lanes(_) => {}
                                LanesTypeContent::Notes(_) => {}
                                LanesTypeContent::Clips(_) => {}
                                LanesTypeContent::ClipSlot(_) => {}
                                LanesTypeContent::Markers(_) => {}
                                LanesTypeContent::Warps(_) => {}
                                LanesTypeContent::Audio(_) => {}
                                LanesTypeContent::Video(_) => {}
                                LanesTypeContent::Points(_) => {}
                            }
                            if let LanesTypeContent::Lanes(lane) = lane_content {
                                if let Some(lane_track_id) = lane.track.as_ref() {
                                    if let Some(riff_daw_track_id) = track_map.get(lane_track_id) {
                                        if let Some(riff_daw_track) = new_project.song_mut().tracks_mut().iter_mut().find(|track| track.uuid().as_str() == riff_daw_track_id.as_str()) {
                                            let mut track_riff = Riff::new_with_name_and_length(Uuid::new_v4(), String::from("1"), 6.0 * 140.0);
                                            let track_riff_ref = RiffReference::new(track_riff.uuid().to_string(), 0.0);
                                            let mut running_position = 0.0;

                                            riff_daw_track.riff_refs_mut().push(track_riff_ref);

                                            // process the clips
                                            for lane_content in lane.content.iter() {
                                                match lane_content {
                                                    LanesTypeContent::Timeline(_) => {

                                                    }
                                                    LanesTypeContent::Lanes(lane) => {
                                                        handle_lanes(state, &mut track_riff, lane);
                                                    }
                                                    LanesTypeContent::Notes(_) => {

                                                    }
                                                    LanesTypeContent::Clips(clips) => {
                                                        for clip in clips.clip.iter() {
                                                            // FIXME use the clip time for the riff_ref and the duration for the riff
                                                            running_position = clip.time;

                                                            if let Some(clip_content) = clip.content.as_ref() {
                                                                if let ClipTypeContent::Lanes(lanes) = clip_content {
                                                                    for lane_content in lanes.content.iter() {
                                                                        // FIXME will need to match the notes to previously created riffs or create a new riff
                                                                        if let LanesTypeContent::Notes(notes) = lane_content {
                                                                            for note in notes.note.iter() {
                                                                                let position = note.time.parse::<f64>().unwrap();
                                                                                let velocity = if let Some(velocity) = note.vel.as_ref() {
                                                                                    if let Ok(velocity) = velocity.parse::<f64>() {
                                                                                        velocity
                                                                                    }
                                                                                    else {
                                                                                        0.0
                                                                                    }
                                                                                }
                                                                                else {
                                                                                    0.0
                                                                                };
                                                                                let duration = note.duration.parse::<f64>().unwrap();
                                                                                track_riff.events_mut().push(
                                                                                    TrackEvent::Note(Note::new_with_params(
                                                                                        -1,
                                                                                        running_position + position,
                                                                                        note.key,
                                                                                        (velocity * 127.0) as i32,
                                                                                        duration))
                                                                                    );
                                                                            }
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                    LanesTypeContent::ClipSlot(_) => {

                                                    }
                                                    LanesTypeContent::Markers(_) => {

                                                    }
                                                    LanesTypeContent::Warps(_) => {

                                                    }
                                                    LanesTypeContent::Audio(_) => {

                                                    }
                                                    LanesTypeContent::Video(_) => {

                                                    }
                                                    LanesTypeContent::Points(_) => {

                                                    }
                                                }
                                            }

                                            riff_daw_track.riffs_mut().push(track_riff);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if let Some(scenes) = daw_project.project().scenes.as_ref() {

                }
            }
        }
    }

    state.set_project(new_project);

    if let Ok(project) = state.get_project().lock().as_mut() {
        // let _ = tx_from_ui.send(DAWEvents::UpdateProgressBarMessage("Setting up VST24 time info...".to_string()));
        let tempo = project.song().tempo();

        // {
        //     let mut time_info = vst_host_time_info.write();
        //     time_info.sample_pos = 0.0;
        //     time_info.sample_rate = state.configuration.audio.sample_rate as f64;; // FIXME is sample rate and block size part of a song or should it be part of configuration???
        //     time_info.nanoseconds = 0.0;
        //     time_info.ppq_pos = 0.0;
        //     time_info.tempo = tempo;
        //     time_info.bar_start_pos = 0.0;
        //     time_info.cycle_start_pos = 0.0;
        //     time_info.cycle_end_pos = 0.0;
        //     time_info.time_sig_numerator = project.song().time_signature_numerator() as i32;
        //     time_info.time_sig_denominator = project.song().time_signature_denominator() as i32;
        //     time_info.smpte_offset = 0;
        //     time_info.smpte_frame_rate = vst::api::SmpteFrameRate::Smpte24fps;
        //     time_info.samples_to_next_clock = 0;
        //     time_info.flags = 3;
        // }
    }
    match state.get_project().lock().as_mut() {
        Ok(project) => {
            // let _ = tx_from_ui.send(DAWEvents::UpdateProgressBarMessage("Sending tempo to track background processor...".to_string()));
            let tempo = project.song().tempo();
            for track in project.song().tracks() {
                match track {
                    TrackType::MidiTrack(track) => {
                        midi_tracks.insert(track.uuid().to_string(), track.name().to_string());
                    }
                    _ => {
                        state.send_to_track_background_processor(track.uuid().to_string(), TrackBackgroundProcessorInwardEvent::Tempo(tempo));
                    }
                }
            }
        },
        Err(_) => debug!("Main - rx_ui processing loop - Open File - could not get lock on state"),
    }
    // let _ = tx_from_ui.send(DAWEvents::UpdateProgressBarMessage("Sending block size to the audio layer...".to_string()));
    if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
        match audio_layer_sender.send(AudioLayerEvent::AudioLayerInward(AudioLayerInwardEvent::BlockSize(state.configuration.audio.block_size as f64))) {
            Ok(_) => (),
            Err(error) => debug!("Problem using tx_to_audio to send block size message to jack layer: {}", error),
        }
    }
    if let Ok(project) = state.project.lock().as_ref() {
        // let _ = tx_from_ui.send(DAWEvents::UpdateProgressBarMessage("Sending tempo to the audio layer...".to_string()));
        if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
            match audio_layer_sender.send(AudioLayerEvent::AudioLayerInward(AudioLayerInwardEvent::Tempo(project.song().tempo()))) {
                Ok(_) => (),
                Err(error) => debug!("Problem using tx_to_audio to send block size message to jack layer: {}", error),
            }
        }
    }
    // let _ = tx_from_ui.send(DAWEvents::UpdateProgressBarMessage("Sending sample rate to the audio layer...".to_string()));
    if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
        match audio_layer_sender.send(AudioLayerEvent::AudioLayerInward(AudioLayerInwardEvent::SampleRate(state.configuration.audio.sample_rate as f64))) {
            Ok(_) => (),
            Err(error) => debug!("Problem using tx_to_audio to send block size message to jack layer: {}", error),
        }
    }

    match state.get_project().lock().as_mut() {
        Ok(project) => {
            // let _ = tx_from_ui.send(DAWEvents::UpdateProgressBarMessage("Creating track midi ports...".to_string()));
            // add midi track ports
            for (track_uuid, _) in midi_tracks {
                if let Some(jack_client) = state.jack_client() {
                    if let Ok(midi_out_port) = jack_client.register_port(track_uuid.as_str(), MidiOut::default()) {
                        if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
                            if let Err(error) = audio_layer_sender.send(AudioLayerEvent::AudioLayerInward(AudioLayerInwardEvent::NewMidiOutPortForTrack(track_uuid.clone(), midi_out_port))) {
                                debug!("Problem using tx_to_audio to send new midi out port message to jack layer: {}", error);
                            }
                        }
                    }
                }
            }
        }
        Err(_) => {}
    }

    if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
        let _ = audio_layer_sender.send(AudioLayerEvent::AudioMode(AudioMode::AudioOut));
    }
}

fn handle_lanes(
    state: &mut RiffDAWState,
    riff: &mut Riff,
    lane: &LanesType
) {
    for content in lane.content.iter() {
        match content {
            LanesTypeContent::Timeline(time_line) => {}
            LanesTypeContent::Lanes(lanes) => {}
            LanesTypeContent::Notes(notes) => {}
            LanesTypeContent::Clips(clips) => {}
            LanesTypeContent::ClipSlot(clip_slot) => {}
            LanesTypeContent::Markers(markers) => {}
            LanesTypeContent::Warps(warps) => {}
            LanesTypeContent::Audio(audio) => {}
            LanesTypeContent::Video(video) => {}
            LanesTypeContent::Points(points) => {}
        }
    }
}

fn process_instrument_audio_plugin(
    state: &mut RiffDAWState,
    plugin_presets: &mut HashMap<String, String>,
    new_track: &mut InstrumentTrack,
    plugin: &AuPluginType,
    plugin_type: &str,
) {
    populate_audio_plugin(
        state,
        plugin_presets,
        plugin,
        plugin_type,
        new_track.instrument_mut()
    )
}

fn process_effect_audio_plugin(
    state: &mut RiffDAWState,
    plugin_presets: &mut HashMap<String, String>,
    new_track: &mut InstrumentTrack,
    plugin: &AuPluginType,
    plugin_type: &str,
) {
    let mut effect_plugin = AudioPlugin::new();
    populate_audio_plugin(
        state,
        plugin_presets,
        plugin,
        plugin_type,
        &mut effect_plugin
    );

    new_track.effects_mut().push(effect_plugin);
}

fn populate_audio_plugin(state: &mut RiffDAWState, plugin_presets: &mut HashMap<String, String>, daw_project_plugin: &AuPluginType, plugin_type: &str, riff_daw_audio_plugin: &mut AudioPlugin) {
    riff_daw_audio_plugin.descriptive_name = daw_project_plugin.device_name.clone();
    if let Some(daw_project_plugin_name) = daw_project_plugin.name.as_ref() {
        // need to split the daw_project_plugin_name on the first (
        let split_daw_project_plugin_name = daw_project_plugin_name.split("(").collect::<Vec<&str>>();
        if let Some(split_plugin_name) = split_daw_project_plugin_name.first() {
            let processed_daw_project_plugin_name = format!("{} ({})", split_plugin_name.trim(), plugin_type);
            riff_daw_audio_plugin.name = processed_daw_project_plugin_name.clone();
            let mut scanned_plugins = if DeviceRoleType::Instrument == daw_project_plugin.device_role {
                state.configuration.scanned_instrument_plugins.successfully_scanned.iter()
            }
            else {
                state.configuration.scanned_effect_plugins.successfully_scanned.iter()
            };
            if let Some((riff_daw_plugin_details, _)) = scanned_plugins
                .find(|(_, riff_daw_plugin_name)| riff_daw_plugin_name.as_str() == processed_daw_project_plugin_name.as_str()) {
                let dynamic_library_path = riff_daw_plugin_details.split(':').collect::<Vec<&str>>();
                if let Some(dynamic_library_path) = dynamic_library_path.get(0) {
                    riff_daw_audio_plugin.file = dynamic_library_path.to_string();
                }
            }
        }
    }
    riff_daw_audio_plugin.format = String::from("Unknown");
    riff_daw_audio_plugin.category = String::from("Unknown");
    if let Some(vendor) = daw_project_plugin.device_vendor.as_ref() {
        riff_daw_audio_plugin.manufacturer = vendor.clone();
    }
    if let Some(version) = daw_project_plugin.plugin_version.as_ref() {
        riff_daw_audio_plugin.version = version.clone();
    }
    // if let Some(xxx) = plugin. {
    //     effect_plugin.uid = plugin.;
    // }
    riff_daw_audio_plugin.is_instrument = true;
    // if let Some(xxx) = plugin. {
    //     effect_plugin.file_time = plugin.;
    // }
    // if let Some(xxx) = plugin. {
    //     effect_plugin.info_update_time = plugin.;
    // }
    // if let Some(num_inputs) = plugin. {
    //     effect_plugin.num_inputs = plugin.;
    // }
    // if let Some(xxx) = plugin. {
    //     effect_plugin.num_outputs = plugin.;
    // }
    riff_daw_audio_plugin.plugin_type = VST24.to_string();
    // if let Some(xxx) = plugin. {
    //     effect_plugin.sub_plugin_id = plugin.;
    // }
    if let Some(state) = daw_project_plugin.state.as_ref() {
        if let Some(plugin_preset_data_base64) = plugin_presets.get(&state.path) {
            riff_daw_audio_plugin.preset_data = plugin_preset_data_base64.clone();
        }
    }
}

pub fn daw_events_ImportMidiFile(state: &mut RiffDAWState, path: String) {
    // gui.clear_ui();
    // gui.ui.dialogue_progress_bar.set_text(Some(format!("Importing midi file {}...", path.as_str().unwrap()).as_str()));
    // gui.ui.progress_dialogue.set_title("Import Midi File");
    // gui.ui.progress_dialogue.show_all();

    {
        // let _ = THREAD_POOL.with_borrow(|thread_pool| thread_pool.spawn(move || {
            if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
                let _ = audio_layer_sender.send(AudioLayerEvent::AudioMode(AudioMode::Coast));
            }
            match state.get_project().lock().as_mut() {
                Ok(project) => {
                    let sample_rate = state.configuration.audio.sample_rate as f64;;
                    let block_size = state.configuration.audio.block_size as f64;;
                    let time_signature_numerator = project.song().time_signature_numerator();
                    let time_signature_denominator = project.song().time_signature_denominator();
                    let tracks = project.song_mut().tracks_mut();

                        match MIDI::from_path(path.as_str()) {
                            Ok(midi) => {
                                let mut track_number = 1;
                                let mut tempo: u32 = 0;
                                let ppq = midi.get_ppqn();
                                let mut instrument_track_senders2 = HashMap::new();
                                let mut instrument_track_receivers2 = HashMap::new();

                                for track in midi.get_tracks().iter() {
                                    debug!("Track: {}", track_number);
                                    let mut freedom_daw_track = InstrumentTrack::new();
                                    let mut current_notes = HashMap::new();
                                    let riff = Riff::new_with_name_and_length(Uuid::new_v4(), "unknown".to_owned(), 4.0);
                                    let riff_ref = RiffReference::new(riff.uuid().to_string(), 0.0);

                                    freedom_daw_track.riffs_mut().push(riff);
                                    freedom_daw_track.riff_refs_mut().push(riff_ref);

                                    let riff = freedom_daw_track.riffs_mut().get_mut(1).unwrap();
                                    let mut track_name = "".to_owned();

                                    for (_, event_id) in track.iter() {
                                        let position = midi.get_event_position(*event_id);
                                        match midi.get_event(*event_id) {
                                            Some(event) => {
                                                debug!("Found event: {:?}", event);
                                                match event {
                                                    apres::MIDIEvent::SequenceNumber(_) => (),
                                                    apres::MIDIEvent::Text(_) => (),
                                                    apres::MIDIEvent::CopyRightNotice(_) => (),
                                                    apres::MIDIEvent::TrackName(name) => track_name.push_str(name.as_str().trim_matches(char::from(0))),
                                                    apres::MIDIEvent::InstrumentName(_) => (),
                                                    apres::MIDIEvent::Lyric(_) => (),
                                                    apres::MIDIEvent::Marker(_) => (),
                                                    apres::MIDIEvent::CuePoint(_) => (),
                                                    apres::MIDIEvent::ChannelPrefix(_) => (),
                                                    apres::MIDIEvent::SetTempo(tempo_value) => {
                                                        tempo = tempo_value;
                                                        debug!("Tempo: {}", tempo);
                                                    },
                                                    apres::MIDIEvent::SMPTEOffset(_, _, _, _, _) => (),
                                                    apres::MIDIEvent::TimeSignature(_, _, _, _) => (),
                                                    apres::MIDIEvent::KeySignature(_) => (),
                                                    apres::MIDIEvent::SequencerSpecific(_) => (),
                                                    apres::MIDIEvent::NoteOn(_, note, velocity) => {
                                                        if let Some((_, ticks)) = position {
                                                            let position_in_beats = *ticks as f64 / ppq as f64;
                                                            let new_note = Note::new_with_params(
                                                                MidiPolyphonicExpressionNoteId::ALL as i32, position_in_beats, note as i32, velocity as i32, 0.0);
                                                            current_notes.insert(note, new_note);
                                                        }
                                                    },
                                                    apres::MIDIEvent::NoteOff(_, note, _) => {
                                                        if let Some((_track, ticks)) = position {
                                                            let position_in_beats = *ticks as f64 / ppq as f64;
                                                            if let Some(current_note) = current_notes.get_mut(&note) {
                                                                current_note.set_length(position_in_beats - current_note.position);
                                                                riff.events_mut().push(TrackEvent::Note(current_note.clone()));
                                                                current_notes.retain(|current_note, _| *current_note != note);
                                                            }
                                                        }
                                                    },
                                                    apres::MIDIEvent::AfterTouch(_, _, _) => (),
                                                    apres::MIDIEvent::BankSelect(_, _) => (),
                                                    apres::MIDIEvent::BankSelectLSB(_, _) => (),
                                                    apres::MIDIEvent::ModulationWheel(_, _) => (),
                                                    apres::MIDIEvent::ModulationWheelLSB(_, _) => (),
                                                    apres::MIDIEvent::BreathController(_, _) => (),
                                                    apres::MIDIEvent::BreathControllerLSB(_, _) => (),
                                                    apres::MIDIEvent::FootPedal(_, _) => (),
                                                    apres::MIDIEvent::FootPedalLSB(_, _) => (),
                                                    apres::MIDIEvent::PortamentoTime(_, _) => (),
                                                    apres::MIDIEvent::PortamentoTimeLSB(_, _) => (),
                                                    apres::MIDIEvent::DataEntry(_, _) => (),
                                                    apres::MIDIEvent::DataEntryLSB(_, _) => (),
                                                    apres::MIDIEvent::Volume(_, _) => (),
                                                    apres::MIDIEvent::VolumeLSB(_, _) => (),
                                                    apres::MIDIEvent::Balance(_, _) => (),
                                                    apres::MIDIEvent::BalanceLSB(_, _) => (),
                                                    apres::MIDIEvent::Pan(_, _) => (),
                                                    apres::MIDIEvent::PanLSB(_, _) => (),
                                                    apres::MIDIEvent::Expression(_, _) => (),
                                                    apres::MIDIEvent::ExpressionLSB(_, _) => (),
                                                    apres::MIDIEvent::EffectControl1(_, _) => (),
                                                    apres::MIDIEvent::EffectControl1LSB(_, _) => (),
                                                    apres::MIDIEvent::EffectControl2(_, _) => (),
                                                    apres::MIDIEvent::EffectControl2LSB(_, _) => (),
                                                    apres::MIDIEvent::GeneralPurpose1(_, _) => (),
                                                    apres::MIDIEvent::GeneralPurpose1LSB(_, _) => (),
                                                    apres::MIDIEvent::GeneralPurpose2(_, _) => (),
                                                    apres::MIDIEvent::GeneralPurpose2LSB(_, _) => (),
                                                    apres::MIDIEvent::GeneralPurpose3(_, _) => (),
                                                    apres::MIDIEvent::GeneralPurpose3LSB(_, _) => (),
                                                    apres::MIDIEvent::GeneralPurpose4(_, _) => (),
                                                    apres::MIDIEvent::GeneralPurpose4LSB(_, _) => (),
                                                    apres::MIDIEvent::HoldPedal(_, _) => (),
                                                    apres::MIDIEvent::Portamento(_, _) => (),
                                                    apres::MIDIEvent::Sustenuto(_, _) => (),
                                                    apres::MIDIEvent::SoftPedal(_, _) => (),
                                                    apres::MIDIEvent::Legato(_, _) => (),
                                                    apres::MIDIEvent::Hold2Pedal(_, _) => (),
                                                    apres::MIDIEvent::SoundVariation(_, _) => (),
                                                    apres::MIDIEvent::SoundTimbre(_, _) => (),
                                                    apres::MIDIEvent::SoundReleaseTime(_, _) => (),
                                                    apres::MIDIEvent::SoundAttack(_, _) => (),
                                                    apres::MIDIEvent::SoundBrightness(_, _) => (),
                                                    apres::MIDIEvent::SoundControl1(_, _) => (),
                                                    apres::MIDIEvent::SoundControl2(_, _) => (),
                                                    apres::MIDIEvent::SoundControl3(_, _) => (),
                                                    apres::MIDIEvent::SoundControl4(_, _) => (),
                                                    apres::MIDIEvent::SoundControl5(_, _) => (),
                                                    apres::MIDIEvent::GeneralPurpose5(_, _) => (),
                                                    apres::MIDIEvent::GeneralPurpose6(_, _) => (),
                                                    apres::MIDIEvent::GeneralPurpose7(_, _) => (),
                                                    apres::MIDIEvent::GeneralPurpose8(_, _) => (),
                                                    apres::MIDIEvent::EffectsLevel(_, _) => (),
                                                    apres::MIDIEvent::TremuloLevel(_, _) => (),
                                                    apres::MIDIEvent::ChorusLevel(_, _) => (),
                                                    apres::MIDIEvent::CelesteLevel(_, _) => (),
                                                    apres::MIDIEvent::PhaserLevel(_, _) => (),
                                                    apres::MIDIEvent::DataIncrement(_) => (),
                                                    apres::MIDIEvent::DataDecrement(_) => (),
                                                    apres::MIDIEvent::RegisteredParameterNumber(_, _) => (),
                                                    apres::MIDIEvent::RegisteredParameterNumberLSB(_, _) => (),
                                                    apres::MIDIEvent::NonRegisteredParameterNumber(_, _) => (),
                                                    apres::MIDIEvent::NonRegisteredParameterNumberLSB(_, _) => (),
                                                    apres::MIDIEvent::AllControllersOff(_) => (),
                                                    apres::MIDIEvent::LocalControl(_, _) => (),
                                                    apres::MIDIEvent::AllNotesOff(_) => (),
                                                    apres::MIDIEvent::AllSoundOff(_) => (),
                                                    apres::MIDIEvent::OmniOff(_) => (),
                                                    apres::MIDIEvent::OmniOn(_) => (),
                                                    apres::MIDIEvent::MonophonicOperation(_, _) => (),
                                                    apres::MIDIEvent::PolyphonicOperation(_) => (),
                                                    apres::MIDIEvent::ControlChange(_, _, _) => (),
                                                    apres::MIDIEvent::ProgramChange(_, _) => (),
                                                    apres::MIDIEvent::ChannelPressure(_, _) => (),
                                                    apres::MIDIEvent::PitchWheelChange(_, _) => (),
                                                    apres::MIDIEvent::SystemExclusive(_) => (),
                                                    apres::MIDIEvent::MTCQuarterFrame(_, _) => (),
                                                    apres::MIDIEvent::SongPositionPointer(_) => (),
                                                    apres::MIDIEvent::SongSelect(_) => (),
                                                    apres::MIDIEvent::TimeCode(_, _, _, _, _) => (),
                                                    apres::MIDIEvent::EndOfTrack => {
                                                        if let Some((_, ticks)) = position {
                                                            let position_in_beats = *ticks as f64 / ppq as f64;
                                                            riff.set_length(position_in_beats);
                                                        }
                                                    },
                                                    apres::MIDIEvent::TuneRequest => (),
                                                    apres::MIDIEvent::MIDIClock => (),
                                                    apres::MIDIEvent::MIDIStart => (),
                                                    apres::MIDIEvent::MIDIContinue => (),
                                                    apres::MIDIEvent::MIDIStop => (),
                                                    apres::MIDIEvent::ActiveSense => (),
                                                    apres::MIDIEvent::Reset => (),
                                                }
                                            },
                                            None => debug!("Could not find event."),
                                        }
                                    }

                                    track_number += 1;
                                    freedom_daw_track.set_name(track_name);
                                    tracks.push(TrackType::InstrumentTrack(freedom_daw_track));

                                    if let Some(track_type) = tracks.last_mut() {
                                        state.init_track(
                                            track_type,
                                            None,
                                            None,
                                            sample_rate,
                                            block_size,
                                            tempo as f64,
                                            time_signature_numerator as i32,
                                            time_signature_denominator as i32,
                                        );
                                    }
                                }

                                state.update_track_senders_and_receivers(instrument_track_senders2, instrument_track_receivers2);
                            },
                            Err(error) => debug!("Couldn't read midi file: {:?}", error),
                        }
                },
                Err(_) => debug!("Main - rx_ui processing loop - Import Midi File - could not get lock on state"),
            }
            if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
                let _ = audio_layer_sender.send(AudioLayerEvent::AudioMode(AudioMode::AudioOut));
            }

            // let _ = tx_from_ui.send(DAWEvents::UpdateUI);
            // let _ = tx_from_ui.send(DAWEvents::HideProgressDialogue);
        // }));
    }
}

pub fn daw_events_ExportMidiFile(state: &mut RiffDAWState, path: String) {
    // gui.ui.dialogue_progress_bar.set_text(Some(format!("Exporting midi file as {}...", path.as_str().unwrap()).as_str()));
    // gui.ui.progress_dialogue.set_title("Export Midi File");
    // gui.ui.progress_dialogue.show_all();

    if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
        let _ = audio_layer_sender.send(AudioLayerEvent::AudioMode(AudioMode::Render));
    }
    {
        // let _ = THREAD_POOL.with_borrow(|thread_pool| thread_pool.spawn(move || {
            debug!("Main - rx_ui processing loop - Export Midi File - attempting to export.");
            let path = PathBuf::from(path);
            if !state.export_to_midi_file(path) {
                // let _ = tx_from_ui.send(DAWEvents::HideProgressDialogue);
                // let _ = tx_from_ui.send(DAWEvents::Notification(NotificationType::Error, "Could not export midi file.".to_string()));
            }
            if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
                let _ = audio_layer_sender.send(AudioLayerEvent::AudioMode(AudioMode::AudioOut));
            }
            // let _ = tx_from_ui.send(DAWEvents::HideProgressDialogue);
        // }));
    }
}

pub fn daw_events_ExportRiffsToMidiFile(state: &mut RiffDAWState, path: String) {
    // gui.ui.dialogue_progress_bar.set_text(Some(format!("Exporting riffs to midi file as {}...", path.as_str().unwrap()).as_str()));
    // gui.ui.progress_dialogue.set_title("Export riffs to midi file");
    // gui.ui.progress_dialogue.show_all();

    if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
        let _ = audio_layer_sender.send(AudioLayerEvent::AudioMode(AudioMode::Render));
    }
    {
        // let _ = THREAD_POOL.with_borrow(|thread_pool| thread_pool.spawn(move || {
            debug!("Main - rx_ui processing loop - Export riffs to midi file - attempting to export.");
            let path = PathBuf::from(path);
            if !state.export_riffs_to_midi_file(path) {
                // let _ = tx_from_ui.send(DAWEvents::HideProgressDialogue);
                // let _ = tx_from_ui.send(DAWEvents::Notification(NotificationType::Error, "Could not export riffs to midi file.".to_string()));
            }
            if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
                let _ = audio_layer_sender.send(AudioLayerEvent::AudioMode(AudioMode::AudioOut));
            }
            // let _ = tx_from_ui.send(DAWEvents::HideProgressDialogue);
        // }));
    }
}

pub fn daw_events_ExportRiffsToSeparateMidiFiles(state: &mut RiffDAWState, path: String) {
    // gui.ui.dialogue_progress_bar.set_text(Some(format!("Exporting riffs to separate midi files to directory {}...", path.as_str().unwrap()).as_str()));
    // gui.ui.progress_dialogue.set_title("Export riffs to separate midi files");
    // gui.ui.progress_dialogue.show_all();

    if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
        let _ = audio_layer_sender.send(AudioLayerEvent::AudioMode(AudioMode::Coast));
    }
    {
        // let _ = THREAD_POOL.with_borrow(|thread_pool| thread_pool.spawn(move || {
            debug!("Main - rx_ui processing loop - Export riffs to separate midi files - attempting to export.");
            let path = PathBuf::from(path);
            if !state.export_riffs_to_separate_midi_files(path) {
                // let _ = tx_from_ui.send(DAWEvents::HideProgressDialogue);
                // let _ = tx_from_ui.send(DAWEvents::Notification(NotificationType::Error, "Could not export riffs to separate midi files.".to_string()));
            }
            if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
                let _ = audio_layer_sender.send(AudioLayerEvent::AudioMode(AudioMode::AudioOut));
            }
            // let _ = tx_from_ui.send(DAWEvents::HideProgressDialogue);
        // }));
    }
}

pub fn daw_events_ExportWaveFile(state: &mut RiffDAWState, path: String) {
    // gui.ui.dialogue_progress_bar.set_text(Some(format!("Exporting wave file as {}...", path.as_str().unwrap()).as_str()));
    // gui.ui.progress_dialogue.set_title("Export Wav File");
    // gui.ui.progress_dialogue.show_all();

    if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
        let _ = audio_layer_sender.send(AudioLayerEvent::AudioMode(AudioMode::Render));
    }
    debug!("Main - rx_ui processing loop - Export Wave File - attempting to export.");
    let path = PathBuf::from(path);
    state.export_to_wave_file(path);
    if let Some(audio_layer_sender) = state.audio_layer_sender.as_mut() {
        let _ = audio_layer_sender.send(AudioLayerEvent::AudioMode(AudioMode::AudioOut));
    }
}
