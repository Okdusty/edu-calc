//! Completion popup for the calculator entry, the REPL and the script editor: while an identifier
//! is typed, a popover under it lists matches from `crate::complete` (signature, dim one-line
//! description) without taking the keyboard focus. Up/Down select, Tab or Return accept, Esc
//! dismisses, a click or tap accepts. Text that code sets (`quiet`) never pops it up, and neither
//! does deleting while it's closed.
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{gdk, glib, graphene, pango};

use super::Ui;
use super::keypad::SLOT;
use crate::complete::{Completion, Lang};

/// Rows shown.
const LIMIT: usize = 8;

/// A text widget with completion.
pub enum Target {
    Entry(gtk::Entry),
    Text(gtk::TextView),
}

pub struct Popup {
    target: Target,
    lang: Lang,
    pop: gtk::Popover,
    list: gtk::ListBox,
    items: RefCell<Vec<Completion>>,
    /// typed prefix (start, cursor) as char offsets in `line()`
    span: Cell<(usize, usize)>,
    /// set while code changes the text (nests)
    mute: Cell<u32>,
    /// text length in chars: typing grows it
    len: Cell<usize>,
    /// update after a change, once the cursor has moved too
    pending: RefCell<Option<glib::SourceId>>,
}

impl Popup {
    pub fn new(target: Target, lang: Lang) -> Popup {
        let list = gtk::ListBox::builder().can_focus(false).build();
        let pop = gtk::Popover::builder()
            .child(&list)
            .autohide(false)
            .has_arrow(false)
            .can_focus(false)
            .position(gtk::PositionType::Bottom)
            .halign(gtk::Align::Start)
            .css_classes(["completion"])
            .build();
        let p = Popup { target, lang, pop, list, items: RefCell::new(vec![]), span: Cell::new((0, 0)), mute: Cell::new(0), len: Cell::new(0), pending: RefCell::new(None) };
        p.pop.set_parent(p.widget());
        p
    }

    fn widget(&self) -> &gtk::Widget {
        match &self.target {
            Target::Entry(e) => e.upcast_ref(),
            Target::Text(v) => v.upcast_ref(),
        }
    }

    pub fn is_open(&self) -> bool {
        self.pop.is_visible()
    }

    pub fn hide(&self) {
        if let Some(id) = self.pending.take() {
            id.remove();
        }
        self.pop.popdown();
    }

    /// Run `f`, which changes the text (history recall, keypad, accepting), without popping up.
    pub fn quiet(&self, f: impl FnOnce()) {
        self.mute.set(self.mute.get() + 1);
        f();
        self.mute.set(self.mute.get() - 1);
        self.len.set(self.chars());
        self.hide();
    }

    fn chars(&self) -> usize {
        match &self.target {
            Target::Entry(e) => e.text_length() as usize,
            Target::Text(v) => v.buffer().char_count() as usize,
        }
    }

    /// The cursor's line (an entry's whole text) and the cursor's char offset in it.
    fn line(&self) -> (String, usize) {
        match &self.target {
            Target::Entry(e) => (e.text().to_string(), e.position().max(0) as usize),
            Target::Text(v) => {
                let b = v.buffer();
                let cur = b.iter_at_mark(&b.get_insert());
                let (mut s, mut e) = (cur, cur);
                s.set_line_offset(0);
                if !e.ends_line() {
                    e.forward_to_line_end();
                }
                (b.text(&s, &e, false).to_string(), cur.line_offset() as usize)
            }
        }
    }

    /// After a typed change: (re)open on the identifier at the cursor; deleting only updates.
    fn refresh(&self, vars: &[(String, String)]) {
        let n = self.chars();
        if n > self.len.replace(n) || self.is_open() {
            self.update(vars);
        }
    }

    fn update(&self, vars: &[(String, String)]) {
        let (line, cur) = self.line();
        let Some((start, items)) = crate::complete::complete(&line, cur, self.lang, vars, LIMIT).filter(|(_, m)| !m.is_empty()) else {
            return self.pop.popdown();
        };
        self.list.remove_all();
        for c in &items {
            self.list.append(&row(c));
        }
        self.list.select_row(self.list.row_at_index(0).as_ref());
        *self.items.borrow_mut() = items;
        self.span.set((start, cur));
        self.pop.set_pointing_to(Some(&self.caret(start)));
        self.pop.popup();
    }

