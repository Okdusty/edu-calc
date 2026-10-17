//! Regressions for the standard Advanced keypad. Simple has its own resize UI.
use super::*;
use gtk::{gio, glib};
use std::time::{Duration, Instant};

fn settle() {
    let ctx = glib::MainContext::default();
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(600) {
        for _ in 0..100 {
            if !ctx.pending() {
                break;
            }
            ctx.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn screenshot(ui: &crate::gui::Ui, name: &str) {
    let image = gtk::WidgetPaintable::new(Some(&ui.win));
    ui.win.queue_draw();
    settle();
    let snapshot = gtk::Snapshot::new();
    image.snapshot(&snapshot, ui.win.width() as f64, ui.win.height() as f64);
    let node = snapshot.to_node().unwrap();
    ui.win.renderer().unwrap().render_texture(&node, None).save_to_png(format!("target/{name}.png")).unwrap();
}

fn check_keypad(ui: &crate::gui::Ui) {
    // The direct mode block and number pad have no resize wrapper or grip.
    assert_eq!(ui.keypad.root.first_child(), Some(ui.keypad.modes.clone().upcast()));
    assert_eq!(ui.keypad.root.last_child(), Some(ui.keypad.numpad.clone().upcast()));
    let (enter, _) = ui.keypad.keys.iter().find(|(_, a)| matches!(a, Act::Enter)).unwrap();
    let key = enter.compute_bounds(&ui.win).unwrap();
    let pad = ui.keypad.root.compute_bounds(&ui.win).unwrap();
    assert!(key.y() >= pad.y() && key.y() + key.height() <= pad.y() + pad.height() + 1.0);
    assert!(key.x() >= pad.x() && key.x() + key.width() <= pad.x() + pad.width() + 1.0);
    assert!(key.y() + key.height() <= ui.win.height() as f32);
}

#[test]
#[ignore = "requires a display and isolated XDG_CONFIG_HOME; run with --test-threads=1"]
fn advanced_layout_smoke() {
    assert!(std::env::var_os("XDG_CONFIG_HOME").is_some());
    gtk::init().unwrap();
    let path = crate::gui::ini_path();
    let (mut cfg, _) = crate::gui::config::Config::load(&path);
    cfg.width = 1366;
    cfg.height = 768;
    cfg.maximized = false;
    cfg.calc_mode = true;
    cfg.keypad_mode = "general";
    cfg.number_pad = true;
    cfg.save_state(&path).unwrap();
    let app = gtk::Application::builder().application_id("com.dusty.qcalcp.advancedtest").flags(gio::ApplicationFlags::NON_UNIQUE).build();
    app.register(None::<&gio::Cancellable>).unwrap();
    let ui = crate::gui::build(&app, None);
    ui.cfg.borrow_mut().keypad_side = "right";
    crate::gui::show_simple(&ui, false);
    ui.win.set_size_request(1366, 768);
    settle();
    check_keypad(&ui);
    assert!(ui.keypad.root.width() < ui.win.width() * 3 / 5);
    screenshot(&ui, "advanced-no-resize-right-1366");

    ui.cfg.borrow_mut().keypad_side = "bottom";
    crate::gui::apply_layout(&ui);
    settle();
    check_keypad(&ui);
    screenshot(&ui, "advanced-no-resize-bottom-1366");
    ui.cfg.borrow_mut().calc_mode = false;
    ui.cfg.borrow_mut().keypad_octave = true;
    crate::gui::apply_layout(&ui);
    settle();
    check_keypad(&ui);
    screenshot(&ui, "advanced-no-resize-octave-1366");

    ui.cfg.borrow_mut().calc_mode = true;
    ui.cfg.borrow_mut().keypad_side = "right";
    crate::gui::apply_layout(&ui);
    ui.win.set_size_request(1920, 1080);
    ui.win.set_default_size(1920, 1080);
    settle();
    check_keypad(&ui);
    screenshot(&ui, "advanced-no-resize-right-1920");
    ui.win.close();
}
