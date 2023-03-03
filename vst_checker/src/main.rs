use std::ffi::{c_void};
use std::ops::{Index, IndexMut};
use std::slice;


use num_traits::Float;

use vst::{AEffect, effect_flags, effect_opcodes, Event, Events, host_opcodes, HostCallbackProc, MIDI_TYPE, MidiEvent, plug_category, TimeInfo};

use crate::vst::{time_info_flags, transport};

mod vst;

const NUMBER_OF_FRAMES: usize = 1024;
static mut PLUG_ID: isize = 0;


/// `AudioBuffer` contains references to the audio buffers for all input and output channels.
///
/// To create an `AudioBuffer` in a host, use a [`HostBuffer`](../host/struct.HostBuffer.html).
pub struct AudioBuffer<'a, T: 'a + Float> {
    inputs: &'a [*const T],
    outputs: &'a mut [*mut T],
    samples: usize,
}

impl<'a, T: 'a + Float> AudioBuffer<'a, T> {
    /// Create an `AudioBuffer` from raw pointers.
    /// Only really useful for interacting with the VST API.
    #[inline]
    pub unsafe fn from_raw(
        input_count: usize,
        output_count: usize,
        inputs_raw: *const *const T,
        outputs_raw: *mut *mut T,
        samples: usize,
    ) -> Self {
        Self {
            inputs: slice::from_raw_parts(inputs_raw, input_count),
            outputs: slice::from_raw_parts_mut(outputs_raw, output_count),
            samples,
        }
    }

    /// The number of input channels that this buffer was created for
    #[inline]
    pub fn input_count(&self) -> usize {
        self.inputs.len()
    }

    /// The number of output channels that this buffer was created for
    #[inline]
    pub fn output_count(&self) -> usize {
        self.outputs.len()
    }

    /// The number of samples in this buffer (same for all channels)
    #[inline]
    pub fn samples(&self) -> usize {
        self.samples
    }

    /// The raw inputs to pass to processReplacing
    #[inline]
    pub(crate) fn raw_inputs(&self) -> &[*const T] {
        self.inputs
    }

    /// The raw outputs to pass to processReplacing
    #[inline]
    pub(crate) fn raw_outputs(&mut self) -> &mut [*mut T] {
        self.outputs
    }

    /// Split this buffer into separate inputs and outputs.
    #[inline]
    pub fn split<'b>(&'b mut self) -> (Inputs<'b, T>, Outputs<'b, T>)
    where
        'a: 'b,
    {
        (
            Inputs {
                bufs: self.inputs,
                samples: self.samples,
            },
            Outputs {
                bufs: self.outputs,
                samples: self.samples,
            },
        )
    }

    /// Create an iterator over pairs of input buffers and output buffers.
    #[inline]
    pub fn zip<'b>(&'b mut self) -> AudioBufferIterator<'a, 'b, T> {
        AudioBufferIterator {
            audio_buffer: self,
            index: 0,
        }
    }
}

/// Iterator over pairs of buffers of input channels and output channels.
pub struct AudioBufferIterator<'a, 'b, T>
where
    T: 'a + Float,
    'a: 'b,
{
    audio_buffer: &'b mut AudioBuffer<'a, T>,
    index: usize,
}

impl<'a, 'b, T> Iterator for AudioBufferIterator<'a, 'b, T>
where
    T: 'b + Float,
{
    type Item = (&'b [T], &'b mut [T]);

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.audio_buffer.inputs.len() && self.index < self.audio_buffer.outputs.len() {
            let input =
                unsafe { slice::from_raw_parts(self.audio_buffer.inputs[self.index], self.audio_buffer.samples) };
            let output =
                unsafe { slice::from_raw_parts_mut(self.audio_buffer.outputs[self.index], self.audio_buffer.samples) };
            let val = (input, output);
            self.index += 1;
            Some(val)
        } else {
            None
        }
    }
}

/// Wrapper type to access the buffers for the input channels of an `AudioBuffer` in a safe way.
/// Behaves like a slice.
#[derive(Copy, Clone)]
pub struct Inputs<'a, T: 'a> {
    bufs: &'a [*const T],
    samples: usize,
}

