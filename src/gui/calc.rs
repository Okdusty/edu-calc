//! Calculator mode: math entry (libqalculate syntax) with a live result, a history of answer
//! cards, history.txt, and (Programming keypad) the result in hex/dec/oct/bin. Shares the
//! interpreter and plots with Octave mode through `qalc::eval_into`.
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{gdk, glib, pango};

use super::Ui;
use super::keypad::{self, SLOT};
use crate::qalc::{self, Answer};

/// Answers kept in the list and in history.txt.
const KEEP: usize = 500;

/// One history.txt line: typed input, math view, result, approximation ("" = none).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Saved {
    pub expr: String,
    pub input: String,
    pub result: String,
    pub approx: String,
}

pub struct Calc {
    /// whole view; an invisible overlay child reports its size (keypad_side=auto)
    pub root: gtk::Overlay,
    pub entry: gtk::Entry,
    /// [history, entry, preview] then the keypad: vertical = keypad below, horizontal = beside
    pub body: gtk::Box,
    preview: gtk::Label,
    /// HEX/DEC/OCT/BIN of the preview or last answer, with the Programming keypad
    bases: gtk::Label,
    /// last answer, for the bases readout
    last: RefCell<Option<Answer>>,
    list: gtk::ListBox,
    scroll: gtk::ScrolledWindow,
    /// typed input per list row ("" for notices), for tap-to-reuse
    exprs: RefCell<Vec<String>>,
    saved: RefCell<Vec<Saved>>,
    pending: RefCell<Option<glib::SourceId>>,
    /// keep the newest card in view
    stick: Cell<bool>,
    /// room for the keypad on the right; too narrow for the number pad beside the mode's keys
    pub wide: Cell<bool>,
    pub narrow: Cell<bool>,
}

impl Calc {
    pub fn new() -> Self {
        let entry = gtk::Entry::builder().placeholder_text("5 km to mi    integrate(x^2, 0, 3)    solve(x^2 = 4)").hexpand(true).build();
        entry.add_css_class("calc-entry");
        let preview = gtk::Label::builder().xalign(0.0).hexpand(true).ellipsize(pango::EllipsizeMode::End).selectable(true).build();
        preview.add_css_class("calc-preview");
        preview.add_css_class("dim-label");
        let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).valign(gtk::Align::End).build();
        list.add_css_class("calc-history");
        let scroll = gtk::ScrolledWindow::builder().child(&list).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();
        let bases = gtk::Label::builder().xalign(0.0).selectable(true).wrap(true).wrap_mode(pango::WrapMode::WordChar).visible(false).build();
        for c in ["calc-bases", "monospace", "dim-label"] {
            bases.add_css_class(c);
        }
        let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).hexpand(true).vexpand(true).build();
        column.append(&scroll);
        column.append(&entry);
        column.append(&preview);
        column.append(&bases);
        let body = gtk::Box::builder().spacing(8).margin_start(8).margin_end(8).margin_top(6).margin_bottom(8).build();
        body.append(&column);
        let root = gtk::Overlay::builder().child(&body).build();
        root.add_css_class("calc");
        Calc {
            root,
            entry,
            body,
            preview,
            bases,
            last: RefCell::new(None),
            list,
            scroll,
            exprs: RefCell::new(vec![]),
            saved: RefCell::new(vec![]),
            pending: RefCell::new(None),
            stick: Cell::new(true),
            wide: Cell::new(false),
            narrow: Cell::new(true), // stacked until measured: side by side could force the window wider
        }
    }
}

