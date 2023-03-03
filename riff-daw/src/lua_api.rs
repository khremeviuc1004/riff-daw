use std::sync::{Arc, Mutex};

use mlua::{UserData, UserDataMethods};

use crate::{DAWEvents, DAWState, GeneralTrackType, TrackChangeType};
use crate::DAWEvents::TrackChange;
use crate::Track;

pub struct LuaState {
    pub state: Arc<Mutex<DAWState>>,
    pub tx_from_ui: crossbeam_channel::Sender<DAWEvents>,
}

impl LuaState {
    pub fn get_first_track_name(&self) -> String {
        match self.state.lock() {
            Ok(state) => state.project().song().tracks().first().unwrap().name().to_string(),
            Err(_) => String::from("xxx"),
        }
    }
}

impl UserData for LuaState {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(_methods: &mut M) {
        _methods.add_method("get_first_track_name", |_, this, ()| Ok(this.get_first_track_name()));
        _methods.add_method_mut("add_instrument_track", |_, this, ()| {
            match this.tx_from_ui.send(TrackChange(TrackChangeType::Added(GeneralTrackType::InstrumentTrack), None)) {
                Ok(_) => {}
                Err(_) => {
                    println!("Could not send instrument track add track change event.");
                }
            }
            Ok(())
        });
    }
}
