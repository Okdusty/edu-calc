//! Touch-first classroom shell. Advanced widgets and preferences remain intact.
mod board;
mod catalog;
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
.simple-result { font-size: 1.6em; font-weight: bold; }
.simple-preview { font-size: 1.3em; }
.simple-entry { font-size: 1.3em; min-height: 44px; }
.simple-card { padding: 6px 10px; border-radius: 8px; background: alpha(currentColor, 0.05); }
.simple-keypad button { min-height: 30px; }
.simple-side button { padding: 0 8px; min-height: 24px; }
.simple-stat { padding: 1px 12px 1px 0; }
.simple-catalog row { padding: 6px 8px; }
.simple-catalog .catalog-topic { font-weight: bold; margin-top: 8px; }
.simple-catalog .catalog-name { font-weight: bold; font-size: 1.1em; }
.simple-catalog .catalog-example { font-family: monospace; opacity: 0.75; }
";

/// Function pages (5 x 4) behind the side column's page buttons: (name, button, keys); □ marks a slot.
const PAGES: [(&str, &str, &[(&str, &str)]); 4] = [
    (
        "basic",
        "Basic",
        &[
            ("x", "x"), ("y", "y"), ("x²", "□²"), ("aᵇ", "□^□"), ("√", "√(□)"),
            ("(", "("), (")", ")"), ("a/b", "□/□"), ("|a|", "abs(□)"), ("π", "π"),
            ("sin", "sin(□)"), ("cos", "cos(□)"), ("tan", "tan(□)"), ("e", "e"), ("%", "%"),
            ("ln", "ln(□)"), ("log", "log10(□)"), ("n!", "□!"), ("1/x", "1/□"), (",", ","),
        ],
    ),
    (
        "functions",
        "f(x)",
        &[
            ("∛", "∛(□)"), ("ⁿ√", "root(□, □)"), ("eˣ", "e^□"), ("10ˣ", "10^□"), ("logₐ", "log(□, □)"),
            ("nCr", "binomial(□, □)"), ("nPr", "perm(□, □)"), ("gcd", "gcd(□, □)"), ("lcm", "lcm(□, □)"), ("mod", "mod(□, □)"),
            ("round", "round(□)"), ("floor", "floor(□)"), ("ceil", "ceil(□)"), ("|a|", "abs(□)"), ("π", "π"),
            ("x", "x"), ("(", "("), (")", ")"), (",", ","), ("aᵇ", "□^□"),
        ],
    ),
    (
        "trig",
        "Trig",
        &[
            ("sin", "sin(□)"), ("cos", "cos(□)"), ("tan", "tan(□)"), ("π", "π"), ("°", "°"),
            ("sin⁻¹", "asin(□)"), ("cos⁻¹", "acos(□)"), ("tan⁻¹", "atan(□)"), ("x", "x"), ("aᵇ", "□^□"),
            ("sec", "sec(□)"), ("csc", "csc(□)"), ("cot", "cot(□)"), ("(", "("), (")", ")"),
            ("sinh", "sinh(□)"), ("cosh", "cosh(□)"), ("tanh", "tanh(□)"), ("a/b", "□/□"), ("√", "√(□)"),
        ],
    ),
    (
        "stats",
        "Stats",
        &[
            ("mean", "mean(□)"), ("median", "median(□)"), ("mode", "mode(□)"), ("sd", "stdev(□)"), ("var", "var(□)"),
            ("min", "min(□)"), ("max", "max(□)"), ("range", "range(□)"), ("Σ", "total(□)"), ("n", "dimension(□)"),
            ("quart", "quartile(□, □)"), ("%ile", "percentile(□, □)"), ("IQR", "iqr(□)"), ("SE", "stderr(□)"), ("cov", "cov(□, □)"),
            ("r", "pearson(□, □)"), ("ρ", "spearman(□, □)"), ("slope", "slope(□, □)"), ("y-int", "intercept(□, □)"), (",", ","),
        ],
    ),
];
/// Number pad, 6 x 4; Enter takes two columns.
const NUMBERS: &[(&str, &str)] = &[
    ("7", "7"), ("8", "8"), ("9", "9"), ("÷", "÷"), ("⌫", "@back"), ("Clear", "@clear"),
    ("4", "4"), ("5", "5"), ("6", "6"), ("×", "×"), ("←", "@left"), ("→", "@right"),
    ("1", "1"), ("2", "2"), ("3", "3"), ("−", "−"), ("Ans", "@ans"), ("Next □", "@slot"),
    ("0", "0"), (".", "."), ("=", "="), ("+", "+"), ("Enter", "@enter"),
];
/// Number-pad keys a table cell has no use for: dimmed on the Data tab.
const CALC_ONLY: &[&str] = &["÷", "×", "+", "=", "@slot"];

struct GraphRow {
    id: usize,
    root: gtk::Box,
    entry: gtk::Entry,
    visible: gtk::CheckButton,
    error: gtk::Label,
    color: plot::Rgb,
}