pub fn attach(ui: &Rc<Ui>) {
    let c = &ui.calc;
    let u = ui.clone();
    c.entry.connect_activate(move |_| run(&u));

    let u = ui.clone();
    c.entry.connect_changed(move |e| {
        e.set_attributes(&pango::AttrList::new());
        e.remove_css_class("error");
        refresh(&u);
    });

    // Tab / Shift+Tab: next / previous □ (Tab accepts a completion while the popup is open)
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let (u, e) = (ui.clone(), c.entry.clone());
    keys.connect_key_pressed(move |_, key, _, state| {
        if !matches!(key, gdk::Key::Tab | gdk::Key::ISO_Left_Tab | gdk::Key::KP_Tab) || u.calc_pop.is_open() {
            return glib::Propagation::Proceed;
        }
        let pos = e.position();
        let (a, b) = e.selection_bounds().unwrap_or((pos, pos));
        let back = key == gdk::Key::ISO_Left_Tab || state.contains(gdk::ModifierType::SHIFT_MASK);
        match keypad::next_slot(&e.text(), (a as usize, b as usize), back) {
            Some(i) => {
                e.select_region(i as i32, i as i32 + 1);
                glib::Propagation::Stop
            }
            None => glib::Propagation::Proceed,
        }
    });
    c.entry.add_controller(keys);

    // tapping a card puts its input back
    let u = ui.clone();
    c.list.connect_row_activated(move |_, row| {
        let expr = usize::try_from(row.index()).ok().and_then(|i| u.calc.exprs.borrow().get(i).cloned()).unwrap_or_default();
        if !expr.is_empty() {
            let e = &u.calc.entry;
            u.calc_pop.quiet(|| e.set_text(&expr));
            e.grab_focus_without_selecting();
            e.set_position(-1);
        }
    });

    // stay at the newest card unless the user scrolled up
    let adj = c.scroll.vadjustment();
    let u = ui.clone();
    adj.connect_value_changed(move |a| u.calc.stick.set(a.value() >= a.upper() - a.page_size() - 1.0));
    let u = ui.clone();
    adj.connect_changed(move |a| {
        if u.calc.stick.get() {
            // emitted during the viewport's allocation: scroll after it, or the move is lost
            let a = a.clone();
            glib::idle_add_local_once(move || a.set_value(a.upper() - a.page_size()));
        }
    });

    let sensor = gtk::DrawingArea::builder().can_target(false).build();
    c.root.add_overlay(&sensor);
    let u = ui.clone();
    sensor.connect_resize(move |_, w, h| {
        // ponytail: "wide" = landscape with ~450 px left for the history; keypad_side forces a side.
        // "narrow": no room for the number pad beside the mode's keys (16 = body margins)
        let (wide, narrow) = (w >= 1000 && w > h, w - 16 < u.keypad.side_by_side_width());
        // both replaced: a short-circuiting || would leave `narrow` stale whenever `wide` changes
        let changed = (u.calc.wide.replace(wide) != wide) | (u.calc.narrow.replace(narrow) != narrow);
        if changed {
            let u = u.clone();
            glib::idle_add_local_once(move || super::apply_layout(&u)); // not during allocation
        }
    });

    let saved = parse_history(&std::fs::read_to_string(history_path()).unwrap_or_default());
    for s in &saved {
        push(ui, &s.expr, &card(s).0);
    }
    *c.saved.borrow_mut() = saved;
}

fn history_path() -> PathBuf {
    crate::config_dir().join("history.txt")
}

/// Live result of the entry, debounced (after typing or an option change); never for plots
/// (they'd draw) or unfilled slots.
pub fn refresh(ui: &Rc<Ui>) {
    let c = &ui.calc;
    if let Some(id) = c.pending.take() {
        id.remove();
    }
    let text = c.entry.text().to_string();
    if text.trim().is_empty() || text.contains("plot(") || text.contains(SLOT) {
        c.preview.set_text("");
        return show_bases(ui, None);
    }
    let u = ui.clone();
    // ponytail: preview runs on the UI thread; fine while qalc::preview's timeout is short
    let id = glib::timeout_add_local_once(Duration::from_millis(250), move || {
        u.calc.pending.take();
        // a half-typed name ("integ") previews as nonsense (int(e·g)) while the popup offers names
        let p = if u.calc_pop.is_open() { None } else { qalc::preview(&text) };
        u.calc.preview.set_text(&p.as_ref().map(|p| format!("= {p}")).unwrap_or_default());
        show_bases(&u, Some(p.as_deref().unwrap_or("")));
    });
    *c.pending.borrow_mut() = Some(id);
}

