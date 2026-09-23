//! Native embedding windows for plugin editor GUIs (X11).
//!
//! GTK4 uses 32-bit ARGB visuals for its windows while many plugin GUIs (the u-he shell,
//! VSTGUI based editors etc.) draw with the screen's default 24-bit visual - embedding
//! them inside a GTK4 window makes their XCopyArea blits fail with BadMatch ("the editor
//! opens but never renders"). GTK4's X11 surface base window additionally is an
//! InputOnly/event shell whose visible content lives in GTK's own child window, so a
//! plain 24-bit child cannot be created inside it at all.
//!
//! The approach here mirrors what other Linux hosts do: create a small override-redirect
//! top-level window (root child, screen default visual), keep it positioned over the GTK
//! editor window's client area, and hand its X id to the plugin. The plugin creates its
//! editor inside this 24-bit window, where its back store and window depths agree.
//!
//! Plugins that follow the XEmbed protocol create their editor as a separate top-level
//! window and ask the host to embed it (EMBEDDED_NOTIFY message, answered with
//! WINDOW_ACTIVATE), so drain_xembed_events reparents such windows into the embedding
//! window; this keeps every plugin editor a real child that moves, resizes and hides with
//! the embedding window.

// X11/Xlib constant names mirror the C definitions on purpose.
#![allow(non_upper_case_globals)]

use std::ffi::c_void;
use std::sync::OnceLock;

type XDisplay = c_void;

const BadAccess: i32 = 10;
const BadWindow: i32 = 3;

thread_local! {
    static X_ERRORS: std::cell::RefCell<Vec<(i32, u64, u64)>> = const { std::cell::RefCell::new(Vec::new()) };
}

type XErrorHandler = extern "C" fn(*mut XDisplay, *mut c_void) -> i32;
static PREVIOUS_ERROR_HANDLER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Collects X errors for logging, and always forwards them to whichever handler was
/// installed before ours (GDK's). The plugin windows we operate on (XEmbed embed requests)
/// are foreign clients' windows, so reparent/geometry races with the plugin destroying a
/// window must not kill the DAW via Xlib's default abort handler. We must not swallow any
/// error either, even on our own connection: the handler is process-wide, and GTK's
/// error-trap logic counts these errors for its internal checks.
extern "C" fn x_error_handler(display: *mut XDisplay, event: *mut c_void) -> i32 {
    // XErrorEvent on 64-bit Linux: type (byte 0), serial (u64 at byte 8), error_code (byte 16),
    // request_code (byte 17), minor_code (byte 18), resourceid (u64 at byte 24).
    let (error_code, request_code, resource_id) = unsafe {
        let bytes = event as *const u8;
        (*bytes.add(16) as i32, *bytes.add(17) as u64, *(bytes.add(24) as *const u64))
    };
    if error_code == BadWindow || error_code == BadAccess {
        X_ERRORS.with(|errors| errors.borrow_mut().push((error_code, request_code, resource_id)));
    }
    let previous = PREVIOUS_ERROR_HANDLER.load(std::sync::atomic::Ordering::Relaxed);
    if previous != 0 {
        let previous: XErrorHandler = unsafe { std::mem::transmute(previous) };
        return previous(display, event);
    }
    0
}

/// Field order/types matching X11's XSetWindowAttributes on 64-bit Linux.
#[repr(C)]
struct XSetWindowAttributes {
    background_pixmap: u64,
    background_pixel: u64,
    border_pixmap: u64,
    border_pixel: u64,
    bit_gravity: i32,
    win_gravity: i32,
    backing_store: i32,
    backing_planes: u64,
    backing_pixel: u64,
    save_under: i32,
    event_mask: i64,
    do_not_propagate_mask: i64,
    override_redirect: i32,
    colormap: u64,
    cursor: u64,
}

const CW_BACK_PIXEL: u64 = 1 << 2;
const CW_EVENT_MASK: u64 = 1 << 11;
const CW_OVERRIDE_REDIRECT: u64 = 1 << 9;
/// SubstructureNotifyMask | StructureNotifyMask. (ClientMessage events are delivered to the
/// owning client regardless of event selection, and selecting bit 31 is not even legal -
/// XCreateWindow fails with BadValue.)
const EMBED_EVENT_MASK: i64 = (1 << 19) | (1 << 17);

