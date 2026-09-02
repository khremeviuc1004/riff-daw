use xilem::view::{button, flex_row, Button, Flex};
use masonry::ui_events::pointer::PointerButton;
use xilem_core::MessageResult;
use masonry::properties::types::AsUnit;
use masonry_winit::app::WindowId;
use crate::actions::{daw_events_ExportMidiFile, daw_events_ExportRiffsToMidiFile, daw_events_ExportRiffsToSeparateMidiFiles, daw_events_ExportWaveFile, daw_events_ImportDAWProjectFile, daw_events_ImportMidiFile, daw_events_NewFile, daw_events_OpenFile, daw_events_Save, daw_events_SaveAs};
use crate::icons::{ICON_DATABASE_EXPORT, ICON_DEVICE_FLOPPY, ICON_FILE_DOWNLOAD, ICON_FILE_EXPORT, ICON_FILE_IMPORT, ICON_FILE_PLUS, ICON_FOLDER_OPEN, ICON_PACKAGE_EXPORT, ICON_TABLE_EXPORT};
use crate::state::RiffDAWState;
use crate::views::{icon, DialogMode, Icon};

pub fn file_toolbar() -> Flex<(Button<impl Fn(&mut RiffDAWState, Option<PointerButton>) -> MessageResult<()> + Send, Icon>, Button<impl Fn(&mut RiffDAWState, Option<PointerButton>) -> MessageResult<()> + Send, Icon>, Button<impl Fn(&mut RiffDAWState, Option<PointerButton>) -> MessageResult<()> + Send, Icon>, Button<impl Fn(&mut RiffDAWState, Option<PointerButton>) -> MessageResult<()> + Send, Icon>, Button<impl Fn(&mut RiffDAWState, Option<PointerButton>) -> MessageResult<()> + Send, Icon>, Button<impl Fn(&mut RiffDAWState, Option<PointerButton>) -> MessageResult<()> + Send, Icon>, Button<impl Fn(&mut RiffDAWState, Option<PointerButton>) -> MessageResult<()> + Send, Icon>, Button<impl Fn(&mut RiffDAWState, Option<PointerButton>) -> MessageResult<()> + Send, Icon>, Button<impl Fn(&mut RiffDAWState, Option<PointerButton>) -> MessageResult<()> + Send, Icon>, Button<impl Fn(&mut RiffDAWState, Option<PointerButton>) -> MessageResult<()> + Send, Icon>), RiffDAWState> {
    flex_row((
        button(icon(ICON_FILE_PLUS.to_string()), |state: &mut RiffDAWState| {
            daw_events_NewFile(state);
        }),
        button(icon(ICON_FOLDER_OPEN.to_string()), |state: &mut RiffDAWState| {
            state.file_dialog.dialog.filter_extension = String::from("fdaw");
            state.file_dialog.dialog.confirm_callback = Some(|state: &mut RiffDAWState, path: String| daw_events_OpenFile(state, path));
            open_dialog(state, DialogMode::Open);
        }),
        button(icon(ICON_DEVICE_FLOPPY.to_string()), |state: &mut RiffDAWState| {
            state.file_dialog.dialog.filter_extension = String::from("fdaw");
            state.file_dialog.dialog.confirm_callback = Some(|state: &mut RiffDAWState, path: String| {
                state.set_current_file_path(Some(path));
                daw_events_Save(state);
            });
            open_dialog(state, DialogMode::Save);
        }),
        button(icon(ICON_FILE_DOWNLOAD.to_string()), |state: &mut RiffDAWState| {
            // state.save_file_dialogue.insert(state.save_file_dialog_window_id.clone(), true);
            let path = std::env::current_dir().unwrap();
            let res = rfd::FileDialog::new()
                .set_title("Open RiffDAW Project")
                .add_filter("RiffDAW", &["fdaw"])
                .set_directory(&path)
                .save_file();

            if let Some(path) = res {
                daw_events_SaveAs(state, path.as_os_str().to_os_string().into_string().unwrap());
            }
        }),
        button(icon(ICON_FILE_IMPORT.to_string()), |state: &mut RiffDAWState| {
            // state.save_file_dialogue.insert(state.save_file_dialog_window_id.clone(), true);
            let path = std::env::current_dir().unwrap();
            let res = rfd::FileDialog::new()
                .set_title("Import MIDI File")
                .add_filter("MIDI", &["mid"])
                .set_directory(&path)
                .pick_file();

            if let Some(path) = res {
                daw_events_ImportMidiFile(state, path.as_os_str().to_os_string().into_string().unwrap());
            }
        }),
        button(icon(ICON_FILE_IMPORT.to_string()), |state: &mut RiffDAWState| {
            let path = std::env::current_dir().unwrap();
            let res = rfd::FileDialog::new()
                .set_title("Import DAWProject File")
                .add_filter("DAWProject", &["dawproject"])
                .set_directory(&path)
                .pick_file();

            if let Some(path) = res {
                daw_events_ImportDAWProjectFile(state, path.as_os_str().to_os_string().into_string().unwrap());
            }
        }),
        button(icon(ICON_FILE_EXPORT.to_string()), |state: &mut RiffDAWState| {
            // state.save_file_dialogue.insert(state.save_file_dialog_window_id.clone(), true);
            let path = std::env::current_dir().unwrap();
            let res = rfd::FileDialog::new()
                .set_title("Export MIDI File")
                .add_filter("MIDI", &["mid"])
                .set_directory(&path)
                .save_file();

            if let Some(path) = res {
                daw_events_ExportMidiFile(state, path.as_os_str().to_os_string().into_string().unwrap());
            }
        }),
        button(icon(ICON_PACKAGE_EXPORT.to_string()), |state: &mut RiffDAWState| {
            // state.save_file_dialogue.insert(state.save_file_dialog_window_id.clone(), true);
            let path = std::env::current_dir().unwrap();
            let res = rfd::FileDialog::new()
                .set_title("Export Riffs to MIDI File")
                .add_filter("MIDI", &["mid"])
                .set_directory(&path)
                .save_file();

            if let Some(path) = res {
                daw_events_ExportRiffsToMidiFile(state, path.as_os_str().to_os_string().into_string().unwrap());
            }
        }),
        button(icon(ICON_DATABASE_EXPORT.to_string()), |state: &mut RiffDAWState| {
            // state.save_file_dialogue.insert(state.save_file_dialog_window_id.clone(), true);
            let path = std::env::current_dir().unwrap();
            let res = rfd::FileDialog::new()
                .set_title("Export Riffs to Separate MIDI Files")
                .add_filter("MIDI", &["mid"])
                .set_directory(&path)
                .save_file();

            if let Some(path) = res {
                daw_events_ExportRiffsToSeparateMidiFiles(state, path.as_os_str().to_os_string().into_string().unwrap());
            }
        }),
        button(icon(ICON_TABLE_EXPORT.to_string()), |state: &mut RiffDAWState| {
            // state.save_file_dialogue.insert(state.save_file_dialog_window_id.clone(), true);
            let path = std::env::current_dir().unwrap();
            let res = rfd::FileDialog::new()
                .set_title("Export WAVE File")
                .add_filter("WAVE", &["wav"])
                .set_directory(&path)
                .save_file();

            if let Some(path) = res {
                daw_events_ExportWaveFile(state, path.as_os_str().to_os_string().into_string().unwrap());
            }
        })
    )).gap(1.px())
}
fn normalized_filter(filter_extension: String) -> String {
    let trimmed = filter_extension.trim().trim_start_matches('.').to_lowercase();
    if !trimmed.is_empty() {
        trimmed
    }
    else {
        "fdaw".to_string()
    }
}

pub fn open_dialog(state: &mut RiffDAWState, mode: DialogMode) {
    if state.file_dialog.dialog_window_id.is_some() {
        return;
    }
    state.file_dialog.dialog.mode = mode;
    state.file_dialog.dialog
        .set_filter_extension(normalized_filter(state.file_dialog.dialog.filter_extension.clone()));
    state.file_dialog.dialog.navigate_to(
        state.file_dialog.dialog.selected_path
            .as_ref()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| state.file_dialog.dialog.current_path.clone()),
    );
    if mode == DialogMode::Save {
        state.file_dialog.dialog.filename_input = state.file_dialog
            .dialog.selected_path
            .as_ref()
            .map(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default()
            })
            .unwrap_or_default();
    } else {
        state.file_dialog.dialog.filename_input.clear();
    }
    state.file_dialog.dialog.selected_path = None;
    state.file_dialog.dialog_window_id = Some(WindowId::next());
}

pub fn close_dialog(state: &mut RiffDAWState) {
    state.file_dialog.dialog_window_id = None;
    state.file_dialog.dialog.selected_path = None;
    state.file_dialog.dialog.filename_input.clear();
}
