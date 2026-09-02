use std::path::PathBuf;

use masonry::properties::types::Length;
use xilem::view::{
    CrossAxisAlignment, FlexExt, flex_col, flex_row, label, portal, sized_box,
    text_button, text_input,
};
use xilem::{Color, WidgetView};
use xilem::style::Style as _;
use crate::state::RiffDAWState;
use crate::views::close_dialog;

pub struct FileEntry {
    name: String,
    is_dir: bool,
    path: PathBuf,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DialogMode {
    Open,
    Save,
}

impl DialogMode {
    pub(crate) fn title(self) -> &'static str {
        match self {
            DialogMode::Open => "Open File",
            DialogMode::Save => "Save File",
        }
    }

    fn confirm_label(self) -> &'static str {
        match self {
            DialogMode::Open => "Open",
            DialogMode::Save => "Save",
        }
    }
}

pub struct Bookmarks {
    pub(crate) items: Vec<PathBuf>,
    pub(crate) selected: Option<usize>,
}

impl Bookmarks {
    fn add(&mut self, path: PathBuf) {
        self.items.push(path);
        self.selected = Some(self.items.len() - 1);
    }

    fn remove_selected(&mut self) {
        if let Some(index) = self.selected.take() {
            self.items.remove(index);
        }
    }
}

pub struct DialogState {
    pub(crate) mode: DialogMode,
    pub(crate) current_path: PathBuf,
    files: Vec<FileEntry>,
    pub(crate) selected_path: Option<PathBuf>,
    pub(crate) filename_input: String,
    pub(crate) filter_extension: String,
    pub(crate) confirm_callback: Option<fn(&mut RiffDAWState, String)>
}

impl DialogState {
    pub fn new(file_extension: String) -> Self {
        let current_path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        let files = read_directory(&current_path, file_extension.as_ref());
        Self {
            mode: DialogMode::Open,
            current_path,
            files,
            selected_path: None,
            filename_input: String::new(),
            filter_extension: file_extension,
            confirm_callback: None,
        }
    }

    pub(crate) fn navigate_to(&mut self, path: PathBuf) {
        self.current_path = path;
        self.files = read_directory(&self.current_path, self.filter_extension.as_ref());
        self.selected_path = None;
    }

    pub(crate) fn set_filter_extension(&mut self, extension: String) {
        self.filter_extension = extension;
        self.files =
            read_directory(&self.current_path, self.filter_extension.as_ref());
        self.selected_path = None;
    }
}

fn read_directory(path: &PathBuf, filter: &str) -> Vec<FileEntry> {
    let mut entries: Vec<FileEntry> = match std::fs::read_dir(path) {
        Ok(rd) => {
            let mut collected: Vec<FileEntry> = Vec::new();
            for entry in rd.flatten() {
                let metadata = match entry.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                let is_dir = metadata.is_dir();
                if !is_dir {
                    let matches = entry
                        .path()
                        .extension()
                        .is_some_and(|ext| ext.to_string_lossy().to_lowercase() == filter);
                    if !matches {
                        continue;
                    }
                }
                collected.push(FileEntry {
                    name,
                    is_dir,
                    path: entry.path(),
                });
            }
            collected
        }
        Err(_) => Vec::new(),
    };

    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    entries
}

