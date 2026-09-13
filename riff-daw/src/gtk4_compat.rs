use std::cell::RefCell;
use std::time::Duration;

use glib::prelude::*;
use gtk4::prelude::*;
use gtk4::Widget;

/// GTK4 porting shim for the GTK3-style event handler API.
///
/// GTK4 replaced `connect_button_press_event`-style widget signals with
/// `EventController`s. These traits keep call sites untouched by backing the
/// old handler names with `EventControllerLegacy` instances and mapping the
/// GTK3 `bool` closure return value to `glib::Propagation`.

pub trait GdkEventCompat {
    fn coords(&self) -> Option<(f64, f64)>;
    fn state(&self) -> gdk4::ModifierType;
    fn keyval(&self) -> gdk4::Key;
    fn scroll_direction(&self) -> Option<gdk4::ScrollDirection>;
}

impl GdkEventCompat for gdk4::Event {
    fn coords(&self) -> Option<(f64, f64)> {
        self.position()
    }

    fn state(&self) -> gdk4::ModifierType {
        self.modifier_state()
    }

    fn keyval(&self) -> gdk4::Key {
        if let Some(key_event) = self.downcast_ref::<gdk4::KeyEvent>() {
            key_event.keyval()
        } else {
            gdk4::Key::VoidSymbol
        }
    }

    fn scroll_direction(&self) -> Option<gdk4::ScrollDirection> {
        if let Some(scroll_event) = self.downcast_ref::<gdk4::ScrollEvent>() {
            Some(scroll_event.direction())
        } else {
            None
        }
    }
}

fn widget_legacy_event_connect<T, F>(widget: &T, event_type: gdk4::EventType, f: F) -> glib::SignalHandlerId
where
    T: gtk4::prelude::IsA<gtk4::Widget> + gtk4::prelude::Cast + 'static,
    F: Fn(&T, &gdk4::Event) -> bool + 'static,
{
    let controller = gtk4::EventControllerLegacy::new();
    controller.set_propagation_phase(gtk4::PropagationPhase::Bubble);
    let id = controller.connect_event(move |controller, event| {
        if event.event_type() != event_type {
            return gtk4::glib::Propagation::Proceed;
        }
        if let Some(widget) = controller.widget() {
            if let Some(t) = widget.downcast_ref::<T>() {
                if f(t, event) {
                    return gtk4::glib::Propagation::Stop;
                }
            }
        }
        gtk4::glib::Propagation::Proceed
    });
    widget.add_controller(controller);
    id
}

fn widget_focus_in_event_connect<T, F>(widget: &T, f: F) -> glib::SignalHandlerId
where
    T: gtk4::prelude::IsA<gtk4::Widget> + gtk4::prelude::Cast + 'static,
    F: Fn(&T, &gdk4::Event) -> bool + 'static,
{
    let controller = gtk4::EventControllerLegacy::new();
    controller.set_propagation_phase(gtk4::PropagationPhase::Bubble);
    let id = controller.connect_event(move |controller, event| {
        if event.event_type() != gdk4::EventType::FocusChange {
            return gtk4::glib::Propagation::Proceed;
        }
        let is_in = event
            .downcast_ref::<gdk4::FocusEvent>()
            .map_or(false, |focus_event| focus_event.is_in());
        if !is_in {
            return gtk4::glib::Propagation::Proceed;
        }
        if let Some(widget) = controller.widget() {
            if let Some(t) = widget.downcast_ref::<T>() {
                if f(t, event) {
                    return gtk4::glib::Propagation::Stop;
                }
            }
        }
        gtk4::glib::Propagation::Proceed
    });
    widget.add_controller(controller);
    id
}

