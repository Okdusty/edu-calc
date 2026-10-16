//! Shared touch/mouse height grip for the Simple and Advanced keypads.
use gtk::prelude::*;
use gtk4 as gtk;
use std::{cell::Cell, rc::Rc};

pub(super) struct KeypadHeight {
    pub root: gtk::Box,
    viewport: gtk::ScrolledWindow,
    preferred: Cell<Option<i32>>,
    default: Cell<i32>,
    minimum: i32,
    reserve: i32,
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
        let text = gtk::Label::new(Some("↕ Drag to resize keypad"));
        text.set_hexpand(true);
        grip.append(&text);
        let viewport = gtk::ScrolledWindow::builder()
            .child(child)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .propagate_natural_width(true)
            .height_request(height)
            .build();
        root.append(&grip);
        root.append(&viewport);
        let s = Rc::new(Self { root, viewport, preferred: Cell::new(None), default: Cell::new(height), minimum, reserve });
        let start = Rc::new(Cell::new(height));
        let drag = gtk::GestureDrag::new();
        let (weak, origin) = (Rc::downgrade(&s), start.clone());
        drag.connect_drag_begin(move |_, _, _| {
            if let Some(s) = weak.upgrade() {
                origin.set(s.viewport.height());
            }
        });
        let weak = Rc::downgrade(&s);
        drag.connect_drag_update(move |_, _, dy| {
            if let Some(s) = weak.upgrade() {
                s.set_height(start.get() - dy.round() as i32);
            }
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
                gtk::gdk::Key::Up => s.set_height(s.viewport.height() + 16),
                gtk::gdk::Key::Down => s.set_height(s.viewport.height() - 16),
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
        self.viewport.set_height_request(height);
    }

    pub fn set_default_height(&self, height: i32) {
        self.default.set(height);
        if self.preferred.get().is_none() {
            self.viewport.set_height_request(height);
        }
    }

    fn reset(&self) {
        self.preferred.set(None);
        self.viewport.set_height_request(self.default.get());
    }
}