// XEmbed protocol (X11 Xembed spec): ClientMessage on the embedding window, property
// _XEMBED, format 32, data[0] = timestamp, data[1] = version, data[2] = message type.
const XEMBED_VERSION_REQUEST: u32 = 1;
const XEMBED_EMBEDDED_NOTIFY: u32 = 0;
const XEMBED_WINDOW_ACTIVATE: u32 = 1;
const XEMBED_WINDOW_DEACTIVATE: u32 = 2;
const XEMBED_FOCUS_IN: u32 = 4;
const XEMBED_FOCUS_OUT: u32 = 5;
const XEMBED_VERSION_MAJOR: u64 = 0;
const XEMBED_VERSION_MINOR: u64 = 3;
/// XEMBED_NOTIFY_CURRENT - default focus-notify style for FOCUS_IN.
const XEMBED_NOTIFY_CURRENT: u64 = 0;

extern "C" {
    fn XOpenDisplay(display_name: *const i8) -> *mut XDisplay;
    fn XDefaultScreen(display: *mut XDisplay) -> i32;
    fn XDefaultVisual(display: *mut XDisplay, screen: i32) -> *mut c_void;
    fn XDefaultDepth(display: *mut XDisplay, screen: i32) -> i32;
    fn XDefaultRootWindow(display: *mut XDisplay) -> u64;
    fn XCreateColormap(display: *mut XDisplay, parent: u64, visual: *mut c_void, alloc: i32) -> u64;
    fn XCreateWindow(
        display: *mut XDisplay,
        parent: u64,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        border_width: u32,
        depth: i32,
        class: u32,
        visual: *mut c_void,
        valuemask: u64,
        attributes: *mut XSetWindowAttributes,
    ) -> u64;
    fn XMapRaised(display: *mut XDisplay, w: u64) -> i32;
    fn XUnmapWindow(display: *mut XDisplay, w: u64) -> i32;
    fn XMoveResizeWindow(display: *mut XDisplay, w: u64, x: i32, y: i32, width: u32, height: u32) -> i32;
    fn XTranslateCoordinates(display: *mut XDisplay, src_w: u64, dst_w: u64, src_x: i32, src_y: i32, dest_x_return: *mut i32, dest_y_return: *mut i32, child_return: *mut u64) -> i32;
    fn XQueryTree(display: *mut XDisplay, w: u64, root_return: *mut u64, parent_return: *mut u64, children_return: *mut *mut u64, nchildren_return: *mut u32) -> i32;
    fn XFree(data: *mut c_void) -> i32;
    fn XInternAtom(display: *mut XDisplay, name: *const i8, only_if_exists: i32) -> u64;
    fn XPending(display: *mut XDisplay) -> i32;
    fn XNextEvent(display: *mut XDisplay, event: *mut XEvent);
    fn XSendEvent(display: *mut XDisplay, w: u64, propagate: i32, event_mask: i64, event: *mut XEvent) -> i32;
    fn XReparentWindow(display: *mut XDisplay, w: u64, parent: u64, x: i32, y: i32);
    fn XGetWindowAttributes(display: *mut XDisplay, w: u64, attributes: *mut XWindowAttributes) -> i32;
    fn XSetErrorHandler(handler: XErrorHandler) -> Option<XErrorHandler>;
    fn XSync(display: *mut XDisplay, discard: i32) -> i32;
}

/// Field order/types matching X11's XWindowAttributes on 64-bit Linux (only x/y/width/
/// height/depth are used; the rest of the fields exist to keep the layout correct).
#[repr(C)]
#[allow(dead_code)]
struct XWindowAttributes {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    border_width: i32,
    depth: i32,
    visual: *mut c_void,
    root: u64,
    class: i32,
    bit_gravity: i32,
    win_gravity: i32,
    backing_store: i32,
    backing_planes: u64,
    backing_pixel: u64,
    save_under: i32,
    colormap: u64,
    map_installed: i32,
    map_state: i32,
    all_event_masks: i64,
    your_event_mask: i64,
    do_not_propagate_mask: i16,
    override_redirect: i16,
    pointer: u64,
}

/// Layout of X11's XEvent union (192 bytes, first field always the type). Only ClientMessage
/// is interpreted; everything else is drained and ignored.
#[repr(C)]
union XEvent {
    _pad: [u8; 192],
    ty: u32,
    client: XClientMessageEvent,
}

