use std::collections::HashMap;
use std::iter::Iterator;
use std::sync::{Arc, Mutex, MutexGuard};
use itertools::Itertools;

use log::*;
use uuid::Uuid;

use crate::domain::{DAWItemLength, DAWItemID, Riff};
use crate::{AudioEffectTrack, DAWItemPosition, DAWState, Note, PlayMode, Track, TrackEvent, TrackType};
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

    // discard the stale redo branch - once a fresh action is applied after one or
    // more undos the entries after the head can never be reached again. (The old
    // descending-intent range `(len-1)..(head)` never iterated, so nothing was ever
    // truncated and undone actions lingered in the history.)
    fn truncate_forward(&mut self) {
        if self.head_index + 1 < self.history.len() as i32 {
            self.history.truncate((self.head_index + 1) as usize);
        }
    }

    // record an action that has already been executed by the handler it came from
    // (the scoped mutation path) - unlike apply this does not call execute().
    pub fn record(&mut self, action: Box<dyn HistoryAction>) {
        self.truncate_forward();
        self.history.push(action);
        self.head_index += 1;
    }

    pub fn apply(&mut self, state: &mut Arc<Mutex<DAWState>>, mut action: Box<dyn HistoryAction>) -> Result<Vec<DAWEvents>, String> {
        debug!("History - apply: self.history.len()={}, self.head_index={}", self.history.len(), self.head_index);
        self.truncate_forward();
        let result = action.execute(state);
        self.history.push(action);
        self.head_index += 1;
        result
    }

    pub fn clear(&mut self) {
        self.history.clear();
        self.head_index = -1;
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

                                                state.set_dirty(true);
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
                                                if let Some((index, _item)) = note {
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
                                                riff_changed = true;
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

    pub fn new_with_track_id(
        id: Uuid,
        name: String,
        duration: f64,
        _state: &mut Arc<Mutex<DAWState>>,
        track_id: Option<String>,
    ) -> Self {
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
                                state.set_dirty(true);
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
                                state.set_dirty(true);
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
/// Marks the project dirty - used by every command execution/undo so all song
/// graph mutations propagate the dirty flag regardless of which command ran.
fn mark_project_dirty(state: &mut Arc<Mutex<DAWState>>) {
    match state.lock() {
        Ok(mut state) => state.set_dirty(true),
        Err(_) => debug!("History - mark_project_dirty - could not get lock on state"),
    }
}

/// A per-mutator history command written at the event handler site, following the
/// same command pattern as the bespoke note editing actions: the redo closure
/// performs the forward mutation of the song graph, the undo closure performs
/// its inverse. A command captures only the minimal inverse data its mutator
/// needs - the overwritten field values, the removed element, the ids touched -
/// never surrounding collections or the song.
pub struct ActionCommand {
    pub description: &'static str,
    redo: Box<dyn FnMut(&mut Arc<Mutex<DAWState>>) -> Result<Vec<DAWEvents>, String> + Send>,
    undo: Box<dyn FnMut(&mut Arc<Mutex<DAWState>>) -> Result<Vec<DAWEvents>, String> + Send>,
}

unsafe impl Send for ActionCommand {}

impl ActionCommand {
    pub fn new(description: &'static str,
               redo: impl FnMut(&mut Arc<Mutex<DAWState>>) -> Result<Vec<DAWEvents>, String> + Send + 'static,
               undo: impl FnMut(&mut Arc<Mutex<DAWState>>) -> Result<Vec<DAWEvents>, String> + Send + 'static) -> Self {
        Self { description, redo: Box::new(redo), undo: Box::new(undo) }
    }
}

impl HistoryAction for ActionCommand {
    fn execute(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<Vec<DAWEvents>, String> {
        debug!("History - ActionCommand - execute '{}'.", self.description);
        let result = (self.redo)(state);
        if result.is_ok() {
            mark_project_dirty(state);
        }
        result
    }

    fn undo(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<Vec<DAWEvents>, String> {
        debug!("History - ActionCommand - undo '{}'.", self.description);
        let result = (self.undo)(state);
        if result.is_ok() {
            mark_project_dirty(state);
        }
        result
    }
}

/// Applies a per-mutator command through the history manager (recording it for
/// undo/redo) and re-enqueues any events it propagates for UI refresh - the
/// pattern used by the existing event handlers that record history.
pub fn apply_history_command(history_manager: &mut Arc<Mutex<HistoryManager>>,
                             tx_from_ui: &crossbeam_channel::Sender<DAWEvents>,
                             state: &mut Arc<Mutex<DAWState>>,
                             command: ActionCommand) {
    let description = command.description;
    match history_manager.lock() {
        Ok(mut history_manager) => {
            match history_manager.apply(state, Box::new(command)) {
                Ok(daw_events) => {
                    for daw_event in daw_events {
                        let _ = tx_from_ui.send(daw_event);
                    }
                },
                Err(error) => debug!("History - could not apply '{}': {}", description, error),
            }
        },
        Err(_) => debug!("History - could not lock the history manager to apply '{}'", description),
    }
}

/// Locks the state, reads the song's current tempo/time signature etc. - small
/// helpers used by command closures at their construction sites to capture the
/// "before" values needed for the inverse mutation.
pub fn song_f64_field(state: &mut Arc<Mutex<DAWState>>, get: impl FnOnce(&DAWState) -> f64) -> f64 {
    match state.lock() {
        Ok(state) => get(&state),
        Err(_) => 0.0,
    }
}

// ----- scoped state commands -----
//
// For the more context dependent mutators (reference drag/copy/paste, play
// modes, riff event drags, routing changes, set/sequence/grid/arrangement
// operations...) the forward mutation body stays at the handler site and the
// command records only the changed SCOPE of the song graph before and after -
// a track's reference list, a riff's events, one collection - rather than
// inverse arithmetic per shape or (worse) the whole song. Both values are small
// slices of the graph; memory per history entry stays bounded to the scope that
// the action touched.

pub fn apply_scoped_state<T: Clone + Send + 'static>(state: &mut Arc<Mutex<DAWState>>,
                                                     writer: fn(&mut DAWState, &str, &T),
                                                     scope_id: &str,
                                                     value: &T) -> Result<Vec<DAWEvents>, String> {
    match state.lock() {
        Ok(mut state) => {
            writer(&mut state, scope_id, value);
            state.get_project().song_mut().recalculate_song_length();
            Ok(vec![DAWEvents::UpdateUI])
        },
        Err(_) => Err("could not get a lock on the state to restore the scoped song graph".to_string()),
    }
}

pub fn scope_command<T: Clone + Send + 'static>(description: &'static str,
                                                writer: fn(&mut DAWState, &str, &T),
                                                scope_id: String,
                                                before: T,
                                                after: T) -> ActionCommand {
    let undo_scope_id = scope_id.clone();
    ActionCommand::new(
        description,
        move |state| apply_scoped_state(state, writer, &scope_id, &after),
        move |state| apply_scoped_state(state, writer, &undo_scope_id, &before),
    )
}

// ---- track riff references ----
pub fn get_track_riff_refs(state: &DAWState, track_uuid: &str) -> Vec<crate::domain::RiffReference> {
    state.project().song().tracks().iter().find(|track| track.uuid().to_string() == track_uuid).map(|track| track.riff_refs().clone()).unwrap_or_default()
}
pub fn set_track_riff_refs(state: &mut DAWState, track_uuid: &str, refs: &Vec<crate::domain::RiffReference>) {
    if let Some(track) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
        track.riff_refs_mut().clear();
        for riff_ref in refs.iter() {
            track.riff_refs_mut().push(riff_ref.clone());
        }
    }
}

// ---- track riff reference play modes ----
pub fn get_track_riff_ref_modes(state: &DAWState, track_uuid: &str) -> Vec<(uuid::Uuid, crate::domain::RiffReferenceMode)> {
    state.project().song().tracks().iter().find(|track| track.uuid().to_string() == track_uuid)
        .map(|track| track.riff_refs().iter().map(|riff_ref| (riff_ref.uuid(), riff_ref.mode().clone())).collect_vec())
        .unwrap_or_default()
}
pub fn set_track_riff_ref_modes(state: &mut DAWState, track_uuid: &str, modes: &Vec<(uuid::Uuid, crate::domain::RiffReferenceMode)>) {
    if let Some(track) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
        for (riff_ref_uuid, mode) in modes.iter() {
            if let Some(riff_ref) = track.riff_refs_mut().iter_mut().find(|riff_ref| riff_ref.uuid() == *riff_ref_uuid) {
                riff_ref.set_mode(mode.clone());
            }
        }
    }
}

// ---- a single riff (whole riff: name, length, colour, events...) ----
// scope id: "<track uuid>|<riff uuid>"
pub fn get_riff_scope(state: &DAWState, scope_id: &str) -> Option<crate::domain::Riff> {
    let mut parts = scope_id.splitn(2, '|');
    let track_uuid = parts.next().unwrap_or("");
    let riff_uuid = parts.next().unwrap_or("");
    state.project().song().tracks().iter().find(|track| track.uuid().to_string() == track_uuid)
        .and_then(|track| track.riffs().iter().find(|riff| riff.uuid().to_string() == riff_uuid))
        .cloned()
}
pub fn set_riff_scope(state: &mut DAWState, scope_id: &str, riff: &Option<crate::domain::Riff>) {
    let mut parts = scope_id.splitn(2, '|');
    let track_uuid = parts.next().unwrap_or("");
    let riff_uuid = parts.next().unwrap_or("");
    if let Some(track) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
        let existing_index = track.riffs().iter().position(|existing| existing.uuid().to_string() == riff_uuid);
        match (existing_index, riff) {
            (Some(index), Some(new_riff)) => { track.riffs_mut()[index] = new_riff.clone(); },
            (Some(_), None) => { track.riffs_mut().retain(|existing| existing.uuid().to_string() != riff_uuid); },
            (None, Some(new_riff)) => { track.riffs_mut().push(new_riff.clone()); },
            (None, None) => {}
        }
    }
}

