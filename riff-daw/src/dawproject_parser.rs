use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io::{BufReader, Read};
use std::sync::{MutexGuard};
use base64::Engine as _;
use dawproject::project::{
    ApplicationType, ArrangementType, AuPluginType, AudioType, BoolParameterType, BuiltinDeviceType,
    ChannelDevicesElementType, ChannelDevicesElementTypeContent, ChannelType, ClipSlotType,
    ClipType, ClipTypeContent, ClipsType, ContentType, ContentTypeList, DeviceParametersElementType,
    DeviceRoleType, FileReferenceType, LanesType, LanesTypeContent, MixerRoleType, NoteType,
    NotesType, ProjectScenesElementType, ProjectStructureElementType,
    ProjectStructureElementTypeContent, ProjectType, RealParameterType, SceneType,
    SceneTypeContent, TimeSignatureParameterType, TimeUnitType, TrackType as DawTrackType,
    TransportType, UnitType, WarpsTypeContent,
};
use dawproject::{Dawproject, DawprojectReadError, DawprojectReader, DawprojectWriteError, DawprojectWriter, MetaData};
use uuid::Uuid;

use crate::domain::{
    AudioEffectTrack, AudioPlugin, AudioTrack, Automation, DAWItemLength, DAWItemPosition,
    InstrumentTrack, MidiTrack, Note, Project, Riff, RiffReference, RiffSet, Sample,
    SampleReference, Song, Track, TrackEvent, TrackType, UuidWrapper,
};

#[derive(Debug)]
pub enum DawprojectParseError {
    Read(DawprojectReadError),
    Write(DawprojectWriteError),
    Missing(String),
}

impl fmt::Display for DawprojectParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DawprojectParseError::Read(error) => write!(f, "failed to read dawproject file: {}", error),
            DawprojectParseError::Write(error) => write!(f, "failed to write dawproject file: {}", error),
            DawprojectParseError::Missing(what) => write!(f, "missing {}", what),
        }
    }
}

impl std::error::Error for DawprojectParseError {}

/// Parse a `.dawproject` file on disk into the domain [`Project`].
pub fn parse_dawproject(path: &str) -> Result<Project, DawprojectParseError> {
    let mut reader = DawprojectReader::open(path).map_err(DawprojectParseError::Read)?;
    reader.read_dawproject().map_err(DawprojectParseError::Read)?;
    let dawproject = reader
        .build_dawproject()
        .ok_or_else(|| DawprojectParseError::Missing("dawproject contents".to_string()))?;
    Ok(parse_dawproject_inner(&dawproject, &mut reader))
}

