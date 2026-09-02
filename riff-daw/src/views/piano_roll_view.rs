use masonry::properties::types::{AsUnit, Length};
use xilem::view::{button, checkbox, flex_col, flex_row, label, portal, split, text_button, Flex, FlexSequence, FlexSpacer, Portal, Split};
use crate::actions::{daw_events_PlayNoteImmediate, daw_events_StopNoteImmediate, track_change_type_RiffAddNote, track_change_type_RiffChangeLengthOfSelected, track_change_type_RiffCopySelected, track_change_type_RiffCutSelected, track_change_type_RiffDeleteNote, track_change_type_RiffEventsDeselectAll, track_change_type_RiffEventsSelectAll, track_change_type_RiffEventsSelectMultiple, track_change_type_RiffPasteSelected, track_change_type_RiffQuantiseSelected, track_change_type_RiffTranslateSelected};
use crate::constants::{MUSICAL_ITEM_LENGTH_OPTIONS, NOTE_SUBDIVISIONS, TRIPLETS};
use crate::event::{OperationModeType, TranslateDirection, TranslationEntityType};
use crate::icons::{ICON_ARROW_DOWN, ICON_ARROW_LEFT, ICON_ARROW_LOOP_RIGHT, ICON_ARROW_RIGHT, ICON_ARROW_UP, ICON_CLIPBOARD, ICON_COPY, ICON_CUT, ICON_DESELECT, ICON_EDIT, ICON_MINUS, ICON_PLAYER_SKIP_BACK, ICON_PLUS, ICON_POINTER, ICON_SELECT_ALL, ICON_ZOOM};
use crate::state::{RiffDAWState};
use crate::utils::DAWUtils;
use crate::views::{generic_number_selector, icon, piano_keyboard, piano_roll_with_size, synced_scroll, BeatGrid, PianoKeyboard, SyncedScroll};
use crate::views::generic_selector::generic_selector;