impl<'a, T> Inputs<'a, T> {
    /// Number of channels
    pub fn len(&self) -> usize {
        self.bufs.len()
    }

    /// Returns true if the buffer is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Access channel at the given index
    pub fn get(&self, i: usize) -> &'a [T] {
        unsafe { slice::from_raw_parts(self.bufs[i], self.samples) }
    }

    /// Split borrowing at the given index, like for slices
    pub fn split_at(&self, i: usize) -> (Inputs<'a, T>, Inputs<'a, T>) {
        let (l, r) = self.bufs.split_at(i);
        (
            Inputs {
                bufs: l,
                samples: self.samples,
            },
            Inputs {
                bufs: r,
                samples: self.samples,
            },
        )
    }
}

impl<'a, T> Index<usize> for Inputs<'a, T> {
    type Output = [T];

    fn index(&self, i: usize) -> &Self::Output {
        self.get(i)
    }
}

/// Iterator over buffers for input channels of an `AudioBuffer`.
pub struct InputIterator<'a, T: 'a> {
    data: Inputs<'a, T>,
    i: usize,
}

impl<'a, T> Iterator for InputIterator<'a, T> {
    type Item = &'a [T];

    fn next(&mut self) -> Option<Self::Item> {
        if self.i < self.data.len() {
            let val = self.data.get(self.i);
            self.i += 1;
            Some(val)
        } else {
            None
        }
    }
}

impl<'a, T: Sized> IntoIterator for Inputs<'a, T> {
    type Item = &'a [T];
    type IntoIter = InputIterator<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        InputIterator { data: self, i: 0 }
    }
}

/// Wrapper type to access the buffers for the output channels of an `AudioBuffer` in a safe way.
/// Behaves like a slice.
pub struct Outputs<'a, T: 'a> {
    bufs: &'a [*mut T],
    samples: usize,
}

impl<'a, T> Outputs<'a, T> {
    /// Number of channels
    pub fn len(&self) -> usize {
        self.bufs.len()
    }

    /// Returns true if the buffer is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Access channel at the given index
    pub fn get(&self, i: usize) -> &'a [T] {
        unsafe { slice::from_raw_parts(self.bufs[i], self.samples) }
    }

    /// Mutably access channel at the given index
    pub fn get_mut(&mut self, i: usize) -> &'a mut [T] {
        unsafe { slice::from_raw_parts_mut(self.bufs[i], self.samples) }
    }

    /// Split borrowing at the given index, like for slices
    pub fn split_at_mut(self, i: usize) -> (Outputs<'a, T>, Outputs<'a, T>) {
        let (l, r) = self.bufs.split_at(i);
        (
            Outputs {
                bufs: l,
                samples: self.samples,
            },
            Outputs {
                bufs: r,
                samples: self.samples,
            },
        )
    }
}

impl<'a, T> Index<usize> for Outputs<'a, T> {
    type Output = [T];

    fn index(&self, i: usize) -> &Self::Output {
        self.get(i)
    }
}

impl<'a, T> IndexMut<usize> for Outputs<'a, T> {
    fn index_mut(&mut self, i: usize) -> &mut Self::Output {
        self.get_mut(i)
    }
}

/// Iterator over buffers for output channels of an `AudioBuffer`.
pub struct OutputIterator<'a, 'b, T>
where
    T: 'a,
    'a: 'b,
{
    data: &'b mut Outputs<'a, T>,
    i: usize,
}

impl<'a, 'b, T> Iterator for OutputIterator<'a, 'b, T>
where
    T: 'b,
{
    type Item = &'b mut [T];

    fn next(&mut self) -> Option<Self::Item> {
        if self.i < self.data.len() {
            let val = self.data.get_mut(self.i);
            self.i += 1;
            Some(val)
        } else {
            None
        }
    }
}

impl<'a, 'b, T: Sized> IntoIterator for &'b mut Outputs<'a, T> {
    type Item = &'b mut [T];
    type IntoIter = OutputIterator<'a, 'b, T>;