pub trait WidgetEventCompat: gtk4::prelude::IsA<gtk4::Widget> + gtk4::prelude::Cast + 'static {
    fn connect_button_press_event<F>(&self, f: F) -> glib::SignalHandlerId
    where
        F: Fn(&Self, &gdk4::Event) -> bool + 'static,
    {
        widget_legacy_event_connect(self, gdk4::EventType::ButtonPress, f)
    }

    fn connect_button_release_event<F>(&self, f: F) -> glib::SignalHandlerId
    where
        F: Fn(&Self, &gdk4::Event) -> bool + 'static,
    {
        widget_legacy_event_connect(self, gdk4::EventType::ButtonRelease, f)
    }

    fn connect_motion_notify_event<F>(&self, f: F) -> glib::SignalHandlerId
    where
        F: Fn(&Self, &gdk4::Event) -> bool + 'static,
    {
        widget_legacy_event_connect(self, gdk4::EventType::MotionNotify, f)
    }

    fn connect_key_press_event<F>(&self, f: F) -> glib::SignalHandlerId
    where
        F: Fn(&Self, &gdk4::Event) -> bool + 'static,
    {
        widget_legacy_event_connect(self, gdk4::EventType::KeyPress, f)
    }

    fn connect_key_release_event<F>(&self, f: F) -> glib::SignalHandlerId
    where
        F: Fn(&Self, &gdk4::Event) -> bool + 'static,
    {
        widget_legacy_event_connect(self, gdk4::EventType::KeyRelease, f)
    }

    fn connect_scroll_event<F>(&self, f: F) -> glib::SignalHandlerId
    where
        F: Fn(&Self, &gdk4::Event) -> bool + 'static,
    {
        widget_legacy_event_connect(self, gdk4::EventType::Scroll, f)
    }

    fn connect_focus_in_event<F>(&self, f: F) -> glib::SignalHandlerId
    where
        F: Fn(&Self, &gdk4::Event) -> bool + 'static,
    {
        widget_focus_in_event_connect(self, f)
    }
}

impl<T: gtk4::prelude::IsA<gtk4::Widget> + gtk4::prelude::Cast + 'static> WidgetEventCompat for T {}

pub trait GtkContainerCompat: gtk4::prelude::WidgetExt {
    fn children(&self) -> Vec<Widget>;
}

pub trait GtkBoxCompat {
    fn set_child_position(&self, child: &impl gtk4::prelude::IsA<gtk4::Widget>, position: i32);
    fn child_position(&self, child: &impl gtk4::prelude::IsA<gtk4::Widget>) -> i32;
    fn pack_start(
        &self,
        child: &impl gtk4::prelude::IsA<gtk4::Widget>,
        _expand: bool,
        _fill: bool,
        _padding: u32,
    );
}

impl GtkBoxCompat for gtk4::Box {
    fn set_child_position(&self, child: &impl gtk4::prelude::IsA<gtk4::Widget>, position: i32) {
        let widgets = self.children();
        if position <= 0 {
            self.reorder_child_after(child.upcast_ref(), None::<&gtk4::Widget>);
        } else {
            let sibling = widgets.get(position as usize - 1).cloned();
            self.reorder_child_after(child.upcast_ref(), sibling.as_ref());
        }
    }

    fn child_position(&self, child: &impl gtk4::prelude::IsA<gtk4::Widget>) -> i32 {
        let child = child.upcast_ref::<gtk4::Widget>();
        self.children()
            .iter()
            .position(|widget| widget == child)
            .map_or(-1, |index| index as i32)
    }

    fn pack_start(
        &self,
        child: &impl gtk4::prelude::IsA<gtk4::Widget>,
        _expand: bool,
        _fill: bool,
        _padding: u32,
    ) {
        self.append(child);
    }
}

impl GtkContainerCompat for gtk4::Box {
    fn children(&self) -> Vec<Widget> {
        let mut result = Vec::new();
        let mut child = self.first_child();
        while let Some(c) = child {
            result.push(c.clone());
            child = c.next_sibling();
        }
        result
    }
}

impl GtkContainerCompat for gtk4::Frame {
    fn children(&self) -> Vec<Widget> {
        let mut result = Vec::new();
        if let Some(child) = self.first_child() {
            result.push(child.clone());
            let mut next = child.next_sibling();
            while let Some(c) = next {
                result.push(c.clone());
                next = c.next_sibling();
            }
        }
        result
    }
}

impl GtkContainerCompat for gtk4::ScrolledWindow {
    fn children(&self) -> Vec<Widget> {
        let mut result = Vec::new();
        let mut child = self.first_child();
        while let Some(c) = child {
            result.push(c.clone());
            child = c.next_sibling();
        }
        result
    }
}

impl GtkContainerCompat for gtk4::DropDown {
    fn children(&self) -> Vec<Widget> {
        let mut result = Vec::new();
        let mut child = self.first_child();
        while let Some(c) = child {
            result.push(c.clone());
            child = c.next_sibling();
        }
        result
    }
}

impl GtkContainerCompat for gtk4::Viewport {
    fn children(&self) -> Vec<Widget> {
        let mut result = Vec::new();
        let mut child = self.first_child();
        while let Some(c) = child {
            result.push(c.clone());
            child = c.next_sibling();
        }
        result
    }
}

impl GtkContainerCompat for gtk4::Paned {
    fn children(&self) -> Vec<Widget> {
        let mut result = Vec::new();
        let mut child = self.first_child();
        while let Some(c) = child {
            result.push(c.clone());
            child = c.next_sibling();
        }
        result
    }
}

