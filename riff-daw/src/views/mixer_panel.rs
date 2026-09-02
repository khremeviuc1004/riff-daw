use std::f64::consts::PI;
use crate::state::RiffDAWState;
use masonry::properties::types::{AsUnit, Length};
use xilem::{Vec2};
use xilem::view::{checkbox, CrossAxisAlignment, MainAxisAlignment, flex_col, flex_row, sized_box, slider, text_button, text_input, FlexSequence, FlexSpacer, transformed};
use crate::actions::{track_change_type_Mute, track_change_type_Pan, track_change_type_SoloOff, track_change_type_SoloOn, track_change_type_TrackNameChanged, track_change_type_Unmute, track_change_type_Volume};
use crate::domain::{Track, TrackBackgroundProcessorInwardEvent};
use crate::event::AudioLayerEvent;

pub fn mixer_panel(state: &RiffDAWState) -> impl FlexSequence<RiffDAWState> {

    let track_mixers = if let Ok(project) = state.project().lock() {
        project.song().tracks().iter().map(|track| {
            let track_uuid_name_change = track.uuid();
            let track_uuid_show_instr = track.uuid();
            let track_uuid_show_track_details = track.uuid();
            let track_uuid_mute = track.uuid();
            let track_uuid_solo = track.uuid();
            let track_uuid_volume = track.uuid();
            let track_uuid_pan = track.uuid();

            sized_box(
                flex_col((
                    sized_box(text_input(track.name().to_string(), move|state: &mut RiffDAWState, name| {
                        track_change_type_TrackNameChanged(state, name, Some(track_uuid_name_change.clone()));
                    })).width(Length::px(80.)).height(Length::px(80.)),
                    flex_row((
                        FlexSpacer::Flex(1.0),
                        text_button("I", move |state: &mut RiffDAWState| {
                            if let Some(sender) = state.audio_layer_sender.as_mut() {
                                let _ = sender.send(AudioLayerEvent::TrackBackgroundProcessorInward(TrackBackgroundProcessorInwardEvent::ShowInstrument, track_uuid_show_instr.clone()));
                            }
                        }), // show instr
                        text_button("=>", move |state: &mut RiffDAWState| {
                            state.selected_track = Some(track_uuid_show_track_details.clone());
                            state.track_details_window.insert(state.track_details_window_id.clone(), true);
                        }), // open track details
                    )),
                    flex_row((
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
                    )),
                    flex_row(
                        slider(0.0, 1.0, track.pan() as f64,move |state: &mut RiffDAWState, value: f64| {
                            track_change_type_Pan(state, None, value as f32, Some(track_uuid_pan.clone()));
                        }) // pan
                    ),
                    flex_row(
                        transformed(sized_box(slider(0.0, 1.0, track.volume() as f64,move |state: &mut RiffDAWState, value: f64| {
                            track_change_type_Volume(state, None, value as f32, Some(track_uuid_volume.clone()));
                        })).width(300.px()).height(50.px())).translate(Vec2::new(-100.0, 40.0)).rotate(-90.0 * PI / 180.0).scale(1.0) // volume
                    ),
                ))
            ).width(100.px()).height(500.px())
        })
            .collect()
    }
    else {
        vec![]
    };

    flex_row((
        track_mixers,
        FlexSpacer::Flex(1.0),
    )).main_axis_alignment(MainAxisAlignment::Start).cross_axis_alignment(CrossAxisAlignment::Start)
}