    fn into_iter(self) -> Self::IntoIter {
        OutputIterator { data: self, i: 0 }
    }
}

pub struct HostBuffer<T: Float> {
    inputs: Vec<*const T>,
    outputs: Vec<*mut T>,
}

impl<T: Float> HostBuffer<T> {
    /// Create a `HostBuffer` for a given number of input and output channels.
    pub fn new(input_count: usize, output_count: usize) -> HostBuffer<T> {
        HostBuffer {
            inputs: vec![core::ptr::null(); input_count],
            outputs: vec![core::ptr::null_mut(); output_count],
        }
    }

    /// Bind sample arrays to the `HostBuffer` to create an `AudioBuffer` to pass to a plugin.
    ///
    /// # Panics
    /// This function will panic if more inputs or outputs are supplied than the `HostBuffer`
    /// was created for, or if the sample arrays do not all have the same length.
    pub fn bind<'a, I, O>(&'a mut self, input_arrays: &[I], output_arrays: &mut [O]) -> AudioBuffer<'a, T>
    where
        I: AsRef<[T]> + 'a,
        O: AsMut<[T]> + 'a,
    {
        // Check that number of desired inputs and outputs fit in allocation
        if input_arrays.len() > self.inputs.len() {
            panic!("Too many inputs for HostBuffer");
        }
        if output_arrays.len() > self.outputs.len() {
            panic!("Too many outputs for HostBuffer");
        }

        // Initialize raw pointers and find common length
        let mut length = None;
        for (i, input) in input_arrays.iter().map(|r| r.as_ref()).enumerate() {
            self.inputs[i] = input.as_ptr();
            match length {
                None => length = Some(input.len()),
                Some(old_length) => {
                    if input.len() != old_length {
                        panic!("Mismatching lengths of input arrays");
                    }
                }
            }
        }
        for (i, output) in output_arrays.iter_mut().map(|r| r.as_mut()).enumerate() {
            self.outputs[i] = output.as_mut_ptr();
            match length {
                None => length = Some(output.len()),
                Some(old_length) => {
                    if output.len() != old_length {
                        panic!("Mismatching lengths of output arrays");
                    }
                }
            }
        }
        let length = length.unwrap_or(0);

        // Construct AudioBuffer
        unsafe {
            AudioBuffer::from_raw(
                input_arrays.len(),
                output_arrays.len(),
                self.inputs.as_ptr(),
                self.outputs.as_mut_ptr(),
                length,
            )
        }
    }

    /// Number of input channels supported by this `HostBuffer`.
    pub fn input_count(&self) -> usize {
        self.inputs.len()
    }

    /// Number of output channels supported by this `HostBuffer`.
    pub fn output_count(&self) -> usize {
        self.outputs.len()
    }
}


pub type PluginMain = fn(callback: HostCallbackProc) -> *mut AEffect;




/// Copy a string into a destination buffer.
///
/// String will be cut at `max` characters.
fn copy_string(dst: *mut c_void, src: &str, max: usize) -> isize {
    unsafe {
        use libc::{memcpy, memset};
        use std::cmp::min;

        let dst = dst as *mut c_void;
        memset(dst, 0, max);
        memcpy(dst, src.as_ptr() as *const c_void, min(max, src.as_bytes().len()));
    }

    1 // Success
}

fn copy_time_info(dst: *mut c_void) -> isize {
    unsafe {
        use libc::{memcpy, memset};

        memset(dst, 0, std::mem::size_of::<TimeInfo>());
        memcpy(dst, std::mem::transmute(&TIME_INFO), std::mem::size_of::<TimeInfo>());
    }

    std::mem::size_of::<TimeInfo>() as isize // Success
}

static mut TIME_INFO: TimeInfo = TimeInfo {
    sample_pos: 0.0,
    sample_rate: 44100.0,
    nano_seconds: 0.0,
    ppq_pos: 0.0,
    tempo: 140.0,
    bar_start_pos: 0.0,
    cycle_start_pos: 0.0,
    cycle_end_pos: 0.0,
    time_sig_numerator: 4,
    time_sig_denominator: 4,
    smpte_offset: 0,
    smpte_frame_rate: 0,
    samples_to_next_clock: 0,
    flags: 3,
};

