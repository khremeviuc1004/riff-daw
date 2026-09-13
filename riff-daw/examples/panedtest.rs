use gtk4::{Application, ApplicationWindow, Box as GtkBox, Orientation, Paned, Label};
use gtk4::prelude::*;

fn dump(label: &str, paned: &Paned) {
    let a = paned.allocation();
    println!("{label}: widget-alloc-w={} pos={} max={} left-visible={} right-visible={}",
        a.width(), paned.position(), paned.max_position(),
        paned.first_child().map(|c| c.is_visible()).unwrap_or(false),
        paned.last_child().map(|c| c.is_visible()).unwrap_or(false));
}

fn main() {
    let app = Application::builder().application_id("org.test.panedtest").build();
    app.connect_startup(|app| {
        let window = ApplicationWindow::builder().application(app).default_width(800).default_height(300).build();
        let outer = GtkBox::new(Orientation::Horizontal, 0);
        let paned = Paned::new(Orientation::Horizontal);
        let left = Label::new(Some("LEFT"));
        left.set_width_request(200);
        let right = Label::new(Some("RIGHT"));
        right.set_hexpand(true);
        paned.set_start_child(Some(&left));
        paned.set_end_child(Some(&right));
        paned.set_position(600);
        outer.append(&paned);
        window.set_child(Some(&outer));
        window.present();

        let paned = paned.clone();
        let left = left.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
            dump("  AFTER-PRESENT", &paned);
            paned.set_position(10000);
            glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
                dump("  AFTER-SET-10000", &paned);
                left.set_visible(false);
                glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
                    dump("  AFTER-HIDE-LEFT", &paned);
                    paned.set_position(0);
                    glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
                        dump("  AFTER-POS-0", &paned);
                        std::process::exit(0);
                    });
                });
            });
        });
    });
    app.connect_activate(|_| {});
    let _ = app.run();
}