pub struct Simple {
    pub root: gtk::Box,
    pub header: gtk::HeaderBar,
    ui: Weak<Ui>,
    tab: Cell<SimpleTab>,
    stack: gtk::Stack,
    entry: gtk::Entry,
    preview: gtk::Label,
    message: gtk::Label,
    history: gtk::Box,
    history_scroll: gtk::ScrolledWindow,
    history_rows: RefCell<Vec<gtk::Widget>>,
    last_expr: RefCell<String>,
    last_answer: RefCell<String>,
    /// the last answer as a table cell (Ans on the Data tab)
    last_number: RefCell<String>,
    /// the table's columns above the entry: tap one to use it
    table_bar: gtk::FlowBox,
    table_note: gtk::Label,
    /// what the calculator was last given, and the names it took
    shared: RefCell<model::TableColumns>,
    shared_names: RefCell<Vec<String>>,
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
    fit: gtk::DropDown,
    column_names: RefCell<Vec<String>>,
    stats: gtk::Label,
    more_stats: gtk::Expander,
    selectors_busy: Cell<bool>,
    /// header's Functions catalog (not on the Data tab)
    functions: gtk::MenuButton,
    keypad: gtk::Box,
    keypad_height: Rc<super::resize::KeypadHeight>,
    side_keys: gtk::Box,
    function_keys: gtk::Stack,
    numbers: RefCell<Option<gtk::Grid>>,
    calc_keys: RefCell<Vec<gtk::Button>>,
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
/// A label that takes no space while it has no text.
fn note(class: &str) -> gtk::Label {
    let l = label("");
    l.add_css_class(class);
    l.set_visible(false);
    l.connect_label_notify(|l| l.set_visible(!l.text().is_empty()));
    l
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
        let root = column(0);
        let stack = gtk::Stack::builder().vexpand(true).hexpand(true).vhomogeneous(false).hhomogeneous(false).build();
        let header = gtk::HeaderBar::new();
        header.set_title_widget(Some(&gtk::StackSwitcher::builder().stack(&stack).build()));
        let advanced = gtk::Button::builder().label("Advanced").tooltip_text("Octave scripts, plots and all calculator options (F2)").focus_on_click(false).build();
        header.pack_start(&advanced);
        let menu = gio::Menu::new();
        menu.append(Some("Fullscreen"), Some("app.fullscreen"));
        menu.append(Some("Export chart…"), Some("app.simple-export"));
        menu.append_submenu(Some("Appearance"), &super::appearance_menu());
        let menu_button = gtk::MenuButton::builder().icon_name("open-menu-symbolic").menu_model(&menu).tooltip_text("Menu").has_frame(false).build();
        header.pack_end(&menu_button);
        let keypad_toggle =
            gtk::Button::builder().icon_name("input-keyboard-symbolic").tooltip_text("Show or hide the keypad (F9)").has_frame(false).focus_on_click(false).build();
        header.pack_end(&keypad_toggle);
        let functions = gtk::MenuButton::builder().label("Functions").tooltip_text("Functions and how to write them; tap one to insert it").focus_on_click(false).build();
        header.pack_end(&functions);

        let work = column(6);
        work.set_margin_top(6);
        work.set_margin_bottom(6);
        work.set_margin_start(12);
        work.set_margin_end(12);
        let history = column(6);
        history.set_valign(gtk::Align::End);
        let history_scroll = scroll(&history);
        work.append(&history_scroll);
        let preview = note("simple-preview");
        preview.set_selectable(true);
        work.append(&preview);
        let message = note("error");
        work.append(&message);
        let entry = gtk::Entry::builder().placeholder_text("Try 12 × 8, 3/4 + 1/2, or sin(30)").hexpand(true).css_classes(["simple-entry"]).build();
        entry.set_input_hints(gtk::InputHints::INHIBIT_OSK);
        slot_navigation(&entry);
        let fraction = gtk::ToggleButton::builder().label("Decimal").tooltip_text("Show results as decimals instead of fractions").focus_on_click(false).build();
        let graph_this = gtk::Button::builder().label("Graph").tooltip_text("Graph this function of x").sensitive(false).focus_on_click(false).build();
        let input = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        input.append(&entry);
        input.append(&fraction);
        input.append(&graph_this);
        let table_note = note("dim-label");
        work.append(&table_note);
        let table_bar = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).min_children_per_line(1).max_children_per_line(30).column_spacing(4).row_spacing(4).visible(false).build();
        work.append(&table_bar);
        work.append(&input);
        stack.add_titled(&work, Some("Calculator"), "Calculator");

        let graph = Board::new(ui, true);
        let graph_list = column(4);
        let graph_left = column(4);
        graph_left.set_margin_start(6);
        graph_left.set_margin_top(6);
        graph_left.set_width_request(240);
        graph_left.append(&scroll(&graph_list));
        let add_graph = gtk::Button::builder().label("Add function").css_classes(["flat"]).halign(gtk::Align::Start).focus_on_click(false).build();
        graph_left.append(&add_graph);
        let graph_pane = gtk::Paned::builder()
            .orientation(gtk::Orientation::Horizontal)
            .start_child(&graph_left)
            .end_child(&graph.root)
            .position(280)
            .resize_start_child(false)
            .shrink_start_child(false)
            .shrink_end_child(false)
            .build();
        stack.add_titled(&graph_pane, Some("Graph"), "Graph");

