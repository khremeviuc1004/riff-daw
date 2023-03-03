use std::iter::Iterator;
use std::sync::{Arc, Mutex, MutexGuard};

use log::*;

use crate::domain::DAWItemLength;
use crate::{DAWItemPosition, DAWState, Note, PlayMode, Track, TrackEvent};
use crate::event::{TranslateDirection, TranslationEntityType};

pub trait HistoryAction {
    fn execute(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<(), String>;
    fn undo(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<(), String>;

    fn get_selected_track_riff_uuid(&self, state: &mut Arc<Mutex<DAWState>>) -> (Option<String>, Option<String>) {
        let mut selected_riff_uuid = None;
        let mut selected_riff_track_uuid = None;

        match state.lock() {
            Ok(state) => {
                selected_riff_track_uuid = state.selected_track();

                match selected_riff_track_uuid {
                    Some(track_uuid) => {
                        selected_riff_uuid = state.selected_riff_uuid(track_uuid.clone());
                        selected_riff_track_uuid = Some(track_uuid);
                    },
                    None => (),
                }
            },
            Err(_) => info!("could not get lock on state"),
        };
        (selected_riff_uuid, selected_riff_track_uuid)
    }

    fn check_riff_changed_and_playing(&self, riff_uuid: String, state: &mut MutexGuard<DAWState>, track_uuid: String, playing: bool, play_mode: PlayMode, playing_riff_set: Option<String>, riff_changed: bool) {
        if riff_changed && playing {
            self.play_riff_set_update_track(riff_uuid, state, track_uuid, play_mode, playing_riff_set)
        }
    }

    fn play_riff_set_update_track(&self, _riff_uuid: String, state: &mut MutexGuard<DAWState>, track_uuid: String, play_mode: PlayMode, playing_riff_set: Option<String>) {
        match play_mode {
            PlayMode::Song => {}
            PlayMode::RiffSet => {
                if let Some(playing_riff_set) = playing_riff_set {
                    info!("RiffSet riff updated - now calling state.play_riff_set_update_track");
                    state.play_riff_set_update_track(playing_riff_set, track_uuid);
                }
            }
            PlayMode::RiffSequence => {}
            PlayMode::RiffArrangement => {}
        }
    }

    fn check_playing(&self, riff_uuid: String, state: &mut MutexGuard<DAWState>, track_uuid: String, playing: bool, play_mode: PlayMode, playing_riff_set: Option<String>) {
        if playing {
            self.play_riff_set_update_track(riff_uuid, state, track_uuid, play_mode, playing_riff_set)
        }
    }
}

pub struct HistoryManager {
    history: Vec<Box<dyn HistoryAction>>,
    head_index: i32,
}

impl HistoryManager {
    pub fn new() -> Self {
        Self {
            history: vec![],
            head_index: -1,
        }
    }

    pub fn apply(&mut self, state: &mut Arc<Mutex<DAWState>>, mut action: Box<dyn HistoryAction>) -> Result<(), String> {
        println!("History - apply: self.history.len()={}, self.head_index={}", self.history.len(), self.head_index);
        if self.head_index >= 0 && !self.history.is_empty() && (self.head_index as usize) != (self.history.len() - 1) {
            // delete everything above the head_index
            for index in (self.history.len() - 1)..(self.head_index as usize) {
                self.history.remove(index);
            }
        }
        let result = action.execute(state);
        self.history.push(action);
        self.head_index += 1;
        result
    }

    pub fn undo(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<(), String> {
        println!("History - undo: self.history.len()={}, self.head_index={}", self.history.len(), self.head_index);
        // decrement the current top of the history
        if self.history.len() > self.head_index as usize && self.head_index >= 0 {
            if let Some(action) = self.history.get_mut(self.head_index as usize ) {
                self.head_index -= 1;
                action.undo(state)
            }
            else {
                println!("Could not find action to undo.");
                Err("Could not find action to undo.".to_string())
            }
        }
        else {
            println!("History head index greater than number of history items.");
            Err("History head index greater than number of history items.".to_string())
        }
    }

    pub fn redo(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<(), String> {
        println!("History - redo: self.history.len()={}, self.head_index={}", self.history.len(), self.head_index);
        // get the current top of the history
        if (self.head_index as usize) < (self.history.len() - 1) {
            self.head_index += 1;
            if let Some(action) = self.history.get_mut(self.head_index as usize) {
                action.execute(state)
            }
            else {
                Err("Could not find action to redo.".to_string())
            }
        }
        else {
            Err("Could not find action to redo.".to_string())
        }
    }
}

#[derive(Clone)]
pub struct RiffAddNoteAction {
    position: f64,
    note: i32,
    velocity: i32,
    duration: f64,
}

impl RiffAddNoteAction {
    pub fn new(
        position: f64,
        note: i32,
        velocity: i32,
        duration: f64,
    ) -> Self {
        Self {
            position,
            note,
            velocity,
            duration,
        }
    }
    pub fn position(&self) -> f64 {
        self.position
    }
    pub fn note(&self) -> i32 {
        self.note
    }
    pub fn velocity(&self) -> i32 {
        self.velocity
    }
    pub fn duration(&self) -> f64 {
        self.duration
    }
}

unsafe impl Send for RiffAddNoteAction {

}

impl HistoryAction for RiffAddNoteAction {
    fn execute(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<(), String> {
        let (selected_riff_uuid, selected_riff_track_uuid) = self.get_selected_track_riff_uuid(state);

        match state.lock() {
            Ok(state) => {
                let mut state = state;

                match selected_riff_track_uuid {
                    Some(track_uuid) => {
                        let playing = state.playing();
                        let play_mode = state.play_mode();
                        let playing_riff_set = state.playing_riff_set().clone();
                        let mut riff_changed = false;

                        for track in state.get_project().song_mut().tracks_mut().iter_mut() {
                            if track.uuid().to_string() == track_uuid {
                                match selected_riff_uuid.clone() {
                                    Some(riff_uuid) => {
                                        for riff in track.riffs_mut().iter_mut() {
                                            if riff.uuid().to_string() == *riff_uuid {
                                                let overlap_found = riff.events_mut().iter_mut().any(|track_event| {
                                                    if let TrackEvent::Note(note) = track_event {
                                                        let new_note_start = self.position();
                                                        let new_note_end = self.position() + self.duration();
                                                        let current_note_start = note.position();
                                                        let current_note_end = note.position() + note.length();

                                                        note.note() == self.note() && (
                                                            (current_note_start <= new_note_start && new_note_start <= current_note_end) ||
                                                            (current_note_start <= new_note_end && new_note_end <= current_note_end) ||
                                                            (new_note_start < current_note_start && current_note_end < new_note_end)
                                                        )
                                                    }
                                                    else {
                                                        false
                                                    }
                                                });
                                                if !overlap_found {
                                                    riff.events_mut().push(TrackEvent::Note(Note::new_with_params(self.position(), self.note(), 127, self.duration())));
                                                    riff.events_mut().sort_by(|a, b| a.position().partial_cmp(&b.position()).unwrap());
                                                    riff_changed = true;
                                                }
                                                break;
                                            }
                                        }
                                        self.check_riff_changed_and_playing(riff_uuid, &mut state, track_uuid, playing, play_mode, playing_riff_set, riff_changed);
                                    }
                                    None => info!("problem getting selected riff index"),
                                }

                                break;
                            }
                        }

                        if riff_changed {
                            state.dirty = true;
                        }
                    },
                    None => info!("problem getting selected riff track number"),
                };
            },
            Err(_) => info!("could not get lock on state"),
        };
        Ok(())
    }

    fn undo(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<(), String> {
        let (selected_riff_uuid, selected_riff_track_uuid) = self.get_selected_track_riff_uuid(state);

        match state.lock() {
            Ok(state) => {
                let mut state = state;

                match selected_riff_track_uuid {
                    Some(track_uuid) => {
                        let playing = state.playing();
                        let play_mode = state.play_mode();
                        let playing_riff_set = state.playing_riff_set().clone();

                        for track in state.get_project().song_mut().tracks_mut().iter_mut() {
                            if track.uuid().to_string() == track_uuid {
                                match selected_riff_uuid.clone() {
                                    Some(riff_uuid) => {
                                        for riff in track.riffs_mut().iter_mut() {
                                            if riff.uuid().to_string() == *riff_uuid {
                                                riff.events_mut().retain(|event| match event {
                                                    TrackEvent::Note(note) => !(note.note() == self.note() && note.position() == self.position() && note.velocity() == self.velocity() && note.length() == self.duration()),
                                                    _ => true,
                                                });

                                                self.check_playing(riff_uuid, &mut state, track_uuid, playing, play_mode, playing_riff_set);
                                                break;
                                            }
                                        }
                                    }
                                    None => info!("problem getting selected riff index"),
                                }
                                break;
                            }
                        }

                    },
                    None => info!("problem getting selected riff track number"),
                };
            },
            Err(_) => info!("could not get lock on state"),
        };
        Ok(())
    }
}

#[derive(Clone)]
pub struct RiffDeleteNoteAction {
    position: f64,
    note: i32,
    velocity: i32,
    duration: f64,
}

unsafe impl Send for RiffDeleteNoteAction {

}

impl RiffDeleteNoteAction {
    pub fn new(
        position: f64,
        note: i32,
    ) -> Self {
        Self {
            position,
            note,
            velocity: 0,
            duration: 0.0,
        }
    }
    pub fn position(&self) -> f64 {
        self.position
    }
    pub fn note(&self) -> i32 {
        self.note
    }
    pub fn velocity(&self) -> i32 {
        self.velocity
    }
    pub fn duration(&self) -> f64 {
        self.duration
    }
}

impl HistoryAction for RiffDeleteNoteAction {
    fn execute(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<(), String> {
        let (selected_riff_uuid, selected_riff_track_uuid) = self.get_selected_track_riff_uuid(state);

        match state.lock() {
            Ok(state) => {
                let mut state = state;

                match selected_riff_track_uuid {
                    Some(track_uuid) => {
                        let playing = state.playing();
                        let play_mode = state.play_mode();
                        let playing_riff_set = state.playing_riff_set().clone();
                        let mut riff_changed = false;

                        for track in state.get_project().song_mut().tracks_mut().iter_mut() {
                            if track.uuid().to_string() == track_uuid {
                                match selected_riff_uuid.clone() {
                                    Some(riff_uuid) => {
                                        for riff in track.riffs_mut().iter_mut() {
                                            if riff.uuid().to_string() == *riff_uuid {
                                                let note = riff.events_mut().iter_mut().find(|event| match event {
                                                    TrackEvent::Note(note) => note.note() == self.note() && note.position() <= self.position() && self.position() <= (note.position() + note.length()),
                                                    _ => false,
                                                });
                                                if let Some(event) = note {
                                                    match event {
                                                        TrackEvent::Note(note) => {
                                                            println!("delete note: position={}, note={}, velocity={}, duration={}", note.position(), note.note(), note.velocity(), note.length());
                                                            self.position = note.position();
                                                            self.velocity = note.velocity();
                                                            self.duration = note.length();
                                                            riff_changed = true;
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                                riff.events_mut().retain(|event| match event {
                                                    TrackEvent::Note(note) => !(note.note() == self.note() && note.position() <= self.position() && self.position() <= (note.position() + note.length())),
                                                    _ => true,
                                                });

                                                self.check_riff_changed_and_playing(riff_uuid, &mut state, track_uuid, playing, play_mode, playing_riff_set, true);
                                                break;
                                            }
                                        }
                                    }
                                    None => info!("problem getting selected riff index"),
                                }

                                break;
                            }
                        }

                        if riff_changed {
                            state.dirty = true;
                        }
                    },
                    None => info!("problem getting selected riff track number"),
                };
            },
            Err(_) => info!("could not get lock on state"),
        };
        Ok(())
    }

    fn undo(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<(), String> {
        let (selected_riff_uuid, selected_riff_track_uuid) = self.get_selected_track_riff_uuid(state);

        match state.lock() {
            Ok(state) => {
                let mut state = state;

                match selected_riff_track_uuid {
                    Some(track_uuid) => {
                        let playing = state.playing();
                        let play_mode = state.play_mode();
                        let playing_riff_set = state.playing_riff_set().clone();
                        let mut riff_changed = false;

                        match selected_riff_uuid.clone() {
                            Some(riff_uuid) => {
                                for track in state.get_project().song_mut().tracks_mut().iter_mut() {
                                    if track.uuid().to_string() == track_uuid {
                                        for riff in track.riffs_mut().iter_mut() {
                                            if riff.uuid().to_string() == *riff_uuid {
                                                riff.events_mut().push(TrackEvent::Note(Note::new_with_params(self.position(), self.note(), 127, self.duration())));
                                                riff_changed = true;
                                                self.check_riff_changed_and_playing(riff_uuid, &mut state, track_uuid, playing, play_mode, playing_riff_set, riff_changed);
                                                break;
                                            }
                                        }
                                        break;
                                    }
                                }
                            }
                            None => info!("problem getting selected riff index"),
                        }

                        if riff_changed {
                            state.dirty = true;
                        }
                    },
                    None => info!("problem getting selected riff track number"),
                };
            },
            Err(_) => info!("could not get lock on state"),
        };

        Ok(())
    }
}


pub struct RiffCutSelectedAction {
    x: f64,
    y: i32,
    x2: f64,
    y2: i32,
    notes: Vec<Note>,
    track_uuid: Option<String>,
    riff_uuid: Option<String>,
}

impl RiffCutSelectedAction {
    pub fn new(
        track_uuid: Option<String>,
        riff_uuid: Option<String>,
        x: f64,
        y: i32,
        x2: f64,
        y2: i32,
    ) -> Self {
        Self {
            x,
            y,
            x2,
            y2,
            notes: vec![],
            track_uuid,
            riff_uuid,
        }
    }
}

impl HistoryAction for RiffCutSelectedAction {
    fn execute(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<(), String> {
        match state.lock() {
            Ok(state) => {
                let mut state = state;

                match self.track_uuid.clone() {
                    Some(track_uuid) => {
                        let playing = state.playing();
                        let play_mode = state.play_mode();
                        let playing_riff_set = state.playing_riff_set().clone();

                        match state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                            Some(track) => {
                                match self.riff_uuid.clone() {
                                    Some(riff_uuid) => {
                                        let mut riff_changed = false;

                                        for riff in track.riffs_mut().iter_mut() {
                                            if riff.uuid().to_string() == *riff_uuid {
                                                // store the notes for an undo
                                                for track_event in riff.events_mut().iter_mut() {
                                                    if let TrackEvent::Note(note) = track_event {
                                                        if self.y <= note.note() && note.note() <= self.y2 && self.x <= note.position() && (note.position() + note.length()) <= self.x2 {
                                                            self.notes.push(note.clone());
                                                            riff_changed = true;
                                                        }
                                                    }
                                                }

                                                // remove the notes with in the window
                                                riff.events_mut().retain(|event| match event {
                                                    TrackEvent::ActiveSense => true,
                                                    TrackEvent::AfterTouch => true,
                                                    TrackEvent::ProgramChange => true,
                                                    TrackEvent::Note(note) => !(self.y <= note.note() && note.note() <= self.y2 && self.x <= note.position() && (note.position() + note.length()) <= self.x2),
                                                    TrackEvent::NoteOn(_) => true,
                                                    TrackEvent::NoteOff(_) => true,
                                                    TrackEvent::Controller(_) => true,
                                                    TrackEvent::PitchBend(_pitch_bend) => true,
                                                    TrackEvent::KeyPressure => true,
                                                    TrackEvent::AudioPluginParameter(_) => true,
                                                    TrackEvent::Sample(_sample) => true,
                                                    TrackEvent::Measure(_) => true,
                                                    TrackEvent::NoteExpression(_) => true,
                                                });
                                                break;
                                            }
                                        }

                                        self.check_riff_changed_and_playing(riff_uuid, &mut state, track_uuid, playing, play_mode, playing_riff_set, riff_changed);

                                        if riff_changed {
                                            state.dirty = true;
                                        }
                                    },
                                    None => info!("Main - rx_ui processing loop - riff cut selected notes - problem getting selected riff index"),
                                }
                            },
                            None => ()
                        }
                    },
                    None => info!("Main - rx_ui processing loop - riff cut selected notes  - problem getting selected riff track number"),
                };
            },
            Err(_) => info!("Main - rx_ui processing loop - riff cut selected notes - could not get lock on state"),
        };

        Ok(())
    }

    fn undo(&mut self, _state: &mut Arc<Mutex<DAWState>>) -> Result<(), String> {

        Ok(())
    }
}

pub struct RiffTranslateSelectedAction {
    x: f64,
    y: i32,
    x2: f64,
    y2: i32,
    track_events: Vec<TrackEvent>,
    track_uuid: Option<String>,
    riff_uuid: Option<String>,
    translation_entity_type: TranslationEntityType,
    translate_direction: TranslateDirection,
    snap_in_beats: f64,
}

impl RiffTranslateSelectedAction {
    pub fn new(
        track_uuid: Option<String>,
        riff_uuid: Option<String>,
        x: f64,
        y: i32,
        x2: f64,
        y2: i32,
        translation_entity_type: TranslationEntityType,
        translate_direction: TranslateDirection,
        snap_in_beats: f64,
    ) -> Self {
        Self {
            x,
            y,
            x2,
            y2,
            track_events: vec![],
            track_uuid,
            riff_uuid,
            translation_entity_type,
            translate_direction,
            snap_in_beats,
        }
    }
}

impl HistoryAction for RiffTranslateSelectedAction {
    fn execute(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<(), String> {
        match state.lock() {
            Ok(state) => {
                let tempo = {
                    state.project().song().tempo()
                };

                let mut state = state;
                let snap_position_in_secs = self.snap_in_beats / tempo * 60.0;

                match self.track_uuid.clone() {
                    Some(track_uuid) => {
                        let playing = state.playing();
                        let play_mode = state.play_mode();
                        let playing_riff_set = state.playing_riff_set().clone();
                        let mut riff_changed = false;

                        match state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                            Some(track) => {
                                match self.riff_uuid.clone() {
                                    Some(riff_uuid) => {
                                        for riff in track.riffs_mut().iter_mut() {
                                            if riff.uuid().to_string() == *riff_uuid {
                                                riff.events_mut().iter_mut().for_each(|event| match event {
                                                    TrackEvent::ActiveSense => {},
                                                    TrackEvent::AfterTouch => {},
                                                    TrackEvent::ProgramChange => {},
                                                    TrackEvent::Note(note) => if self.y <= note.note() && note.note() <= self.y2 && self.x <= note.position() && (note.position() + note.length()) <= self.x2 {
                                                        let mut note_number = note.note();
                                                        let mut note_position = note.position();

                                                        match self.translate_direction {
                                                            TranslateDirection::Up => {
                                                                note_number += 1;
                                                                if note_number > 127 {
                                                                    note_number = 127;
                                                                }
                                                                note.set_note(note_number);
                                                            },
                                                            TranslateDirection::Down => {
                                                                note_number -= 1;
                                                                if note_number < 0 {
                                                                    note_number = 0;
                                                                }
                                                                note.set_note(note_number);
                                                            },
                                                            TranslateDirection::Left => {
                                                                note_position -= snap_position_in_secs;
                                                                if note_position < 0.0 {
                                                                    note_position = 0.0;
                                                                }
                                                                note.set_position(note_position);
                                                            },
                                                            TranslateDirection::Right => {
                                                                note_position += snap_position_in_secs;
                                                                if note_position < 0.0 {
                                                                    note_position = 0.0;
                                                                }
                                                                note.set_position(note_position);
                                                            },
                                                        }

                                                        riff_changed = true;
                                                    }
                                                    TrackEvent::NoteOn(_) => {}
                                                    TrackEvent::NoteOff(_) => {}
                                                    TrackEvent::Controller(_) => {}
                                                    TrackEvent::PitchBend(_pitch_bend) => {}
                                                    TrackEvent::KeyPressure => {}
                                                    TrackEvent::AudioPluginParameter(_) => {}
                                                    TrackEvent::Sample(_sample) => {}
                                                    TrackEvent::Measure(_) => {}
                                                    TrackEvent::NoteExpression(_) => {}
                                                });

                                                self.check_riff_changed_and_playing(riff_uuid, &mut state, track_uuid, playing, play_mode, playing_riff_set, riff_changed);
                                                break;
                                            }
                                        }
                                    },
                                    None => info!("Main - rx_ui processing loop - riff translate selected notes - problem getting selected riff index"),
                                }
                            },
                            None => ()
                        }

                        if riff_changed {
                            state.dirty = true;
                        }
                    },
                    None => info!("Main - rx_ui processing loop - riff translate selected  - problem getting selected riff track number"),
                };
            },
            Err(_) => info!("Main - rx_ui processing loop - riff translate selected - could not get lock on state"),
        };

        Ok(())
    }

    fn undo(&mut self, _state: &mut Arc<Mutex<DAWState>>) -> Result<(), String> {

        Ok(())
    }
}

pub struct RiffChangeLengthOfSelectedAction {
    x: f64,
    y: i32,
    x2: f64,
    y2: i32,
    notes: Vec<Note>,
    track_uuid: Option<String>,
    riff_uuid: Option<String>,
    length_increment_in_beats: f64,
    lengthen: bool,
}

impl RiffChangeLengthOfSelectedAction {
    pub fn new(
        track_uuid: Option<String>,
        riff_uuid: Option<String>,
        x: f64,
        y: i32,
        x2: f64,
        y2: i32,
        length_increment_in_beats: f64,
        lengthen: bool,
    ) -> Self {
        Self {
            x,
            y,
            x2,
            y2,
            notes: vec![],
            track_uuid,
            riff_uuid,
            length_increment_in_beats,
            lengthen,
        }
    }
}

impl HistoryAction for RiffChangeLengthOfSelectedAction {
    fn execute(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<(), String> {
        match state.lock() {
            Ok(state) => {
                let tempo = {
                    state.project().song().tempo()
                };

                let mut state = state;
                let length_increment_in_secs = self.length_increment_in_beats / tempo * 60.0;

                match self.track_uuid.clone() {
                    Some(track_uuid) => {
                        let playing = state.playing();
                        let play_mode = state.play_mode();
                        let playing_riff_set = state.playing_riff_set().clone();
                        let mut riff_changed = false;

                        match state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                            Some(track) => {
                                match self.riff_uuid.clone() {
                                    Some(riff_uuid) => {
                                        for riff in track.riffs_mut().iter_mut() {
                                            if riff.uuid().to_string() == *riff_uuid {
                                                riff.events_mut().iter_mut().for_each(|event| match event {
                                                    TrackEvent::Note(note) => if self.y <= note.note() && note.note() <= self.y2 && self.x <= note.position() && (note.position() + note.length()) <= self.x2 {
                                                        let note_length = note.length();

                                                        if note_length > 0.0 {
                                                            if self.lengthen {
                                                                note.set_length(note_length + length_increment_in_secs);
                                                            }
                                                            else if (note_length - length_increment_in_secs) > 0.0 {
                                                                note.set_length(note_length - length_increment_in_secs);
                                                            }
                                                        }

                                                        riff_changed = true;
                                                    },
                                                    _ => {},
                                                });

                                                self.check_riff_changed_and_playing(riff_uuid, &mut state, track_uuid, playing, play_mode, playing_riff_set, riff_changed);
                                                break;
                                            }
                                        }
                                    },
                                    None => info!("Main - rx_ui processing loop - riff lengthen selected notes - problem getting selected riff index"),
                                }
                            },
                            None => ()
                        }

                        if riff_changed {
                            state.dirty = true;
                        }
                    },
                    None => info!("Main - rx_ui processing loop - riff lengthen selected notes - problem getting selected riff track number"),
                };
            },
            Err(_) => info!("Main - rx_ui processing loop - riff lengthen selected notes - could not get lock on state"),
        };

        Ok(())
    }

    fn undo(&mut self, _state: &mut Arc<Mutex<DAWState>>) -> Result<(), String> {

        Ok(())
    }
}

pub struct RiffPasteSelectedAction {
    edit_cursor_position_in_beats: f64,
    notes: Vec<Note>,
    track_uuid: Option<String>,
    riff_uuid: Option<String>,
}

impl RiffPasteSelectedAction {
    pub fn new(
        track_uuid: Option<String>,
        riff_uuid: Option<String>,
        edit_cursor_position_in_beats: f64,
    ) -> Self {
        Self {
            edit_cursor_position_in_beats,
            notes: vec![],
            track_uuid,
            riff_uuid,
        }
    }
}

impl HistoryAction for RiffPasteSelectedAction {
    fn execute(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<(), String> {
        match state.lock() {
            Ok(state) => {
                let mut copy_buffer: Vec<TrackEvent> = vec![];
                let mut pasted_events_buffer: Vec<Note> = vec![];

                if self.notes.is_empty() {
                    state.track_event_copy_buffer().iter().for_each(|event| copy_buffer.push(event.clone()));
                }
                else {
                    self.notes.iter().for_each(|event| copy_buffer.push(TrackEvent::Note(event.clone())));
                }

                let mut state = state;

                match self.track_uuid.as_ref() {
                    Some(track_uuid) => {
                        let playing = state.playing();
                        let play_mode = state.play_mode();
                        let playing_riff_set = state.playing_riff_set().clone();

                        match state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid.to_string()) {
                            Some(track) => {
                                match self.riff_uuid.as_ref() {
                                    Some(riff_uuid) => {
                                        let mut riff_changed = false;

                                        for riff in track.riffs_mut().iter_mut() {
                                            if riff.uuid().to_string() == *riff_uuid {
                                                copy_buffer.iter_mut().for_each(|event| {
                                                    let cloned_event = event.clone();
                                                    match cloned_event {
                                                        TrackEvent::ActiveSense => info!("TrackChangeType::RiffPasteSelectedNotes ActiveSense not yet implemented!"),
                                                        TrackEvent::AfterTouch => info!("TrackChangeType::RiffPasteSelectedNotes AfterTouch not yet implemented!"),
                                                        TrackEvent::ProgramChange => info!("TrackChangeType::RiffPasteSelectedNotes ProgramChange not yet implemented!"),
                                                        TrackEvent::Note(mut note) => {
                                                            if self.notes.is_empty() {
                                                                note.set_position(note.position() + self.edit_cursor_position_in_beats);
                                                                pasted_events_buffer.push(note.clone());
                                                            }
                                                            riff.events_mut().push(TrackEvent::Note(note));

                                                            riff_changed = true;
                                                        },
                                                        TrackEvent::NoteOn(_) => info!("TrackChangeType::RiffPasteSelectedNotes NoteOn not yet implemented!"),
                                                        TrackEvent::NoteOff(_) => info!("TrackChangeType::RiffPasteSelectedNotes NoteOff not yet implemented!"),
                                                        TrackEvent::Controller(_) => info!("TrackChangeType::RiffPasteSelectedNotes Controller not yet implemented!"),
                                                        TrackEvent::PitchBend(_pitch_bend) => info!("TrackChangeType::RiffPasteSelectedNotes PitchBend not yet implemented!"),
                                                        TrackEvent::KeyPressure => info!("TrackChangeType::RiffPasteSelectedNotes KeyPressure not yet implemented!"),
                                                        TrackEvent::AudioPluginParameter(_) => info!("TrackChangeType::RiffPasteSelectedNotes AudioPluginParameter not yet implemented!"),
                                                        TrackEvent::Sample(_sample) => info!("TrackChangeType::RiffPasteSelectedNotes Sample not yet implemented!"),
                                                        TrackEvent::Measure(_) => {}
                                                        TrackEvent::NoteExpression(_) => {}
                                                        
                                                    }
                                                });
                                                break;
                                            }
                                        }

                                        if riff_changed {
                                            for note in pasted_events_buffer.iter() {
                                                self.notes.push(note.clone());
                                            }
                                            state.dirty = true;
                                        }

                                        self.check_riff_changed_and_playing(riff_uuid.to_string(), &mut state, track_uuid.to_string(), playing, play_mode, playing_riff_set, riff_changed);
                                    },
                                    None => info!("Main - rx_ui processing loop - riff paste selected - problem getting selected riff index"),
                                }
                            },
                            None => ()
                        }
                    },
                    None => info!("Main - rx_ui processing loop - riff references paste selected  - problem getting selected riff track number"),
                };
            },
            Err(_) => info!("Main - rx_ui processing loop - riff paste selected - could not get lock on state"),
        };

        Ok(())
    }

    fn undo(&mut self, _state: &mut Arc<Mutex<DAWState>>) -> Result<(), String> {
        Ok(())
    }
}

pub struct RiffQuantiseSelectedAction {
    x: f64,
    y: i32,
    x2: f64,
    y2: i32,
    original_notes: Vec<Note>,
    quantised_notes: Vec<Note>,
    track_uuid: Option<String>,
    riff_uuid: Option<String>,
    snap_in_beats: f64,
}

impl RiffQuantiseSelectedAction {
    pub fn new(
        x: f64,
        y: i32,
        x2: f64,
        y2: i32,
        track_uuid: Option<String>,
        riff_uuid: Option<String>,
        snap_in_beats: f64,
    ) -> Self {
        Self {
            x,
            y,
            x2,
            y2,
            original_notes: vec![],
            quantised_notes: vec![],
            track_uuid,
            riff_uuid,
            snap_in_beats,
        }
    }
}

impl HistoryAction for RiffQuantiseSelectedAction {
    fn execute(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<(), String> {
        match state.lock() {
            Ok(state) => {
                let mut state = state;

                match self.track_uuid.as_ref() {
                    Some(track_uuid) => {
                        let playing = state.playing();
                        let play_mode = state.play_mode();
                        let playing_riff_set = state.playing_riff_set().clone();

                        match state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == *track_uuid) {
                            Some(track) => {
                                match self.riff_uuid.as_ref() {
                                    Some(riff_uuid) => {
                                        let mut riff_changed = false;

                                        for riff in track.riffs_mut().iter_mut() {
                                            if riff.uuid().to_string() == *riff_uuid {
                                                riff.events_mut().iter_mut().for_each(|event| match event {
                                                    TrackEvent::ActiveSense => {},
                                                    TrackEvent::AfterTouch => {},
                                                    TrackEvent::ProgramChange => {},
                                                    TrackEvent::Note(note) => {
                                                        let lower_y = if self.y < self.y2 {
                                                            self.y
                                                        }
                                                        else {
                                                            self.y2
                                                        };
                                                        let upper_y = if self.y > self.y2 {
                                                            self.y
                                                        }
                                                        else {
                                                            self.y2
                                                        };
                                                        let lower_x = if self.x < self.x2 {
                                                            self.x
                                                        }
                                                        else {
                                                            self.x2
                                                        };
                                                        let upper_x = if self.x > self.x2 {
                                                            self.x
                                                        }
                                                        else {
                                                            self.x2
                                                        };
                                                        if lower_y <= note.note() && note.note() <= upper_y && lower_x <= note.position() && (note.position() + note.length()) <= upper_x {
                                                            let note_position = note.position();

                                                            if note_position > 0.0 {
                                                                let snap_delta = note_position % self.snap_in_beats;
                                                                if (note_position - snap_delta) >= 0.0 {
                                                                    note.set_position(note_position - snap_delta);

                                                                    riff_changed = true;
                                                                }
                                                            }
                                                        }
                                                    },
                                                    TrackEvent::NoteOn(_) => {},
                                                    TrackEvent::NoteOff(_) => {},
                                                    TrackEvent::Controller(_) => {},
                                                    TrackEvent::PitchBend(_pitch_bend) => {},
                                                    TrackEvent::KeyPressure => {},
                                                    TrackEvent::AudioPluginParameter(_) => {},
                                                    TrackEvent::Sample(_sample) => {},
                                                    TrackEvent::Measure(_) => {}
                                                    TrackEvent::NoteExpression(_) => {}
                                                });
                                                break;
                                            }
                                        }

                                        self.check_riff_changed_and_playing(riff_uuid.to_string(), &mut state, track_uuid.to_string(), playing, play_mode, playing_riff_set, riff_changed);

                                        if riff_changed {
                                            state.dirty = true;
                                        }
                                    },
                                    None => info!("Main - rx_ui processing loop - riff quantise selected event - problem getting selected riff index"),
                                }
                            },
                            None => ()
                        }
                    },
                    None => info!("Main - rx_ui processing loop - riff quantise selected event  - problem getting selected riff track number"),
                };
            },
            Err(_) => info!("Main - rx_ui processing loop - riff quantise selected - could not get lock on state"),
        };

        Ok(())
    }

    fn undo(&mut self, _state: &mut Arc<Mutex<DAWState>>) -> Result<(), String> {
        Ok(())
    }
}