impl GtkContainerCompat for gtk4::Grid {
    fn children(&self) -> Vec<Widget> {
        let mut result = Vec::new();
        let mut child = self.first_child();
        while let Some(c) = child {
            result.push(c.clone());
            child = c.next_sibling();
        }
        result
    }
}

impl GtkContainerCompat for gtk4::ApplicationWindow {
    fn children(&self) -> Vec<Widget> {
        let mut result = Vec::new();
        let mut child = self.first_child();
        while let Some(c) = child {
            result.push(c.clone());
            child = c.next_sibling();
        }
        result
    }
}

impl GtkContainerCompat for gtk4::Window {
    fn children(&self) -> Vec<Widget> {
        let mut result = Vec::new();
        let mut child = self.first_child();
        while let Some(c) = child {
            result.push(c.clone());
            child = c.next_sibling();
        }
        result
    }
}

impl GtkContainerCompat for Widget {
    fn children(&self) -> Vec<Widget> {
        let mut result = Vec::new();
        let mut child = self.first_child();
        while let Some(c) = child {
            result.push(c.clone());
            child = c.next_sibling();
        }
        result
    }
}

/// Replacement for the GTK3 `Dialog::run()` blocking dialog exec.
///
/// GTK3's `Dialog::run()` made the dialog modal, showed it, and ran a nested
/// main loop until the user responded. GTK4 removed `Dialog::run()` in favour
/// of `present()` + the `response` signal, so this compatibility trait
/// reimplements the blocking behaviour with a nested `glib::MainLoop`.
pub trait GtkDialogRunCompat: gtk4::prelude::IsA<gtk4::Window> + gtk4::prelude::Cast + 'static {
    fn run(&self) -> gtk4::ResponseType {
        use std::cell::Cell;
        use std::rc::Rc;

        let response_holder: Rc<Cell<Option<gtk4::ResponseType>>> = Rc::new(Cell::new(None));
        let main_loop = Rc::new(glib::MainLoop::new(None, false));

        let close_response_holder = response_holder.clone();
        let close_main_loop = main_loop.clone();
        self.connect_close_request(move |_| {
            if close_main_loop.is_running() {
                close_response_holder.set(Some(gtk4::ResponseType::DeleteEvent));
                close_main_loop.quit();
            }
            glib::Propagation::Proceed
        });

        self.set_modal(true);
        self.present();
        main_loop.run();
        response_holder.get().unwrap_or(gtk4::ResponseType::DeleteEvent)
    }
}

impl<T: gtk4::prelude::IsA<gtk4::Window> + gtk4::prelude::Cast + 'static> GtkDialogRunCompat for T {}

/// GTK3-style drag target flags (a plain bitmask; GTK4 has no equivalent).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetFlags(u32);

impl TargetFlags {
    pub const SAME_APP: Self = Self(0x1);
    pub const SAME_WIDGET: Self = Self(0x2);
    pub const OTHER_APP: Self = Self(0x4);
    pub const OTHER_WIDGET: Self = Self(0x8);
}

/// GTK3-style `GtkTargetEntry`.
#[derive(Debug, Clone, Copy)]
pub struct TargetEntry {
    pub target: &'static str,
    pub flags: TargetFlags,
    pub info: u32,
}

impl TargetEntry {
    pub fn new(target: &'static str, flags: TargetFlags, info: u32) -> Self {
        Self { target, flags, info }
    }
}

/// GTK3-style drag destination default flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DestDefaults(u32);

impl DestDefaults {
    pub const ALL: Self = Self(0xffffffff);
    pub const MOTION: Self = Self(0x1);
    pub const HIGHLIGHT: Self = Self(0x2);
    pub const DROP: Self = Self(0x4);
}

/// Placeholder for the GTK3 `GdkDragContext`, which GTK4 has replaced with
/// the `gdk4::Drop` / `gdk4::Drag` APIs. The call sites only ever ignore it.
#[derive(Debug, Clone, Copy)]
pub struct DragContext;

impl DragContext {
    pub fn new() -> Self {
        Self
    }
}

/// Compatibility stand-in for the GTK3 `GtkSelectionData`.
///
/// GTK4 delivers drag&drop payloads as `glib::Value`s instead, so the GTK3
/// style "get"/"received" callbacks are adapted to an in-memory holder with
/// the same `set_text`/`text` surface.
#[derive(Default)]
pub struct SelectionData {
    text: RefCell<Option<String>>,
}

impl SelectionData {
    pub fn new() -> Self {
        Self { text: RefCell::new(None) }
    }

