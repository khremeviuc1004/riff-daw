use crate::domain::GeneralTrackType;
use crate::icons::{ICON_ARROW_DOWN, ICON_ARROW_UP, ICON_PLUS};
use crate::state::RiffDAWState;
use crate::views::{icon};
use masonry::properties::types::AsUnit;
use uuid::Uuid;
use xilem::view::{button, flex_col, indexed_stack, label, sized_box, FlexSequence};
use crate::actions::track_change_type_Added;

pub fn add_track_selector(
    data: &mut RiffDAWState,
) -> impl FlexSequence<RiffDAWState> + use<> {
    let selected_track_type = data.selected_track_type.clone() as usize;
    (
        indexed_stack((
            flex_col(sized_box(label("Instrument track")).width(100.px())),
            flex_col(sized_box(label("Audio track")).width(100.px())),
            flex_col(sized_box(label("Midi track")).width(100.px())),
        ))
        .active(selected_track_type),
        button(
            icon(ICON_ARROW_UP.to_string()),
            |state: &mut RiffDAWState| match state.selected_track_type.clone() {
                GeneralTrackType::InstrumentTrack => {
                    state.selected_track_type = GeneralTrackType::MidiTrack
                }
                GeneralTrackType::AudioTrack => {
                    state.selected_track_type = GeneralTrackType::InstrumentTrack
                }
                GeneralTrackType::MidiTrack => {
                    state.selected_track_type = GeneralTrackType::AudioTrack
                }
                _ => (),
            },
        ),
        button(
            icon(ICON_ARROW_DOWN.to_string()),
            |state: &mut RiffDAWState| match state.selected_track_type.clone() {
                GeneralTrackType::InstrumentTrack => {
                    state.selected_track_type = GeneralTrackType::AudioTrack
                }
                GeneralTrackType::AudioTrack => {
                    state.selected_track_type = GeneralTrackType::MidiTrack
                }
                GeneralTrackType::MidiTrack => {
                    state.selected_track_type = GeneralTrackType::InstrumentTrack
                }
                _ => (),
            },
        ),
        button(icon(ICON_PLUS.to_string()), |state: &mut RiffDAWState| {
            println!("Add a new track of type: {:?}", state.selected_track_type);
            track_change_type_Added(state, state.selected_track_type.clone(), Some(Uuid::new_v4().to_string()));
        }),
    )
}
