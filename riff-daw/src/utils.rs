use std::any::Any;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use clap_sys::events::{CLAP_CORE_EVENT_SPACE_ID, clap_event_header, CLAP_EVENT_MIDI, clap_event_midi, clap_event_note, clap_event_note_expression, CLAP_EVENT_NOTE_EXPRESSION, CLAP_EVENT_NOTE_OFF, CLAP_EVENT_NOTE_ON, clap_event_param_value, CLAP_EVENT_PARAM_VALUE, CLAP_NOTE_EXPRESSION_BRIGHTNESS, CLAP_NOTE_EXPRESSION_EXPRESSION, CLAP_NOTE_EXPRESSION_PAN, CLAP_NOTE_EXPRESSION_PRESSURE, CLAP_NOTE_EXPRESSION_TUNING, CLAP_NOTE_EXPRESSION_VIBRATO, CLAP_NOTE_EXPRESSION_VOLUME};
use clap_sys::id::clap_id;
use vst::event::*;
use log::*;

use crate::domain::{AudioRouting, AudioRoutingNodeType, Controller, DAWItemPosition, Measure, NoteOff, NoteOn, PitchBend, PluginParameter, Riff, RiffItemType, RiffReference, Track, TrackEvent, TrackEventRouting, TrackEventRoutingNodeType, DAWItemLength, RiffGrid, RiffReferenceMode, AutomationEnvelope, Automation};
use crate::DAWState;
use crate::state::MidiPolyphonicExpressionNoteId;

pub struct CalculatedSnap {
    pub snapped_value: f64,
    pub calculated_delta: f64,
    pub snapped: bool,
}

pub struct DAWUtils;

impl DAWUtils {

    pub fn sort_by_daw_position(a: &dyn DAWItemPosition, b: &dyn DAWItemPosition) -> Ordering {
        if (a.position() - b.position()) > f64::EPSILON {
            return Ordering::Greater
        }
        else if (b.position() - a.position()) > f64::EPSILON {
            return Ordering::Less
        };

        Ordering::Equal
    }

    pub fn sort_track_events(a: &TrackEvent, b: &TrackEvent) -> Ordering {
        // match a {
        //     TrackEvent::NoteOn(note_on) => debug!("Note on: position={}", note_on.position()),
        //     TrackEvent::NoteOff(note_off) => debug!("Note off: position={}", note_off.position()),
        //     TrackEvent::Measure(measure) => debug!("Measure: position={}", measure.position()),
        //     _ => debug!("Unknown event type")
        // }
        if (a.position() - b.position()) > f64::EPSILON {
            Ordering::Greater
        }
        else if (b.position() - a.position()) > f64::EPSILON {
            Ordering::Less
        }
        else {
            if let TrackEvent::Measure(measure) = &a {
                Ordering::Less
            }
            else if let TrackEvent::Measure(measure) = &b {
                Ordering::Greater
            }
            else {
                Ordering::Equal
            }
        }
    }

    pub fn get_snap_quantise_value_in_seconds_from_choice_text(
        choice_text_value: &str,
        tempo: f64,
        beats_per_bar: f64
    ) -> f64 {
        DAWUtils::get_snap_quantise_value_in_beats_from_choice_text(choice_text_value, beats_per_bar) * tempo / 60.0
    }

    pub fn get_snap_quantise_value_in_beats_from_choice_text(
        choice_text_value: &str,
        beats_per_bar: f64
    ) -> f64 {
        if choice_text_value =="1.5"
        {
            1.5 * beats_per_bar
        }
        else if choice_text_value =="10"
        {
            10.0 * beats_per_bar
        }
        else if choice_text_value =="9"
        {
            9.0 * beats_per_bar
        }
        else if choice_text_value =="8"
        {
            8.0 * beats_per_bar
        }
        else if choice_text_value =="7"
        {
            7.0 * beats_per_bar
        }
        else if choice_text_value =="6"
        {
            6.0 * beats_per_bar
        }
        else if choice_text_value =="5"
        {
            5.0 * beats_per_bar
        }
        else if choice_text_value =="4"
        {
            4.0 * beats_per_bar
        }
        else if choice_text_value =="3"
        {
            3.0 * beats_per_bar
        }
        else if choice_text_value =="2"
        {
            2.0 * beats_per_bar
        }
        else if choice_text_value =="1"
        {
            4.0
        }
        else if choice_text_value =="1/2."
        {
            3.0
        }
        else if choice_text_value =="1/2"
        {
            2.0
        }
        else if choice_text_value =="1/4."
        {
            1.5
        }
        else if choice_text_value =="1/4"
        {
            1.0
        }
        else if choice_text_value =="1/4 triplet"
        {
            1.0 * 2.0 / 3.0
        }
        else if choice_text_value =="1/8."
        {
            0.75
        }
        else if choice_text_value =="1/8"
        {
            0.5
        }
        else if choice_text_value =="1/8 triplet"
        {
            0.5 * 2.0 / 3.0
        }
        else if choice_text_value =="1/16."
        {
            0.375
        }
        else if choice_text_value =="1/16"
        {
            0.25
        }
        else if choice_text_value =="1/16 triplet"
        {
            0.25 * 2.0 / 3.0
        }
        else if choice_text_value =="1/32."
        {
            0.1875
        }
        else if choice_text_value =="1/32"
        {
            0.125
        }
        else if choice_text_value =="1/64."
        {
            0.09375
        }
        else if choice_text_value =="1/64"
        {
            0.0625
        }
        else
        {
            0.0
        }
    }

    pub fn quantise(
        value: f64,
        snap_in_beats: f64,
        strength: f64,
        length: bool,
    ) -> CalculatedSnap {
        // need to determine which direction to snap in
        // work out backwards and forwards deltas
        let backward_snap_delta = value % snap_in_beats;
        let mut calculated_delta = 0.0;
        let mut snapped = false;
        let mut snapped_value = value;

        if length && value < snap_in_beats {
            calculated_delta = snap_in_beats - value;
            snapped_value = snap_in_beats;
            snapped = true;
        }
        else if backward_snap_delta > 0.0 {
            let forward_snap_delta = snap_in_beats - backward_snap_delta;

            // use smallest delta
            if backward_snap_delta < forward_snap_delta {
                calculated_delta = backward_snap_delta * strength * -1.0;
                let new_value = value + calculated_delta;
                if new_value >= 0.0 {
                    snapped_value = new_value;
                    snapped = true;
                } else {
                    calculated_delta = 0.0;
                }
            } else if forward_snap_delta > 0.0 {
                calculated_delta = forward_snap_delta * strength;
                snapped_value = value + calculated_delta;
                snapped = true;
            }
        }

        CalculatedSnap { snapped_value, calculated_delta, snapped }
    }

    pub fn convert_to_event_blocks(
        automation: &Automation,
        riffs: &Vec<Riff>,
        riff_refs: &Vec<RiffReference>,
        bpm: f64,
        block_size_in_samples: f64,
        sample_rate: f64,
        passage_length_in_beats: f64,
        midi_channel: i32,
        automation_discrete: bool,
    ) -> (Vec<Vec<TrackEvent>>, Vec<Vec<PluginParameter>>) {
        debug!("Automation events for track: {}", automation.events().len());
        debug!("Automation envelopes for track: {}", automation.envelopes().len());
        // TODO need to make sure that this doesn't cross over into the next measure
        // let passage_length_in_frames = passage_length_in_beats / bpm * 60.0 * sample_rate - 1024.0; 
        let passage_length_in_frames = passage_length_in_beats / bpm * 60.0 * sample_rate; 

        debug!("util - convert_to_event_blocks: passage_length_in_frames={}", passage_length_in_frames);

        let mut track_events: Vec<TrackEvent> = Self::extract_riff_ref_events(riffs, riff_refs, bpm, sample_rate, midi_channel);
        debug!("Number of riff ref events extracted for track: {}", track_events.len());
        let plugin_parameter_events = if automation_discrete {
            Self::convert_automation_events(automation.events(), bpm, sample_rate, &mut track_events, midi_channel)
        }
        else {
            Self::convert_automation_envelope_events(automation.envelopes(), bpm, sample_rate, block_size_in_samples, &mut track_events, passage_length_in_frames)
        };
        debug!("Number of riff ref automation parameter events extracted for track: {}", plugin_parameter_events.len());

        let event_blocks = Self::create_track_event_blocks(block_size_in_samples, passage_length_in_frames, &mut track_events);
        let param_event_blocks = Self::create_plugin_parameter_blocks(block_size_in_samples, passage_length_in_frames, &plugin_parameter_events);

        (event_blocks, param_event_blocks)
    }