    pub fn set_text(&self, text: &str) -> bool {
        *self.text.borrow_mut() = Some(text.to_string());
        true
    }

    pub fn set(&self, text: &str, _length: i32) -> bool {
        self.set_text(text)
    }

    pub fn text(&self) -> Option<String> {
        self.text.borrow().clone()
    }
}

/// GTK4 translation of the GTK3 `gtk_widget_drag_source_set()` +
/// `connect_drag_data_get()` API.
///
/// Each source widget gets one `gdk4::DragSource` stored on it (as qdata).
/// `connect_drag_data_get` stores the GTK3-style callback on that `DragSource`
/// and wires it up to `connect_prepare`, feeding it a fresh `SelectionData`
/// and turning the produced text into a `gdk4::ContentProvider`.
pub trait GtkDragSourceCompat:
    gtk4::prelude::IsA<gtk4::Widget> + gtk4::prelude::Cast + 'static
{
    fn drag_source_set(
        &self,
        _start_button_mask: gdk4::ModifierType,
        _targets: &[TargetEntry],
        actions: gdk4::DragAction,
    ) {
        let source = gtk4::DragSource::new();
        source.set_actions(actions);
        self.add_controller(source.clone());
        unsafe {
            self.set_data("riff_gtk4_drag_source", source);
        }
    }

    fn connect_drag_data_get<F>(&self, f: F) -> glib::SignalHandlerId
    where
        F: Fn(&Self, DragContext, &SelectionData, u32, u32) + 'static,
    {
        let source = unsafe {
            match self.data::<gtk4::DragSource>("riff_gtk4_drag_source") {
                Some(ptr) => ptr.as_ref().clone(),
                None => {
                    let source = gtk4::DragSource::new();
                    source.set_actions(gdk4::DragAction::COPY);
                    self.set_data("riff_gtk4_drag_source", source.clone());
                    self.add_controller(source.clone());
                    source
                }
            }
        };
        unsafe {
            source.set_data(
                "riff_gtk4_drag_data_get_closure",
                Box::new(move |widget: &gtk4::Widget,
                                drag_context: DragContext,
                                selection_data: &SelectionData,
                                info: u32,
                                time: u32| {
                    if let Some(t) = widget.downcast_ref::<Self>() {
                        f(t, drag_context, selection_data, info, time);
                    }
                }) as Box<dyn Fn(&gtk4::Widget, DragContext, &SelectionData, u32, u32)>,
            );
        }
        source.connect_prepare(move |source, _x, _y| {
            let closure = unsafe {
                source
                    .data::<Box<dyn Fn(&gtk4::Widget, DragContext, &SelectionData, u32, u32)>>(
                        "riff_gtk4_drag_data_get_closure",
                    )
                    .map(|ptr| &*ptr.as_ptr())
            };
            if let Some(closure) = closure {
                if let Some(widget) = source.widget() {
                    let selection_data = SelectionData::new();
                    closure(&widget, DragContext, &selection_data, 0, 0);
                    if let Some(text) = selection_data.text() {
                        return Some(gdk4::ContentProvider::for_value(&text.to_value()));
                    }
                }
            }
            None
        })
    }
}

impl<T: gtk4::prelude::IsA<gtk4::Widget> + gtk4::prelude::Cast + 'static> GtkDragSourceCompat for T {}

