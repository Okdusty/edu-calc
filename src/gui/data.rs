//! Data editor: a window with a spreadsheet grid. Row 0 holds the column names (echoed in the
//! column titles, which stay in view), the rows below the numbers: blank = NaN, text is flagged
//! and becomes NaN, a decimal comma is fine. Enter / Down, Shift+Enter / Up, Tab, Shift+Tab move
//! between cells; Enter on the last row adds one. Ctrl+V (or Paste) fills a TSV/CSV block from a
//! spreadsheet or text at the current cell; a header line pasted at the top goes to the names row.
//! Import reads csv/tsv/txt/xlsx/xls/ods like readmatrix (a sheet picker for workbooks), Export
//! writes csv/tsv. To Workspace assigns the matrix plus every column whose name is a valid
//! identifier, running/printing the equivalent Octave (`data = readmatrix(...)` for an unedited
//! import) so scripts can repeat it. Plot plots column 1 against the rest through Octave. Σ shows
//! n, mean, median, std, min, max per column. CSS: window.data-editor.
//! ponytail: rows are virtualized (thousands are fine), columns aren't (hundreds get slow); every
//! edit re-reads the grid for the status line and stats, fine for that size.
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};

use super::Ui;
use crate::builtins::io::{self, Grid};
use crate::value::{Matrix, Value};

pub(super) struct Ed {
    ui: Rc<Ui>,
    win: gtk::Window,
    pub root: gtk::Box,
    embedded: bool,
    changed: RefCell<Option<Box<dyn Fn()>>>,
    active: RefCell<Option<(gtk::Entry, gtk::ListItem, usize)>>,
    keypad_shown: Cell<bool>,
    cv: gtk::ColumnView,
    /// one glib::BoxedAnyObject(Vec<String>) per row, each `cols` long; row 0 = column names
    store: gio::ListStore,
    cols: Cell<usize>,
    /// last focused cell: (row, column)
    cur: Cell<(u32, usize)>,
    name: gtk::Entry,
    sheets: gtk::DropDown,
    stats: gtk::Label,
    stats_box: gtk::ScrolledWindow,
    status: gtk::Label,
    /// last imported file and sheet; `pristine` while the grid is still exactly that
    source: RefCell<Option<(PathBuf, Option<String>)>>,
    pristine: Cell<bool>,
    /// edits that aren't in the workspace or a file yet
    dirty: Cell<bool>,
    /// set while code fills widgets (sheet list, cells being bound), so their signals are ignored
    busy: Cell<bool>,
}

impl Ed {
    pub fn grid(&self) -> Grid {
        (0..self.store.n_items())
            .filter_map(|i| self.store.item(i).and_downcast::<glib::BoxedAnyObject>())
            .map(|o| o.borrow::<Vec<String>>().clone())
            .collect()
    }

    pub fn connect_changed(&self, f: impl Fn() + 'static) {
        *self.changed.borrow_mut() = Some(Box::new(f));
    }

    pub fn is_dirty(&self) -> bool { self.dirty.get() }
    pub fn discard(&self) { self.dirty.set(false); }

