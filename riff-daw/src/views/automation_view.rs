use masonry::properties::types::{AsUnit, Length};
use strum::IntoEnumIterator;
use tracing::Instrument;
use uuid::Uuid;
use xilem::view::{button, flex_col, flex_row, indexed_stack, label, text_button, Flex, FlexSequence, FlexSpacer};
use crate::actions::{handle_automation_controller_copy, handle_automation_controller_cut, handle_automation_controller_paste, handle_automation_controller_translate_selected, handle_automation_effect_copy, handle_automation_effect_cut, handle_automation_effect_paste, handle_automation_effect_translate_selected, handle_automation_instrument_copy, handle_automation_instrument_cut, handle_automation_instrument_paste, handle_automation_instrument_translate_selected, handle_automation_note_expression_copy, handle_automation_note_expression_cut, handle_automation_note_expression_paste, handle_automation_note_expression_translate_selected, handle_automation_note_velocities_translate_selected, handle_automation_pitch_bend_copy, handle_automation_pitch_bend_cut, handle_automation_pitch_bend_paste, handle_automation_pitch_bend_translate_selected, track_change_type_RiffQuantiseSelected};
use crate::constants::{CONTROLLER_TYPES, MUSICAL_ITEM_LENGTH_OPTIONS};
use crate::domain::{AudioEffectTrack, AudioPlugin, NoteExpressionType, PluginParameterDetail, Track, TrackBackgroundProcessorInwardEvent, TrackType};
use crate::event::{AudioLayerEvent, AutomationEditType, OperationModeType, TranslateDirection};
use crate::icons::{ICON_ARROW_DOWN, ICON_ARROW_LEFT, ICON_ARROW_RIGHT, ICON_ARROW_UP, ICON_CLIPBOARD, ICON_COPY, ICON_CUT, ICON_DESELECT, ICON_EDIT, ICON_MINUS, ICON_PLAYER_SKIP_BACK, ICON_PLUS, ICON_POINTER, ICON_SELECT_ALL, ICON_ZOOM};
use crate::state::{AutomationViewMode, MidiPolyphonicExpressionNoteId, NoteExpressionChannel, NoteExpressionKey, NoteExpressionPortIndex, RiffDAWState};
use crate::views::{automation_grid_with_size, generic_selector, icon, BeatGrid, DrawMode};


fn get_effects_data(track_uuid: String, effects: &[AudioPlugin], state: &RiffDAWState) -> (String, Vec<PluginParameterDetail>, Vec<(String, String)>) {
    let mut selected_effect_uuid = String::new();
    let mut effect_plugin_parameters: Vec<PluginParameterDetail> = vec![];


    if let Some(selected_effect) = state.selected_effect_plugin_uuid.as_ref() {
        selected_effect_uuid = selected_effect.clone();

    }

    if let Some(track_audio_plugin_params) = state.audio_plugin_parameters().get(track_uuid.as_str()) {
        if let Some(plugin_params) = track_audio_plugin_params.get(selected_effect_uuid.as_str()) {
            for x in plugin_params.iter() {
                effect_plugin_parameters.push(x.clone());
            }
        }
        else {
            effect_plugin_parameters.push(PluginParameterDetail {
                index: 0,
                name: "".to_string(),
                label: "".to_string(),
                text: "".to_string(),
            });
        }
    }

    (selected_effect_uuid, effect_plugin_parameters, effects.iter().map(|effect| (effect.uuid(), effect.name().to_string())).collect())
}