    fn create_plugin_parameter_blocks(block_size_in_samples: f64, passage_length_in_frames: f64, plugin_parameter_events: &Vec<PluginParameter>) -> Vec<Vec<PluginParameter>> {
        let mut param_event_blocks = vec![];
        for current_start_frame in (0..passage_length_in_frames as i32).step_by(block_size_in_samples as usize) {
            let mut param_event_block: Vec<PluginParameter> = Vec::new();
            let current_end_frame = current_start_frame + block_size_in_samples as i32;

            // loop through param events
            // only start processing when events are in range
            for event in plugin_parameter_events.iter() {
                let absolute_position_in_frames = event.position() as i32;
                if current_start_frame <= absolute_position_in_frames && absolute_position_in_frames < current_end_frame {
                    param_event_block.push(event.clone());
                }

                if absolute_position_in_frames >= current_end_frame {
                    break;
                }
            }

            param_event_blocks.push(param_event_block);
        }
        param_event_blocks
    }

    fn create_midi_event_blocks(block_size_in_samples: f64, passage_length_in_frames: f64, midi_events: &mut Vec<MidiEvent>) -> Vec<Vec<MidiEvent>> {
        let mut event_blocks = vec![];
        for current_start_frame in (0..passage_length_in_frames as i32).step_by(block_size_in_samples as usize) {
            let mut event_block: Vec<MidiEvent> = Vec::new();
            let current_end_frame = current_start_frame + block_size_in_samples as i32;

            // loop through events
            // only start processing when events are in range
            // adjust the delta frames back from absolute frames to block relative delta frames
            for event in midi_events.iter() {
                let absolute_delta_frames = event.delta_frames;
                if current_start_frame <= absolute_delta_frames && absolute_delta_frames < current_end_frame {
                    let mut adjusted_event = *event;
                    adjusted_event.delta_frames = absolute_delta_frames - current_start_frame;
                    event_block.push(adjusted_event);
                }

                if absolute_delta_frames >= current_end_frame {
                    break;
                }
            }

            event_blocks.push(event_block);
        }
        event_blocks
    }

    fn create_track_event_blocks(block_size_in_samples: f64, passage_length_in_frames: f64, track_events: &mut Vec<TrackEvent>) -> Vec<Vec<TrackEvent>> {
        let mut event_blocks = vec![];
        for current_start_frame in (0..passage_length_in_frames as i32).step_by(block_size_in_samples as usize) {
            let mut event_block: Vec<TrackEvent> = Vec::new();
            let current_end_frame = current_start_frame + block_size_in_samples as i32;

            // loop through events
            // only start processing when events are in range
            // adjust the delta frames back from absolute frames to block relative delta frames
            for event in track_events.iter() {
                let absolute_delta_frames = event.position() as i32;
                // debug!("create_track_event_blocks: event position={}, current_start_frame={}, current_end_frame={}", event.position(), current_start_frame, current_end_frame);
                if current_start_frame <= absolute_delta_frames && absolute_delta_frames < current_end_frame {
                    let mut adjusted_event = event.clone();
                    adjusted_event.set_position((absolute_delta_frames - current_start_frame) as f64);
                    event_block.push(adjusted_event);
                }

                if absolute_delta_frames >= current_end_frame {
                    break;
                }
            }

            // debug!("Created track event block length: {}", event_block.len());
            event_blocks.push(event_block);
        }
        event_blocks
    }

    fn convert_automation_events_to_vst(automation: &Vec<TrackEvent>, bpm: f64, sample_rate: f64, events_all: &mut Vec<MidiEvent>, midi_channel: i32) -> Vec<PluginParameter> {
        let mut plugin_parameter_events: Vec<PluginParameter> = Vec::new();
        for event in automation {
            match event {
                TrackEvent::Note(_) => {},
                TrackEvent::NoteOn(_) => {}
                TrackEvent::NoteOff(_) => {}
                TrackEvent::Controller(controller) => {
                    let position_in_frames = controller.position() / bpm * 60.0 * sample_rate;
                    let controller_event = MidiEvent {
                        data: [176 + (midi_channel as u8), controller.controller() as u8, controller.value() as u8],
                        delta_frames: position_in_frames as i32,
                        live: true,
                        note_length: None,
                        note_offset: None,
                        detune: 0,
                        note_off_velocity: 0,
                    };
                    events_all.push(controller_event);
                },
                TrackEvent::PitchBend(_pitch_bend) => {}
                TrackEvent::AudioPluginParameter(parameter) => {
                    let mut param_copy = parameter.clone();
                    let param_position_in_frames = param_copy.position() / bpm * 60.0 * sample_rate;
                    param_copy.set_position(param_position_in_frames);
                    plugin_parameter_events.push(param_copy);
                },
                TrackEvent::Sample(_sample) => {}
                _ => {}
            }
        }
        plugin_parameter_events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
        plugin_parameter_events
    }

    fn convert_automation_events(automation: &Vec<TrackEvent>, bpm: f64, sample_rate: f64, events_all: &mut Vec<TrackEvent>, _midi_channel: i32) -> Vec<PluginParameter> {
        let mut plugin_parameter_events: Vec<PluginParameter> = Vec::new();
        for event in automation {
            match event {
                TrackEvent::NoteExpression(note_expression) => {
                    let mut event = note_expression.clone();
                    event.set_position(event.position() / bpm * 60.0 * sample_rate);
                    events_all.push(TrackEvent::NoteExpression(event));
                }
                TrackEvent::Controller(controller) => {
                    let mut controller_event = controller.clone();
                    controller_event.set_position(controller_event.position() / bpm * 60.0 * sample_rate);
                    events_all.push(TrackEvent::Controller(controller_event));
                }
                TrackEvent::PitchBend(_pitch_bend) => {
                    let mut pitch_bend = _pitch_bend.clone();
                    pitch_bend.set_position(pitch_bend.position() / bpm * 60.0 * sample_rate);
                    events_all.push(TrackEvent::PitchBend(pitch_bend));
                }
                TrackEvent::AudioPluginParameter(parameter) => {
                    let mut param_copy = parameter.clone();
                    param_copy.set_position(param_copy.position() / bpm * 60.0 * sample_rate);
                    plugin_parameter_events.push(param_copy);
                }
                _ => {}
            }
        }
        events_all.sort_by(|event1, event2| DAWUtils::sort_by_daw_position(event1, event2));
        plugin_parameter_events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
        plugin_parameter_events
    }

    fn convert_automation_envelope_events(
        automation_envelopes: &Vec<AutomationEnvelope>,
        bpm: f64,
        sample_rate: f64,
        block_size_in_samples: f64,
        events_all: &mut Vec<TrackEvent>,
        passage_length_in_frames: f64
    ) -> Vec<PluginParameter> {
        let mut plugin_parameter_events: Vec<PluginParameter> = Vec::new();
        for envelope in automation_envelopes.iter() {
            let event_details: TrackEvent = envelope.event_details().clone();

            for position_in_samples in (0..(passage_length_in_frames as i32)).step_by(block_size_in_samples as usize) {
                // find applicable envelope events
                let mut point_1 = None;
                let mut point_2 = None;
                // zoom until an envelope event position is greater than the current position
                for event in envelope.events().iter() {
                    let envelope_position = (event.position() / bpm * 60.0 * sample_rate) as i32;
                    if envelope_position >= position_in_samples {
                        point_2 = Some((envelope_position as f64, event.value()));
                        break;
                    }
                    if position_in_samples > envelope_position {
                        point_1 = Some((envelope_position as f64, event.value()));
                    }
                }

                if let Some(point_1) = point_1 {
                    if let Some(point_2) = point_2 {
                        let slope = (point_2.1 - point_1.1) / (point_2.0 - point_1.0);
                        let mut event = event_details.clone();
                        let value = slope * (position_in_samples as f64 - point_1.0) + point_1.1;

                        event.set_position(position_in_samples as f64);
                        event.set_value(value);

                        if let TrackEvent::AudioPluginParameter(param) = event {
                            plugin_parameter_events.push(param);
                        }
                        else {
                            events_all.push(event);
                        }
                    }
                    else {
                        // the position is greater than the last point in the envelope so we generate events with the same value (slope of 0)
                        let mut event = event_details.clone();

                        event.set_position(position_in_samples as f64);
                        event.set_value(point_1.1);

                        if let TrackEvent::AudioPluginParameter(param) = event {
                            plugin_parameter_events.push(param);
                        }
                        else {
                            events_all.push(event);
                        }
                    }
                }
            }
        }
        events_all.sort_by(|event1, event2| DAWUtils::sort_by_daw_position(event1, event2));
        plugin_parameter_events.sort_by(|param1, param2| DAWUtils::sort_by_daw_position(param1, param2));
        plugin_parameter_events
    }

