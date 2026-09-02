use log::debug;
use crate::audio_plugin_util::scan_for_audio_plugins;
use crate::event::DAWEvents;
use crate::state::RiffDAWState;

pub fn daw_events_ScanPlugins (state: &mut RiffDAWState) {
    // gui.ui.dialogue_progress_bar.set_text(Some("Scanning for plugins..."));
    // gui.ui.progress_dialogue.set_title("Scanning Plugins...");
    // gui.ui.progress_dialogue.show_all();

    // let state = state.clone();
    // let tx_from_ui = tx_from_ui.clone();
    // let _ = THREAD_POOL.with_borrow(|thread_pool| thread_pool.spawn(move || {
        let mut vst24_plugin_paths = vec![];
        let mut clap_plugin_paths = vec![];
        let mut vst3_plugin_paths = vec![];

        debug!("Main - rx_ui processing loop - DAWEvents::ScanPlugins.");
        vst24_plugin_paths = state.configuration.vst24_plugin_paths.clone();
        clap_plugin_paths = state.configuration.clap_plugin_paths.clone();
        vst3_plugin_paths = state.configuration.vst3_plugin_paths.clone();

        let (instrument_audio_plugins, effect_audio_plugins) = scan_for_audio_plugins(
            &vst24_plugin_paths,
            &clap_plugin_paths,
            &vst3_plugin_paths,
        );

        debug!("Main - rx_ui processing loop - DAWEvents::ScanPlugins.");
        state.configuration.scanned_instrument_plugins.successfully_scanned = instrument_audio_plugins;
        state.configuration.scanned_effect_plugins.successfully_scanned = effect_audio_plugins;


        // let _ = tx_from_ui.send(DAWEvents::UpdateUIPlugins);
        // let _ = tx_from_ui.send(DAWEvents::HideProgressDialogue);
    // }));
}
