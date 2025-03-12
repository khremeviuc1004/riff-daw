use std::collections::HashMap;
use std::iter::Iterator;
use std::sync::{Arc, Mutex, MutexGuard};
use itertools::Itertools;

use log::*;
use uuid::Uuid;

use crate::domain::{DAWItemLength, DAWItemID, Riff};
use crate::{DAWItemPosition, DAWState, Note, PlayMode, Track, TrackEvent};
use crate::event::{DAWEvents, TrackChangeType, TranslateDirection, TranslationEntityType};
use crate::utils::DAWUtils;

/// Command pattern variation with undo
/// Memento pattern not used to hold state - a bit heavy
pub trait HistoryAction {
    fn execute(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<Vec<DAWEvents>, String>;
    fn undo(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<Vec<DAWEvents>, String>;

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
                    debug!("RiffSet riff updated - now calling state.play_riff_set_update_track");
                    state.play_riff_set_update_track_as_riff(playing_riff_set, track_uuid);
                }
            }
            PlayMode::RiffSequence => {}
            PlayMode::RiffGrid => {}
            PlayMode::RiffArrangement => {}
        }
    }

    fn check_playing(&self, riff_uuid: String, state: &mut MutexGuard<DAWState>, track_uuid: String, playing: bool, play_mode: PlayMode, playing_riff_set: Option<String>) {
        if playing {
            self.play_riff_set_update_track(riff_uuid, state, track_uuid, play_mode, playing_riff_set)
        }
    }
}