    fn convert_riff_ref_events_to_vst(riffs: &Vec<Riff>, riff_refs: &Vec<RiffReference>, bpm: f64, sample_rate: f64, midi_channel: i32) -> Vec<MidiEvent> {
        let mut events_all: Vec<MidiEvent> = Vec::new();

        for riff_ref in riff_refs {
            for riff in riffs.iter() {
                if riff.uuid().to_string() == riff_ref.linked_to() {
                    debug!("util-convert_to_vst_events: riff name={}", riff.name());
                    for event in riff.events().iter() {
                        match event {
                            TrackEvent::Note(note) => {
                                debug!("DAWUtils.convert_riff_ref_events_to_vst: note off - riff_ref.position={}, note.end position={}, note.duration={}", riff_ref.position(), note.position(), note.length());
                                let note_on_position_in_frames = (riff_ref.position() + note.position()) / bpm * 60.0 * sample_rate;
                                let note_on = MidiEvent {
                                    data: [144 + (midi_channel as u8), note.note() as u8, note.velocity() as u8],
                                    delta_frames: note_on_position_in_frames as i32,
                                    live: true,
                                    note_length: None,
                                    note_offset: None,
                                    detune: 0,
                                    note_off_velocity: 0,
                                };
                                events_all.push(note_on);
                                let note_off_position_in_frames = (riff_ref.position() + note.position() + note.length()) / bpm * 60.0 * sample_rate;
                                let note_off = MidiEvent {
                                    data: [128 + (midi_channel as u8), note.note() as u8, 0],
                                    delta_frames: note_off_position_in_frames as i32,
                                    live: true,
                                    note_length: None,
                                    note_offset: None,
                                    detune: 0,
                                    note_off_velocity: 0,
                                };
                                events_all.push(note_off);
                            }
                            TrackEvent::NoteOn(_) => {}
                            TrackEvent::NoteOff(_) => {}
                            TrackEvent::Controller(controller) => {
                                let position_in_frames = controller.position() / bpm * 60.0 * sample_rate;
                                let controller_event = MidiEvent {
                                    data: [176 + (midi_channel as u8), controller.controller() as u8, controller.value() as u8],
                                    delta_frames: position_in_frames as i32,
                                    live: true,
                                    note_length: None,
                                    note_offset: None,
                                    detune: 0,
                                    note_off_velocity: 0,
                                };
                                events_all.push(controller_event);
                            }
                            TrackEvent::PitchBend(pitch_bend) => {
                                let position_in_frames = pitch_bend.position() / bpm * 60.0 * sample_rate;
                                let (lsb, msb) = pitch_bend.midi_bytes_from_value();
                                let pitch_bend_event = MidiEvent {
                                    data: [224 + (midi_channel as u8), lsb, msb],
                                    delta_frames: position_in_frames as i32,
                                    live: true,
                                    note_length: None,
                                    note_offset: None,
                                    detune: 0,
                                    note_off_velocity: 0,
                                };
                                events_all.push(pitch_bend_event);
                            }
                            TrackEvent::AudioPluginParameter(_) => {}
                            TrackEvent::Sample(sample) => {
                                let note_on_position_in_frames = (riff_ref.position() + sample.position()) / bpm * 60.0 * sample_rate;
                                let note_on = MidiEvent {
                                    data: [144 + (midi_channel as u8), 60, 127],
                                    delta_frames: note_on_position_in_frames as i32,
                                    live: true,
                                    note_length: None,
                                    note_offset: None,
                                    detune: 0,
                                    note_off_velocity: 0,
                                };
                                events_all.push(note_on);
                                let note_off_position_in_frames = (riff_ref.position() + sample.position() + 1.0 /* FIXME needs to be the sample length */) / bpm * 60.0 * sample_rate;
                                let note_off = MidiEvent {
                                    data: [128 + (midi_channel as u8), 60, 127],
                                    delta_frames: note_off_position_in_frames as i32,
                                    live: true,
                                    note_length: None,
                                    note_offset: None,
                                    detune: 0,
                                    note_off_velocity: 0,
                                };
                                events_all.push(note_off);
                            }
                            _ => {}
                        }
                    }

                    // add the measure boundary markers
                    let number_of_measures = (riff.length() / 4.0) as i32; // TODO need to pass through the beats per bar
                    for measure_number in 0..number_of_measures {
                        let measure_boundary_marker = MidiEvent {
                            data: [255, 0, 0],
                            delta_frames: ((riff_ref.position() + (((measure_number + 1) * 4) as f64)) / bpm * 60.0 * sample_rate) as i32,
                            live: true,
                            note_length: None,
                            note_offset: None,
                            detune: 0,
                            note_off_velocity: 0,
                        };
                        events_all.push(measure_boundary_marker);
                        debug!("^^^^^^^^^^^^^^^^^^^^^^ added a measure boundary");
                    }

                    // somehow add the full play out loop point marker


                    break;
                }
            }
        }

        events_all.sort_by(|a, b| a.delta_frames.cmp(&b.delta_frames));
        events_all
    }

    pub fn convert_vst_events_to_track_events_with_timing_in_frames(vst_events: Vec<MidiEvent>) -> Vec<TrackEvent> {
        let mut track_events = vec![];

        for event in vst_events.iter() {
            if 128 <= event.data[0] && event.data[0] <= 143 { // note off
                track_events.push(TrackEvent::NoteOff(NoteOff::new_with_params(MidiPolyphonicExpressionNoteId::ALL as i32, event.delta_frames as f64, event.data[1] as i32, event.data[2] as i32)));
            } 
            else if 144 <= event.data[0] && event.data[0] <= 159  { // note on
                track_events.push(TrackEvent::NoteOn(NoteOn::new_with_params(MidiPolyphonicExpressionNoteId::ALL as i32, event.delta_frames as f64, event.data[1] as i32, event.data[2] as i32)));
            } 
            else if 176 <= event.data[0] && event.data[0] <= 191 { // controller
                track_events.push(TrackEvent::Controller(Controller::new(event.delta_frames as f64, event.data[1] as i32, event.data[2] as i32)));
            }
            else if 224 <= event.data[0] && event.data[0] <= 239 { // pitch bend
                track_events.push(TrackEvent::PitchBend(PitchBend::new_from_midi_bytes(event.delta_frames as f64, event.data[1], event.data[2])));
            } 
            else {
                debug!("Attempted to convert unknown VST24 event: frame={}, midi type={}", event.delta_frames , event.data[0]);
            }
        }

        track_events
    }