        let data = data::embedded(ui);
        let data_board = Board::new(ui, false);
        let chart = gtk::DropDown::from_strings(&["Scatter", "Line", "Bar", "Histogram"]);
        let xcol = gtk::DropDown::from_strings(&["Row number", "X", "Y"]);
        xcol.set_selected(1);
        let ycol = gtk::DropDown::from_strings(&["X", "Y"]);
        ycol.set_selected(1);
        let selectors = gtk::Box::builder().spacing(6).css_classes(["toolbar"]).build();
        selectors.append(&chart);
        let fit = gtk::DropDown::from_strings(&model::FitKind::NAMES);
        fit.set_selected(1);
        fit.set_tooltip_text(Some("Curve through the points, with its equation and r²"));
        for (name, d) in [("x", &xcol), ("y", &ycol), ("fit", &fit)] {
            let l = gtk::Label::builder().label(name).margin_start(6).css_classes(["dim-label"]).build();
            selectors.append(&l);
            selectors.append(d);
        }
        let data_right = column(0);
        data_right.append(&selectors);
        data_right.append(&data_board.root);
        let stats = label("");
        stats.set_margin_start(8);
        stats.set_margin_end(8);
        stats.set_margin_bottom(6);
        data_right.append(&stats);
        let more_stats = gtk::Expander::builder().label("More statistics").margin_start(8).margin_end(8).margin_bottom(6).build();
        data_right.append(&more_stats);
        let data_pane = gtk::Paned::builder()
            .orientation(gtk::Orientation::Horizontal)
            .start_child(&data.root)
            .end_child(&data_right)
            .position(300)
            .resize_start_child(false)
            .shrink_start_child(false)
            .shrink_end_child(false)
            .build();
        stack.add_titled(&data_pane, Some("Data"), "Data");
        // Keep the keypad reachable on boards with a small logical viewport. The
        // workspace can scroll vertically instead of forcing the window offscreen.
        let workspace = scroll(&stack);
        workspace.set_min_content_height(80);
        root.append(&workspace);