    /// Where the popup points: under char `at` of the line, in target coordinates.
    fn caret(&self, at: usize) -> gdk::Rectangle {
        match &self.target {
            Target::Entry(e) => {
                let x = e.delegate().and_downcast::<gtk::Text>().and_then(|t| {
                    let (r, _) = t.compute_cursor_extents(at);
                    t.compute_point(e, &graphene::Point::new(r.x(), r.y())).map(|p| p.x() as i32)
                });
                gdk::Rectangle::new(x.unwrap_or(0), 0, 1, e.height())
            }
            Target::Text(v) => {
                let b = v.buffer();
                let mut it = b.iter_at_mark(&b.get_insert());
                it.set_line_offset(at as i32);
                let r = v.iter_location(&it);
                let (x, y) = v.buffer_to_window_coords(gtk::TextWindowType::Widget, r.x(), r.y());
                gdk::Rectangle::new(x, y, 1, r.height())
            }
        }
    }

    fn step(&self, d: i32) {
        let n = self.items.borrow().len().max(1) as i32;
        let i = self.list.selected_row().map_or(0, |r| r.index());
        self.list.select_row(self.list.row_at_index((i + d).rem_euclid(n)).as_ref());
    }

    /// Replace the typed prefix with match `i`.
    fn accept(&self, i: usize) {
        let Some(c) = self.items.borrow().get(i).cloned() else { return };
        let (start, cur) = self.span.get();
        if self.line().1 != cur {
            return self.hide(); // the cursor moved since: the prefix isn't there any more
        }
        let math = self.lang == Lang::Math;
        let (text, slot) = insertion(&c, math, self.line().0.chars().nth(cur) == Some('('));
        self.quiet(|| match &self.target {
            Target::Entry(e) => {
                let (a, mut p) = (start as i32, start as i32);
                e.delete_text(a, cur as i32);
                e.insert_text(&text, &mut p);
                match slot {
                    Some(k) if math => e.select_region(a + k as i32, a + k as i32 + 1),
                    Some(k) => e.set_position(a + k as i32),
                    None => e.set_position(p),
                }
            }
            Target::Text(v) => {
                let b = v.buffer();
                let line = b.iter_at_mark(&b.get_insert()).line();
                let at = |k: usize| b.iter_at_line_offset(line, k as i32);
                let (Some(mut s), Some(mut e)) = (at(start), at(cur)) else { return };
                b.begin_user_action();
                b.delete(&mut s, &mut e);
                b.insert(&mut s, &text);
                b.end_user_action();
                b.place_cursor(&slot.and_then(|k| at(start + k)).unwrap_or(s));
                v.scroll_mark_onscreen(&b.get_insert());
            }
        });
    }
}

/// Text that replaces the prefix, and the char offset of its first □: kept in the calculator (it
/// gets selected), dropped from Octave code (the caret goes there). Before a "(" just the name.
fn insertion(c: &Completion, keep_slots: bool, before_paren: bool) -> (String, Option<usize>) {
    if before_paren && c.insert.starts_with(&format!("{}(", c.name)) {
        return (c.name.clone(), None);
    }
    let slot = c.insert.chars().position(|ch| ch == SLOT);
    (if keep_slots { c.insert.clone() } else { c.insert.replace(SLOT, "") }, slot)
}

#[derive(Debug, PartialEq)]
enum Key {
    Step(i32),
    Accept,
    Close,
    /// close and let the widget have the key (cursor moves, shortcuts)
    Leave,
    Pass,
}

/// What a key press does while the popup is open.
fn key(k: gdk::Key, state: gdk::ModifierType) -> Key {
    use gdk::Key as K;
    if state.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK) {
        return Key::Leave;
    }
    match k {
        K::Up | K::KP_Up => Key::Step(-1),
        K::Down | K::KP_Down => Key::Step(1),
        K::Tab | K::KP_Tab | K::Return | K::KP_Enter | K::ISO_Enter => Key::Accept,
        K::Escape => Key::Close,
        K::Left | K::Right | K::Home | K::End | K::Page_Up | K::Page_Down | K::KP_Left | K::KP_Right | K::KP_Home | K::KP_End => Key::Leave,
        _ => Key::Pass,
    }
}

fn row(c: &Completion) -> gtk::ListBoxRow {
    let label = |text: &str, classes: &[&str]| {
        let l = gtk::Label::builder().label(text).xalign(0.0).ellipsize(pango::EllipsizeMode::End).max_width_chars(60).build();
        classes.iter().for_each(|c| l.add_css_class(c));
        l
    };
    let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
    b.append(&label(&c.signature, &["monospace"]));
    if !c.desc.is_empty() {
        b.append(&label(&c.desc, &["desc", "dim-label"]));
    }
    gtk::ListBoxRow::builder().child(&b).focusable(false).build()
}