extern "C" fn vst_host_callback(effect: *mut AEffect, op_code: i32, _index: i32, _value: isize, ptr: *mut c_void, _optional: f32) -> isize {
    unsafe {
        if op_code == host_opcodes::VERSION {
            println!("Opcode=VERSION");
            24
        }
        else if op_code == host_opcodes::WANT_MIDI {
            println!("Opcode=WANT_MIDI");
            0
        }
        else if op_code == host_opcodes::GET_PRODUCT_STRING {
            println!("Opcode=GET_PRODUCT_STRING");
            let product_string = "frdm".to_owned();
            copy_string(ptr, &product_string, 64)
        }
        else if op_code == host_opcodes::CURRENT_ID {
            println!("Opcode=CURRENT_ID");
            PLUG_ID
        }
        else if op_code == host_opcodes::SIZE_WINDOW {
            println!("Opcode=SIZE_WINDOW");
            1
        }
        else if op_code == host_opcodes::CAN_DO {
            println!("Opcode=CAN_DO");
            1
        }
        else if op_code == host_opcodes::GET_TIME {
            println!("Opcode=GET_TIME");
            // copy_time_info(ptr)

            // TODO this needs to increments the ppq_pos beats properly
            TIME_INFO.ppq_pos += 1.0;

            let mut flags = transport::CHANGED;

            flags |= transport::PLAYING; // transport playing
            flags |= time_info_flags::TEMPO_VALID; // tempo valid
            flags |= time_info_flags::TIME_SIG_VALID; // time signature valid
            flags |= time_info_flags::PPQ_POS_VALID; // ppq position valid

            TIME_INFO.flags = flags;
    
            std::mem::transmute(&TIME_INFO)
        }
        else if op_code == host_opcodes::GET_CURRENT_PROCESS_LEVEL {
            println!("Opcode=GET_CURRENT_PROCESS_LEVEL");
            2
        }
        else if op_code == host_opcodes::IDLE {
            println!("Opcode=IDLE");
            let dispatcher = (*effect).dispatcher;
            dispatcher(effect, effect_opcodes::IDLE, 0 , 0, core::ptr::null_mut(), 0.0);
            0
        }
        else {
            println!("Opcode=Unknown: {}", op_code);
            0
        }
    }
}