    pub fn convert_events_with_timing_in_frames_to_vst(daw_events: &Vec<TrackEvent>, midi_channel: i32) -> Vec<MidiEvent> {
        let mut events_all: Vec<MidiEvent> = Vec::new();

        for event in daw_events.iter() {
            match event {
                TrackEvent::Note(_note) => {
                    // can't use notes in the background processor because the duration may be outside of the block
                }
                TrackEvent::NoteOn(note_on) => {
                    let note_on = MidiEvent {
                        data: [144 + (midi_channel as u8), note_on.note() as u8, note_on.velocity() as u8],
                        delta_frames: note_on.position() as i32,
                        live: false,
                        note_length: None,
                        note_offset: None,
                        detune: 0,
                        note_off_velocity: 0,
                    };
                    events_all.push(note_on);
                }
                TrackEvent::NoteOff(note_off) => {
                    let note_off = MidiEvent {
                        data: [128 + (midi_channel as u8), note_off.note() as u8, 0],
                        delta_frames: note_off.position() as i32,
                        live: false,
                        note_length: None,
                        note_offset: None,
                        detune: 0,
                        note_off_velocity: 0,
                    };
                    events_all.push(note_off);
                }
                TrackEvent::Controller(controller) => {
                    let controller_event = MidiEvent {
                        data: [176 + (midi_channel as u8), controller.controller() as u8, controller.value() as u8],
                        delta_frames: controller.position() as i32,
                        live: false,
                        note_length: None,
                        note_offset: None,
                        detune: 0,
                        note_off_velocity: 0,
                    };
                    events_all.push(controller_event);
                }
                TrackEvent::PitchBend(pitch_bend) => {
                    let (lsb, msb) = pitch_bend.midi_bytes_from_value();
                    let pitch_bend_event = MidiEvent {
                        data: [224 + (midi_channel as u8), lsb, msb],
                        delta_frames: pitch_bend.position() as i32,
                        live: false,
                        note_length: None,
                        note_offset: None,
                        detune: 0,
                        note_off_velocity: 0,
                    };
                    events_all.push(pitch_bend_event);
                }
                TrackEvent::AudioPluginParameter(_) => {}
                TrackEvent::Sample(sample) => {
                    let note_on = MidiEvent {
                        data: [144 + (midi_channel as u8), 60, 127],
                        delta_frames: sample.position() as i32,
                        live: false,
                        note_length: None,
                        note_offset: None,
                        detune: 0,
                        note_off_velocity: 0,
                    };
                    events_all.push(note_on);
                    let note_off = MidiEvent {
                        data: [128 + (midi_channel as u8), 60, 127],
                        delta_frames: (sample.position() + 1.0) as i32,
                        live: false,
                        note_length: None,
                        note_offset: None,
                        detune: 0,
                        note_off_velocity: 0,
                    };
                    events_all.push(note_off);
                }
                _ => {}
            }
        }

        events_all.sort_by(|a, b| a.delta_frames.cmp(&b.delta_frames));
        events_all
    }

    pub fn convert_events_with_timing_in_frames_to_clap(daw_events: &Vec<TrackEvent>, midi_channel: i32) -> Vec<simple_clap_host_helper_lib::plugin::instance::process::Event> {
        let mut events_all: Vec<simple_clap_host_helper_lib::plugin::instance::process::Event> = Vec::new();

        for event in daw_events.iter() {
            match event {
                TrackEvent::Note(_note) => {
                    // can't use notes in the background processor because the duration may be outside of the block
                }
                TrackEvent::NoteOn(note_on) => {
                    let note_on_clap_event = clap_event_note {
                        header: clap_event_header {
                            size: std::mem::size_of::<clap_event_note>() as u32,
                            time: note_on.position() as u32,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_: CLAP_EVENT_NOTE_ON,
                            flags: 0,
                        },
                        note_id: note_on.note_id(),
                        port_index: note_on.port() as i16,
                        channel: note_on.channel() as i16,
                        key: note_on.note() as i16,
                        velocity: note_on.velocity() as f64,
                    };
                    events_all.push(simple_clap_host_helper_lib::plugin::instance::process::Event::Note(note_on_clap_event));
                }
                TrackEvent::NoteOff(note_off) => {
                    let note_off_clap_event = clap_event_note {
                        header: clap_event_header {
                            size: std::mem::size_of::<clap_event_note>() as u32,
                            time: note_off.position() as u32,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_: CLAP_EVENT_NOTE_OFF,
                            flags: 0,
                        },
                        note_id: note_off.note_id(),
                        port_index: note_off.port() as i16,
                        channel: note_off.channel() as i16,
                        key: note_off.note() as i16,
                        velocity: note_off.velocity() as f64,
                    };
                    events_all.push(simple_clap_host_helper_lib::plugin::instance::process::Event::Note(note_off_clap_event));
                }
                TrackEvent::NoteExpression(note_expression) => {
                    let note_expr_clap_event = clap_event_note_expression {
                        header: clap_event_header {
                            size: std::mem::size_of::<clap_event_note_expression>() as u32,
                            time: note_expression.position() as u32,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_: CLAP_EVENT_NOTE_EXPRESSION,
                            flags: 0,
                        },
                        note_id: note_expression.note_id(),
                        port_index: note_expression.port(),
                        channel: note_expression.channel(),
                        key: note_expression.key() as i16,
                        value: match note_expression.expression_type() {
                            crate::domain::NoteExpressionType::Volume => (note_expression.value() * 4.0).log10() * 20.0, // with 0 < x <= 4, plain = 20 * log(x)
                            crate::domain::NoteExpressionType::Pan => note_expression.value(),
                            crate::domain::NoteExpressionType::Tuning => note_expression.value() * 240.0 - 120.0,
                            crate::domain::NoteExpressionType::Vibrato => note_expression.value(),
                            crate::domain::NoteExpressionType::Expression => note_expression.value(),
                            crate::domain::NoteExpressionType::Pressure => note_expression.value(),
                            crate::domain::NoteExpressionType::Brightness => note_expression.value(),
                        },
                        expression_id: match note_expression.expression_type() {
                            crate::domain::NoteExpressionType::Volume => {CLAP_NOTE_EXPRESSION_VOLUME}
                            crate::domain::NoteExpressionType::Pan => {CLAP_NOTE_EXPRESSION_PAN}
                            crate::domain::NoteExpressionType::Tuning => {CLAP_NOTE_EXPRESSION_TUNING}
                            crate::domain::NoteExpressionType::Vibrato => {CLAP_NOTE_EXPRESSION_VIBRATO}
                            crate::domain::NoteExpressionType::Expression => {CLAP_NOTE_EXPRESSION_EXPRESSION}
                            crate::domain::NoteExpressionType::Pressure => {CLAP_NOTE_EXPRESSION_PRESSURE}
                            crate::domain::NoteExpressionType::Brightness => {CLAP_NOTE_EXPRESSION_BRIGHTNESS}
                        }
                    };
                    events_all.push(simple_clap_host_helper_lib::plugin::instance::process::Event::NoteExpression(note_expr_clap_event));
                }
                TrackEvent::Controller(controller) => {
                    let controller_clap_event = clap_event_midi {
                        header: clap_event_header {
                            size: std::mem::size_of::<clap_event_midi>() as u32,
                            time: controller.position() as u32,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_: CLAP_EVENT_MIDI,
                            flags: 0,
                        },
                        port_index: 0,
                        data: [176 + (midi_channel as u8), controller.controller() as u8, controller.value() as u8],
                    };
                    events_all.push(simple_clap_host_helper_lib::plugin::instance::process::Event::Midi(controller_clap_event));
                }
                TrackEvent::PitchBend(pitch_bend) => {
                    let (lsb, msb) = pitch_bend.midi_bytes_from_value();
                    let pitch_bend_clap_event = clap_event_midi {
                        header: clap_event_header {
                            size: std::mem::size_of::<clap_event_midi>() as u32,
                            time: pitch_bend.position() as u32,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_: CLAP_EVENT_MIDI,
                            flags: 0,
                        },
                        port_index: 0,
                        data: [224 + (midi_channel as u8), lsb, msb],
                    };
                    events_all.push(simple_clap_host_helper_lib::plugin::instance::process::Event::Midi(pitch_bend_clap_event));
                }
                TrackEvent::AudioPluginParameter(parameter) => {
                    let param_value = clap_event_param_value{
                        header: clap_event_header {
                            size: std::mem::size_of::<clap_event_midi>() as u32,
                            time: parameter.position() as u32,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_: CLAP_EVENT_PARAM_VALUE,
                            flags: 0,
                        },
                        param_id: parameter.index as clap_id,
                        value: parameter.value as f64,
                        channel: -1,
                        port_index: 1,
                        key: -1,
                        note_id: -1,
                        cookie: std::ptr::null_mut(),
                    };
                    events_all.push(simple_clap_host_helper_lib::plugin::instance::process::Event::ParamValue(param_value));
                }
                TrackEvent::Sample(_) => {}
                _ => {}
            }
        }

        // events_all.sort_by(|a, b| a.delta_frames.cmp(&b.delta_frames));
        events_all
    }