/// The bases readout (Programming keypad only): of the live result `preview` when it's an integer,
/// else of the last answer when the entry is empty (`preview` None).
pub fn show_bases(ui: &Ui, preview: Option<&str>) {
    let c = &ui.calc;
    let answer = match preview {
        _ if ui.cfg.borrow().keypad_mode != "programming" => None,
        Some(p) => preview_int(p, qalc::options().base).map(|n| Answer { result: n.to_string(), value: Some(crate::value::Value::num(n as f64)), ..Default::default() }),
        None => c.last.borrow().clone(),
    };
    let text = answer.as_ref().and_then(qalc::bases).map(|b| b.iter().map(|(name, v)| format!("{name} {v}")).collect::<Vec<_>>().join("   "));
    c.bases.set_text(text.as_deref().unwrap_or(""));
    c.bases.set_visible(text.is_some());
}

/// The integer a live result shows ("255", "0xFF", "−12", "1111 1111" in base 2), if it is one.
// ponytail: parses the preview text; a pure qalc::preview_answer would hand over the Answer itself
fn preview_int(p: &str, base: u32) -> Option<i128> {
    if p.contains('≈') {
        return None;
    }
    let s: String = p.chars().filter(|c| !c.is_whitespace()).collect();
    let (neg, s) = match s.strip_prefix(['-', '−']) {
        Some(r) => (true, r),
        None => (false, s.as_str()),
    };
    let (radix, digits) = match s.get(..2).map(str::to_ascii_lowercase).as_deref() {
        Some("0x") => (16, &s[2..]),
        Some("0b") => (2, &s[2..]),
        Some("0o") => (8, &s[2..]),
        _ => (base, s),
    };
    let n = i128::from_str_radix(digits, radix.clamp(2, 36)).ok()?;
    Some(if neg { -n } else { n })
}

/// Evaluate the entry; leftover □ are highlighted instead.
pub fn run(ui: &Rc<Ui>) {
    let c = &ui.calc;
    let text = c.entry.text().to_string();
    let expr = text.trim();
    if expr.is_empty() {
        return;
    }
    if text.contains(SLOT) {
        let attrs = pango::AttrList::new();
        for (b, _) in text.char_indices().filter(|(_, ch)| *ch == SLOT) {
            let mut a = pango::AttrColor::new_background(0xe5e5, 0x5353, 0x4b4b);
            a.set_start_index(b as u32);
            a.set_end_index((b + SLOT.len_utf8()) as u32);
            attrs.insert(a);
        }
        c.entry.set_attributes(&attrs);
        c.entry.add_css_class("error");
        let i = keypad::slots(&text)[0] as i32;
        c.entry.select_region(i, i + 1);
        return;
    }
    let r = eval(ui, expr);
    let msg = answer(ui, expr, &r);
    super::sync_options(ui); // `set angle deg` and friends
    if let Ok(a) = &r {
        let auto = ui.cfg.borrow().auto_visual;
        if auto && let Some(v) = &a.visual {
            if let Err(e) = super::eval_octave(ui, v) {
                show(&msg, &e);
            }
        }
        let s = Saved { expr: expr.into(), input: a.input.clone(), result: a.result.clone(), approx: a.approx.clone().unwrap_or_default() };
        let text = {
            let mut saved = c.saved.borrow_mut();
            saved.push(s);
            let extra = saved.len().saturating_sub(KEEP);
            saved.drain(..extra);
            history_text(&saved)
        };
        // ponytail: whole file per answer (500 short lines); two windows: last one wins
        if let Err(e) = std::fs::write(history_path(), text) {
            eprintln!("history.txt: {e}");
        }
    }
    *c.last.borrow_mut() = r.ok();
    ui.calc_pop.quiet(|| c.entry.set_text(""));
}

fn eval(ui: &Ui, expr: &str) -> Result<Answer, String> {
    let r = {
        let mut ip = ui.interp.borrow_mut();
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| qalc::eval_into(&mut ip, expr)))
    };
    super::sync(ui);
    r.unwrap_or_else(|_| Err("internal error (panic), see stderr".into()))
}