fn check_vst_plugin(vst_plugin_path: &str) {
    unsafe {
        match libloading::Library::new(vst_plugin_path.clone()) {
            Ok(lib) => {
                let lib_vst_plug_in_main_function: Result<libloading::Symbol<PluginMain>, libloading::Error> = lib.get(b"VSTPluginMain");
                match lib_vst_plug_in_main_function {
                    Ok(vst_main) => {
                        let effect = vst_main(vst_host_callback);
                        let _num_inputs = (*effect).num_inputs;
                        let num_outputs = (*effect).num_outputs;

                        println!("Got effect: magic={}, num_programs={}, num_params={}, num_inputs={}, num_outputs={}, flags={}, initial_delay={}. unique_id={}, version={}",
                            (*effect).magic        ,
                            (*effect).num_programs ,
                            (*effect).num_params   ,
                            (*effect).num_inputs   ,
                            (*effect).num_outputs  ,
                            (*effect).flags        ,
                            (*effect).initial_delay,
                            (*effect).unique_id    ,
                            (*effect).version);

                        println!("Can replacing: {}", (*effect).flags & effect_flags::CAN_REPLACING == 16);

                        let dispatcher = (*effect).dispatcher;
                        let process = (*effect).process_replacing;

                        let mut plugin_category = dispatcher(effect, effect_opcodes::GET_PLUG_CATEGORY, 0 , 0, core::ptr::null_mut(), 0.0);
                        if plugin_category as i32 == plug_category::SHELL {
                            // println!("shell=true");
                            loop {
                                let buffer: [u8; 40] = [0; 40];
                                let plug_id = dispatcher(effect, effect_opcodes::SHELL_GET_NEXT_PLUGIN, 0 , 0, std::mem::transmute(&buffer), 0.0);
                                if plug_id == 0 {
                                    break;
                                }
                                else {
                                    PLUG_ID = plug_id;
                                }
                                if PLUG_ID != 0 {
                                    let shell_plug_effect = vst_main(vst_host_callback);

                                    plugin_category = dispatcher(shell_plug_effect, effect_opcodes::GET_PLUG_CATEGORY, 0 , 0, core::ptr::null_mut(), 0.0);

                                    println!("shell_plugin_id={}", plug_id);
                                    println!("shell_plugin_name={}", std::str::from_utf8(&buffer).expect("msg").trim_matches(char::from(0)));
                                    println!("shell_plugin_category={}", plugin_category);
                                    println!("##########{}:{}:{}:{}:VST24", std::str::from_utf8(&buffer).expect("Could not unpack plugin name").trim_matches(char::from(0)), vst_plugin_path, plug_id, plugin_category);

                                    if num_outputs > 0 {
                                        check_plugin_process_replacing(&shell_plug_effect, dispatcher, process);
                                    }
                                }
                            }
                        }
                        else {
                            let buffer: [u8; 40] = [0; 40];
                            println!("plugin_category={}", plugin_category);
                            dispatcher(effect, effect_opcodes::GET_EFFECT_NAME, 0 , 0, std::mem::transmute(&buffer), 0.0);
                            println!("##########{}:{}::{}:VST24", std::str::from_utf8(&buffer).expect("Could not unpack plugin name").trim_matches(char::from(0)), vst_plugin_path, plugin_category);

                            if num_outputs > 0 {
                                check_plugin_process_replacing(&effect, dispatcher, process);
                            }
                        }
                    },
                    Err(_) => (),
                }
            },
            Err(_) => println!("Couldn't load library: {}", vst_plugin_path),
        }
    }
}