    pub fn convert_param_events_with_timing_in_frames_to_clap(plugin_param_events: &Vec<&PluginParameter>, midi_channel: i32, param_info: &simple_clap_host_helper_lib::plugin::ext::params::ParamInfo) -> Vec<simple_clap_host_helper_lib::plugin::instance::process::Event> {
        let mut events_all: Vec<simple_clap_host_helper_lib::plugin::instance::process::Event> = Vec::new();

        for event in plugin_param_events.iter() {
            debug!("Plugin parameter value: {}", event.value);
            if let Some(param) = param_info.get(&(event.index as u32)) {
                let param_value = event.value as f64 * (param.range.end() - param.range.start());
                debug!("Plugin parameter info: original value={}, value={}, start={}, end={}", event.value, param_value, param.range.start(), param.range.end());
                let clap_event = clap_event_param_value {
                    header: clap_event_header {
                        size: std::mem::size_of::<clap_event_param_value>() as u32,
                        time: 0,
                        space_id: CLAP_CORE_EVENT_SPACE_ID,
                        type_: CLAP_EVENT_PARAM_VALUE,
                        flags: 0,
                    },
                    param_id: event.index as u32,
                    cookie: param.cookie,
                    note_id: -1,
                    port_index: 0,
                    channel: -1,
                    key: -1,
                    value: param_value,
                };
                events_all.push(simple_clap_host_helper_lib::plugin::instance::process::Event::ParamValue(clap_event));
            }
        }

        // events_all.sort_by(|a, b| a.delta_frames.cmp(&b.delta_frames));
        events_all
    }

    pub fn extract_riff_ref_events(riffs: &Vec<Riff>, riff_refs: &Vec<RiffReference>, bpm: f64, sample_rate: f64, _midi_channel: i32) -> Vec<TrackEvent> {
        let mut events_all: Vec<TrackEvent> = Vec::new();

        for riff_ref in riff_refs {
            for riff in riffs.iter() {
                if riff.uuid().to_string() == riff_ref.linked_to() {
                    debug!("util-extract_riff_ref_events: riff name={}", riff.name());
                    let mut use_notes = match riff_ref.mode() {
                        RiffReferenceMode::Normal => true,
                        RiffReferenceMode::Start => false,
                        RiffReferenceMode::End => true,
                    };
                    for event in riff.events().iter() {
                        if let TrackEvent::Note(note) = event {
                            use_notes = match &riff_ref.mode() {
                                RiffReferenceMode::Start => {
                                    if !use_notes && note.riff_start_note() { true }
                                    else if use_notes { true }
                                    else { false }
                                }
                                RiffReferenceMode::End => {
                                    if use_notes && note.riff_start_note() { false }
                                    else if !use_notes { false }
                                    else { true }
                                }
                                RiffReferenceMode::Normal => true,
                            };

                            if use_notes {
                                // create a note on and a note off event
                                let note_on = NoteOn::new_with_params(
                                    note.note_id(),
                                    (riff_ref.position() + note.position()) / bpm * 60.0 * sample_rate,
                                    note.note(),
                                    note.velocity());
                                events_all.push(TrackEvent::NoteOn(note_on));
                                let note_off = NoteOff::new_with_params(
                                    note.note_id(),
                                    (riff_ref.position() + note.position() + note.length()) / bpm * 60.0 * sample_rate,
                                    note.note(),
                                    note.velocity());
                                events_all.push(TrackEvent::NoteOff(note_off));
                            }
                        }
                        else {
                            let mut cloned_track_event = event.clone();
                            cloned_track_event.set_position((riff_ref.position() + event.position()) / bpm * 60.0 * sample_rate);
                            events_all.push(cloned_track_event);
                        }
                    }

                    // add the measure boundary markers
                    let number_of_measures = (riff.length() / 4.0) as i32; // TODO need to pass through the beats per bar
                    for measure_number in 0..number_of_measures {
                        let measure_boundary_marker = Measure::new((riff_ref.position() + ((measure_number + 1) * 4) as f64) / bpm * 60.0 * sample_rate);
                        events_all.push(TrackEvent::Measure(measure_boundary_marker));

                        debug!("^^^^^^^^^^^^^^^^^^^^^^ added a measure boundary");
                    }

                    // somehow add the full play out loop point marker


                    break;
                }
            }
        }

        events_all.sort_by(&DAWUtils::sort_track_events);

        events_all
    }

    pub fn constant_power_stereo_pan(pan: f32) -> (f32 /* left */, f32 /* right */) {
        let pi_over_2: f32 = 4.0 * 1.0_f32.atan() * 0.5;
        let root_2_over_2: f32 = 2.0_f32.sqrt() * 0.5;

        let current_position: f32 = pan * pi_over_2;
        let angle: f32 = current_position * 0.5;

        (
            root_2_over_2 * (angle.cos() - angle.sin()), // left
            root_2_over_2 * (angle.cos() + angle.sin())  // right
        )
    }