fn get_selected_track_riff_uuid(state: &mut Arc<Mutex<DAWState>>) -> (Option<String>, Option<String>) {
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
        Err(_) => debug!("could not get lock on state"),
    }
    (selected_riff_uuid, selected_riff_track_uuid)
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

    pub fn apply(&mut self, state: &mut Arc<Mutex<DAWState>>, mut action: Box<dyn HistoryAction>) -> Result<Vec<DAWEvents>, String> {
        debug!("History - apply: self.history.len()={}, self.head_index={}", self.history.len(), self.head_index);
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

    pub fn undo(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<Vec<DAWEvents>, String> {
        debug!("History - undo: self.history.len()={}, self.head_index={}", self.history.len(), self.head_index);
        // decrement the current top of the history
        if self.history.len() > self.head_index as usize && self.head_index >= 0 {
            if let Some(action) = self.history.get_mut(self.head_index as usize ) {
                self.head_index -= 1;
                action.undo(state)
            }
            else {
                debug!("Could not find action to undo.");
                Err("Could not find action to undo.".to_string())
            }
        }
        else {
            debug!("History head index greater than number of history items.");
            Err("History head index greater than number of history items.".to_string())
        }
    }

    pub fn redo(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<Vec<DAWEvents>, String> {
        debug!("History - redo: self.history.len()={}, self.head_index={}", self.history.len(), self.head_index);
        // get the current top of the history
        if self.head_index == -1 || ((self.head_index as usize) < (self.history.len() - 1)) {
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
    note_id: i32,
    position: f64,
    note: i32,
    velocity: i32,
    duration: f64,
    id: Option<String>,
    track_id: Option<String>,
    riff_id: Option<String>,
}

impl RiffAddNoteAction {
    pub fn new(
        note_id: i32,
        position: f64,
        note: i32,
        velocity: i32,
        duration: f64,
        state: &mut Arc<Mutex<DAWState>>
    ) -> Self {
        let (riff_id, track_id) = get_selected_track_riff_uuid(state);
        Self {
            note_id,
            position,
            note,
            velocity,
            duration,
            id: None,
            track_id,
            riff_id,
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
    pub fn id(&self) -> &Option<String> {
        &self.id
    }
    pub fn track_id(&self) -> &Option<String> {
        &self.track_id
    }
    pub fn riff_id(&self) -> &Option<String> {
        &self.riff_id
    }
    pub fn set_track_id(&mut self, track_id: Option<String>) {
        self.track_id = track_id;
    }
    pub fn set_riff_id(&mut self, riff_id: Option<String>) {
        self.riff_id = riff_id;
    }
    pub fn note_id(&self) -> i32 {
        self.note_id
    }
}

unsafe impl Send for RiffAddNoteAction {

}

impl HistoryAction for RiffAddNoteAction {
    fn execute(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<Vec<DAWEvents>, String> {
        match state.lock() {
            Ok(state) => {
                let mut state = state;
                let track_id = self.track_id().clone();
                let riff_id = self.riff_id().clone();

                match track_id {
                    Some(track_uuid) => {
                        let playing = state.playing();
                        let play_mode = state.play_mode();
                        let playing_riff_set = state.playing_riff_set().clone();
                        let mut riff_changed = false;

                        for track in state.get_project().song_mut().tracks_mut().iter_mut() {
                            if track.uuid().to_string() == *track_uuid {
                                match riff_id {
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
                                                    let new_note = Note::new_with_params(self.note_id(), self.position(), self.note(), 127, self.duration());
                                                    self.id = Some(new_note.id());
                                                    riff.events_mut().push(TrackEvent::Note(new_note));
                                                    riff.events_mut().sort_by(|a, b| a.position().partial_cmp(&b.position()).unwrap());
                                                    riff_changed = true;
                                                }
                                                break;
                                            }
                                        }
                                        self.check_riff_changed_and_playing(riff_uuid.clone(), &mut state, track_uuid.clone(), playing, play_mode, playing_riff_set, riff_changed);
                                    }
                                    None => debug!("problem getting selected riff index"),
                                }

                                break;
                            }
                        }

                        if riff_changed {
                            state.dirty = true;
                        }
                    },
                    None => debug!("problem getting selected riff track number"),
                };
            },
            Err(_) => debug!("could not get lock on state"),
        };
        Ok(vec![])
    }

    fn undo(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<Vec<DAWEvents>, String> {
        match state.lock() {
            Ok(state) => {
                let mut state = state;

                match self.track_id() {
                    Some(track_uuid) => {
                        let playing = state.playing();
                        let play_mode = state.play_mode();
                        let playing_riff_set = state.playing_riff_set().clone();

                        for track in state.get_project().song_mut().tracks_mut().iter_mut() {
                            if track.uuid().to_string() == *track_uuid {
                                match self.riff_id() {
                                    Some(riff_uuid) => {
                                        for riff in track.riffs_mut().iter_mut() {
                                            if riff.uuid().to_string() == *riff_uuid {
                                                riff.events_mut().retain(|event| match event {
                                                    TrackEvent::Note(note) => !(note.note() == self.note() && note.position() == self.position() && note.velocity() == self.velocity() && note.length() == self.duration()),
                                                    _ => true,
                                                });

                                                self.check_playing(riff_uuid.clone(), &mut state, track_uuid.clone(), playing, play_mode, playing_riff_set);
                                                break;
                                            }
                                        }
                                    }
                                    None => debug!("problem getting selected riff index"),
                                }
                                break;
                            }
                        }

                    },
                    None => debug!("problem getting selected riff track number"),
                };
            },
            Err(_) => debug!("could not get lock on state"),
        };
        Ok(vec![])
    }
}

#[derive(Clone)]
pub struct RiffDeleteNoteAction {
    position: f64,
    note: i32,
    velocity: i32,
    duration: f64,
    id: Option<String>,
    track_id: Option<String>,
    riff_id: Option<String>,
    deleted_note: Option<TrackEvent>,
}

unsafe impl Send for RiffDeleteNoteAction {

}

impl RiffDeleteNoteAction {
    pub fn new(
        position: f64,
        note: i32,
        state: &mut Arc<Mutex<DAWState>>
    ) -> Self {
        let (riff_id, track_id) = get_selected_track_riff_uuid(state);
        Self {
            position,
            note,
            velocity: 0,
            duration: 0.0,
            id: None,
            track_id,
            riff_id,
            deleted_note: None,
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
    fn execute(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<Vec<DAWEvents>, String> {
        match state.lock() {
            Ok(state) => {
                let mut state = state;
                let track_id = self.track_id.clone();
                let riff_id = self.riff_id.clone();

                match track_id {
                    Some(track_uuid) => {
                        let playing = state.playing();
                        let play_mode = state.play_mode();
                        let playing_riff_set = state.playing_riff_set().clone();
                        let mut riff_changed = false;

                        for track in state.get_project().song_mut().tracks_mut().iter_mut() {
                            if track.uuid().to_string() == track_uuid {
                                match riff_id {
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
                                                            debug!("delete note: position={}, note={}, velocity={}, duration={}", note.position(), note.note(), note.velocity(), note.length());
                                                            self.position = note.position();
                                                            self.velocity = note.velocity();
                                                            self.duration = note.length();
                                                            riff_changed = true;
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                                let note = riff.events_mut().iter_mut().find_position(|event| match event {
                                                    TrackEvent::Note(note) => note.note() == self.note() && note.position() <= self.position() && self.position() <= (note.position() + note.length()),
                                                    _ => false,
                                                });
                                                if let Some((index, item)) = note {
                                                    self.deleted_note = Some(riff.events_mut().remove(index));
                                                }

                                                self.check_riff_changed_and_playing(riff_uuid, &mut state, track_uuid, playing, play_mode, playing_riff_set, true);
                                                break;
                                            }
                                        }
                                    }
                                    None => debug!("problem getting selected riff index"),
                                }

                                break;
                            }
                        }

                        if riff_changed {
                            state.dirty = true;
                        }
                    },
                    None => debug!("problem getting selected riff track number"),
                };
            },
            Err(_) => debug!("could not get lock on state"),
        };
        Ok(vec![])
    }

    fn undo(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<Vec<DAWEvents>, String> {
        match state.lock() {
            Ok(state) => {
                let mut state = state;

                match self.track_id.clone() {
                    Some(track_uuid) => {
                        let playing = state.playing();
                        let play_mode = state.play_mode();
                        let playing_riff_set = state.playing_riff_set().clone();
                        let mut riff_changed = false;

                        match self.riff_id.clone() {
                            Some(riff_uuid) => {
                                for track in state.get_project().song_mut().tracks_mut().iter_mut() {
                                    if track.uuid().to_string() == track_uuid {
                                        for riff in track.riffs_mut().iter_mut() {
                                            if riff.uuid().to_string() == *riff_uuid {
                                                if let Some(deleted_note) = self.deleted_note.clone() {
                                                    riff.events_mut().push(deleted_note);
                                                    riff_changed = true;
                                                    self.check_riff_changed_and_playing(riff_uuid, &mut state, track_uuid, playing, play_mode, playing_riff_set, riff_changed);
                                                }
                                                break;
                                            }
                                        }
                                        break;
                                    }
                                }
                            }
                            None => debug!("problem getting selected riff index"),
                        }

                        if riff_changed {
                            state.dirty = true;
                        }
                    },
                    None => debug!("problem getting selected riff track number"),
                };
            },
            Err(_) => debug!("could not get lock on state"),
        };

        Ok(vec![])
    }
}


pub struct RiffCutSelectedAction {
    riff_event_uuids: Vec<String>,
    notes: Vec<Note>,
    track_uuid: Option<String>,
    riff_uuid: Option<String>,
}

impl RiffCutSelectedAction {
    pub fn new(
        track_uuid: Option<String>,
        riff_uuid: Option<String>,
        riff_event_uuids: Vec<String>,
    ) -> Self {
        Self {
            riff_event_uuids,
            notes: vec![],
            track_uuid,
            riff_uuid,
        }
    }
}

impl HistoryAction for RiffCutSelectedAction {
    fn execute(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<Vec<DAWEvents>, String> {
        match state.lock() {
            Ok(mut state) => {
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
                                                let notes_empty = self.notes.is_empty();

                                                for track_event in riff.events_mut().iter_mut() {
                                                    if let TrackEvent::Note(note) = track_event {
                                                        if self.riff_event_uuids.contains(&note.id_mut()) {
                                                            if notes_empty {
                                                                self.notes.push(note.clone());
                                                            }
                                                            riff_changed = true;
                                                        }
                                                    }
                                                }

                                                // remove the notes with in the window
                                                riff.events_mut().retain(|event| match event {
                                                    TrackEvent::ActiveSense => true,
                                                    TrackEvent::AfterTouch => true,
                                                    TrackEvent::ProgramChange => true,
                                                    TrackEvent::Note(note) => !self.riff_event_uuids.contains(&note.id()),
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
                                    None => debug!("Main - rx_ui processing loop - riff cut selected notes - problem getting selected riff index"),
                                }
                            },
                            None => ()
                        }
                    },
                    None => debug!("Main - rx_ui processing loop - riff cut selected notes  - problem getting selected riff track number"),
                }
            },
            Err(_) => debug!("Main - rx_ui processing loop - riff cut selected notes - could not get lock on state"),
        }

        Ok(vec![])
    }

    fn undo(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<Vec<DAWEvents>, String> {
        match state.lock() {
            Ok(mut state) => {
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
                                                // put back the notes from the cut
                                                for event in self.notes.iter() {
                                                    riff.events_mut().push(TrackEvent::Note(event.clone()));
                                                    riff_changed = true;
                                                }
                                                break;
                                            }
                                        }

                                        self.check_riff_changed_and_playing(riff_uuid, &mut state, track_uuid, playing, play_mode, playing_riff_set, riff_changed);

                                        if riff_changed {
                                            state.dirty = true;
                                        }
                                    },
                                    None => debug!("Main - rx_ui processing loop - riff undo cut selected notes - problem getting selected riff index"),
                                }
                            },
                            None => ()
                        }
                    },
                    None => debug!("Main - rx_ui processing loop - riff undo cut selected notes  - problem getting selected riff track number"),
                }
            },
            Err(_) => debug!("Main - rx_ui processing loop - riff undo cut selected notes - could not get lock on state"),
        }

        Ok(vec![])
    }
}