fn check_plugin_process_replacing(effect: &*mut AEffect, dispatcher: extern "C" fn(*mut AEffect, i32, i32, isize, *mut c_void, f32) -> isize, process: extern "C" fn(*mut AEffect, *const *const f32, *mut *mut f32, i32)) {
    unsafe {
        let buffer: [u8; 40] = [0; 40];
        dispatcher(*effect, effect_opcodes::GET_VENDOR_STRING, 0, 0, std::mem::transmute(&buffer), 0.0);
        println!("vendor string: {}", std::str::from_utf8(&buffer).expect("Could not unpack vendor string").trim_matches(char::from(0)));
        let buffer: [u8; 40] = [0; 40];
        dispatcher(*effect, effect_opcodes::GET_PRODUCT_STRING, 0, 0, std::mem::transmute(&buffer), 0.0);
        println!("product string: {}", std::str::from_utf8(&buffer).expect("Could not unpack product string").trim_matches(char::from(0)));
        let buffer: [u8; 40] = [0; 40];
        dispatcher(*effect, effect_opcodes::GET_VENDOR_VERSION, 0, 0, std::mem::transmute(&buffer), 0.0);
        println!("vendor version: {}", std::str::from_utf8(&buffer).expect("Could not unpack vendor version").trim_matches(char::from(0)));


        dispatcher(*effect, effect_opcodes::OPEN, 0, 0, core::ptr::null_mut(), 0.0);
        dispatcher(*effect, effect_opcodes::SET_SAMPLE_RATE, 0, 0, core::ptr::null_mut(), 44100.0);
        dispatcher(*effect, effect_opcodes::SET_BLOCK_SIZE, 0, 1024, core::ptr::null_mut(), 0.0);
        dispatcher(*effect, effect_opcodes::MAINS_CHANGED, 0, 1, core::ptr::null_mut(), 0.0);
        dispatcher(*effect, effect_opcodes::START_PROCESS, 0, 0, core::ptr::null_mut(), 0.0);


        let mut host_buffer_2ch: HostBuffer<f32> = HostBuffer::new(16, 16);
        let inputs_2ch = vec![vec![0.0; NUMBER_OF_FRAMES]; 16];
        let mut outputs_2ch = vec![vec![0.0; NUMBER_OF_FRAMES]; 16];
        let mut audio_buffer_2ch = host_buffer_2ch.bind(&inputs_2ch, &mut outputs_2ch);

        let mut count = 0;
        let mut note_sounding = false;
        let mut found_non_zero = 0;
        while count < 20 {
            if count % 4 == 0 {
                if note_sounding {
                    // println!("Stopping note.");
                    note_sounding = false;
                    let midi_event = MidiEvent {
                        event_type: MIDI_TYPE,
                        byte_size: core::mem::size_of::<MidiEvent>() as i32,
                        delta_frames: 0,
                        flags: 0,
                        note_length: 0,
                        note_offset: 0,
                        midi_data: [0x80, 24, 127, 0],
                        detune: 0,
                        note_off_velocity: 0,
                        reserved_1: 0,
                        reserved_2: 0
                    };
                    let event = Event {
                        dump: std::mem::transmute(midi_event)
                    };
                    let mut events = Events {
                        num_events: 1,
                        reserved: core::ptr::null_mut(),
                        events: [&event, &event]
                    };
                    let ptr_events: *mut c_void = &mut events as *mut _ as *mut c_void;
                    // println!("Sending node off...");
                    dispatcher(*effect, effect_opcodes::PROCESS_EVENTS, 0, 0, ptr_events, 0.0);
                } else {
                    note_sounding = true;
                    // println!("Playing note...");
                    let midi_event = MidiEvent {
                        event_type: MIDI_TYPE,
                        byte_size: core::mem::size_of::<MidiEvent>() as i32,
                        delta_frames: 0,
                        flags: 0,
                        note_length: 0,
                        note_offset: 0,
                        midi_data: [0x90, 24, 127, 0],
                        detune: 0,
                        note_off_velocity: 0,
                        reserved_1: 0,
                        reserved_2: 0
                    };
                    let event = Event {
                        dump: std::mem::transmute(midi_event)
                    };
                    let mut events = Events {
                        num_events: 1,
                        reserved: core::ptr::null_mut(),
                        events: [&event, &event]
                    };
                    let ptr_events: *mut c_void = &mut events as *mut _ as *mut c_void;
                    // println!("Sending node on...");
                    dispatcher(*effect, effect_opcodes::PROCESS_EVENTS, 0, 0, ptr_events, 0.0);
                }
            }

            // println!("Processing audio...");
            process(*effect, audio_buffer_2ch.raw_outputs().as_ptr() as *const *const f32, audio_buffer_2ch.raw_outputs().as_mut_ptr() as *mut *mut _, NUMBER_OF_FRAMES as i32);
            // println!("Processed audio.");

            let frames = audio_buffer_2ch.samples();
            let (_, mut outputs_64x) = audio_buffer_2ch.split();
            let channels = outputs_64x.len();
            for frame_index in 0..frames {
                for channel_index in 0..channels {
                    let channel_64 = outputs_64x.get_mut(channel_index);

                    if channel_64[frame_index] != 0.0 {
                        found_non_zero += 1;
                    }
                }
            }

            count += 1;
        }

        if found_non_zero > 0 {
            println!("Calling process produced non zero data.");
        } else {
            println!("Calling process did not produce non zero data.");
        }

        dispatcher(*effect, effect_opcodes::MAINS_CHANGED, 0, 0, core::ptr::null_mut(), 0.0);
        dispatcher(*effect, effect_opcodes::CLOSE, 0, 0, core::ptr::null_mut(), 0.0);
    }
}


fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() == 2 {
        if let Some(vst_plugin_path) = args.get(1) {
            if vst_plugin_path.contains(',') {
                for plugin in vst_plugin_path.replace('\"', "").as_str().split(',').collect::<Vec<&str>>().iter() {
                    check_vst_plugin(plugin);
                }
            }
            else {
                check_vst_plugin(vst_plugin_path.replace('\"', "").as_str());
            }
        }
    }
    else {
        println!("Something wrong with command line argument(s) given: {:?}", args);
    }
}