    pub fn copy_riff_set_to_position(uuid: String, position_in_beats: f64, state: Arc<Mutex<DAWState>>) -> f64 {
        match state.lock() {
            Ok(mut state) => {
                // find the riff set
                let riff_set = state.get_project().song_mut().riff_sets_mut().iter_mut().find(|riff_set| riff_set.uuid() == uuid).map(|riff_set| riff_set.clone());

                if let Some(riff_set) = riff_set {
                    let mut riff_lengths = vec![];
                    for track_type in state.get_project().song_mut().tracks_mut().iter_mut() {
                        if let Some(riff_ref) = riff_set.riff_refs().get(&track_type.uuid().to_string()) {
                            if let Some(riff) = track_type.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == riff_ref.linked_to()) {
                                if riff.name() != "empty" {
                                    riff_lengths.push(riff.length() as i32);
                                }
                            }
                        }
                    }
                    let (product, unique_riff_lengths) = DAWState::get_length_product(riff_lengths);
                    let lowest_common_factor_in_beats = DAWState::get_lowest_common_factor(unique_riff_lengths, product);
                    for track_type in state.get_project().song_mut().tracks_mut().iter_mut() {
                        if let Some(riff_ref) = riff_set.riff_refs().get(&track_type.uuid().to_string()) {
                            if let Some(riff) = track_type.riffs_mut().iter_mut().find(|riff| riff.uuid().to_string() == riff_ref.linked_to()) {
                                let riff_length = riff.length();
                                if riff.name() != "empty" {
                                    let repeats = lowest_common_factor_in_beats / riff_length as i32;
                                    for index in 0..repeats {
                                        let mut riff_ref_copy = RiffReference::new(riff_ref.linked_to(), riff_ref.position());

                                        riff_ref_copy.set_position(position_in_beats + (riff_length * (index as f64)));
                                        track_type.riff_refs_mut().push(riff_ref_copy);
                                    }
                                }
                            }
                        }
                    }

                    position_in_beats + lowest_common_factor_in_beats as f64
                }
                else {
                    0.0
                }
            }
            Err(_) => 0.0
        }
    }

    pub fn copy_riff_sequence_to_position(uuid: String, position_in_beats: f64, state: Arc<Mutex<DAWState>>) -> f64 {
        let mut riff_set_references = vec![];
        match state.lock() {
            Ok(state) => {
                if let Some(riff_sequence) = state.project().song().riff_sequence(uuid) {
                    for riff_set_uuid in riff_sequence.riff_sets() {
                        riff_set_references.push(riff_set_uuid.clone());
                    }
                }
            }
            Err(_) => {}
        }

        let mut running_position_in_beats = position_in_beats;
        for riff_set_reference in riff_set_references.iter() {
            running_position_in_beats = DAWUtils::copy_riff_set_to_position(riff_set_reference.item_uuid().to_string(), running_position_in_beats, state.clone());
        }

        running_position_in_beats
    }

    pub fn get_riff_grid_length(riff_grid: &RiffGrid, state: &DAWState) -> f64 {
        let mut riff_grid_actual_play_length = 0.0;
        for track_uuid in riff_grid.tracks() {
            if let Some(track) =  state.project().song().track(track_uuid.clone()) {
                for track_riff_references in riff_grid.track_riff_references(track_uuid.clone()).iter() {
                    for riff_ref in track_riff_references.iter() {
                        if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_ref.linked_to()) {
                            let length = riff_ref.position() + riff.length();
                            if length > riff_grid_actual_play_length {
                                riff_grid_actual_play_length = length;
                            }
                        }
                    }
                }
            }
        }

        riff_grid_actual_play_length
    }

    pub fn copy_riff_grid_to_position(uuid: String, position_in_beats: f64, state: Arc<Mutex<DAWState>>) -> f64 {
        let mut tracks_riff_refs = HashMap::new();
        let mut riff_grid_length = 0.0;
        match state.lock() {
            Ok(state) => {
                if let Some(riff_grid) = state.project().song().riff_grid(uuid) {
                    for track_uuid in riff_grid.tracks() {
                        let mut riff_references = vec![];
                        for track_riff_ref in riff_grid.track_riff_references(track_uuid.clone()).unwrap().iter() {
                            let mut reference = track_riff_ref.clone();
                            reference.set_position(position_in_beats + track_riff_ref.position());
                            riff_references.push(reference);

                            // get the end position of the riff grid track
                            if let Some(track) = state.project().song().track(track_uuid.clone()) {
                                if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == track_riff_ref.linked_to()) {
                                    let riff_grid_track_end_position = track_riff_ref.position() + riff.length();

                                    if riff_grid_track_end_position > riff_grid_length {
                                        riff_grid_length = riff_grid_track_end_position;
                                    }
                                }
                            }
                        }
                        tracks_riff_refs.insert(track_uuid.clone(), riff_references);
                    }
                }
            }
            Err(_) => {}
        }

        match state.lock() {
            Ok(mut state) => {
                for track in state.get_project().song_mut().tracks_mut() {
                    if let Some(track_riff_refs) = tracks_riff_refs.get_mut(&track.uuid().to_string()) {
                        for track_riff_ref in track_riff_refs.iter() {
                            let riff_ref = RiffReference::new(track_riff_ref.linked_to(), track_riff_ref.position());
                            track.riff_refs_mut().push(riff_ref);
                        }
                        track.riff_refs_mut().sort_by(|a, b| DAWUtils::sort_by_daw_position(a, b));
                    }
                }
            }
            Err(_) => {}
        }

        position_in_beats + riff_grid_length
    }

    pub fn copy_riff_arrangement_to_position(uuid: String, position_in_beats: f64, state: Arc<Mutex<DAWState>>) {
        struct ArrangementElement {
            uuid: String,
            element_type: RiffItemType,
        }
        let mut arrangement_elements = vec![];
        match state.lock() {
            Ok(state) => {
                if let Some(riff_arrangement) = state.project().song().riff_arrangement(uuid) {
                    for item in riff_arrangement.items() {
                        arrangement_elements.push(ArrangementElement {
                            uuid: item.item_uuid().to_string(),
                            element_type: item.item_type().clone()
                        });
                    }
                }
            }
            Err(_) => {}
        }

        let mut running_position_in_beats = position_in_beats;
        for element in arrangement_elements.iter() {
            match element.element_type {
                RiffItemType::RiffSet => {
                    running_position_in_beats = DAWUtils::copy_riff_set_to_position(element.uuid.clone(), running_position_in_beats, state.clone());
                }
                RiffItemType::RiffSequence => {
                    running_position_in_beats = DAWUtils::copy_riff_sequence_to_position(element.uuid.clone(), running_position_in_beats, state.clone());
                }
                RiffItemType::RiffGrid => {
                    running_position_in_beats = DAWUtils::copy_riff_grid_to_position(element.uuid.clone(), running_position_in_beats, state.clone());
                }
            }
        }
    }

    pub fn parse_midi_routing_id(midi_routing_id: String, description: String) -> Option<TrackEventRouting> {
        // tokenise the id
        let ids: Vec<&str> = midi_routing_id.split(":").collect();

        let source_track_uuid = ids.get(0).unwrap();
        let source_item_key = ids.get(1).unwrap();
        let source_item_key_parts: Vec<&str> = source_item_key.split(";").collect();
        let source_item_type = source_item_key_parts.get(0).unwrap();
        let source_item_uuid = if let Some(source_item_uuid) = source_item_key_parts.get(1) {
            source_item_uuid.to_string()
        }
        else {
            "".to_string()
        };

        let destination_track_uuid = ids.get(2).unwrap();
        let destination_item_key = ids.get(3).unwrap();
        let destination_item_key_parts: Vec<&str> = destination_item_key.split(";").collect();
        let destination_item_type = destination_item_key_parts.get(0).unwrap();
        let destination_item_uuid = if let Some(destination_item_uuid) = destination_item_key_parts.get(1) {
            destination_item_uuid.to_string()
        }
        else {
            "".to_string()
        };

        let source = match *source_item_type {
            "instrument" => Some(TrackEventRoutingNodeType::Instrument(source_track_uuid.to_string(), source_item_uuid)),
            "effect" => Some(TrackEventRoutingNodeType::Effect(source_track_uuid.to_string(), source_item_uuid)),
            "none" => Some(TrackEventRoutingNodeType::Track(source_track_uuid.to_string())),
            _ => None,
        };
        let destination = match *destination_item_type {
            "instrument" => Some(TrackEventRoutingNodeType::Instrument(destination_track_uuid.to_string(), destination_item_uuid)),
            "effect" => Some(TrackEventRoutingNodeType::Effect(destination_track_uuid.to_string(), destination_item_uuid)),
            "none" => Some(TrackEventRoutingNodeType::Track(source_track_uuid.to_string())),
            _ => None,
        };

        if let Some(source) = source {
            if let Some(destination) = destination {
                Some(TrackEventRouting::new(description, source, destination))
            }
            else {
                None
            }
        }
        else {
            None
        }
    }

    pub fn parse_audio_routing_id(audio_routing_id: String, description: String) -> Option<AudioRouting> {
        // tokenise the id
        let ids: Vec<&str> = audio_routing_id.split(":").collect();

        let source_track_uuid = ids.get(0).unwrap();
        let source_item_key = ids.get(1).unwrap();
        let source_item_key_parts: Vec<&str> = source_item_key.split(";").collect();
        let source_item_type = source_item_key_parts.get(0).unwrap();
        let source_item_uuid = if let Some(source_item_uuid) = source_item_key_parts.get(1) {
            source_item_uuid.to_string()
        }
        else {
            "".to_string()
        };

        let destination_track_uuid = ids.get(2).unwrap();
        let destination_item_key = ids.get(3).unwrap();
        let destination_item_key_parts: Vec<&str> = destination_item_key.split(";").collect();
        let destination_item_type = destination_item_key_parts.get(0).unwrap();
        let destination_item_uuid = if let Some(destination_item_uuid) = destination_item_key_parts.get(1) {
            destination_item_uuid.to_string()
        }
        else {
            "".to_string()
        };

        let source = match *source_item_type {
            "instrument" => Some(AudioRoutingNodeType::Instrument(source_track_uuid.to_string(), source_item_uuid, 0, 1)),
            "effect" => Some(AudioRoutingNodeType::Effect(source_track_uuid.to_string(), source_item_uuid, 0, 1)),
            "none" => Some(AudioRoutingNodeType::Track(source_track_uuid.to_string())),
            _ => None,
        };
        let destination = match *destination_item_type {
            "instrument" => Some(AudioRoutingNodeType::Instrument(destination_track_uuid.to_string(), destination_item_uuid, 2, 3)),
            "effect" => Some(AudioRoutingNodeType::Effect(destination_track_uuid.to_string(), destination_item_uuid, 2, 3)),
            "none" => Some(AudioRoutingNodeType::Track(destination_track_uuid.to_string())),
            _ => None,
        };

        if let Some(source) = source {
            if let Some(destination) = destination {
                Some(AudioRouting::new(description, source, destination))
            }
            else {
                None
            }
        }
        else {
            None
        }
    }
}


