use masonry::properties::types::{AsUnit, CrossAxisAlignment, Length, MainAxisAlignment};
use xilem::view::{button, flex_col, flex_row, label, portal, sized_box, split, text_button, Flex, FlexExt, FlexSequence, FlexSpacer};
use xilem::WidgetView;
use crate::actions::{track_change_type_RiffReferenceAdd, track_change_type_RiffReferenceCopySelected, track_change_type_RiffReferenceCutSelected, track_change_type_RiffReferenceDelete, track_change_type_RiffReferencePaste, track_change_type_RiffReferencesDeselectAll, track_change_type_RiffReferencesSelectAll, track_change_type_RiffReferencesSelectMultiple};
use crate::constants::MUSICAL_ITEM_LENGTH_OPTIONS;
use crate::domain::GeneralTrackType;
use crate::event::{OperationModeType};
use crate::icons::{ICON_ARROW_LOOP_RIGHT, ICON_AUTOMATION, ICON_CLIPBOARD, ICON_COPY, ICON_CUT, ICON_DESELECT, ICON_EDIT, ICON_MINUS, ICON_MUSIC, ICON_PLAYER_SKIP_BACK, ICON_PLUS, ICON_POINTER, ICON_ROLLERCOASTER, ICON_SCALE, ICON_SELECT_ALL, ICON_ZOOM};
use crate::state::RiffDAWState;
use crate::views::{beat_grid_ruler, dropdown_view, icon, synced_scroll, track_grid_snap_quantize_selector, track_grid_with_size, track_panel_sequence, BeatGrid, BeatGridRuler, SyncedScroll};


pub fn track_view_toolbar(
    data: &RiffDAWState
) -> Flex<impl FlexSequence<RiffDAWState>, RiffDAWState> {
    let mut index: i32 = -1;
    let snap_options: Vec<_> = MUSICAL_ITEM_LENGTH_OPTIONS.iter().map(|snap_quantise| {
        index += 1;
        (index as usize, (*snap_quantise).to_string())
    }).collect();

    flex_row(
        (
            flex_row(
                (
                    button(icon(ICON_POINTER.to_string()), |state: &mut RiffDAWState| state.track_grid_state.track_grid_operation_mode = OperationModeType::PointMode),
                    button(icon(ICON_PLUS.to_string()), |state: &mut RiffDAWState| state.track_grid_state.track_grid_operation_mode = OperationModeType::Add),
                    button(icon(ICON_MINUS.to_string()), |state: &mut RiffDAWState| state.track_grid_state.track_grid_operation_mode = OperationModeType::Delete),
                    button(icon(ICON_EDIT.to_string()), |state: &mut RiffDAWState| state.track_grid_state.track_grid_operation_mode = OperationModeType::Change),
                    button(icon(ICON_ARROW_LOOP_RIGHT.to_string()), |state: &mut RiffDAWState| state.track_grid_state.track_grid_operation_mode = OperationModeType::LoopPointMode),
                    button(icon(ICON_PLAYER_SKIP_BACK.to_string()), |state: &mut RiffDAWState| state.track_grid_state.track_grid_operation_mode = OperationModeType::SelectRiffReferenceMode),
                    button(icon(ICON_ZOOM.to_string()), |state: &mut RiffDAWState| state.track_grid_state.track_grid_operation_mode = OperationModeType::WindowedZoom),
                )
            ).gap(3.px()),
            flex_row(
                (
                    button(icon(ICON_CUT.to_string()), |state: &mut RiffDAWState| track_change_type_RiffReferenceCutSelected(state)),
                    button(icon(ICON_COPY.to_string()), |state: &mut RiffDAWState| track_change_type_RiffReferenceCopySelected(state)),
                    button(icon(ICON_CLIPBOARD.to_string()), |state: &mut RiffDAWState| track_change_type_RiffReferencePaste(state)),
                    button(icon(ICON_SELECT_ALL.to_string()), |state: &mut RiffDAWState| track_change_type_RiffReferencesSelectAll(state)),
                    button(icon(ICON_DESELECT.to_string()), |state: &mut RiffDAWState| track_change_type_RiffReferencesDeselectAll(state)),
                )
            ).gap(3.px()),
            flex_row(
                (
                    label("Snap"),
                    track_grid_snap_quantize_selector(data),
                    dropdown_view(snap_options, Some(data.track_grid_state.track_grid_selected_snap.clone()), |state: &mut RiffDAWState, snap_index: usize| {
                        state.track_grid_state.track_grid_selected_snap = snap_index;
                    })
                )
            ).gap(3.px()),
            flex_row(
                (
                    label("Show events"),
                    button(icon(ICON_AUTOMATION.to_string()), |state: &mut RiffDAWState| state.track_grid_state.show_automation = !state.track_grid_state.show_automation),
                    button(icon(ICON_ROLLERCOASTER.to_string()), |state: &mut RiffDAWState| state.track_grid_state.show_note_velocities = !state.track_grid_state.show_note_velocities),
                    button(icon(ICON_MUSIC.to_string()), |state: &mut RiffDAWState| state.track_grid_state.show_notes = !state.track_grid_state.show_notes),
                    button(icon(ICON_SCALE.to_string()), |state: &mut RiffDAWState| state.track_grid_state.show_pan = !state.track_grid_state.show_pan),
                )
            ).gap(3.px()),
            flex_row(
                (
                    text_button("Follow the  cursor", |state: &mut RiffDAWState| state.track_grid_state.track_grid_cursor_follow = !state.track_grid_state.track_grid_cursor_follow),
                )
            ).gap(3.px()),
            FlexSpacer::Flex(1.0)
        )
    ).gap(Length::px(10.))
}