// ---- a track's automation (events + envelopes) ----
pub fn get_track_automation(state: &DAWState, track_uuid: &str) -> Option<crate::domain::Automation> {
    state.project().song().tracks().iter().find(|track| track.uuid().to_string() == track_uuid).map(|track| track.automation().clone())
}
pub fn set_track_automation(state: &mut DAWState, track_uuid: &str, automation: &Option<crate::domain::Automation>) {
    if let Some(track) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
        if let Some(automation) = automation {
            *track.automation_mut() = automation.clone();
        }
    }
}

// ---- a track's midi routings ----
pub fn get_track_midi_routings(state: &DAWState, track_uuid: &str) -> Vec<crate::domain::TrackEventRouting> {
    state.project().song().tracks().iter().find(|track| track.uuid().to_string() == track_uuid).map(|track| track.midi_routings().clone()).unwrap_or_default()
}
pub fn set_track_midi_routings(state: &mut DAWState, track_uuid: &str, routings: &Vec<crate::domain::TrackEventRouting>) {
    if let Some(track) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
        *track.midi_routings_mut() = routings.clone();
    }
}

// ---- a track's audio routings ----
pub fn get_track_audio_routings(state: &DAWState, track_uuid: &str) -> Vec<crate::domain::AudioRouting> {
    state.project().song().tracks().iter().find(|track| track.uuid().to_string() == track_uuid).map(|track| track.audio_routings().clone()).unwrap_or_default()
}
pub fn set_track_audio_routings(state: &mut DAWState, track_uuid: &str, routings: &Vec<crate::domain::AudioRouting>) {
    if let Some(track) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
        *track.audio_routings_mut() = routings.clone();
    }
}

