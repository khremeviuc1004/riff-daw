use masonry::properties::types::{AsUnit, CrossAxisAlignment, Length, MainAxisAlignment};
use xilem::view::{checkbox, flex_row, label, sized_box, text_button, text_input, FlexSequence};
use crate::actions::{track_change_type_Deleted, track_change_type_Mute, track_change_type_SoloOff, track_change_type_SoloOn, track_change_type_TrackNameChanged, track_change_type_Unmute};
use crate::audio::AudioLayer;
use crate::event::{AudioLayerEvent, TrackBackgroundProcessorInwardEvent};
use crate::state::{RiffDAWState};
use crate::domain::{Track};

pub fn track_panel_sequence<State>(
    state: &RiffDAWState,
    height: Length,
) -> impl FlexSequence<RiffDAWState> {
    state.project.lock().unwrap().song.tracks().iter().enumerate().map(|(track_index, track)| {
        let track_uuid_select = track.uuid().clone();
        let track_uuid_name_change = track.uuid().clone();
        let track_uuid_show_instrument = track.uuid().clone();
        let track_uuid_show_track_details = track.uuid().clone();
        let track_uuid_delete = track.uuid().clone();
        let track_uuid_solo = track.uuid().clone();
        let track_uuid_mute = track.uuid().clone();
        let first_real_riff_uuid = if let Some(riff)  = track.riffs().get(1) {
            riff.uuid.uuid.to_string()
        }
        else {
            track.riffs().get(0).unwrap().uuid.uuid.to_string()
        };
        let first_real_riff_uuid2 = first_real_riff_uuid.clone();

        // (
        sized_box(
            flex_row(
                (
                    // delete track
                    text_button("X", move |state: &mut RiffDAWState| {
                        track_change_type_Deleted(state, Some(track_uuid_delete.clone()));
                    }),
                    // select track
                    text_button("C", move |state: &mut RiffDAWState| {
                        state.selected_track = Some(track_uuid_select.clone());
                        state.selected_riff_uuid = first_real_riff_uuid.clone();
                        state.set_selected_riff_uuid(track_uuid_select.clone(), first_real_riff_uuid.clone());
                        if let Some(audio_layer_sender) = state.audio_layer_sender.as_ref() {
                            let _ = audio_layer_sender.send(AudioLayerEvent::SelectTrackBackgroundProcessor(track_uuid_select.clone()));
                        }
                    }),
                    // track name
                    sized_box(text_input(track.name().to_string(), move|state: &mut RiffDAWState, name| {
                        track_change_type_TrackNameChanged(state, name, Some(track_uuid_name_change.clone()));
                    })).width(Length::px(150.)),
                    // mute track
                    checkbox("M", track.mute(), move |state: &mut RiffDAWState, checked| {
                        if checked {
                            track_change_type_Mute(state, Some(track_uuid_mute.clone()));
                        }
                        else {
                            track_change_type_Unmute(state, Some(track_uuid_mute.clone()));
                        }
                    }),
                    // solo track
                    checkbox("S", track.solo(), move |state: &mut RiffDAWState, checked| {
                        if checked {
                            track_change_type_SoloOn(state, Some(track_uuid_solo.clone()));
                        }
                        else {
                            track_change_type_SoloOff(state, Some(track_uuid_solo.clone()));
                        }
                    }),
                    // record on track
                    // checkbox("R", track.solo(), move |state: &mut RiffDAWState, checked| {
                    //
                    // }),
                    // show instrument audio plugin window
                    text_button("I", move |state: &mut RiffDAWState| {
                        if let Some(sender) = state.audio_layer_sender.as_mut() {
                            let _ = sender.send(AudioLayerEvent::TrackBackgroundProcessorInward(TrackBackgroundProcessorInwardEvent::ShowInstrument, track_uuid_show_instrument.clone()));
                        }
                    }),
                    // show track details window
                    text_button("D", move |state: &mut RiffDAWState| {
                        state.selected_track = Some(track_uuid_show_track_details.clone());
                        state.selected_riff_uuid = first_real_riff_uuid2.clone();
                        state.track_details_window.insert(state.track_details_window_id.clone(), true);
                    }),
                )
            ).main_axis_alignment(MainAxisAlignment::Start)
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .gap(0.5.px())
        ).height(height)
        // )
    }).collect::<Vec<_>>()
}