/// GTK4 translation of the GTK3 `gtk_widget_drag_dest_set()` +
/// `connect_drag_motion()` + `connect_drag_data_received()` API.
///
/// Each destination widget gets one `gtk4::DropTarget` stored on it (as
/// qdata). The GTK3-style callbacks are stored on the `DropTarget` and wired
/// to `connect_motion` / `connect_drop`.
pub trait GtkDropDestCompat:
    gtk4::prelude::IsA<gtk4::Widget> + gtk4::prelude::Cast + 'static
{
    fn drag_dest_set(
        &self,
        _flags: DestDefaults,
        targets: &[TargetEntry],
        actions: gdk4::DragAction,
    ) {
        let mime_types: Vec<&str> = targets.iter().map(|entry| entry.target).collect();
        let formats = gdk4::ContentFormats::new(&mime_types);
        let drop_target = gtk4::DropTarget::builder()
            .actions(actions)
            .formats(&formats)
            .build();
        unsafe {
            self.set_data("riff_gtk4_drop_target", drop_target.clone());
        }
        self.add_controller(drop_target.clone());
    }

    fn connect_drag_motion<F>(&self, f: F) -> glib::SignalHandlerId
    where
        F: Fn(&Self, DragContext, i32, i32, u32) -> bool + 'static,
    {
        let drop_target = unsafe {
            match self.data::<gtk4::DropTarget>("riff_gtk4_drop_target") {
                Some(ptr) => ptr.as_ref().clone(),
                None => {
                    let drop_target = gtk4::DropTarget::builder()
                        .actions(gdk4::DragAction::COPY)
                        .build();
                    self.set_data("riff_gtk4_drop_target", drop_target.clone());
                    self.add_controller(drop_target.clone());
                    drop_target
                }
            }
        };
        unsafe {
            drop_target.set_data(
                "riff_gtk4_drag_motion_closure",
                Box::new(move |widget: &gtk4::Widget, x: f64, y: f64| {
                    if let Some(t) = widget.downcast_ref::<Self>() {
                        f(t, DragContext, x as i32, y as i32, 0)
                    } else {
                        false
                    }
                }) as Box<dyn Fn(&gtk4::Widget, f64, f64) -> bool>,
            );
        }
        drop_target.connect_motion(move |dt, x, y| {
            let closure = unsafe {
                dt.data::<Box<dyn Fn(&gtk4::Widget, f64, f64) -> bool>>(
                    "riff_gtk4_drag_motion_closure",
                )
                .map(|ptr| &*ptr.as_ptr())
            };
            if let Some(closure) = closure {
                if let Some(widget) = dt.widget() {
                    if closure(&widget, x, y) {
                        return gdk4::DragAction::COPY;
                    }
                }
            }
            gdk4::DragAction::empty()
        })
    }

    fn connect_drag_data_received<F>(&self, f: F) -> glib::SignalHandlerId
    where
        F: Fn(&Self, DragContext, i32, i32, &SelectionData, u32, u32) + 'static,
    {
        let drop_target = unsafe {
            match self.data::<gtk4::DropTarget>("riff_gtk4_drop_target") {
                Some(ptr) => ptr.as_ref().clone(),
                None => {
                    let drop_target = gtk4::DropTarget::builder()
                        .actions(gdk4::DragAction::COPY)
                        .build();
                    self.set_data("riff_gtk4_drop_target", drop_target.clone());
                    self.add_controller(drop_target.clone());
                    drop_target
                }
            }
        };
        unsafe {
            drop_target.set_data(
                "riff_gtk4_drag_data_received_closure",
                Box::new(move |widget: &gtk4::Widget,
                                value: &glib::Value,
                                x: f64,
                                y: f64| {
                    if let Some(t) = widget.downcast_ref::<Self>() {
                        let selection_data = SelectionData::new();
                        if let Ok(text) = value.get::<String>() {
                            selection_data.set_text(&text);
                        }
                        f(t, DragContext, x as i32, y as i32, &selection_data, 0, 0);
                        true
                    } else {
                        false
                    }
                }) as Box<dyn Fn(&gtk4::Widget, &glib::Value, f64, f64) -> bool>,
            );
        }
        drop_target.connect_drop(move |dt, value, x, y| {
            let closure = unsafe {
                dt.data::<Box<dyn Fn(&gtk4::Widget, &glib::Value, f64, f64) -> bool>>(
                    "riff_gtk4_drag_data_received_closure",
                )
                .map(|ptr| &*ptr.as_ptr())
            };
            if let Some(closure) = closure {
                if let Some(widget) = dt.widget() {
                    return closure(&widget, value, x, y);
                }
            }
            false
        })
    }
}

impl<T: gtk4::prelude::IsA<gtk4::Widget> + gtk4::prelude::Cast + 'static> GtkDropDestCompat for T {}

/// GTK3's `TreeModelExt::value` was renamed to `get_value` in GTK4, this
/// compatibility trait restores the old name.

/// GTK4 replacement for the GTK3 `GtkFileChooserDialog`.
///
/// GTK4 replaced the chooser dialog with the async `gtk4::FileDialog`. This
/// wrapper keeps the old synchronous-looking call sites (`new` -> `run` ->
/// `filename`) by driving the `FileDialog` future with a nested
/// `MainContext::block_on` inside `run()`.
#[derive(Clone)]
pub struct FileChooserDialog {
    title: Option<String>,
    action: gtk4::FileChooserAction,
    parent: Option<gtk4::Window>,
    result: std::rc::Rc<std::cell::RefCell<Option<gio::File>>>,
    filters: std::rc::Rc<std::cell::RefCell<Vec<gtk4::FileFilter>>>,
}