pub fn dialog_view(state: &mut RiffDAWState) -> impl WidgetView<RiffDAWState> + use<> {
    let path_display = label(format!("Path: {}", state.file_dialog.dialog.current_path.display()));

    let filter_display = state.file_dialog.dialog.filter_extension.clone();

    let up_button = text_button("Up", |state: &mut RiffDAWState| {
        if let Some(parent) = state.file_dialog.dialog.current_path.parent() {
            state.file_dialog.dialog.navigate_to(parent.to_path_buf());
        }
    });

    let home_button = text_button("Home", |state: &mut RiffDAWState| {
        if let Some(home) = std::env::home_dir() {
            state.file_dialog.dialog.navigate_to(home);
        }
    });

    let root_button = text_button("Root", |state: &mut RiffDAWState| {
        state.file_dialog.dialog.navigate_to(PathBuf::from("/"));
    });

    let add_bookmark_button = text_button("Add", |state: &mut RiffDAWState| {
        state.file_dialog.bookmarks.add(state.file_dialog.dialog.current_path.clone());
    });

    let delete_bookmark_button = text_button("Delete", |state: &mut RiffDAWState| {
        state.file_dialog.bookmarks.remove_selected();
    });

    let bookmark_views: Vec<_> = if state.file_dialog.bookmarks.items.is_empty() {
        vec![label("No bookmarks").boxed()]
    } else {
        state
            .file_dialog
            .bookmarks
            .items
            .iter()
            .enumerate()
            .map(|(index, path)| {
                let is_selected = state.file_dialog.bookmarks.selected == Some(index);
                let display_name = if is_selected {
                    format!("> {}", path.display())
                } else {
                    path.display().to_string()
                };
                let path = path.clone();
                text_button(display_name, move |state: &mut RiffDAWState| {
                    state.file_dialog.bookmarks.selected = Some(index);
                    state.file_dialog.dialog.navigate_to(path.clone());
                })
                    .boxed()
            })
            .collect()
    };

    let bookmark_panel = flex_col((
        label("Bookmarks"),
        sized_box(portal(
            flex_col(bookmark_views).cross_axis_alignment(CrossAxisAlignment::Start),
        ))
            .width(Length::px(160.0))
            .height(Length::px(350.0))
            .border(Color::from_rgb8(0x88, 0x88, 0x88), 1.0),
        flex_row((add_bookmark_button, delete_bookmark_button)),
    ));

    let file_views: Vec<_> = if state.file_dialog.dialog.files.is_empty() {
        vec![label("No files").boxed()]
    } else {
        state
            .file_dialog
            .dialog
            .files
            .iter()
            .map(|entry| {
                let is_selected = state.file_dialog.dialog.selected_path.as_ref() == Some(&entry.path);
                let display_name = if entry.is_dir {
                    format!("[DIR] {}/", entry.name)
                } else if is_selected {
                    format!("> {}", entry.name)
                } else {
                    entry.name.clone()
                };
                let path = entry.path.clone();
                let is_dir = entry.is_dir;

                text_button(display_name, move |state: &mut RiffDAWState| {
                    if is_dir {
                        state.file_dialog.dialog.navigate_to(path.clone());
                    } else {
                        match state.file_dialog.dialog.mode {
                            DialogMode::Open => {
                                state.file_dialog.dialog.selected_path = Some(path.clone());
                            }
                            DialogMode::Save => {
                                state.file_dialog.dialog.filename_input = path
                                    .file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_default();
                            }
                        }
                    }
                })
                    .boxed()
            })
            .collect()
    };
    let file_list = portal(flex_col(file_views).cross_axis_alignment(CrossAxisAlignment::Start));

    let cancel_button = text_button("Cancel", |state: &mut RiffDAWState| {
        close_dialog(state);
    });

    let confirm_button = text_button(state.file_dialog.dialog.mode.confirm_label(), |state: &mut RiffDAWState| {
        let selected_path = match state.file_dialog.dialog.mode {
            DialogMode::Open => {
                let Some(ref path) = state.file_dialog.dialog.selected_path.clone() else {
                    return;
                };
                if !path.is_file() {
                    return;
                }
                path.clone()
            }
            DialogMode::Save => {
                let name = state.file_dialog.dialog.filename_input.trim();
                if name.is_empty() {
                    return;
                }
                let mut path = state.file_dialog.dialog.current_path.join(name);
                if path.extension().is_none()
                {
                    path.set_extension(state.file_dialog.dialog.filter_extension.clone());
                }
                path
            }
        };
        state.file_dialog.dialog.selected_path = Some(selected_path.clone());
        if let Some(confirm_callback) = state.file_dialog.dialog.confirm_callback {
            confirm_callback(state, selected_path.to_string_lossy().to_string());
        }
        close_dialog(state);
    });

    let filename_row = if state.file_dialog.dialog.mode == DialogMode::Save {
        flex_row((
            label("File name:"),
            text_input(state.file_dialog.dialog.filename_input.clone(), |state: &mut RiffDAWState, value: String| {
                state.file_dialog.dialog.filename_input = value;
            })
                .placeholder("untitled"),
        ))
            .boxed()
    } else {
        label(String::new()).boxed()
    };

    let file_panel = flex_col((
        flex_row((up_button, home_button, root_button, path_display))
            .flex(CrossAxisAlignment::Fill),
        label(filter_display),
        sized_box(file_list)
            .width(Length::px(500.0))
            .height(Length::px(350.0))
            .border(Color::from_rgb8(0x88, 0x88, 0x88), 1.0),
        filename_row,
        flex_row((cancel_button, confirm_button)),
    ));

    flex_col((
        label(state.file_dialog.dialog.mode.title()),
        flex_row((bookmark_panel, file_panel)),
    ))
}