// ---- a track's instrument plugin description ----
pub fn get_track_instrument(state: &DAWState, track_uuid: &str) -> Option<crate::domain::AudioPlugin> {
    match state.project().song().tracks().iter().find(|track| track.uuid().to_string() == track_uuid) {
        Some(TrackType::InstrumentTrack(track)) => Some(track.instrument().clone()),
        _ => None,
    }
}
pub fn set_track_instrument(state: &mut DAWState, track_uuid: &str, instrument: &Option<crate::domain::AudioPlugin>) {
    if let Some(TrackType::InstrumentTrack(track)) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
        if let Some(instrument) = instrument {
            *track.instrument_mut() = instrument.clone();
        }
    }
}

// ---- a track's effect plugin list ----
pub fn get_track_effects(state: &DAWState, track_uuid: &str) -> Vec<crate::domain::AudioPlugin> {
    match state.project().song().tracks().iter().find(|track| track.uuid().to_string() == track_uuid) {
        Some(TrackType::InstrumentTrack(track)) => track.effects().to_vec(),
        _ => vec![],
    }
}
pub fn set_track_effects(state: &mut DAWState, track_uuid: &str, effects: &Vec<crate::domain::AudioPlugin>) {
    if let Some(TrackType::InstrumentTrack(track)) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == track_uuid) {
        *track.effects_mut() = effects.clone();
    }
}

// ---- the track order ----
pub fn get_track_order(state: &DAWState, _scope_id: &str) -> Vec<String> {
    state.project().song().tracks().iter().map(|track| track.uuid().to_string()).collect_vec()
}
pub fn set_track_order(state: &mut DAWState, _scope_id: &str, order: &Vec<String>) {
    // settle one position at a time by swapping the track that belongs at this
    // index into place - avoids cloning the whole track list (TrackType is not Clone).
    let tracks = state.get_project().song_mut().tracks_mut();
    let mut index = 0;
    while index < tracks.len() {
        let wanted_uuid = match order.get(index) {
            Some(wanted_uuid) => wanted_uuid.clone(),
            None => break,
        };
        if tracks[index].uuid().to_string() == wanted_uuid {
            index += 1;
            continue;
        }
        let mut candidate = None;
        for search_index in (index + 1)..tracks.len() {
            if tracks[search_index].uuid().to_string() == wanted_uuid {
                candidate = Some(search_index);
                break;
            }
        }
        match candidate {
            Some(candidate) => tracks.swap(index, candidate),
            None => break,
        }
    }
}

// ----- scoped mutation slots -----
//
// Event handlers that mutate the song graph declare their mutation SCOPE at the
// top of the arm (`scoped_mutation = Some(begin_scoped_mutation(state, Scope::X))`);
// the dispatch tail records the command once the handler body has run, by
// reading the scope again and comparing it against the captured "before". The
// command stored in history holds only the two copies of the affected slice of
// the graph, applied back via a per-scope writer on redo/undo.

