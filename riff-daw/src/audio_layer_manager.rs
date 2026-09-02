use std::collections::HashMap;
use std::ops::DerefMut;
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;
use crossbeam_channel::unbounded;
use log::debug;
use parking_lot::{RwLock};
use simple_clap_host_helper_lib::plugin::library::PluginLibrary;
use uuid::Uuid;
use vst3_host::Vst3Host;
use vst::api::TimeInfo;
use vst::host::PluginLoader;
use xilem::tokio::sync::mpsc::UnboundedReceiver;
use xilem_core::MessageProxy;
use crate::audio::AudioLayer;
use crate::domain::{GeneralTrackType, InstrumentTrackBackgroundProcessor, TrackBackgroundProcessor, TrackBackgroundProcessorInwardEvent, AudioMode, TrackBackgroundProcessorOutwardEvent, VstHost, TrackType};
use crate::event::{AudioLayerEvent, AudioLayerInwardEvent, AudioLayerOutwardEvent, AudioLayerTimeCriticalOutwardEvent};
use crate::event::AudioLayerEvent::{AddTrackBackgroundProcessor, TrackBackgroundProcessorOutward};

pub struct AudioLayerManager {
    pub vst24_plugin_loaders: Arc<Mutex<HashMap<String, PluginLoader<VstHost>>>>,
    pub clap_plugin_loaders: Arc<Mutex<HashMap<String, PluginLibrary>>>,
    pub vst_host_time_info: Arc<RwLock<TimeInfo>>,
    pub sample_rate: i32,
    pub block_size: i32,
    pub tempo: f64,
    pub time_signature_numerator: i32,
    pub time_signature_denominator: i32,
    pub track_background_processors: HashMap<String, Box<dyn TrackBackgroundProcessor>>,
    pub track_senders: HashMap<String, Sender<TrackBackgroundProcessorInwardEvent>>,
    pub track_receivers: HashMap<String, Receiver<TrackBackgroundProcessorOutwardEvent>>,
    pub track_audio_coast: Arc<Mutex<AudioMode>>,
    pub tx_to_audio: crossbeam_channel::Sender<AudioLayerInwardEvent>,
    pub audio_layer: AudioLayer,
    pub jack_midi_receiver_ui: crossbeam_channel::Receiver<AudioLayerOutwardEvent>,
    pub jack_midi_receiver: crossbeam_channel::Receiver<AudioLayerOutwardEvent>,
    pub jack_time_critical_midi_receiver: crossbeam_channel::Receiver<AudioLayerTimeCriticalOutwardEvent>,
    pub selected_track: Option<String>,
    pub vst3_host: Arc<Mutex<Vst3Host>>,
}

impl AudioLayerManager {

