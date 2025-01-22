use std::ffi::CString;
use std::os::raw::c_char;
use std::sync::mpsc::Sender;
use cxx::UniquePtr;
use log::debug;
use crate::event::AudioPluginHostOutwardEvent;

#[cxx::bridge(namespace = "org::hremeviuc")]
pub mod ffi {
    enum EventType {
        NoteOn,
        NoteOff,
        NoteExpression,
        Controller,
        KeyPressureAfterTouch,
        PitchBend,
        Parameter
    }

    extern "Rust" {
        type Vst3Host;
    }

    unsafe extern "C++" {
        include!("riff-daw/include/vst3cxxbridge.h");

        fn createPlugin(
            vst3_plugin_path: String,
            riff_daw_plugin_uuid: String,
            vst3_plugin_uid: String,
            sampleRate: f64,
            blockSize: i32,
            vst3Host: Box<Vst3Host>,
            sendParameterChange: fn(context: Box<Vst3Host>, param_id: i32, param_value: f32) -> Box<Vst3Host>
        ) -> bool;
        fn showPluginEditor(
            riff_daw_plugin_uuid: String,
            xid: u32,
            vst3Host: Box<Vst3Host>,
            sendPluginWindowResize: fn(context: Box<Vst3Host>, new_window_width: i32, new_window_height: i32) -> Box<Vst3Host>,
        ) -> bool;
        fn vst3_plugin_get_window_height(riff_daw_plugin_uuid: String) -> u32;
        fn vst3_plugin_get_window_width(riff_daw_plugin_uuid: String) -> u32;
        fn vst3_plugin_get_window_refresh(riff_daw_plugin_uuid: String);
        fn vst3_plugin_process(
            riff_daw_plugin_uuid: String,
            channel1InputBuffer: &[f32],
            channel2InputBuffer: &[f32],
            channel1OutputBuffer: &mut [f32],
            channel2OutputBuffer: &mut [f32]) -> bool;
        fn addEvent(riff_daw_plugin_uuid: String, eventType: EventType, blockPosition: i32, data1: u32, data2: u32, data3: i32, data4: f64) -> bool;
        fn getVstPluginName(riff_daw_plugin_uuid: String) -> String;

        fn setProcessing(riff_daw_plugin_uuid: String, processing: bool) -> bool;
        fn setActive(riff_daw_plugin_uuid: String, active: bool) -> bool;

        fn vst3_plugin_get_preset(riff_daw_plugin_uuid: String, preset_buffer: &mut [u8], maxSize: u32) -> i32;
        fn vst3_plugin_set_preset(riff_daw_plugin_uuid: String, preset_buffer: &mut [u8]);
        fn vst3_plugin_get_parameter_count(riff_daw_plugin_uuid: String) -> i32;
        fn vst3_plugin_get_parameter_info(
            riff_daw_plugin_uuid: String,
            index: i32,
            id: &mut u32,
            title: &mut [u16],
            short_title: &mut [u16],
            units: &mut [u16],
            step_count: &mut i32,
            default_normalised_value: &mut f64,
            unit_id: &mut i32,
            flags: &mut i32,
        );
        fn vst3_plugin_remove(riff_daw_plugin_uuid: String);
    }
}

pub struct Vst3Host (
        pub String, // track_uuid
        pub String, // plugin_uuid
        pub bool, // instrument
        pub Sender<AudioPluginHostOutwardEvent>, // sender
    );