#[repr(C)]
#[derive(Clone, Copy)]
#[allow(non_snake_case)]
struct XClientMessageEvent {
    type_: u32,
    serial: u64,
    send_event: i32,
    display: *mut XDisplay,
    window: u64,
    message_type: u64,
    format: i32,
    pad: i32,
    data: [u64; 5],
}

const ClientMessage: u32 = 33;
const NotifyMask: i64 = 1 << 17; // same bit as StructureNotifyMask

fn xembed_atom(display: *mut XDisplay) -> u64 {
    static XEMBED: OnceLock<u64> = OnceLock::new();
    *XEMBED.get_or_init(|| unsafe {
        let name = b"_XEMBED\0";
        XInternAtom(display, name.as_ptr() as *const i8, 0)
    })
}

/// Dedicated Xlib connection used only from the GTK main thread (window creation on
/// editor open, placement/visibility updates), hence safe to share as a static.
struct PluginXDisplay(*mut XDisplay);
unsafe impl Send for PluginXDisplay {}
unsafe impl Sync for PluginXDisplay {}

fn x_display() -> *mut XDisplay {
    static DISPLAY: OnceLock<PluginXDisplay> = OnceLock::new();
    DISPLAY.get_or_init(|| unsafe {
        // chain over whatever handler is installed (Xlib's aborts, GDK's logs): operations on
        // plugin-owned foreign windows can legitimately race with the plugin destroying them.
            let previous = XSetErrorHandler(x_error_handler);
        PREVIOUS_ERROR_HANDLER.store(previous.map_or(0, |handler| handler as usize), std::sync::atomic::Ordering::Relaxed);
        PluginXDisplay(XOpenDisplay(std::ptr::null()))
    }).0
}

/// Creates the override-redirect (screen default visual) plugin embedding window.
/// Returns its X id (0 on failure). The window starts unmapped; place it with
/// position_embedding_window as part of showing it.
pub fn create_embedding_window(width: u32, height: u32) -> u32 {
    let display = x_display();
    if display.is_null() {
        log::error!("xembed_host: couldn't open X display.");
        return 0;
    }

    unsafe {
        let screen = XDefaultScreen(display);
        let root = XDefaultRootWindow(display);
        let visual = XDefaultVisual(display, screen);
        let depth = XDefaultDepth(display, screen);
        let colormap = XCreateColormap(display, root, visual, 0); // AllocNone
        let mut attributes: XSetWindowAttributes = std::mem::zeroed();
        attributes.override_redirect = 1;
        attributes.background_pixel = 0;
        attributes.event_mask = EMBED_EVENT_MASK;
        let win = XCreateWindow(
            display,
            root,
            0,
            0,
            width.max(1),
            height.max(1),
            0,
            depth,
            1, // InputOutput
            visual,
            CW_OVERRIDE_REDIRECT | CW_BACK_PIXEL | CW_EVENT_MASK,
            &mut attributes as *mut XSetWindowAttributes,
        );
        if win == 0 {
            log::error!("xembed_host: couldn't create the plugin embedding window.");
            return 0;
        }
        // keep the colormap alive: it is referenced by the window (freed only when the
        // connection closes at program exit).
        let _ = colormap;
        log::debug!("xembed_host: plugin embedding window {:#x} created ({}x{}, depth {}).", win, width, height, depth);
        win as u32
    }
}

/// Top-left of `anchor_window` (the GTK editor window's surface) in root coordinates,
/// or None when the anchor isn't viewable (unmapped/destroyed surface).
pub fn anchor_root_origin(anchor_window: u32) -> Option<(i32, i32)> {
    let display = x_display();
    if display.is_null() || anchor_window == 0 {
        return None;
    }
    unsafe {
        let root = XDefaultRootWindow(display);
        let mut x = 0i32;
        let mut y = 0i32;
        let mut child = 0u64;
        if XTranslateCoordinates(display, anchor_window as u64, root, 0, 0, &mut x, &mut y, &mut child) == 0 {
            return None;
        }
        Some((x, y))
    }
}

