use masonry::properties::types::{AsUnit, CrossAxisAlignment, Length, MainAxisAlignment};
use xilem::view::{flex_col, flex_row, indexed_stack, split, text_button, FlexExt, FlexSequence};
use crate::state::{EventEditView, RiffDAWMainView, RiffDAWState};
use crate::views::*;
use masonry_core::core::Axis;
use xilem_core::map_action;

pub fn main_view(
    data: &mut RiffDAWState,
) -> impl FlexSequence<RiffDAWState> + 'static {
    let tempo = if let Ok(project) = data.project.lock() {
        project.song().tempo()
    }
    else {
        140.0
    };

    (
        flex_row(main_view_toolbar(data)).gap(Length::px(1.)).gap(1.px()),
        indexed_stack(
            (
                track_view_toolbar(data),
                riff_set_views_toolbars(data)
            )
        ).active(if let RiffDAWMainView::Track = data.main_view {0} else {1}),
        flex_row(
            (
                map_action(
                    tempo_spin_box(tempo),
                    |state: &mut RiffDAWState, message: TempoChangeEvent| {
                        if let Ok(project) = state.get_project().lock().as_mut() {
                            match message {
                                TempoChangeEvent::Increment(amount) => project.song_mut().tempo = project.song_mut().tempo + amount,
                                TempoChangeEvent::Decrement(amount) => if (project.song_mut().tempo - amount) > 0.0 { project.song_mut().tempo = project.song_mut().tempo - amount; },
                            }
                        }
                    }
                ),
                transport(),
            )
        ),
        split(
            indexed_stack(
                (
                    track_view(data),
                    indexed_stack(
                        (
                            riff_set_view(data),
                            riff_sequence_view(data),
                            riff_grid_view(data),
                            riff_arrangement_view(data),
                        )
                    ).active(data.riff_view.clone() as usize),
                )
            ).active(if let RiffDAWMainView::Track = data.main_view {0} else {1}),
            flex_col(
                (
                    flex_row(
                        (
                            text_button("Piano Roll", |state: &mut RiffDAWState| state.event_edit_view = EventEditView::PianoRoll),
                            text_button("Automation", |state: &mut RiffDAWState| state.event_edit_view = EventEditView::Automation),
                            text_button("Sample Roll", |state: &mut RiffDAWState| state.event_edit_view = EventEditView::SampleRoll),
                            text_button("Sample Library", |state: &mut RiffDAWState| state.event_edit_view = EventEditView::SampleLibrary),
                            text_button("Mixer", |state: &mut RiffDAWState| state.event_edit_view = EventEditView::Mixer),
                            text_button("Scripting", |state: &mut RiffDAWState| state.event_edit_view = EventEditView::Scripting),
                        )
                    ).gap(Length::px(10.)),
                    flex_row(
                        indexed_stack(
                            (
                                piano_roll_view_toolbar(data),
                                automation_view_toolbar(data),
                                sample_view_toolbar(data),
                                sample_library_view_toolbar(data),
                                mixer_view_toolbar(data),
                                scripting_view_toolbar(data)
                            )
                        ).active(data.event_edit_view.clone() as usize)
                    ).gap(Length::px(10.)),
                    indexed_stack(
                        (
                            piano_roll_view(data),
                            portal(
                                automation_view(data)
                            ),
                            sample_view(data),
                            sample_library_view(data),
                            mixer_view(data),
                            scripting_view(data),
                        )
                    ).active(data.event_edit_view.clone() as usize).flex(1.0)
                )
            )
                .main_axis_alignment(MainAxisAlignment::Start)
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .must_fill_major_axis(true),
        ).split_axis(Axis::Vertical).flex(1.0),
    )
}
