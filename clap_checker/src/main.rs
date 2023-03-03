use std::{ffi::{CString, CStr, c_char}, path::PathBuf, ptr::NonNull, os::raw::c_void};

use clap_sys::{ entry::clap_plugin_entry, plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID}, plugin::{clap_plugin, clap_plugin_descriptor}, host::clap_host, version::clap_version, ext::{audio_ports::{CLAP_EXT_AUDIO_PORTS, clap_host_audio_ports, clap_plugin_audio_ports, clap_audio_port_info, CLAP_AUDIO_PORT_IS_MAIN, CLAP_AUDIO_PORT_SUPPORTS_64BITS, CLAP_AUDIO_PORT_PREFERS_64BITS, CLAP_AUDIO_PORT_REQUIRES_COMMON_SAMPLE_SIZE}, note_ports::{CLAP_EXT_NOTE_PORTS, clap_note_dialect, CLAP_NOTE_DIALECT_CLAP, CLAP_NOTE_DIALECT_MIDI, CLAP_NOTE_DIALECT_MIDI_MPE, clap_host_note_ports, clap_plugin_note_ports, clap_note_port_info, CLAP_NOTE_DIALECT_MIDI2}, params::{clap_param_clear_flags, clap_param_rescan_flags, clap_host_params}, state::{clap_host_state}, thread_check::{clap_host_thread_check}, gui::{clap_plugin_gui, CLAP_EXT_GUI, CLAP_WINDOW_API_X11, CLAP_WINDOW_API_WAYLAND}}, id::clap_id, events::{CLAP_EVENT_TRANSPORT, CLAP_CORE_EVENT_SPACE_ID, CLAP_TRANSPORT_HAS_TEMPO, CLAP_TRANSPORT_HAS_BEATS_TIMELINE, CLAP_TRANSPORT_HAS_SECONDS_TIMELINE, CLAP_TRANSPORT_HAS_TIME_SIGNATURE, CLAP_TRANSPORT_IS_PLAYING, clap_event_note, clap_event_header, CLAP_EVENT_MIDI, clap_event_midi, CLAP_EVENT_IS_LIVE}, plugin_features::{CLAP_PLUGIN_FEATURE_INSTRUMENT, CLAP_PLUGIN_FEATURE_AUDIO_EFFECT, CLAP_PLUGIN_FEATURE_NOTE_EFFECT}};

use libloading::Library;


fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() == 2 {
        if let Some(audio_plugin_path) = args.get(1) {
            if audio_plugin_path.contains(',') {
                for plugin in audio_plugin_path.replace('\"', "").as_str().split(',').collect::<Vec<&str>>().iter() {
                    check_audio_plugin(plugin);
                }
            }
            else {
                check_audio_plugin(audio_plugin_path.replace('\"', "").as_str());
            }
        }
    }
    else {
        println!("Something wrong with command line argument(s) given: {:?}", args);
    }
}

