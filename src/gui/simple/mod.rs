//! Touch-first classroom shell. Advanced widgets and preferences remain intact.
mod board;
mod model;
mod worker;

use super::{Ui, calc, data, keypad};
use crate::{plot, qalc};
use board::Board;
use gtk::prelude::*;
use gtk::{gio, glib};
use gtk4 as gtk;
pub(super) use model::SimpleTab;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
    mpsc,
};
use std::time::Duration;
use worker::{Job, Reply};

pub const CSS: &str = "
.simple { background: #f4f7fb; color: #182639; font-size: 18px; }
.simple button, headerbar.simple button, .simple menubutton, .simple dropdown { min-height: 56px; min-width: 48px; padding: 0 10px; }
.simple entry { min-height: 56px; font-size: 22px; background: #fff; color: #182639; }
.simple entry:focus-within { outline: 3px solid #2878d1; outline-offset: -3px; }
.simple .error { color: #a51d2d; }
.simple .warning { background: #fff0d8; color: #823600; }
.simple .simple-hint { font-size: 15px; color: #52647a; }
.simple .simple-result { font-size: 32px; font-weight: bold; }
.simple .simple-card { background: #fff; border: 1px solid #d8e1ed; border-radius: 8px; padding: 10px; }
.simple .simple-keypad { background: #e5ebf3; padding: 6px; }
.simple .simple-keypad button { font-size: 22px; min-height: 28px; transition: background-color 120ms ease-in-out; }
.simple .simple-keypad .suggested-action { background: #226ccc; color: #fff; }
.simple .simple-keypad .number { background: #fff; color: #182639; }
.simple .simple-keypad .number:hover { background: #dce5ef; }
.simple .simple-keypad .number:active { background: #c6d5e6; }
.simple .simple-data-tools button { min-height: 36px; min-width: 0; padding: 0 8px; font-size: 15px; }
.simple .simple-tabs button:checked { background: #226ccc; color: #fff; }
.simple columnview { background: #fff; color: #182639; }
.simple .simple-stats { background: #fff; padding: 10px; border-radius: 8px; }
";

struct GraphRow {
    id: usize,
    root: gtk::Box,
    entry: gtk::Entry,
    visible: gtk::ToggleButton,
    error: gtk::Label,
    color: plot::Rgb,
}

pub struct Simple {
    pub root: gtk::Box,
    pub header: gtk::HeaderBar,
    ui: Weak<Ui>,
    tab: Cell<SimpleTab>,
    stack: gtk::Stack,
    tabs: Vec<gtk::ToggleButton>,
    entry: gtk::Entry,
    preview: gtk::Label,
    message: gtk::Label,
    history: gtk::Box,
    history_scroll: gtk::ScrolledWindow,
    history_rows: RefCell<Vec<gtk::Widget>>,
    last_expr: RefCell<String>,
    last_answer: RefCell<String>,
    graph_this: gtk::Button,
    options: RefCell<qalc::Options>,
    graph_rows: RefCell<Vec<Rc<GraphRow>>>,
    graph_list: gtk::Box,
    next_row: Cell<usize>,
    graph_focus: Cell<usize>,
    graph: Rc<Board>,
    pub data: Rc<data::Ed>,
    data_board: Rc<Board>,
    chart: gtk::DropDown,
    xcol: gtk::DropDown,
    ycol: gtk::DropDown,
    column_names: RefCell<Vec<String>>,
    stats: gtk::Label,
    selectors_busy: Cell<bool>,
    keypad: gtk::Box,
    keypad_height: Rc<super::resize::KeypadHeight>,
    function_keys: gtk::Stack,
    more: gtk::ToggleButton,
    angle: gtk::ToggleButton,
    pad_bar: gtk::Box,
    keypad_toggle: gtk::Button,
    sender: mpsc::Sender<Job>,
    calc_rev: Arc<AtomicU64>,
    graph_rev: Arc<AtomicU64>,
    calc_pending: RefCell<Option<glib::SourceId>>,
    graph_pending: RefCell<Option<glib::SourceId>>,
    data_pending: RefCell<Option<glib::SourceId>>,
}

fn label(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).wrap(true).wrap_mode(gtk::pango::WrapMode::WordChar).build()
}
fn column(spacing: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, spacing)
}
fn scroll(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder().child(child).vexpand(true).hexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build()
}

fn slot_navigation(input: &gtk::Entry) {
    let controller = gtk::EventControllerKey::new();
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    let entry = input.clone();
    controller.connect_key_pressed(move |_, key, _, mods| {
        if !matches!(key, gtk::gdk::Key::Tab | gtk::gdk::Key::ISO_Left_Tab) {
            return glib::Propagation::Proceed;
        }
        let p = entry.position().max(0) as usize;
        let bounds = entry.selection_bounds().map(|(a, b)| (a as usize, b as usize)).unwrap_or((p, p));
        let backwards = key == gtk::gdk::Key::ISO_Left_Tab || mods.contains(gtk::gdk::ModifierType::SHIFT_MASK);
        if let Some(i) = keypad::next_slot(&entry.text(), bounds, backwards) {
            entry.select_region(i as i32, i as i32 + 1);
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    input.add_controller(controller);
}

impl Simple {
    pub fn new(ui: &Rc<Ui>) -> Rc<Self> {
        let root = column(6);
        root.add_css_class("simple");
        let header = gtk::HeaderBar::new();
        header.add_css_class("simple");
        let nav = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        nav.add_css_class("simple-tabs");
        let mut tabs = vec![];
        for tab in [SimpleTab::Calculator, SimpleTab::Graph, SimpleTab::Data] {
            let button = gtk::ToggleButton::with_label(tab.name());
            if let Some(first) = tabs.first() {
                button.set_group(Some(first));
            }
            button.set_focus_on_click(false);
            nav.append(&button);
            tabs.push(button);
        }
        header.pack_start(&nav);
        let advanced = gtk::Button::with_label("Advanced");
        advanced.set_focus_on_click(false);
        header.pack_end(&advanced);
        let menu = gio::Menu::new();
        menu.append(Some("Fullscreen"), Some("app.fullscreen"));
        menu.append(Some("Export chart…"), Some("app.simple-export"));
        let menu_button = gtk::MenuButton::builder().label("Menu").menu_model(&menu).build();
        header.pack_end(&menu_button);
        let stack = gtk::Stack::builder().vexpand(true).hexpand(true).vhomogeneous(false).hhomogeneous(false).build();
        let work = column(8);
        work.set_margin_start(12);
        work.set_margin_end(12);
        let history = column(8);
        history.set_valign(gtk::Align::End);
        let history_scroll = scroll(&history);
        work.append(&history_scroll);
        let intro = label("Enter a calculation. Tap an earlier answer to use it again.");
        intro.add_css_class("simple-hint");
        work.append(&intro);
        let entry = gtk::Entry::builder().placeholder_text("Try 12 × 8, 3/4 + 1/2, or sin(30)").hexpand(true).build();
        entry.set_input_hints(gtk::InputHints::INHIBIT_OSK);
        slot_navigation(&entry);
        work.append(&entry);
        let preview = label("");
        preview.add_css_class("simple-result");
        preview.set_selectable(true);
        work.append(&preview);
        let message = label("");
        message.add_css_class("error");
        work.append(&message);
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let fraction = gtk::ToggleButton::with_label("Decimal result");
        controls.append(&fraction);
        let graph_this = gtk::Button::with_label("Graph this");
        graph_this.set_sensitive(false);
        controls.append(&graph_this);
        work.append(&controls);
        stack.add_named(&work, Some("Calculator"));

        let graph = Board::new(true);
        graph.root.remove(&graph.tools);
        graph.root.prepend(&gtk::ScrolledWindow::builder()
            .child(&graph.tools)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .build());
        let graph_list = column(8);
        let graph_left = column(6);
        graph_left.set_width_request(310);
        let hint = label("Functions of x");
        graph_left.append(&hint);
        graph_left.append(&scroll(&graph_list));
        let add_graph = gtk::Button::with_label("+ Add function");
        graph_left.append(&add_graph);
        let graph_pane = gtk::Paned::builder()
            .orientation(gtk::Orientation::Horizontal)
            .start_child(&graph_left)
            .end_child(&graph.root)
            .position(340)
            .shrink_start_child(false)
            .shrink_end_child(false)
            .wide_handle(true)
            .build();
        stack.add_named(&graph_pane, Some("Graph"));

        let data = data::embedded(ui);
        let data_board = Board::new(false);
        let chart = gtk::DropDown::from_strings(&["Scatter", "Line", "Bar", "Histogram"]);
        let xcol = gtk::DropDown::from_strings(&["Row number", "X", "Y"]);
        xcol.set_selected(1);
        let ycol = gtk::DropDown::from_strings(&["X", "Y"]);
        ycol.set_selected(1);
        let data_right = column(6);
        let selectors = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        selectors.append(&chart);
        selectors.append(&label("X"));
        selectors.append(&xcol);
        selectors.append(&label("Y / values"));
        selectors.append(&ycol);
        data_board.root.remove(&data_board.tools);
        selectors.append(&data_board.tools);
        // Horizontal scrolling keeps touch controls reachable at increased display scaling.
        data_right.append(&gtk::ScrolledWindow::builder().child(&selectors).vscrollbar_policy(gtk::PolicyType::Never).propagate_natural_height(true).build());
        data_right.append(&data_board.root);
        let stats = label("Enter data to see statistics.");
        stats.add_css_class("simple-stats");
        data_right.append(&stats);
        let data_pane = gtk::Paned::builder()
            .orientation(gtk::Orientation::Horizontal)
            .start_child(&data.root)
            .end_child(&data_right)
            .position(460)
            .shrink_start_child(false)
            .shrink_end_child(false)
            .wide_handle(true)
            .build();
        stack.add_named(&data_pane, Some("Data"));
        // Keep the keypad reachable on boards with a small logical viewport. The
        // workspace can scroll vertically instead of forcing the window offscreen.
        let workspace = scroll(&stack);
        workspace.set_min_content_height(80);
        root.append(&workspace);

        let pad_bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let angle = gtk::ToggleButton::with_label("DEG");
        angle.set_tooltip_text(Some("Degrees. Tap to switch to radians."));
        angle.set_focus_on_click(false);
        let more = gtk::ToggleButton::with_label("More functions");
        more.set_focus_on_click(false);
        let keypad_toggle = gtk::Button::with_label("Hide keypad");
        keypad_toggle.set_focus_on_click(false);
        pad_bar.append(&angle);
        pad_bar.append(&more);
        pad_bar.append(&keypad_toggle);
        root.append(&pad_bar);
        let keypad = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        keypad.add_css_class("simple-keypad");
        let function_keys = gtk::Stack::builder().hexpand(true).hhomogeneous(true).vhomogeneous(true).build();
        keypad.append(&function_keys);
        let keypad_height = super::resize::KeypadHeight::new(&keypad, 256, 160, 260);
        root.append(&keypad_height.root);
        let calc_rev = Arc::new(AtomicU64::new(0));
        let graph_rev = Arc::new(AtomicU64::new(0));
        let (sender, replies) = worker::start(calc_rev.clone(), graph_rev.clone());
        let s = Rc::new(Self {
            root,
            header,
            ui: Rc::downgrade(ui),
            tab: Cell::new(SimpleTab::Calculator),
            stack,
            tabs,
            entry,
            preview,
            message,
            history,
            history_scroll,
            history_rows: RefCell::new(vec![]),
            last_expr: RefCell::new(String::new()),
            last_answer: RefCell::new(String::new()),
            graph_this,
            options: RefCell::new(qalc::Options { angle: "deg".into(), ..Default::default() }),
            graph_rows: RefCell::new(vec![]),
            graph_list,
            next_row: Cell::new(0),
            graph_focus: Cell::new(0),
            graph,
            data,
            data_board,
            chart,
            xcol,
            ycol,
            column_names: RefCell::new(vec!["X".into(), "Y".into()]),
            stats,
            selectors_busy: Cell::new(false),
            keypad,
            keypad_height,
            function_keys,
            more,
            angle: angle.clone(),
            pad_bar,
            keypad_toggle,
            sender,
            calc_rev,
            graph_rev,
            calc_pending: RefCell::new(None),
            graph_pending: RefCell::new(None),
            data_pending: RefCell::new(None),
        });
        s.build_keypad();
        for (button, tab) in s.tabs.iter().zip([SimpleTab::Calculator, SimpleTab::Graph, SimpleTab::Data]) {
            let weak = Rc::downgrade(&s);
            button.connect_toggled(move |b| {
                if b.is_active() {
                    if let Some(s) = weak.upgrade() {
                        s.select(tab);
                    }
                }
            });
        }
        let weak = Rc::downgrade(ui);
        advanced.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                super::show_simple(&ui, false);
            }
        });
        let weak = Rc::downgrade(&s);
        s.entry.connect_changed(move |_| {
            if let Some(s) = weak.upgrade() {
                s.queue_calculation(false);
            }
        });
        let weak = Rc::downgrade(&s);
        s.entry.connect_activate(move |_| {
            if let Some(s) = weak.upgrade() {
                s.queue_calculation(true);
            }
        });
        let weak = Rc::downgrade(&s);
        fraction.connect_toggled(move |b| {
            if let Some(s) = weak.upgrade() {
                s.options.borrow_mut().fraction = if b.is_active() { "decimal" } else { "auto" }.into();
                if s.entry.text().trim().is_empty() {
                    s.entry.set_text(&s.last_expr.borrow());
                } else {
                    s.queue_calculation(false);
                }
            }
        });
        let weak = Rc::downgrade(&s);
        angle.connect_toggled(move |b| {
            if let Some(s) = weak.upgrade() {
                let unit = if b.is_active() { "rad" } else { "deg" };
                s.options.borrow_mut().angle = unit.into();
                b.set_label(if b.is_active() { "RAD" } else { "DEG" });
                b.set_tooltip_text(Some(if b.is_active() { "Radians. Tap for degrees." } else { "Degrees. Tap for radians." }));
                s.queue_calculation(false);
                s.queue_graph(false);
            }
        });
        let weak = Rc::downgrade(&s);
        s.graph_this.connect_clicked(move |_| {
            if let Some(s) = weak.upgrade() {
                let text = if s.entry.text().trim().is_empty() { s.last_expr.borrow().clone() } else { s.entry.text().to_string() };
                s.add_graph(&text);
                s.select(SimpleTab::Graph);
            }
        });
        let weak = Rc::downgrade(&s);
        add_graph.connect_clicked(move |_| {
            if let Some(s) = weak.upgrade() {
                s.add_graph("");
            }
        });
        let weak = Rc::downgrade(&s);
        s.more.connect_toggled(move |_| {
            if let Some(s) = weak.upgrade() {
                s.show_keys();
            }
        });
        let weak = Rc::downgrade(&s);
        s.keypad_toggle.connect_clicked(move |_| {
            if let Some(s) = weak.upgrade() {
                s.toggle_keypad();
            }
        });
        let weak = Rc::downgrade(&s);
        s.graph.connect_changed(move || {
            if let Some(s) = weak.upgrade() {
                s.queue_graph(false);
            }
        });
        let weak = Rc::downgrade(&s);
        s.data.connect_changed(move || {
            if let Some(s) = weak.upgrade() {
                s.queue_data();
            }
        });
        for d in [&s.chart, &s.xcol, &s.ycol] {
            let weak = Rc::downgrade(&s);
            d.connect_selected_notify(move |_| {
                if let Some(s) = weak.upgrade() {
                    if !s.selectors_busy.get() {
                        s.queue_data();
                    }
                }
            });
        }
        let weak = Rc::downgrade(&s);
        glib::timeout_add_local(Duration::from_millis(30), move || {
            let Some(s) = weak.upgrade() else { return glib::ControlFlow::Break };
            for reply in replies.try_iter() {
                s.receive(reply);
            }
            glib::ControlFlow::Continue
        });
        // Keep the established calculator history format and display only classroom controls.
        let saved = calc::parse_history(&std::fs::read_to_string(calc::history_path()).unwrap_or_default());
        for record in saved {
            s.history_card(&record);
        }
        s.add_graph("");
        s.select(SimpleTab::Calculator);
        s.refresh_data();
        let weak = Rc::downgrade(&s);
        let a = gio::SimpleAction::new("simple-export", None);
        a.connect_activate(move |_, _| {
            if let Some(s) = weak.upgrade() {
                s.export();
            }
        });
        if let Some(app) = ui.win.application() {
            app.add_action(&a);
        }
        s
    }

    pub fn select(&self, tab: SimpleTab) {
        self.tab.set(tab);
        self.stack.set_visible_child_name(tab.name());
        let i = match tab {
            SimpleTab::Calculator => 0,
            SimpleTab::Graph => 1,
            SimpleTab::Data => 2,
        };
        self.tabs[i].set_active(true);
        self.show_keys();
        self.focus();
    }
    pub fn focus(&self) {
        if self.root.root().is_none() {
            return;
        }
        match self.tab.get() {
            SimpleTab::Calculator => {
                self.entry.grab_focus_without_selecting();
            }
            SimpleTab::Graph => {
                if let Some(row) = self.graph_rows.borrow().iter().find(|r| r.id == self.graph_focus.get()) {
                    let entry = row.entry.clone();
                    glib::idle_add_local_once(move || {
                        if entry.is_mapped() {
                            entry.grab_focus_without_selecting();
                        }
                    });
                }
            }
            SimpleTab::Data => self.data.focus(),
        }
    }
    pub fn toggle_keypad(&self) {
        let show = !self.keypad.is_visible();
        self.keypad.set_visible(show);
        self.keypad_height.root.set_visible(show);
        self.keypad_toggle.set_label(if show { "Hide keypad" } else { "Show keypad" });
        self.entry.set_input_hints(if show { gtk::InputHints::INHIBIT_OSK } else { gtk::InputHints::NONE });
        self.data.set_keypad_shown(show);
        for row in self.graph_rows.borrow().iter() {
            row.entry.set_input_hints(if show { gtk::InputHints::INHIBIT_OSK } else { gtk::InputHints::NONE });
        }
    }
    fn show_keys(&self) {
        let data = self.tab.get() == SimpleTab::Data;
        let graph = self.tab.get() == SimpleTab::Graph;
        let functions = if graph { &self.graph.tools } else { &self.pad_bar };
        for button in [&self.angle, &self.more] {
            if button.parent().as_ref() != Some(functions.upcast_ref()) {
                if let Some(parent) = button.parent().and_downcast::<gtk::Box>() {
                    parent.remove(button);
                }
                functions.append(button);
            }
        }
        let destination = if data { &self.data_board.tools } else { functions };
        if self.keypad_toggle.parent().as_ref() != Some(destination.upcast_ref()) {
            if let Some(parent) = self.keypad_toggle.parent().and_downcast::<gtk::Box>() {
                parent.remove(&self.keypad_toggle);
            }
            destination.append(&self.keypad_toggle);
        }
        self.pad_bar.set_visible(self.tab.get() == SimpleTab::Calculator);
        self.more.set_visible(!data);
        self.angle.set_visible(!data);
        self.function_keys.set_visible_child_name(if self.tab.get() == SimpleTab::Data {
            "data"
        } else if self.more.is_active() {
            "more"
        } else {
            "basic"
        });
    }
    pub fn export(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        if self.tab.get() == SimpleTab::Data {
            self.data_board.export(&ui.win);
        } else {
            self.graph.export(&ui.win);
        }
    }
    pub fn reset_view(&self) {
        if self.tab.get() == SimpleTab::Data {
            self.data_board.reset();
        } else {
            self.graph.reset();
        }
    }
    pub fn run(self: &Rc<Self>) {
        match self.tab.get() {
            SimpleTab::Calculator => self.queue_calculation(true),
            SimpleTab::Graph => self.queue_graph(true),
            SimpleTab::Data => self.data.press(&keypad::Act::Enter),
        }
    }

    fn build_keypad(self: &Rc<Self>) {
        let pages: [(&str, &[(&str, &str)]); 3] = [
            (
                "basic",
                &[
                    ("x", "x"),
                    ("y", "y"),
                    ("x²", "□²"),
                    ("aᵇ", "□^□"),
                    ("(", "("),
                    (")", ")"),
                    ("a/b", "□/□"),
                    ("√", "√(□)"),
                    ("sin", "sin(□)"),
                    ("cos", "cos(□)"),
                    ("tan", "tan(□)"),
                    ("π", "π"),
                    ("←", "@left"),
                    ("→", "@right"),
                    ("Next □", "@slot"),
                    ("Ans", "@ans"),
                ],
            ),
            (
                "more",
                &[
                    ("abs", "abs(□)"),
                    ("∛", "∛(□)"),
                    ("1/x", "1/□"),
                    ("%", "%"),
                    ("asin", "asin(□)"),
                    ("acos", "acos(□)"),
                    ("atan", "atan(□)"),
                    ("e", "e"),
                    ("ln", "ln(□)"),
                    ("log₁₀", "log10(□)"),
                    ("!", "!"),
                    ("10ˣ", "10^□"),
                    ("←", "@left"),
                    ("→", "@right"),
                    ("Next □", "@slot"),
                    ("Ans", "@ans"),
                ],
            ),
            ("data", &[("←", "@left"), ("→", "@right"), ("−", "−"), (",", ","), ("Next row", "@enter"), ("Clear cell", "@clear"), ("⌫", "@back"), (".", ".")]),
        ];
        for (name, keys) in pages {
            let grid = self.key_grid(keys, 4);
            self.function_keys.add_named(&grid, Some(name));
        }
        let numbers = self.key_grid(
            &[
                ("7", "7"),
                ("8", "8"),
                ("9", "9"),
                ("÷", "÷"),
                ("⌫", "@back"),
                ("4", "4"),
                ("5", "5"),
                ("6", "6"),
                ("×", "×"),
                ("Clear", "@clear"),
                ("1", "1"),
                ("2", "2"),
                ("3", "3"),
                ("−", "−"),
                ("=", "="),
                ("0", "0"),
                (".", "."),
                ("Enter", "@enter"),
                ("+", "+"),
                (",", ","),
            ],
            5,
        );
        numbers.set_hexpand(true);
        self.keypad.append(&numbers);
    }
    fn key_grid(self: &Rc<Self>, keys: &[(&str, &str)], cols: usize) -> gtk::Grid {
        let grid = gtk::Grid::builder().row_homogeneous(true).column_homogeneous(true).column_spacing(5).row_spacing(5).build();
        for (i, (face, action)) in keys.iter().enumerate() {
            let b = gtk::Button::with_label(face);
            b.set_focus_on_click(false);
            b.set_hexpand(true);
            b.set_vexpand(true);
            if action.chars().all(|c| c.is_ascii_digit()) {
                b.add_css_class("number");
            }
            if *action == "@enter" {
                b.add_css_class("suggested-action");
            }
            let (weak, action) = (Rc::downgrade(self), action.to_string());
            b.connect_clicked(move |_| {
                if let Some(s) = weak.upgrade() {
                    s.press(&action);
                }
            });
            grid.attach(&b, (i % cols) as i32, (i / cols) as i32, 1, 1);
        }
        grid
    }
    fn press(self: &Rc<Self>, text: &str) {
        let action = match text {
            "@left" => keypad::Act::Left,
            "@right" => keypad::Act::Right,
            "@clear" => keypad::Act::Clear,
            "@back" => keypad::Act::Back,
            "@enter" => keypad::Act::Enter,
            "@ans" => keypad::Act::Insert(format!("({})", self.last_answer.borrow())),
            "−" if self.tab.get() == SimpleTab::Data => keypad::Act::Insert("-".into()),
            _ => keypad::Act::Insert(text.into()),
        };
        if self.tab.get() == SimpleTab::Data {
            self.data.press(&action);
            return;
        }
        let entry = if self.tab.get() == SimpleTab::Calculator {
            self.entry.clone()
        } else {
            let rows = self.graph_rows.borrow();
            let Some(row) = rows.iter().find(|r| r.id == self.graph_focus.get()) else { return };
            row.entry.clone()
        };
        entry.grab_focus_without_selecting();
        if text == "@slot" {
            let p = entry.position().max(0) as usize;
            let (a, b) = entry.selection_bounds().map(|(a, b)| (a as usize, b as usize)).unwrap_or((p, p));
            if let Some(i) = keypad::next_slot(&entry.text(), (a, b), false) {
                entry.select_region(i as i32, i as i32 + 1);
            }
        } else {
            keypad::press(keypad::Target::Entry(&entry, false), &action);
        }
    }

    fn queue_calculation(self: &Rc<Self>, commit: bool) {
        if let Some(id) = self.calc_pending.take() {
            id.remove();
        }
        let id = self.calc_rev.fetch_add(1, Ordering::Relaxed) + 1;
        self.message.set_text("");
        self.graph_this.set_sensitive(false);
        let text = self.entry.text().trim().to_string();
        if text.is_empty() {
            self.preview.set_text("");
            return;
        }
        if text.contains(keypad::SLOT) {
            self.preview.set_text("");
            if commit {
                self.message.set_text("Fill each □ before calculating.");
                self.press("@slot");
            }
            return;
        }
        let weak = Rc::downgrade(self);
        let send = move || {
            if let Some(s) = weak.upgrade() {
                s.calc_pending.take();
                let _ = s.sender.send(Job::Calculate { id, text, options: s.options.borrow().clone(), commit });
            }
        };
        if commit {
            send();
        } else {
            *self.calc_pending.borrow_mut() = Some(glib::timeout_add_local_once(Duration::from_millis(250), send));
        }
    }

    fn add_graph(self: &Rc<Self>, text: &str) {
        let id = self.next_row.get();
        self.next_row.set(id + 1);
        let colors = [[36, 105, 205], [205, 66, 64], [25, 139, 91], [139, 72, 192], [188, 112, 15], [17, 134, 151]];
        let color = colors[id % colors.len()];
        let root = column(4);
        root.add_css_class("simple-card");
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let visible = gtk::ToggleButton::with_label("Show");
        visible.set_active(true);
        visible.set_focus_on_click(false);
        let swatch = gtk::DrawingArea::builder().content_width(14).content_height(14).build();
        swatch.set_draw_func(move |_, cr, w, h| {
            cr.set_source_rgb(color[0] as f64 / 255.0, color[1] as f64 / 255.0, color[2] as f64 / 255.0);
            cr.rectangle(0.0, 0.0, w as f64, h as f64);
            let _ = cr.fill();
        });
        let remove = gtk::Button::with_label("Delete");
        remove.set_focus_on_click(false);
        controls.append(&swatch);
        controls.append(&label(&format!("{}", id + 1)));
        controls.append(&visible);
        controls.append(&remove);
        root.append(&controls);
        let entry = gtk::Entry::builder().placeholder_text("y = x²").text(text).width_chars(1).hexpand(true).build();
        slot_navigation(&entry);
        entry.set_input_hints(gtk::InputHints::INHIBIT_OSK);
        root.append(&entry);
        let error = label("");
        error.add_css_class("error");
        error.add_css_class("simple-hint");
        root.append(&error);
        let row = Rc::new(GraphRow { id, root, entry, visible, error, color });
        self.graph_list.append(&row.root);
        let weak = Rc::downgrade(self);
        row.entry.connect_changed(move |_| {
            if let Some(s) = weak.upgrade() {
                s.queue_graph(false);
            }
        });
        let weak = Rc::downgrade(self);
        row.entry.connect_activate(move |_| {
            if let Some(s) = weak.upgrade() {
                s.queue_graph(true);
                s.add_graph("");
            }
        });
        let fc = gtk::EventControllerFocus::new();
        let weak = Rc::downgrade(self);
        fc.connect_enter(move |_| {
            if let Some(s) = weak.upgrade() {
                s.graph_focus.set(id);
            }
        });
        row.entry.add_controller(fc);
        let weak = Rc::downgrade(self);
        row.visible.connect_toggled(move |_| {
            if let Some(s) = weak.upgrade() {
                s.queue_graph(true);
            }
        });
        let weak = Rc::downgrade(self);
        remove.connect_clicked(move |_| {
            if let Some(s) = weak.upgrade() {
                let mut rows = s.graph_rows.borrow_mut();
                if let Some(i) = rows.iter().position(|r| r.id == id) {
                    let r = rows.remove(i);
                    s.graph_list.remove(&r.root);
                }
                if let Some(row) = rows.last() {
                    s.graph_focus.set(row.id);
                }
                let empty = rows.is_empty();
                drop(rows);
                if empty {
                    s.add_graph("");
                }
                s.queue_graph(true);
                s.focus();
            }
        });
        self.graph_focus.set(id);
        self.graph_rows.borrow_mut().push(row.clone());
        if self.tab.get() == SimpleTab::Graph {
            self.focus();
        }
        if !text.is_empty() {
            self.queue_graph(false);
        }
    }
    fn queue_graph(self: &Rc<Self>, immediate: bool) {
        if let Some(id) = self.graph_pending.take() {
            id.remove();
        }
        let id = self.graph_rev.fetch_add(1, Ordering::Relaxed) + 1;
        let weak = Rc::downgrade(self);
        let send = move || {
            if let Some(s) = weak.upgrade() {
                s.graph_pending.take();
                let rows = s
                    .graph_rows
                    .borrow()
                    .iter()
                    .filter(|r| r.visible.is_active() && !r.entry.text().trim().is_empty())
                    .map(|r| (r.id, r.entry.text().to_string()))
                    .collect();
                let _ = s.sender.send(Job::Graph { id, rows, options: s.options.borrow().clone(), range: s.graph.range() });
            }
        };
        if immediate {
            send();
        } else {
            *self.graph_pending.borrow_mut() = Some(glib::timeout_add_local_once(Duration::from_millis(300), send));
        }
    }

    fn receive(self: &Rc<Self>, reply: Reply) {
        match reply {
            Reply::Calculate { id, text, commit, answer } => {
                if !commit && id != self.calc_rev.load(Ordering::Relaxed) {
                    return;
                }
                match answer {
                    Ok(a) => {
                        if id == self.calc_rev.load(Ordering::Relaxed) {
                            self.preview.set_text(&format!("= {}{}", a.result, if a.approx.is_empty() { String::new() } else { format!("  ≈ {}", a.approx) }));
                            self.graph_this.set_sensitive(a.free_vars == ["x"]);
                        }
                        if commit {
                            let saved = calc::Saved { expr: text.clone(), input: a.input, result: a.result.clone(), approx: a.approx };
                            self.history_card(&saved);
                            let mut history = calc::parse_history(&std::fs::read_to_string(calc::history_path()).unwrap_or_default());
                            history.push(saved);
                            let excess = history.len().saturating_sub(500);
                            history.drain(..excess);
                            if let Err(e) = std::fs::write(calc::history_path(), calc::history_text(&history)) {
                                self.message.set_text(&format!("Answer calculated; history could not be saved: {e}"));
                            }
                            *self.last_expr.borrow_mut() = text.clone();
                            *self.last_answer.borrow_mut() = a.result;
                            if self.entry.text().as_str() == text {
                                self.entry.set_text("");
                                self.graph_this.set_sensitive(a.free_vars == ["x"]);
                            }
                        }
                    }
                    Err(e) => {
                        if id == self.calc_rev.load(Ordering::Relaxed) {
                            self.preview.set_text("");
                            self.message.set_text(&e);
                        }
                    }
                }
            }
            Reply::Graph { id, rows } => {
                if id != self.graph_rev.load(Ordering::Relaxed) {
                    return;
                }
                let mut fig = self.graph.figure.borrow_mut();
                fig.axes().series.clear();
                fig.axes().pins.clear();
                for row in self.graph_rows.borrow().iter() {
                    row.error.set_text("");
                    let Some((_, result)) = rows.iter().find(|(i, _)| *i == row.id) else { continue };
                    match result {
                        Ok((x, y)) => {
                            let mut series = plot::Series::new(plot::Data::Line { x: x.clone(), y: y.clone() });
                            series.color = Some(row.color);
                            series.width = 2.5;
                            series.label = row.entry.text().to_string();
                            fig.axes().series.push(series);
                            if y.iter().all(|v| !v.is_finite()) {
                                row.error.set_text("No real values in this view.");
                            }
                        }
                        Err(e) => row.error.set_text(e),
                    }
                }
                drop(fig);
                self.graph.redraw();
            }
        }
    }
    fn history_card(self: &Rc<Self>, saved: &calc::Saved) {
        let body = column(4);
        body.add_css_class("simple-card");
        body.append(&label(&saved.input));
        let result = label(&saved.result);
        result.add_css_class("simple-result");
        body.append(&result);
        if !saved.approx.is_empty() {
            body.append(&label(&format!("≈ {}", saved.approx)));
        }
        let button = gtk::Button::builder().child(&body).has_frame(false).build();
        let (weak, text) = (Rc::downgrade(self), saved.expr.clone());
        button.connect_clicked(move |_| {
            if let Some(s) = weak.upgrade() {
                s.entry.set_text(&text);
                s.entry.grab_focus_without_selecting();
                s.entry.set_position(-1);
            }
        });
        self.history.append(&button);
        self.history_rows.borrow_mut().push(button.upcast());
        if self.history_rows.borrow().len() > 500 {
            let old = self.history_rows.borrow_mut().remove(0);
            self.history.remove(&old);
        }
        let adj = self.history_scroll.vadjustment();
        glib::idle_add_local_once(move || adj.set_value((adj.upper() - adj.page_size()).max(0.0)));
    }
    fn queue_data(self: &Rc<Self>) {
        if let Some(id) = self.data_pending.take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        *self.data_pending.borrow_mut() = Some(glib::timeout_add_local_once(Duration::from_millis(200), move || {
            if let Some(s) = weak.upgrade() {
                s.data_pending.take();
                s.refresh_data();
            }
        }));
    }
    fn refresh_data(&self) {
        let grid = self.data.grid();
        let names: Vec<String> =
            grid.first().into_iter().flatten().enumerate().map(|(i, n)| if n.trim().is_empty() { format!("Column {}", i + 1) } else { n.clone() }).collect();
        if names != *self.column_names.borrow() {
            self.selectors_busy.set(true);
            let old_x = self.xcol.selected();
            let old_y = self.ycol.selected();
            let mut xs = vec!["Row number"];
            xs.extend(names.iter().map(String::as_str));
            self.xcol.set_model(Some(&gtk::StringList::new(&xs)));
            self.ycol.set_model(Some(&gtk::StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>())));
            self.xcol.set_selected(old_x.min(names.len() as u32));
            self.ycol.set_selected(old_y.min(names.len().saturating_sub(1) as u32));
            *self.column_names.borrow_mut() = names;
            self.selectors_busy.set(false);
        }
        let chart = model::Chart::from_index(self.chart.selected());
        self.xcol.set_sensitive(chart != model::Chart::Histogram);
        let x = self.xcol.selected().checked_sub(1).map(|i| i as usize);
        let (fig, stats, skipped) = model::data_chart(&grid, chart, x, self.ycol.selected() as usize);
        *self.data_board.figure.borrow_mut() = fig;
        self.data_board.redraw();
        let fmt = |v: Option<f64>| v.filter(|v| v.is_finite()).map(plot::cursor::num).unwrap_or_else(|| "—".into());
        self.stats.set_text(&format!(
            "Count: {}    Mean: {}    Median: {}\nMin: {}    Max: {}    Sample standard deviation: {}\n{} missing or invalid values · {} rows skipped in chart",
            stats.count,
            fmt(stats.mean),
            fmt(stats.median),
            fmt(stats.min),
            fmt(stats.max),
            fmt(stats.std),
            stats.skipped,
            skipped
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pump_until(mut ready: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let ctx = glib::MainContext::default();
        loop {
            for _ in 0..100 {
                if !ctx.pending() {
                    break;
                }
                ctx.iteration(false);
            }
            if ready() {
                return;
            }
            assert!(std::time::Instant::now() < deadline, "classroom UI timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn settle() {
        let start = std::time::Instant::now();
        pump_until(|| start.elapsed() > Duration::from_millis(500));
    }

    fn screenshot(ui: &Ui, name: &str) {
        let image = gtk::WidgetPaintable::new(Some(&ui.win));
        ui.win.queue_draw();
        settle();
        let snapshot = gtk::Snapshot::new();
        image.snapshot(&snapshot, ui.win.width() as f64, ui.win.height() as f64);
        let node = snapshot.to_node().expect("window render node");
        let renderer = ui.win.renderer().expect("window renderer");
        renderer.render_texture(&node, None).save_to_png(format!("target/{name}.png")).unwrap();
        eprintln!("{name}: {}×{}", ui.win.width(), ui.win.height());
    }

    #[test]
    #[ignore = "requires a display and an isolated XDG_CONFIG_HOME; run separately with --test-threads=1"]
    fn classroom_ui_smoke() {
        assert!(std::env::var_os("XDG_CONFIG_HOME").is_some(), "set an isolated XDG_CONFIG_HOME for this test");
        gtk::init().unwrap();
        std::fs::create_dir_all(crate::config_dir()).unwrap();
        let path = super::super::ini_path();
        let (mut cfg, _) = super::super::config::Config::load(&path);
        let width = std::env::var("QCALCP_UI_WIDTH").ok().and_then(|v| v.parse().ok()).unwrap_or(1366);
        let height = std::env::var("QCALCP_UI_HEIGHT").ok().and_then(|v| v.parse().ok()).unwrap_or(768);
        cfg.width = width;
        cfg.height = height;
        cfg.maximized = false;
        cfg.calc_mode = false;
        cfg.dark = true;
        cfg.save_state(&path).unwrap();
        let app = gtk::Application::builder().application_id("com.dusty.qcalcp.classroomtest").flags(gio::ApplicationFlags::NON_UNIQUE).build();
        app.register(None::<&gio::Cancellable>).unwrap();
        let ui = super::super::build(&app, None);
        assert_eq!(gtk::Window::default_icon_name().as_deref(), Some("qcalcp"));
        assert!(gtk::IconTheme::for_display(&WidgetExt::display(&ui.win)).has_icon("qcalcp"), "embedded application icon is missing");
        let s = ui.simple.borrow().as_ref().unwrap().clone();
        assert!(ui.simple_mode.get());
        assert_eq!(s.tab.get(), SimpleTab::Calculator);
        assert_eq!(s.options.borrow().angle, "deg");
        assert!(s.keypad.is_visible());
        let advanced_options = qalc::options();
        let advanced_mode = ui.cfg.borrow().calc_mode;
        ui.win.unmaximize();
        ui.win.set_size_request(width, height);
        ui.win.set_default_size(width, height);
        settle();
        s.entry.set_text("3/4 + 1/2");
        s.press("@enter");
        pump_until(|| *s.last_answer.borrow() == "5/4");
        assert!(s.entry.text().is_empty());
        s.press("sin(□)");
        s.press("3");
        s.press("0");
        s.press("@enter");
        pump_until(|| *s.last_answer.borrow() == "1/2");
        s.entry.set_text("sin(");
        s.press("@enter");
        pump_until(|| !s.message.text().is_empty());
        assert_eq!(s.entry.text(), "sin(");
        s.entry.set_text("");
        s.entry.set_text("1234");
        s.entry.select_region(1, 3);
        s.press("9");
        assert_eq!(s.entry.text(), "194");
        s.press("@back");
        assert_eq!(s.entry.text(), "14");
        s.entry.set_text("");
        screenshot(&ui, "simple-calculator-1366");

        s.select(SimpleTab::Graph);
        settle();
        assert!(!s.pad_bar.is_visible());
        for button in [s.angle.upcast_ref::<gtk::Widget>(), s.more.upcast_ref(), s.keypad_toggle.upcast_ref()] {
            assert_eq!(button.parent(), Some(s.graph.tools.clone().upcast()));
        }
        let graph_height = s.graph.area.height();
        let workspace_height = s.root.first_child().unwrap().height();
        s.keypad_height.set_height(160);
        settle();
        assert!(s.root.first_child().unwrap().height() > workspace_height, "shrinking the keypad must expose more workspace");
        assert!(s.graph.area.height() >= graph_height);
        s.keypad_height.set_height(256);
        settle();
        let first = s.graph_rows.borrow()[0].clone();
        first.entry.set_text("");
        s.press("y");
        s.press("=");
        s.press("x");
        assert_eq!(first.entry.text(), "y=x");
        first.entry.set_text("y = x²");
        s.add_graph("y = 2x + 1");
        pump_until(|| s.graph.figure.borrow().axes[0].series.len() == 2);
        let before = s.graph.figure.borrow().axes[0].xlim;
        first.entry.set_text("x² + 1");
        s.queue_graph(true);
        settle();
        assert_eq!(s.graph.figure.borrow().axes[0].xlim, before);
        first.visible.set_active(false);
        pump_until(|| s.graph.figure.borrow().axes[0].series.len() == 1);
        first.visible.set_active(true);
        pump_until(|| s.graph.figure.borrow().axes[0].series.len() == 2);
        first.entry.set_text("sin(");
        pump_until(|| !first.error.text().is_empty());
        assert_eq!(s.graph.figure.borrow().axes[0].series.len(), 1);
        first.entry.set_text("1/x");
        pump_until(|| first.error.text().is_empty() && s.graph.figure.borrow().axes[0].series.len() == 2);
        screenshot(&ui, "simple-graph-1366");
        assert!(ui.interp.borrow().plots.figures.is_empty(), "classroom graph changed Advanced figures");

        s.select(SimpleTab::Data);
        settle();
        assert!(!s.pad_bar.is_visible());
        assert_eq!(s.keypad_toggle.parent(), Some(s.data_board.tools.clone().upcast()));
        let chart_height = s.data_board.area.height();
        s.toggle_keypad();
        settle();
        assert!(s.data_board.area.height() > chart_height);
        assert_eq!(s.keypad_toggle.label().as_deref(), Some("Show keypad"));
        s.toggle_keypad();
        s.select(SimpleTab::Calculator);
        assert!(s.pad_bar.is_visible());
        assert_eq!(s.keypad_toggle.parent(), Some(s.pad_bar.clone().upcast()));
        s.select(SimpleTab::Data);
        settle();
        s.press("−");
        s.press("1");
        assert_eq!(s.data.grid()[1][0], "-1");
        s.press("@clear");
        s.press("1");
        s.press("@enter");
        s.press("2");
        s.press("@enter");
        s.press("3");
        settle();
        let grid = s.data.grid();
        assert_eq!((&grid[1][0], &grid[2][0], &grid[3][0]), (&"1".to_string(), &"2".to_string(), &"3".to_string()));
        s.xcol.set_selected(0);
        s.ycol.set_selected(0);
        for i in 0..4 {
            s.chart.set_selected(i);
            settle();
            assert!(s.stats.text().contains("Mean: 2"), "{}", s.stats.text());
            assert!(s.stats.text().contains("standard deviation: 1"));
        }
        screenshot(&ui, "simple-data-1366");
        assert!(s.data_board.figure.borrow().axes[0].origin_axes);
        s.data.root.activate_action("ed.del-row", None).unwrap();
        settle();
        assert!(s.stats.text().contains("Mean: 1.5"));
        s.data.root.activate_action("ed.del-col", None).unwrap();
        settle();
        assert_eq!(s.data.grid()[0].len(), 1);
        let table = s.data.grid();
        super::super::show_simple(&ui, false);
        settle();
        assert_eq!(qalc::options(), advanced_options);
        assert_eq!(ui.cfg.borrow().calc_mode, advanced_mode);
        super::super::show_simple(&ui, true);
        settle();
        assert_eq!(s.data.grid(), table);
        assert_eq!(s.graph_rows.borrow().len(), 2);
        s.toggle_keypad();
        assert!(!s.keypad.is_visible());
        s.toggle_keypad();
        assert!(s.keypad.is_visible());
        app.activate_action("fullscreen", None);
        pump_until(|| ui.win.is_fullscreen());
        app.activate_action("fullscreen", None);
        pump_until(|| !ui.win.is_fullscreen());
        ui.win.set_size_request(1920, 1080);
        ui.win.set_default_size(1920, 1080);
        s.select(SimpleTab::Graph);
        settle();
        screenshot(&ui, "simple-graph-1920");
        plot::render::save(&s.graph.figure.borrow(), &plot::Theme::light(), std::path::Path::new("target/simple-export.svg"), 1200, 800).unwrap();
        s.data.discard();
        ui.win.close();
        settle();
        let script_ui = super::super::build(&app, Some(std::path::PathBuf::from("demo/euler.m")));
        assert!(!script_ui.simple_mode.get());
        assert!(!script_ui.cfg.borrow().calc_mode);
        script_ui.win.close();
    }
}