#[cfg(test)]
mod tests {
    use uuid::Uuid;
    use log::*;

    use crate::DAWUtils;
    // use {DAWEventPosition, Riff, RiffReference, Track, TrackEvent, VstPluginParameter};
    use crate::domain::{Automation, AutomationEnvelope, DAWItemPosition, Note, PluginParameter, Riff, RiffReference, TrackEvent};
    use crate::event::TranslationEntityType::AudioPluginParameter;
    use crate::state::MidiPolyphonicExpressionNoteId;

    #[test]
    fn riff_sequence_convert_to_vst_events_one_measure_gap_before_first_note() {
        let bpm = 140.0;
        let sample_rate = 44100.0;
        let block_size = 1024.0;
        let song_length_in_beats = 10.0;
        let automation = Automation::new();
        let mut riffs: Vec<Riff> = vec![];
        let mut riff_refs: Vec<RiffReference> = vec![];

        // create a riff
        let mut riff = Riff::new_with_name_and_length(Uuid::new_v4(), "test".to_string(), 4.0);
        for position in 0..4 {
            let note = Note::new_with_params(MidiPolyphonicExpressionNoteId::ALL as i32, position as f64, 60, 127, 0.05357142857142857);
            riff.events_mut().push(TrackEvent::Note(note));
        }

        // create a riff ref that does not state at position 0
        let mut riff_ref = RiffReference::new(Uuid::new_v4().to_string(), 5.0);
        riff_ref.set_linked_to(riff.uuid().to_string());

        riffs.push(riff);
        riff_refs.push(riff_ref);

        // do the conversion
        let (event_blocks, _param_event_blocks) =
            DAWUtils::convert_to_event_blocks(&automation, &riffs, &riff_refs, bpm, block_size, sample_rate, song_length_in_beats, 0, true);

        // calculate how many blocks are expected
        let expected_blocks = ((sample_rate * 60.0 /* secs */ * song_length_in_beats) / (block_size * bpm)).round() as usize;

        // check the number of blocks received
        assert_eq!(expected_blocks, event_blocks.len());

        // find which block the first vst event appears in
        let first_event_found_block_number_option = event_blocks.iter().position(|block| !block.is_empty());

        let first_event_found_block_number = if let Some(first_event_found_block_number) = first_event_found_block_number_option {
            first_event_found_block_number as i32
        }
        else {
            -1
        };

        assert_ne!(-1, first_event_found_block_number);
        assert_ne!(0, first_event_found_block_number);

        // calculate the expected frame position of the first event
        let expected_first_event_frame_position = ((5.0 /* riff position in beats */ + 0.0 /* note position in beats */) / bpm * 60.0 * sample_rate) as i32;
        let expected_frames_into_block = expected_first_event_frame_position % (block_size as i32);
        let expected_block_number = expected_first_event_frame_position / (block_size as i32);
        let expected_block_number2 = ((sample_rate * 60.0 /* secs */ * 5.0) / (block_size * bpm)).round() as i32;

        assert_eq!(expected_block_number, expected_block_number2);
        assert_eq!(expected_block_number, first_event_found_block_number);
        assert_eq!(expected_frames_into_block, event_blocks.get(first_event_found_block_number as usize).unwrap().get(0_usize).unwrap().position() as i32);

        let mut number_of_found_events = 0;
        for block in event_blocks.iter() {
            number_of_found_events += block.len() as i32;
        }
        assert_eq!(8 + 1 /* measure end */, number_of_found_events);
    }

    #[test]
    fn convert_riff_ref_events_to_vst_events_one_measure_gap_before_first_note() {
        let bpm = 140.0;
        let sample_rate = 44100.0;
        let _block_size = 1024.0;
        let _song_length_in_beats = 10.0;
        let _automation: Vec<TrackEvent> = vec![];
        let mut riffs: Vec<Riff> = vec![];
        let mut riff_refs: Vec<RiffReference> = vec![];

        // create a riff
        let mut riff = Riff::new_with_name_and_length(Uuid::new_v4(), "test".to_string(), 4.0);
        for position in 0..4 {
            let note = Note::new_with_params(MidiPolyphonicExpressionNoteId::ALL as i32, position as f64, 60, 127, 0.05357142857142857);
            riff.events_mut().push(TrackEvent::Note(note));
        }

        // create a riff ref that does not start at position 0
        let mut riff_ref = RiffReference::new(Uuid::new_v4().to_string(), 5.0);
        riff_ref.set_linked_to(riff.uuid().to_string());

        riffs.push(riff);
        riff_refs.push(riff_ref);

        let midi_events = DAWUtils::convert_riff_ref_events_to_vst(&riffs, &riff_refs, bpm, sample_rate, 0);

        assert_eq!(8 + 1 /* measure end */, midi_events.len() as i32);

        // calculate the expected frame position of the first event
        let expected_first_event_frame_position = ((5.0 /* riff position in beats */ + 0.0 /* note position in beats */) / bpm * 60.0 * sample_rate) as i32;

        assert_eq!(expected_first_event_frame_position, midi_events.get(0_usize).unwrap().delta_frames);
    }

    #[test]
    fn convert_riff_ref_events_to_vst_events_one_measure_bass_line() {
        let bpm = 140.0;
        let sample_rate = 44100.0;
        let _block_size = 1024.0;
        let _song_length_in_beats = 10.0;
        let _automation: Vec<TrackEvent> = vec![];
        let mut riffs: Vec<Riff> = vec![];
        let mut riff_refs: Vec<RiffReference> = vec![];

        // create a riff
        let mut riff = Riff::new_with_name_and_length(Uuid::new_v4(), "Bass line 1 - 1".to_string(), 4.0);
        let riff_uuid = riff.uuid().to_string();
        let note1 = Note::new_with_params(MidiPolyphonicExpressionNoteId::ALL as i32, 0.0, 44, 71, 0.45535714285714285);
        riff.events_mut().push(TrackEvent::Note(note1));
        let note2 = Note::new_with_params(MidiPolyphonicExpressionNoteId::ALL as i32, 0.5, 46, 27, 0.24107142857142855);
        riff.events_mut().push(TrackEvent::Note(note2));
        let note3 = Note::new_with_params(MidiPolyphonicExpressionNoteId::ALL as i32, 1.0, 46, 68, 0.42857142857142855);
        riff.events_mut().push(TrackEvent::Note(note3));
        let note4 = Note::new_with_params(MidiPolyphonicExpressionNoteId::ALL as i32, 1.5, 36, 41, 0.45535714285714285);
        riff.events_mut().push(TrackEvent::Note(note4));
        let note5 = Note::new_with_params(MidiPolyphonicExpressionNoteId::ALL as i32, 2.5, 46, 36, 0.45535714285714285);
        riff.events_mut().push(TrackEvent::Note(note5));
        let note6 = Note::new_with_params(MidiPolyphonicExpressionNoteId::ALL as i32, 3.0, 36, 45, 0.45535714285714285);
        riff.events_mut().push(TrackEvent::Note(note6));
        riffs.push(riff);

        // create riff refs
        let mut position = 0.0;
        for _x in 0..24 {
            let mut riff_ref = RiffReference::new(Uuid::new_v4().to_string(), position);
            riff_ref.set_linked_to(riff_uuid.clone());
            riff_refs.push(riff_ref);
            position += 4.0;
        }

        let midi_events = DAWUtils::convert_riff_ref_events_to_vst(&riffs, &riff_refs, bpm, sample_rate, 0);

        assert_eq!(24 * 6 * 2 + 24 /* measure ends */, midi_events.len() as i32);

        let mut event_index = 1;
        for event in midi_events.iter() {
            debug!("Event: index={}, frame={}, data[0]={}, data[1]={}, data[2]={}", event_index, event.delta_frames, event.data[0], event.data[1], event.data[2]);
            if event_index == 13 {
                event_index = 1;
            }
            else {
                event_index += 1;
            }
        }

        assert_eq!(0, midi_events.get(0_usize).unwrap().delta_frames);
    }