    pub fn new() -> Self {
        let (tx_to_audio, rx_to_audio) = unbounded::<AudioLayerInwardEvent>();
        let (jack_midi_sender_ui, jack_midi_receiver_ui) = unbounded::<AudioLayerOutwardEvent>();
        let (jack_midi_sender, jack_midi_receiver) = unbounded::<AudioLayerOutwardEvent>();
        let (jack_time_critical_midi_sender, jack_time_critical_midi_receiver) = unbounded::<AudioLayerTimeCriticalOutwardEvent>();
        let track_audio_coast = Arc::new(Mutex::new(AudioMode::AudioOut));
        let jack_audio_coast = track_audio_coast.clone();
        let sample_rate = 44100;
        let block_size = 2048;
        let tempo = 140.0;
        let time_signature_numerator= 4;
        let time_signature_denominator = 4;


        let vst_host_time_info = Arc::new(RwLock::new(TimeInfo {
            sample_pos: 0.0,
            sample_rate: sample_rate as f64,
            nanoseconds: 0.0,
            ppq_pos: 0.0,
            tempo,
            bar_start_pos: 0.0,
            cycle_start_pos: 0.0,
            cycle_end_pos: 0.0,
            time_sig_numerator: time_signature_numerator,
            time_sig_denominator: time_signature_denominator,
            smpte_offset: 0,
            smpte_frame_rate: vst::api::SmpteFrameRate::Smpte24fps,
            samples_to_next_clock: 0,
            flags: 3,
        }));


        let mut audio_layer = AudioLayer::new();


        audio_layer.start_jack(
            rx_to_audio.clone(),
            jack_midi_sender.clone(),
            jack_midi_sender_ui.clone(),
            jack_time_critical_midi_sender.clone(),
            jack_audio_coast.clone(),
            vst_host_time_info.clone(),
            sample_rate,
            block_size,
            tempo);

        Self {
            vst24_plugin_loaders: Arc::new(Mutex::new(HashMap::new())),
            clap_plugin_loaders: Arc::new(Mutex::new(HashMap::new())),
            vst_host_time_info,
            sample_rate,
            block_size,
            tempo,
            time_signature_numerator,
            time_signature_denominator,
            track_background_processors: HashMap::new(),
            track_senders: HashMap::new(),
            track_receivers: HashMap::new(),
            track_audio_coast: Arc::new(Mutex::new( crate::domain::AudioMode::AudioOut)),
            tx_to_audio,
            audio_layer,
            jack_midi_receiver,
            jack_midi_receiver_ui,
            jack_time_critical_midi_receiver,
            selected_track: None,
            vst3_host: Arc::new(Mutex::new(Vst3Host::builder().sample_rate(sample_rate as f64).block_size(block_size as usize).tempo(tempo).time_signature(time_signature_numerator, time_signature_denominator).build().unwrap())),
        }
    }