impl FileChooserDialog {
    pub fn new(
        title: Option<&str>,
        parent: Option<&impl gtk4::prelude::IsA<gtk4::Window>>,
        action: gtk4::FileChooserAction,
    ) -> Self {
        Self {
            title: title.map(|t| t.to_string()),
            action,
            parent: parent.map(|p| p.as_ref().clone()),
            result: std::rc::Rc::new(std::cell::RefCell::new(None)),
            filters: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
        }
    }

    pub fn run(&self) -> gtk4::ResponseType {
        let file_dialog = gtk4::FileDialog::new();
        if let Some(title) = &self.title {
            file_dialog.set_title(title);
        }
        file_dialog.set_modal(true);
        {
            let filters = self.filters.borrow();
            if !filters.is_empty() {
                let store = gio::ListStore::new::<gtk4::FileFilter>();
                for filter in filters.iter() {
                    store.append(filter);
                }
                file_dialog.set_filters(Some(&store));
            }
        }
        let parent = self.parent.clone();
        let future = match self.action {
            gtk4::FileChooserAction::Save => file_dialog.save_future(parent.as_ref()),
            gtk4::FileChooserAction::SelectFolder => {
                file_dialog.select_folder_future(parent.as_ref())
            }
            _ => file_dialog.open_future(parent.as_ref()),
        };
        match glib::MainContext::default().block_on(future) {
            Ok(file) => {
                *self.result.borrow_mut() = Some(file);
                gtk4::ResponseType::Ok
            }
            Err(_) => gtk4::ResponseType::Cancel,
        }
    }

    pub fn add_button(&self, _button_text: &str, _response_id: gtk4::ResponseType) {
        // FileDialog provides its own accept/cancel buttons.
    }

    pub fn add_filter(&self, filter: &gtk4::FileFilter) {
        self.filters.borrow_mut().push(filter.clone());
    }

    pub fn file(&self) -> Option<gio::File> {
        self.result.borrow().clone()
    }

    pub fn filename(&self) -> Option<std::path::PathBuf> {
        self.result.borrow().as_ref().and_then(|file| file.path())
    }

    pub fn current_folder(&self) -> Option<std::path::PathBuf> {
        self.result
            .borrow()
            .as_ref()
            .and_then(|file| file.parent())
            .and_then(|folder| folder.path())
    }

    pub fn list_shortcut_folders(&self) -> Vec<std::path::PathBuf> {
        Vec::new()
    }

    pub fn add_shortcut_folder(&self, _folder: std::path::PathBuf) -> Result<(), glib::Error> {
        Ok(())
    }

    pub fn set_visible(&self, _visible: bool) {
        // The FileDialog tears itself down once responded to.
    }
}

/// The embedded `GtkFileChooserWidget` was deprecated in GTK 4.10 and GTK4
/// ships no drop-in replacement, so the sample library and scripting panels
/// keep using it (it remains fully functional in 4.16).
/// GTK3's `FileChooserWidget::connect_selection_changed` doesn't exist in
/// GTK4, so poll the widget's `filename` on a short timer.
pub trait FileChooserWidgetCompat {
    fn connect_selection_changed<F>(&self, f: F) -> glib::SourceId
    where
        F: Fn(&Self) + 'static;
}

#[allow(deprecated)] // no GTK4 replacement for the embedded chooser widget
impl FileChooserWidgetCompat for gtk4::FileChooserWidget {
    fn connect_selection_changed<F>(&self, f: F) -> glib::SourceId
    where
        F: Fn(&Self) + 'static,
    {
        let widget = self.clone();
        let mut last_path: Option<std::path::PathBuf> = None;
        glib::timeout_add_local(Duration::from_millis(200), move || {
            let current_path = widget.file().and_then(|file| file.path());
            if current_path != last_path {
                last_path = current_path;
                f(&widget);
            }
            glib::ControlFlow::Continue
        })
    }
}

/// GTK4 stand-in for the GTK3 `GtkRecentInfo` returned by the recent chooser.
#[derive(Debug, Clone)]
pub struct RecentInfo {
    pub uri: String,
    pub display_name: String,
}

impl RecentInfo {
    pub fn uri_display(&self) -> Option<String> {
        if !self.display_name.is_empty() {
            return Some(self.display_name.clone());
        }
        self.uri
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .map(|name| name.to_string())
    }
}

