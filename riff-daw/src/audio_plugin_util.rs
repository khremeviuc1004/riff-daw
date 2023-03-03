use std::{collections::HashMap, sync::{Arc, mpsc::Sender, Mutex}};
use std::{path::Path};
use std::path::PathBuf;
use std::process::{Command};

use pathsearch::find_executable_in_path;

use simple_clap_host_helper_lib::{plugin::{instance::process::{AudioBuffers, OutOfPlaceAudioBuffers, ProcessConfig, ProcessData}, library::PluginLibrary, ext::audio_ports::AudioPorts}, host::{DAWCallback}};
use vst::{host::{PluginInstance, PluginLoader}, plugin::Category, plugin::Plugin};
use vst::api::TimeInfo;

use crate::{domain::{VstHost}, event::AudioPluginHostOutwardEvent, constants::{VST24_CHECKER_EXECUTABLE_NAME, CLAP_CHECKER_EXECUTABLE_NAME}};

pub fn create_vst24_audio_plugin(
    vst24_plugin_loaders: Arc<Mutex<HashMap<String, PluginLoader<VstHost>>>>,
    library_path: &str,
    track_uuid: String,
    vst_plugin_uuid: String,
    sub_plugin_id: Option<String>,
    sender: Sender<AudioPluginHostOutwardEvent>,
    instrument: bool,
    vst_host_time_info: Arc<parking_lot::RwLock<TimeInfo>>,
) -> (Arc<Mutex<VstHost>>, PluginInstance) {
    let mut path_buf = PathBuf::new();
    let mut path = Path::new(library_path.clone());
    let host = Arc::new(Mutex::new(VstHost::new(
        track_uuid, 
        match &sub_plugin_id {
            Some(shell_id) => match shell_id.parse::<isize>() {
                Ok(shell_id) => Some(shell_id),
                Err(_) => None,
            },
            None => None,
        }, 
        sender, 
        vst_plugin_uuid, 
        instrument, 
        vst_host_time_info)));

    if !path.exists() || !path.is_file() {
        if let Ok(vst_path) = std::env::var("VST_PATH") {
            path_buf.push(vst_path.as_str());
            path_buf.push(library_path.clone());
            path = path_buf.as_path();
        }
    }

    println!("Loading {}...", path.to_str().unwrap());
    match vst24_plugin_loaders.lock() {
        Ok(mut loaders) => {
            let mut plugin_identifier = library_path.to_owned();
            if let Some(id) = sub_plugin_id {
                plugin_identifier.push('_');
                let shell_id = id.to_string();
                plugin_identifier.push_str(shell_id.as_str());
            }
            let plugin_loader = if let Some(_vst_plugin_loader) = loaders.get(&plugin_identifier) {
                loaders.get_mut(&plugin_identifier)
            }
            else {
                let plugin_loader = match PluginLoader::load(path, host.clone()) {
                    Ok(loader) => {
                        loaders.insert(plugin_identifier.clone(), loader);
                        loaders.get_mut(&plugin_identifier)
                    },
                    Err(error) => {
                        println!("{:?}", error);
                        panic!()
                    }
                };
                plugin_loader
            };
            let vst_loader = plugin_loader.unwrap();
            let mut instance = vst_loader.instance().unwrap();
            let info = instance.get_info();

            println!(
                "Loaded '{}':\n\t\
                    Vendor: {}\n\t\
                    Presets: {}\n\t\
                    Parameters: {}\n\t\
                    VST ID: {}\n\t\
                    Version: {}\n\t\
                    Input channels: {}\n\t\
                    Output channels: {}\n\t\
                    Category: {:?}\n\t\
                    F64 precision: {}\n\t\
                    Initial Delay: {} samples\n\t\
                    Can send events: {}\n\t\
                    Can send midi events: {}\n\t\
                    Can receive events: {}\n\t\
                    Can receive midi events: {}\n\t\
                    Can receive time info: {}\n\t\
                    Can offline: {}\n\t\
                    Can midi program names: {}\n\t\
                    Can bypass: {}\n\t\
                    Can receive sysex events: {}\n\t\
                    Can missing single note tune: {}\n\t\
                    Can midi key based instrument control: {}\n\t",
                info.name, info.vendor, info.presets, info.parameters, info.unique_id, info.version,
                info.inputs,
                info.outputs,
                info.category,
                info.f64_precision,
                info.initial_delay,
                match instance.can_do(vst::prelude::CanDo::SendEvents) {
                    vst::prelude::Supported::Yes => "Yes".to_string(),
                    vst::prelude::Supported::Maybe => "Maybe".to_string(),
                    vst::prelude::Supported::No => "No".to_string(),
                    vst::prelude::Supported::Custom(custom) => format!("Custom: {}", custom),
                },
                match instance.can_do(vst::prelude::CanDo::SendMidiEvent) {
                    vst::prelude::Supported::Yes => "Yes".to_string(),
                    vst::prelude::Supported::Maybe => "Maybe".to_string(),
                    vst::prelude::Supported::No => "No".to_string(),
                    vst::prelude::Supported::Custom(custom) => format!("Custom: {}", custom),
                },
                match instance.can_do(vst::prelude::CanDo::ReceiveEvents) {
                    vst::prelude::Supported::Yes => "Yes".to_string(),
                    vst::prelude::Supported::Maybe => "Maybe".to_string(),
                    vst::prelude::Supported::No => "No".to_string(),
                    vst::prelude::Supported::Custom(custom) => format!("Custom: {}", custom),
                },
                match instance.can_do(vst::prelude::CanDo::ReceiveMidiEvent) {
                    vst::prelude::Supported::Yes => "Yes".to_string(),
                    vst::prelude::Supported::Maybe => "Maybe".to_string(),
                    vst::prelude::Supported::No => "No".to_string(),
                    vst::prelude::Supported::Custom(custom) => format!("Custom: {}", custom),
                },
                match instance.can_do(vst::prelude::CanDo::ReceiveTimeInfo) {
                    vst::prelude::Supported::Yes => "Yes".to_string(),
                    vst::prelude::Supported::Maybe => "Maybe".to_string(),
                    vst::prelude::Supported::No => "No".to_string(),
                    vst::prelude::Supported::Custom(custom) => format!("Custom: {}", custom),
                },
                match instance.can_do(vst::prelude::CanDo::Offline) {
                    vst::prelude::Supported::Yes => "Yes".to_string(),
                    vst::prelude::Supported::Maybe => "Maybe".to_string(),
                    vst::prelude::Supported::No => "No".to_string(),
                    vst::prelude::Supported::Custom(custom) => format!("Custom: {}", custom),
                },
                match instance.can_do(vst::prelude::CanDo::MidiProgramNames) {
                    vst::prelude::Supported::Yes => "Yes".to_string(),
                    vst::prelude::Supported::Maybe => "Maybe".to_string(),
                    vst::prelude::Supported::No => "No".to_string(),
                    vst::prelude::Supported::Custom(custom) => format!("Custom: {}", custom),
                },
                match instance.can_do(vst::prelude::CanDo::Bypass) {
                    vst::prelude::Supported::Yes => "Yes".to_string(),
                    vst::prelude::Supported::Maybe => "Maybe".to_string(),
                    vst::prelude::Supported::No => "No".to_string(),
                    vst::prelude::Supported::Custom(custom) => format!("Custom: {}", custom),
                },
                match instance.can_do(vst::prelude::CanDo::ReceiveSysExEvent) {
                    vst::prelude::Supported::Yes => "Yes".to_string(),
                    vst::prelude::Supported::Maybe => "Maybe".to_string(),
                    vst::prelude::Supported::No => "No".to_string(),
                    vst::prelude::Supported::Custom(custom) => format!("Custom: {}", custom),
                },
                match instance.can_do(vst::prelude::CanDo::MidiSingleNoteTuningChange) {
                    vst::prelude::Supported::Yes => "Yes".to_string(),
                    vst::prelude::Supported::Maybe => "Maybe".to_string(),
                    vst::prelude::Supported::No => "No".to_string(),                
                    vst::prelude::Supported::Custom(custom) => format!("Custom: {}", custom),
                },
                match instance.can_do(vst::prelude::CanDo::MidiKeyBasedInstrumentControl) {
                    vst::prelude::Supported::Yes => "Yes".to_string(),
                    vst::prelude::Supported::Maybe => "Maybe".to_string(),
                    vst::prelude::Supported::No => "No".to_string(),
                    vst::prelude::Supported::Custom(custom) => format!("Custom: {}", custom),
                },
            );

            match info.category {
                Category::Synth => match host.lock() {
                    Ok(mut vst_host) => vst_host.set_instrument(true),
                    Err(_) => (),
                },
                _ => match host.lock() {
                    Ok(mut vst_host) => vst_host.set_instrument(false),
                    Err(_) => (),
                },
            }

            instance.set_sample_rate(44100.0);
            instance.set_block_size(1024);

            let presets = instance.get_parameter_object();
            for index in 0..info.presets {
                println!("Preset: Number={}, Name={}", index, presets.get_preset_name(index));
            }

            instance.init();
            instance.resume();
            instance.start_process();

            (host, instance)
        },
        Err(error) => {
            println!("Couldn't lock vst24_plugin_loaders: path={} error={:?}", path.to_str().unwrap(), error);
            panic!()
        },
    }
 }

 pub fn create_clap_audio_plugin(
    plugin_libraries: Arc<Mutex<HashMap<String, PluginLibrary>>>,
    audio_plugin_path: &str,
    _track_uuid: String,
    _plugin_uuid: String,
    clap_plugin_id: Option<String>,
    _sender: Sender<AudioPluginHostOutwardEvent>,
    _instrument: bool,   
 ) -> (simple_clap_host_helper_lib::plugin::instance::Plugin, ProcessData, crossbeam_channel::Receiver<DAWCallback>) {
    let path = Path::new(audio_plugin_path.clone());

    println!("Loading {}...", path.to_str().unwrap());
    match plugin_libraries.lock() {
        Ok(mut libraries) => {
            let plugin_identifier = audio_plugin_path.to_owned();
            if let Some(clap_plugin_id) = clap_plugin_id {
                let plugin_library = if let Some(clap_plugin_library) = libraries.get_mut(&plugin_identifier) {
                    clap_plugin_library
                }
                else {
                    let plugin_library = match PluginLibrary::load(path) {
                        Ok(library) => {
                            libraries.insert(plugin_identifier.clone(), library);
                            libraries.get_mut(&plugin_identifier)
                        },
                        Err(error) => {
                            println!("{:?}", error);
                            panic!()
                        }
                    };
                    plugin_library.unwrap()
                };
    
                let (host_sender, host_receiver) = crossbeam_channel::unbounded();
                let host = simple_clap_host_helper_lib::host::Host::new(host_sender);
            
                let plugin = if let Ok(plugin) = plugin_library.create_plugin(clap_plugin_id.as_str(), host) {
                    plugin
                }
                else {
                    panic!("Couldn't create the plugin.");
                };
            
            
                let _ = plugin.init();
            
                let audio_ports_config = match plugin.get_extension::<AudioPorts>() {
                    Some(audio_ports) => if let Ok(config) = audio_ports.config(&plugin) {
                        config
                    }
                    else {
                        panic!("Error while querying 'audio-ports' IO configuration");
                    }
                    None => {
                        panic!("No 'audio-ports' found");
                    }
                };
            
                // // host.handle_callbacks_once();
            
                let process_config = ProcessConfig {
                    sample_rate: 44_100.0,
                    tempo: 140.0,
                    time_sig_numerator: 4,
                    time_sig_denominator: 4,
                };
            
                let (input_buffers, output_buffers) = audio_ports_config.create_buffers(1024);
                let audio_buffers = if let Ok(buffers) = OutOfPlaceAudioBuffers::new(
                    input_buffers,
                    output_buffers,
                ) {
                    AudioBuffers::OutOfPlace(buffers)
                }
                else {
                    panic!("Couldn't allocate audio buffers.");
                };
            
                let _ = plugin.activate(44100.0, 1, 1024);
                let _ = plugin.start_processing();
            
                let process_data = ProcessData::new(audio_buffers, process_config);
            
                return (plugin, process_data, host_receiver);
            }
            else {
                panic!("No clap plugin id provided.");
            }
        }
        Err(error) => panic!("Could not log clap_plugin_loaders: {}", error),
    }
}