/// Error-styled card for messages (bad config, CSS errors) while the console is hidden.
pub fn notice(ui: &Ui, text: &str) {
    let b = card_box();
    b.append(&label(text.trim_end(), &["calc-message", "error"]));
    push(ui, "", &b);
}

fn card_box() -> gtk::Box {
    gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).css_classes(["calc-card"]).build()
}

fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::builder().label(text).xalign(0.0).wrap(true).wrap_mode(pango::WrapMode::WordChar).build();
    classes.iter().for_each(|c| l.add_css_class(c));
    l
}

fn show(l: &gtk::Label, text: &str) {
    l.set_text(text);
    l.set_visible(!text.is_empty());
}

/// Card body: input (math view), result (selectable), approximation. Returns the result label.
fn card(s: &Saved) -> (gtk::Box, gtk::Label) {
    let b = card_box();
    b.append(&label(&s.input, &["calc-input", "dim-label"]));
    let result = label(&s.result, &["calc-result"]);
    result.set_selectable(true);
    b.append(&result);
    if !s.approx.is_empty() {
        b.append(&label(&format!("≈ {}", s.approx), &["calc-approx", "dim-label"]));
    }
    (b, result)
}

/// Append a list row (capped at KEEP) and scroll to it.
fn push(ui: &Ui, expr: &str, w: &gtk::Box) {
    let c = &ui.calc;
    c.list.append(w);
    let mut exprs = c.exprs.borrow_mut();
    exprs.push(expr.to_string());
    if exprs.len() > KEEP {
        exprs.remove(0);
        if let Some(r) = c.list.row_at_index(0) {
            c.list.remove(&r);
        }
    }
    c.stick.set(true);
}

/// Card for a fresh answer: alternative forms, Octave code, Visualize, messages. Returns the
/// message label, for errors that come later.
fn answer(ui: &Rc<Ui>, expr: &str, r: &Result<Answer, String>) -> gtk::Label {
    let msg = label("", &["calc-message"]);
    let a = match r {
        Ok(a) => a,
        Err(e) => {
            let b = card_box();
            b.append(&label(expr, &["calc-input", "dim-label"]));
            b.append(&label(e, &["calc-result", "error"]));
            push(ui, expr, &b);
            return msg;
        }
    };
    let (b, result) = card(&Saved { input: a.input.clone(), result: a.result.clone(), approx: a.approx.clone().unwrap_or_default(), ..Default::default() });
    show(&msg, &a.messages.join("\n"));
    b.append(&msg);
    let chips = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).max_children_per_line(32).row_spacing(4).column_spacing(4).halign(gtk::Align::Start).build();
    // forms (fraction, hex, units ...): clicking one shows it in place of the result
    if !a.alts.is_empty() {
        let mut first: Option<gtk::ToggleButton> = None;
        for (name, text) in std::iter::once(("result".to_string(), a.result.clone())).chain(a.alts.iter().cloned()) {
            let t = gtk::ToggleButton::builder().label(name.as_str()).tooltip_text(text.as_str()).active(first.is_none()).build();
            t.add_css_class("calc-chip");
            t.set_group(first.as_ref());
            let r = result.clone();
            t.connect_toggled(move |t| {
                if t.is_active() {
                    r.set_text(&text);
                }
            });
            chips.append(&t);
            first.get_or_insert(t);
        }
    }
    if let Some(code) = &a.octave {
        chips.append(&octave_chip(ui, code));
    }
    // a plot(...) answer is its own visualization
    if a.visual.is_some() || !(a.free_vars.is_empty() || expr.starts_with("plot")) {
        let v = gtk::Button::builder().label("Visualize").tooltip_text(a.visual.as_deref().unwrap_or("plot it")).build();
        v.add_css_class("calc-chip");
        let (u, m, visual, expr) = (ui.clone(), msg.clone(), a.visual.clone(), expr.to_string());
        v.connect_clicked(move |_| {
            let r = match &visual {
                Some(code) => super::eval_octave(&u, code),
                None => eval(&u, &format!("plot({expr})")).map(|_| ()),
            };
            show(&m, &r.err().unwrap_or_default());
        });
        chips.append(&v);
    }
    if chips.first_child().is_some() {
        b.append(&chips);
    }
    push(ui, expr, &b);
    msg
}