/// Write the domain [`Project`] to a `.dawproject` file on disk.
pub fn write_dawproject(project: MutexGuard<Project>, path: &str) -> Result<(), DawprojectParseError> {
    let (project_type, preset_files) = domain_to_dawproject(&project.song);
    let metadata = MetaData {
        title: Some(project.song.name().to_string()),
        artist: None,
        album: None,
        original_artist: None,
        composer: None,
        songwriter: None,
        producer: None,
        arranger: None,
        year: None,
        genre: None,
        copyright: None,
        website: None,
        comment: None,
    };
    let dawproject = Dawproject::new(metadata, project_type);
    let mut writer = DawprojectWriter::create(path).map_err(DawprojectParseError::Write)?;
    writer
        .write_dawproject(&dawproject)
        .map_err(DawprojectParseError::Write)?;
    for (file_name, data) in preset_files {
        writer
            .write_file(&file_name, &data)
            .map_err(DawprojectParseError::Write)?;
    }
    writer.finish().map_err(DawprojectParseError::Write)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Read: dawproject -> domain
// ---------------------------------------------------------------------------

fn parse_dawproject_inner(
    dawproject: &Dawproject,
    reader: &mut DawprojectReader<BufReader<File>>,
) -> Project {
    let project_type = dawproject.project();
    let mut song = Song::new();
    song.tracks_mut().clear();

    if let Some(title) = dawproject.metadata().title.as_ref() {
        *song.name_mut() = title.clone();
    }

    read_transport(&mut song, &project_type.transport);
    let track_id_map = read_structure(&mut song, &project_type.structure, reader);
    read_arrangement(&mut song, &project_type.arrangement, &track_id_map);
    read_scenes(&mut song, &project_type.scenes, &track_id_map);
    song.recalculate_song_length();

    Project { song }
}

fn read_transport(song: &mut Song, transport: &Option<TransportType>) {
    if let Some(transport) = transport {
        if let Some(tempo) = &transport.tempo
            && let Some(value) = &tempo.value
            && let Ok(tempo_value) = value.parse::<f64>()
        {
            song.set_tempo(tempo_value);
        }
        if let Some(time_signature) = &transport.time_signature {
            song.set_time_signature_numerator(time_signature.numerator as f64);
            song.set_time_signature_denominator(time_signature.denominator as f64);
        }
    }
}

/// Convert the `<Structure>` into domain tracks and return a map of
/// dawproject track id -> domain track uuid.
fn read_structure(
    song: &mut Song,
    structure: &Option<ProjectStructureElementType>,
    reader: &mut DawprojectReader<BufReader<File>>,
) -> HashMap<String, String> {
    let mut track_id_map = HashMap::new();
    if let Some(structure) = structure {
        for content in &structure.content {
            if let ProjectStructureElementTypeContent::Track(track) = content {
                let domain_track = read_track(track, reader);
                if let Some(daw_track_id) = &track.id {
                    track_id_map.insert(daw_track_id.clone(), domain_track.uuid());
                }
                song.tracks_mut().push(domain_track);
            }
        }
    }
    track_id_map
}

fn read_track(track: &DawTrackType, reader: &mut DawprojectReader<BufReader<File>>) -> TrackType {
    let track_uuid = track
        .id
        .as_deref()
        .and_then(|id| Uuid::parse_str(id).ok())
        .unwrap_or_else(Uuid::new_v4)
        .to_string();
    let name = track.name.clone().unwrap_or_else(|| "Unknown".to_string());
    let (red, green, blue, alpha) = track
        .color
        .as_deref()
        .and_then(parse_color)
        .unwrap_or((1.0, 0.0, 0.0, 0.5));

    let channel = track.channel.as_ref();
    let mute = channel
        .and_then(|c| c.mute.as_ref())
        .and_then(|mute| mute.value)
        .unwrap_or(false);
    let solo = channel.and_then(|c| c.solo).unwrap_or(false);
    let volume = channel
        .and_then(|c| c.volume.as_ref())
        .and_then(|v| v.value.as_ref())
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(1.0) as f32;
    let pan = channel
        .and_then(|c| c.pan.as_ref())
        .and_then(|p| p.value.as_ref())
        .and_then(|p| p.parse::<f64>().ok())
        .map(daw_pan_to_domain_pan)
        .unwrap_or(0.0) as f32;

    let (plugins, has_instrument) = read_devices(channel, reader);
    let has_audio = track_has_content_type(track, ContentType::Audio);

    if has_instrument {
        let mut instrument = AudioPlugin::new();
        let mut effects = vec![];
        for plugin in plugins {
            if plugin.is_instrument && instrument.name() == "Unknown" {
                instrument = plugin;
            } else {
                effects.push(plugin);
            }
        }
        TrackType::InstrumentTrack(InstrumentTrack {
            uuid: track_uuid,
            name,
            mute,
            solo,
            red,
            green,
            blue,
            alpha,
            instrument,
            effects,
            riffs: vec![],
            riff_refs: vec![],
            automation: Automation::new(),
            volume,
            pan,
            midi_routings: vec![],
            audio_routings: vec![],
        })
    } else if has_audio {
        let mut domain_track = AudioTrack::new();
        domain_track.set_uuid(Uuid::parse_str(&track_uuid).unwrap_or_else(|_| Uuid::new_v4()));
        domain_track.set_name(name);
        domain_track.set_mute(mute);
        domain_track.set_solo(solo);
        domain_track.set_colour(red, green, blue, alpha);
        domain_track.set_volume(volume);
        domain_track.set_pan(pan);
        domain_track.set_effects(plugins);
        TrackType::AudioTrack(domain_track)
    } else {
        let mut domain_track = MidiTrack::new();
        domain_track.set_uuid(Uuid::parse_str(&track_uuid).unwrap_or_else(|_| Uuid::new_v4()));
        domain_track.set_name(name);
        domain_track.set_mute(mute);
        domain_track.set_solo(solo);
        domain_track.set_colour(red, green, blue, alpha);
        domain_track.set_volume(volume);
        domain_track.set_pan(pan);
        TrackType::MidiTrack(domain_track)
    }
}

fn read_devices(
    channel: Option<&ChannelType>,
    reader: &mut DawprojectReader<BufReader<File>>,
) -> (Vec<AudioPlugin>, bool) {
    let mut plugins = vec![];
    let mut has_instrument = false;
    if let Some(channel) = channel
        && let Some(devices) = &channel.devices
    {
        for device in &devices.content {
                let (plugin, is_instrument) = match device {
                    ChannelDevicesElementTypeContent::AuPlugin(plugin) => {
                        (plugin_from_au(plugin, "AU", reader), plugin.device_role == DeviceRoleType::Instrument)
                    }
                    ChannelDevicesElementTypeContent::Vst2Plugin(plugin) => {
                        (plugin_from_au(plugin, "VST2", reader), plugin.device_role == DeviceRoleType::Instrument)
                    }
                    ChannelDevicesElementTypeContent::Vst3Plugin(plugin) => {
                        (plugin_from_au(plugin, "VST3", reader), plugin.device_role == DeviceRoleType::Instrument)
                    }
                    ChannelDevicesElementTypeContent::ClapPlugin(plugin) => {
                        (plugin_from_au(plugin, "CLAP", reader), plugin.device_role == DeviceRoleType::Instrument)
                    }
                    ChannelDevicesElementTypeContent::BuiltinDevice(device) => (
                        plugin_from_device(
                            device.device_id.clone(),
                            &device.device_name,
                            device.device_vendor.clone(),
                            &device.device_role,
                            "Builtin",
                            device.state.as_ref(),
                            reader,
                        ),
                        device.device_role == DeviceRoleType::Instrument,
                    ),
                    ChannelDevicesElementTypeContent::Device(device) => (
                        plugin_from_device(
                            device.device_id.clone(),
                            &device.device_name,
                            device.device_vendor.clone(),
                            &device.device_role,
                            "Device",
                            device.state.as_ref(),
                            reader,
                        ),
                        device.device_role == DeviceRoleType::Instrument,
                    ),
                    _ => continue,
                };
                has_instrument |= is_instrument;
                plugins.push(plugin);
            }
        }
    (plugins, has_instrument)
}
fn plugin_from_au(
    plugin: &AuPluginType,
    format: &str,
    reader: &mut DawprojectReader<BufReader<File>>,
) -> AudioPlugin {
    let is_clap = format.eq_ignore_ascii_case("CLAP");
    let uuid = plugin
        .device_id
        .clone()
        .or_else(|| plugin.id.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let sub_plugin_id = if is_clap { plugin.device_id.clone() } else { None };
    let mut audio_plugin = AudioPlugin {
        uuid,
        name: plugin.device_name.clone(),
        descriptive_name: plugin.name.clone().unwrap_or_default(),
        format: format.to_string(),
        category: device_role_to_string(&plugin.device_role),
        manufacturer: plugin.device_vendor.clone().unwrap_or_default(),
        version: plugin.plugin_version.clone().unwrap_or_default(),
        file: String::new(),
        uid: plugin.device_id.clone().unwrap_or_default(),
        is_instrument: plugin.device_role == DeviceRoleType::Instrument,
        file_time: String::new(),
        info_update_time: String::new(),
        num_inputs: 0,
        num_outputs: 0,
        plugin_type: plugin_type_from_format(format),
        sub_plugin_id,
        preset_data: String::new(),
    };
    read_preset_data(&mut audio_plugin, plugin.state.as_ref(), reader);
    audio_plugin
}

fn plugin_type_from_format(format: &str) -> String {
    match format.to_uppercase().as_str() {
        "VST2" => "VST24".to_string(),
        other => other.to_string(),
    }
}

fn plugin_from_device(
    device_id: Option<String>,
    device_name: &str,
    device_vendor: Option<String>,
    device_role: &DeviceRoleType,
    format: &str,
    state: Option<&FileReferenceType>,
    reader: &mut DawprojectReader<BufReader<File>>,
) -> AudioPlugin {
    let uuid = device_id.clone().unwrap_or_else(|| Uuid::new_v4().to_string());
    let mut audio_plugin = AudioPlugin {
        uuid,
        name: device_name.to_string(),
        descriptive_name: String::new(),
        format: format.to_string(),
        category: device_role_to_string(device_role),
        manufacturer: device_vendor.unwrap_or_default(),
        version: String::new(),
        file: String::new(),
        uid: device_id.unwrap_or_default(),
        is_instrument: *device_role == DeviceRoleType::Instrument,
        file_time: String::new(),
        info_update_time: String::new(),
        num_inputs: 0,
        num_outputs: 0,
        plugin_type: format.to_string(),
        sub_plugin_id: None,
        preset_data: String::new(),
    };
    read_preset_data(&mut audio_plugin, state, reader);
    audio_plugin
}

fn read_preset_data(
    audio_plugin: &mut AudioPlugin,
    state: Option<&FileReferenceType>,
    reader: &mut DawprojectReader<BufReader<File>>,
) {
    let Some(state) = state else { return };
    let Ok(mut file) = reader.by_name(&state.path) else { return };
    let mut buf = Vec::new();
    if file.read_to_end(&mut buf).is_err() {
        return;
    }
    audio_plugin.preset_data = base64::engine::general_purpose::STANDARD.encode(&buf);
}

fn device_role_to_string(role: &DeviceRoleType) -> String {
    match role {
        DeviceRoleType::Instrument => "instrument",
        DeviceRoleType::NoteFx => "noteFx",
        DeviceRoleType::AudioFx => "audioFx",
        DeviceRoleType::Analyzer => "analyzer",
    }
    .to_string()
}

fn track_has_content_type(track: &DawTrackType, wanted: ContentType) -> bool {
    track
        .content_types
        .as_ref()
        .map(|content_types| content_types.0.contains(&wanted))
        .unwrap_or(false)
}

/// Convert the `<Arrangement>` lanes into per-track riffs, riff refs and samples.
fn read_arrangement(
    song: &mut Song,
    arrangement: &Option<ArrangementType>,
    track_id_map: &HashMap<String, String>,
) {
    let Some(arrangement) = arrangement else { return };
    let Some(lanes) = &arrangement.lanes else { return };

    let mut samples: HashMap<String, Sample> = HashMap::new();
    let mut builders: HashMap<String, TrackContentBuilder> = HashMap::new();
    collect_lanes(lanes, 0.0, &mut samples, &mut builders);

    for (daw_track_id, builder) in builders {
        let Some(domain_uuid) = track_id_map.get(&daw_track_id).and_then(|id| Uuid::parse_str(id).ok()) else {
            continue;
        };
        if let Some(track) = song.track_mut(&domain_uuid) {
            *track.riffs_mut() = builder.riffs;
            *track.riff_refs_mut() = builder.riff_refs;
        }
    }

    for (sample_uuid, sample) in samples {
        song.samples_mut().insert(sample_uuid, sample);
    }
}

#[derive(Default)]
struct TrackContentBuilder {
    riffs: Vec<Riff>,
    riff_refs: Vec<RiffReference>,
}

fn collect_lanes(
    lanes: &LanesType,
    offset: f64,
    samples: &mut HashMap<String, Sample>,
    builders: &mut HashMap<String, TrackContentBuilder>,
) {
    let track_override = lanes.track.as_deref();
    for content in &lanes.content {
        match content {
            LanesTypeContent::Lanes(child) => collect_lanes(child, offset, samples, builders),
            LanesTypeContent::Notes(notes) => {
                let track_id = notes.track.as_deref().or(track_override);
                if let Some(track_id) = track_id {
                    let builder = builders.entry(track_id.to_string()).or_default();
                    let riff = notes_to_riff(notes);
                    let riff_uuid = riff.uuid().to_string();
                    builder.riffs.push(riff);
                    builder.riff_refs.push(RiffReference::new(riff_uuid, offset));
                }
            }
            LanesTypeContent::Clips(clips) => {
                let track_id = clips.track.as_deref().or(track_override);
                if let Some(track_id) = track_id {
                    let builder = builders.entry(track_id.to_string()).or_default();
                    let lane_name = clips.name.clone().unwrap_or_default();
                    for clip in &clips.clip {
                        collect_clip(clip, offset, &lane_name, builder, samples);
                    }
                }
            }
            LanesTypeContent::ClipSlot(clip_slot) => {
                let track_id = clip_slot.track.as_deref().or(track_override);
                if let Some(track_id) = track_id {
                    if let Some(clip) = clip_slot.clip.as_deref() {
                        let builder = builders.entry(track_id.to_string()).or_default();
                        let lane_name = clip_slot.name.clone().unwrap_or_default();
                        collect_clip(clip, offset, &lane_name, builder, samples);
                    }
                }
            }
            LanesTypeContent::Audio(audio) => {
                let track_id = audio.track.as_deref().or(track_override);
                if let Some(track_id) = track_id {
                    let builder = builders.entry(track_id.to_string()).or_default();
                    let name = audio.name.clone().unwrap_or_default();
                    collect_audio(audio, offset, None, &name, builder, samples);
                }
            }
            _ => {}
        }
    }
}

fn collect_clip(
    clip: &ClipType,
    offset: f64,
    default_name: &str,
    builder: &mut TrackContentBuilder,
    samples: &mut HashMap<String, Sample>,
) {
    let clip_time = offset + clip.time;
    let clip_name = clip.name.as_deref().unwrap_or(default_name);
    match &clip.content {
        Some(ClipTypeContent::Notes(notes)) => {
            let mut riff = notes_to_riff(notes);
            riff.set_name(clip_name.to_string());
            if let Some(duration) = clip.duration
                && duration > riff.length()
            {
                riff.set_length(duration);
            }
            let riff_uuid = riff.uuid().to_string();
            builder.riffs.push(riff);
            builder.riff_refs.push(RiffReference::new(riff_uuid, clip_time));
        }
        Some(ClipTypeContent::Clips(clips)) => {
            for nested in &clips.clip {
                collect_clip(nested, clip_time, clip_name, builder, samples);
            }
        }
        Some(ClipTypeContent::Audio(audio)) => {
            collect_audio(audio, clip_time, clip.duration, clip_name, builder, samples);
        }
        Some(ClipTypeContent::Warps(warps)) => {
            for warp_content in &warps.content {
                if let WarpsTypeContent::Audio(audio) = warp_content {
                    collect_audio(audio, clip_time, clip.duration, clip_name, builder, samples);
                }
            }
        }
        _ => {}
    }
}

fn collect_audio(
    audio: &AudioType,
    offset: f64,
    duration: Option<f64>,
    name: &str,
    builder: &mut TrackContentBuilder,
    samples: &mut HashMap<String, Sample>,
) {
    let sample_uuid = audio
        .id
        .as_deref()
        .and_then(|id| Uuid::parse_str(id).ok())
        .unwrap_or_else(Uuid::new_v4)
        .to_string();
    let sample_name = audio.name.clone().unwrap_or_else(|| name.to_string());
    let sample = Sample::new(sample_name, audio.file.path.clone(), sample_uuid.clone());
    samples.insert(sample_uuid.clone(), sample);

    let length = duration.unwrap_or(audio.duration);
    let mut riff = Riff::new_with_name_and_length(Uuid::new_v4(), name.to_string(), length);
    riff.events_mut()
        .push(TrackEvent::Sample(SampleReference::new(0.0, sample_uuid)));
    let riff_uuid = riff.uuid().to_string();
    builder.riffs.push(riff);
    builder.riff_refs.push(RiffReference::new(riff_uuid, offset));
}

fn notes_to_riff(notes: &NotesType) -> Riff {
    let mut events = vec![];
    let mut end_beats = 0.0f64;
    for note in &notes.note {
        let time = parse_f64(&note.time);
        let duration = parse_f64(&note.duration);
        let velocity = note
            .vel
            .as_deref()
            .map(parse_f64)
            .unwrap_or(0.8);
        end_beats = end_beats.max(time + duration);
        events.push(TrackEvent::Note(Note {
            id: UuidWrapper::new_v4(),
            note_id: 0,
            port: 0,
            channel: note.channel as u16,
            position: time,
            note: note.key,
            velocity: (velocity * 127.0).round() as i32,
            length: duration,
            riff_start_note: false,
        }));
    }
    let uuid = notes
        .id
        .as_deref()
        .and_then(|id| Uuid::parse_str(id).ok())
        .unwrap_or_else(Uuid::new_v4);
    let name = notes.name.clone().unwrap_or_default();
    let mut riff = Riff::new_with_name_and_length(uuid, name, end_beats);
    *riff.events_mut() = events;
    riff
}

/// Convert the `<Scenes>` into per-track riffs and [`RiffSet`]s.
fn read_scenes(
    song: &mut Song,
    scenes: &Option<ProjectScenesElementType>,
    track_id_map: &HashMap<String, String>,
) {
    let Some(scenes) = scenes else { return };

    let mut samples: HashMap<String, Sample> = HashMap::new();
    for scene in &scenes.scene {
        let mut builders: HashMap<String, TrackContentBuilder> = HashMap::new();
        collect_scene_content(&scene.content, &mut samples, &mut builders);
        if builders.is_empty() {
            continue;
        }

        let scene_uuid = scene
            .id
            .as_deref()
            .and_then(|id| Uuid::parse_str(id).ok())
            .unwrap_or_else(Uuid::new_v4);
        let mut riff_set = RiffSet::new_with_uuid(scene_uuid);
        riff_set.set_name(scene.name.clone().unwrap_or_default());

        for (daw_track_id, builder) in builders {
            let Some(domain_uuid) = track_id_map
                .get(&daw_track_id)
                .and_then(|id| Uuid::parse_str(id).ok())
            else {
                continue;
            };
            let Some(track) = song.track_mut(&domain_uuid) else { continue };
            track.riffs_mut().extend(builder.riffs);
            for riff_ref in builder.riff_refs {
                riff_set.set_riff_ref_for_track(domain_uuid.to_string(), riff_ref);
            }
        }

        song.add_riff_set(riff_set);
    }

    for (sample_uuid, sample) in samples {
        song.samples_mut().insert(sample_uuid, sample);
    }
}

fn collect_scene_content(
    scene_content: &SceneTypeContent,
    samples: &mut HashMap<String, Sample>,
    builders: &mut HashMap<String, TrackContentBuilder>,
) {
    match scene_content {
        SceneTypeContent::Lanes(lanes) => collect_lanes(lanes, 0.0, samples, builders),
        SceneTypeContent::Clips(clips) => {
            let track_id = clips.track.as_deref();
            if let Some(track_id) = track_id {
                let builder = builders.entry(track_id.to_string()).or_default();
                let lane_name = clips.name.clone().unwrap_or_default();
                for clip in &clips.clip {
                    collect_clip(clip, 0.0, &lane_name, builder, samples);
                }
            }
        }
        SceneTypeContent::Notes(notes) => {
            let track_id = notes.track.as_deref();
            if let Some(track_id) = track_id {
                let builder = builders.entry(track_id.to_string()).or_default();
                let riff = notes_to_riff(notes);
                let riff_uuid = riff.uuid().to_string();
                builder.riffs.push(riff);
                builder.riff_refs.push(RiffReference::new(riff_uuid, 0.0));
            }
        }
        SceneTypeContent::Audio(audio) => {
            let track_id = audio.track.as_deref();
            if let Some(track_id) = track_id {
                let builder = builders.entry(track_id.to_string()).or_default();
                let name = audio.name.clone().unwrap_or_default();
                collect_audio(audio, 0.0, None, &name, builder, samples);
            }
        }
        SceneTypeContent::Warps(warps) => {
            for warp_content in &warps.content {
                if let WarpsTypeContent::Audio(audio) = warp_content {
                    let track_id = audio.track.as_deref();
                    if let Some(track_id) = track_id {
                        let builder = builders.entry(track_id.to_string()).or_default();
                        let name = audio.name.clone().unwrap_or_default();
                        collect_audio(audio, 0.0, None, &name, builder, samples);
                    }
                }
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Write: domain -> dawproject
// ---------------------------------------------------------------------------

fn domain_to_dawproject(song: &Song) -> (ProjectType, Vec<(String, Vec<u8>)>) {
    let mut structure_content = vec![];
    let mut arrangement_lanes = vec![];
    let mut preset_files = vec![];
    let mut daw_track_ids: HashMap<String, String> = HashMap::new();
    let mut next_id: u32 = 10000;
    for (track_counter, track) in song.tracks().iter().enumerate() {
        let daw_track_id = format!("id{}", track_counter);
        daw_track_ids.insert(track.uuid(), daw_track_id.clone());
        let (daw_track, lanes) = domain_track_to_daw_track(
            track,
            &daw_track_id,
            song.samples(),
            &mut preset_files,
            &mut next_id,
        );
        structure_content.push(ProjectStructureElementTypeContent::Track(daw_track));
        arrangement_lanes.push(LanesTypeContent::Lanes(lanes));
    }

    let transport = TransportType {
        tempo: Some(real_parameter(
            "Tempo",
            song.tempo(),
            UnitType::Bpm,
            "666.0",
            "20.0",
        )),
        time_signature: Some(TimeSignatureParameterType {
            name: Some("TimeSignature".to_string()),
            color: None,
            comment: None,
            id: None,
            parameter_id: None,
            denominator: song.time_signature_denominator() as i32,
            numerator: song.time_signature_numerator() as i32,
        }),
    };

    let project = ProjectType {
        version: "1.0".to_string(),
        application: ApplicationType {
            name: "daw_project_parser".to_string(),
            version: "1.0".to_string(),
        },
        transport: Some(transport),
        structure: Some(ProjectStructureElementType {
            content: structure_content,
        }),
        arrangement: Some(ArrangementType {
            name: None,
            color: None,
            comment: None,
            id: None,
            lanes: Some(LanesType {
                name: None,
                color: None,
                comment: None,
                id: None,
                time_unit: Some(TimeUnitType::Beats),
                track: None,
                content: arrangement_lanes,
            }),
            markers: None,
            tempo_automation: None,
            time_signature_automation: None,
        }),
        scenes: riff_sets_to_scenes(song, &daw_track_ids, &mut next_id),
    };
    (project, preset_files)
}

fn domain_track_to_daw_track(
    track: &TrackType,
    daw_track_id: &str,
    samples: &HashMap<String, Sample>,
    preset_files: &mut Vec<(String, Vec<u8>)>,
    next_id: &mut u32,
) -> (DawTrackType, LanesType) {
    let name = track.name().to_string();
    let colour = track.colour();
    let mute = track.mute();
    let solo = track.solo();
    let volume = track.volume();
    let pan = track.pan();

    let (devices, _is_instrument) = domain_devices(track, preset_files, next_id);

    let content_types = match track {
        TrackType::AudioTrack(_) => vec![ContentType::Audio],
        _ => vec![ContentType::Notes],
    };

    let channel = ChannelType {
        name: None,
        color: None,
        comment: None,
        id: None,
        audio_channels: Some(2),
        destination: None,
        role: Some(MixerRoleType::Regular),
        solo: Some(solo),
        devices: Some(ChannelDevicesElementType { content: devices }),
        mute: Some(bool_parameter("Mute", mute)),
        pan: Some(real_parameter(
            "Pan",
            domain_pan_to_daw_pan(pan),
            UnitType::Normalized,
            "1.0",
            "0.0",
        )),
        sends: None,
        volume: Some(real_parameter("Volume", volume as f64, UnitType::Linear, "2.0", "0.0")),
    };

    let daw_track = DawTrackType {
        name: Some(name),
        color: Some(to_color_string(colour)),
        comment: None,
        id: Some(daw_track_id.to_string()),
        content_types: Some(ContentTypeList(content_types)),
        loaded: Some(true),
        channel: Some(channel),
        track: vec![],
    };

    let lanes = domain_track_to_lanes(track, daw_track_id, samples);
    (daw_track, lanes)
}

fn domain_devices(
    track: &TrackType,
    preset_files: &mut Vec<(String, Vec<u8>)>,
    next_id: &mut u32,
) -> (Vec<ChannelDevicesElementTypeContent>, bool) {
    let mut devices = vec![];
    let mut has_instrument = false;
    match track {
        TrackType::InstrumentTrack(instrument_track) => {
            devices.push(plugin_to_device(
                &instrument_track.instrument,
                DeviceRoleType::Instrument,
                preset_files,
                next_id,
            ));
            has_instrument = true;
            for effect in instrument_track.effects() {
                devices.push(plugin_to_device(
                    effect,
                    DeviceRoleType::AudioFx,
                    preset_files,
                    next_id,
                ));
            }
        }
        TrackType::AudioTrack(audio_track) => {
            for effect in audio_track.effects() {
                devices.push(plugin_to_device(
                    effect,
                    DeviceRoleType::AudioFx,
                    preset_files,
                    next_id,
                ));
            }
        }
        TrackType::MidiTrack(_) => {}
    }
    (devices, has_instrument)
}

fn plugin_to_device(
    plugin: &AudioPlugin,
    role: DeviceRoleType,
    preset_files: &mut Vec<(String, Vec<u8>)>,
    next_id: &mut u32,
) -> ChannelDevicesElementTypeContent {
    let format = plugin_format(plugin);
    let id = if matches!(format.as_str(), "VST3" | "VST2" | "VST24" | "VST" | "CLAP") {
        let id = format!("id{}", *next_id);
        *next_id += 1;
        id
    } else {
        plugin.uuid.clone()
    };
    match format.as_str() {
        "VST3" => ChannelDevicesElementTypeContent::Vst3Plugin(au_plugin(plugin, &role, preset_files, &id)),
        "VST2" | "VST24" | "VST" => ChannelDevicesElementTypeContent::Vst2Plugin(au_plugin(plugin, &role, preset_files, &id)),
        "CLAP" => ChannelDevicesElementTypeContent::ClapPlugin(au_plugin(plugin, &role, preset_files, &id)),
        "AU" => ChannelDevicesElementTypeContent::AuPlugin(au_plugin(plugin, &role, preset_files, &id)),
        _ => ChannelDevicesElementTypeContent::BuiltinDevice(BuiltinDeviceType {
            name: Some(plugin.name.clone()),
            color: None,
            comment: None,
            id: Some(plugin.uuid.clone()),
            device_id: Some(plugin.uuid.clone()),
            device_name: plugin.name.clone(),
            device_role: role,
            device_vendor: optional_string(&plugin.manufacturer),
            loaded: Some(true),
            parameters: Some(DeviceParametersElementType { content: vec![] }),
            enabled: Some(bool_parameter("On/Off", true)),
            state: plugin_state(plugin, preset_files),
        }),
    }
}

/// Resolve the plugin format to use when writing, preferring the explicit
/// `format` field and falling back to `plugin_type` (e.g. "VST24") which is
/// what the domain uses when `format` is left as "Unknown".
fn plugin_format(plugin: &AudioPlugin) -> String {
    let format = plugin.format.to_uppercase();
    if matches!(format.as_str(), "VST2" | "VST3" | "CLAP" | "AU") {
        format
    } else {
        plugin.plugin_type.to_uppercase()
    }
}

fn au_plugin(
    plugin: &AudioPlugin,
    role: &DeviceRoleType,
    preset_files: &mut Vec<(String, Vec<u8>)>,
    id: &str,
) -> AuPluginType {
    let format = plugin_format(plugin);
    let device_id = if format == "CLAP" {
        plugin
            .sub_plugin_id
            .clone()
            .filter(|id| !id.is_empty())
            .or_else(|| Some(plugin.uuid.clone()))
    } else {
        Some(plugin.uuid.clone())
    };
    AuPluginType {
        name: Some(plugin.name.clone()),
        color: None,
        comment: None,
        id: Some(id.to_string()),
        device_id,
        device_name: plugin.name.clone(),
        device_role: role.clone(),
        device_vendor: optional_string(&plugin.manufacturer),
        loaded: Some(true),
        plugin_version: optional_string(&plugin.version),
        parameters: Some(DeviceParametersElementType { content: vec![] }),
        enabled: Some(bool_parameter("On/Off", true)),
        state: plugin_state(plugin, preset_files),
    }
}

fn plugin_state(
    plugin: &AudioPlugin,
    preset_files: &mut Vec<(String, Vec<u8>)>,
) -> Option<FileReferenceType> {
    if plugin.preset_data.is_empty() {
        return None;
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(plugin.preset_data.as_bytes())
        .ok()?;
    let path = format!("files/{}.state", Uuid::new_v4());
    preset_files.push((path.clone(), decoded));
    Some(FileReferenceType {
        path,
        external: Some(false),
    })
}

fn optional_string(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn domain_track_to_lanes(
    track: &TrackType,
    daw_track_id: &str,
    samples: &HashMap<String, Sample>,
) -> LanesType {
    let mut clips = vec![];
    for riff_ref in track.riff_refs() {
        let Some(riff) = track
            .riffs()
            .iter()
            .find(|riff| riff.uuid().to_string() == riff_ref.linked_to())
        else {
            continue;
        };
        let (content, _has_content) = riff_to_clip_content(riff, samples);
        clips.push(riff_ref_to_clip(riff_ref, riff, content));
    }

    LanesType {
        name: None,
        color: None,
        comment: None,
        id: None,
        time_unit: Some(TimeUnitType::Beats),
        track: Some(daw_track_id.to_string()),
        content: vec![LanesTypeContent::Clips(ClipsType {
            name: None,
            color: None,
            comment: None,
            id: None,
            time_unit: Some(TimeUnitType::Beats),
            track: Some(daw_track_id.to_string()),
            clip: clips,
        })],
    }
}

fn riff_ref_to_clip(
    riff_ref: &RiffReference,
    riff: &Riff,
    content: Option<ClipTypeContent>,
) -> ClipType {
    ClipType {
        name: if riff.name().is_empty() {
            None
        } else {
            Some(riff.name().to_string())
        },
        color: None,
        comment: None,
        time: riff_ref.position(),
        duration: Some(riff.length()),
        content_time_unit: Some(TimeUnitType::Beats),
        play_start: Some(0.0),
        play_stop: None,
        loop_start: None,
        loop_end: None,
        fade_time_unit: Some(TimeUnitType::Beats),
        fade_in_time: Some(0.0),
        fade_out_time: Some(0.0),
        reference: None,
        content,
        enable: Some(true),
    }
}

/// Convert the song's [`RiffSet`]s into `<Scenes>`.
fn riff_sets_to_scenes(
    song: &Song,
    daw_track_ids: &HashMap<String, String>,
    next_id: &mut u32,
) -> Option<ProjectScenesElementType> {
    if song.riff_sets().is_empty() {
        return None;
    }
    let scenes = song
        .riff_sets()
        .iter()
        .filter_map(|riff_set| {
            let lanes_content = riff_set_to_lanes_content(riff_set, song, daw_track_ids);
            if lanes_content.is_empty() {
                return None;
            }
            let scene_id = format!("id{}", *next_id);
            *next_id += 1;
            Some(SceneType {
                name: Some(riff_set.name().to_string()),
                color: None,
                comment: None,
                id: Some(scene_id),
                content: SceneTypeContent::Lanes(LanesType {
                    name: None,
                    color: None,
                    comment: None,
                    id: None,
                    time_unit: Some(TimeUnitType::Beats),
                    track: None,
                    content: lanes_content,
                }),
            })
        })
        .collect();
    Some(ProjectScenesElementType { scene: scenes })
}

fn riff_set_to_lanes_content(
    riff_set: &RiffSet,
    song: &Song,
    daw_track_ids: &HashMap<String, String>,
) -> Vec<LanesTypeContent> {
    let mut content = vec![];
    for (track_uuid, riff_ref) in riff_set.riff_refs() {
        let Some(daw_track_id) = daw_track_ids.get(track_uuid) else {
            continue;
        };
        let Some(track) = song.track(track_uuid.clone()) else {
            continue;
        };
        let Some(riff) = track
            .riffs()
            .iter()
            .find(|riff| riff.uuid().to_string() == riff_ref.linked_to())
        else {
            continue;
        };
        let (clip_content, _has_content) = riff_to_clip_content(riff, song.samples());
        content.push(LanesTypeContent::ClipSlot(ClipSlotType {
            name: None,
            color: None,
            comment: None,
            id: None,
            time_unit: Some(TimeUnitType::Beats),
            track: Some(daw_track_id.clone()),
            has_stop: Some(true),
            clip: Some(Box::new(riff_ref_to_clip(riff_ref, riff, clip_content))),
        }));
    }
    content
}

fn riff_to_clip_content(
    riff: &Riff,
    samples: &HashMap<String, Sample>,
) -> (Option<ClipTypeContent>, bool) {
    let mut notes = vec![];
    let mut audio = None;
    for event in riff.events() {
        match event {
            TrackEvent::Note(note) => {
                let velocity = note.velocity as f64 / 127.0;
                notes.push(NoteType {
                    time: fmt_time(note.position),
                    duration: fmt_time(note.length),
                    channel: note.channel as i32,
                    key: note.note,
                    vel: Some(fmt_time(velocity)),
                    rel: Some(fmt_time(velocity)),
                    content: None,
                });
            }
            TrackEvent::Sample(sample_ref) => {
                if let Some(sample) = samples.get(&sample_ref.sample_ref_uuid()) {
                    audio = Some(AudioType {
                        name: Some(sample.name().to_string()),
                        color: None,
                        comment: None,
                        id: Some(sample.uuid().to_string()),
                        time_unit: Some(TimeUnitType::Beats),
                        track: None,
                        duration: 0.0,
                        algorithm: None,
                        channels: 2,
                        sample_rate: 44100,
                        file: FileReferenceType {
                            path: sample.file_name().to_string(),
                            external: Some(false),
                        },
                    });
                }
            }
            _ => {}
        }
    }

    if !notes.is_empty() {
        (
            Some(ClipTypeContent::Notes(NotesType {
                name: Some(riff.name().to_string()),
                color: None,
                comment: None,
                id: None,
                time_unit: Some(TimeUnitType::Beats),
                track: None,
                note: notes,
            })),
            true,
        )
    } else if let Some(audio) = audio {
        (Some(ClipTypeContent::Audio(audio)), true)
    } else {
        (None, false)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn real_parameter(name: &str, value: f64, unit: UnitType, max: &str, min: &str) -> RealParameterType {
    RealParameterType {
        name: Some(name.to_string()),
        color: None,
        comment: None,
        id: None,
        parameter_id: None,
        max: Some(max.to_string()),
        min: Some(min.to_string()),
        unit,
        value: Some(value.to_string()),
    }
}

fn bool_parameter(name: &str, value: bool) -> BoolParameterType {
    BoolParameterType {
        name: Some(name.to_string()),
        color: None,
        comment: None,
        id: None,
        parameter_id: None,
        value: Some(value),
    }
}

fn parse_f64(value: &str) -> f64 {
    value.parse::<f64>().unwrap_or(0.0)
}

fn fmt_time(value: f64) -> String {
    let mut formatted = format!("{}", value);
    if !formatted.contains('.') {
        formatted.push_str(".0");
    }
    formatted
}

fn parse_color(hex: &str) -> Option<(f64, f64, f64, f64)> {
    let hex = hex.trim_start_matches('#');
    let (alpha_hex, rgb) = if hex.len() == 8 {
        (Some(&hex[0..2]), &hex[2..])
    } else if hex.len() == 6 {
        (None, hex)
    } else {
        return None;
    };
    let channel = |slice: &str| u8::from_str_radix(slice, 16).ok();
    let (red, green, blue) = (
        channel(&rgb[0..2])?,
        channel(&rgb[2..4])?,
        channel(&rgb[4..6])?,
    );
    let alpha = alpha_hex
        .and_then(channel)
        .map(|alpha| alpha as f64 / 255.0)
        .unwrap_or(1.0);
    Some((
        red as f64 / 255.0,
        green as f64 / 255.0,
        blue as f64 / 255.0,
        alpha,
    ))
}

fn to_color_string(colour: (f64, f64, f64, f64)) -> String {
    let (red, green, blue, alpha) = colour;
    if alpha >= 0.999 {
        format!(
            "#{:02X}{:02X}{:02X}",
            (red * 255.0) as u8,
            (green * 255.0) as u8,
            (blue * 255.0) as u8
        )
    } else {
        format!(
            "#{:02X}{:02X}{:02X}{:02X}",
            (alpha * 255.0) as u8,
            (red * 255.0) as u8,
            (green * 255.0) as u8,
            (blue * 255.0) as u8
        )
    }
}

fn daw_pan_to_domain_pan(pan: f64) -> f64 {
    (pan - 0.5) * 2.0
}

fn domain_pan_to_daw_pan(pan: f32) -> f64 {
    ((pan as f64 / 2.0) + 0.5).clamp(0.0, 1.0)
}