pub fn scan_for_audio_plugins(vst_path: String, clap_path: String) -> (HashMap<String, String>, HashMap<String, String>) {
    let mut instrument_audio_plugins: HashMap<String, String> = HashMap::new();
    let mut effect_audio_plugins: HashMap<String, String> = HashMap::new();

    if let Some(vst24_checker) = find_executable_in_path(VST24_CHECKER_EXECUTABLE_NAME) {
        if let Some(vst24_checker) = vst24_checker.to_str() {
            scan_for_audio_plugins_of_type(vst24_checker, vst_path.as_str(), &mut instrument_audio_plugins, &mut effect_audio_plugins);
        }
    }
    
    if let Some(clap_checker) = find_executable_in_path(CLAP_CHECKER_EXECUTABLE_NAME) {
        if let Some(clap_checker) = clap_checker.to_str() {
            scan_for_audio_plugins_of_type(clap_checker, clap_path.as_str(), &mut instrument_audio_plugins, &mut effect_audio_plugins);
        }
    }

    (instrument_audio_plugins, effect_audio_plugins)
}

fn scan_for_audio_plugins_of_type(
    audio_plugin_checker: &str, 
    shared_library_path: &str, 
    instrument_audio_plugins: &mut HashMap<String, String>, 
    effect_audio_plugins: &mut HashMap<String, String>
) {
    if let Ok(read_dir) = std::fs::read_dir(shared_library_path) {
        for dir_entry in read_dir {
            if let Ok(entry) = dir_entry {
                if let Ok(file_type) = entry.file_type() {
                    if file_type.is_file() || file_type.is_symlink() {
                        if let Some(path) = entry.path().to_str() {
                            if path.ends_with(".so") || path.ends_with(".clap") {
                                println!("Found shared library: {}", path);
                                let plugin_path = path.to_string();

                                if let Ok(output) = Command::new(audio_plugin_checker)
                                    .arg(format!("\"{}\"", plugin_path.as_str()))
                                    .output() {
                                    if let Ok(command_output) = std::str::from_utf8(&output.stdout) {
                                        println!("{}", command_output);
                                        if command_output.contains("##########") {
                                            for line in command_output.lines() {
                                                if line.starts_with("##########") {
                                                    let adjusted_line = line.replace("##########", "");
                                                    let elements = adjusted_line.split(':').collect::<Vec<&str>>();
                                                    let plugin_name = match elements.first() {
                                                        Some(plugin_name) => *plugin_name,
                                                        None => "unknown",
                                                    };
                                                    let library_path = match elements.get(1) {
                                                        Some(path) => *path,
                                                        None => "",
                                                    };
                                                    let plugin_id = match elements.get(2) {
                                                        Some(id) => *id,
                                                        None => "",
                                                    };
                                                    let plugin_category = match elements.get(3) {
                                                        Some(category) => (*category).parse::<isize>().unwrap_or(0),
                                                        None => 0,
                                                    };
                                                    let plugin_type = match elements.get(4) {
                                                        Some(plugin_type) => *plugin_type,
                                                        None => "unknown",
                                                    };


                                                    if !plugin_name.is_empty() &&
                                                        !library_path.is_empty() {
                                                        let id = format!("{}:{}:{}", library_path, plugin_id, plugin_type);
                                                        let plugin_name = format!("{} ({})", plugin_name, plugin_type);

                                                        match plugin_category {
                                                            // unknown
                                                            0 => {
                                                                effect_audio_plugins.insert(id, plugin_name);
                                                            }
                                                            // effect
                                                            1 => {
                                                                effect_audio_plugins.insert(id, plugin_name);
                                                            }
                                                            // instrument
                                                            2 => {
                                                                instrument_audio_plugins.insert(id, plugin_name);
                                                            }
                                                            // generator
                                                            11 => {
                                                                instrument_audio_plugins.insert(id, plugin_name);
                                                            }
                                                            _ => {}
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    else {
                                        println!("Couldn't process command output.");
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