/// "Octave: code" chip; its popover copies the code or sends it to Octave mode.
fn octave_chip(ui: &Rc<Ui>, code: &str) -> gtk::MenuButton {
    let text = gtk::Label::builder().label(code).selectable(true).wrap(true).wrap_mode(pango::WrapMode::WordChar).max_width_chars(48).xalign(0.0).build();
    text.add_css_class("monospace");
    let copy = gtk::Button::with_label("Copy");
    let send = gtk::Button::with_label("Send to Octave");
    send.add_css_class("suggested-action");
    let row = gtk::Box::builder().spacing(6).homogeneous(true).build();
    row.append(&copy);
    row.append(&send);
    let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build();
    col.append(&text);
    col.append(&row);
    let pop = gtk::Popover::builder().child(&col).build();
    let face = gtk::Label::builder().label(format!("Octave: {code}")).ellipsize(pango::EllipsizeMode::End).max_width_chars(28).build();
    let chip = gtk::MenuButton::builder().child(&face).popover(&pop).tooltip_text(code).build();
    chip.add_css_class("calc-chip");
    let (c, p) = (code.to_string(), pop.clone());
    copy.connect_clicked(move |b| {
        b.clipboard().set_text(&c);
        p.popdown();
    });
    let (u, c, p) = (ui.clone(), code.to_string(), pop);
    send.connect_clicked(move |_| {
        p.popdown();
        super::send_to_octave(&u, &c);
    });
    chip
}

/// history.txt: one answer per line, tab-separated fields of `Saved`.
fn history_text(saved: &[Saved]) -> String {
    let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    saved.iter().map(|s| format!("{}\t{}\t{}\t{}\n", clean(&s.expr), clean(&s.input), clean(&s.result), clean(&s.approx))).collect()
}

/// Last KEEP well-formed lines (at least input, math view, result).
fn parse_history(text: &str) -> Vec<Saved> {
    let all: Vec<Saved> = text
        .lines()
        .filter_map(|l| {
            let mut f = l.splitn(4, '\t');
            let s = Saved { expr: f.next()?.into(), input: f.next()?.into(), result: f.next()?.into(), approx: f.next().unwrap_or("").into() };
            (!s.expr.trim().is_empty()).then_some(s)
        })
        .collect();
    all[all.len().saturating_sub(KEEP)..].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_file() {
        let s = |e: &str, r: &str, a: &str| Saved { expr: e.into(), input: e.into(), result: r.into(), approx: a.into() };
        let h = vec![s("1/3", "1/3", "0.3333333333"), s("x\ty", "a\nb", "")];
        let text = history_text(&h);
        assert_eq!(text, "1/3\t1/3\t1/3\t0.3333333333\nx y\tx y\ta b\t\n");
        let back = parse_history(&text);
        assert_eq!(back[0], h[0]);
        assert_eq!(back[1].result, "a b");
        assert_eq!(parse_history("garbage\n\n\tx\ty\n2+2\t2 + 2\t4\n"), [Saved { expr: "2+2".into(), input: "2 + 2".into(), result: "4".into(), approx: "".into() }]);
        let many: String = (0..KEEP + 7).map(|i| format!("{i}\t{i}\t{i}\t\n")).collect();
        let p = parse_history(&many);
        assert_eq!((p.len(), p[0].expr.as_str()), (KEEP, "7"));
    }

    #[test]
    fn preview_integers() {
        assert_eq!(preview_int("255", 10), Some(255));
        assert_eq!(preview_int("0xFF", 16), Some(255));
        assert_eq!(preview_int("FF", 16), Some(255));
        assert_eq!(preview_int("0b1111 1111", 2), Some(255));
        assert_eq!(preview_int("−12", 10), Some(-12));
        assert_eq!(preview_int("1 000 000", 10), Some(1_000_000));
        for no in ["1/3 ≈ 0.3333333333", "2.5", "x + 1", "5 km", "", "-"] {
            assert_eq!(preview_int(no, 10), None, "{no}");
        }
    }
}