#[derive(Clone)]
pub enum Scope {
    SongProperties,                                   // tempo + time signature
    Loops,
    RiffSets,
    RiffSequences,
    RiffGrids,
    RiffArrangements,
    Samples,
    TrackOrder,
    TrackAutomation(String),                          // track uuid: automation + selected riff of that track (covers every automation view)
    TrackRiffRefs(String),                            // track uuid
    TrackRiffs(String),                               // track uuid (covers riff add/copy/delete, events)
    Riff(String, String),                             // track uuid, riff uuid
    MidiRoutings(String),                             // track uuid
    AudioRoutings(String),                            // track uuid
    Instrument(String),                               // track uuid
    Effects(String),                                  // track uuid
    AllTrackRiffRefs,                                 // every track's reference list (multi track reference edits)
    AllTrackRiffsAndRefs,                             // every track's riffs and references (structure wide edits)
    AllReferenceStructures,                           // track refs + every grid's and set's refs (id regeneration)
}

#[derive(Clone, serde::Serialize)]
pub enum ScopeValue {
    SongProperties((f64, f64, f64)),
    Loops(Vec<crate::domain::Loop>),
    RiffSets(Vec<crate::domain::RiffSet>),
    RiffSequences(Vec<crate::domain::RiffSequence>),
    RiffGrids(Vec<crate::domain::RiffGrid>),
    RiffArrangements(Vec<crate::domain::RiffArrangement>),
    Samples(std::collections::HashMap<String, crate::domain::Sample>),
    TrackOrder(Vec<String>),
    TrackAutomation(Box<(Option<crate::domain::Automation>, Option<String>, Option<crate::domain::Riff>, Vec<crate::domain::RiffArrangement>)>),
    TrackRiffRefs(Vec<crate::domain::RiffReference>),
    TrackRiffs(Vec<crate::domain::Riff>),
    Riff(Option<crate::domain::Riff>),
    MidiRoutings(Vec<crate::domain::TrackEventRouting>),
    AudioRoutings(Vec<crate::domain::AudioRouting>),
    Instrument(Option<crate::domain::AudioPlugin>),
    Effects(Vec<crate::domain::AudioPlugin>),
    AllTrackRiffRefs(Vec<(String, Vec<crate::domain::RiffReference>)>),
    AllTrackRiffsAndRefs(Vec<(String, Vec<crate::domain::Riff>, Vec<crate::domain::RiffReference>)>),
    AllReferenceStructures(Box<(Vec<(String, Vec<crate::domain::RiffReference>)>, Vec<crate::domain::RiffGrid>, Vec<crate::domain::RiffSet>)>),
}

pub struct ScopedMutation {
    description: &'static str,
    scope: Scope,
    before: ScopeValue,
}

pub fn read_scope(state: &DAWState, scope: &Scope) -> ScopeValue {
    match scope {
        Scope::SongProperties => ScopeValue::SongProperties((state.project().song().tempo(), state.project().song().time_signature_numerator(), state.project().song().time_signature_denominator())),
        Scope::Loops => ScopeValue::Loops(state.project().song().loops().to_vec()),
        Scope::RiffSets => ScopeValue::RiffSets(state.project().song().riff_sets().clone()),
        Scope::RiffSequences => ScopeValue::RiffSequences(state.project().song().riff_sequences().clone()),
        Scope::RiffGrids => ScopeValue::RiffGrids(state.project().song().riff_grids().clone()),
        Scope::RiffArrangements => ScopeValue::RiffArrangements(state.project().song().riff_arrangements().clone()),
        Scope::Samples => ScopeValue::Samples(state.project().song().samples().clone()),
        Scope::TrackOrder => ScopeValue::TrackOrder(get_track_order(state, "")),
        Scope::TrackAutomation(track_uuid) => {
            let automation = get_track_automation(state, track_uuid);
            let selected_riff = state.selected_riff_uuid(track_uuid.clone());
            let riff = match &selected_riff {
                Some(riff_uuid) => get_riff_scope(state, &format!("{track_uuid}|{riff_uuid}")),
                None => None,
            };
            // automation edits in the RiffArrangement view target the arrangement's
            // own per-track automation, so the arrangements are part of this scope.
            ScopeValue::TrackAutomation(Box::new((automation, selected_riff, riff, state.project().song().riff_arrangements().clone())))
        }
        Scope::TrackRiffRefs(track_uuid) => ScopeValue::TrackRiffRefs(get_track_riff_refs(state, track_uuid)),
        Scope::TrackRiffs(track_uuid) => ScopeValue::TrackRiffs(state.project().song().tracks().iter().find(|track| track.uuid().to_string() == *track_uuid).map(|track| track.riffs().clone()).unwrap_or_default()),
        Scope::Riff(track_uuid, riff_uuid) => ScopeValue::Riff(get_riff_scope(state, &format!("{track_uuid}|{riff_uuid}"))),
        Scope::MidiRoutings(track_uuid) => ScopeValue::MidiRoutings(get_track_midi_routings(state, track_uuid)),
        Scope::AudioRoutings(track_uuid) => ScopeValue::AudioRoutings(get_track_audio_routings(state, track_uuid)),
        Scope::Instrument(track_uuid) => ScopeValue::Instrument(get_track_instrument(state, track_uuid)),
        Scope::Effects(track_uuid) => ScopeValue::Effects(get_track_effects(state, track_uuid)),
        Scope::AllTrackRiffRefs => ScopeValue::AllTrackRiffRefs(state.project().song().tracks().iter().map(|track| (track.uuid().to_string(), track.riff_refs().clone())).collect_vec()),
        Scope::AllTrackRiffsAndRefs => ScopeValue::AllTrackRiffsAndRefs(state.project().song().tracks().iter().map(|track| (track.uuid().to_string(), track.riffs().clone(), track.riff_refs().clone())).collect_vec()),
        Scope::AllReferenceStructures => ScopeValue::AllReferenceStructures(Box::new((
            state.project().song().tracks().iter().map(|track| (track.uuid().to_string(), track.riff_refs().clone())).collect_vec(),
            state.project().song().riff_grids().clone(),
            state.project().song().riff_sets().clone(),
        ))),
    }
}

