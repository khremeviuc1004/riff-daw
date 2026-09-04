use xilem::view::{flex_col, flex_row, text_button, FlexSpacer};
use xilem::WidgetView;
use masonry::properties::types::AsUnit;
use crate::actions::daw_events_ScanPlugins;
use crate::state::RiffDAWState;

pub fn settings_dialog(window_id: xilem::WindowId) -> impl WidgetView<RiffDAWState> + use<> {
    flex_col((
        text_button("Scan plugins".to_string(), |state: &mut RiffDAWState| {
            daw_events_ScanPlugins(state);
        }),
        flex_row((
            FlexSpacer::Flex(1.0),
            text_button("Save".to_string(), move |state: &mut RiffDAWState| {
                state.settings_window.insert(window_id, false);
            }),
            text_button("Cancel".to_string(), move |state: &mut RiffDAWState| {
                state.settings_window.insert(window_id, false);
            }),
            FlexSpacer::Fixed(20.px()),
        )),
    )).cross_axis_alignment(xilem::view::CrossAxisAlignment::Start)
}
