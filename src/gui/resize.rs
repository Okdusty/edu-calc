//! Shared touch/mouse height grip for the Simple and Advanced keypads.
use gtk::prelude::*;
use gtk4 as gtk;
use std::{cell::Cell, rc::Rc};

pub(super) struct KeypadHeight {
    pub root: gtk::Box,
    child: gtk::Widget,
    preferred: Cell<Option<i32>>,
    default: Cell<i32>,
    minimum: i32,
    reserve: i32,
}

/// Widget point -> window y. The grip moves while it resizes, so drag offsets in its own
/// coordinates feed back into themselves (the height flickers between two values).
fn window_y(d: &gtk::GestureDrag, x: f64, y: f64) -> f64 {
    let w = d.widget();
    w.as_ref().and_then(|w| w.compute_point(&w.root()?, &gtk::graphene::Point::new(x as f32, y as f32))).map_or(y, |p| p.y() as f64)
}

impl KeypadHeight {
    pub fn new(child: &impl IsA<gtk::Widget>, height: i32, minimum: i32, reserve: i32) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_vexpand(false);
        let grip = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        grip.add_css_class("keypad-height-grip");
        grip.set_cursor_from_name(Some("ns-resize"));
        grip.set_focusable(true);
        grip.set_tooltip_text(Some("Drag up or down to resize the keypad. Arrow keys adjust height; double-click or Home resets."));
        let handle = gtk::Box::builder().css_classes(["keypad-height-handle"]).hexpand(true).halign(gtk::Align::Center).valign(gtk::Align::Center).build();
        grip.append(&handle);
        // no scroller: below the keys' own minimum the request is ignored, so a row is never cut off
        let child = child.clone().upcast::<gtk::Widget>();
        child.set_height_request(height);
        root.append(&grip);
        root.append(&child);
        let s = Rc::new(Self { root, child, preferred: Cell::new(None), default: Cell::new(height), minimum, reserve });
        let start = Rc::new(Cell::new((height, 0.0)));
        let drag = gtk::GestureDrag::new();
        let (weak, origin) = (Rc::downgrade(&s), start.clone());
        drag.connect_drag_begin(move |d, x, y| {
            if let Some(s) = weak.upgrade() {
                origin.set((s.child.height(), window_y(d, x, y)));
            }
        });
        let weak = Rc::downgrade(&s);
        drag.connect_drag_update(move |d, dx, dy| {
            let (Some(s), Some((x, y))) = (weak.upgrade(), d.start_point()) else { return };
            let (h, y0) = start.get();
            s.set_height(h - (window_y(d, x + dx, y + dy) - y0).round() as i32);
        });
        grip.add_controller(drag);
        let tap = gtk::GestureClick::new();
        let weak = Rc::downgrade(&s);
        tap.connect_pressed(move |_, n, _, _| {
            if n == 2 {
                if let Some(s) = weak.upgrade() {
                    s.reset();
                }
            }
        });
        grip.add_controller(tap);
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(&s);
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(s) = weak.upgrade() else { return gtk::glib::Propagation::Proceed };
            match key {
                gtk::gdk::Key::Up => s.set_height(s.child.height() + 16),
                gtk::gdk::Key::Down => s.set_height(s.child.height() - 16),
                gtk::gdk::Key::Home => s.reset(),
                _ => return gtk::glib::Propagation::Proceed,
            }
            gtk::glib::Propagation::Stop
        });
        grip.add_controller(keys);
        s
    }

    pub fn set_height(&self, height: i32) {
        let maximum = self.root.root().map(|r| r.height() - self.reserve).unwrap_or(700).max(self.minimum);
        let height = height.clamp(self.minimum, maximum);
        self.preferred.set(Some(height));
        self.child.set_height_request(height);
    }

    fn reset(&self) {
        self.preferred.set(None);
        self.child.set_height_request(self.default.get());
    }
}