/// Serialise-equality helper for scoped/inline command capture comparisons.
pub fn json_equal<T: serde::Serialize + ?Sized>(a: &T, b: &T) -> bool {
    serde_json::to_string(a).ok() == serde_json::to_string(b).ok()
}

fn write_scope(state: &mut DAWState, scope: &Scope, value: &ScopeValue) {
    match (scope, value) {
        (Scope::SongProperties, ScopeValue::SongProperties((tempo, numerator, denominator))) => {
            state.get_project().song_mut().set_tempo(*tempo);
            state.get_project().song_mut().set_time_signature_numerator(*numerator);
            state.get_project().song_mut().set_time_signature_denominator(*denominator);
            // keep the track background processors in step with the restored song clock
            let track_uuids = state.get_project().song().tracks().iter().map(|track| track.uuid().to_string()).collect_vec();
            for track_uuid in track_uuids {
                state.send_to_track_background_processor(track_uuid.clone(), crate::event::TrackBackgroundProcessorInwardEvent::Tempo(*tempo));
                state.send_to_track_background_processor(track_uuid, crate::event::TrackBackgroundProcessorInwardEvent::TimeSignatureChange(*numerator as u32, *denominator as u32));
            }
        }
        (Scope::Loops, ScopeValue::Loops(loops)) => { *state.get_project().song_mut().loops_mut() = loops.clone(); },
        (Scope::RiffSets, ScopeValue::RiffSets(riff_sets)) => { *state.get_project().song_mut().riff_sets_mut() = riff_sets.clone(); },
        (Scope::RiffSequences, ScopeValue::RiffSequences(riff_sequences)) => { *state.get_project().song_mut().riff_sequences_mut() = riff_sequences.clone(); },
        (Scope::RiffGrids, ScopeValue::RiffGrids(riff_grids)) => { *state.get_project().song_mut().riff_grids_mut() = riff_grids.clone(); },
        (Scope::RiffArrangements, ScopeValue::RiffArrangements(riff_arrangements)) => { *state.get_project().song_mut().riff_arrangements_mut() = riff_arrangements.clone(); },
        (Scope::Samples, ScopeValue::Samples(samples)) => { *state.get_project().song_mut().samples_mut() = samples.clone(); },
        (Scope::TrackOrder, ScopeValue::TrackOrder(order)) => set_track_order(state, "", order),
        (Scope::TrackAutomation(track_uuid), ScopeValue::TrackAutomation(automation_scope)) => {
            let (automation, _, riff, riff_arrangements) = automation_scope.as_ref();
            set_track_automation(state, track_uuid, automation);
            if let (Some(riff_uuid), Some(riff)) = (automation_scope.1.as_ref(), automation_scope.2.as_ref()) {
                set_riff_scope(state, &format!("{track_uuid}|{riff_uuid}"), &Some(riff.clone()));
            }
            *state.get_project().song_mut().riff_arrangements_mut() = riff_arrangements.clone();
        }
        (Scope::TrackRiffRefs(track_uuid), ScopeValue::TrackRiffRefs(refs)) => set_track_riff_refs(state, track_uuid, refs),
        (Scope::TrackRiffs(track_uuid), ScopeValue::TrackRiffs(riffs)) => {
            if let Some(track) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == *track_uuid) {
                *track.riffs_mut() = riffs.clone();
            }
        }
        (Scope::Riff(track_uuid, riff_uuid), ScopeValue::Riff(riff)) => set_riff_scope(state, &format!("{track_uuid}|{riff_uuid}"), riff),
        (Scope::MidiRoutings(track_uuid), ScopeValue::MidiRoutings(routings)) => {
            // sync the track background processors with the restored routing graph.
            let previous = get_track_midi_routings(state, track_uuid);
            for removed in previous.iter().filter(|route| !routings.iter().any(|desired| desired.uuid() == route.uuid())) {
                let destination_track_uuid = match &removed.destination {
                    crate::domain::TrackEventRoutingNodeType::Track(track_uuid) => track_uuid.clone(),
                    crate::domain::TrackEventRoutingNodeType::Instrument(track_uuid, _) => track_uuid.clone(),
                    crate::domain::TrackEventRoutingNodeType::Effect(track_uuid, _) => track_uuid.clone(),
                };
                state.send_to_track_background_processor(track_uuid.clone(), crate::event::TrackBackgroundProcessorInwardEvent::RemoveTrackEventSendRouting(removed.uuid()));
                state.send_to_track_background_processor(destination_track_uuid, crate::event::TrackBackgroundProcessorInwardEvent::RemoveTrackEventReceiveRouting(removed.uuid()));
            }
            for added in routings.iter().filter(|route| !previous.iter().any(|existing| existing.uuid() == route.uuid())) {
                state.send_midi_routing_to_track_background_processors(track_uuid.clone(), added.clone());
            }
            // routes kept but with changed details (channel/note range) - update in place.
            for desired in routings.iter().filter(|route| previous.iter().any(|existing| existing.uuid() == route.uuid() && !json_equal(existing, route))) {
                state.send_to_track_background_processor(track_uuid.clone(), crate::event::TrackBackgroundProcessorInwardEvent::UpdateTrackEventSendRouting(desired.uuid(), desired.clone()));
                let destination_track_uuid = match &desired.destination {
                    crate::domain::TrackEventRoutingNodeType::Track(track_uuid) => track_uuid.clone(),
                    crate::domain::TrackEventRoutingNodeType::Instrument(track_uuid, _) => track_uuid.clone(),
                    crate::domain::TrackEventRoutingNodeType::Effect(track_uuid, _) => track_uuid.clone(),
                };
                state.send_to_track_background_processor(destination_track_uuid, crate::event::TrackBackgroundProcessorInwardEvent::UpdateTrackEventReceiveRouting(desired.uuid(), desired.clone()));
            }
            set_track_midi_routings(state, track_uuid, routings);
        }
        (Scope::AudioRoutings(track_uuid), ScopeValue::AudioRoutings(routings)) => {
            let previous = get_track_audio_routings(state, track_uuid);
            for removed in previous.iter().filter(|route| !routings.iter().any(|desired| desired.uuid() == route.uuid())) {
                let destination_track_uuid = match &removed.destination {
                    crate::domain::AudioRoutingNodeType::Track(track_uuid) => track_uuid.clone(),
                    crate::domain::AudioRoutingNodeType::Instrument(track_uuid, _, _, _) => track_uuid.clone(),
                    crate::domain::AudioRoutingNodeType::Effect(track_uuid, _, _, _) => track_uuid.clone(),
                };
                state.send_to_track_background_processor(track_uuid.clone(), crate::event::TrackBackgroundProcessorInwardEvent::RemoveAudioSendRouting(removed.uuid()));
                state.send_to_track_background_processor(destination_track_uuid, crate::event::TrackBackgroundProcessorInwardEvent::RemoveAudioReceiveRouting(removed.uuid()));
            }
            for added in routings.iter().filter(|route| !previous.iter().any(|existing| existing.uuid() == route.uuid())) {
                state.send_audio_routing_to_track_background_processors(track_uuid.clone(), added.clone());
            }
            set_track_audio_routings(state, track_uuid, routings);
        }
        (Scope::Instrument(track_uuid), ScopeValue::Instrument(instrument)) => set_track_instrument(state, track_uuid, instrument),
        (Scope::Effects(track_uuid), ScopeValue::Effects(effects)) => set_track_effects(state, track_uuid, effects),
        (Scope::AllTrackRiffRefs, ScopeValue::AllTrackRiffRefs(all_refs)) => {
            for (track_uuid, refs) in all_refs.iter() {
                set_track_riff_refs(state, track_uuid, &refs.clone());
            }
        }
        (Scope::AllReferenceStructures, ScopeValue::AllReferenceStructures(all)) => {
            let (all_refs, riff_grids, riff_sets) = all.as_ref();
            for (track_uuid, refs) in all_refs.iter() {
                set_track_riff_refs(state, track_uuid, &refs.clone());
            }
            *state.get_project().song_mut().riff_grids_mut() = riff_grids.clone();
            *state.get_project().song_mut().riff_sets_mut() = riff_sets.clone();
        }
        (Scope::AllTrackRiffsAndRefs, ScopeValue::AllTrackRiffsAndRefs(all)) => {
            for (track_uuid, riffs, refs) in all.iter() {
                if let Some(track) = state.get_project().song_mut().tracks_mut().iter_mut().find(|track| track.uuid().to_string() == *track_uuid) {
                    *track.riffs_mut() = riffs.clone();
                    *track.riff_refs_mut() = refs.clone();
                }
            }
        }
        _ => debug!("History - write_scope - scope/value mismatch!"),
    }
}