    pub fn handle_events(&mut self, rx: &mut UnboundedReceiver<AudioLayerEvent>, proxy: &MessageProxy<AudioLayerEvent>) -> bool {
        for (track_uuid, track_receiver) in self.track_receivers.iter() {
            if let Ok(track_outward_event) = track_receiver.try_recv() {
                match track_outward_event {
                    TrackBackgroundProcessorOutwardEvent::InstrumentName(name) => {
                        let _ = proxy.message(AudioLayerEvent::TrackBackgroundProcessorOutward(track_uuid.clone(), TrackBackgroundProcessorOutwardEvent::InstrumentName(name)));
                    }
                    TrackBackgroundProcessorOutwardEvent::InstrumentParameters(instrument_parameters) => {
                        let _ = proxy.message(AudioLayerEvent::TrackBackgroundProcessorOutward(track_uuid.clone(), TrackBackgroundProcessorOutwardEvent::InstrumentParameters(instrument_parameters)));
                    }
                    TrackBackgroundProcessorOutwardEvent::EffectParameters(effect_params) => {
                        let _ = proxy.message(AudioLayerEvent::TrackBackgroundProcessorOutward(track_uuid.clone(), TrackBackgroundProcessorOutwardEvent::EffectParameters(effect_params)));
                    }
                    TrackBackgroundProcessorOutwardEvent::GetPresetData(instrument_audio_plugin_preset_data, effect_audio_plugins_preset_data) => {
                        let _ = proxy.message(AudioLayerEvent::TrackBackgroundProcessorOutward(track_uuid.clone(), TrackBackgroundProcessorOutwardEvent::GetPresetData(instrument_audio_plugin_preset_data, effect_audio_plugins_preset_data)));
                    }
                    TrackBackgroundProcessorOutwardEvent::InstrumentPluginWindowSize(_, _, _) => {}
                    TrackBackgroundProcessorOutwardEvent::EffectPluginWindowSize(_, _, _, _) => {}
                    TrackBackgroundProcessorOutwardEvent::Automation(_, _, _, _, _) => {}
                    TrackBackgroundProcessorOutwardEvent::TrackRenderAudioConsumer(_) => {}
                    TrackBackgroundProcessorOutwardEvent::ChannelLevels(_, _, _) => {}
                }
            }
        }

        if let Ok(audio_layer_event) = rx.try_recv() {
            println!("Received an audio layer inward message.");

            match audio_layer_event {
                AudioLayerEvent::AudioLayerInward(audio_layer_inward_event) => {
                    match audio_layer_inward_event {
                        AudioLayerInwardEvent::NewAudioConsumer(_) => {}
                        AudioLayerInwardEvent::NewMidiConsumer(_) => {}
                        AudioLayerInwardEvent::Play(play, number_of_blocks, start_block) => {
                            let _ = self.tx_to_audio.send(AudioLayerInwardEvent::Play(play, number_of_blocks, start_block));
                        }
                        AudioLayerInwardEvent::ExtentsChange(_) => {}
                        AudioLayerInwardEvent::Stop => {}
                        AudioLayerInwardEvent::Tempo(_) => {}
                        AudioLayerInwardEvent::SampleRate(_) => {}
                        AudioLayerInwardEvent::BlockSize(_) => {}
                        AudioLayerInwardEvent::Volume(_) => {}
                        AudioLayerInwardEvent::Pan(_) => {}
                        AudioLayerInwardEvent::Shutdown => {}
                        AudioLayerInwardEvent::RemoveTrack(track_uuid) => {
                            let _ = self.tx_to_audio.send(AudioLayerInwardEvent::RemoveTrack(track_uuid.clone()));
                            if let Some(track_sender) = self.track_senders.get(&track_uuid) {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::Kill);
                            }
                            self.track_background_processors.retain(|found_track_uuid, yyy| found_track_uuid.clone() != track_uuid);
                        }
                        AudioLayerInwardEvent::NewMidiOutPortForTrack(_, _) => {}
                        AudioLayerInwardEvent::PreviewSample(_) => {}
                        AudioLayerInwardEvent::TrackBackgroundProcessorSender(_, _) => {
                            // not used here
                        }
                        AudioLayerInwardEvent::SelectTrackBackgroundProcessor(_) => {
                            // not used here
                        }
                    }
                }
                AudioLayerEvent::AudioLayerOutward(_) => {}
                AudioLayerEvent::AudioLayerTimeCriticalOutwardEvent(_) => {}
                AudioLayerEvent::TrackBackgroundProcessorInward(event, track_uuid) => {
                    println!("^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ audio layer manager - TrackBackgroundProcessorInward event.");
                    if let Some(track_sender) = self.track_senders.get(&track_uuid) {
                        println!("^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ audio layer manager - TrackBackgroundProcessorInward track_sender found.");
                        match event {
                            TrackBackgroundProcessorInwardEvent::SetSample(sample_data) => {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::SetSample(sample_data));
                            }
                            TrackBackgroundProcessorInwardEvent::SetEvents(event_block_data, transition_to) => {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::SetEvents(event_block_data, transition_to));
                            }
                            TrackBackgroundProcessorInwardEvent::SetEventProcessorType(event_processor_type) => {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::SetEventProcessorType(event_processor_type));
                            }
                            TrackBackgroundProcessorInwardEvent::GotoStart => {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::GotoStart);
                            }
                            TrackBackgroundProcessorInwardEvent::MoveBack => {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::MoveBack);
                            }
                            TrackBackgroundProcessorInwardEvent::Play(start_block) => {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::Play(start_block));
                            }
                            TrackBackgroundProcessorInwardEvent::Stop => {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::Stop);
                            }
                            TrackBackgroundProcessorInwardEvent::Loop(start_looping) => {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::Loop(start_looping));
                            }
                            TrackBackgroundProcessorInwardEvent::LoopExtents(start_block, end_block) => {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::LoopExtents(start_block, end_block));
                            }
                            TrackBackgroundProcessorInwardEvent::Pause => {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::Pause);
                            }
                            TrackBackgroundProcessorInwardEvent::MoveForward => {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::MoveForward);
                            }
                            TrackBackgroundProcessorInwardEvent::GotoEnd => {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::GotoEnd);
                            }
                            TrackBackgroundProcessorInwardEvent::Mute => {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::Mute);
                            }
                            TrackBackgroundProcessorInwardEvent::Unmute => {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::Unmute);
                            }
                            TrackBackgroundProcessorInwardEvent::Kill => {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::Kill);
                                self.track_senders.remove(&track_uuid);
                            }
                            TrackBackgroundProcessorInwardEvent::AddEffect(vst24_plugin_loaders, clap_plugin_loaders, uuid, plugin_details) => {
                                println!("******************************** Effect {}", plugin_details.as_str());
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::AddEffect(self.vst24_plugin_loaders.clone(), self.clap_plugin_loaders.clone(), uuid, plugin_details));
                                std::thread::sleep(Duration::from_secs(1));
                            }
                            TrackBackgroundProcessorInwardEvent::DeleteEffect(effect_uuid) => {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::DeleteEffect(effect_uuid));
                            }
                            TrackBackgroundProcessorInwardEvent::ShowEffect(effect_uuid) => {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::ShowEffect(effect_uuid));
                            }
                            TrackBackgroundProcessorInwardEvent::ChangeInstrument(vst24_plugin_loaders, clap_plugin_loaders, uuid, plugin_details) => {
                                println!("******************************** Instrument {}", plugin_details.as_str());
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::ChangeInstrument(self.vst24_plugin_loaders.clone(), self.clap_plugin_loaders.clone(), uuid, plugin_details));
                                std::thread::sleep(Duration::from_secs(1));
                            }
                            TrackBackgroundProcessorInwardEvent::ShowInstrument => {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::ShowInstrument);
                            }
                            TrackBackgroundProcessorInwardEvent::SetInstrumentParameter(parameter_index, value) => {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::SetInstrumentParameter(parameter_index, value));
                            }
                            TrackBackgroundProcessorInwardEvent::SetPresetData(instrument_preset_data, effects_preset_data) => {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::SetPresetData(instrument_preset_data, effects_preset_data));
                            }
                            TrackBackgroundProcessorInwardEvent::RequestPresetData => {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::RequestPresetData);
                            }
                            TrackBackgroundProcessorInwardEvent::PlayNoteImmediate(note_number, midi_channel_number) => {
                                println!("^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ audio layer manager - play note immediate.");
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::PlayNoteImmediate(note_number, midi_channel_number));
                            }
                            TrackBackgroundProcessorInwardEvent::StopNoteImmediate(note_number, midi_channel_number) => {
                                println!("^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ audio layer manager - stop note immediate.");
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::StopNoteImmediate(note_number, midi_channel_number));
                            }
                            TrackBackgroundProcessorInwardEvent::PlayControllerImmediate(controller, value, midi_channel) => {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::PlayControllerImmediate(controller, value, midi_channel));
                            }
                            TrackBackgroundProcessorInwardEvent::PlayPitchBendImmediate(lsb, msb, midi_channel) => {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::PlayPitchBendImmediate(lsb, msb, midi_channel));
                            }
                            TrackBackgroundProcessorInwardEvent::RequestInstrumentParameters => {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::RequestInstrumentParameters);
                            }
                            TrackBackgroundProcessorInwardEvent::RequestEffectParameters(uuid) => {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::RequestEffectParameters(uuid));
                            }
                            TrackBackgroundProcessorInwardEvent::SetBlockPosition(block_position) => {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::SetBlockPosition(block_position));
                            }
                            TrackBackgroundProcessorInwardEvent::Volume(vol) => {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::Volume(vol));
                            }
                            TrackBackgroundProcessorInwardEvent::Pan(spatial_pan) => {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::Pan(spatial_pan));
                            }
                            TrackBackgroundProcessorInwardEvent::Tempo(bpm) => {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::Tempo(bpm));
                            }
                            TrackBackgroundProcessorInwardEvent::TimeSignatureChange(time_signature_numerator, time_signature_denominator) => {
                                let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::TimeSignatureChange(time_signature_numerator, time_signature_denominator));
                            }
                            TrackBackgroundProcessorInwardEvent::AddTrackEventSendRouting(_, _, _) => {}
                            TrackBackgroundProcessorInwardEvent::RemoveTrackEventSendRouting(_) => {}
                            TrackBackgroundProcessorInwardEvent::UpdateTrackEventSendRouting(_, _) => {}
                            TrackBackgroundProcessorInwardEvent::AddTrackEventReceiveRouting(_, _) => {}
                            TrackBackgroundProcessorInwardEvent::RemoveTrackEventReceiveRouting(_) => {}
                            TrackBackgroundProcessorInwardEvent::UpdateTrackEventReceiveRouting(_, _) => {}
                            TrackBackgroundProcessorInwardEvent::AddAudioSendRouting(_, _, _) => {}
                            TrackBackgroundProcessorInwardEvent::RemoveAudioSendRouting(_) => {}
                            TrackBackgroundProcessorInwardEvent::AddAudioReceiveRouting(_, _) => {}
                            TrackBackgroundProcessorInwardEvent::RemoveAudioReceiveRouting(_) => {}
                        }
                    }
                }
                AudioLayerEvent::TrackBackgroundProcessorOutward(track_uuid, event) => {
                    match event {
                        TrackBackgroundProcessorOutwardEvent::InstrumentName(name) => {
                            println!("******************************************* Received instrument name: {}", name.as_str());
                        }
                        TrackBackgroundProcessorOutwardEvent::GetPresetData(track_uuid, audio_plugin_preset_data) => {
                            println!("******************************************* Received track audio plugin preset data: {}", track_uuid.as_str());
                        }
                        _ => {}
                    }
                }
                AudioLayerEvent::AddTrackBackgroundProcessor(track_type, track_uuid) => {
                    println!("^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ audio layer manager - add track: {}", track_uuid.as_str());
                    match track_type {
                        GeneralTrackType::InstrumentTrack => {
                            let mut instrument_track: Box<dyn TrackBackgroundProcessor> = Box::new(InstrumentTrackBackgroundProcessor::new());
                            let (tx_track_background_thread_inward, rx_track_background_thread_inward) = std::sync::mpsc::channel::<TrackBackgroundProcessorInwardEvent>();
                            let (tx_track_background_thread_outward, rx_track_background_thread_outward) = std::sync::mpsc::channel::<crate::event::TrackBackgroundProcessorOutwardEvent>();

                            let tx_track_background_thread_inward2 = tx_track_background_thread_inward.clone();
                            let _ = self.tx_to_audio.send(AudioLayerInwardEvent::TrackBackgroundProcessorSender(track_uuid.clone(), tx_track_background_thread_inward2));

                            instrument_track.start_processing(
                                track_uuid.to_string(),
                                self.tx_to_audio.clone(),
                                rx_track_background_thread_inward,
                                tx_track_background_thread_outward,
                                self.track_audio_coast.clone(),
                                1.0,
                                0.5,
                                self.vst_host_time_info.clone(),
                                self.sample_rate as f64,
                                self.block_size as f64,
                                self.tempo,
                                self.time_signature_numerator,
                                self.time_signature_denominator,
                                self.vst3_host.clone(),
                            );

                            self.track_background_processors.insert(track_uuid.to_string(), instrument_track);
                            self.track_senders.insert(track_uuid.to_string(), tx_track_background_thread_inward);
                            self.track_receivers.insert(track_uuid, rx_track_background_thread_outward);
                        }
                        GeneralTrackType::AudioTrack => {}
                        GeneralTrackType::MidiTrack => {}
                        GeneralTrackType::MasterTrack => {}
                    }
                }
                AudioLayerEvent::DeleteTrackBackgroundProcessor(track_uuid) => {
                    println!("^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ audio layer manager - delete track: {}", track_uuid.as_str());
                    // delete the track background processor sender
                    if let Some(track_sender) = self.track_senders.remove(&track_uuid) {
                        // stop playing if playing
                        let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::Stop);
                        // stop all the audio plugins and let the background processor exit the processing loop
                        let _ = track_sender.send(TrackBackgroundProcessorInwardEvent::Kill);
                    }
                    // delete the background processor
                    self.track_background_processors.remove(&track_uuid.to_string());
                    // delete the track background processor receiver
                    self.track_receivers.remove(&track_uuid.to_string());
                    if let Err(error) = self.tx_to_audio.send(AudioLayerInwardEvent::RemoveTrack(track_uuid.clone())) {
                        debug!("Main - rx_ui processing loop - Track Deleted - could send delete track to audio layer: {}", error);
                    }
                }
                AudioLayerEvent::SelectTrackBackgroundProcessor(track_uuid) => {
                    println!("^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ audio layer manager - select track: {}", track_uuid.as_str());
                    self.selected_track = Some(track_uuid.clone());
                    let _ = self.tx_to_audio.send(AudioLayerInwardEvent::SelectTrackBackgroundProcessor(track_uuid));
                }
                AudioLayerEvent::Shutdown => {
                    println!("^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ audio layer manager - shutdown audio layer");
                    self.audio_layer.stop_jack();
                    return false;
                }
                AudioLayerEvent::AudioMode(audio_mode_value) => {
                    println!("^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ audio layer manager - audio mode");
                    if let Ok(audio_mode) = self.track_audio_coast.lock().as_mut() {
                        *(audio_mode.deref_mut()) = audio_mode_value;
                    }
                }
            }
        }

        return true;
    }

    pub fn handle_incoming_jack_events(&mut self, proxy: &MessageProxy<AudioLayerEvent>) {
        if let Ok(event) = self.jack_midi_receiver.try_recv() {
            match event {
                AudioLayerOutwardEvent::MidiControlEvent(event) => {
                    let _ = proxy.message(AudioLayerEvent::AudioLayerOutward(AudioLayerOutwardEvent::MidiControlEvent(event)));
                }
                AudioLayerOutwardEvent::GeneralMMCEvent(event) => {
                    let _ = proxy.message(AudioLayerEvent::AudioLayerOutward(AudioLayerOutwardEvent::GeneralMMCEvent(event)));
                }
                AudioLayerOutwardEvent::PlayPositionInFrames(position) => {
                    let _ = proxy.message(AudioLayerEvent::AudioLayerOutward(AudioLayerOutwardEvent::PlayPositionInFrames(position)));
                }
                AudioLayerOutwardEvent::JackRestartRequired => {
                    let _ = proxy.message(AudioLayerEvent::AudioLayerOutward(AudioLayerOutwardEvent::JackRestartRequired));
                }
                AudioLayerOutwardEvent::JackConnect(thing, thang) => {
                    let _ = proxy.message(AudioLayerEvent::AudioLayerOutward(AudioLayerOutwardEvent::JackConnect(thing, thang)));
                }
                AudioLayerOutwardEvent::MasterChannelLevels(volume, pan) => {
                    let _ = proxy.message(AudioLayerEvent::AudioLayerOutward(AudioLayerOutwardEvent::MasterChannelLevels(volume, pan)));
                }
            }
        }
        if let Ok(event) = self.jack_midi_receiver_ui.try_recv() {
            match event {
                AudioLayerOutwardEvent::MidiControlEvent(event) => {
                    let _ = proxy.message(AudioLayerEvent::AudioLayerOutward(AudioLayerOutwardEvent::MidiControlEvent(event)));
                }
                AudioLayerOutwardEvent::GeneralMMCEvent(event) => {
                    let _ = proxy.message(AudioLayerEvent::AudioLayerOutward(AudioLayerOutwardEvent::GeneralMMCEvent(event)));
                }
                AudioLayerOutwardEvent::PlayPositionInFrames(position) => {
                    let _ = proxy.message(AudioLayerEvent::AudioLayerOutward(AudioLayerOutwardEvent::PlayPositionInFrames(position)));
                }
                AudioLayerOutwardEvent::JackRestartRequired => {
                    let _ = proxy.message(AudioLayerEvent::AudioLayerOutward(AudioLayerOutwardEvent::JackRestartRequired));
                }
                AudioLayerOutwardEvent::JackConnect(thing, thang) => {
                    let _ = proxy.message(AudioLayerEvent::AudioLayerOutward(AudioLayerOutwardEvent::JackConnect(thing, thang)));
                }
                AudioLayerOutwardEvent::MasterChannelLevels(volume, pan) => {
                    let _ = proxy.message(AudioLayerEvent::AudioLayerOutward(AudioLayerOutwardEvent::MasterChannelLevels(volume, pan)));
                }
            }
        }
    }

    pub fn handle_incoming_jack_immediate_events(&mut self, proxy: &MessageProxy<AudioLayerEvent>) {
        while !self.jack_time_critical_midi_receiver.is_empty() {
            if let Ok(event) = self.jack_time_critical_midi_receiver.try_recv() {
                match event {
                    AudioLayerTimeCriticalOutwardEvent::MidiEvent(jack_midi_event) => {
                        // println!("handle_incoming_jack_immediate_events AudioLayerTimeCriticalOutwardEvent::MidiEvent: {}", event.data[0]);
                        // let _ = proxy.message(AudioLayerEvent::AudioLayerTimeCriticalOutwardEvent(AudioLayerTimeCriticalOutwardEvent::MidiEvent(event)));
                        // send this straight to the selected track background processsor
                        if let Some(track_uuid) = self.selected_track.as_ref() {
                            if let Some(selected_track_background_processor_sender) = self.track_senders.get(track_uuid) {
                                let midi_msg_type = jack_midi_event.data[0] as i32;
                                let midi_channel = 0;

                                if (144..=159).contains(&midi_msg_type) {
                                    // selected_track_background_processor_sender.send(TrackBackgroundProcessorInwardEvent::PlayNoteImmediate(jack_midi_event.data[1] as i32, midi_channel));
                                } else if (128..=143).contains(&midi_msg_type) {
                                    // selected_track_background_processor_sender.send(TrackBackgroundProcessorInwardEvent::StopNoteImmediate(jack_midi_event.data[1] as i32, midi_channel));
                                } else if (176..=191).contains(&midi_msg_type) {
                                    selected_track_background_processor_sender.send(TrackBackgroundProcessorInwardEvent::PlayControllerImmediate(jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32, midi_channel));
                                } else if (224..=239).contains(&midi_msg_type) {
                                    selected_track_background_processor_sender.send(TrackBackgroundProcessorInwardEvent::PlayPitchBendImmediate(jack_midi_event.data[1] as i32, jack_midi_event.data[2] as i32, midi_channel));
                                } else {
                                    debug!("Unknown jack midi event: ");
                                    for event_byte in jack_midi_event.data.iter() {
                                        debug!(" {}", event_byte);
                                    }
                                    debug!("");
                                }
                            }
                        }
                    }
                    AudioLayerTimeCriticalOutwardEvent::TrackVolumePanLevel(volumePanEvent) => {
                        println!("handle_incoming_jack_immediate_events AudioLayerTimeCriticalOutwardEvent::TrackVolumePanLevel: {}", volumePanEvent.data[0]);
                        let _ = proxy.message(AudioLayerEvent::AudioLayerTimeCriticalOutwardEvent(AudioLayerTimeCriticalOutwardEvent::TrackVolumePanLevel(volumePanEvent)));
                    }
                }
            }
        }
    }
}