/// Extracts `(uri, display-name)` from a recent-files variant entry, be it a
/// dictionary (`{"uri": ..., "display-name": ...}`), a tuple (`("uri",
/// "display")`) or a bare URI string.
fn variant_uri_and_display(variant: &glib::Variant) -> Option<(String, Option<String>)> {
    if let Some(uri) = variant.try_child_value(0) {
        if let Some(uri) = uri.str() {
            if let Some(display) = variant.try_child_value(1) {
                if let Some(display) = display.str() {
                    return Some((uri.to_string(), Some(display.to_string())));
                }
            }
            return Some((uri.to_string(), None));
        }
    }
    if variant.n_children() >= 1 {
        let mut uri = None;
        let mut display = None;
        for index in 0..variant.n_children() {
            let entry = variant.child_value(index);
            if entry.n_children() == 2 {
                if let Some(key) = entry.child_value(0).str() {
                    let value = entry.child_value(1);
                    if key == "uri" {
                        if let Some(uri_value) = value.str() {
                            uri = Some(uri_value.to_string());
                        }
                    } else if key == "display-name" {
                        if let Some(display_value) = value.str() {
                            display = Some(display_value.to_string());
                        }
                    }
                }
            }
        }
        if let Some(uri) = uri {
            return Some((uri, display));
        }
    }
    if let Some(uri) = variant.str() {
        return Some((uri.to_string(), None));
    }
    None
}

fn recent_files_from_settings() -> Vec<(String, Option<String>)> {
    let mut recent = Vec::new();
    if let Some(schema_source) = gio::SettingsSchemaSource::default() {
        if schema_source.lookup("org.gtk.recent-files", true).is_none() {
            return recent;
        }
    } else {
        return recent;
    }
    let settings = gio::Settings::new("org.gtk.recent-files");
    let recent_files = settings.value("recent-files");
    for index in 0..recent_files.n_children().min(50) {
        if let Some((uri, display)) = variant_uri_and_display(&recent_files.child_value(index)) {
            recent.push((uri, display));
        }
    }
    recent
}

/// GTK3's `RecentChooser`/`RecentChooserMenu` API on a `gtk4::MenuButton`,
/// backed by a popover listing the entries of the `org.gtk.recent-files`
/// GSettings schema.
pub trait RecentChooserMenuCompat: gtk4::prelude::IsA<gtk4::MenuButton> + gtk4::prelude::Cast + 'static {
    fn connect_item_activated<F>(&self, f: F) -> glib::SignalHandlerId
    where
        F: Fn(&Self) + 'static,
    {
        let menu_button: gtk4::MenuButton = self.upcast_ref::<gtk4::MenuButton>().clone();
        unsafe {
            menu_button.set_data(
                "riff_gtk4_recent_activated_closure",
                Box::new(move |target: &gtk4::MenuButton| {
                    if let Some(t) = target.downcast_ref::<Self>() {
                        f(t);
                    }
                }) as Box<dyn Fn(&gtk4::MenuButton)>,
            );
        }

        let popover = gtk4::Popover::new();
        let list_box = gtk4::ListBox::new();
        list_box.set_selection_mode(gtk4::SelectionMode::None);
        popover.set_child(Some(&list_box));
        popover.set_position(gtk4::PositionType::Bottom);

        for (uri, display) in recent_files_from_settings() {
            let display_name = match display {
                Some(display) => display,
                None => uri
                    .rsplit('/')
                    .next()
                    .filter(|name| !name.is_empty())
                    .unwrap_or(uri.as_str())
                    .to_string(),
            };
            let row = gtk4::ListBoxRow::new();
            row.set_activatable(true);
            let label = gtk4::Label::new(Some(display_name.as_str()));
            label.set_halign(gtk4::Align::Start);
            label.set_max_width_chars(60);
            label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
            row.set_child(Some(&label));
            unsafe {
                row.set_data::<RecentInfo>(
                    "riff_gtk4_recent_info",
                    RecentInfo {
                        uri: uri.clone(),
                        display_name,
                    },
                );
            }
            list_box.append(&row);
        }

        if list_box.first_child().is_none() {
            let row = gtk4::ListBoxRow::new();
            let label = gtk4::Label::new(Some("No recent files"));
            label.set_halign(gtk4::Align::Start);
            row.set_child(Some(&label));
            list_box.append(&row);
        }

        let menu_button_for_item = menu_button.clone();
        let id = list_box.connect_row_activated(move |_, row| {
            if let Some(info) = unsafe {
                row.data::<RecentInfo>("riff_gtk4_recent_info")
                    .map(|ptr| &*ptr.as_ptr())
            } {
                let info = info.clone();
                unsafe {
                    menu_button_for_item.set_data::<RecentInfo>(
                        "riff_gtk4_recent_current_item",
                        info,
                    );
                }
            }
            if let Some(closure) = unsafe {
                menu_button_for_item
                    .data::<Box<dyn Fn(&gtk4::MenuButton)>>("riff_gtk4_recent_activated_closure")
                    .map(|ptr| &*ptr.as_ptr())
            } {
                closure(&menu_button_for_item);
            }
        });

        unsafe {
            menu_button.set_data::<gtk4::Popover>("riff_gtk4_recent_popover", popover.clone());
        }
        menu_button.set_popover(Some(&popover));
        id
    }

    fn current_item(&self) -> Option<RecentInfo> {
        let menu_button: &gtk4::MenuButton = self.upcast_ref::<gtk4::MenuButton>();
        unsafe {
            menu_button
                .data::<RecentInfo>("riff_gtk4_recent_current_item")
                .map(|ptr| ptr.as_ref().clone())
        }
    }
}