pub fn automation_view_toolbar(
    data: &RiffDAWState,
) -> Flex<impl FlexSequence<RiffDAWState>, RiffDAWState> {
    let note_expression_types = NoteExpressionType::iter().map(|enum_type| enum_type.to_string()).collect::<Vec<String>>();
    let note_expression_note_ids = MidiPolyphonicExpressionNoteId::iter().map(|enum_type| enum_type.to_string()).collect::<Vec<String>>();
    let note_expression_port_indexes = NoteExpressionPortIndex::iter().map(|enum_type| enum_type.to_string()).collect::<Vec<String>>();
    let note_expression_channel_ids = NoteExpressionChannel::iter().map(|enum_type| enum_type.to_string()).collect::<Vec<String>>();
    let note_expression_keys = NoteExpressionKey::iter().map(|enum_type| enum_type.to_string()).collect::<Vec<String>>();
    let mut instrument_name = "None".to_string();
    let mut instrument_plugin_parameters: Vec<PluginParameterDetail> = vec![];
    let mut selected_effect_uuid = "None".to_string();
    let mut selected_effect_name = "None".to_string();
    let mut selected_effect_plugin_parameters: Vec<PluginParameterDetail> = vec![];
    let mut effects_plugins_details: Vec<(String, String)> = vec![];

    if let Some(track_uuid) = data.selected_track.as_ref() {
        if let Ok(project) = data.project.lock().as_ref() {
            if let Some(track) = project.song().track(track_uuid.clone()) {
                match track {
                    TrackType::InstrumentTrack(instrument_track) => {
                        instrument_name = instrument_track.instrument.name.clone();
                        let instrument_uuid = instrument_track.instrument().uuid.to_string();
                        if let Some(track_audio_plugin_params) = data.audio_plugin_parameters().get(track_uuid.as_str()) {
                            if let Some(plugin_params) = track_audio_plugin_params.get(instrument_uuid.as_str()) {
                                for x in plugin_params.iter() {
                                    instrument_plugin_parameters.push(x.clone());
                                }
                            }
                        }

                        let (effect_uuid, effect_params, effects_details) = get_effects_data(track_uuid.clone(), instrument_track.effects(), data);
                        selected_effect_uuid = effect_uuid;
                        selected_effect_plugin_parameters = effect_params;
                        effects_plugins_details = effects_details;
                    }
                    TrackType::AudioTrack(audio_track) => {
                        let (effect_uuid, effect_params, effects_details) = get_effects_data(track_uuid.clone(), audio_track.effects(), data);
                        selected_effect_uuid = effect_uuid;
                        selected_effect_plugin_parameters = effect_params;
                        effects_plugins_details = effects_details;
                    }
                    _ => {}
                }
            }
        }
    }

    if instrument_plugin_parameters.iter().count() == 0 {
        instrument_plugin_parameters.push(PluginParameterDetail {
            index: 0,
            name: "".to_string(),
            label: "".to_string(),
            text: "".to_string(),
        });
    }

    if let Some(track_uuid) = data.selected_track.as_ref() {
        if let Ok(project) = data.project.lock().as_ref() {
            if let Some(track) = project.song().track(track_uuid.clone()) {
                // FIXME need to adapt this to the selected effects plugin
                if let TrackType::InstrumentTrack(instrument_track)  = track {
                    selected_effect_name = instrument_track.instrument.name.clone();
                    let instrument_uuid = instrument_track.instrument().uuid.to_string();
                    if let Some(track_audio_plugin_params) = data.audio_plugin_parameters().get(track_uuid.as_str()) {
                        if let Some(plugin_params) = track_audio_plugin_params.get(instrument_uuid.as_str()) {
                            for x in plugin_params.iter() {
                                selected_effect_plugin_parameters.push(x.clone());
                            }
                        }
                    }
                }
            }
        }
    }

    if selected_effect_plugin_parameters.iter().count() == 0 {
        selected_effect_plugin_parameters.push(PluginParameterDetail {
            index: 0,
            name: "".to_string(),
            label: "".to_string(),
            text: "".to_string(),
        });
    }

    flex_col(
        (
            flex_row(
                (
                    flex_row(
                        (
                            button(icon(ICON_POINTER.to_string()), |state: &mut RiffDAWState| state.automation_view_state.automation_grid_operation_mode = OperationModeType::PointMode),
                            button(icon(ICON_PLUS.to_string()), |state: &mut RiffDAWState| state.automation_view_state.automation_grid_operation_mode = OperationModeType::Add),
                            button(icon(ICON_MINUS.to_string()), |state: &mut RiffDAWState| state.automation_view_state.automation_grid_operation_mode = OperationModeType::Delete),
                            button(icon(ICON_EDIT.to_string()), |state: &mut RiffDAWState| state.automation_view_state.automation_grid_operation_mode = OperationModeType::Change),
                            button(icon(ICON_PLAYER_SKIP_BACK.to_string()), |state: &mut RiffDAWState| state.automation_view_state.automation_grid_operation_mode = OperationModeType::SelectRiffReferenceMode),
                            button(icon(ICON_ZOOM.to_string()), |state: &mut RiffDAWState| state.automation_view_state.automation_grid_operation_mode = OperationModeType::WindowedZoom),
                        )
                    ).gap(1.px()),
                    flex_row(
                        (
                            button(icon(ICON_CUT.to_string()), |state: &mut RiffDAWState| {
                                match state.automation_view_state.automation_view_mode {
                                    AutomationViewMode::NoteVelocities => {}
                                    AutomationViewMode::Controllers => {
                                        handle_automation_controller_cut(state, state.automation_view_state.automation_edit_cursor_time_in_beats);
                                    }
                                    AutomationViewMode::PitchBend => {
                                        handle_automation_pitch_bend_cut(state, state.automation_view_state.automation_edit_cursor_time_in_beats);
                                    }
                                    AutomationViewMode::Instrument => {
                                        handle_automation_instrument_cut(state, state.automation_view_state.automation_edit_cursor_time_in_beats);
                                    }
                                    AutomationViewMode::Effect => {
                                        handle_automation_effect_cut(state, state.automation_view_state.automation_edit_cursor_time_in_beats);
                                    }
                                    AutomationViewMode::NoteExpression => {
                                        handle_automation_note_expression_cut(state, state.automation_view_state.automation_edit_cursor_time_in_beats);
                                    }
                                }
                            }),
                            button(icon(ICON_COPY.to_string()), |state: &mut RiffDAWState| {
                                match state.automation_view_state.automation_view_mode {
                                    AutomationViewMode::NoteVelocities => {
                                    }
                                    AutomationViewMode::Controllers => {
                                        handle_automation_controller_copy(state, state.automation_view_state.automation_edit_cursor_time_in_beats);
                                    }
                                    AutomationViewMode::PitchBend => {
                                        handle_automation_pitch_bend_copy(state, state.automation_view_state.automation_edit_cursor_time_in_beats);
                                    }
                                    AutomationViewMode::Instrument => {
                                        handle_automation_instrument_copy(state, state.automation_view_state.automation_edit_cursor_time_in_beats);
                                    }
                                    AutomationViewMode::Effect => {
                                        handle_automation_effect_copy(state, state.automation_view_state.automation_edit_cursor_time_in_beats);
                                    }
                                    AutomationViewMode::NoteExpression => {
                                        handle_automation_note_expression_copy(state, state.automation_view_state.automation_edit_cursor_time_in_beats);
                                    }
                                }
                            }),
                            button(icon(ICON_CLIPBOARD.to_string()), |state: &mut RiffDAWState| {
                                match state.automation_view_state.automation_view_mode {
                                    AutomationViewMode::NoteVelocities => {

                                    }
                                    AutomationViewMode::Controllers => {
                                        handle_automation_controller_paste(state, state.automation_view_state.automation_edit_cursor_time_in_beats);
                                    }
                                    AutomationViewMode::PitchBend => {
                                        handle_automation_pitch_bend_paste(state, state.automation_view_state.automation_edit_cursor_time_in_beats);
                                    }
                                    AutomationViewMode::Instrument => {
                                        handle_automation_instrument_paste(state, state.automation_view_state.automation_edit_cursor_time_in_beats);
                                    }
                                    AutomationViewMode::Effect => {
                                        handle_automation_effect_paste(state, state.automation_view_state.automation_edit_cursor_time_in_beats);
                                    }
                                    AutomationViewMode::NoteExpression => {
                                        handle_automation_note_expression_paste(state, state.automation_view_state.automation_edit_cursor_time_in_beats);
                                    }
                                }
                            }),
                            button(icon(ICON_SELECT_ALL.to_string()), |state: &mut RiffDAWState| {
                                match state.automation_view_state.automation_view_mode {
                                    AutomationViewMode::NoteVelocities => {

                                    }
                                    AutomationViewMode::Controllers => {

                                    }
                                    AutomationViewMode::PitchBend => {

                                    }
                                    AutomationViewMode::Instrument => {

                                    }
                                    AutomationViewMode::Effect => {

                                    }
                                    AutomationViewMode::NoteExpression => {

                                    }
                                }
                            }),
                            button(icon(ICON_DESELECT.to_string()), |state: &mut RiffDAWState| {
                                match state.automation_view_state.automation_view_mode {
                                    AutomationViewMode::NoteVelocities => {

                                    }
                                    AutomationViewMode::Controllers => {

                                    }
                                    AutomationViewMode::PitchBend => {

                                    }
                                    AutomationViewMode::Instrument => {

                                    }
                                    AutomationViewMode::Effect => {

                                    }
                                    AutomationViewMode::NoteExpression => {

                                    }
                                }
                            }),
                        )
                    ).gap(1.px()),
                    flex_row(
                        (
                            button(icon(ICON_ARROW_LEFT.to_string()), |state: &mut RiffDAWState| {
                                match state.automation_view_state.automation_view_mode {
                                    AutomationViewMode::NoteVelocities => {
                                    }
                                    AutomationViewMode::Controllers => {
                                        handle_automation_controller_translate_selected(state, TranslateDirection::Left, 1.0);
                                    }
                                    AutomationViewMode::PitchBend => {
                                        handle_automation_pitch_bend_translate_selected(state, TranslateDirection::Left, 1.0);
                                    }
                                    AutomationViewMode::Instrument => {
                                        handle_automation_instrument_translate_selected(state, TranslateDirection::Left, 1.0);
                                    }
                                    AutomationViewMode::Effect => {
                                        handle_automation_effect_translate_selected(state, TranslateDirection::Left, 1.0);
                                    }
                                    AutomationViewMode::NoteExpression => {
                                        handle_automation_note_expression_translate_selected(state, TranslateDirection::Left, 1.0);
                                    }
                                }
                            }),
                            button(icon(ICON_ARROW_RIGHT.to_string()), |state: &mut RiffDAWState| {
                                match state.automation_view_state.automation_view_mode {
                                    AutomationViewMode::NoteVelocities => {
                                    }
                                    AutomationViewMode::Controllers => {
                                        handle_automation_controller_translate_selected(state, TranslateDirection::Right, 1.0);
                                    }
                                    AutomationViewMode::PitchBend => {
                                        handle_automation_pitch_bend_translate_selected(state, TranslateDirection::Right, 1.0);
                                    }
                                    AutomationViewMode::Instrument => {
                                        handle_automation_instrument_translate_selected(state, TranslateDirection::Right, 1.0);
                                    }
                                    AutomationViewMode::Effect => {
                                        handle_automation_effect_translate_selected(state, TranslateDirection::Right, 1.0);
                                    }
                                    AutomationViewMode::NoteExpression => {
                                        handle_automation_note_expression_translate_selected(state, TranslateDirection::Right, 1.0);
                                    }
                                }
                            }),
                            button(icon(ICON_ARROW_UP.to_string()), |state: &mut RiffDAWState| {
                                match state.automation_view_state.automation_view_mode {
                                    AutomationViewMode::NoteVelocities => {
                                        handle_automation_note_velocities_translate_selected(state, TranslateDirection::Up);
                                    }
                                    AutomationViewMode::Controllers => {
                                        handle_automation_controller_translate_selected(state, TranslateDirection::Up, 1.0);
                                    }
                                    AutomationViewMode::PitchBend => {
                                        handle_automation_pitch_bend_translate_selected(state, TranslateDirection::Up, 1.0);
                                    }
                                    AutomationViewMode::Instrument => {
                                        handle_automation_instrument_translate_selected(state, TranslateDirection::Up, 1.0);
                                    }
                                    AutomationViewMode::Effect => {
                                        handle_automation_effect_translate_selected(state, TranslateDirection::Up, 1.0);
                                    }
                                    AutomationViewMode::NoteExpression => {
                                        handle_automation_note_expression_translate_selected(state, TranslateDirection::Up, 1.0);
                                    }
                                }
                            }),
                            button(icon(ICON_ARROW_DOWN.to_string()), |state: &mut RiffDAWState| {
                                match state.automation_view_state.automation_view_mode {
                                    AutomationViewMode::NoteVelocities => {
                                        handle_automation_note_velocities_translate_selected(state, TranslateDirection::Down);
                                    }
                                    AutomationViewMode::Controllers => {
                                        handle_automation_controller_translate_selected(state, TranslateDirection::Down, 1.0);
                                    }
                                    AutomationViewMode::PitchBend => {
                                        handle_automation_pitch_bend_translate_selected(state, TranslateDirection::Down, 1.0);
                                    }
                                    AutomationViewMode::Instrument => {
                                        handle_automation_instrument_translate_selected(state, TranslateDirection::Down, 1.0);
                                    }
                                    AutomationViewMode::Effect => {
                                        handle_automation_effect_translate_selected(state, TranslateDirection::Down, 1.0);
                                    }
                                    AutomationViewMode::NoteExpression => {
                                        handle_automation_note_expression_translate_selected(state, TranslateDirection::Down, 1.0);
                                    }
                                }
                            }),
                        )
                    ).gap(1.px()),
                    flex_row(
                        (
                            label("Quantize and Snap"),
                            generic_selector(
                                MUSICAL_ITEM_LENGTH_OPTIONS.to_vec(), data.piano_roll_state.piano_roll_selected_snap,
                                |state: &mut RiffDAWState| {
                                    let mut new_index: i32 = state.piano_roll_state.piano_roll_selected_snap as i32 - 1;
                                    if new_index < 0 {
                                        new_index = 0;
                                    }
                                    state.piano_roll_state.piano_roll_selected_snap =  new_index as usize;
                                    println!("Quantize and Snap: index={}", state.piano_roll_state.piano_roll_selected_snap);
                                },
                                |state: &mut RiffDAWState| {
                                    let mut new_index = state.piano_roll_state.piano_roll_selected_snap + 1;
                                    if new_index >= MUSICAL_ITEM_LENGTH_OPTIONS.len() {
                                        new_index = 0;
                                    }
                                    state.piano_roll_state.piano_roll_selected_snap =  new_index;
                                    println!("Quantize and Snap: index={}", state.piano_roll_state.piano_roll_selected_snap);
                                }
                            ),
                            text_button("Q", |state: &mut RiffDAWState| track_change_type_RiffQuantiseSelected(state)),
                        )
                    ).gap(1.px()),
                    FlexSpacer::Flex(1.0)
                )
            ).gap(Length::px(10.)),
            flex_row(
                (
                    flex_row(
                        (
                            text_button("Note Velocities", |state: &mut RiffDAWState| {
                                state.automation_view_state.automation_view_mode = AutomationViewMode::NoteVelocities;
                            }),
                            text_button("Note Expression", |state: &mut RiffDAWState| {
                                state.automation_view_state.automation_view_mode = AutomationViewMode::NoteExpression;
                            }),
                            text_button("Controllers", |state: &mut RiffDAWState| {
                                state.automation_view_state.automation_view_mode = AutomationViewMode::Controllers;
                            }),
                            text_button("Pitch Bend", |state: &mut RiffDAWState| {
                                state.automation_view_state.automation_view_mode = AutomationViewMode::PitchBend;
                            }),
                            text_button("Instrument", |state: &mut RiffDAWState| {
                                state.automation_view_state.automation_view_mode = AutomationViewMode::Instrument;
                            }),
                            text_button("Effects", |state: &mut RiffDAWState| {
                                state.automation_view_state.automation_view_mode = AutomationViewMode::Effect;
                            }),
                        )
                    ).gap(1.px()),
                    flex_row(
                        (
                            text_button("Track", |state: &mut RiffDAWState| {
                                state.automation_view_state.automation_edit_type = AutomationEditType::Track;
                            }),
                            text_button("Riff", |state: &mut RiffDAWState| {
                                state.automation_view_state.automation_edit_type = AutomationEditType::Riff;
                            }),
                        )
                    ).gap(1.px()),
                    flex_row(
                        (
                            text_button("Point", |state: &mut RiffDAWState| {
                                state.automation_view_state.draw_mode = DrawMode::Point;
                            }),
                            text_button("Line", |state: &mut RiffDAWState| {
                                state.automation_view_state.draw_mode = DrawMode::Line;
                            }),
                            text_button("Curve", |state: &mut RiffDAWState| {
                                state.automation_view_state.draw_mode = DrawMode::Curve;
                            }),
                        )
                    ).gap(1.px()),
                    flex_row(
                        (
                            text_button("Discrete", |state: &mut RiffDAWState| {
                                state.automation_view_state.automation_discrete = true;
                            }),
                            text_button("Continuous", |state: &mut RiffDAWState| {
                                state.automation_view_state.automation_discrete = false;
                            }),
                        )
                    ).gap(1.px()),
                    FlexSpacer::Flex(1.0)
                )
            ).gap(Length::px(10.)),
            flex_row(
                indexed_stack((
                    // note velocities
                    flex_row(
                        (
                        )
                    ).gap(1.px()),
                    // note expression
                    flex_row(
                        (
                            label("Note Expression"),
                            generic_selector(
                                note_expression_types.iter().map(|enum_type| enum_type.as_str()).collect(),
                                data.automation_view_state.note_expression_type as usize,
                                |state: &mut RiffDAWState| {
                                    let mut new_index = state.automation_view_state.note_expression_type as i32 - 1;
                                    if new_index < 0 {
                                        new_index = 0;
                                    }
                                    state.automation_view_state.note_expression_type =  match new_index {
                                        0 => NoteExpressionType::Volume,
                                        1 => NoteExpressionType::Pan,
                                        2 => NoteExpressionType::Tuning,
                                        3 => NoteExpressionType::Vibrato,
                                        4 => NoteExpressionType::Expression,
                                        5 => NoteExpressionType::Pressure,
                                        6 => NoteExpressionType::Brightness,
                                        _ => panic!("Automation: note_expression_type unknown value: {}", new_index),
                                    };
                                    println!("Automation: note_expression_type={}", state.automation_view_state.note_expression_type.to_string());
                                },
                                |state: &mut RiffDAWState| {
                                    let mut new_index = state.automation_view_state.note_expression_type as i32 + 1;
                                    if new_index >= NoteExpressionType::iter().len() as i32 {
                                        new_index = 0;
                                    }
                                    state.automation_view_state.note_expression_type =  match new_index {
                                        0 => NoteExpressionType::Volume,
                                        1 => NoteExpressionType::Pan,
                                        2 => NoteExpressionType::Tuning,
                                        3 => NoteExpressionType::Vibrato,
                                        4 => NoteExpressionType::Expression,
                                        5 => NoteExpressionType::Pressure,
                                        6 => NoteExpressionType::Brightness,
                                        _ => panic!("Automation: note_expression_type unknown value: {}", new_index),
                                    };
                                    println!("Automation: note_expression_type={}", state.automation_view_state.note_expression_type.to_string());
                                }
                            ),
                            label("Note ID"),
                            generic_selector(
                                note_expression_note_ids.iter().map(|enum_type| enum_type.as_str()).collect(),
                                (data.automation_view_state.note_expression_id.clone() as i32 + 1) as usize,
                                |state: &mut RiffDAWState| {
                                    let mut new_index = state.automation_view_state.note_expression_id.clone() as i32 - 1;
                                    if new_index < -1 {
                                        new_index = -1;
                                    }
                                    state.automation_view_state.note_expression_id =  match new_index {
                                        -1 => MidiPolyphonicExpressionNoteId::ALL,
                                        0 => MidiPolyphonicExpressionNoteId::NoteId0,
                                        1 => MidiPolyphonicExpressionNoteId::NoteId1,
                                        2 => MidiPolyphonicExpressionNoteId::NoteId2,
                                        3 => MidiPolyphonicExpressionNoteId::NoteId3,
                                        4 => MidiPolyphonicExpressionNoteId::NoteId4,
                                        5 => MidiPolyphonicExpressionNoteId::NoteId5,
                                        6 => MidiPolyphonicExpressionNoteId::NoteId6,
                                        7 => MidiPolyphonicExpressionNoteId::NoteId7,
                                        8 => MidiPolyphonicExpressionNoteId::NoteId8,
                                        9 => MidiPolyphonicExpressionNoteId::NoteId9,
                                        10 => MidiPolyphonicExpressionNoteId::NoteId10,
                                        _ => panic!("Automation: note_expression_id unknown value: {}", new_index),
                                    };
                                    println!("Automation: note_expression_id={}", state.automation_view_state.note_expression_id.to_string());
                                },
                                |state: &mut RiffDAWState| {
                                    let mut new_index = state.automation_view_state.note_expression_id.clone() as i32 + 1;
                                    if new_index > 10 {
                                        new_index = -1;
                                    }
                                    state.automation_view_state.note_expression_id =  match new_index {
                                        -1 => MidiPolyphonicExpressionNoteId::ALL,
                                        0 => MidiPolyphonicExpressionNoteId::NoteId0,
                                        1 => MidiPolyphonicExpressionNoteId::NoteId1,
                                        2 => MidiPolyphonicExpressionNoteId::NoteId2,
                                        3 => MidiPolyphonicExpressionNoteId::NoteId3,
                                        4 => MidiPolyphonicExpressionNoteId::NoteId4,
                                        5 => MidiPolyphonicExpressionNoteId::NoteId5,
                                        6 => MidiPolyphonicExpressionNoteId::NoteId6,
                                        7 => MidiPolyphonicExpressionNoteId::NoteId7,
                                        8 => MidiPolyphonicExpressionNoteId::NoteId8,
                                        9 => MidiPolyphonicExpressionNoteId::NoteId9,
                                        10 => MidiPolyphonicExpressionNoteId::NoteId10,
                                        _ => panic!("Automation: note_expression_id unknown value: {}", new_index),
                                    };
                                    println!("Automation: note_expression_id={}", state.automation_view_state.note_expression_id.to_string());
                                }
                            ),
                            label("Port Index"),
                            generic_selector(
                                note_expression_port_indexes.iter().map(|enum_type| enum_type.as_str()).collect(),
                                (data.automation_view_state.note_expression_port_index.clone() as i32 + 1) as usize,
                                |state: &mut RiffDAWState| {
                                    let mut new_index = state.automation_view_state.note_expression_port_index.clone() as i32 - 1;
                                    if new_index < -1 {
                                        new_index = -1;
                                    }
                                    state.automation_view_state.note_expression_port_index =  match new_index {
                                        -1 => NoteExpressionPortIndex::Global,
                                        0 => NoteExpressionPortIndex::PortIndex0,
                                        1 => NoteExpressionPortIndex::PortIndex1,
                                        2 => NoteExpressionPortIndex::PortIndex2,
                                        3 => NoteExpressionPortIndex::PortIndex3,
                                        4 => NoteExpressionPortIndex::PortIndex4,
                                        5 => NoteExpressionPortIndex::PortIndex5,
                                        6 => NoteExpressionPortIndex::PortIndex6,
                                        7 => NoteExpressionPortIndex::PortIndex7,
                                        8 => NoteExpressionPortIndex::PortIndex8,
                                        9 => NoteExpressionPortIndex::PortIndex9,
                                        10 => NoteExpressionPortIndex::PortIndex10,
                                        _ => panic!("Automation: note_expression_port_index unknown value: {}", new_index),
                                    };
                                    println!("Automation: note_expression_port_index={}", state.automation_view_state.note_expression_port_index.to_string());
                                },
                                |state: &mut RiffDAWState| {
                                    let mut new_index = state.automation_view_state.note_expression_port_index.clone() as i32 + 1;
                                    if new_index > 10 {
                                        new_index = -1;
                                    }
                                    state.automation_view_state.note_expression_port_index =  match new_index {
                                        -1 => NoteExpressionPortIndex::Global,
                                        0 => NoteExpressionPortIndex::PortIndex0,
                                        1 => NoteExpressionPortIndex::PortIndex1,
                                        2 => NoteExpressionPortIndex::PortIndex2,
                                        3 => NoteExpressionPortIndex::PortIndex3,
                                        4 => NoteExpressionPortIndex::PortIndex4,
                                        5 => NoteExpressionPortIndex::PortIndex5,
                                        6 => NoteExpressionPortIndex::PortIndex6,
                                        7 => NoteExpressionPortIndex::PortIndex7,
                                        8 => NoteExpressionPortIndex::PortIndex8,
                                        9 => NoteExpressionPortIndex::PortIndex9,
                                        10 => NoteExpressionPortIndex::PortIndex10,
                                        _ => panic!("Automation: note_expression_port_index unknown value: {}", new_index),
                                    };
                                    println!("Automation: note_expression_port_index={}", state.automation_view_state.note_expression_port_index.to_string());
                                }
                            ),
                            label("Channel"),
                            generic_selector(
                                note_expression_channel_ids.iter().map(|enum_type| enum_type.as_str()).collect(),
                                (data.automation_view_state.note_expression_channel.clone() as i32 + 1) as usize,
                                |state: &mut RiffDAWState| {
                                    let mut new_index = state.automation_view_state.note_expression_channel.clone() as i32 - 1;
                                    if new_index < -1 {
                                        new_index = -1;
                                    }
                                    state.automation_view_state.note_expression_channel =  match new_index {
                                        -1 => NoteExpressionChannel::Global,
                                        0 => NoteExpressionChannel::Channel0,
                                        1 => NoteExpressionChannel::Channel1,
                                        2 => NoteExpressionChannel::Channel2,
                                        3 => NoteExpressionChannel::Channel3,
                                        4 => NoteExpressionChannel::Channel4,
                                        5 => NoteExpressionChannel::Channel5,
                                        6 => NoteExpressionChannel::Channel6,
                                        7 => NoteExpressionChannel::Channel7,
                                        8 => NoteExpressionChannel::Channel8,
                                        9 => NoteExpressionChannel::Channel9,
                                        10 => NoteExpressionChannel::Channel10,
                                        _ => panic!("Automation: note_expression_channel unknown value: {}", new_index),
                                    };
                                    println!("Automation: note_expression_channel={}", state.automation_view_state.note_expression_channel.to_string());
                                },
                                |state: &mut RiffDAWState| {
                                    let mut new_index = state.automation_view_state.note_expression_channel.clone() as i32 + 1;
                                    if new_index > 10 {
                                        new_index = -1;
                                    }
                                    state.automation_view_state.note_expression_channel =  match new_index {
                                        -1 => NoteExpressionChannel::Global,
                                        0 => NoteExpressionChannel::Channel0,
                                        1 => NoteExpressionChannel::Channel1,
                                        2 => NoteExpressionChannel::Channel2,
                                        3 => NoteExpressionChannel::Channel3,
                                        4 => NoteExpressionChannel::Channel4,
                                        5 => NoteExpressionChannel::Channel5,
                                        6 => NoteExpressionChannel::Channel6,
                                        7 => NoteExpressionChannel::Channel7,
                                        8 => NoteExpressionChannel::Channel8,
                                        9 => NoteExpressionChannel::Channel9,
                                        10 => NoteExpressionChannel::Channel10,
                                        _ => panic!("Automation: note_expression_channel unknown value: {}", new_index),
                                    };
                                    println!("Automation: note_expression_channel={}", state.automation_view_state.note_expression_channel.to_string());
                                }
                            ),
                            label("Key"),
                            generic_selector(
                                note_expression_keys.iter().map(|enum_type| enum_type.as_str()).collect(),
                                (data.automation_view_state.note_expression_key.clone() as i32 + 1) as usize,
                                |state: &mut RiffDAWState| {
                                    let mut new_index = state.automation_view_state.note_expression_key.clone() as i32 - 1;
                                    if new_index < -1 {
                                        new_index = -1;
                                    }
                                    state.automation_view_state.note_expression_key =  match new_index {
                                        -1 => NoteExpressionKey::Global,
                                        0 => NoteExpressionKey::Cminus2,
                                        1 => NoteExpressionKey::Csharp_Dbminus2,
                                        2 => NoteExpressionKey::Dminus2,
                                        3 => NoteExpressionKey::Dsharp_Ebminus2,
                                        4 => NoteExpressionKey::Eminus2,
                                        5 => NoteExpressionKey::Fminus2,
                                        6 => NoteExpressionKey::Fsharp_Gbminus2,
                                        7 => NoteExpressionKey::Gminus2,
                                        8 => NoteExpressionKey::Gsharp_Abminus2,
                                        9 => NoteExpressionKey::Aminus2,
                                        10 => NoteExpressionKey::Asharp_Bbminus2,
                                        11 => NoteExpressionKey::Bminus2,
                                        12 => NoteExpressionKey::Cminus1,
                                        13 => NoteExpressionKey::Csharp_Dbminus1,
                                        14 => NoteExpressionKey::Dminus1,
                                        15 => NoteExpressionKey::Dsharp_Ebminus1,
                                        16 => NoteExpressionKey::Eminus1,
                                        17 => NoteExpressionKey::Fminus1,
                                        18 => NoteExpressionKey::Fsharp_Gbminus1,
                                        19 => NoteExpressionKey::Gminus1,
                                        20 => NoteExpressionKey::Gsharp_Abminus1,
                                        21 => NoteExpressionKey::Aminus1,
                                        22 => NoteExpressionKey::Asharp_Bbminus1,
                                        23 => NoteExpressionKey::Bminus1,
                                        24 => NoteExpressionKey::C0,
                                        25 => NoteExpressionKey::Csharp_Db0,
                                        26 => NoteExpressionKey::D0,
                                        27 => NoteExpressionKey::Dsharp_Eb0,
                                        28 => NoteExpressionKey::E0,
                                        29 => NoteExpressionKey::F0,
                                        30 => NoteExpressionKey::Fsharp_Gb0,
                                        31 => NoteExpressionKey::G0,
                                        32 => NoteExpressionKey::Gsharp_Ab0,
                                        33 => NoteExpressionKey::A0,
                                        34 => NoteExpressionKey::Asharp_Bb0,
                                        35 => NoteExpressionKey::B0,
                                        36 => NoteExpressionKey::C1,
                                        37 => NoteExpressionKey::Csharp_Db1,
                                        38 => NoteExpressionKey::D1,
                                        39 => NoteExpressionKey::Dsharp_Eb1,
                                        40 => NoteExpressionKey::E1,
                                        41 => NoteExpressionKey::F1,
                                        42 => NoteExpressionKey::Fsharp_Gb1,
                                        43 => NoteExpressionKey::G1,
                                        44 => NoteExpressionKey::Gsharp_Ab1,
                                        45 => NoteExpressionKey::A1,
                                        46 => NoteExpressionKey::Asharp_Bb1,
                                        47 => NoteExpressionKey::B1,
                                        48 => NoteExpressionKey::C2,
                                        49 => NoteExpressionKey::Csharp_Db2,
                                        50 => NoteExpressionKey::D2,
                                        51 => NoteExpressionKey::Dsharp_Eb2,
                                        52 => NoteExpressionKey::E2,
                                        53 => NoteExpressionKey::F2,
                                        54 => NoteExpressionKey::Fsharp_Gb2,
                                        55 => NoteExpressionKey::G2,
                                        56 => NoteExpressionKey::Gsharp_Ab2,
                                        57 => NoteExpressionKey::A2,
                                        58 => NoteExpressionKey::Asharp_Bb2,
                                        59 => NoteExpressionKey::B2,
                                        60 => NoteExpressionKey::C3,
                                        61 => NoteExpressionKey::Csharp_Db3,
                                        62 => NoteExpressionKey::D3,
                                        63 => NoteExpressionKey::Dsharp_Eb3,
                                        64 => NoteExpressionKey::E3,
                                        65 => NoteExpressionKey::F3,
                                        66 => NoteExpressionKey::Fsharp_Gb3,
                                        67 => NoteExpressionKey::G3,
                                        68 => NoteExpressionKey::Gsharp_Ab3,
                                        69 => NoteExpressionKey::A3,
                                        70 => NoteExpressionKey::Asharp_Bb3,
                                        71 => NoteExpressionKey::B3,
                                        72 => NoteExpressionKey::C4,
                                        73 => NoteExpressionKey::Csharp_Db4,
                                        74 => NoteExpressionKey::D4,
                                        75 => NoteExpressionKey::Dsharp_Eb4,
                                        76 => NoteExpressionKey::E4,
                                        77 => NoteExpressionKey::F4,
                                        78 => NoteExpressionKey::Fsharp_Gb4,
                                        79 => NoteExpressionKey::G4,
                                        80 => NoteExpressionKey::Gsharp_Ab4,
                                        81 => NoteExpressionKey::A4,
                                        82 => NoteExpressionKey::Asharp_Bb4,
                                        83 => NoteExpressionKey::B4,
                                        84 => NoteExpressionKey::C5,
                                        85 => NoteExpressionKey::Csharp_Db5,
                                        86 => NoteExpressionKey::D5,
                                        87 => NoteExpressionKey::Dsharp_Eb5,
                                        88 => NoteExpressionKey::E5,
                                        89 => NoteExpressionKey::F5,
                                        90 => NoteExpressionKey::Fsharp_Gb5,
                                        91 => NoteExpressionKey::G5,
                                        92 => NoteExpressionKey::Gsharp_Ab5,
                                        93 => NoteExpressionKey::A5,
                                        94 => NoteExpressionKey::Asharp_Bb5,
                                        95 => NoteExpressionKey::B5,
                                        96 => NoteExpressionKey::C6,
                                        97 => NoteExpressionKey::Csharp_Db6,
                                        98 => NoteExpressionKey::D6,
                                        99 => NoteExpressionKey::Dsharp_Eb6,
                                        100 => NoteExpressionKey::E6,
                                        101 => NoteExpressionKey::F6,
                                        102 => NoteExpressionKey::Fsharp_Gb6,
                                        103 => NoteExpressionKey::G6,
                                        104 => NoteExpressionKey::Gsharp_Ab6,
                                        105 => NoteExpressionKey::A6,
                                        106 => NoteExpressionKey::Asharp_Bb6,
                                        107 => NoteExpressionKey::B6,
                                        108 => NoteExpressionKey::C7,
                                        109 => NoteExpressionKey::Csharp_Db7,
                                        110 => NoteExpressionKey::D7,
                                        111 => NoteExpressionKey::Dsharp_Eb7,
                                        112 => NoteExpressionKey::E7,
                                        113 => NoteExpressionKey::F7,
                                        114 => NoteExpressionKey::Fsharp_Gb7,
                                        115 => NoteExpressionKey::G7,
                                        116 => NoteExpressionKey::Gsharp_Ab7,
                                        117 => NoteExpressionKey::A7,
                                        118 => NoteExpressionKey::Asharp_Bb7,
                                        119 => NoteExpressionKey::B7,
                                        120 => NoteExpressionKey::C8,
                                        121 => NoteExpressionKey::Csharp_Db8,
                                        122 => NoteExpressionKey::D8,
                                        123 => NoteExpressionKey::Dsharp_Eb8,
                                        124 => NoteExpressionKey::E8,
                                        125 => NoteExpressionKey::F8,
                                        126 => NoteExpressionKey::Fsharp_Gb8,
                                        127 => NoteExpressionKey::G8,
                                        _ => panic!("Automation: note_expression_key unknown value: {}", new_index),
                                    };
                                    println!("Automation: note_expression_key={}", state.automation_view_state.note_expression_key.to_string());
                                },
                                |state: &mut RiffDAWState| {
                                    let mut new_index = state.automation_view_state.note_expression_key.clone() as i32 + 1;
                                    if new_index > 127 {
                                        new_index = -1;
                                    }
                                    state.automation_view_state.note_expression_key =  match new_index {
                                        -1 => NoteExpressionKey::Global,
                                        0 => NoteExpressionKey::Cminus2,
                                        1 => NoteExpressionKey::Csharp_Dbminus2,
                                        2 => NoteExpressionKey::Dminus2,
                                        3 => NoteExpressionKey::Dsharp_Ebminus2,
                                        4 => NoteExpressionKey::Eminus2,
                                        5 => NoteExpressionKey::Fminus2,
                                        6 => NoteExpressionKey::Fsharp_Gbminus2,
                                        7 => NoteExpressionKey::Gminus2,
                                        8 => NoteExpressionKey::Gsharp_Abminus2,
                                        9 => NoteExpressionKey::Aminus2,
                                        10 => NoteExpressionKey::Asharp_Bbminus2,
                                        11 => NoteExpressionKey::Bminus2,
                                        12 => NoteExpressionKey::Cminus1,
                                        13 => NoteExpressionKey::Csharp_Dbminus1,
                                        14 => NoteExpressionKey::Dminus1,
                                        15 => NoteExpressionKey::Dsharp_Ebminus1,
                                        16 => NoteExpressionKey::Eminus1,
                                        17 => NoteExpressionKey::Fminus1,
                                        18 => NoteExpressionKey::Fsharp_Gbminus1,
                                        19 => NoteExpressionKey::Gminus1,
                                        20 => NoteExpressionKey::Gsharp_Abminus1,
                                        21 => NoteExpressionKey::Aminus1,
                                        22 => NoteExpressionKey::Asharp_Bbminus1,
                                        23 => NoteExpressionKey::Bminus1,
                                        24 => NoteExpressionKey::C0,
                                        25 => NoteExpressionKey::Csharp_Db0,
                                        26 => NoteExpressionKey::D0,
                                        27 => NoteExpressionKey::Dsharp_Eb0,
                                        28 => NoteExpressionKey::E0,
                                        29 => NoteExpressionKey::F0,
                                        30 => NoteExpressionKey::Fsharp_Gb0,
                                        31 => NoteExpressionKey::G0,
                                        32 => NoteExpressionKey::Gsharp_Ab0,
                                        33 => NoteExpressionKey::A0,
                                        34 => NoteExpressionKey::Asharp_Bb0,
                                        35 => NoteExpressionKey::B0,
                                        36 => NoteExpressionKey::C1,
                                        37 => NoteExpressionKey::Csharp_Db1,
                                        38 => NoteExpressionKey::D1,
                                        39 => NoteExpressionKey::Dsharp_Eb1,
                                        40 => NoteExpressionKey::E1,
                                        41 => NoteExpressionKey::F1,
                                        42 => NoteExpressionKey::Fsharp_Gb1,
                                        43 => NoteExpressionKey::G1,
                                        44 => NoteExpressionKey::Gsharp_Ab1,
                                        45 => NoteExpressionKey::A1,
                                        46 => NoteExpressionKey::Asharp_Bb1,
                                        47 => NoteExpressionKey::B1,
                                        48 => NoteExpressionKey::C2,
                                        49 => NoteExpressionKey::Csharp_Db2,
                                        50 => NoteExpressionKey::D2,
                                        51 => NoteExpressionKey::Dsharp_Eb2,
                                        52 => NoteExpressionKey::E2,
                                        53 => NoteExpressionKey::F2,
                                        54 => NoteExpressionKey::Fsharp_Gb2,
                                        55 => NoteExpressionKey::G2,
                                        56 => NoteExpressionKey::Gsharp_Ab2,
                                        57 => NoteExpressionKey::A2,
                                        58 => NoteExpressionKey::Asharp_Bb2,
                                        59 => NoteExpressionKey::B2,
                                        60 => NoteExpressionKey::C3,
                                        61 => NoteExpressionKey::Csharp_Db3,
                                        62 => NoteExpressionKey::D3,
                                        63 => NoteExpressionKey::Dsharp_Eb3,
                                        64 => NoteExpressionKey::E3,
                                        65 => NoteExpressionKey::F3,
                                        66 => NoteExpressionKey::Fsharp_Gb3,
                                        67 => NoteExpressionKey::G3,
                                        68 => NoteExpressionKey::Gsharp_Ab3,
                                        69 => NoteExpressionKey::A3,
                                        70 => NoteExpressionKey::Asharp_Bb3,
                                        71 => NoteExpressionKey::B3,
                                        72 => NoteExpressionKey::C4,
                                        73 => NoteExpressionKey::Csharp_Db4,
                                        74 => NoteExpressionKey::D4,
                                        75 => NoteExpressionKey::Dsharp_Eb4,
                                        76 => NoteExpressionKey::E4,
                                        77 => NoteExpressionKey::F4,
                                        78 => NoteExpressionKey::Fsharp_Gb4,
                                        79 => NoteExpressionKey::G4,
                                        80 => NoteExpressionKey::Gsharp_Ab4,
                                        81 => NoteExpressionKey::A4,
                                        82 => NoteExpressionKey::Asharp_Bb4,
                                        83 => NoteExpressionKey::B4,
                                        84 => NoteExpressionKey::C5,
                                        85 => NoteExpressionKey::Csharp_Db5,
                                        86 => NoteExpressionKey::D5,
                                        87 => NoteExpressionKey::Dsharp_Eb5,
                                        88 => NoteExpressionKey::E5,
                                        89 => NoteExpressionKey::F5,
                                        90 => NoteExpressionKey::Fsharp_Gb5,
                                        91 => NoteExpressionKey::G5,
                                        92 => NoteExpressionKey::Gsharp_Ab5,
                                        93 => NoteExpressionKey::A5,
                                        94 => NoteExpressionKey::Asharp_Bb5,
                                        95 => NoteExpressionKey::B5,
                                        96 => NoteExpressionKey::C6,
                                        97 => NoteExpressionKey::Csharp_Db6,
                                        98 => NoteExpressionKey::D6,
                                        99 => NoteExpressionKey::Dsharp_Eb6,
                                        100 => NoteExpressionKey::E6,
                                        101 => NoteExpressionKey::F6,
                                        102 => NoteExpressionKey::Fsharp_Gb6,
                                        103 => NoteExpressionKey::G6,
                                        104 => NoteExpressionKey::Gsharp_Ab6,
                                        105 => NoteExpressionKey::A6,
                                        106 => NoteExpressionKey::Asharp_Bb6,
                                        107 => NoteExpressionKey::B6,
                                        108 => NoteExpressionKey::C7,
                                        109 => NoteExpressionKey::Csharp_Db7,
                                        110 => NoteExpressionKey::D7,
                                        111 => NoteExpressionKey::Dsharp_Eb7,
                                        112 => NoteExpressionKey::E7,
                                        113 => NoteExpressionKey::F7,
                                        114 => NoteExpressionKey::Fsharp_Gb7,
                                        115 => NoteExpressionKey::G7,
                                        116 => NoteExpressionKey::Gsharp_Ab7,
                                        117 => NoteExpressionKey::A7,
                                        118 => NoteExpressionKey::Asharp_Bb7,
                                        119 => NoteExpressionKey::B7,
                                        120 => NoteExpressionKey::C8,
                                        121 => NoteExpressionKey::Csharp_Db8,
                                        122 => NoteExpressionKey::D8,
                                        123 => NoteExpressionKey::Dsharp_Eb8,
                                        124 => NoteExpressionKey::E8,
                                        125 => NoteExpressionKey::F8,
                                        126 => NoteExpressionKey::Fsharp_Gb8,
                                        127 => NoteExpressionKey::G8,
                                        _ => panic!("Automation: note_expression_key unknown value: {}", new_index),
                                    };
                                    println!("Automation: note_expression_key={}", state.automation_view_state.note_expression_key.to_string());
                                }
                            ),
                            FlexSpacer::Flex(1.0)
                        )
                    ).gap(1.px()),
                    // controllers
                    flex_row(
                        (
                            label("Controllers"),
                            generic_selector(
                                CONTROLLER_TYPES.iter().map(|(_, description)| description.clone()).collect(),
                                if let Some(controller_type_index) = data.automation_view_state.controller_type_index.as_ref() { controller_type_index.clone() } else { 0 } as usize,
                                |state: &mut RiffDAWState| {
                                    let mut new_index = if let Some(controller_type_index) = state.automation_view_state.controller_type_index.as_ref() { controller_type_index.clone() } else { 0 } as i32 - 1;
                                    if new_index < 0 {
                                        new_index = 0;
                                    }
                                    state.automation_view_state.controller_type_index =  Some(new_index);
                                    println!("Automation: controller_type={}", if let Some(controller_type_index) = state.automation_view_state.controller_type_index.as_ref() { controller_type_index.clone() } else { 0 }.to_string());
                                },
                                |state: &mut RiffDAWState| {
                                    let mut new_index = if let Some(controller_type) = state.automation_view_state.controller_type_index.as_ref() { controller_type.clone() } else { 0 } as i32 + 1;
                                    if new_index >= CONTROLLER_TYPES.iter().len() as i32 {
                                        new_index = 0;
                                    }
                                    state.automation_view_state.controller_type_index = Some(new_index);
                                    println!("Automation: controller_type={}", if let Some(controller_type_index) = state.automation_view_state.controller_type_index.as_ref() { controller_type_index.clone() } else { 0 }.to_string());
                                }
                            ),
                            FlexSpacer::Flex(1.0)
                        )
                    ).gap(1.px()),
                    // pitch bend
                    flex_row(
                        (
                        )
                    ).gap(1.px()),
                    // instrument parameters
                    flex_row(
                        (
                            label("Instrument"),
                            label(instrument_name.as_str()),
                            generic_selector(
                                instrument_plugin_parameters.iter().map(|plugin_parameter_detail: &PluginParameterDetail| plugin_parameter_detail.name()).collect(),
                                if let Some(instrument_parameter_type) = data.automation_view_state.instrument_parameter_type.as_ref() { instrument_parameter_type.clone() } else { 0 } as usize,
                                |state: &mut RiffDAWState| {
                                    let mut new_index = if let Some(instrument_parameter_type) = state.automation_view_state.instrument_parameter_type.as_ref() { instrument_parameter_type.clone() } else { 0 } as i32 - 1;
                                    if new_index < 0 {
                                        new_index = 0;
                                    }
                                    state.automation_view_state.instrument_parameter_type =  Some(new_index);
                                    println!("Automation: instrument_parameter_type={}", if let Some(instrument_parameter_type) = state.automation_view_state.instrument_parameter_type.as_ref() { instrument_parameter_type.clone() } else { 0 }.to_string());
                                },
                                |state: &mut RiffDAWState| {
                                    let mut number_of_parameters = 0;
                                    if let Some(track_uuid) = state.selected_track.as_ref() {
                                        if let Ok(project) = state.project.lock().as_ref() {
                                            if let Some(track) = project.song().track(track_uuid.clone()) {
                                                if let TrackType::InstrumentTrack(instrument_track)  = track {
                                                    let instrument_uuid = instrument_track.instrument().uuid.to_string();
                                                    if let Some(track_audio_plugin_params) = state.audio_plugin_parameters().get(track_uuid.as_str()) {
                                                        if let Some(plugin_params) = track_audio_plugin_params.get(instrument_uuid.as_str()) {
                                                            number_of_parameters = plugin_params.len();
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    let mut new_index = if let Some(instrument_parameter_type) = state.automation_view_state.instrument_parameter_type.as_ref() { instrument_parameter_type.clone() } else { 0 } as i32 + 1;
                                    if new_index >= number_of_parameters as i32 {
                                        new_index = 0;
                                    }
                                    state.automation_view_state.instrument_parameter_type = Some(new_index);
                                    println!("Automation: instrument_parameter_type={}", if let Some(instrument_parameter_type) = state.automation_view_state.instrument_parameter_type.as_ref() { instrument_parameter_type.clone() } else { 0 }.to_string());
                                }
                            ),
                            FlexSpacer::Flex(1.0)
                        )
                    ).gap(1.px()),
                    // effet parameters
                    flex_row(
                        (
                            label("Effects"),
                            generic_selector(
                                effects_plugins_details.iter().map(|(_, description)| description.as_str()).collect(),
                                if let Some(effect_parameter_type) = data.automation_view_state.effect_parameter_type.as_ref() { effect_parameter_type.clone() } else { 0 } as usize,
                                |state: &mut RiffDAWState| {
                                    let mut new_index = if let Some(effect_parameter_type) = state.automation_view_state.effect_parameter_type.as_ref() { effect_parameter_type.clone() } else { 0 } as i32 - 1;
                                    if new_index < 0 {
                                        new_index = 0;
                                    }
                                    state.automation_view_state.effect_parameter_type =  Some(new_index);
                                    println!("Automation: effect_parameter_type={}", if let Some(effect_parameter_type) = state.automation_view_state.effect_parameter_type.as_ref() { effect_parameter_type.clone() } else { 0 }.to_string());
                                },
                                |state: &mut RiffDAWState| {
                                    let mut new_index = if let Some(effect_parameter_type) = state.automation_view_state.effect_parameter_type.as_ref() { effect_parameter_type.clone() } else { 0 } as i32 + 1;
                                    let mut number_of_parameters = 0;
                                    if let Some(track_uuid) = state.selected_track.as_ref() {
                                        if let Ok(project) = state.project.lock().as_ref() {
                                            if let Some(track) = project.song().track(track_uuid.clone()) {
                                                match track {
                                                    TrackType::InstrumentTrack(instrument_track) => {
                                                        let (_, _, effects_data) = get_effects_data(track_uuid.clone(), instrument_track.effects(), state);
                                                        number_of_parameters = effects_data.len();
                                                    }
                                                    TrackType::AudioTrack(audio_track) => {
                                                        let (_, _, effects_data) = get_effects_data(track_uuid.clone(), audio_track.effects(), state);
                                                        number_of_parameters = effects_data.len();
                                                    }
                                                    _ => {}
                                                }
                                            }
                                        }
                                    }
                                    if new_index >= number_of_parameters as i32 {
                                        new_index = 0;
                                    }
                                    state.automation_view_state.effect_parameter_type = Some(new_index);
                                    println!("Automation: effect_parameter_type={}", if let Some(effect_parameter_type) = state.automation_view_state.effect_parameter_type.as_ref() { effect_parameter_type.clone() } else { 0 }.to_string());
                                }
                            ),
                            label("Effect Params"),
                            generic_selector(
                                selected_effect_plugin_parameters.iter().map(|plugin_parameter_detail: &PluginParameterDetail| plugin_parameter_detail.name()).collect(),
                                if let Some(effect_parameter_type) = data.automation_view_state.effect_parameter_type.as_ref() { effect_parameter_type.clone() } else { 0 } as usize,
                                |state: &mut RiffDAWState| {
                                    let mut new_index = if let Some(effect_parameter_type) = state.automation_view_state.effect_parameter_type.as_ref() { effect_parameter_type.clone() } else { 0 } as i32 - 1;
                                    if new_index < 0 {
                                        new_index = 0;
                                    }
                                    state.automation_view_state.effect_parameter_type =  Some(new_index);
                                    println!("Automation: effect_parameter_type={}", if let Some(effect_parameter_type) = state.automation_view_state.effect_parameter_type.as_ref() { effect_parameter_type.clone() } else { 0 }.to_string());
                                },
                                |state: &mut RiffDAWState| {
                                    let mut number_of_parameters = 0;
                                    if let Some(track_uuid) = state.selected_track.as_ref() {
                                        if let Ok(project) = state.project.lock().as_ref() {
                                            if let Some(track) = project.song().track(track_uuid.clone()) {
                                                match track {
                                                    TrackType::InstrumentTrack(instrument_track) => {
                                                        let (_, selected_effect_params, _) = get_effects_data(track_uuid.clone(), instrument_track.effects(), state);
                                                        number_of_parameters = selected_effect_params.len();
                                                    }
                                                    TrackType::AudioTrack(audio_track) => {
                                                        let (_, selected_effect_params, _) = get_effects_data(track_uuid.clone(), audio_track.effects(), state);
                                                        number_of_parameters = selected_effect_params.len();
                                                    }
                                                    _ => {}
                                                }
                                            }
                                        }
                                    }
                                    let mut new_index = if let Some(effect_parameter_type) = state.automation_view_state.effect_parameter_type.as_ref() { effect_parameter_type.clone() } else { 0 } as i32 + 1;
                                    if new_index >= number_of_parameters as i32 {
                                        new_index = 0;
                                    }
                                    state.automation_view_state.effect_parameter_type = Some(new_index);
                                    println!("Automation: effect_parameter_type={}", if let Some(effect_parameter_type) = state.automation_view_state.effect_parameter_type.as_ref() { effect_parameter_type.clone() } else { 0 }.to_string());
                                }
                            ),
                            FlexSpacer::Flex(1.0)
                        )
                    ).gap(1.px()),
                )).active(data.automation_view_state.automation_view_mode.clone() as usize)
            ).gap(Length::px(10.)),
        )
    )
}

pub fn automation_view(
    data: &RiffDAWState
) -> BeatGrid<RiffDAWState, ()> {
    automation_grid_with_size(
        data.project.clone(),
        1280.0,
        60000.0,
        data.piano_roll_state.piano_roll_mpe_note_id.clone(),
        data.selected_track.as_ref().unwrap_or(&"".to_string()).clone(),
        data.selected_riff_uuid.clone(),
        data.selected_riff_events.clone(),
        OperationModeType::PointMode,
        data.automation_view_state.clone(),
    )
}