fn scopes_equal(before: &ScopeValue, after: &ScopeValue) -> bool {
    serde_json::to_string(before).ok() == serde_json::to_string(after).ok()
}

pub fn begin_scoped_mutation(state: &mut Arc<Mutex<DAWState>>, description: &'static str, scope: Scope) -> Option<ScopedMutation> {
    match state.lock() {
        Ok(state) => {
            let before = read_scope(&state, &scope);
            Some(ScopedMutation { description, scope, before })
        },
        Err(_) => {
            debug!("History - begin_scoped_mutation - could not get lock on state");
            None
        }
    }
}

pub struct ScopedCommand {
    description: &'static str,
    scope: Scope,
    before: ScopeValue,
    after: ScopeValue,
}

impl HistoryAction for ScopedCommand {
    fn execute(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<Vec<DAWEvents>, String> {
        match state.lock() {
            Ok(mut state) => {
                write_scope(&mut state, &self.scope, &self.after);
                state.get_project().song_mut().recalculate_song_length();
                state.set_dirty(true);
                Ok(vec![DAWEvents::UpdateUI])
            },
            Err(_) => Err("could not get a lock on the state to re-apply the scoped history action".to_string()),
        }
    }

    fn undo(&mut self, state: &mut Arc<Mutex<DAWState>>) -> Result<Vec<DAWEvents>, String> {
        match state.lock() {
            Ok(mut state) => {
                write_scope(&mut state, &self.scope, &self.before);
                state.get_project().song_mut().recalculate_song_length();
                state.set_dirty(true);
                Ok(vec![DAWEvents::UpdateUI])
            },
            Err(_) => Err("could not get a lock on the state to undo the scoped history action".to_string()),
        }
    }
}

unsafe impl Send for ScopedCommand {}

/// Called at the dispatch tail with the slot an arm marked during this event:
/// reads the scope again, records a scoped command when it actually changed.
pub fn record_scoped_mutation(scoped_mutation: Option<ScopedMutation>,
                              history_manager: &mut Arc<Mutex<HistoryManager>>,
                              state: &mut Arc<Mutex<DAWState>>) {
    if let Some(scoped_mutation) = scoped_mutation {
        let (after, changed) = match state.lock() {
            Ok(mut state) => {
                let after = read_scope(&state, &scoped_mutation.scope);
                let changed = !scopes_equal(&scoped_mutation.before, &after);
                if changed {
                    state.set_dirty(true);
                }
                (after, changed)
            },
            Err(_) => return,
        };
        if !changed {
            return;
        }
        let command = ScopedCommand {
            description: scoped_mutation.description,
            scope: scoped_mutation.scope,
            before: scoped_mutation.before,
            after,
        };
        match history_manager.lock() {
            Ok(mut history_manager) => {
                // the forward mutation has already been performed by the handler,
                // so record without re-executing.
                history_manager.record(Box::new(command));
            },
            Err(_) => debug!("History - record_scoped_mutation - could not lock the history manager"),
        }
    }
}


/// Rebuilds a plugin load descriptor from a persisted AudioPlugin domain object,
/// so undo/redo of instrument and effect changes can send the live plugin thread
/// its ChangeInstrument/AddEffect message from the graph data alone.
pub fn scanned_plugin_from_audio_plugin(plugin: &crate::domain::AudioPlugin) -> Option<crate::domain::ScannedPlugin> {
    match crate::domain::AudioPluginType::from_str(plugin.plugin_type()) {
        Ok(audio_plugin_stack) => Some(crate::domain::ScannedPlugin {
            name: plugin.name().to_string(),
            path: plugin.file().to_string(),
            id: plugin.uid().to_string(),
            sub_id: plugin.sub_plugin_id().clone(),
            audio_plugin_stack,
        }),
        Err(_) => None,
    }
}

use std::str::FromStr;

/// Instrument change command: restores/replaces the track's instrument domain
/// description AND tells the track's background thread to (re)load the plugin,
/// including the persisted preset state for the restored plugin.
pub fn instrument_change_command(track_uuid: String,
                                 before: Option<crate::domain::AudioPlugin>,
                                 after: Option<crate::domain::AudioPlugin>,
                                 vst24_plugin_loaders: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, vst::host::PluginLoader<crate::domain::VstHost>>>>,
                                 clap_plugin_loaders: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, simple_clap_host_helper_lib::plugin::library::PluginLibrary>>>) -> ActionCommand {
    let redo_track_uuid = track_uuid.clone();
    let undo_track_uuid = track_uuid;
    let redo_vst24_plugin_loaders = vst24_plugin_loaders.clone();
    let redo_clap_plugin_loaders = clap_plugin_loaders.clone();
    ActionCommand::new(
        "track instrument changed",
        move |state| apply_instrument_change(state, &redo_track_uuid, &after, redo_vst24_plugin_loaders.clone(), redo_clap_plugin_loaders.clone()),
        move |state| apply_instrument_change(state, &undo_track_uuid, &before, vst24_plugin_loaders.clone(), clap_plugin_loaders.clone()),
    )
}

pub fn apply_instrument_change(state: &mut Arc<Mutex<DAWState>>,
                               track_uuid: &str,
                               instrument: &Option<crate::domain::AudioPlugin>,
                               vst24_plugin_loaders: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, vst::host::PluginLoader<crate::domain::VstHost>>>>,
                               clap_plugin_loaders: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, simple_clap_host_helper_lib::plugin::library::PluginLibrary>>>) -> Result<Vec<DAWEvents>, String> {
    match state.lock() {
        Ok(mut state) => {
            set_track_instrument(&mut state, track_uuid, instrument);
            if let Some(instrument) = instrument {
                if let Some(scanned_plugin) = scanned_plugin_from_audio_plugin(instrument) {
                    state.send_to_track_background_processor(track_uuid.to_string(), crate::event::TrackBackgroundProcessorInwardEvent::ChangeInstrument(vst24_plugin_loaders, clap_plugin_loaders, instrument.uuid(), scanned_plugin));
                    if !instrument.preset_data().is_empty() {
                        state.send_to_track_background_processor(track_uuid.to_string(), crate::event::TrackBackgroundProcessorInwardEvent::SetPresetData(instrument.preset_data().to_string(), vec![]));
                    }
                }
            }
            Ok(vec![DAWEvents::TrackChange(crate::event::TrackChangeType::UpdateTrackDetails, Some(track_uuid.to_string()))])
        },
        Err(_) => Err("could not get a lock on the state to change the track instrument".to_string()),
    }
}

/// Effect list change command: restores/replaces the track's effect plugin list
/// and tells the track's background thread to add/remove the delta effects.
pub fn effects_change_command(track_uuid: String,
                              before: Vec<crate::domain::AudioPlugin>,
                              after: Vec<crate::domain::AudioPlugin>,
                              vst24_plugin_loaders: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, vst::host::PluginLoader<crate::domain::VstHost>>>>,
                              clap_plugin_loaders: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, simple_clap_host_helper_lib::plugin::library::PluginLibrary>>>) -> ActionCommand {
    let redo_track_uuid = track_uuid.clone();
    let undo_track_uuid = track_uuid;
    let redo_before = before.clone();
    let undo_after = after.clone();
    let redo_vst24_plugin_loaders = vst24_plugin_loaders.clone();
    let redo_clap_plugin_loaders = clap_plugin_loaders.clone();
    ActionCommand::new(
        "track effects changed",
        move |state| apply_effects_change(state, &redo_track_uuid, &redo_before, &after, redo_vst24_plugin_loaders.clone(), redo_clap_plugin_loaders.clone()),
        move |state| apply_effects_change(state, &undo_track_uuid, &undo_after, &before, vst24_plugin_loaders.clone(), clap_plugin_loaders.clone()),
    )
}

pub fn apply_effects_change(state: &mut Arc<Mutex<DAWState>>,
                            track_uuid: &str,
                            previous: &Vec<crate::domain::AudioPlugin>,
                            desired: &Vec<crate::domain::AudioPlugin>,
                            vst24_plugin_loaders: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, vst::host::PluginLoader<crate::domain::VstHost>>>>,
                            clap_plugin_loaders: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, simple_clap_host_helper_lib::plugin::library::PluginLibrary>>>) -> Result<Vec<DAWEvents>, String> {
        match state.lock() {
            Ok(mut state) => {
                // removed effects: present in previous, gone from desired.
                for effect in previous.iter().filter(|effect| !desired.iter().any(|desired_effect| desired_effect.uuid() == effect.uuid())) {
                    state.send_to_track_background_processor(track_uuid.to_string(), crate::event::TrackBackgroundProcessorInwardEvent::DeleteEffect(effect.uuid().to_string()));
                }
                // added effects: present in desired, not in previous.
                for effect in desired.iter().filter(|effect| !previous.iter().any(|previous_effect| previous_effect.uuid() == effect.uuid())) {
                    if let Some(scanned_plugin) = scanned_plugin_from_audio_plugin(effect) {
                        state.send_to_track_background_processor(track_uuid.to_string(), crate::event::TrackBackgroundProcessorInwardEvent::AddEffect(vst24_plugin_loaders.clone(), clap_plugin_loaders.clone(), effect.uuid(), scanned_plugin));
                        if !effect.preset_data().is_empty() {
                            state.send_to_track_background_processor(track_uuid.to_string(), crate::event::TrackBackgroundProcessorInwardEvent::SetPresetData(String::new(), vec![effect.preset_data().to_string()]));
                        }
                    }
                }
                set_track_effects(&mut state, track_uuid, desired);
                Ok(vec![DAWEvents::TrackChange(crate::event::TrackChangeType::UpdateTrackDetails, Some(track_uuid.to_string()))])
            },
            Err(_) => Err("could not get a lock on the state to change the track effects".to_string()),
        }
}
