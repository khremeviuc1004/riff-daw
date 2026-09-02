use log::debug;
use crate::event::{AutomationEditType, OperationModeType, ShowType};
use crate::state::{AutomationViewMode, RiffDAWState};

pub fn daw_events_AutomationEditTypeChange(state: &mut RiffDAWState, automation_edit_type: AutomationEditType) {
    state.set_automation_edit_type(automation_edit_type);
}

pub fn daw_events_AutomationViewShowTypeChange(state: &mut RiffDAWState, show_type: ShowType) {
    let type_to_show = match show_type {
        ShowType::Velocity => AutomationViewMode::NoteVelocities,
        ShowType::Controller => AutomationViewMode::Controllers,
        ShowType::PitchBend => AutomationViewMode::PitchBend,
        ShowType::InstrumentParameter => AutomationViewMode::Instrument,
        ShowType::EffectParameter => AutomationViewMode::Effect,
        ShowType::NoteExpression => AutomationViewMode::NoteExpression,
    };
    state.set_automation_view_mode(type_to_show);
    // gui.ui.automation_drawing_area.queue_draw();
}

pub fn daw_events_ControllerOperationModeChange(state: &mut RiffDAWState, mode: OperationModeType) {
    debug!("Event: ControllerOperationModeChange");
}

pub fn daw_events_RepaintAutomationView (state: &mut RiffDAWState) {
    // gui.ui.automation_drawing_area.queue_draw();
}
