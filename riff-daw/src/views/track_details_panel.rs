use itertools::Itertools;
use crate::constants::MUSICAL_ITEM_LENGTH_OPTIONS;
use crate::domain::{Track, TrackBackgroundProcessorInwardEvent, TrackType};
use crate::state::RiffDAWState;
use crate::views::portal;
use masonry::properties::types::{AsUnit, CrossAxisAlignment, MainAxisAlignment};
use uuid::Uuid;
use xilem::view::{flex_col, flex_row, label, sized_box, text_button, text_input, FlexSequence, FlexSpacer};
use crate::actions::{track_change_type_EffectAdded, track_change_type_EffectDeleted, track_change_type_InstrumentChanged, track_change_type_RiffAdd, track_change_type_RiffLengthChange, track_change_type_RiffNameChange};
use crate::event::AudioLayerEvent;
use crate::utils::DAWUtils;

pub fn track_details_panel(state: &mut RiffDAWState) -> impl FlexSequence<RiffDAWState> {
    let available_instrument_plugins_list =
        state.configuration.scanned_instrument_plugins.successfully_scanned
            .iter()
            .sorted_by(|entry1, entry2| {
                entry1.1.cmp(entry2.1)
            })
            .map(|(x, y)| {
                let audio_plugin_details = x.clone();
                let audio_plugin_name = y.clone();
                flex_row((
                    text_button("S", move |state: &mut RiffDAWState| {
                        if let Some(selected_track_uuid) = state.selected_track.as_ref() {
                            println!("Add instrument: track uuid={}, x={}, y={}", selected_track_uuid, audio_plugin_details.as_str(), audio_plugin_name.as_str());
                            track_change_type_InstrumentChanged(state, audio_plugin_details.as_str().to_string(), Some(selected_track_uuid.to_string()));
                        }
                    }),
                    label(y.to_string()),
                    FlexSpacer::Flex(1.0),
                ))
            })
            .collect::<Vec<_>>();
    let available_effect_plugins_list =
        state.configuration.scanned_effect_plugins.successfully_scanned
            .iter()
            .sorted_by(|entry1, entry2| {
                entry1.1.cmp(entry2.1)
            })
            .map(|(x, y)| {
                let audio_plugin_details = x.clone();
                let audio_plugin_name = y.clone();
                flex_row((
                    text_button("S", move |state: &mut RiffDAWState| {
                        if let Some(selected_track_uuid) = state.selected_track.as_ref() {
                            println!("Add effect: track uuid={}, x={}, y={}", selected_track_uuid, audio_plugin_details.as_str(), audio_plugin_name.as_str());
                            track_change_type_EffectAdded(state, Uuid::new_v4(), audio_plugin_name.clone(), audio_plugin_details.as_str().to_string(), Some(selected_track_uuid.to_string()));
                        }
                    }),
                    label(y.to_string()),
                    FlexSpacer::Flex(1.0),
                ))
            })
            .collect::<Vec<_>>();
    let (track_uuid, track_name, instrument_name, effects) = if let Some(selected_track_uuid) = state.selected_track.as_ref() {
        let (track_name, instrument_name, effects) =  if let Ok(project) = state.project().lock() {
            if let Some(track) = project.song().tracks().iter().find(|track| track.uuid() == *selected_track_uuid) {
                let (instrument_name, effects) = match track {
                    TrackType::InstrumentTrack(instrument_track) => (
                        instrument_track.instrument().name().to_string(),
                        instrument_track.effects.iter().sorted_by(|entry1, entry2| {
                            entry1.name().cmp(entry2.name())
                        })
                            .map(|effect| {
                                (effect.uuid(), effect.name().to_string())
                            })
                            .collect()
                    ),
                    _ => ("None".to_string(), vec![]),
                };
                (track.name().to_string(), instrument_name, effects)
            }
            else {
                ("Unknown".to_string(), "None".to_string(), vec![])
            }
        }
        else {
            ("Unknown".to_string(), "None".to_string(), vec![])
        };
        (selected_track_uuid.clone(), track_name, instrument_name, effects)
    }
    else {
        ("Unknown".to_string(), "Unknown".to_string(), "None".to_string(), vec![])
    };

    let track_uuid_riff_add = track_uuid.clone();
    let track_uuid_riff_name_change = track_uuid.clone();
    let track_uuid_riff_length_change = track_uuid.clone();

    let track_effect_plugins_list =
        effects
            .iter()
            .map(|(x, y)| {
                let audio_plugin_uuid = x.clone();
                let audio_plugin_name = y.clone();
                let audio_plugin_uuid2 = x.clone();
                let audio_plugin_name2 = y.clone();
                flex_row((
                    text_button("X", move |state: &mut RiffDAWState| {
                        if let Some(selected_track_uuid) = state.selected_track.as_ref() {
                            println!("Delete effect: track uuid={}, x={}, y={}", selected_track_uuid, audio_plugin_uuid.as_str(), audio_plugin_name.as_str());
                            track_change_type_EffectDeleted(state, audio_plugin_uuid.as_str().to_string(), Some(selected_track_uuid.to_string()));
                        }
                    }),
                    text_button("I", move |state: &mut RiffDAWState| {
                        if let Some(selected_track_uuid) = state.selected_track.as_ref() {
                            println!("Show effect UI: track uuid={}, x={}, y={}", selected_track_uuid, audio_plugin_uuid2.as_str(), audio_plugin_name2.as_str());
                            if let Some(selected_track_uuid) = state.selected_track.as_ref() {
                                if let Some(sender) = state.audio_layer_sender.as_mut() {
                                    let _ = sender.send(AudioLayerEvent::TrackBackgroundProcessorInward(TrackBackgroundProcessorInwardEvent::ShowEffect(audio_plugin_uuid2.clone()), selected_track_uuid.clone()));
                                }
                            }
                        }
                    }),
                    label(y.to_string()),
                    FlexSpacer::Flex(1.0),
                ))
            })
            .collect::<Vec<_>>();

    let mut selected_riff_name = "None".to_string();
    let mut selected_riff_uuid_1 = "None".to_string();
    if let Some(selected_riff_uuid) = state.selected_riff_uuid_map.get(&track_uuid) {
        // get the riff name
        if let Ok(project) = state.project().as_ref().lock() {
            if let Some(track) = project.song().tracks().iter().find(|track| track.uuid() == track_uuid) {
                if let Some(riff) = track.riffs().iter().find(|riff| riff.uuid().to_string() == *selected_riff_uuid) {
                    selected_riff_name = riff.name().to_string();
                    selected_riff_uuid_1 = riff.uuid().to_string();
                }
            }

        }
    }

    let selected_riff_uuid_riff_name_change = selected_riff_uuid_1.clone();
    let selected_riff_uuid_riff_length_change = selected_riff_uuid_1.clone();

    let mut track_riffs = vec![];
    if let Ok(project) = state.project.lock() {
        if let Some(track) = project.song.tracks().iter().find(|track| track.uuid() == track_uuid) {
            track_riffs = track.riffs().iter()
                .filter(|riff| riff.name() != "empty".to_string())
                .map(|riff| {
                    let riff_uuid = riff.uuid().to_string();
                    let track_uuid2 = track_uuid.clone();
                    flex_row((
                        text_button("S", move |state: &mut RiffDAWState| {
                            state.selected_riff_uuid_map.insert(track_uuid2.clone(), riff_uuid.clone());
                        }),
                        label(riff.name().to_string()),
                        FlexSpacer::Flex(1.0),
                    ))
                })
                .collect::<Vec<_>>();
        }
    }

    let riff_length_options = MUSICAL_ITEM_LENGTH_OPTIONS.iter()
        .map(|riff_length_option| flex_row((
            text_button("S", move |state: &mut RiffDAWState| {
                state.track_detail_view_state.add_riff_length_text = riff_length_option.to_string();
                state.track_detail_view_state.add_riff_length = if let Ok(project) = state.project().as_ref().lock() {
                    DAWUtils::get_snap_quantise_value_in_beats_from_choice_text(
                        riff_length_option, project.song().time_signature_denominator())
                }
                else { 4.0 }
            }),
            label(riff_length_option.to_string()),
        )).main_axis_alignment(MainAxisAlignment::Start).cross_axis_alignment(CrossAxisAlignment::Start).must_fill_major_axis(true))
        .collect::<Vec<_>>();

    (
        flex_row((
            label("Track:"),
            label(track_name),
            FlexSpacer::Flex(1.0)
        )).main_axis_alignment(MainAxisAlignment::Start).cross_axis_alignment(CrossAxisAlignment::Start),
        flex_row((
            label("Riffs"),
            FlexSpacer::Flex(1.0)
        )).main_axis_alignment(MainAxisAlignment::Start).cross_axis_alignment(CrossAxisAlignment::Start),
        flex_row((
            sized_box(label("-----------")).width(200.px()),
            FlexSpacer::Flex(1.0)
        )),
        flex_row((
            sized_box(label("Selected:")).width(200.px()),
            sized_box(text_input(selected_riff_name, move |state: &mut RiffDAWState, name: String| {
                track_change_type_RiffNameChange(state, selected_riff_uuid_riff_name_change.clone(), name, Some(track_uuid_riff_name_change.clone()));
            })).width(200.px()),
            sized_box(text_button("Update", move |state: &mut RiffDAWState| {
                track_change_type_RiffLengthChange(state, selected_riff_uuid_riff_length_change.clone(), state.track_detail_view_state.add_riff_length, Some(track_uuid_riff_length_change.clone()));
            })).width(80.px()),
            sized_box(label("Add riff")).width(200.px()),
            sized_box(text_input(state.track_detail_view_state.add_riff_name.clone(), |state: &mut RiffDAWState, text: String| {
                state.track_detail_view_state.add_riff_name = text;
            })).width(200.px()),
            label("Selected riff length:"),
            label(format!("{}", state.track_detail_view_state.add_riff_length_text.as_str())),
            sized_box(text_button("Add", move |state: &mut RiffDAWState| {
                track_change_type_RiffAdd(state, state.track_detail_view_state.add_riff_name.clone(), state.track_detail_view_state.add_riff_length.clone(), Some(track_uuid_riff_add.clone()));
                state.track_detail_view_state.add_riff_name.clear();
            })).width(80.px()),
            FlexSpacer::Flex(1.0)
        )).main_axis_alignment(MainAxisAlignment::Start).cross_axis_alignment(CrossAxisAlignment::Start),
        flex_row((
            sized_box(label("Available Riffs")).width(200.px()),
            sized_box(
                portal(
                    flex_col((
                        track_riffs,
                    ))
                )
            ).height(200.px()).width(200.px()),
            FlexSpacer::Fixed(80.px()),
            sized_box(label("Available Riff Lengths")).width(200.px()),
            sized_box(
                portal(
                    flex_col((
                        riff_length_options,
                    ))
                )
            ).height(200.px()).width(200.px()),
            FlexSpacer::Flex(1.0)
        )).main_axis_alignment(MainAxisAlignment::Start).cross_axis_alignment(CrossAxisAlignment::Start),
        flex_row((
            sized_box(label("Instruments")).width(400.px()),
            sized_box(label("Effects")).width(400.px()),
            FlexSpacer::Flex(1.0)
        )),
        flex_row((
            sized_box(label("-----------")).width(400.px()),
            sized_box(label("--------")).width(400.px()),
            FlexSpacer::Flex(1.0)
        )),
        flex_row((
            sized_box(label("Selected:")).width(200.px()),
            text_button("I", move |state: &mut RiffDAWState| {
                if let Some(sender) = state.audio_layer_sender.as_mut() {
                    let _ = sender.send(AudioLayerEvent::TrackBackgroundProcessorInward(TrackBackgroundProcessorInwardEvent::ShowInstrument, track_uuid.clone()));
                }
            }),
            sized_box(label(instrument_name)).width(200.px()),
            sized_box(label(" ")).width(200.px()),
            sized_box(label(" ")).width(200.px()),
            FlexSpacer::Flex(1.0)
        )),
        flex_row((
            sized_box(label("Available Instruments")).width(400.px()),
            sized_box(label("Track Effects")).width(400.px()),
            sized_box(label("Available Effects")).width(400.px()),
            FlexSpacer::Flex(1.0)
        )),
        flex_row((
            sized_box(
                portal(
                    flex_col((
                        available_instrument_plugins_list,
                    ))
                )
            ).height(200.px()).width(400.px()),
            sized_box(
                portal(
                    flex_col((
                        track_effect_plugins_list,
                    ))
                )
            ).height(200.px()).width(400.px()),
            sized_box(
                portal(
                    flex_col((
                        available_effect_plugins_list,
                    ))
                )
            ).height(200.px()).width(400.px()),
            FlexSpacer::Flex(1.0)
        )).main_axis_alignment(MainAxisAlignment::Start).cross_axis_alignment(CrossAxisAlignment::Start),
    )
}