        let angle = gtk::ToggleButton::builder().label("DEG").tooltip_text("Degrees. Tap for radians.").focus_on_click(false).build();
        let side_keys = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).homogeneous(true).css_classes(["simple-side"]).build();
        let keypad = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        keypad.add_css_class("keypad");
        keypad.add_css_class("simple-keypad");
        keypad.append(&side_keys);
        let function_keys = gtk::Stack::builder().hexpand(true).hhomogeneous(true).vhomogeneous(true).build();
        let mut first: Option<gtk::ToggleButton> = None;
        for (name, face, _) in PAGES {
            let page = gtk::ToggleButton::builder().label(face).active(first.is_none()).focus_on_click(false).build();
            page.set_group(first.as_ref());
            let stack = function_keys.clone();
            page.connect_toggled(move |b| {
                if b.is_active() {
                    stack.set_visible_child_name(name);
                }
            });
            side_keys.append(&page);
            first.get_or_insert(page);
        }
        side_keys.append(&angle);
        keypad.append(&function_keys);
        let keypad_height = super::resize::KeypadHeight::new(&keypad, 200, 140, 240);
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
            entry,
            preview,
            message,
            history,
            history_scroll,
            history_rows: RefCell::new(vec![]),
            last_expr: RefCell::new(String::new()),
            last_answer: RefCell::new(String::new()),
            last_number: RefCell::new(String::new()),
            table_bar,
            table_note,
            shared: RefCell::default(),
            shared_names: RefCell::default(),
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
            fit,
            column_names: RefCell::new(vec!["X".into(), "Y".into()]),
            stats,
            more_stats,
            selectors_busy: Cell::new(false),
            functions,
            keypad,
            keypad_height,
            side_keys,
            function_keys,
            numbers: RefCell::new(None),
            calc_keys: RefCell::new(vec![]),
            sender,
            calc_rev,
            graph_rev,
            calc_pending: RefCell::new(None),
            graph_pending: RefCell::new(None),
            data_pending: RefCell::new(None),
        });
        s.build_keypad();
        let weak = Rc::downgrade(&s);
        s.functions.set_popover(Some(&catalog::popover(move |t| {
            if let Some(s) = weak.upgrade() {
                s.press(t);
            }
        })));
        let weak = Rc::downgrade(&s);
        s.stack.connect_visible_child_name_notify(move |_| {
            if let Some(s) = weak.upgrade() {
                s.sync_tab();
            }
        });
        let weak = Rc::downgrade(ui);
        advanced.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                super::show_simple(&ui, false);
            }
        });
        let weak = Rc::downgrade(&s);
        keypad_toggle.connect_clicked(move |_| {
            if let Some(s) = weak.upgrade() {
                s.toggle_keypad();
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
        s.graph.connect_changed(move || {
            if let Some(s) = weak.upgrade() {
                s.queue_graph(true); // the board throttles gestures itself
            }
        });
        let weak = Rc::downgrade(&s);
        s.data.connect_changed(move || {
            if let Some(s) = weak.upgrade() {
                s.queue_data();
            }
        });
        for d in [&s.chart, &s.xcol, &s.ycol, &s.fit] {
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
        s.sync_tab();
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
        self.stack.set_visible_child_name(tab.name());
        self.sync_tab();
    }
    /// The header's stack switcher and select() both land here.
    fn sync_tab(&self) {
        self.tab.set(match self.stack.visible_child_name().as_deref() {
            Some("Graph") => SimpleTab::Graph,
            Some("Data") => SimpleTab::Data,
            _ => SimpleTab::Calculator,
        });
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
        self.entry.set_input_hints(if show { gtk::InputHints::INHIBIT_OSK } else { gtk::InputHints::NONE });
        self.data.set_keypad_shown(show);
        for row in self.graph_rows.borrow().iter() {
            row.entry.set_input_hints(if show { gtk::InputHints::INHIBIT_OSK } else { gtk::InputHints::NONE });
        }
    }
    fn show_keys(&self) {
        // the table takes numbers only: no function keys, and a number pad that doesn't span the window
        let data = self.tab.get() == SimpleTab::Data;
        self.side_keys.set_visible(!data);
        self.functions.set_sensitive(!data);
        self.function_keys.set_visible(!data);
        if let Some(numbers) = &*self.numbers.borrow() {
            numbers.set_halign(if data { gtk::Align::Center } else { gtk::Align::Fill });
        }
        for b in self.calc_keys.borrow().iter() {
            b.set_sensitive(!data);
        }
    }
    pub fn export(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        if self.tab.get() == SimpleTab::Data {
            self.data_board.export(&ui.win);
        } else {
            self.graph.export(&ui.win);
        }
    }
    /// The boards draw in the Advanced plot theme, which follows the appearance.
    pub fn theme_changed(&self) {
        self.graph.redraw();
        self.data_board.redraw();
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
        for (name, _, keys) in PAGES {
            let grid = self.key_grid(keys, 5);
            self.function_keys.add_named(&grid, Some(name));
        }
        let numbers = self.key_grid(NUMBERS, 6);
        numbers.set_hexpand(true);
        self.keypad.append(&numbers);
        *self.numbers.borrow_mut() = Some(numbers);
    }
    fn key_grid(self: &Rc<Self>, keys: &[(&str, &str)], cols: i32) -> gtk::Grid {
        let grid = gtk::Grid::builder().row_homogeneous(true).column_homogeneous(true).column_spacing(4).row_spacing(4).build();
        let mut at = 0;
        for (face, action) in keys {
            let b = gtk::Button::with_label(face);
            b.set_focus_on_click(false);
            b.set_hexpand(true);
            b.set_vexpand(true);
            let span = if *action == "@enter" { 2 } else { 1 };
            if *action == "@enter" {
                b.add_css_class("suggested-action");
            }
            if CALC_ONLY.contains(action) {
                self.calc_keys.borrow_mut().push(b.clone());
            }
            let (weak, action) = (Rc::downgrade(self), action.to_string());
            b.connect_clicked(move |_| {
                if let Some(s) = weak.upgrade() {
                    s.press(&action);
                }
            });
            grid.attach(&b, at % cols, at / cols, span, 1);
            at += span;
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
            "@ans" if self.tab.get() == SimpleTab::Data => keypad::Act::Insert(self.last_number.borrow().clone()),
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
        let visible = gtk::CheckButton::builder().active(true).tooltip_text("Show on the graph").focus_on_click(false).build();
        let swatch = gtk::DrawingArea::builder().content_width(12).content_height(12).valign(gtk::Align::Center).build();
        swatch.set_draw_func(move |_, cr, w, h| {
            cr.set_source_rgb(color[0] as f64 / 255.0, color[1] as f64 / 255.0, color[2] as f64 / 255.0);
            cr.arc(w as f64 / 2.0, h as f64 / 2.0, w.min(h) as f64 / 2.0, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        });
        let entry = gtk::Entry::builder().placeholder_text("y = x²").text(text).width_chars(1).hexpand(true).build();
        slot_navigation(&entry);
        entry.set_input_hints(gtk::InputHints::INHIBIT_OSK);
        let remove = gtk::Button::builder().icon_name("edit-delete-symbolic").tooltip_text("Delete").css_classes(["flat"]).focus_on_click(false).build();
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        line.append(&visible);
        line.append(&swatch);
        line.append(&entry);
        line.append(&remove);
        let root = column(2);
        root.append(&line);
        let error = note("error");
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
                // half a view more on each side, so a pan shows curve before the next samples land
                let (lo, hi) = s.graph.range();
                let pad = (hi - lo) / 2.0;
                let _ = s.sender.send(Job::Graph { id, rows, options: s.options.borrow().clone(), range: (lo - pad, hi + pad) });
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
                            *self.last_number.borrow_mut() = model::answer_number(&a.result, &a.approx);
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
                let old: Vec<String> = fig.axes().series.drain(..).map(|s| s.label).collect();
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
                // pins stay while only the view moved
                if fig.axes().series.iter().map(|s| &s.label).ne(old.iter()) {
                    fig.axes().pins.clear();
                }
                drop(fig);
                self.graph.redraw();
            }
            Reply::Shared { names } => {
                *self.shared_names.borrow_mut() = names;
                self.rebuild_table_bar();
                // what's in the entry may use the columns that just changed
                if !self.entry.text().trim().is_empty() {
                    self.queue_calculation(false);
                }
            }
        }
    }
    /// Give the calculator the table's columns when they changed, or always with `force` (Advanced
    /// may have redefined the names meanwhile).
    pub fn share_table(&self, force: bool) {
        let table = model::table_columns(&self.data.grid());
        if !force && table == *self.shared.borrow() {
            return;
        }
        let names = |t: &model::TableColumns| t.named.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
        let now = names(&table);
        let gone = names(&self.shared.borrow()).into_iter().filter(|n| !now.contains(n)).collect();
        let _ = self.sender.send(Job::Share { cols: table.named.clone(), gone });
        *self.shared.borrow_mut() = table;
    }
    /// The two columns the Data tab is charting, when the calculator has them; else its first two.
    fn table_pair(&self) -> Option<(String, String)> {
        let (names, taken) = (self.column_names.borrow(), self.shared_names.borrow());
        let pick = |i: Option<u32>| i.and_then(|i| names.get(i as usize)).map(|n| n.trim().to_string()).filter(|n| taken.contains(n));
        let y = pick(Some(self.ycol.selected())).or_else(|| taken.get(1).cloned())?;
        let x = pick(self.xcol.selected().checked_sub(1)).filter(|x| *x != y).or_else(|| taken.iter().find(|n| **n != y).cloned())?;
        Some((x, y))
    }
    fn rebuild_table_bar(self: &Rc<Self>) {
        self.table_bar.remove_all();
        let taken = self.shared_names.borrow().clone();
        let add = |text: &str, tip: &str, f: Box<dyn Fn(&Rc<Self>)>| {
            let b = gtk::Button::builder().label(text).tooltip_text(tip).focus_on_click(false).build();
            let weak = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                if let Some(s) = weak.upgrade() {
                    f(&s);
                }
            });
            self.table_bar.append(&b);
        };
        if !taken.is_empty() {
            let caption = gtk::Label::builder().label("Table").css_classes(["dim-label"]).build();
            self.table_bar.append(&caption);
        }
        for name in &taken {
            let n = name.clone();
            add(name, "This table column. For example: mean(X)", Box::new(move |s| s.press(&n)));
        }
        if taken.len() > 1 {
            type Pick = fn(&str, &str) -> String;
            let tools: [(&str, &str, Pick); 3] = [
                ("Correlation r", "Pearson's r of the charted columns", |x, y| format!("pearson({x}, {y})")),
                ("Slope", "Slope m of their best-fit line y = mx + b", |x, y| format!("slope({x}, {y})")),
                ("Intercept", "Intercept b of their best-fit line", |x, y| format!("intercept({x}, {y})")),
            ];
            for (text, tip, make) in tools {
                add(text, tip, Box::new(move |s| {
                    if let Some((x, y)) = s.table_pair() {
                        s.press(&make(&x, &y));
                    }
                }));
            }
        }
        let mut child = self.table_bar.first_child();
        while let Some(w) = child {
            w.set_focusable(false);
            child = w.next_sibling();
        }
        self.table_bar.set_visible(!taken.is_empty());
        let shared = self.shared.borrow();
        let mut unusable = shared.unnamed.clone();
        unusable.extend(shared.named.iter().map(|(n, _)| n).filter(|n| !taken.contains(n)).cloned());
        let mut notes = vec![];
        if !unusable.is_empty() {
            notes.push(format!("Not available here: {}. Name a column with one word that isn't a unit or constant.", unusable.join(", ")));
        }
        if shared.misaligned {
            notes.push("Blank cells are skipped, so the columns no longer line up row by row. Fill them before using r or the slope here.".to_string());
        }
        self.table_note.set_text(&notes.join("\n"));
    }
    fn history_card(self: &Rc<Self>, saved: &calc::Saved) {
        let body = column(0);
        let input = label(&saved.input);
        input.add_css_class("dim-label");
        body.append(&input);
        let answer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let result = label(&saved.result);
        result.add_css_class("simple-result");
        answer.append(&result);
        if !saved.approx.is_empty() {
            let approx = label(&format!("≈ {}", saved.approx));
            approx.add_css_class("dim-label");
            approx.set_valign(gtk::Align::Center);
            answer.append(&approx);
        }
        body.append(&answer);
        let button = gtk::Button::builder().child(&body).has_frame(false).css_classes(["simple-card"]).tooltip_text("Tap to use again").build();
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
        self.share_table(false);
        let chart = model::Chart::from_index(self.chart.selected());
        let paired = chart != model::Chart::Histogram;
        self.xcol.set_sensitive(paired);
        self.fit.set_sensitive(paired);
        let x = self.xcol.selected().checked_sub(1).map(|i| i as usize);
        let y = self.ycol.selected() as usize;
        let (mut fig, stats, skipped) = model::data_chart(&grid, chart, x, y);
        let points = if paired { model::pairs(&grid, x, y) } else { vec![] };
        let kind = if paired { model::FitKind::from_index(self.fit.selected()) } else { model::FitKind::None };
        let fitted = (kind != model::FitKind::None).then(|| model::fit(&points, kind));
        if let Some(Ok(f)) = &fitted {
            let lo = points.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
            let hi = points.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
            let xs: Vec<f64> = (0..=200).map(|i| lo + (hi - lo) * i as f64 / 200.0).collect();
            let ys = xs.iter().map(|x| f.at(*x)).collect();
            let mut curve = plot::Series::new(plot::Data::Line { x: xs, y: ys });
            curve.color = Some([205, 66, 64]);
            curve.width = 2.5;
            fig.axes().series.push(curve);
        }
        *self.data_board.figure.borrow_mut() = fig;
        self.data_board.redraw();
        let fmt = |v: Option<f64>| v.filter(|v| v.is_finite()).map(|v| plot::cursor::num(v).replace('-', "−"));
        let mut lines = vec![];
        let figures: Vec<String> = [("n", Some(stats.count as f64)), ("mean", stats.mean), ("median", stats.median), ("sd", stats.std), ("min", stats.min), ("max", stats.max)]
            .into_iter()
            .filter_map(|(name, v)| Some(format!("{name} = {}", fmt(v)?)))
            .collect();
        if stats.count > 0 {
            lines.push(figures.join("   "));
        }
        let r = model::pearson(&points);
        let shown = |name: &str, v: Option<f64>| fmt(v).map_or(String::new(), |v| format!("   {name} = {v}"));
        let name = model::FitKind::NAMES[self.fit.selected().min(5) as usize];
        match &fitted {
            Some(Ok(f)) if kind == model::FitKind::Linear => lines.push(format!("Best-fit line: {}{}{}", f.equation(), shown("r", r), shown("r²", f.r2))),
            Some(Ok(f)) => lines.push(format!("{name} fit: {}{}{}", f.equation(), shown("r²", f.r2), shown("Pearson r", r))),
            // a table still being filled isn't a problem for the default line
            Some(Err(e)) if points.len() >= 2 || (kind != model::FitKind::Linear && !points.is_empty()) => lines.push(format!("No {} fit: {e}", name.to_lowercase())),
            None if paired && r.is_some() => lines.push(format!("Pearson r = {}", fmt(r).unwrap_or_default())),
            _ => {}
        }
        let skipped = skipped.max(stats.skipped);
        if skipped > 0 {
            lines.push(format!("{skipped} row{} skipped: not a number", if skipped == 1 { "" } else { "s" }));
        }
        self.stats.set_text(&lines.join("\n"));
        self.show_more_stats(&stats, &points, self.ycol.selected());
    }
    /// The Expander's cards: the y column, then x and y together. They wrap to the pane's width.
    fn show_more_stats(&self, s: &model::Summary, points: &[(f64, f64)], ycol: u32) {
        let fmt = |v: Option<f64>| v.filter(|v| v.is_finite()).map_or("–".to_string(), |v| plot::cursor::num(v).replace('-', "−"));
        let section = |title: String, items: Vec<(&str, String)>| {
            let body = column(2);
            let t = label(&title);
            t.add_css_class("heading");
            t.set_margin_top(4);
            body.append(&t);
            let flow = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).min_children_per_line(2).max_children_per_line(8).homogeneous(true).build();
            for (name, value) in items {
                let card = column(0);
                card.add_css_class("simple-stat");
                let n = label(name);
                n.add_css_class("dim-label");
                n.add_css_class("caption");
                let v = label(&value);
                v.set_selectable(true);
                card.append(&n);
                card.append(&v);
                flow.append(&card);
                if let Some(child) = flow.last_child() {
                    child.set_focusable(false);
                }
            }
            body.append(&flow);
            body
        };
        let both = |a: Option<f64>, b: Option<f64>| a.zip(b);
        let modes = if s.modes.is_empty() { "none repeats".to_string() } else { s.modes.iter().map(|m| fmt(Some(*m))).collect::<Vec<_>>().join(", ") };
        let name = self.column_names.borrow().get(ycol as usize).cloned().unwrap_or_else(|| "y".into());
        let body = column(4);
        body.append(&section(format!("Column {name}"), vec![
            ("Count n", s.count.to_string()),
            ("Sum", fmt(s.sum)),
            ("Mean", fmt(s.mean)),
            ("Median", fmt(s.median)),
            ("Mode", modes),
            ("Minimum", fmt(s.min)),
            ("Q1", fmt(s.q1)),
            ("Q3", fmt(s.q3)),
            ("Maximum", fmt(s.max)),
            ("Range", fmt(both(s.max, s.min).map(|(a, b)| a - b))),
            ("IQR", fmt(both(s.q3, s.q1).map(|(a, b)| a - b))),
            ("SD, sample", fmt(s.std)),
            ("SD, population", fmt(s.pop_std)),
            ("Variance, sample", fmt(s.std.map(|v| v * v))),
            ("Variance, population", fmt(s.pop_std.map(|v| v * v))),
            ("Standard error", fmt(s.std.map(|v| v / (s.count as f64).sqrt()))),
        ]));
        if points.len() >= 2 {
            let r = model::pearson(points);
            let line = model::least_squares(points).ok();
            let n = points.len() as f64;
            let cov = model::covariance(points);
            body.append(&section("x and y together".to_string(), vec![
                ("Complete rows", points.len().to_string()),
                ("Pearson r", fmt(r)),
                ("r²", fmt(r.map(|r| r * r))),
                ("Spearman ρ", fmt(model::spearman(points))),
                ("Slope m", fmt(line.map(|l| l.0))),
                ("Intercept b", fmt(line.map(|l| l.1))),
                ("Covariance, sample", fmt(cov.map(|c| c * n / (n - 1.0)))),
                ("Covariance, population", fmt(cov)),
            ]));
        }
        let hint = label("In the Calculator: mean(X), pearson(X, Y), slope(X, Y)… see Functions.");
        hint.add_css_class("dim-label");
        hint.add_css_class("caption");
        hint.set_margin_top(4);
        body.append(&hint);
        self.more_stats.set_child(Some(&body));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_keypad_template_calculates() {
        let o = qalc::Options { angle: "deg".into(), ..Default::default() };
        for (page, _, keys) in PAGES {
            assert_eq!(keys.len(), 20, "{page}: a full 5 x 4 page");
            for (face, t) in keys.iter().filter(|(_, t)| t.contains(keypad::SLOT)) {
                // some functions want integers, inverse trig wants |a| <= 1, statistics want lists
                let ok = ["2", "1/2", "[1, 2, 3]"].iter().any(|v| qalc::classroom_eval(&t.replace(keypad::SLOT, v), &o).is_ok())
                    || qalc::classroom_eval(&t.replacen(keypad::SLOT, "[1, 2, 3]", 1).replace(keypad::SLOT, "1"), &o).is_ok();
                assert!(ok, "{page} key {face}: {t}");
            }
        }
        let cells: usize = NUMBERS.iter().map(|(_, a)| if *a == "@enter" { 2 } else { 1 }).sum();
        assert_eq!(cells, 24, "a full 6 x 4 number pad");
    }

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
        pump_until(|| ui.win.is_mapped());
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
        cfg.appearance = "dark";
        cfg.save_state(&path).unwrap();
        let app = gtk::Application::builder().application_id("com.dusty.qcalcp.classroomtest").flags(gio::ApplicationFlags::NON_UNIQUE).build();
        app.register(None::<&gio::Cancellable>).unwrap();
        let ui = super::super::build(&app, None);
        assert_eq!(gtk::Window::default_icon_name().as_deref(), Some("qcalcp"));
        assert!(gtk::IconTheme::for_display(&WidgetExt::display(&ui.win)).has_icon("qcalcp"), "embedded application icon is missing");
        let s = ui.simple.borrow().as_ref().unwrap().clone();
        assert!(ui.simple_mode.get());
        assert_eq!(ui.win.title().as_deref(), Some("EduCalc"));
        assert!(gtk::Settings::default().unwrap().is_gtk_application_prefer_dark_theme(), "Simple must follow the dark theme setting");
        assert_eq!(glib::application_name().as_deref(), Some("EduCalculator"));
        assert_eq!(s.tab.get(), SimpleTab::Calculator);
        assert_eq!(s.options.borrow().angle, "deg");
        assert!(s.keypad.is_visible());
        let advanced_options = qalc::options();
        let advanced_mode = ui.cfg.borrow().calc_mode;
        ui.win.unmaximize();
        ui.win.set_size_request(width, height);
        ui.win.set_default_size(width, height);
        settle();
        assert!(s.entry.parent().unwrap().next_sibling().is_none(), "the input must sit at the bottom of the calculator workspace");
        assert!(!s.entry.vexpands(), "moving the input must not stretch its height");
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
        assert!(s.message.is_visible());
        assert_eq!(s.entry.text(), "sin(");
        s.entry.set_text("");
        s.entry.set_text("1234");
        s.entry.select_region(1, 3);
        s.press("9");
        assert_eq!(s.entry.text(), "194");
        s.press("@back");
        assert_eq!(s.entry.text(), "14");
        s.entry.set_text("");
        s.functions.popup();
        settle();
        let pop = s.functions.popover().unwrap();
        let image = gtk::WidgetPaintable::new(Some(&pop));
        let snapshot = gtk::Snapshot::new();
        image.snapshot(&snapshot, pop.width() as f64, pop.height() as f64);
        // visual QA only: the popover may not be mapped yet on a slow display
        if let (Some(r), Some(node)) = (pop.native().and_then(|n| n.renderer()), snapshot.to_node()) {
            r.render_texture(&node, None).save_to_png("target/simple-functions.png").unwrap();
        }
        s.functions.popdown();
        s.press("log(□, □)");
        assert_eq!(s.entry.text(), "log(□, □)");
        s.entry.set_text("");
        screenshot(&ui, "simple-calculator-1366");
        assert!(!s.preview.is_visible());
        assert!(!s.message.is_visible());

        s.select(SimpleTab::Graph);
        settle();
        assert!(s.side_keys.is_visible() && s.function_keys.is_visible());
        let pages: Vec<gtk::ToggleButton> = std::iter::successors(s.side_keys.first_child(), |w| w.next_sibling()).filter_map(|w| w.downcast().ok()).collect();
        pages[2].set_active(true);
        assert_eq!(s.function_keys.visible_child_name().as_deref(), Some("trig"));
        assert!(!pages[0].is_active(), "one function page at a time");
        pages[0].set_active(true);
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
        assert_eq!(first.error.text(), "“sin(” is missing a closing “)”.");
        screenshot(&ui, "simple-graph-error");
        assert_eq!(s.graph.figure.borrow().axes[0].series.len(), 1);
        first.entry.set_text("1/x");
        pump_until(|| first.error.text().is_empty() && s.graph.figure.borrow().axes[0].series.len() == 2);
        screenshot(&ui, "simple-graph-1366");
        assert!(ui.interp.borrow().plots.figures.is_empty(), "classroom graph changed Advanced figures");

        s.select(SimpleTab::Data);
        settle();
        assert!(!s.side_keys.is_visible() && !s.function_keys.is_visible(), "the table takes only the number pad");
        let icons = gtk::IconTheme::for_display(&WidgetExt::display(&ui.win));
        for name in ["row-insert-symbolic", "row-delete-symbolic", "column-insert-symbolic", "column-delete-symbolic"] {
            assert!(icons.has_icon(name), "embedded {name} is missing");
        }
        let chart_height = s.data_board.area.height();
        s.toggle_keypad();
        pump_until(|| s.data_board.area.height() > chart_height);
        s.toggle_keypad();
        s.select(SimpleTab::Calculator);
        assert!(s.function_keys.is_visible());
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
            assert!(s.stats.text().contains("mean = 2"), "{}", s.stats.text());
            assert!(s.stats.text().contains("sd = 1"));
            assert_eq!(s.stats.text().contains("Best-fit line: y = 1x + 0"), i != 3, "{}", s.stats.text());
            if i == 0 {
                screenshot(&ui, "simple-data-regression-1366");
            }
        }
        screenshot(&ui, "simple-data-1366");
        assert!(s.data_board.figure.borrow().axes[0].origin_axes);
        s.data.root.activate_action("ed.del-row", None).unwrap();
        settle();
        assert!(s.stats.text().contains("mean = 1.5"));
        // the row / column buttons, as a finger presses them
        let button = |act: &str| {
            let mut stack = vec![s.data.root.clone().upcast::<gtk::Widget>()];
            while let Some(w) = stack.pop() {
                if let Some(b) = w.downcast_ref::<gtk::Button>().filter(|b| b.action_name().as_deref() == Some(act)) {
                    return b.clone();
                }
                stack.extend(std::iter::successors(w.first_child(), |c| c.next_sibling()));
            }
            panic!("no {act} button");
        };
        let rows = s.data.grid().len();
        button("ed.add-row").emit_clicked();
        settle();
        assert_eq!(s.data.grid().len(), rows + 1);
        button("ed.del-row").emit_clicked();
        settle();
        assert_eq!(s.data.grid().len(), rows);
        button("ed.add-col").emit_clicked();
        settle();
        assert_eq!(s.data.grid()[0].len(), 3);
        button("ed.del-col").emit_clicked();
        settle();
        assert_eq!(s.data.grid()[0].len(), 2);
        assert!(s.stats.text().contains("mean = 1.5"), "blank rows and columns came and went: {}", s.stats.text());

        // one table on both sides: Advanced gets it as `data` plus X, and Octave's changes come back
        super::super::show_simple(&ui, false);
        settle();
        let data = |ui: &Ui| match ui.interp.borrow().vars.get("data") {
            Some(crate::value::Value::Num(m)) => m.data.clone(),
            _ => vec![],
        };
        assert_eq!(data(&ui), [1.0, 2.0]);
        assert!(ui.interp.borrow().vars.contains_key("X") && !ui.interp.borrow().vars.contains_key("Y"));
        super::super::eval_octave(&ui, "data = [4 5; 6 7];").unwrap();
        super::super::show_simple(&ui, true);
        settle();
        let grid = s.data.grid();
        assert_eq!((&grid[0][..2], &grid[1][..2], &grid[2][..2]), (&["X", "Y"].map(String::from)[..], &["4", "5"].map(String::from)[..], &["6", "7"].map(String::from)[..]));
        assert!(s.stats.text().contains("n = 2"), "{}", s.stats.text());
        // one set of numbers, read from both tabs
        super::super::show_simple(&ui, false);
        settle();
        super::super::eval_octave(&ui, "data = [1 2.1; 2 3.9; 3 6.2; 4 7.8; 5 10.1; 6 12.2];").unwrap();
        super::super::show_simple(&ui, true);
        settle();
        s.chart.set_selected(0);
        s.xcol.set_selected(1);
        s.ycol.set_selected(1);
        settle();
        let points = model::pairs(&s.data.grid(), Some(0), 1);
        assert_eq!(points.len(), 6);
        let r = model::pearson(&points).unwrap();
        assert!(s.stats.text().contains("Best-fit line: y = 2.02"), "{}", s.stats.text());
        assert!(s.stats.text().contains(&format!("r = {}", plot::cursor::num(r))), "{}", s.stats.text());
        s.more_stats.set_expanded(true);
        settle();
        screenshot(&ui, "simple-data-statistics");
        s.more_stats.set_expanded(false);
        s.fit.set_selected(2);
        settle();
        assert!(s.stats.text().contains("Quadratic fit: y = "), "{}", s.stats.text());
        assert_eq!(s.data_board.figure.borrow().axes[0].series.len(), 2, "points and the curve");
        s.fit.set_selected(0);
        settle();
        assert!(s.stats.text().contains("Pearson r = "), "{}", s.stats.text());
        assert_eq!(s.data_board.figure.borrow().axes[0].series.len(), 1);
        s.fit.set_selected(1);
        settle();
        s.select(SimpleTab::Calculator);
        pump_until(|| *s.shared_names.borrow() == ["X", "Y"]);
        assert!(s.table_bar.is_visible() && !s.table_note.is_visible());
        // the strip's r button, then the same value as the table's
        let tool = std::iter::successors(s.table_bar.first_child(), |w| w.next_sibling())
            .filter_map(|c| c.first_child().and_downcast::<gtk::Button>())
            .find(|b| b.label().as_deref() == Some("Correlation r"))
            .expect("the strip's r button");
        tool.emit_clicked();
        assert_eq!(s.entry.text(), "pearson(X, Y)");
        settle();
        screenshot(&ui, "simple-calculator-table");
        s.press("@enter");
        pump_until(|| *s.last_expr.borrow() == "pearson(X, Y)");
        let number = s.last_number.borrow().parse::<f64>().unwrap();
        assert!((number - r).abs() < 1e-8, "{number} vs {r}");
        s.select(SimpleTab::Data);
        settle();
        s.data.root.activate_action("ed.del-col", None).unwrap();
        settle();
        assert_eq!(s.data.grid()[0].len(), 1);
        let table = s.data.grid();
        super::super::show_simple(&ui, false);
        settle();
        assert!(ui.win.title().unwrap().ends_with(" - EduCalc"));
        app.activate_action("appearance", Some(&"light".to_variant()));
        assert!(!ui.cfg.borrow().dark && !gtk::Settings::default().unwrap().is_gtk_application_prefer_dark_theme());
        app.activate_action("appearance", Some(&"system".to_variant()));
        assert_eq!(ui.cfg.borrow().appearance, "system");
        eprintln!("system appearance dark: {}", ui.cfg.borrow().dark);
        app.activate_action("appearance", Some(&"light".to_variant()));
        assert_eq!(qalc::options(), advanced_options);
        assert_eq!(ui.cfg.borrow().calc_mode, advanced_mode);
        super::super::show_simple(&ui, true);
        settle();
        screenshot(&ui, "simple-data-light");
        assert_eq!(s.data.grid(), table);
        // Ans puts the calculator's last answer into the selected cell
        *s.last_number.borrow_mut() = "1.25".into();
        s.select(SimpleTab::Data);
        settle();
        s.press("@ans");
        settle();
        assert!(s.data.grid().iter().flatten().any(|c| c.contains("1.25")), "{:?}", s.data.grid());
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