pub fn track_view(
    state: &RiffDAWState,
) -> impl WidgetView<RiffDAWState, ()> + 'static {
    split(
        synced_scroll(
            flex_col(
                (
                    track_panel_sequence::<RiffDAWState>(state, 20.px()),
                    FlexSpacer::Fixed(60000.px())
                )
            )
                .main_axis_alignment(MainAxisAlignment::Start)
                .cross_axis_alignment(CrossAxisAlignment::Start),
            "track_panel_sequence_horizontal",
            "track_view_vertical"
        ),
        flex_col((
            synced_scroll(
                beat_grid_ruler(1.0, 50.0, 4, 60000.0),
                "track_grid_horizontal",
                "track_grid_ruler_vertical"
            ),
            synced_scroll(
                track_grid_with_size(state.project.clone(),
                                     60000.0,
                                     60000.0,
                                     state.track_grid_state.selected_track_grid_riff_references.clone(),
                                     state.track_grid_state.track_grid_operation_mode.clone(),
                                     state.track_grid_state.clone(),
                )
                    .on_select_multiple(Box::new(|data, x: &f64, y: &i32, x2: &f64, y2: &i32, add_to_select: &bool| track_change_type_RiffReferencesSelectMultiple(data, *x, *y, *x2, *y2, *add_to_select)))
                    .on_add_riff_reference(Box::new(|data, position: &f64, track_index: &i32| track_change_type_RiffReferenceAdd(data, *track_index, *position, None)))
                    .on_delete_riff_reference(Box::new(|data, position: &f64, track_index: &i32| track_change_type_RiffReferenceDelete(data, *track_index, *position, None)))
                    .on_cut(Box::new(|data| track_change_type_RiffReferenceCutSelected(data)))
                    .on_copy(Box::new(|data| track_change_type_RiffReferenceCopySelected(data)))
                    .on_paste(Box::new(|data| track_change_type_RiffReferencePaste(data)))
                    .on_edit_cursor_position_change(Box::new(|data, position| {
                        data.track_grid_state.track_grid_edit_cursor_position = position;
                    })),
                "track_grid_horizontal",
                "track_view_vertical"
            )
                .flex(1.0)
        ))
            .must_fill_major_axis(true),
    ).split_point(0.2)
}