    pub fn focus(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || if let Some(ed) = weak.upgrade().filter(|ed| ed.root.is_mapped()) {
            let active = ed.active.borrow().clone();
            if let Some((entry, _, _)) = active.filter(|(_, item, col)| item.position() != gtk::INVALID_LIST_POSITION && ed.cur.get() == (item.position(), *col)) {
                entry.grab_focus_without_selecting();
            } else {
                let (r, c) = ed.cur.get();
                go(&ed, r as i64, c as i64);
            }
        });
    }

    /// Simple -> Advanced: unsent table edits become the workspace's `data` (plus X, Y... per named column).
    pub fn push(self: &Rc<Self>) {
        if self.dirty.get() {
            to_workspace(self);
        }
    }

    /// Advanced -> Simple: a `data` that Octave changed meanwhile fills the table; unsent edits win.
    pub fn pull(self: &Rc<Self>) {
        let m = match self.ui.interp.borrow().vars.get(&self.var()) {
            Some(Value::Num(m) | Value::Bool(m)) => m.clone(),
            _ => return,
        };
        let cur = self.table().1;
        let same = (cur.rows, cur.cols) == (m.rows, m.cols) && cur.data.iter().zip(&m.data).all(|(a, b)| a == b || a.is_nan() && b.is_nan());
        if self.dirty.get() || same {
            return;
        }
        set_grid(self, with_names(self.grid().swap_remove(0), &m), (1, 0));
        refresh(self);
    }

    pub fn set_keypad_shown(&self, shown: bool) {
        self.keypad_shown.set(shown);
        if let Some((entry, item, _)) = self.active.borrow().as_ref() {
            entry.set_input_hints(if shown && item.position() > 0 { gtk::InputHints::INHIBIT_OSK } else { gtk::InputHints::NONE });
        }
    }

    pub fn press(self: &Rc<Self>, act: &super::keypad::Act) {
        let Some((entry, item, col)) = self.active.borrow().clone() else {
            let (r, c) = self.cur.get();
            go(self, r as i64, c as i64);
            let weak = Rc::downgrade(self);
            let act = act.clone();
            glib::idle_add_local_once(move || if let Some(ed) = weak.upgrade() {
                if ed.active.borrow().is_some() { ed.press(&act); }
            });
            return;
        };
        // ColumnView recycles entries. A stale widget must never edit another row.
        if item.position() == gtk::INVALID_LIST_POSITION || self.cur.get() != (item.position(), col) {
            self.active.borrow_mut().take();
            self.press(act);
            return;
        }
        entry.grab_focus_without_selecting();
        if matches!(act, super::keypad::Act::Enter) {
            go(self, item.position() as i64 + 1, col as i64);
        } else {
            super::keypad::press(super::keypad::Target::Entry(&entry, false), act);
        }
    }

    /// Column names and data rows: trailing blank rows and columns dropped, decimal commas made points.
    fn cells(&self) -> (Vec<String>, Grid) {
        let mut g = self.grid();
        let names = if g.is_empty() { vec![] } else { g.remove(0) };
        let filled = |r: &Vec<String>| r.iter().rposition(|c| !c.trim().is_empty()).map_or(0, |i| i + 1);
        g.truncate(g.iter().rposition(|r| filled(r) > 0).map_or(0, |i| i + 1));
        let w = g.iter().map(filled).max().unwrap_or(0);
        for r in &mut g {
            r.truncate(w);
            for c in r.iter_mut() {
                *c = c.trim().to_string();
                if io::to_num(c, false).is_none() && io::to_num(c, true).is_some() {
                    *c = c.replace(',', ".");
                }
            }
        }
        (names.into_iter().take(w).map(|n| n.trim().to_string()).collect(), g)
    }

    fn table(&self) -> (Vec<String>, Matrix) {
        let (names, g) = self.cells();
        (names, io::to_matrix(&g, f64::NAN))
    }

    /// The name entry, `data` when blank.
    fn var(&self) -> String {
        Some(self.name.text().trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| "data".into())
    }

    fn say(&self, msg: &str, error: bool) {
        self.status.set_text(msg);
        set_class(&self.status, "error", error);
        set_class(&self.status, "dim-label", !error);
    }
}

/// Open the editor: on workspace variable `var` (a numeric matrix), else an empty sheet.
pub fn open(ui: &Rc<Ui>, var: Option<&str>) {
    create(ui, var, false);
}

pub(super) fn embedded(ui: &Rc<Ui>) -> Rc<Ed> {
    create(ui, None, true).expect("empty data editor")
}