pub fn piano_roll_view_toolbar(
    data: &RiffDAWState,
) -> Flex<impl FlexSequence<RiffDAWState>, RiffDAWState> {
    flex_col(
        (
            flex_row(
                (
                    flex_row(
                        (
                            button(icon(ICON_POINTER.to_string()), |state: &mut RiffDAWState| state.piano_roll_state.piano_roll_grid_operation_mode = OperationModeType::PointMode),
                            button(icon(ICON_PLUS.to_string()), |state: &mut RiffDAWState| state.piano_roll_state.piano_roll_grid_operation_mode = OperationModeType::Add),
                            button(icon(ICON_MINUS.to_string()), |state: &mut RiffDAWState| state.piano_roll_state.piano_roll_grid_operation_mode = OperationModeType::Delete),
                            button(icon(ICON_EDIT.to_string()), |state: &mut RiffDAWState| state.piano_roll_state.piano_roll_grid_operation_mode = OperationModeType::Change),
                            button(icon(ICON_PLAYER_SKIP_BACK.to_string()), |state: &mut RiffDAWState| state.piano_roll_state.piano_roll_grid_operation_mode = OperationModeType::SelectRiffReferenceMode),
                            button(icon(ICON_ZOOM.to_string()), |state: &mut RiffDAWState| state.piano_roll_state.piano_roll_grid_operation_mode = OperationModeType::WindowedZoom),
                        )
                    ).gap(1.px()),
                    flex_row(
                        (
                            button(icon(ICON_CUT.to_string()), |state: &mut RiffDAWState| track_change_type_RiffCutSelected(state)),
                            button(icon(ICON_COPY.to_string()), |state: &mut RiffDAWState| track_change_type_RiffCopySelected(state)),
                            button(icon(ICON_CLIPBOARD.to_string()), |state: &mut RiffDAWState| track_change_type_RiffPasteSelected(state)),
                            button(icon(ICON_SELECT_ALL.to_string()), |state: &mut RiffDAWState| track_change_type_RiffEventsSelectAll(state)),
                            button(icon(ICON_DESELECT.to_string()), |state: &mut RiffDAWState| track_change_type_RiffEventsDeselectAll(state)),
                        )
                    ).gap(1.px()),
                    flex_row(
                        (
                            button(icon(ICON_ARROW_LEFT.to_string()), |state: &mut RiffDAWState| {
                                track_change_type_RiffTranslateSelected(state, TranslationEntityType::Note, TranslateDirection::Left);
                            }),
                            button(icon(ICON_ARROW_RIGHT.to_string()), |state: &mut RiffDAWState| {
                                track_change_type_RiffTranslateSelected(state, TranslationEntityType::Note, TranslateDirection::Right);
                            }),
                            button(icon(ICON_ARROW_UP.to_string()), |state: &mut RiffDAWState| {
                                track_change_type_RiffTranslateSelected(state, TranslationEntityType::Note, TranslateDirection::Up);
                            }),
                            button(icon(ICON_ARROW_DOWN.to_string()), |state: &mut RiffDAWState| {
                                track_change_type_RiffTranslateSelected(state, TranslationEntityType::Note, TranslateDirection::Down);
                            }),
                        )
                    ).gap(1.px()),
                    flex_row(
                        (
                            label("Note Subdivision"),
                            generic_selector(
                                NOTE_SUBDIVISIONS.to_vec(), data.piano_roll_state.selected_piano_roll_subdivision,
                                |state: &mut RiffDAWState| {
                                    let mut new_index: i32 = state.piano_roll_state.selected_piano_roll_subdivision as i32 - 1;
                                    if new_index < 0 {
                                        new_index = 0;
                                    }
                                    state.piano_roll_state.selected_piano_roll_subdivision =  new_index as usize;
                                    println!("Note Subdivision: index={}", state.piano_roll_state.selected_piano_roll_subdivision);
                                },
                                |state: &mut RiffDAWState| {
                                    let mut new_index = state.piano_roll_state.selected_piano_roll_subdivision + 1;
                                    if new_index >= NOTE_SUBDIVISIONS.len() {
                                        new_index = 0;
                                    }
                                    state.piano_roll_state.selected_piano_roll_subdivision =  new_index;
                                    println!("New Note Length: index={}", state.piano_roll_state.selected_piano_roll_subdivision);
                                }
                            ),
                            label("Triplet Type"),
                            generic_selector(
                                TRIPLETS.to_vec(), data.piano_roll_state.selected_piano_roll_triplet,
                                |state: &mut RiffDAWState| {
                                    let mut new_index: i32 = state.piano_roll_state.selected_piano_roll_triplet as i32 - 1;
                                    if new_index < 0 {
                                        new_index = 0;
                                    }
                                    state.piano_roll_state.selected_piano_roll_triplet =  new_index as usize;
                                    println!("New Note Length: index={}", state.piano_roll_state.selected_piano_roll_triplet);
                                },
                                |state: &mut RiffDAWState| {
                                    let mut new_index = state.piano_roll_state.selected_piano_roll_triplet + 1;
                                    if new_index >= TRIPLETS.len() {
                                        new_index = 0;
                                    }
                                    state.piano_roll_state.selected_piano_roll_triplet =  new_index;
                                    println!("Triplet Type: index={}", state.piano_roll_state.selected_piano_roll_triplet);
                                }
                            ),
                        )
                    ).gap(1.px()),
                    flex_row(
                        (
                            label("New Note Length"),
                            generic_selector(
                                MUSICAL_ITEM_LENGTH_OPTIONS.to_vec(), data.piano_roll_state.selected_piano_roll_note_length_option,
                                |state: &mut RiffDAWState| {
                                    let mut new_index: i32 = state.piano_roll_state.selected_piano_roll_note_length_option as i32 - 1;
                                    if new_index < 0 {
                                        new_index = 0;
                                    }
                                    state.piano_roll_state.selected_piano_roll_note_length_option =  new_index as usize;
                                    println!("New Note Length: index={}", state.piano_roll_state.selected_piano_roll_note_length_option);
                                },
                                |state: &mut RiffDAWState| {
                                    let mut new_index = state.piano_roll_state.selected_piano_roll_note_length_option + 1;
                                    if new_index >= MUSICAL_ITEM_LENGTH_OPTIONS.len() {
                                        new_index = 0;
                                    }
                                    state.piano_roll_state.selected_piano_roll_note_length_option =  new_index;
                                    println!("New Note Length: index={}", state.piano_roll_state.selected_piano_roll_note_length_option);
                                }
                            ),
                        )
                    ).gap(1.px()),
                    FlexSpacer::Flex(1.0)
                )
            ).gap(Length::px(10.)),
            flex_row(
                (
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
                            checkbox("Start", data.piano_roll_state.piano_roll_quantise_start.clone(), |state: &mut RiffDAWState, checked: bool| {
                                state.piano_roll_state.piano_roll_quantise_start = checked;
                            }),
                            checkbox("End", data.piano_roll_state.piano_roll_quantise_end.clone(), |state: &mut RiffDAWState, checked: bool| {
                                state.piano_roll_state.piano_roll_quantise_end = checked;
                            }),
                            label("Quantize Strength"),
                            generic_number_selector(
                                1..101, (data.piano_roll_state.piano_roll_quantise_quantise_strength - 1) as usize,
                                |state: &mut RiffDAWState| {
                                    let mut new_index: i32 = state.piano_roll_state.piano_roll_quantise_quantise_strength as i32 - 1;
                                    if new_index < 1 {
                                        new_index = 1;
                                    }
                                    state.piano_roll_state.piano_roll_quantise_quantise_strength =  new_index as u32;
                                    println!("Quantize Strength: index={}", state.piano_roll_state.piano_roll_quantise_quantise_strength);
                                },
                                |state: &mut RiffDAWState| {
                                    let mut new_index = state.piano_roll_state.piano_roll_quantise_quantise_strength + 1;
                                    if new_index > 100 {
                                        new_index = 1;
                                    }
                                    state.piano_roll_state.piano_roll_quantise_quantise_strength =  new_index;
                                    println!("Quantize Strength: index={}", state.piano_roll_state.piano_roll_quantise_quantise_strength);
                                }
                            ),
                            text_button("Q", |state: &mut RiffDAWState| track_change_type_RiffQuantiseSelected(state)),
                        )
                    ).gap(1.px()),
                    flex_row(
                        (
                            label("Note Length Adjustment"),
                            generic_selector(
                                MUSICAL_ITEM_LENGTH_OPTIONS.to_vec(), data.piano_roll_state.selected_piano_roll_note_adj,
                                |state: &mut RiffDAWState| {
                                    let mut new_index: i32 = state.piano_roll_state.selected_piano_roll_note_adj as i32 - 1;
                                    if new_index < 0 {
                                        new_index = 0;
                                    }
                                    state.piano_roll_state.selected_piano_roll_note_adj =  new_index as usize;
                                    println!("Note Length Adjustment: index={}", state.piano_roll_state.selected_piano_roll_note_adj);
                                },
                                |state: &mut RiffDAWState| {
                                    let mut new_index = state.piano_roll_state.selected_piano_roll_note_adj + 1;
                                    if new_index >= MUSICAL_ITEM_LENGTH_OPTIONS.len() {
                                        new_index = 0;
                                    }
                                    state.piano_roll_state.selected_piano_roll_note_adj =  new_index;
                                    println!("Note Length Adjustment: index={}", state.piano_roll_state.selected_piano_roll_note_adj);
                                }
                            ),
                        )
                    ).gap(1.px()),
                    flex_row(
                        (
                            button(icon(ICON_ARROW_LEFT.to_string()), |state: &mut RiffDAWState| {
                                track_change_type_RiffChangeLengthOfSelected(state, false);
                            }),
                            button(icon(ICON_ARROW_RIGHT.to_string()), |state: &mut RiffDAWState| {
                                track_change_type_RiffChangeLengthOfSelected(state, true);
                            }),
                        )
                    ).gap(1.px()),
                    flex_row(
                        (
                            label("MPE"),
                            generic_number_selector(
                                -1..11,
                                (data.piano_roll_state.piano_roll_mpe_note_id.clone() as i32 + 1) as usize,
                                |state: &mut RiffDAWState| {
                                    let mut new_index: i32 = state.piano_roll_state.piano_roll_mpe_note_id.clone() as i32 - 1;
                                    if new_index < -1 {
                                        new_index = -1;
                                    }
                                    state.piano_roll_state.piano_roll_mpe_note_id = DAWUtils::convert_i32_to_midi_polyphonic_expression_id(new_index);
                                    println!("Quantize Strength: index={}", state.piano_roll_state.piano_roll_mpe_note_id.clone() as i32);
                                },
                                |state: &mut RiffDAWState| {
                                    let mut new_index = state.piano_roll_state.piano_roll_mpe_note_id.clone() as i32 + 1;
                                    if new_index > 10 {
                                        new_index = 10;
                                    }
                                    state.piano_roll_state.piano_roll_mpe_note_id = DAWUtils::convert_i32_to_midi_polyphonic_expression_id(new_index);
                                    println!("Quantize Strength: index={}", state.piano_roll_state.piano_roll_mpe_note_id.clone() as i32);
                                }
                            ),
                        )
                    ).gap(1.px()),
                    flex_row(
                        (
                            label("Dock/Undock"),
                            checkbox("Undock", data.piano_roll_state.window_undock, |state: &mut RiffDAWState, checked: bool| state.piano_roll_state.window_undock = checked),
                        )
                    ).gap(1.px()),
                    FlexSpacer::Flex(1.0)
                )
            ).gap(Length::px(10.))
        )
    )
}