/// Positions/resizes the embedding window at root coordinates `(x, y)` above the GTK
/// editor window (using `width`/`height` as the client area size), raising it so it stays
/// on top, and makes the plugin's own editor window (a child of the embedding window)
/// fill the embedding window so it receives a ConfigureNotify and redraws. When `visible`
/// is false the embedding window is unmapped (editor hidden). The anchor window's X id is
/// not needed on the hide path: GTK4 destroys a top-level's surface when it is hidden.
pub fn position_embedding_window(embed_xid: u32, visible: bool, x: i32, y: i32, width: u32, height: u32) {
    let display = x_display();
    if display.is_null() || embed_xid == 0 {
        return;
    }
    unsafe {
        if !visible {
            for window in embedded_windows(display, embed_xid) {
                xembed_message(display, window, embed_xid as u64, XEMBED_WINDOW_DEACTIVATE, 0, 0);
                xembed_message(display, window, embed_xid as u64, XEMBED_FOCUS_OUT, 0, 0);
            }
            XUnmapWindow(display, embed_xid as u64);
            return;
        }
        let width = width.max(1);
        let height = height.max(1);
        XMoveResizeWindow(display, embed_xid as u64, x, y, width, height);
        XMapRaised(display, embed_xid as u64);
        resize_embedded_children(display, embed_xid as u64, width, height);
        let embedded = embedded_windows(display, embed_xid);
        for (index, window) in embedded.iter().enumerate() {
            xembed_message(display, *window, embed_xid as u64, XEMBED_WINDOW_ACTIVATE, 0, 0);
            xembed_message(display, *window, embed_xid as u64, XEMBED_FOCUS_IN, 0, if index == 0 { XEMBED_NOTIFY_CURRENT } else { 0 });
        }
    }
}

/// Direct children of the embedding window that are the plugin's editor windows (all of
/// them - most plugins create exactly one).
unsafe fn embedded_windows(display: *mut XDisplay, embed_xid: u32) -> Vec<u64> {
    let mut root = 0u64;
    let mut parent = 0u64;
    let mut children: *mut u64 = std::ptr::null_mut();
    let mut nchildren = 0u32;
    let mut windows = Vec::new();
    if XQueryTree(display, embed_xid as u64, &mut root, &mut parent, &mut children, &mut nchildren) != 0 {
        if !children.is_null() {
            for i in 0..nchildren as usize {
                windows.push(*children.add(i));
            }
            XFree(children as *mut c_void);
        }
    }
    windows
}

/// Drains this connection's event queue, handling `_XEMBED` protocol messages sent to the
/// embedding windows. XEmbed-conformant plugins (CLAP/many VST3 editors) do not reparent
/// themselves: they create a top-level window and ask the host to embed it by sending
/// EMBEDDED_NOTIFY to the given xid, waiting for WINDOW_ACTIVATE. Without this handling the
/// plugin's editor floats over the embedding window: it does not move/resize with it and
/// survives the embedding window being unmapped when the editor is closed.
///
/// Called from the GTK main loop on every tracking tick; a no-op when no events are pending.
pub fn drain_xembed_events() {
    let display = x_display();
    if display.is_null() {
        return;
    }
    let xembed = xembed_atom(display);
    unsafe {
        while XPending(display) > 0 {
            let mut event: XEvent = std::mem::zeroed();
            XNextEvent(display, &mut event);
            if event.ty != ClientMessage || event.client.message_type != xembed || event.client.format != 32 {
                continue;
            }
            let embed_window = event.client.window;
            let data = event.client.data.as_ptr();
            let message = (data as *const u32).add(2).read_unaligned();
            match message {
                XEMBED_EMBEDDED_NOTIFY => {
                    // data.l[1] = the embedding window, data.l[2] = the window to embed.
                    let window = (data as *const u64).add(2).read_unaligned();
                    embed_window_request(display, embed_window, window);
                }
                XEMBED_VERSION_REQUEST => {
                    // answer with our protocol version: data.l[1]=1, data.l[2]=major, data.l[3]=minor.
                    let mut reply: XEvent = std::mem::zeroed();
                    reply.client.type_ = ClientMessage;
                    reply.client.window = embed_window;
                    reply.client.message_type = xembed;
                    reply.client.format = 32;
                    let reply_data = reply.client.data.as_mut_ptr();
                    (reply_data as *mut u32).add(2).write(XEMBED_VERSION_REQUEST);
                    (reply_data as *mut u32).add(4).write(XEMBED_VERSION_MAJOR as u32);
                    (reply_data as *mut u32).add(6).write(XEMBED_VERSION_MINOR as u32);
                    XSendEvent(display, embed_window, 0, NotifyMask, &mut reply);
                }
                XEMBED_FOCUS_IN | XEMBED_FOCUS_OUT => {
                    // focus bookkeeping - nothing to do.
                }
                _ => {}
            }
        }
        // flush the reparents/etc issued above so their X errors (if any) are collected now.
        XSync(display, 0);
        X_ERRORS.with(|collected| {
            let mut collected = collected.borrow_mut();
            for (code, request, resource) in collected.drain(..) {
                if code == BadWindow || code == BadAccess {
                    log::debug!("xembed_host: ignoring X error {code} (request {request}) on {resource:#x}.");
                } else {
                    log::error!("xembed_host: X error {code} (request {request}) on {resource:#x}.");
                }
            }
        });
    }
}