impl<T: gtk4::prelude::IsA<gtk4::MenuButton> + gtk4::prelude::Cast + 'static>
    RecentChooserMenuCompat for T
{}

/// Replacement for the deprecated `gtk4::MessageDialog` + `Dialog::run()`
/// pattern: shows a modal `gtk4::AlertDialog` with the given buttons and
/// blocks (nested main context) until the user responds. Returns the chosen
/// button index, or None if dismissed.
pub fn alert_dialog<P: gtk4::prelude::IsA<gtk4::Window> + Clone + 'static>(
    parent: Option<&P>,
    text: &str,
    buttons: &[&str],
) -> Option<usize> {
    let alert = gtk4::AlertDialog::builder().modal(true).build();
    alert.set_property("text", text);
    alert.set_buttons(buttons);
    match gtk4::glib::MainContext::default().block_on(alert.choose_future(parent)) {
        Ok(index) if index >= 0 => Some(index as usize),
        _ => None,
    }
}

/// Sets up a `gtk4::ListView` with a `SingleSelection` backed by a
/// `gio::ListStore` of `ComboItemObject` rows, rendering each row's `text`
/// property in a plain label. Returns the selection model and the store.
pub fn setup_text_list_view(
    list_view: &gtk4::ListView,
) -> (gtk4::SingleSelection, gio::ListStore) {
    use crate::combo_box_text_compat::ComboItemObject;

    let store = gio::ListStore::new::<ComboItemObject>();
    let selection = gtk4::SingleSelection::new(Some(store.clone()));
    let factory = gtk4::SignalListItemFactory::new();
    factory.connect_setup(move |_, list_item| {
        let list_item = list_item.downcast_ref::<gtk4::ListItem>().unwrap();
        let label = gtk4::Label::builder().xalign(0.0).build();
        list_item.set_child(Some(&label));
    });
    factory.connect_bind(move |_, list_item| {
        let list_item = list_item.downcast_ref::<gtk4::ListItem>().unwrap();
        if let (Some(label), Some(row)) = (
            list_item.child().and_then(|w| w.downcast::<gtk4::Label>().ok()),
            list_item.item().and_then(|i| i.downcast::<ComboItemObject>().ok()),
        ) {
            label.set_text(row.text().as_str());
        }
    });
    list_view.set_factory(Some(&factory));
    list_view.set_model(Some(&selection));
    (selection, store)
}

/// The `SingleSelection` currently installed on a `gtk4::ListView`.
pub fn list_view_selection(list_view: &gtk4::ListView) -> Option<gtk4::SingleSelection> {
    list_view.model().and_then(|m| m.downcast::<gtk4::SingleSelection>().ok())
}

/// The `gio::ListStore` behind a `gtk4::ListView`'s selection model.
pub fn list_view_store(list_view: &gtk4::ListView) -> Option<gio::ListStore> {
    list_view_selection(list_view)?.model()?.downcast::<gio::ListStore>().ok()
}

/// The selected row of a `gtk4::ListView` as a `ComboItemObject`.
pub fn list_view_selected_row(
    list_view: &gtk4::ListView,
) -> Option<crate::combo_box_text_compat::ComboItemObject> {
    list_view_selection(list_view)?
        .selected_item()
        .and_then(|item| item.downcast().ok())
}

/// The embedded `GtkFileChooserWidget` was deprecated in GTK 4.10 and GTK4
/// ships no drop-in replacement, so the sample library and scripting panels
/// keep using it (it remains fully functional in 4.16). This alias keeps the
/// deprecation confined to this module.
#[allow(deprecated)]
pub type EmbeddedFileChooser = gtk4::FileChooserWidget;

pub trait EmbeddedFileChooserCompat {
    fn selected_file(&self) -> Option<gio::File>;
}

#[allow(deprecated)]
impl EmbeddedFileChooserCompat for EmbeddedFileChooser {
    fn selected_file(&self) -> Option<gio::File> {
        self.file()
    }
}