pub struct RiffTranslateSelectedAction {
    riff_event_uuids: Vec<String>,
    track_events: Vec<TrackEvent>,
    track_uuid: Option<String>,
    riff_uuid: Option<String>,
    translation_entity_type: TranslationEntityType,
    translate_direction: TranslateDirection,
    snap_in_beats: f64,
    tempo: f64,
}

impl RiffTranslateSelectedAction {
    pub fn new(
        track_uuid: Option<String>,
        riff_uuid: Option<String>,
        riff_event_uuids: Vec<String>,
        translation_entity_type: TranslationEntityType,
        translate_direction: TranslateDirection,
        snap_in_beats: f64,
    ) -> Self {
        Self {
            riff_event_uuids,
            track_events: vec![],
            track_uuid,
            riff_uuid,
            translation_entity_type,
            translate_direction,
            snap_in_beats,
            tempo: -1.0,
        }
    }
}

impl HistoryAction for RiffTranslateSelectedAction {
    fn execute(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<Vec<DAWEvents>, String> {
        match state.lock() {
            Ok(state) => {
                if self.tempo < 0.0 {
                    self.tempo = state.project().song().tempo();
                }

                let mut state = state;
                let snap_position_in_secs = self.snap_in_beats / self.tempo * 60.0;

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
                                                    TrackEvent::Note(note) => if self.riff_event_uuids.contains(&note.id_mut()) {
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
                                    None => debug!("Main - rx_ui processing loop - riff translate selected notes - problem getting selected riff index"),
                                }
                            },
                            None => ()
                        }

                        if riff_changed {
                            state.dirty = true;
                        }
                    },
                    None => debug!("Main - rx_ui processing loop - riff translate selected  - problem getting selected riff track number"),
                };
            },
            Err(_) => debug!("Main - rx_ui processing loop - riff translate selected - could not get lock on state"),
        };

        Ok(vec![])
    }

    fn undo(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<Vec<DAWEvents>, String> {
        match state.lock() {
            Ok(state) => {
                let mut state = state;
                let snap_position_in_secs = self.snap_in_beats / self.tempo * 60.0;

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
                                                    TrackEvent::Note(note) => if self.riff_event_uuids.contains(&note.id_mut()) {
                                                        let mut note_number = note.note();
                                                        let mut note_position = note.position();

                                                        match self.translate_direction {
                                                            TranslateDirection::Up => {
                                                                note_number -= 1;
                                                                if note_number > 127 {
                                                                    note_number = 127;
                                                                }
                                                                note.set_note(note_number);
                                                            },
                                                            TranslateDirection::Down => {
                                                                note_number += 1;
                                                                if note_number < 0 {
                                                                    note_number = 0;
                                                                }
                                                                note.set_note(note_number);
                                                            },
                                                            TranslateDirection::Left => {
                                                                note_position += snap_position_in_secs;
                                                                if note_position < 0.0 {
                                                                    note_position = 0.0;
                                                                }
                                                                note.set_position(note_position);
                                                            },
                                                            TranslateDirection::Right => {
                                                                note_position -= snap_position_in_secs;
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
                                    None => debug!("Main - rx_ui processing loop - riff translate selected notes - problem getting selected riff index"),
                                }
                            },
                            None => ()
                        }

                        if riff_changed {
                            state.dirty = true;
                        }
                    },
                    None => debug!("Main - rx_ui processing loop - riff undo translate selected  - problem getting selected riff track number"),
                };
            },
            Err(_) => debug!("Main - rx_ui processing loop - riff undo translate - could not get lock on state"),
        };

        Ok(vec![])
    }
}