    #[test]
    fn envelope_interpolation_positive_slope() {
        let event_details = PluginParameter {
            id: Default::default(),
            index: 0,
            position: 0.0,
            value: 0.0,
            instrument: false,
            plugin_uuid: Default::default(),
        };
        let mut automation_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
        let bpm = 140.0;
        let sample_rate = 44100.0;
        let block_size_in_samples = 1024.0;
        let mut events_all: Vec<TrackEvent> = vec![];
        let passage_length_in_frames = sample_rate * 10.0 /* seconds */;

        // add 2 points to the envelope - positive slope
        let envelope_point_1 = PluginParameter {
            id: Default::default(),
            index: 0,
            position: 0.0,
            value: 0.0,
            instrument: false,
            plugin_uuid: Default::default(),
        };
        automation_envelope.events_mut().push(TrackEvent::AudioPluginParameter(envelope_point_1));
        let envelope_point_2 = PluginParameter {
            id: Default::default(),
            index: 0,
            position: passage_length_in_frames / sample_rate * bpm / 60.0,
            value: 1.0,
            instrument: false,
            plugin_uuid: Default::default(),
        };
        automation_envelope.events_mut().push(TrackEvent::AudioPluginParameter(envelope_point_2));
        let automation_envelopes: Vec<AutomationEnvelope> = vec![automation_envelope];

        let param_events = DAWUtils::convert_automation_envelope_events(&automation_envelopes, bpm, sample_rate, block_size_in_samples, &mut events_all, passage_length_in_frames);
        assert_eq!((passage_length_in_frames / block_size_in_samples) as usize, param_events.len());
        let mut previous_value = None;
        for param_event in param_events.iter() {
            println!("position={}, value={}", param_event.position, param_event.value);

            if let Some(value) = previous_value {
                assert!(value < param_event.value);
            }

            previous_value = Some(param_event.value);
        }
    }

    #[test]
    fn envelope_interpolation_positive_slope_y_offset() {
        let event_details = PluginParameter {
            id: Default::default(),
            index: 0,
            position: 0.0,
            value: 0.0,
            instrument: false,
            plugin_uuid: Default::default(),
        };
        let mut automation_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
        let bpm = 140.0;
        let sample_rate = 44100.0;
        let block_size_in_samples = 1024.0;
        let mut events_all: Vec<TrackEvent> = vec![];
        let passage_length_in_frames = sample_rate * 10.0 /* seconds */;

        // add 2 points to the envelope - positive slope
        let envelope_point_1 = PluginParameter {
            id: Default::default(),
            index: 0,
            position: 0.0,
            value: 0.2,
            instrument: false,
            plugin_uuid: Default::default(),
        };
        automation_envelope.events_mut().push(TrackEvent::AudioPluginParameter(envelope_point_1));
        let envelope_point_2 = PluginParameter {
            id: Default::default(),
            index: 0,
            position: passage_length_in_frames / sample_rate * bpm / 60.0,
            value: 0.8,
            instrument: false,
            plugin_uuid: Default::default(),
        };
        automation_envelope.events_mut().push(TrackEvent::AudioPluginParameter(envelope_point_2));
        let automation_envelopes: Vec<AutomationEnvelope> = vec![automation_envelope];

        let param_events = DAWUtils::convert_automation_envelope_events(&automation_envelopes, bpm, sample_rate, block_size_in_samples, &mut events_all, passage_length_in_frames);
        assert_eq!((passage_length_in_frames / block_size_in_samples) as usize, param_events.len());
        let mut previous_value = None;
        for param_event in param_events.iter() {
            println!("position={}, value={}", param_event.position, param_event.value);

            if let Some(value) = previous_value {
                assert!(value < param_event.value);
            }

            previous_value = Some(param_event.value);
        }
    }

    #[test]
    fn envelope_interpolation_values_after_last_env_point() {
        let event_details = PluginParameter {
            id: Default::default(),
            index: 0,
            position: 0.0,
            value: 0.0,
            instrument: true,
            plugin_uuid: Default::default(),
        };
        let mut automation_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
        let bpm = 140.0;
        let sample_rate = 44100.0;
        let block_size_in_samples = 1024.0;
        let mut events_all: Vec<TrackEvent> = vec![];
        let passage_length_in_frames = sample_rate * 10.0 /* seconds */;

        // add 2 points to the envelope - positive slope
        let envelope_point_1 = PluginParameter {
            id: Default::default(),
            index: 0,
            position: 0.0,
            value: 0.0,
            instrument: false,
            plugin_uuid: Default::default(),
        };
        automation_envelope.events_mut().push(TrackEvent::AudioPluginParameter(envelope_point_1));
        let position_quarter_way = passage_length_in_frames / sample_rate * bpm / 60.0 / 4.0;
        let envelope_point_2 = PluginParameter {
            id: Default::default(),
            index: 0,
            position: position_quarter_way,
            value: 0.2,
            instrument: false,
            plugin_uuid: Default::default(),
        };
        automation_envelope.events_mut().push(TrackEvent::AudioPluginParameter(envelope_point_2));
        let position_half_way = passage_length_in_frames / sample_rate * bpm / 60.0 / 2.0;
        let envelope_point_3 = PluginParameter {
            id: Default::default(),
            index: 0,
            position: position_half_way,
            value: 0.8,
            instrument: false,
            plugin_uuid: Default::default(),
        };
        automation_envelope.events_mut().push(TrackEvent::AudioPluginParameter(envelope_point_3));
        let automation_envelopes: Vec<AutomationEnvelope> = vec![automation_envelope];

        let param_events = DAWUtils::convert_automation_envelope_events(&automation_envelopes, bpm, sample_rate, block_size_in_samples, &mut events_all, passage_length_in_frames);
        // assert_eq!((passage_length_in_frames / block_size_in_samples) as usize, param_events.len());
        let mut previous_value = None;
        for param_event in param_events.iter() {
            println!("half way={}, quarter way={}, position={}, value={}", passage_length_in_frames / 2.0, passage_length_in_frames / 4.0, param_event.position, param_event.value);

            if let Some(value) = previous_value {
                if param_event.position > 221184.0 {
                    assert_eq!(value, param_event.value);
                }
                else {
                    assert!(value < param_event.value);
                }
            }

            previous_value = Some(param_event.value);
        }
    }

    #[test]
    fn envelope_interpolation_negative_slope() {
        let event_details = PluginParameter {
            id: Default::default(),
            index: 0,
            position: 0.0,
            value: 0.0,
            instrument: false,
            plugin_uuid: Default::default(),
        };
        let mut automation_envelope = AutomationEnvelope::new(TrackEvent::AudioPluginParameter(event_details));
        let bpm = 140.0;
        let sample_rate = 44100.0;
        let block_size_in_samples = 1024.0;
        let mut events_all: Vec<TrackEvent> = vec![];
        let passage_length_in_frames = sample_rate * 10.0 /* seconds */;

        // add 2 points to the envelope - positive slope
        let envelope_point_1 = PluginParameter {
            id: Default::default(),
            index: 0,
            position: 0.0,
            value: 1.0,
            instrument: false,
            plugin_uuid: Default::default(),
        };
        automation_envelope.events_mut().push(TrackEvent::AudioPluginParameter(envelope_point_1));
        let envelope_point_2 = PluginParameter {
            id: Default::default(),
            index: 0,
            position: passage_length_in_frames / sample_rate * bpm / 60.0,
            value: 0.0,
            instrument: false,
            plugin_uuid: Default::default(),
        };
        automation_envelope.events_mut().push(TrackEvent::AudioPluginParameter(envelope_point_2));
        let automation_envelopes: Vec<AutomationEnvelope> = vec![automation_envelope];

        let param_events = DAWUtils::convert_automation_envelope_events(&automation_envelopes, bpm, sample_rate, block_size_in_samples, &mut events_all, passage_length_in_frames);
        assert_eq!((passage_length_in_frames / block_size_in_samples) as usize, param_events.len());
        let mut previous_value = None;
        for param_event in param_events.iter() {
            println!("position={}, value={}", param_event.position, param_event.value);

            if let Some(value) = previous_value {
                assert!(value > param_event.value);
            }

            previous_value = Some(param_event.value);
        }
    }
}