/// Reparents a plugin's editor window (sent via XEMBED_EMBEDDED_NOTIFY) into the embedding
/// window so it becomes a real child: it then moves/resizes with the embedding window and is
/// hidden when the embedding window is unmapped (editor window closed).
unsafe fn embed_window_request(display: *mut XDisplay, embed_xid: u64, window: u64) {
    let mut embed_attributes: XWindowAttributes = std::mem::zeroed();
    let mut child_attributes: XWindowAttributes = std::mem::zeroed();
    if XGetWindowAttributes(display, embed_xid, &mut embed_attributes) == 0 {
        log::error!("xembed_host: couldn't get attributes of embedding window {:#x}.", embed_xid);
        return;
    }
    if XGetWindowAttributes(display, window, &mut child_attributes) == 0 {
        log::error!("xembed_host: couldn't get attributes of XEmbed window {:#x}.", window);
        return;
    }
    if child_attributes.depth != embed_attributes.depth {
        log::error!("xembed_host: XEmbed window {:#x} depth {} doesn't match embedding window depth {} - not embedding.", window, child_attributes.depth, embed_attributes.depth);
        return;
    }
    XReparentWindow(display, window, embed_xid, 0, 0);
    // no event selection on the plugin's own window: on non-root windows the event mask is
    // per-window, so selecting would wipe the plugin's own expose/key/input selections -
    // child activity is observed via SubstructureNotifyMask on the embedding window instead.
    XMoveResizeWindow(display, window, 0, 0, embed_attributes.width.max(1) as u32, embed_attributes.height.max(1) as u32);
    XMapRaised(display, window);
    xembed_message(display, window, embed_xid, XEMBED_WINDOW_ACTIVATE, 0, 0);
    xembed_message(display, window, embed_xid, XEMBED_FOCUS_IN, 0, XEMBED_NOTIFY_CURRENT);
    log::debug!("xembed_host: embedded plugin window {:#x} into {:#x}.", window, embed_xid);
}

/// Sends an `_XEMBED` ClientMessage to `to` (the embedded window), as the embedding protocol
/// requires from the host: data.l[0] is the embedding window/timestamp, data.l[1] (low 32
/// bits) the message type, data.l[2]/data.l[3] the message-specific values.
unsafe fn xembed_message(display: *mut XDisplay, to: u64, embed_xid: u64, message: u32, data2: u64, data3: u64) {
    let mut event: XEvent = std::mem::zeroed();
    event.client.type_ = ClientMessage;
    event.client.window = to;
    event.client.message_type = xembed_atom(display);
    event.client.format = 32;
    let data = event.client.data.as_mut_ptr();
    (data as *mut u64).add(0).write(embed_xid);
    (data as *mut u32).add(2).write(message);
    (data as *mut u64).add(2).write(data2);
    (data as *mut u64).add(3).write(data3);
    XSendEvent(display, to, 0, NotifyMask, &mut event);
}

/// X11 does not resize a window's children when its parent is resized, and plugins do not
/// track their parent's size after opening the editor - so after moving/resizing the
/// embedding window, size every direct child (the plugin's editor window) to fill it. The
/// plugin then sees a normal ConfigureNotify and lays out/redraws at the new size.
unsafe fn resize_embedded_children(display: *mut XDisplay, embed_xid: u64, width: u32, height: u32) {
    let mut root = 0u64;
    let mut parent = 0u64;
    let mut children: *mut u64 = std::ptr::null_mut();
    let mut nchildren = 0u32;
    if XQueryTree(display, embed_xid, &mut root, &mut parent, &mut children, &mut nchildren) != 0 {
        if !children.is_null() {
            for i in 0..nchildren as usize {
                XMoveResizeWindow(display, *children.add(i), 0, 0, width, height);
            }
            XFree(children as *mut c_void);
        }
    }
}