pub struct RiffChangeLengthOfSelectedAction {
    riff_event_uuids: Vec<String>,
    notes: Vec<Note>,
    track_uuid: Option<String>,
    riff_uuid: Option<String>,
    length_increment_in_beats: f64,
    lengthen: bool,
    tempo: f64,
}

impl RiffChangeLengthOfSelectedAction {
    pub fn new(
        track_uuid: Option<String>,
        riff_uuid: Option<String>,
        riff_event_uuids: Vec<String>,
        length_increment_in_beats: f64,
        lengthen: bool,
    ) -> Self {
        Self {
            riff_event_uuids,
            notes: vec![],
            track_uuid,
            riff_uuid,
            length_increment_in_beats,
            lengthen,
            tempo: -1.0,
        }
    }
}

impl HistoryAction for RiffChangeLengthOfSelectedAction {
    fn execute(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<Vec<DAWEvents>, String> {
        match state.lock() {
            Ok(state) => {
                if self.tempo < 0.0 {
                    self.tempo = state.project().song().tempo();
                }

                let mut state = state;
                let length_increment_in_secs = self.length_increment_in_beats / self.tempo * 60.0;

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
                                                    TrackEvent::Note(note) => if self.riff_event_uuids.contains(&note.id_mut()) {
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
                                    None => debug!("Main - rx_ui processing loop - riff lengthen selected notes - problem getting selected riff index"),
                                }
                            },
                            None => ()
                        }

                        if riff_changed {
                            state.dirty = true;
                        }
                    },
                    None => debug!("Main - rx_ui processing loop - riff lengthen selected notes - problem getting selected riff track number"),
                };
            },
            Err(_) => debug!("Main - rx_ui processing loop - riff lengthen selected notes - could not get lock on state"),
        }

        Ok(vec![])
    }

    fn undo(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<Vec<DAWEvents>, String> {
        match state.lock() {
            Ok(state) => {
                let mut state = state;
                let length_increment_in_secs = self.length_increment_in_beats / self.tempo * 60.0;

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
                                                    TrackEvent::Note(note) => if self.riff_event_uuids.contains(&note.id_mut()) {
                                                        let note_length = note.length();

                                                        if self.lengthen && (note_length - length_increment_in_secs) > 0.0 {
                                                            note.set_length(note_length - length_increment_in_secs);
                                                        }
                                                        else {
                                                            note.set_length(note_length + length_increment_in_secs);
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
                                    None => debug!("Main - rx_ui processing loop - riff undo lengthen selected notes - problem getting selected riff index"),
                                }
                            },
                            None => ()
                        }

                        if riff_changed {
                            state.dirty = true;
                        }
                    },
                    None => debug!("Main - rx_ui processing loop - riff undo lengthen selected notes - problem getting selected riff track number"),
                };
            },
            Err(_) => debug!("Main - rx_ui processing loop - riff undo lengthen selected notes - could not get lock on state"),
        };

        Ok(vec![])
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
    fn execute(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<Vec<DAWEvents>, String> {
        match state.lock() {
            Ok(state) => {
                let mut copy_buffer: Vec<TrackEvent> = vec![];
                let mut pasted_events_buffer: Vec<Note> = vec![];

                if self.notes.is_empty() {
                    state.track_event_copy_buffer().iter().for_each(|event| {
                        let mut new_note = event.clone();
                        new_note.set_id(Uuid::new_v4().to_string());
                        copy_buffer.push(new_note);
                    });
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
                                                        TrackEvent::ActiveSense => debug!("TrackChangeType::RiffPasteSelectedNotes ActiveSense not yet implemented!"),
                                                        TrackEvent::AfterTouch => debug!("TrackChangeType::RiffPasteSelectedNotes AfterTouch not yet implemented!"),
                                                        TrackEvent::ProgramChange => debug!("TrackChangeType::RiffPasteSelectedNotes ProgramChange not yet implemented!"),
                                                        TrackEvent::Note(mut note) => {
                                                            if self.notes.is_empty() {
                                                                note.set_position(note.position() + self.edit_cursor_position_in_beats);
                                                            }

                                                            pasted_events_buffer.push(note.clone());
                                                            riff.events_mut().push(TrackEvent::Note(note));

                                                            riff_changed = true;
                                                        },
                                                        TrackEvent::NoteOn(_) => debug!("TrackChangeType::RiffPasteSelectedNotes NoteOn not yet implemented!"),
                                                        TrackEvent::NoteOff(_) => debug!("TrackChangeType::RiffPasteSelectedNotes NoteOff not yet implemented!"),
                                                        TrackEvent::Controller(_) => debug!("TrackChangeType::RiffPasteSelectedNotes Controller not yet implemented!"),
                                                        TrackEvent::PitchBend(_pitch_bend) => debug!("TrackChangeType::RiffPasteSelectedNotes PitchBend not yet implemented!"),
                                                        TrackEvent::KeyPressure => debug!("TrackChangeType::RiffPasteSelectedNotes KeyPressure not yet implemented!"),
                                                        TrackEvent::AudioPluginParameter(_) => debug!("TrackChangeType::RiffPasteSelectedNotes AudioPluginParameter not yet implemented!"),
                                                        TrackEvent::Sample(_sample) => debug!("TrackChangeType::RiffPasteSelectedNotes Sample not yet implemented!"),
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
                                    None => debug!("Main - rx_ui processing loop - riff paste selected - problem getting selected riff index"),
                                }
                            },
                            None => ()
                        }
                    },
                    None => debug!("Main - rx_ui processing loop - riff references paste selected  - problem getting selected riff track number"),
                }
            },
            Err(_) => debug!("Main - rx_ui processing loop - riff paste selected - could not get lock on state"),
        }

        Ok(vec![])
    }

    fn undo(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<Vec<DAWEvents>, String> {
        match state.lock() {
            Ok(mut state) => {

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
                                                self.notes.iter_mut().for_each(|event| riff.events_mut().retain(|riff_event| riff_event.id() != event.id_mut()));
                                                break;
                                            }
                                        }

                                        if riff_changed {
                                            state.dirty = true;
                                        }

                                        self.check_riff_changed_and_playing(riff_uuid.to_string(), &mut state, track_uuid.to_string(), playing, play_mode, playing_riff_set, riff_changed);
                                    },
                                    None => debug!("Main - rx_ui processing loop - riff undo paste selected - problem getting selected riff index"),
                                }
                            },
                            None => ()
                        }
                    },
                    None => debug!("Main - rx_ui processing loop - riff undo paste selected  - problem getting selected riff track number"),
                }
            },
            Err(_) => debug!("Main - rx_ui processing loop - riff undo paste selected - could not get lock on state"),
        }
        Ok(vec![])
    }
}