pub fn piano_roll_view(
    data: &RiffDAWState,
) -> Split<SyncedScroll<RiffDAWState, (), PianoKeyboard<RiffDAWState, ()>>, SyncedScroll<RiffDAWState, (), BeatGrid<RiffDAWState, ()>>, RiffDAWState> {
    split (
        synced_scroll(
            piano_keyboard::<RiffDAWState, ()>()
                .on_note_on(Box::new(|state: &mut RiffDAWState, note: i32, channel: i32| {
                    println!("^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ piano roll view - note on event received.");
                    daw_events_PlayNoteImmediate(state, note);
                }))
                .on_note_off(Box::new(|state: &mut RiffDAWState, note: i32, channel: i32| {
                    println!("^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ piano roll view - note off event received.");
                    daw_events_StopNoteImmediate(state, note);
                })),
            "piano_keyboard_horizontal",
            "piano_roll_view_vertical",
        ),
        synced_scroll(
            piano_roll_with_size(
                data.project.clone(),
                1280.0,
                60000.0,
                data.piano_roll_state.piano_roll_mpe_note_id.clone(),
                data.selected_track.as_ref().unwrap_or(&"".to_string()).clone(),
                data.selected_riff_uuid.clone(),
                data.selected_riff_events.clone(),
                data.piano_roll_state.piano_roll_grid_operation_mode.clone(),
                data.piano_roll_state.clone()
            )
                .on_select_multiple(Box::new(|data, x: &f64, y: &i32, x2: &f64, y2: &i32, add_to_select: &bool| {
                    track_change_type_RiffEventsSelectMultiple(data, *x, *y, *x2, *y2, *add_to_select, None);
                }))
                .on_add_riff_note(Box::new(|data, new_notes: Vec<(i32, f64, f64)>| {
                    track_change_type_RiffAddNote(data, new_notes, None);
                }))
                .on_delete_riff_note(Box::new(|data, note: i32, position: f64| {
                    track_change_type_RiffDeleteNote(data, note, position, None);
                }))
                .on_cut(Box::new(|data| track_change_type_RiffCutSelected(data)))
                .on_copy(Box::new(|data| track_change_type_RiffCopySelected(data)))
                .on_paste(Box::new(|data| track_change_type_RiffPasteSelected(data)))
                .on_edit_cursor_position_change(Box::new(|data, position| {
                    data.piano_roll_state.piano_roll_edit_cursor_position = position;
                })),
            "piano_roll_horizontal",
            "piano_roll_view_vertical"
        )
    ).split_point(0.1)
}