fn create(ui: &Rc<Ui>, var: Option<&str>, embedded: bool) -> Option<Rc<Ed>> {
    let start: Grid = match var {
        None if embedded => std::iter::once(vec!["X".into(), "Y".into()]).chain(vec![vec![String::new(); 2]; 10]).collect(),
        None => vec![vec![String::new(); 3]; 11],
        Some(n) => {
            let m = match ui.interp.borrow().vars.get(n) {
                Some(Value::Num(m) | Value::Bool(m)) => m.clone(),
                Some(v) => { ui.print(&format!("Data Editor: {n} is {}, it edits real matrices\n", v.summary()), true); return None; }
                None => { ui.print(&format!("Data Editor: no variable '{n}'\n"), true); return None; }
            };
            with_names(vec![], &m)
        }
    };

    let store = gio::ListStore::new::<glib::BoxedAnyObject>();
    let cv = gtk::ColumnView::builder()
        .model(&gtk::NoSelection::new(Some(store.clone())))
        .show_row_separators(true)
        .show_column_separators(true)
        .reorderable(false)
        .build();
    let btn = |icon: &str, tip: &str| gtk::Button::builder().icon_name(icon).tooltip_text(tip).build();
    let import_b = btn("document-open-symbolic", "Import csv, tsv, txt, xlsx, xls, ods…");
    let export_b = btn("document-save-as-symbolic", "Export as .csv or .tsv…");
    let paste_b = btn("edit-paste-symbolic", "Paste cells from a spreadsheet or text at the current cell (Ctrl+V)");
    let sheets = gtk::DropDown::builder().tooltip_text("Sheet").visible(false).build();
    let menu = gio::Menu::new();
    for (label, act) in [("Insert Row Below", "ed.add-row"), ("Delete Row", "ed.del-row"), ("Insert Column Right", "ed.add-col"), ("Delete Column", "ed.del-col")] {
        menu.append(Some(label), Some(act));
    }
    let more = gtk::MenuButton::builder().icon_name("view-more-symbolic").menu_model(&menu).tooltip_text("Rows and columns").build();
    let stats_b = gtk::ToggleButton::builder().label("Σ").tooltip_text("Column statistics: n, mean, median, std, min, max").build();
    let name = gtk::Entry::builder().text(var.unwrap_or("data")).width_chars(8).tooltip_text("Workspace variable").build();
    let to_ws = gtk::Button::builder().label("To Workspace").tooltip_text("Assign the matrix, and each column with a valid name as its own variable").build();
    let send = gtk::Box::builder().css_classes(["linked"]).build();
    send.append(&name);
    send.append(&to_ws);
    let plot_b = gtk::Button::builder().label("Plot").tooltip_text("Plot columns 2… against column 1 (a single column against its index)").build();
    let header = gtk::HeaderBar::new();
    for w in [import_b.upcast_ref::<gtk::Widget>(), export_b.upcast_ref(), paste_b.upcast_ref(), more.upcast_ref(), sheets.upcast_ref()] {
        header.pack_start(w);
    }
    header.pack_end(&plot_b);
    header.pack_end(&send);
    header.pack_end(&stats_b);

    let stats = gtk::Label::builder().xalign(0.0).yalign(0.0).selectable(true).css_classes(["monospace"]).margin_start(8).margin_end(8).margin_top(4).build();
    let stats_box = gtk::ScrolledWindow::builder().child(&stats).vscrollbar_policy(gtk::PolicyType::Never).visible(false).build();
    let status = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).margin_start(8).margin_end(8).margin_top(2).margin_bottom(4).build();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    if embedded {
        // the standalone header's buttons as a slim toolbar; rows and columns stay in the ⋮ menu
        let tools = gtk::Box::builder().spacing(2).css_classes(["toolbar"]).build();
        for w in [import_b.upcast_ref::<gtk::Widget>(), export_b.upcast_ref(), paste_b.upcast_ref(), more.upcast_ref(), sheets.upcast_ref()] {
            header.remove(w);
            w.set_focus_on_click(false);
            tools.append(w);
        }
        body.append(&tools);
    }
    body.append(&gtk::ScrolledWindow::builder().child(&cv).vexpand(true).build());
    body.append(&stats_box);
    body.append(&status);
    let win = if embedded { ui.win.clone().upcast() } else { gtk::Window::builder()
        .title("Data Editor")
        .transient_for(&ui.win)
        .destroy_with_parent(true)
        .default_width(900)
        .default_height(600)
        .titlebar(&header)
        .child(&body)
        .build() };
    if !embedded { win.add_css_class("data-editor"); }

    let ed = Rc::new(Ed {
        ui: ui.clone(),
        win,
        root: body,
        embedded,
        changed: RefCell::new(None),
        active: RefCell::new(None),
        keypad_shown: Cell::new(embedded),
        cv,
        store,
        cols: Cell::new(0),
        cur: Cell::new((1, 0)),
        name,
        sheets,
        stats,
        stats_box,
        status,
        source: RefCell::new(None),
        pristine: Cell::new(false),
        dirty: Cell::new(false),
        busy: Cell::new(false),
    });

    // every handler holds the editor weakly; the close handler keeps it alive with the window
    let on = |f: fn(&Rc<Ed>)| {
        let w = Rc::downgrade(&ed);
        move || {
            if let Some(ed) = w.upgrade() {
                f(&ed)
            }
        }
    };
    let f = on(import);
    import_b.connect_clicked(move |_| f());
    let f = on(export);
    export_b.connect_clicked(move |_| f());
    let f = on(paste);
    paste_b.connect_clicked(move |_| f());
    let f = on(|ed| drop(to_workspace(ed)));
    to_ws.connect_clicked(move |_| f());
    let f = on(|ed| drop(to_workspace(ed)));
    ed.name.connect_activate(move |_| f());
    let f = on(plot);
    plot_b.connect_clicked(move |_| f());
    let w = Rc::downgrade(&ed);
    stats_b.connect_toggled(move |b| {
        if let Some(ed) = w.upgrade() {
            ed.stats_box.set_visible(b.is_active());
            refresh(&ed);
        }
    });
    let w = Rc::downgrade(&ed);
    ed.sheets.connect_selected_notify(move |d| {
        let Some(ed) = w.upgrade() else { return };
        let Some((path, loaded)) = ed.source.borrow().clone() else { return };
        let sheet = d.selected_item().and_downcast::<gtk::StringObject>().map(|s| s.string().to_string());
        if !ed.busy.get() && sheet.is_some() && sheet != loaded {
            confirm(&ed, move |ed| load(ed, path, sheet));
        }
    });

    let group = gio::SimpleActionGroup::new();
    for (n, f) in [("add-row", add_row as fn(&Rc<Ed>)), ("del-row", del_row), ("add-col", add_col), ("del-col", del_col)] {
        let a = gio::SimpleAction::new(n, None);
        let f = on(f);
        a.connect_activate(move |_, _| f());
        group.add_action(&a);
    }
    ed.root.insert_action_group("ed", Some(&group));
    if !embedded { ed.win.insert_action_group("ed", Some(&group)); }

    // Ctrl+V anywhere in the window: a block fills cells, one value goes into the focused entry
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let f = on(paste);
    keys.connect_key_pressed(move |_, key, _, mods| {
        if mods.contains(gdk::ModifierType::CONTROL_MASK) && matches!(key, gdk::Key::v | gdk::Key::V) {
            f();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    ed.root.add_controller(keys);

    if !embedded {
    let keep = RefCell::new(Some(ed.clone()));
    ed.win.connect_close_request(move |_| {
        let Some(ed) = keep.borrow().clone() else { return glib::Propagation::Proceed };
        if ed.dirty.get() {
            confirm(&ed, |ed| ed.win.close());
            return glib::Propagation::Stop;
        }
        keep.take();
        glib::Propagation::Proceed
    });
    }

    set_grid(&ed, start, (1, 0));
    refresh(&ed);
    if !embedded { ed.win.present(); }
    Some(ed)
}

/// Names row (padded to the width) above the matrix; NaN shows blank.
fn with_names(mut names: Vec<String>, m: &Matrix) -> Grid {
    names.resize(m.cols, String::new());
    let cell = |x: f64| if x.is_nan() { String::new() } else { io::fmt_num(x) };
    std::iter::once(names).chain((0..m.rows).map(|i| (0..m.cols).map(|j| cell(m.get(i, j))).collect())).collect()
}

fn set_class(w: &impl IsA<gtk::Widget>, class: &str, on: bool) {
    if on {
        w.add_css_class(class);
    } else {
        w.remove_css_class(class);
    }
}

/// Octave string literal.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Replace the grid (rows padded to the widest, at least one data row; columns rebuilt if needed),
/// then focus cell `at` once the new rows have widgets.
fn set_grid(ed: &Rc<Ed>, mut g: Grid, at: (i64, i64)) {
    ed.active.borrow_mut().take();
    let w = g.iter().map(Vec::len).max().unwrap_or(0).max(1);
    g.resize(g.len().max(2), vec![]);
    for r in &mut g {
        r.resize(w, String::new());
    }
    if w != ed.cols.get() {
        ed.cols.set(w);
        columns(ed);
    }
    let (r, c) = ed.cur.get();
    ed.cur.set((r.min(g.len() as u32 - 1), c.min(w - 1)));
    let rows: Vec<glib::BoxedAnyObject> = g.into_iter().map(glib::BoxedAnyObject::new).collect();
    ed.store.splice(0, ed.store.n_items(), &rows);
    let names = rows[0].borrow::<Vec<String>>().clone();
    for (c, n) in names.iter().enumerate() {
        title(ed, c, n);
    }
    let w = Rc::downgrade(ed);
    glib::idle_add_local_once(move || {
        if let Some(ed) = w.upgrade().filter(|ed| ed.root.is_mapped()) {
            go(&ed, at.0.min(ed.store.n_items() as i64 - 1), at.1);
        }
    });
}

fn title(ed: &Ed, c: usize, name: &str) {
    if let Some(col) = ed.cv.columns().item(c as u32 + 1).and_downcast::<gtk::ColumnViewColumn>() {
        col.set_title(Some(&format!("{}  {}", c + 1, name.trim())));
    }
}

/// Row-number column plus one entry column per data column.
fn columns(ed: &Rc<Ed>) {
    while let Some(c) = ed.cv.columns().item(0).and_downcast::<gtk::ColumnViewColumn>() {
        ed.cv.remove_column(&c);
    }
    let f = gtk::SignalListItemFactory::new();
    f.connect_setup(|_, obj| {
        let Some(item) = obj.downcast_ref::<gtk::ListItem>() else { return };
        let l = gtk::Label::builder().xalign(1.0).css_classes(["dim-label"]).build();
        item.bind_property("position", &l, "label").transform_to(|_, p: u32| Some(if p == 0 { String::new() } else { p.to_string() })).sync_create().build();
        item.set_child(Some(&l));
    });
    ed.cv.append_column(&gtk::ColumnViewColumn::builder().factory(&f).fixed_width(56).build());
    for c in 0..ed.cols.get() {
        ed.cv.append_column(&gtk::ColumnViewColumn::builder().factory(&cell_factory(ed, c)).fixed_width(110).resizable(true).build());
    }
}

fn cell_factory(ed: &Rc<Ed>, c: usize) -> gtk::SignalListItemFactory {
    let f = gtk::SignalListItemFactory::new();
    let w = Rc::downgrade(ed);
    let wb = w.clone();
    f.connect_setup(move |_, obj| {
        let Some(item) = obj.downcast_ref::<gtk::ListItem>() else { return };
        let e = gtk::Entry::builder().has_frame(false).width_chars(1).build();
        item.set_child(Some(&e));
        let (w2, it) = (w.clone(), item.downgrade());
        e.connect_changed(move |e| {
            if let (Some(ed), Some(item)) = (w2.upgrade(), it.upgrade()) {
                edited(&ed, &item, c, e);
            }
        });
        let (w2, it) = (w.clone(), item.downgrade());
        let focused_entry = e.clone();
        let fc = gtk::EventControllerFocus::new();
        fc.connect_enter(move |_| {
            if let (Some(ed), Some(item)) = (w2.upgrade(), it.upgrade()) {
                ed.cur.set((item.position(), c));
                focused_entry.set_input_hints(if ed.embedded && ed.keypad_shown.get() && item.position() > 0 { gtk::InputHints::INHIBIT_OSK } else { gtk::InputHints::NONE });
                *ed.active.borrow_mut() = Some((focused_entry.clone(), item, c));
            }
        });
        e.add_controller(fc);
        let (w2, it) = (w.clone(), item.downgrade());
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(move |_, key, _, mods| {
            let (Some(ed), Some(item)) = (w2.upgrade(), it.upgrade()) else { return glib::Propagation::Proceed };
            let (r, c, shift) = (item.position() as i64, c as i64, mods.contains(gdk::ModifierType::SHIFT_MASK));
            match key {
                gdk::Key::Return | gdk::Key::KP_Enter => go(&ed, if shift { r - 1 } else { r + 1 }, c),
                gdk::Key::Down => go(&ed, r + 1, c),
                gdk::Key::Up => go(&ed, r - 1, c),
                gdk::Key::Tab => go(&ed, r, c + 1),
                gdk::Key::ISO_Left_Tab => go(&ed, r, c - 1),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        e.add_controller(keys);
        let (w2, it) = (w.clone(), item.downgrade());
        e.connect_activate(move |_| {
            if let (Some(ed), Some(item)) = (w2.upgrade(), it.upgrade()) {
                go(&ed, item.position() as i64 + 1, c as i64);
            }
        });
    });
    f.connect_bind(move |_, obj| {
        let Some(item) = obj.downcast_ref::<gtk::ListItem>() else { return };
        let (Some(e), Some(row)) = (item.child().and_downcast::<gtk::Entry>(), item.item().and_downcast::<glib::BoxedAnyObject>()) else { return };
        let text = row.borrow::<Vec<String>>().get(c).cloned().unwrap_or_default();
        // set_text clears first: that "changed" must not reach the row
        let ed = wb.upgrade();
        ed.iter().for_each(|ed| ed.busy.set(true));
        e.set_text(&text);
        ed.iter().for_each(|ed| ed.busy.set(false));
        style(&e, item.position() == 0);
    });
    f
}

/// Names row bold with a hint; data cells that aren't numbers flagged.
fn style(e: &gtk::Entry, names: bool) {
    let t = e.text();
    let text = !names && !t.trim().is_empty() && !io::to_num(&t, true).is_some_and(f64::is_finite);
    e.set_placeholder_text(names.then_some("name"));
    e.set_tooltip_text(text.then_some("Enter a finite number. This cell is excluded from charts and statistics."));
    set_class(e, "heading", names);
    set_class(e, "warning", text);
}

/// Entry text -> its row in the model.
fn edited(ed: &Rc<Ed>, item: &gtk::ListItem, c: usize, e: &gtk::Entry) {
    let Some(row) = item.item().and_downcast::<glib::BoxedAnyObject>().filter(|_| !ed.busy.get()) else { return };
    let text = e.text();
    {
        let mut r = row.borrow_mut::<Vec<String>>();
        match r.get_mut(c) {
            Some(s) if *s != text => *s = text.to_string(),
            _ => return,
        }
    }
    let names = item.position() == 0;
    style(e, names);
    if names {
        title(ed, c, &text);
    }
    changed(ed);
}

fn changed(ed: &Ed) {
    ed.dirty.set(true);
    ed.pristine.set(false);
    refresh(ed);
}

/// Status line (size, source) and the statistics when shown.
fn refresh(ed: &Ed) {
    let (names, m) = ed.table();
    let from = match (&*ed.source.borrow(), ed.pristine.get()) {
        (Some((p, sheet)), true) => {
            let file = p.file_name().map_or(String::new(), |f| f.to_string_lossy().into_owned());
            format!(" · {file}{}", sheet.as_ref().map_or(String::new(), |s| format!(" [{s}]")))
        }
        _ => String::new(),
    };
    if m.is_empty() {
        ed.say("Type numbers (Enter moves down), paste a block with Ctrl+V, or import a file", false);
    } else {
        ed.say(&format!("{} rows × {} columns{from}", m.rows, m.cols), false);
    }
    if ed.stats_box.is_visible() {
        ed.stats.set_text(&stats(&names, &m));
    }
    if let Some(f) = &*ed.changed.borrow() { f(); }
}

/// Focus cell (r, c); Tab past the last column wraps to the next row, moving past the last row adds one.
fn go(ed: &Rc<Ed>, r: i64, c: i64) {
    let w = ed.cols.get() as i64;
    let (r, c) = if c >= w { (r + 1, 0) } else if c < 0 { (r - 1, w - 1) } else { (r, c) };
    if r < 0 {
        return;
    }
    if r >= ed.store.n_items() as i64 {
        ed.store.append(&glib::BoxedAnyObject::new(vec![String::new(); w as usize]));
    }
    let r = r.min(ed.store.n_items() as i64 - 1) as u32;
    if let Some(col) = ed.cv.columns().item(c as u32 + 1).and_downcast::<gtk::ColumnViewColumn>() {
        ed.cur.set((r, c as usize));
        // scroll_to only moves the focus when it is already inside the list
        ed.cv.grab_focus();
        ed.cv.scroll_to(r, Some(&col), gtk::ListScrollFlags::FOCUS, None);
    }
}

fn add_row(ed: &Rc<Ed>) {
    let (r, c) = ed.cur.get();
    let mut g = ed.grid();
    g.insert(r as usize + 1, vec![]);
    set_grid(ed, g, (r as i64 + 1, c as i64));
    changed(ed);
}

fn del_row(ed: &Rc<Ed>) {
    let (r, c) = ed.cur.get();
    let mut g = ed.grid();
    if r == 0 || r as usize >= g.len() {
        return ed.say("Pick a data row first (the names row stays)", true);
    }
    g.remove(r as usize);
    set_grid(ed, g, (r as i64, c as i64));
    changed(ed);
}

fn add_col(ed: &Rc<Ed>) {
    let (r, c) = ed.cur.get();
    let mut g = ed.grid();
    for row in &mut g {
        row.insert((c + 1).min(row.len()), String::new());
    }
    set_grid(ed, g, (r as i64, c as i64 + 1));
    changed(ed);
}

fn del_col(ed: &Rc<Ed>) {
    let (r, c) = ed.cur.get();
    if ed.cols.get() < 2 {
        return ed.say("The last column stays", true);
    }
    let mut g = ed.grid();
    for row in &mut g {
        if c < row.len() {
            row.remove(c);
        }
    }
    set_grid(ed, g, (r as i64, c.min(ed.cols.get() - 2) as i64));
    changed(ed);
}

fn paste(ed: &Rc<Ed>) {
    let w = Rc::downgrade(ed);
    ed.win.clipboard().read_text_async(None::<&gio::Cancellable>, move |r| {
        let Some(ed) = w.upgrade() else { return };
        let Ok(Some(text)) = r else { return ed.say("Paste: no text on the clipboard", true) };
        let block = io::parse_text(&text, None);
        let entry = ed.active.borrow().as_ref().filter(|(_, item, c)| item.position() != gtk::INVALID_LIST_POSITION && ed.cur.get() == (item.position(), *c)).map(|(e, _, _)| e.clone());
        if let ([row], Some(e)) = (&block[..], entry)
            && let [v] = &row[..]
        {
            e.delete_selection();
            let mut p = e.position();
            e.insert_text(v, &mut p);
            e.set_position(p);
            return;
        }
        put(&ed, block);
    });
}

/// Paste a block at the current cell, growing the grid. At the top, a header line fills the names row.
fn put(ed: &Rc<Ed>, block: Grid) {
    if block.is_empty() {
        return;
    }
    let (r0, c) = ed.cur.get();
    let mut g = ed.grid();
    let write = |g: &mut Grid, r: usize, cells: &[String]| {
        if g.len() <= r {
            g.resize(r + 1, vec![]);
        }
        let row = &mut g[r];
        if row.len() < c + cells.len() {
            row.resize(c + cells.len(), String::new());
        }
        row[c..c + cells.len()].clone_from_slice(cells);
    };
    let (head, body) = io::split_header(block.clone());
    let (mut r, block) = if r0 == 0 || (r0 == 1 && !head.is_empty() && !body.is_empty()) {
        if let Some(names) = head.last() {
            write(&mut g, 0, names);
        }
        (1, body)
    } else {
        (r0 as usize, block)
    };
    for row in &block {
        write(&mut g, r, row);
        r += 1;
    }
    set_grid(ed, g, (r0 as i64, c as i64));
    changed(ed);
}

/// Run `then` now if nothing is unsaved, else once the user agrees to discard it.
fn confirm(ed: &Rc<Ed>, then: impl FnOnce(&Rc<Ed>) + 'static) {
    if !ed.dirty.get() {
        return then(ed);
    }
    let d = gtk::AlertDialog::builder()
        .message("Discard the edited data?")
        .detail(if ed.embedded { "Export the table first if you want to keep it." } else { "It isn't in the workspace or a file yet." })
        .buttons(["Cancel", "Discard"])
        .cancel_button(0)
        .default_button(0)
        .modal(true)
        .build();
    let w = Rc::downgrade(ed);
    d.choose(Some(&ed.win), None::<&gio::Cancellable>, move |r| {
        if let (Ok(1), Some(ed)) = (r, w.upgrade()) {
            ed.dirty.set(false);
            then(&ed);
        }
    });
}

fn import(ed: &Rc<Ed>) {
    confirm(ed, |ed| {
        let data = gtk::FileFilter::new();
        data.set_name(Some("Data (csv, tsv, txt, xlsx, xls, ods)"));
        for s in ["csv", "tsv", "txt", "dat", "xlsx", "xlsm", "xlsb", "xls", "ods"] {
            data.add_suffix(s);
        }
        let all = gtk::FileFilter::new();
        all.set_name(Some("All files"));
        all.add_pattern("*");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&data);
        filters.append(&all);
        let d = gtk::FileDialog::builder().title("Import data").filters(&filters).modal(true).build();
        let w = Rc::downgrade(ed);
        d.open(Some(&ed.win), None::<&gio::Cancellable>, move |r| {
            if let (Some(ed), Some(p)) = (w.upgrade(), r.ok().and_then(|f| f.path())) {
                load(&ed, p, None);
            }
        });
    });
}

/// Import a file (a workbook's `sheet`, default the first): its last header row names the columns.
fn load(ed: &Rc<Ed>, path: PathBuf, sheet: Option<String>) {
    let names = if io::is_book(&path) { io::sheet_names(&path).unwrap_or_default() } else { vec![] };
    let sheet = sheet.or_else(|| names.first().cloned());
    let g = match io::read_cells(&path, sheet.as_deref(), None) {
        Ok(g) => g,
        Err(e) => return ed.say(&e, true),
    };
    ed.busy.set(true);
    ed.sheets.set_model(Some(&gtk::StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>())));
    ed.sheets.set_selected(sheet.as_ref().and_then(|s| names.iter().position(|n| n == s)).unwrap_or(0) as u32);
    ed.sheets.set_visible(names.len() > 1);
    ed.busy.set(false);
    let (mut head, mut body) = io::split_header(g);
    body.insert(0, head.pop().unwrap_or_default());
    set_grid(ed, body, (1, 0));
    *ed.source.borrow_mut() = Some((path, sheet));
    ed.pristine.set(true);
    ed.dirty.set(false);
    refresh(ed);
}

fn export(ed: &Rc<Ed>) {
    let d = gtk::FileDialog::builder().title("Export data (.csv or .tsv)").initial_name(format!("{}.csv", ed.var())).modal(true).build();
    let w = Rc::downgrade(ed);
    d.save(Some(&ed.win), None::<&gio::Cancellable>, move |r| {
        let (Some(ed), Some(p)) = (w.upgrade(), r.ok().and_then(|f| f.path())) else { return };
        let tab = p.extension().is_some_and(|e| ["tsv", "tab", "txt"].iter().any(|t| e.eq_ignore_ascii_case(t)));
        let (names, mut g) = ed.cells();
        if names.iter().any(|n| !n.is_empty()) {
            g.insert(0, names);
        }
        match io::write_cells(&p, &g, if tab { '\t' } else { ',' }) {
            Ok(()) => {
                ed.dirty.set(false);
                ed.say(&format!("Exported to {}", p.display()), false);
            }
            Err(e) => ed.say(&e, true),
        }
    });
}

/// Run Octave code in the main window, echoed like the REPL.
fn run(ed: &Ed, code: &str) -> bool {
    ed.ui.print(&format!(">> {code}\n"), false);
    match super::eval_octave(&ed.ui, code) {
        Ok(()) => true,
        Err(e) => {
            ed.ui.print(&format!("error: {e}\n"), true);
            ed.say(&e, true);
            false
        }
    }
}

/// Assign the matrix and its validly named columns; the variable name and column names on success.
fn to_workspace(ed: &Rc<Ed>) -> Option<(String, Vec<String>)> {
    let name = ed.var();
    if io::valid_name(&name) != name {
        ed.say(&format!("'{name}' isn't a valid variable name"), true);
        return None;
    }
    let (names, m) = ed.table();
    if m.is_empty() {
        ed.say("No data yet", true);
        return None;
    }
    let cols: Vec<String> =
        names.iter().enumerate().filter(|(_, n)| !n.is_empty() && io::valid_name(n) == **n && **n != name).map(|(j, n)| format!("{n} = {name}(:,{});", j + 1)).collect();
    // an unedited import is readmatrix's result: run that, so the console holds a repeatable line
    let source = ed.source.borrow().clone().filter(|_| ed.pristine.get());
    let mut code = match source {
        Some((p, sheet)) => {
            let sheet = sheet.map_or(String::new(), |s| format!(", 'Sheet', {}", quote(&s)));
            format!("{name} = readmatrix({}{sheet});", quote(&p.to_string_lossy()))
        }
        None => {
            ed.ui.interp.borrow_mut().vars.insert(name.clone(), Value::Num(m.clone()));
            ed.ui.print(&format!("% {name}: {}x{} from the data editor\n", m.rows, m.cols), false);
            String::new()
        }
    };
    code = [code, cols.join(" ")].join(" ").trim().to_string();
    if code.is_empty() {
        super::sync(&ed.ui);
    } else if !run(ed, &code) {
        return None;
    }
    ed.dirty.set(false);
    ed.say(&format!("{name}: {}x{} in the workspace", m.rows, m.cols), false);
    Some((name, names))
}

/// To workspace, then plot through Octave: columns 2… against column 1, names as legend / label.
fn plot(ed: &Rc<Ed>) {
    let Some((v, names)) = to_workspace(ed) else { return };
    let code = if names.len() == 1 {
        let label = if names[0].is_empty() { String::new() } else { format!(" ylabel({});", quote(&names[0])) };
        format!("plot({v});{label}")
    } else {
        let mut s = format!("plot({v}(:,1), {v}(:,2:end));");
        if names[1..].iter().any(|n| !n.is_empty()) {
            let leg: Vec<String> = names[1..].iter().enumerate().map(|(k, n)| quote(if n.is_empty() { format!("column {}", k + 2) } else { n.clone() }.as_str())).collect();
            s += &format!(" legend({});", leg.join(", "));
        }
        if !names[0].is_empty() {
            s += &format!(" xlabel({});", quote(&names[0]));
        }
        s
    };
    run(ed, &code);
}

/// n, mean, median, std, min, max per column (NaN skipped), as an aligned text table.
fn stats(names: &[String], m: &Matrix) -> String {
    // 6 significant digits, exponent when tiny or huge
    let short = |x: f64| {
        let y: f64 = format!("{x:.5e}").parse().unwrap_or(x);
        if y.is_finite() && y != 0.0 && (y.abs() < 1e-3 || y.abs() >= 1e6) { format!("{y:e}") } else { io::fmt_num(y) }
    };
    let mut cols: Vec<Vec<String>> = vec![["", "n", "mean", "median", "std", "min", "max"].map(String::from).to_vec()];
    for j in 0..m.cols {
        let mut v: Vec<f64> = (0..m.rows).map(|i| m.get(i, j)).filter(|x| !x.is_nan()).collect();
        v.sort_by(f64::total_cmp);
        let head = names.get(j).filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| format!("{}", j + 1));
        let n = v.len();
        if n == 0 {
            cols.push(vec![head, "0".into(), "".into(), "".into(), "".into(), "".into(), "".into()]);
            continue;
        }
        let mean = v.iter().sum::<f64>() / n as f64;
        let median = if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 };
        let std = if n > 1 { (v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1) as f64).sqrt() } else { 0.0 };
        cols.push(vec![head, n.to_string(), short(mean), short(median), short(std), short(v[0]), short(v[n - 1])]);
    }
    let widths: Vec<usize> = cols.iter().map(|c| c.iter().map(|s| s.chars().count()).max().unwrap_or(0)).collect();
    (0..7)
        .map(|i| {
            let line: Vec<String> = cols.iter().zip(&widths).enumerate().map(|(k, (c, &w))| if k == 0 { format!("{:<w$}", c[i]) } else { format!("{:>w$}", c[i]) }).collect();
            line.join("   ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    #[test]
    fn stats_table() {
        let m = crate::value::Matrix::from_rows(&[vec![1., f64::NAN], vec![2., f64::NAN], vec![4., f64::NAN]]);
        let t = super::stats(&["x".into(), String::new()], &m);
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(lines.len(), 7);
        assert!(lines[0].contains('x') && lines[0].trim_end().ends_with('2'), "{t}");
        assert!(lines[1].starts_with("n ") && lines[1].contains(" 3 ") && lines[1].trim_end().ends_with('0'), "{t}");
        assert!(lines[2].contains("2.33333") && lines[3].contains(" 2 "), "{t}");
        assert!(lines[4].contains("1.52753"), "{t}"); // sample std of 1 2 4
    }
}