pub struct RiffQuantiseSelectedAction {
    riff_event_uuids: Vec<String>,
    track_uuid: Option<String>,
    riff_uuid: Option<String>,
    snap_in_beats: f64,
    snap_strength: f64,
    snap_deltas: HashMap<String, f64>,
    length_snap_deltas: HashMap<String, f64>,
    snap_start: bool,
    snap_end: bool,
}

impl RiffQuantiseSelectedAction {
    pub fn new(
        riff_event_uuids: Vec<String>,
        track_uuid: Option<String>,
        riff_uuid: Option<String>,
        snap_in_beats: f64,
        snap_strength: f64,
        snap_start: bool,
        snap_end: bool,
    ) -> Self {
        Self {
            riff_event_uuids,
            track_uuid,
            riff_uuid,
            snap_in_beats,
            snap_strength,
            snap_deltas: HashMap::new(),
            length_snap_deltas: HashMap::new(),
            snap_start,
            snap_end
        }
    }
}

impl HistoryAction for RiffQuantiseSelectedAction {
    fn execute(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<Vec<DAWEvents>, String> {
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
                                                        if self.riff_event_uuids.contains(&note.id_mut()) {
                                                            if self.snap_start {
                                                                let note_position = note.position();
                                                                let calculated_snap = DAWUtils::quantise(note_position, self.snap_in_beats, self.snap_strength, false);

                                                                if calculated_snap.snapped {
                                                                    note.set_position(calculated_snap.snapped_value);
                                                                    self.snap_deltas.insert(note.id_mut(), calculated_snap.calculated_delta);
                                                                    riff_changed = true;
                                                                }
                                                            }
                                                            if self.snap_end {
                                                                let note_length = note.length();
                                                                let calculated_snap = DAWUtils::quantise(note_length, self.snap_in_beats, self.snap_strength, true);

                                                                if calculated_snap.snapped {
                                                                    note.set_length(calculated_snap.snapped_value);
                                                                    self.length_snap_deltas.insert(note.id_mut(), calculated_snap.calculated_delta);
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
                                    None => debug!("Main - rx_ui processing loop - riff quantise selected event - problem getting selected riff index"),
                                }
                            },
                            None => ()
                        }
                    },
                    None => debug!("Main - rx_ui processing loop - riff quantise selected event  - problem getting selected riff track number"),
                };
            },
            Err(_) => debug!("Main - rx_ui processing loop - riff quantise selected - could not get lock on state"),
        };

        Ok(vec![])
    }

    fn undo(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<Vec<DAWEvents>, String> {
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
                                                        if self.snap_start {
                                                            if self.riff_event_uuids.contains(&note.id_mut()) {
                                                                let note_position = note.position();

                                                                if note_position >= 0.0 {
                                                                    if let Some(snap_delta) = self.snap_deltas.get(&note.id_mut()) {
                                                                        if (note_position + snap_delta) >= 0.0 {
                                                                            note.set_position(note_position + snap_delta);

                                                                            riff_changed = true;
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                        if self.snap_end {
                                                            if self.riff_event_uuids.contains(&note.id_mut()) {
                                                                let note_length = note.length();

                                                                if note_length >= 0.0 {
                                                                    if let Some(snap_delta) = self.length_snap_deltas.get(&note.id_mut()) {
                                                                        if (note_length + snap_delta) > 0.0 {
                                                                            note.set_length(note_length + snap_delta);

                                                                            riff_changed = true;
                                                                        }
                                                                    }
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
                                    None => debug!("Main - rx_ui processing loop - riff undo quantise selected event - problem getting selected riff index"),
                                }
                            },
                            None => ()
                        }
                    },
                    None => debug!("Main - rx_ui processing loop - riff undo quantise selected event  - problem getting selected riff track number"),
                };
            },
            Err(_) => debug!("Main - rx_ui processing loop - riff undo quantise selected - could not get lock on state"),
        };

        Ok(vec![])
    }
}


#[derive(Clone)]
pub struct RiffAdd {
    name: String,
    duration: f64,
    id: Uuid,
    track_id: Option<String>,
}

impl RiffAdd {
    pub fn new(
        id: Uuid,
        name: String,
        duration: f64,
        state: &mut Arc<Mutex<DAWState>>
    ) -> Self {
        let (_, track_id) = get_selected_track_riff_uuid(state);
        Self {
            id,
            name,
            duration,
            track_id,
        }
    }
}

unsafe impl Send for RiffAdd {}

impl HistoryAction for RiffAdd {
    fn execute(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<Vec<DAWEvents>, String> {
        let mut daw_events_to_propagate = vec![];

        match state.lock() {
            Ok(mut state) => {
                match self.track_id.clone() {
                    Some(track_uuid) => {
                        state.set_selected_track(Some(track_uuid.clone()));
                        state.set_selected_riff_uuid(track_uuid.clone(), self.id.to_string());

                        match state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                            Some(track) => {
                                track.riffs_mut().push(Riff::new_with_name_and_length(self.id.clone(), self.name.clone(), self.duration));
                                state.set_dirty(true);
                                daw_events_to_propagate.push(DAWEvents::TrackChange(TrackChangeType::UpdateTrackDetails, Some(track_uuid)));
                            }
                            None => ()
                        }
                    },
                    None => debug!("Main - rx_ui processing loop - riff add  - problem getting selected riff track uuid"),
                }
            },
            Err(_) => debug!("Main - rx_ui processing loop - riff add - could not get lock on state"),
        }

        Ok(daw_events_to_propagate)
    }

    fn undo(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<Vec<DAWEvents>, String> {
        let mut daw_events_to_propagate = vec![];

        match state.lock() {
            Ok(mut state) => {
                match self.track_id.clone() {
                    Some(track_uuid) => {
                        state.set_selected_track(Some(track_uuid.clone()));
                        state.set_selected_riff_uuid(track_uuid.clone(), self.id.to_string());

                        match state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                            Some(track) => {
                                track.riffs_mut().retain(|riff| riff.id() != self.id.to_string().clone());
                                daw_events_to_propagate.push(DAWEvents::TrackChange(TrackChangeType::UpdateTrackDetails, Some(track_uuid)));
                            }
                            None => ()
                        }
                    },
                    None => debug!("Main - rx_ui processing loop - riff undo add  - problem getting selected riff track uuid"),
                }
            },
            Err(_) => debug!("Main - rx_ui processing loop - riff undo add - could not get lock on state"),
        }

        Ok(daw_events_to_propagate)
    }
}


#[derive(Clone)]
pub struct RiffDelete {
    id: String,
    track_id: Option<String>,
    riff: Option<Riff>,
}

impl RiffDelete {
    pub fn new(
        id: String,
        track_id: Option<String>,
    ) -> Self {
        Self {
            id,
            track_id,
            riff: None,
        }
    }
}

unsafe impl Send for RiffDelete {}

impl HistoryAction for RiffDelete {
    fn execute(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<Vec<DAWEvents>, String> {
        let mut daw_events_to_propagate = vec![];

        match state.lock() {
            Ok(mut state) => {
                match self.track_id.clone() {
                    Some(track_uuid) => {
                        state.set_selected_track(Some(track_uuid.clone()));
                        state.set_selected_riff_uuid(track_uuid.clone(), self.id.to_string());

                        match state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                            Some(track) => {
                                // get the riff index

                                //remove the riff - take ownership and hold onto the riff
                                let mut riff_index: usize = usize::MAX;
                                for (index, riff) in track.riffs_mut().iter_mut().enumerate() {
                                    if riff.id() == self.id.to_string().clone() {
                                        riff_index = index;
                                        break;
                                    }
                                }
                                if riff_index < usize::MAX {
                                    self.riff = Some(track.riffs_mut().remove(riff_index));
                                }
                                daw_events_to_propagate.push(DAWEvents::TrackChange(TrackChangeType::UpdateTrackDetails, Some(track_uuid)));
                            }
                            None => ()
                        }
                    },
                    None => debug!("Main - rx_ui processing loop - riff delete  - problem getting selected riff track uuid"),
                }
            },
            Err(_) => debug!("Main - rx_ui processing loop - riff delete - could not get lock on state"),
        }

        Ok(daw_events_to_propagate)
    }

    fn undo(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<Vec<DAWEvents>, String> {
        let mut daw_events_to_propagate = vec![];

        match state.lock() {
            Ok(mut state) => {
                match self.track_id.clone() {
                    Some(track_uuid) => {
                        state.set_selected_track(Some(track_uuid.clone()));
                        state.set_selected_riff_uuid(track_uuid.clone(), self.id.to_string());

                        match state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
                            Some(track) => {
                                if let Some(riff) = self.riff.take() {
                                    track.riffs_mut().push(riff);
                                    state.set_dirty(true);
                                    daw_events_to_propagate.push(DAWEvents::TrackChange(TrackChangeType::UpdateTrackDetails, Some(track_uuid)));
                                }
                            }
                            None => ()
                        }
                    },
                    None => debug!("Main - rx_ui processing loop - riff delete undo  - problem getting selected riff track uuid"),
                }
            },
            Err(_) => debug!("Main - rx_ui processing loop - riff delete undo - could not get lock on state"),
        }

        Ok(daw_events_to_propagate)
    }
}