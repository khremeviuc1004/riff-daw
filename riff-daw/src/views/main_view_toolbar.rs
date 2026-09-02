use xilem::view::{button, checkbox, flex_row, text_button, Flex, FlexSequence, FlexSpacer};
use masonry::peniko::Color;
use masonry::properties::types::{AsUnit, Length};
use xilem::style::Style;
use crate::actions::{daw_events_Panic, daw_events_Redo, daw_events_Undo};
use crate::domain::GeneralTrackType;
use crate::icons::{ICON_ARROW_BACK_UP, ICON_ARROW_FORWARD_UP, ICON_ARROW_LEFT, ICON_ARROW_LEFT_TAIL, ICON_RESTORE};
use crate::state::{RiffDAWMainView, RiffDAWState};
use crate::views::{add_track_selector, dropdown_view, icon, loop_selector, toggle_button};
use crate::views::file_toolbar::file_toolbar;

pub fn main_view_toolbar(
    data: &mut RiffDAWState,
) -> impl FlexSequence<RiffDAWState> + use<> {
    let track_types = vec![
        (GeneralTrackType::InstrumentTrack, "Instrument track".to_string()),
        (GeneralTrackType::AudioTrack, "Audio track".to_string()),
        (GeneralTrackType::MidiTrack, "Midi track".to_string())];

        (
            file_toolbar(),
            flex_row(dropdown_view(track_types, Some(data.selected_track_type.clone()), |state: &mut RiffDAWState, track_type: GeneralTrackType| {
                state.selected_track_type = track_type.clone();
            })).gap(1.px()),
            // flex_row(add_track_selector::add_track_selector(data)).gap(1.px()),
            flex_row(loop_selector::loop_selector(data)).gap(1.px()),
            flex_row((
                button(icon(ICON_ARROW_BACK_UP.to_string()), |state: &mut RiffDAWState| daw_events_Undo(state)),
                button(icon(ICON_ARROW_FORWARD_UP.to_string()), |state: &mut RiffDAWState| daw_events_Redo(state)),
            )).gap(1.px()),
            button(icon(ICON_RESTORE.to_string()), |state: &mut RiffDAWState| daw_events_Panic(state)),
            flex_row((
                text_button("Track View", |state: &mut RiffDAWState| state.main_view = RiffDAWMainView::Track),
                text_button("Riff View", |state: &mut RiffDAWState| state.main_view = RiffDAWMainView::Riff),
            )).gap(1.px()),
            FlexSpacer::Flex(1.0)
        )
}