/// Workspace variables (with their summary) and user functions.
fn vars(ui: &Ui) -> Vec<(String, String)> {
    let Ok(ip) = ui.interp.try_borrow() else { return vec![] };
    let funcs = ip.func_names().into_iter().map(|n| (n, "function".to_string()));
    ip.vars.iter().map(|(n, v)| (n.clone(), v.summary())).chain(funcs).collect()
}

/// Hook `pick(ui)`'s popup to its widget: typing, keys, clicks, focus.
pub fn attach(ui: &Rc<Ui>, pick: fn(&Ui) -> &Popup) {
    let p = pick(ui);
    let u = ui.clone();
    let changed = move || {
        let p = pick(&u);
        if p.mute.get() > 0 {
            return;
        }
        // the entry moves its cursor after "changed": look once it has
        if p.pending.borrow().is_none() {
            let u = u.clone();
            let id = glib::idle_add_local_once(move || {
                let p = pick(&u);
                p.pending.take();
                p.refresh(&vars(&u));
            });
            *p.pending.borrow_mut() = Some(id);
        }
    };
    match &p.target {
        Target::Entry(e) => {
            e.connect_changed(move |_| changed());
        }
        Target::Text(v) => {
            v.buffer().connect_changed(move |_| changed());
        }
    }

    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let u = ui.clone();
    keys.connect_key_pressed(move |_, k, _, state| {
        let p = pick(&u);
        if !p.is_open() {
            return glib::Propagation::Proceed;
        }
        match key(k, state) {
            Key::Step(d) => p.step(d),
            Key::Accept => p.accept(p.list.selected_row().map_or(0, |r| r.index().max(0) as usize)),
            Key::Close => p.hide(),
            Key::Leave => {
                p.hide();
                return glib::Propagation::Proceed;
            }
            Key::Pass => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    p.widget().add_controller(keys);

    let u = ui.clone();
    p.list.connect_row_activated(move |_, row| pick(&u).accept(row.index().max(0) as usize));
    let click = gtk::GestureClick::new();
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let u = ui.clone();
    click.connect_pressed(move |g, _, _, _| {
        // the popup is the target's child, so clicks on its rows come through here too
        let p = pick(&u);
        if g.current_event().and_then(|e| e.surface()) == p.widget().native().and_then(|n| n.surface()) {
            p.hide();
        }
    });
    p.widget().add_controller(click);
    let focus = gtk::EventControllerFocus::new();
    let u = ui.clone();
    focus.connect_leave(move |_| pick(&u).hide());
    p.widget().add_controller(focus);
    let pop = p.pop.clone();
    p.widget().connect_destroy(move |_| pop.unparent());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::complete::Kind;

    #[test]
    fn insert_text() {
        let c = |name: &str, insert: &str| Completion { name: name.into(), insert: insert.into(), signature: String::new(), desc: String::new(), kind: Kind::Function };
        let integ = c("integrate", "integrate(□, □, □)");
        assert_eq!(insertion(&integ, true, false), ("integrate(□, □, □)".into(), Some(10)));
        assert_eq!(insertion(&integ, false, false), ("integrate(, , )".into(), Some(10)));
        assert_eq!(insertion(&integ, true, true), ("integrate".into(), None)); // "integ|(x)"
        assert_eq!(insertion(&c("√", "√(□)"), true, false), ("√(□)".into(), Some(2))); // chars, not bytes
        assert_eq!(insertion(&c("pi", "pi"), true, true), ("pi".into(), None));
        assert_eq!(insertion(&c("tic", "tic()"), false, false), ("tic()".into(), None));
    }

    #[test]
    fn keys() {
        use gdk::ModifierType as M;
        assert_eq!(key(gdk::Key::Down, M::empty()), Key::Step(1));
        assert_eq!(key(gdk::Key::Up, M::empty()), Key::Step(-1));
        assert_eq!(key(gdk::Key::Tab, M::empty()), Key::Accept);
        assert_eq!(key(gdk::Key::Return, M::empty()), Key::Accept);
        assert_eq!(key(gdk::Key::KP_Enter, M::empty()), Key::Accept);
        assert_eq!(key(gdk::Key::Escape, M::empty()), Key::Close);
        assert_eq!(key(gdk::Key::Left, M::empty()), Key::Leave);
        assert_eq!(key(gdk::Key::Return, M::CONTROL_MASK), Key::Leave); // Ctrl+Return runs
        assert_eq!(key(gdk::Key::a, M::empty()), Key::Pass);
        assert_eq!(key(gdk::Key::BackSpace, M::empty()), Key::Pass);
        assert_eq!(key(gdk::Key::ISO_Left_Tab, M::SHIFT_MASK), Key::Pass);
    }
}