fn check_audio_plugin(audio_plugin_path: &str) {
    unsafe {
        let path_buf = PathBuf::from(audio_plugin_path);
        let filename_c_str = CString::new(path_buf.as_os_str().to_str().unwrap());

        match Library::new(audio_plugin_path) {
            Ok(lib) => {
                match lib.get::<*const clap_plugin_entry>(b"clap_entry\0") {
                    Ok(symbol) => {
                        let plugin_entry = NonNull::new(*symbol as *mut clap_plugin_entry).unwrap();
                        let plugin_entry_ref = plugin_entry.as_ref();
                        println!("Plugin version: {}.{}.{}", plugin_entry_ref.clap_version.major, plugin_entry_ref.clap_version.minor, plugin_entry_ref.clap_version.revision);

                        if let Some(plugin_init) = plugin_entry_ref.init {
                            if let Ok(c_filename) = filename_c_str {
                                if plugin_init(c_filename.as_ptr()) {
                                    println!("Successfully initialised the clap plugin.");

                                    if let Some(plugin_factory) = plugin_entry_ref.get_factory {
                                        let plugin_factory_ptr = plugin_factory(CLAP_PLUGIN_FACTORY_ID.as_ptr());
                                        let plugin_factory = NonNull::new(plugin_factory_ptr as *mut clap_plugin_factory).unwrap();
                                        let plugin_factory_ref = plugin_factory.as_ref();

                                        let plugin_count = if let Some(plugin_count) = plugin_factory_ref.get_plugin_count {
                                            let plugin_count = plugin_count(plugin_factory.as_ptr());

                                            println!("Plugin count: {}", plugin_count);
                                            plugin_count
                                        }
                                        else {
                                            0
                                        };

                                        for plugin_index in 0..plugin_count {
                                            let (plugin_id, _plugin_name, _plugin_type) = if let Some(plugin_descriptor) = plugin_factory_ref.get_plugin_descriptor {
                                                let plugin_descriptor_ptr = plugin_descriptor(plugin_factory.as_ptr(), plugin_index);
                                                let plugin_descriptor = NonNull::new(plugin_descriptor_ptr as *mut clap_plugin_descriptor).unwrap();
                                                let plugin_descriptor_ref = plugin_descriptor.as_ref();
        
        
                                                if let Ok(id) = CStr::from_ptr(plugin_descriptor_ref.id).to_str() {
                                                    println!("Plugin descriptor - id: {}", id);
                                                }
                                                let plugin_name = if let Ok(name) = CStr::from_ptr(plugin_descriptor_ref.name).to_str() {
                                                    println!("Plugin descriptor - name: {}", name);
                                                    name.to_string()
                                                }
                                                else {
                                                    "unknown".to_string()
                                                };
                                                if let Ok(description) = CStr::from_ptr(plugin_descriptor_ref.description).to_str() {
                                                    println!("Plugin descriptor - description: {}", description);
                                                }
                                                if !plugin_descriptor_ref.support_url.is_null() {
                                                    if let Ok(support_url) = CStr::from_ptr(plugin_descriptor_ref.support_url).to_str() {
                                                        println!("Plugin descriptor - support_url: {}", support_url);
                                                    }
                                                }
                                                if let Ok(manual_url) = CStr::from_ptr(plugin_descriptor_ref.manual_url).to_str() {
                                                    println!("Plugin descriptor - manual_url: {}", manual_url);
                                                }
                                                if let Ok(url) = CStr::from_ptr(plugin_descriptor_ref.url).to_str() {
                                                    println!("Plugin descriptor - url: {}", url);
                                                }
                                                if let Ok(vendor) = CStr::from_ptr(plugin_descriptor_ref.vendor).to_str() {
                                                    println!("Plugin descriptor - vendor: {}", vendor);
                                                }
                                                if let Ok(version) = CStr::from_ptr(plugin_descriptor_ref.version).to_str() {
                                                    println!("Plugin descriptor - version: {}", version);
                                                }
        
                                                println!("Plugin descriptor - features:");
                                                let mut ptr = plugin_descriptor_ref.features;
                                                let mut plugin_type = 0;
                                                while !(ptr).is_null() && !(*ptr).is_null() {
                                                    let feature_cstr = CStr::from_ptr(*ptr);

                                                    if feature_cstr == CLAP_PLUGIN_FEATURE_INSTRUMENT {
                                                        plugin_type = 2;
                                                    }
                                                    if feature_cstr == CLAP_PLUGIN_FEATURE_AUDIO_EFFECT || feature_cstr.to_str().unwrap() == "audio_effect" {
                                                        plugin_type = 1;
                                                    }
                                                    if feature_cstr == CLAP_PLUGIN_FEATURE_NOTE_EFFECT {
                                                        plugin_type = 11;
                                                    }

                                                    if let Ok(feature) = feature_cstr.to_str() {
                                                        println!("\t{}", feature);
                                                    }
                                                    ptr = ptr.offset(1);
                                                }
        
                                                println!("##########{}:{}:{}:{}:CLAP", plugin_name, audio_plugin_path, CStr::from_ptr(plugin_descriptor_ref.id).to_str().unwrap(), plugin_type);

                                                (plugin_descriptor_ref.id, plugin_name, plugin_type)
                                            }
                                            else {
                                                return;
                                            };
        
                                            if let Some(create_plugin) = plugin_factory_ref.create_plugin {
        
                                                unsafe extern "C" fn ext_audio_ports_is_rescan_flag_supported(
                                                    _host: *const clap_host,
                                                    _flag: u32,
                                                ) -> bool {
                                                    true
                                                }
        
                                                unsafe extern "C" fn ext_audio_ports_rescan(_host: *const clap_host, _flags: u32) {
                                                }
        
                                                unsafe extern "C" fn ext_note_ports_supported_dialects(
                                                    _host: *const clap_host,
                                                ) -> clap_note_dialect {
                                                    CLAP_NOTE_DIALECT_CLAP | CLAP_NOTE_DIALECT_MIDI | CLAP_NOTE_DIALECT_MIDI_MPE
                                                }
        
                                                unsafe extern "C" fn ext_note_ports_rescan(_host: *const clap_host, _flags: u32) {
                                                }
        
                                                unsafe extern "C" fn ext_params_rescan(
                                                    _host: *const clap_host,
                                                    _flags: clap_param_rescan_flags,
                                                ) {
                                                }
        
                                                unsafe extern "C" fn ext_params_clear(
                                                    _host: *const clap_host,
                                                    _param_id: clap_id,
                                                    _flags: clap_param_clear_flags,
                                                ) {
                                                }
        
                                                unsafe extern "C" fn ext_params_request_flush(_host: *const clap_host) {
                                                }
        
                                                unsafe extern "C" fn ext_state_mark_dirty(_host: *const clap_host) {
                                                }
        
                                                unsafe extern "C" fn ext_thread_check_is_main_thread(_host: *const clap_host) -> bool {
                                                    true
                                                }
        
                                                unsafe extern "C" fn ext_thread_check_is_audio_thread(_host: *const clap_host) -> bool {
                                                    true
                                                }
        
                                                let _clap_host_audio_ports_instance = clap_host_audio_ports {
                                                    is_rescan_flag_supported: Some(ext_audio_ports_is_rescan_flag_supported),
                                                    rescan: Some(ext_audio_ports_rescan),
                                                };
                                                let _clap_host_note_ports_instance = clap_host_note_ports {
                                                    supported_dialects: Some(ext_note_ports_supported_dialects),
                                                    rescan: Some(ext_note_ports_rescan),
                                                };
                                                let _clap_host_params_instance = clap_host_params {
                                                    rescan: Some(ext_params_rescan),
                                                    clear: Some(ext_params_clear),
                                                    request_flush: Some(ext_params_request_flush),
                                                };
                                                let _clap_host_state_instance = clap_host_state {
                                                    mark_dirty: Some(ext_state_mark_dirty),
                                                };
                                                let _clap_host_thread_check_instance = clap_host_thread_check {
                                                    is_main_thread: Some(ext_thread_check_is_main_thread),
                                                    is_audio_thread: Some(ext_thread_check_is_audio_thread),
                                                };
        
        
                                                unsafe extern "C" fn get_extension(
                                                    _host: *const clap_host,
                                                    _extension_id: *const c_char,
                                                ) -> *const c_void {
                                                    // let extension_id_cstr = CStr::from_ptr(extension_id);
                                                    // if extension_id_cstr == CLAP_EXT_AUDIO_PORTS {
                                                    //     &clap_host_audio_ports_instance as *const _ as *const c_void
                                                    // } else if extension_id_cstr == CLAP_EXT_NOTE_PORTS {
                                                    //     &clap_host_note_ports_instance as *const _ as *const c_void
                                                    // } else if extension_id_cstr == CLAP_EXT_PARAMS {
                                                    //     &clap_host_params_instance as *const _ as *const c_void
                                                    // } else if extension_id_cstr == CLAP_EXT_STATE {
                                                    //     &clap_host_state_instance as *const _ as *const c_void
                                                    // } else if extension_id_cstr == CLAP_EXT_THREAD_CHECK {
                                                    //     &clap_host_thread_check_instance as *const _ as *const c_void
                                                    // } else {
                                                        std::ptr::null()
                                                    // }
                                                }
        
                                                unsafe extern "C" fn request_restart(_host: *const clap_host) {
                                                }
        
                                                unsafe extern "C" fn request_process(_host: *const clap_host) {
                                                }
        
                                                unsafe extern "C" fn request_callback(_host: *const clap_host) {
                                                }
        
                                                unsafe extern "C" fn in_events_size(events: *const clap_sys::events::clap_input_events) -> u32 {
                                                    if events.is_null() || (*events).ctx.is_null() {
                                                        0
                                                    }
                                                    else {
                                                        let list = &*((*events).ctx as *mut Vec<clap_event_note>);
                                                    
                                                        if list.len() > 0 {
                                                            println!("list: {}", list.len());
                                                            list.len() as u32
                                                        }
                                                        else {
                                                            0
                                                        }
                                                    }
                                                }
        
                                                unsafe extern "C" fn in_events_get(events: *const clap_sys::events::clap_input_events, index: u32) -> *const clap_sys::events::clap_event_header {
                                                    let list = &*((*events).ctx as *mut Vec<clap_event_note>);
        
                                                    match list.get(index as usize) {
                                                        Some(event) => {
                                                            println!("in_events_get called and an event was returned.");
                                                            &event.header
                                                        },
                                                        None => std::ptr::null_mut(),
                                                    }
                                                }
        
                                                unsafe extern "C" fn out_events_try_push(_events: *const clap_sys::events::clap_output_events, _header: *const clap_sys::events::clap_event_header) -> bool {
                                                    true
                                                }
        
                                                let my_host = clap_host {
                                                    clap_version: clap_version {
                                                        major: 1,
                                                        minor: 0,
                                                        revision: 3,
                                                    },
                                                    host_data: std::ptr::null_mut(),
                                                    name: b"FreedomDAW\0".as_ptr() as *const c_char,
                                                    vendor: b"HremTech\0".as_ptr() as *const c_char,
                                                    url: b"http://pwcu.com.au\0".as_ptr() as *const c_char,
                                                    version: b"0.1.0\0".as_ptr() as *const c_char,
                                                    get_extension: Some(get_extension),
                                                    request_restart: Some(request_restart),
                                                    request_process: Some(request_process),
                                                    request_callback: Some(request_callback),
                                                };
                                                let plugin_instance_ptr = create_plugin(plugin_factory.as_ptr(), &my_host, plugin_id);
                                                let plugin_instance = NonNull::new(plugin_instance_ptr as *mut clap_plugin).unwrap();
                                                let plugin_instance_ref = plugin_instance.as_ref();
        
                                                if let Some(plugin_init) = plugin_instance_ref.init {
                                                    if plugin_init(plugin_instance_ptr) {
                                                        println!("Successfully initialised the plugin.");
                                                    }
                                                }
        
                                                if let Some(plugin_activate) = plugin_instance_ref.activate {
                                                    if plugin_activate(plugin_instance_ptr, 44100.0, 1024, 1024) {
                                                        println!("Successfully activated the plugin.");
                                                    }
                                                }
        
                                                if let Some(plugin_start_processing) = plugin_instance_ref.start_processing {
                                                    if plugin_start_processing(plugin_instance_ptr) {
                                                        println!("Successfully started the plugin processing.");
                                                    }
                                                }
        
                                                if let Some(plugin_get_extension) = plugin_instance_ref.get_extension {
                                                    let note_ports_c_void = plugin_get_extension(plugin_instance_ptr, CLAP_EXT_NOTE_PORTS.as_ptr());
                                                    if note_ports_c_void != std::ptr::null() {
                                                        println!("Successfully queried the plugin for note ports.");
        
                                                        let note_ports = note_ports_c_void as *mut clap_plugin_note_ports;
                                                        if let Some(number_of_note_ports) = (*note_ports).count {
                                                            let number_of_input_ports = number_of_note_ports(plugin_instance_ptr, true);
                                                            let number_of_output_ports = number_of_note_ports(plugin_instance_ptr, false);
        
                                                            println!("Number of input note ports: {}", number_of_input_ports);
                                                            println!("Number of output note ports: {}", number_of_output_ports);
        
                                                            if let Some(get_note_port) = (*note_ports).get {
                                                                let mut note_port_info = clap_note_port_info {
                                                                    id: 0,
                                                                    supported_dialects: 0,
                                                                    preferred_dialect: 0,
                                                                    name: [0 as c_char; 256],
                                                                };
                                                                if get_note_port(plugin_instance_ptr, 0, true, &mut note_port_info) {
                                                                    println!("First input note port id: {}", note_port_info.id);
                                                                    println!("Prefered dialect: CLAP_NOTE_DIALECT_MIDI: {}", note_port_info.preferred_dialect & CLAP_NOTE_DIALECT_MIDI == CLAP_NOTE_DIALECT_MIDI);
                                                                    println!("Prefered dialect: CLAP_NOTE_DIALECT_MIDI2: {}", note_port_info.preferred_dialect & CLAP_NOTE_DIALECT_MIDI2 == CLAP_NOTE_DIALECT_MIDI2);
                                                                    println!("Prefered dialect: CLAP_NOTE_DIALECT_CLAP: {}", note_port_info.preferred_dialect & CLAP_NOTE_DIALECT_CLAP == CLAP_NOTE_DIALECT_CLAP);
                                                                    println!("Prefered dialect: CLAP_NOTE_DIALECT_MIDI_MPE: {}", note_port_info.preferred_dialect & CLAP_NOTE_DIALECT_MIDI_MPE == CLAP_NOTE_DIALECT_MIDI_MPE);
                                                                
                                                                    println!("Supported dialect: CLAP_NOTE_DIALECT_MIDI: {}", note_port_info.supported_dialects & CLAP_NOTE_DIALECT_MIDI == CLAP_NOTE_DIALECT_MIDI);
                                                                    println!("Supported dialect: CLAP_NOTE_DIALECT_MIDI2: {}", note_port_info.supported_dialects & CLAP_NOTE_DIALECT_MIDI2 == CLAP_NOTE_DIALECT_MIDI2);
                                                                    println!("Supported dialect: CLAP_NOTE_DIALECT_CLAP: {}", note_port_info.supported_dialects & CLAP_NOTE_DIALECT_CLAP == CLAP_NOTE_DIALECT_CLAP);
                                                                    println!("Supported dialect: CLAP_NOTE_DIALECT_MIDI_MPE: {}", note_port_info.supported_dialects & CLAP_NOTE_DIALECT_MIDI_MPE == CLAP_NOTE_DIALECT_MIDI_MPE);
        
                                                                    let port_name = CStr::from_ptr(note_port_info.name.as_ptr());
        
                                                                    if let Ok(port_name) = port_name.to_str() {
                                                                        println!("Note port name: {}", port_name);
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
        
                                                    let audio_ports_c_void = plugin_get_extension(plugin_instance_ptr, CLAP_EXT_AUDIO_PORTS.as_ptr());
                                                    if audio_ports_c_void != std::ptr::null() {
                                                        println!("Successfully queried the plugin for audio ports.");
        
                                                        let audio_ports = audio_ports_c_void as *mut clap_plugin_audio_ports;
                                                        if let Some(number_of_audio_ports) = (*audio_ports).count {
                                                            let number_of_audio_input_ports = number_of_audio_ports(plugin_instance_ptr, true);
                                                            let number_of_audio_output_ports = number_of_audio_ports(plugin_instance_ptr, false);
        
                                                            println!("Number of input audio ports: {}", number_of_audio_input_ports);
                                                            println!("Number of output audio ports: {}", number_of_audio_output_ports);
        
                                                            if let Some(get_audio_port) = (*audio_ports).get {
                                                                let mut audio_port_info = clap_audio_port_info {
                                                                    id: 0,
                                                                    name: [0 as c_char; 256],
                                                                    flags: 0,
                                                                    channel_count: 0,
                                                                    port_type: std::ptr::null(),
                                                                    in_place_pair: 0,
                                                                };
                                                                if get_audio_port(plugin_instance_ptr, 0, false, &mut audio_port_info) {
                                                                    println!("First output audio port id: {}", audio_port_info.id);
                                                                    println!("Prefered dialect: CLAP_AUDIO_PORT_IS_MAIN: {}", audio_port_info.flags & CLAP_AUDIO_PORT_IS_MAIN == CLAP_AUDIO_PORT_IS_MAIN);
                                                                    println!("Prefered dialect: CLAP_AUDIO_PORT_SUPPORTS_64BITS: {}", audio_port_info.flags & CLAP_AUDIO_PORT_SUPPORTS_64BITS == CLAP_AUDIO_PORT_SUPPORTS_64BITS);
                                                                    println!("Prefered dialect: CLAP_AUDIO_PORT_PREFERS_64BITS: {}", audio_port_info.flags & CLAP_AUDIO_PORT_PREFERS_64BITS == CLAP_AUDIO_PORT_PREFERS_64BITS);
                                                                    println!("Prefered dialect: CLAP_AUDIO_PORT_REQUIRES_COMMON_SAMPLE_SIZE: {}", audio_port_info.flags & CLAP_AUDIO_PORT_REQUIRES_COMMON_SAMPLE_SIZE == CLAP_AUDIO_PORT_REQUIRES_COMMON_SAMPLE_SIZE);
        
                                                                    let port_type = CStr::from_ptr(audio_port_info.port_type);
        
                                                                    if let Ok(port_type) = port_type.to_str() {
                                                                        println!("Audio port type: {}", port_type);
                                                                    }
        
                                                                    let port_name = CStr::from_ptr(audio_port_info.name.as_ptr());
        
                                                                    if let Ok(port_name) = port_name.to_str() {
                                                                        println!("Audio port name: {}", port_name);
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                
                                                    let gui_extension_c_void = plugin_get_extension(plugin_instance_ptr, CLAP_EXT_GUI.as_ptr());
                                                    if gui_extension_c_void != std::ptr::null() {
                                                        println!("Successfully queried the plugin for gui support.");
        
                                                        let gui = gui_extension_c_void as *mut clap_plugin_gui;
                                                        if let Some(gui_is_api_supported) = (*gui).is_api_supported {
                                                            if gui_is_api_supported(plugin_instance_ptr, CLAP_WINDOW_API_X11.as_ptr(), false) {
                                                                println!("X11 gui support: true");
                                                            }
                                                            else {
                                                                println!("X11 gui support: false");
                                                            }
                                                            if gui_is_api_supported(plugin_instance_ptr, CLAP_WINDOW_API_WAYLAND.as_ptr(), false) {
                                                                println!("Wayland gui support: true");
                                                            }
                                                            else {
                                                                println!("Wayland gui support: false");
                                                            }
                                                        }
                                                    }
                                                }
        
                                                let mut events = Vec::<clap_sys::events::clap_event_midi>::new();
        
                                                let in_events = clap_sys::events::clap_input_events {
                                                    ctx: &mut events as *mut _ as *mut c_void,
                                                    size: Some(in_events_size),
                                                    get: Some(in_events_get),
                                                };
        
                                                let out_events = clap_sys::events::clap_output_events {
                                                    ctx: std::ptr::null_mut(),
                                                    try_push: Some(out_events_try_push),
                                                };
        
                                                let transport = clap_sys::events::clap_event_transport {
                                                    header: clap_sys::events::clap_event_header {
                                                        size: std::mem::size_of::<clap_sys::events::clap_event_transport>() as u32,
                                                        time: 0,
                                                        space_id: CLAP_CORE_EVENT_SPACE_ID,
                                                        type_: CLAP_EVENT_TRANSPORT,
                                                        flags: 0,
                                                    },
                                                    flags: CLAP_TRANSPORT_HAS_TEMPO
                                                    | CLAP_TRANSPORT_HAS_BEATS_TIMELINE
                                                    | CLAP_TRANSPORT_HAS_SECONDS_TIMELINE
                                                    | CLAP_TRANSPORT_HAS_TIME_SIGNATURE
                                                    | CLAP_TRANSPORT_IS_PLAYING,
                                                    song_pos_beats: 0,
                                                    song_pos_seconds: 0,
                                                    tempo: 140.0,
                                                    tempo_inc: 0.0,
                                                    loop_start_beats: 0,
                                                    loop_end_beats: 0,
                                                    loop_start_seconds: 0,
                                                    loop_end_seconds: 0,
                                                    bar_start: 0,
                                                    bar_number: 0,
                                                    tsig_num: 4,
                                                    tsig_denom: 4,
                                                };
        
                                                let channel_1 = vec![0.0f32; 1024];
                                                let mut boxed_slice_channel_1 = channel_1.into_boxed_slice();
                                                let channel_2 = vec![0.0f32; 1024];
                                                let mut boxed_slice_channel_2 = channel_2.into_boxed_slice();
                                                let output_buffer = vec![boxed_slice_channel_1.as_mut_ptr(), boxed_slice_channel_2.as_mut_ptr()];
                                                let mut boxed_slice_output_buffer = output_buffer.into_boxed_slice();
                                                let audio_output_buffer = clap_sys::audio_buffer::clap_audio_buffer {
                                                    channel_count: 2,
                                                    constant_mask: 0,
                                                    latency: 0,
                                                    data64: std::ptr::null_mut(),
                                                    data32: boxed_slice_output_buffer.as_mut_ptr() as *mut _ as *const *const f32,
                                                };
                                                let mut audio_output_buffers = [audio_output_buffer];
                                                std::mem::forget(boxed_slice_channel_1);
                                                std::mem::forget(boxed_slice_channel_2);
                                                std::mem::forget(boxed_slice_output_buffer);
                                                
                                                let mut input_buffer = vec![vec![0.0f32; 1024]; 4];
                                                let audio_input_buffer = clap_sys::audio_buffer::clap_audio_buffer {
                                                    channel_count: 0,
                                                    constant_mask: 0,
                                                    latency: 0,
                                                    data64: std::ptr::null_mut(),
                                                    data32: input_buffer.as_mut_ptr() as *const *const f32,
                                                };
                                                let audio_input_buffers = [audio_input_buffer];
        
                                                let mut clap_process_box = clap_sys::process::clap_process {
                                                    steady_time: 0,
                                                    frames_count: 1024,
                                                    audio_inputs_count: 0,
                                                    audio_outputs_count: 1,
                                                    // audio_inputs: &mut audio_input_buffer,
                                                    // audio_outputs: &mut audio_output_buffer,
                                                    audio_inputs: audio_input_buffers.as_ptr(),
                                                    audio_outputs: audio_output_buffers.as_mut_ptr(),
                                                    in_events: &in_events,
                                                    out_events: &out_events,
                                                    transport: &transport,
                                                };
        
                                                if let Some(plugin_process) = plugin_instance_ref.process {
                                                    let mut non_zero_data_found = false;
        
                                                    for block in 0..1024 {                                                    
                                                        if block == 0 {
                                                            let note_on = clap_event_midi {
                                                                header: clap_event_header {
                                                                    size: std::mem::size_of::<clap_event_midi>() as u32,
                                                                    time: 10,
                                                                    space_id: CLAP_CORE_EVENT_SPACE_ID,
                                                                    type_: CLAP_EVENT_MIDI,
                                                                    flags: CLAP_EVENT_IS_LIVE,
                                                                },
                                                                port_index: 0 as u16,
                                                                data: [144 as u8, 60 as u8, 127 as u8],
                                                            };
                                                            events.push(note_on);
                                                        }
                                                        else if block == 1023 {
                                                            let note_off = clap_event_midi {
                                                                header: clap_event_header {
                                                                    size: std::mem::size_of::<clap_event_midi>() as u32,
                                                                    time: 1023,
                                                                    space_id: CLAP_CORE_EVENT_SPACE_ID,
                                                                    type_: CLAP_EVENT_MIDI,
                                                                    flags: CLAP_EVENT_IS_LIVE,
                                                                },
                                                                port_index: 0 as u16,
                                                                data: [128 as u8, 60 as u8, 127 as u8],
                                                            };
                                                            events.push(note_off);
                                                        }
        
                                                        plugin_process(plugin_instance_ptr, &clap_process_box);
        
                                                        // check that there is non zero output sample frame data
                                                        let buffer = audio_output_buffer.data32 as *mut *mut f32;
                                                        let channels = std::slice::from_raw_parts_mut(buffer, 2);
                                                        let channel1 = std::slice::from_raw_parts(channels[0], 1024);
                                                        let channel2 = std::slice::from_raw_parts(channels[1], 1024);
                                                        'found_some: for index in 0..1024 {
                                                            let channel1_frame = channel1[index];
                                                            let channel2_frame = channel2[index];
                                                            // println!("Block: {}, Frame index: {}, left: {}, right: {}", block, index, channel1_frame, channel2_frame);
                                                            if channel1_frame > 0.0 || channel1_frame < 0.0 || channel2_frame > 0.0 || channel2_frame < 0.0 {
                                                                non_zero_data_found = true;
                                                                break 'found_some;
                                                            }
                                                        }
        
                                                        events.clear();
                                                        clap_process_box.steady_time += 1024;
                                                        clap_process_box.in_events = &in_events;
                                                    }
        
                                                    println!("Successfully processed.");
                                                    println!("Non zero frame data found: {}", non_zero_data_found);
                                                }
        
                                                if let Some(plugin_stop_processing) = plugin_instance_ref.stop_processing {
                                                    plugin_stop_processing(plugin_instance_ptr);
                                                    println!("Successfully stopped the plugin from processing.");
                                                }
        
                                                if let Some(deactivate) = plugin_instance_ref.deactivate {
                                                    deactivate(plugin_instance_ptr);
                                                    println!("Successfully deactivated the plugin.");
                                                }
        
                                                if let Some(destroy) = plugin_instance_ref.destroy {
                                                    destroy(plugin_instance_ptr);
                                                    println!("Successfully destroyed the plugin.");
                                                }
                                            }
                                        }
                                    }
                                }
                                else {
                                    println!("Successfully initialised the clap plugin.");
                                }
                            }
                        }
                    },
                    Err(_) => println!("Failed to load clap entry from plugin dynamic library."),
                }
            },
            Err(_) => println!("Failed to load clap plugin dynamic library."),
        }
    }
}
