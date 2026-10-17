//! GTK4 frontend, two modes over one interpreter and plot session (header toggle, F2):
//!   Calculator (default): math entry (libqalculate syntax) with the result as you type, answer
//!     cards (tap one to reuse its input; chips switch to other forms, show the Octave code and send
//!     it to Octave, Visualize plots it), an on-screen keypad. The plot pane shows once a figure exists.
//!   Octave: script editor, console + REPL, workspace (math-view tooltips; numeric variables have a
//!     Data Editor button), interactive plot canvas.
//!     "→ Calculator" (REPL/console/editor context menu, Ctrl+M) moves the selection or current
//!     line to the calculator in math view. ans and numeric variables are shared both ways.
//! Header (icons, tooltips): Calc | Octave, Run (Octave mode); keypad menu (modes, number pad),
//! calculator options (angle, approximation, precision, fractions, notation, result base, two's
//! complement, bits; `set KEY VALUE` in the entry changes them too), ☰ for everything else.
//!
//! Completion (calculator entry, REPL, script editor; complete.rs): typing an identifier lists
//! matches with a signature and a short description; Up/Down, Tab or Return accepts, Esc closes.
//!
//! Keypad: modes General / Programming / Algebra / Custom (keypad.ini groups) with an optional
//! number pad beside them; F9 hides it and brings the last mode back. □ marks a slot. A key's
//! template wraps the selection into its first □ and selects the next one; Tab / Shift+Tab in the
//! calculator entry jump between □, Enter refuses (and highlights) leftover ones. Programming's
//! BIN/OCT/DEC/HEX set the input and result base (A-F need HEX) and show hex/dec/oct/bin of the
//! result under the entry. In Octave mode keys go to the REPL or script, whichever had focus last.
//!
//! Customizing (files in ~/.config/qcalcp, created with documented defaults on first run):
//!   qcalcp.ini
//!     [ui]     mode=calc|octave (remembered), keypad_mode=general|programming|algebra|custom|
//!              numpad|none, number_pad, keypad_octave
//!     [calc]   auto_visual (run an answer's visualization after Enter), keypad_side=auto|bottom|right,
//!              angle approx precision fraction notation base input_base twos_complement bits
//!     [layout] orientation=horizontal|vertical  side=plot-right|plot-left (plot-left = plot first,
//!              i.e. on top when vertical)  split, editor_split (px)
//!              show_editor, show_console, show_workspace
//!     [window] width, height, maximized     ([ui], [layout], [window], [calc] but keypad_side, and
//!              dark are saved on exit)
//!     [theme]  dark, plot_bg, plot_fg, plot_grid (CSS colors), palette=#..;#..;..., font,
//!              font_size (plot), editor_font (Pango, e.g. "Monospace 11")
//!     [input]  rotate_speed (rad per dragged px), zoom_step (per wheel notch),
//!              hover_cursor (readout under the mouse), pin_radius (px reach for picks and unpinning)
//!     [keys]   <action>=<GTK accel>[;<accel>...], empty = unbound. Actions: run mode keypad
//!              number_pad data to_calc auto_visual new open save save_as export reset_view trace
//!              clear_pins toggle_layout swap_sides toggle_editor toggle_console toggle_workspace
//!              dark fullscreen quit calc_twos
//!   keypad.ini one [group] per page: columns=N, keys=label|template;... (see the file's comments);
//!              General, Programming, Algebra, Number pad by name, any other group is a Custom page
//!   history.txt calculator history, one answer per line: input, math view, result, approximation
//!              (tab-separated, last 500)
//!   style.css  user CSS at USER priority, live-reloaded. Names: window.qcalcp, headerbar, .editor,
//!              .console, .repl, .workspace, .plot-bar, .figure-selector, .plot-status, .plot,
//!              paned > separator; calculator: .calc, .calc-entry, .calc-preview, .calc-history,
//!              .calc-bases, .calc-card, .calc-input, .calc-result, .calc-approx, .calc-message,
//!              .calc-chip; completion: popover.completion, .completion row, .completion .desc;
//!              keypad: .keypad, .keypad button, .keypad .op (operators, actions), .keypad .fn
//!              (function templates), .keypad .general / .programming / .number-pad / ... (group)
//!
//! Canvas: drag pans (2D) / rotates (3D), pinch or wheel zooms, two-finger twist turns 3D,
//! double-click or long-press resets the axes under the pointer. Data cursor: hovering shows the
//! nearest point (x, y, z and Δ to the axes' last pin, mirrored in the plot bar); a click or tap
//! pins it, clicking a pin removes it. Trace mode (Trace button): dragging follows one series,
//! with the canvas focused Left/Right step, Up/Down switch series, Enter pins.
mod calc;
mod canvas;
mod complete;
mod config;
mod data;
mod keypad;
mod icon;
mod resize;
mod simple;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};

use crate::complete::Lang;
use crate::lang::Interp;
use crate::qalc;
use crate::value::{Func, Value};
use complete::Popup;
use config::Config;

const BASE_CSS: &str = "
headerbar button, .plot-bar button { min-height: 36px; min-width: 36px; }
.repl { min-height: 36px; }
.calc-entry { font-size: 1.5em; min-height: 52px; }
.calc-preview { font-size: 1.2em; }
.calc-history { background: none; }
.calc-card { padding: 8px 6px; }
.calc-input { font-size: 0.9em; }
.calc-result { font-size: 1.6em; }
.calc-message { color: #e5a50a; }
.calc-chip { min-height: 32px; padding: 0 12px; }
.keypad { padding: 4px; }
.keypad button { min-height: 44px; min-width: 40px; padding-left: 4px; padding-right: 4px; font-size: 1.15em; }
.keypad .op { font-weight: bold; }
.keypad-height-grip { min-height: 22px; background: alpha(#718096, 0.14); font-size: 12px; }
.keypad-height-grip:hover, .keypad-height-grip:focus { background: alpha(#718096, 0.3); }
.calc-bases { font-size: 0.9em; }
.completion row { padding: 2px 6px; }
.completion .desc { font-size: 0.85em; }
";

/// Calculator options with radio items: [calc] key, menu title, (label, value).
const OPTIONS: [(&str, &str, &[(&str, &str)]); 6] = [
    ("angle", "Angle", &[("Radians", "rad"), ("Degrees", "deg"), ("Gradians", "gra")]),
    ("approx", "Approximation", &[("Exact", "exact"), ("Try Exact", "try"), ("Approximate", "approx")]),
    ("fraction", "Fractions", &[("Auto", "auto"), ("Decimal", "decimal"), ("Fraction", "fraction"), ("Mixed", "mixed")]),
    ("notation", "Notation", &[("Auto", "auto"), ("Normal", "normal"), ("Scientific", "sci"), ("Engineering", "eng")]),
    ("base", "Result Base", &[("Binary", "2"), ("Octal", "8"), ("Decimal", "10"), ("Hexadecimal", "16")]),
    ("bits", "Bits", &[("Auto", "0"), ("8", "8"), ("16", "16"), ("32", "32"), ("64", "64")]),
];

/// Text widgets, for keypad targets, "→ Calculator" sources and "Send to Octave".
#[derive(Clone, Copy, PartialEq)]
enum Focus {
    Calc,
    Repl,
    Editor,
    Console,
}

struct Ui {
    win: gtk::ApplicationWindow,
    shell: gtk::Stack,
    headers: gtk::Stack,
    simple: RefCell<Option<Rc<simple::Simple>>>,
    simple_mode: Cell<bool>,
    interp: RefCell<Interp>,
    cfg: RefCell<Config>,
    file: RefCell<Option<PathBuf>>,
    editor: gtk::TextView,
    console: gtk::TextView,
    repl: gtk::Entry,
    calc: calc::Calc,
    /// completion popups of the calculator entry, the REPL and the editor
    calc_pop: Popup,
    repl_pop: Popup,
    editor_pop: Popup,
    /// moved between the calculator view and the Octave side
    keypad: keypad::Keypad,
    /// options menu's precision
    precision: gtk::SpinButton,
    /// last focused text widget, and the last of REPL / editor
    focus: Cell<Focus>,
    octave_focus: Cell<Focus>,
    /// header mode switch: [octave, calc]
    mode_btns: [gtk::ToggleButton; 2],
    run_btn: gtk::Button,
    /// REPL lines and the browsing position (== len when not browsing)
    history: RefCell<(Vec<String>, usize)>,
    vars: gtk::ListBox,
    var_names: RefCell<Vec<String>>,
    figs: gtk::StringList,
    fig_select: gtk::DropDown,
    /// plots version the selector was last built for
    version: Cell<u64>,
    canvas: canvas::Canvas,
    main: gtk::Paned,
    /// work side: "calc" view or "octave" (work paned above the keypad slot)
    side: gtk::Stack,
    octave_side: gtk::Box,
    work: gtk::Paned,
    edit_paned: gtk::Paned,
    editor_box: gtk::ScrolledWindow,
    console_box: gtk::Box,
    workspace_box: gtk::Box,
    plot_side: gtk::Box,
    css_monitor: RefCell<Option<gio::FileMonitor>>,
}

impl Ui {
    fn print(&self, s: &str, error: bool) {
        if s.is_empty() {
            return;
        }
        let b = self.console.buffer();
        let mut end = b.end_iter();
        if error {
            b.insert_with_tags_by_name(&mut end, s, &["error"]);
        } else {
            b.insert(&mut end, s);
        }
        b.place_cursor(&b.end_iter());
        self.console.scroll_mark_onscreen(&b.get_insert());
        // the console is hidden in calculator mode: show problems there too
        if error && self.cfg.try_borrow().is_ok_and(|c| c.calc_mode) {
            calc::notice(self, s);
        }
    }
}

/// Open the GUI; with `script`, load it into the editor and run it.
pub fn run(script: Option<PathBuf>) -> ExitCode {
    let app = gtk::Application::builder()
        .application_id("com.dusty.qcalcp")
        // ponytail: one process per window, so `EduCalc gui x.m` always runs x.m; single-instance needs open() handling
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let script = RefCell::new(script);
    app.connect_activate(move |app| { build(app, script.take()); });
    app.run_with_args(&["EduCalc"]).into()
}

fn ini_path() -> PathBuf {
    crate::config_dir().join("qcalcp.ini")
}

fn build(app: &gtk::Application, script: Option<PathBuf>) -> Rc<Ui> {
    icon::install();
    let (mut cfg, warnings) = Config::load(&ini_path());
    let (opts, opt_warn) = qalc::Options::load_ini(&ini_path());
    qalc::set_options(&opts);
    cfg.calc_mode &= script.is_none(); // a script opens in Octave mode
    let (interp, msg) = crate::new_interp();
    let scroll = |child: &gtk::Widget| gtk::ScrolledWindow::builder().child(child).hexpand(true).vexpand(true).build();
    let paned = |o: gtk::Orientation| gtk::Paned::builder().orientation(o).wide_handle(true).shrink_start_child(false).shrink_end_child(false).build();

    // work side: [editor / console+repl] | workspace
    // add_css_class, not the css_classes builder: that would wipe defaults like .view / .background
    let editor = gtk::TextView::builder().monospace(true).left_margin(8).top_margin(6).build();
    editor.add_css_class("editor");
    let console = gtk::TextView::builder()
        .monospace(true)
        .editable(false)
        .cursor_visible(false)
        .wrap_mode(gtk::WrapMode::WordChar)
        .left_margin(8)
        .build();
    console.add_css_class("console");
    console.buffer().create_tag(Some("error"), &[("foreground", &"#e5534b")]);
    let repl = gtk::Entry::builder().placeholder_text(">> Octave code, Enter runs, Up/Down history").build();
    repl.add_css_class("repl");
    let editor_box = scroll(editor.upcast_ref());
    let console_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    console_box.append(&scroll(console.upcast_ref()));
    console_box.append(&repl);
    let edit_paned = paned(gtk::Orientation::Vertical);
    edit_paned.set_start_child(Some(&editor_box));
    edit_paned.set_end_child(Some(&console_box));
    edit_paned.set_position(cfg.editor_split);

    let vars = gtk::ListBox::new();
    let workspace_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).width_request(180).css_classes(["workspace"]).build();
    workspace_box.append(&gtk::Label::builder().label("Workspace").css_classes(["heading"]).margin_top(6).margin_bottom(6).build());
    workspace_box.append(&scroll(vars.upcast_ref()));
    let work = paned(gtk::Orientation::Horizontal);
    work.set_start_child(Some(&edit_paned));
    work.set_end_child(Some(&workspace_box));
    work.set_resize_end_child(false);
    let octave_side = gtk::Box::new(gtk::Orientation::Vertical, 4);
    octave_side.append(&work);
    // "→ Calculator" in the text widgets' context menus
    let extra = gio::Menu::new();
    extra.append(Some("→ Calculator"), Some("app.to-calc"));
    editor.set_extra_menu(Some(&extra));
    console.set_extra_menu(Some(&extra));
    repl.set_extra_menu(Some(&extra));

    let calc = calc::Calc::new();
    let side = gtk::Stack::new();
    side.add_named(&calc.root, Some("calc"));
    side.add_named(&octave_side, Some("octave"));

    // plot side: [figure selector, reset] above the canvas
    let figs = gtk::StringList::new(&[]);
    let fig_select = gtk::DropDown::builder().model(&figs).tooltip_text("Figure").build();
    fig_select.add_css_class("figure-selector");
    let reset = gtk::Button::builder().icon_name("zoom-original-symbolic").tooltip_text("Reset view (double-click / long-press on an axes)").action_name("app.reset-view").build();
    let trace = gtk::ToggleButton::builder()
        .icon_name("find-location-symbolic")
        .tooltip_text("Trace mode: drag follows a series; Left/Right step, Up/Down switch series, Enter or tap pins")
        .action_name("app.trace")
        .build();
    let canvas = canvas::Canvas::new();
    let bar = gtk::Box::builder().spacing(6).css_classes(["plot-bar", "toolbar"]).build();
    bar.append(&fig_select);
    bar.append(&reset);
    bar.append(&trace);
    bar.append(&canvas.tools);
    bar.append(&canvas.status);
    let plot_side = gtk::Box::new(gtk::Orientation::Vertical, 0);
    plot_side.append(&bar);
    plot_side.append(&canvas.area);

    let main = paned(gtk::Orientation::Horizontal);
    main.set_position(cfg.split);

    let header = gtk::HeaderBar::new();
    let calc_btn = gtk::ToggleButton::builder().icon_name("accessories-calculator-symbolic").tooltip_text("Calculator: math entry, keypad, answer history (F2 switches)").build();
    let octave_btn = gtk::ToggleButton::builder().icon_name("utilities-terminal-symbolic").tooltip_text("Octave: script editor, console, workspace (F2 switches)").group(&calc_btn).build();
    let modes = gtk::Box::builder().css_classes(["linked"]).build();
    modes.append(&calc_btn);
    modes.append(&octave_btn);
    header.pack_start(&modes);
    let run_btn = gtk::Button::builder().icon_name("media-playback-start-symbolic").tooltip_text("Run selection or whole script (Ctrl+Return, F5)").action_name("app.run").build();
    run_btn.add_css_class("suggested-action");
    header.pack_start(&run_btn);
    let precision = gtk::SpinButton::with_range(1.0, 100.0, 1.0);
    for (icon, tip, pop) in [
        ("open-menu-symbolic", "Menu", gtk::PopoverMenu::from_model(Some(&menu()))),
        ("preferences-system-symbolic", "Calculator options", options_menu(&precision)),
        ("input-keyboard-symbolic", "Keypad (F9 shows / hides)", gtk::PopoverMenu::from_model(Some(&keypad_menu()))),
    ] {
        let b = gtk::MenuButton::builder().icon_name(icon).popover(&pop).tooltip_text(tip).primary(tip == "Menu").has_frame(false).build();
        header.pack_end(&b);
    }

    let simple_button = gtk::Button::with_label("Simple");
    header.pack_start(&simple_button);
    let shell = gtk::Stack::builder().vhomogeneous(false).hhomogeneous(false).build();
    shell.add_named(&main, Some("advanced"));
    let headers = gtk::Stack::builder().vhomogeneous(false).hhomogeneous(false).build();
    headers.add_named(&header, Some("advanced"));
    let win = gtk::ApplicationWindow::builder()
        .application(app)
        .default_width(cfg.width)
        .default_height(cfg.height)
        .maximized(cfg.maximized)
        .titlebar(&headers)
        .child(&shell)
        .build();
    win.add_css_class("qcalcp");

    let dark = cfg.dark;
    let (pages, keypad_warn) = keypad::load(&crate::config_dir().join("keypad.ini"));
    let calc_pop = Popup::new(complete::Target::Entry(calc.entry.clone()), Lang::Math);
    let repl_pop = Popup::new(complete::Target::Entry(repl.clone()), Lang::Octave);
    let editor_pop = Popup::new(complete::Target::Text(editor.clone()), Lang::Octave);
    let ui = Rc::new(Ui {
        win,
        shell,
        headers,
        simple: RefCell::new(None),
        simple_mode: Cell::new(false),
        interp: RefCell::new(interp),
        cfg: RefCell::new(cfg),
        file: RefCell::new(None),
        editor,
        console,
        repl,
        calc,
        calc_pop,
        repl_pop,
        editor_pop,
        keypad: keypad::Keypad::new(&pages),
        precision,
        focus: Cell::new(Focus::Calc),
        octave_focus: Cell::new(Focus::Repl),
        mode_btns: [octave_btn, calc_btn],
        run_btn,
        history: RefCell::new((vec![], 0)),
        vars,
        var_names: RefCell::new(vec![]),
        figs,
        fig_select,
        version: Cell::new(u64::MAX),
        canvas,
        main,
        side,
        octave_side,
        work,
        edit_paned,
        editor_box,
        console_box,
        workspace_box,
        plot_side,
        css_monitor: RefCell::new(None),
    });

    let u = Rc::downgrade(&ui);
    simple_button.connect_clicked(move |_| if let Some(ui) = u.upgrade() { show_simple(&ui, true); });

    let u = ui.clone();
    ui.keypad.connect(move |act| key_press(&u, act));
    for (b, calc) in ui.mode_btns.iter().zip([false, true]) {
        let u = ui.clone();
        b.connect_toggled(move |b| {
            if b.is_active() && u.cfg.borrow().calc_mode != calc {
                set_mode(&u, calc);
            }
        });
    }
    for (w, f) in [(ui.calc.entry.upcast_ref::<gtk::Widget>(), Focus::Calc), (ui.repl.upcast_ref(), Focus::Repl), (ui.editor.upcast_ref(), Focus::Editor), (ui.console.upcast_ref(), Focus::Console)] {
        let fc = gtk::EventControllerFocus::new();
        let u = ui.clone();
        fc.connect_enter(move |_| {
            u.focus.set(f);
            if matches!(f, Focus::Repl | Focus::Editor) {
                u.octave_focus.set(f);
            }
        });
        w.add_controller(fc);
    }

    calc::attach(&ui); // history first, so startup warnings land below it
    complete::attach(&ui, |u| &u.calc_pop);
    complete::attach(&ui, |u| &u.repl_pop);
    complete::attach(&ui, |u| &u.editor_pop);
    actions(app, &ui);
    sync_options(&ui);
    apply_layout(&ui);
    load_css(&ui);
    set_dark(&ui, dark);
    canvas::attach(&ui);
    ui.print(&warnings, true);
    ui.print(&opt_warn, true);
    ui.print(&keypad_warn, true);
    ui.print(&msg, false);

    let u = ui.clone();
    let repl = ui.repl.clone();
    repl.connect_activate(move |e| {
        let line = e.text().to_string();
        e.set_text("");
        if line.trim().is_empty() {
            return;
        }
        {
            let (h, pos) = &mut *u.history.borrow_mut();
            if h.last() != Some(&line) {
                h.push(line.clone());
            }
            *pos = h.len();
        }
        u.print(&format!(">> {line}\n"), false);
        run_code(&u, &line);
    });
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let (u, r) = (ui.clone(), repl.clone());
    keys.connect_key_pressed(move |_, key, _, _| {
        let step = match key {
            _ if u.repl_pop.is_open() => return glib::Propagation::Proceed, // Up/Down pick a completion
            gdk::Key::Up => -1,
            gdk::Key::Down => 1,
            _ => return glib::Propagation::Proceed,
        };
        let (h, pos) = &mut *u.history.borrow_mut();
        *pos = (*pos as isize + step).clamp(0, h.len() as isize) as usize;
        u.repl_pop.quiet(|| r.set_text(h.get(*pos).map_or("", String::as_str)));
        r.set_position(-1);
        glib::Propagation::Stop
    });
    repl.add_controller(keys);

    let u = ui.clone();
    ui.vars.connect_row_activated(move |_, row| {
        let text = {
            let (names, ip) = (u.var_names.borrow(), u.interp.borrow());
            usize::try_from(row.index()).ok().and_then(|i| names.get(i)).and_then(|n| ip.vars.get(n).map(|v| crate::lang::display(n, v)))
        };
        u.print(&text.unwrap_or_default(), false);
    });

    let u = ui.clone();
    ui.fig_select.connect_selected_notify(move |d| {
        let i = d.selected() as usize;
        // try_borrow: sync() holds a borrow while rebuilding the list, so its own notifies are ignored here
        if let Ok(mut ip) = u.interp.try_borrow_mut() {
            if i < ip.plots.figures.len() && i != ip.plots.current {
                ip.plots.current = i;
                ip.plots.touch();
                u.canvas.area.queue_draw();
            }
        }
    });

    let u = ui.clone();
    ui.editor.buffer().connect_modified_changed(move |_| update_title(&u));
    let u = ui.clone();
    ui.win.connect_close_request(move |_| {
        if let Some(s) = u.simple.borrow().as_ref().filter(|s| s.data.is_dirty()) {
            let data = s.data.clone();
            let win = u.win.clone();
            let dialog = gtk::AlertDialog::builder().message("Close without exporting the data?")
                .detail("The classroom table has unsaved edits. Cancel to export it first.")
                .buttons(["Cancel", "Close"]).cancel_button(0).default_button(0).modal(true).build();
            dialog.choose(Some(&u.win), None::<&gio::Cancellable>, move |r| if r == Ok(1) { data.discard(); win.close(); });
            return glib::Propagation::Stop;
        }
        if u.editor.buffer().is_modified() {
            confirm_discard(&u, |u| {
                u.editor.buffer().set_modified(false);
                u.win.close();
            });
            return glib::Propagation::Stop;
        }
        save_state(&u);
        glib::Propagation::Proceed
    });

    update_title(&ui);
    sync(&ui);
    let simple = simple::Simple::new(&ui);
    ui.shell.add_named(&simple.root, Some("simple"));
    ui.headers.add_named(&simple.header, Some("simple"));
    *ui.simple.borrow_mut() = Some(simple);
    show_simple(&ui, script.is_none());
    ui.win.present();
    if ui.simple_mode.get() { ui.simple.borrow().as_ref().unwrap().focus(); } else { focus_mode(&ui); }
    if let Some(p) = script {
        if load_file(&ui, p) {
            let u = ui.clone();
            glib::idle_add_local_once(move || run_editor(&u));
        }
    }
    ui
}

fn menu() -> gio::Menu {
    let section = |items: &[(&str, &str)]| {
        let m = gio::Menu::new();
        for (label, action) in items {
            m.append(Some(label), Some(&format!("app.{action}")));
        }
        m
    };
    let menu = gio::Menu::new();
    menu.append_section(
        None,
        &section(&[("Calculator ⇄ Octave", "mode"), ("Keypad", "keypad"), ("Auto-Visualize", "auto-visual"), ("Octave → Calculator", "to-calc"), ("Data Editor…", "data")]),
    );
    menu.append_section(None, &section(&[("New", "new"), ("Open…", "open"), ("Save", "save"), ("Save As…", "save-as")]));
    menu.append_section(None, &section(&[("Export Plot…", "export"), ("Reset View", "reset-view"), ("Clear Pins", "clear-pins")]));
    menu.append_section(
        None,
        &section(&[("Flip Layout", "toggle-layout"), ("Swap Sides", "swap-sides"), ("Editor", "toggle-editor"), ("Console", "toggle-console"), ("Workspace", "toggle-workspace")]),
    );
    menu.append_section(None, &section(&[("Dark Theme", "dark"), ("Fullscreen", "fullscreen")]));
    menu.append_section(None, &section(&[("Quit", "quit")]));
    menu
}

fn keypad_menu() -> gio::Menu {
    let modes = gio::Menu::new();
    for (mode, label) in keypad::MODES.into_iter().chain([("none", "None")]) {
        modes.append(Some(label), Some(&format!("app.keypad-mode::{mode}")));
    }
    let menu = gio::Menu::new();
    menu.append_section(None, &modes);
    menu.append(Some("Always Show Number Pad"), Some("app.number-pad"));
    menu
}

/// Angle and approximation inline, the other choices in submenus, then two's complement and the
/// precision spin.
fn options_menu(precision: &gtk::SpinButton) -> gtk::PopoverMenu {
    let menu = gio::Menu::new();
    let more = gio::Menu::new();
    for (i, (key, title, choices)) in OPTIONS.iter().enumerate() {
        let m = gio::Menu::new();
        for (label, v) in *choices {
            m.append(Some(label), Some(&format!("app.calc-{key}::{v}")));
        }
        if i < 2 {
            menu.append_section(Some(title), &m);
        } else {
            more.append_submenu(Some(title), &m);
        }
    }
    more.append(Some("Two's Complement"), Some("app.calc-twos"));
    let spin = gio::MenuItem::new(None, None);
    spin.set_attribute_value("custom", Some(&"precision".to_variant()));
    more.append_item(&spin);
    menu.append_section(None, &more);
    let pop = gtk::PopoverMenu::from_model(Some(&menu));
    let row = gtk::Box::builder().spacing(12).build();
    row.append(&gtk::Label::builder().label("Precision").tooltip_text("Significant digits of approximate results").hexpand(true).xalign(0.0).build());
    row.append(precision);
    pop.add_child(&row, "precision");
    pop
}

fn action(app: &gtk::Application, ui: &Rc<Ui>, name: &str, f: impl Fn(&Rc<Ui>) + 'static) {
    let a = gio::SimpleAction::new(name, None);
    let ui = ui.clone();
    let name = name.to_string();
    a.connect_activate(move |_, _| {
        if ui.simple_mode.get() && !matches!(name.as_str(), "quit" | "fullscreen") {
            if name == "mode" { show_simple(&ui, false); return; }
            if let Some(s) = ui.simple.borrow().as_ref() {
                match name.as_str() {
                    "run" => s.run(),
                    "data" => s.select(simple::SimpleTab::Data),
                    "export" => s.export(),
                    "reset-view" => s.reset_view(),
                    _ => {}
                }
            }
            return;
        }
        f(&ui)
    });
    app.add_action(&a);
}

/// Boolean action; shows as a check item in the menu.
fn toggle(app: &gtk::Application, ui: &Rc<Ui>, name: &str, init: bool, f: impl Fn(&Rc<Ui>, bool) + 'static) {
    let a = gio::SimpleAction::new_stateful(name, None, &init.to_variant());
    let ui = ui.clone();
    let name = name.to_string();
    a.connect_activate(move |a, _| {
        if ui.simple_mode.get() && name != "fullscreen" {
            if name == "keypad" { if let Some(s) = ui.simple.borrow().as_ref() { s.toggle_keypad(); } }
            return;
        }
        let on = !a.state().and_then(|s| s.get::<bool>()).unwrap_or(false);
        a.set_state(&on.to_variant());
        f(&ui, on);
    });
    app.add_action(&a);
}

fn actions(app: &gtk::Application, ui: &Rc<Ui>) {
    action(app, ui, "run", |ui| if ui.cfg.borrow().calc_mode { calc::run(ui) } else { run_editor(ui) });
    action(app, ui, "mode", |ui| {
        let calc = ui.cfg.borrow().calc_mode;
        set_mode(ui, !calc);
    });
    action(app, ui, "to-calc", to_calc);
    action(app, ui, "new", |ui| {
        confirm_discard(ui, |ui| {
            ui.editor.buffer().set_text("");
            ui.editor.buffer().set_modified(false);
            *ui.file.borrow_mut() = None;
            update_title(ui);
        })
    });
    action(app, ui, "open", open);
    action(app, ui, "save", |ui| save(ui, false, |_| {}));
    action(app, ui, "save-as", |ui| save(ui, true, |_| {}));
    action(app, ui, "export", export);
    action(app, ui, "reset-view", |ui| canvas::for_all_axes(ui, canvas::reset));
    action(app, ui, "clear-pins", |ui| canvas::for_all_axes(ui, |a| a.pins.clear()));
    toggle(app, ui, "trace", false, |ui, on| canvas::set_trace(ui, on));
    action(app, ui, "toggle-layout", |ui| {
        let v = {
            let mut c = ui.cfg.borrow_mut();
            c.vertical = !c.vertical;
            c.vertical
        };
        apply_layout(ui);
        ui.main.set_position(if v { ui.main.height() } else { ui.main.width() } / 2);
    });
    action(app, ui, "swap-sides", |ui| {
        let flip = !ui.cfg.borrow().plot_left;
        ui.cfg.borrow_mut().plot_left = flip;
        apply_layout(ui);
        ui.main.set_position(ui.main.max_position() - ui.main.position());
    });
    let c = ui.cfg.borrow().clone();
    toggle(app, ui, "toggle-editor", c.show_editor, |ui, on| {
        ui.cfg.borrow_mut().show_editor = on;
        apply_layout(ui);
    });
    toggle(app, ui, "toggle-console", c.show_console, |ui, on| {
        ui.cfg.borrow_mut().show_console = on;
        apply_layout(ui);
    });
    toggle(app, ui, "toggle-workspace", c.show_workspace, |ui, on| {
        ui.cfg.borrow_mut().show_workspace = on;
        apply_layout(ui);
    });
    toggle(app, ui, "dark", c.dark, set_dark);
    // states are set by apply_layout
    toggle(app, ui, "keypad", false, |ui, on| {
        let last = ui.cfg.borrow().keypad_last;
        set_keypad(ui, if on { last } else { "none" });
    });
    let a = gio::SimpleAction::new_stateful("keypad-mode", Some(glib::VariantTy::STRING), &"none".to_variant());
    let u = ui.clone();
    a.connect_activate(move |_, v| {
        let v = v.and_then(|v| v.str().map(String::from)).unwrap_or_default();
        if let Some(mode) = keypad::MODES.iter().map(|m| m.0).chain(["none"]).find(|m| *m == v) {
            set_keypad(&u, mode);
        }
    });
    app.add_action(&a);
    toggle(app, ui, "number-pad", c.number_pad, |ui, on| {
        ui.cfg.borrow_mut().number_pad = on;
        apply_layout(ui);
    });
    toggle(app, ui, "auto-visual", c.auto_visual, |ui, on| ui.cfg.borrow_mut().auto_visual = on);
    action(app, ui, "fullscreen", |ui| {
        if ui.win.is_fullscreen() {
            ui.win.unfullscreen();
        } else {
            ui.win.fullscreen();
        }
    });
    action(app, ui, "data", |ui| data::open(ui, None));
    let a = gio::SimpleAction::new("edit-var", Some(glib::VariantTy::STRING));
    let u = ui.clone();
    a.connect_activate(move |_, v| {
        if let Some(name) = v.and_then(|v| v.str().map(String::from)) {
            data::open(&u, Some(&name));
        }
    });
    app.add_action(&a);
    // calculator options; states are set by sync_options
    for (key, _, _) in OPTIONS {
        let a = gio::SimpleAction::new_stateful(&format!("calc-{key}"), Some(glib::VariantTy::STRING), &"".to_variant());
        let u = ui.clone();
        a.connect_activate(move |_, v| {
            let mut o = qalc::options();
            match v.and_then(|v| v.str().map(String::from)).map(|v| o.set(key, &v)) {
                Some(Ok(())) => apply_options(&u, &o),
                Some(Err(e)) => u.print(&format!("{key}: {e}\n"), true),
                None => {}
            }
        });
        app.add_action(&a);
    }
    toggle(app, ui, "calc-twos", false, |ui, on| {
        let mut o = qalc::options();
        o.twos_complement = on;
        apply_options(ui, &o);
    });
    let u = ui.clone();
    ui.precision.connect_value_changed(move |s| {
        let mut o = qalc::options();
        let p = s.value_as_int().max(1) as u32;
        if o.precision != p {
            o.precision = p;
            apply_options(&u, &o);
        }
    });
    action(app, ui, "quit", |ui| ui.win.close());

    let mut warn = String::new();
    // first binding of an accelerator wins (an old ini's toggle_workspace=F9 vs keypad=F9)
    let mut taken: Vec<((gdk::Key, gdk::ModifierType), &str)> = vec![];
    for (key, accels) in &c.keys {
        let name = key.replace('_', "-");
        if !app.has_action(&name) {
            warn += &format!("qcalcp.ini: [keys] {key}: no such action\n");
            continue;
        }
        let mut good = vec![];
        for a in accels {
            match gtk::accelerator_parse(a) {
                None => warn += &format!("qcalcp.ini: [keys] {key}: bad accelerator {a}\n"),
                Some(k) => match taken.iter().find(|(t, _)| *t == k) {
                    Some((_, other)) => warn += &format!("qcalcp.ini: [keys] {key}: {a} is already {other}'s\n"),
                    None => {
                        taken.push((k, key));
                        good.push(a.as_str());
                    }
                },
            }
        }
        app.set_accels_for_action(&format!("app.{name}"), &good);
    }
    ui.print(&warn, true);
}

/// Mode, orientation, side order, panel and keypad placement/visibility from the config.
fn apply_layout(ui: &Ui) {
    let c = ui.cfg.borrow();
    ui.main.set_orientation(if c.vertical { gtk::Orientation::Vertical } else { gtk::Orientation::Horizontal });
    let (work, plot) = (ui.side.upcast_ref::<gtk::Widget>(), ui.plot_side.upcast_ref::<gtk::Widget>());
    let (a, b) = if c.plot_left { (plot, work) } else { (work, plot) };
    if ui.main.start_child().as_ref() != Some(a) {
        ui.main.set_start_child(None::<&gtk::Widget>);
        ui.main.set_end_child(None::<&gtk::Widget>);
        ui.main.set_start_child(Some(a));
        ui.main.set_end_child(Some(b));
    }
    ui.editor_box.set_visible(c.show_editor);
    ui.console_box.set_visible(c.show_console);
    ui.edit_paned.set_visible(c.show_editor || c.show_console);
    ui.workspace_box.set_visible(c.show_workspace);
    ui.work.set_visible(c.show_editor || c.show_console || c.show_workspace);

    let calc = c.calc_mode;
    ui.side.set_visible_child_name(if calc { "calc" } else { "octave" });
    ui.side.set_visible(calc || c.show_editor || c.show_console || c.show_workspace);
    ui.run_btn.set_visible(!calc);
    // keypad: under / beside the calculator entry, or under the Octave panels
    let mode = if c.keypad_mode == "none" { c.keypad_last } else { c.keypad_mode };
    let shown = if calc { c.keypad_mode != "none" } else { c.keypad_octave };
    let right = calc && match c.keypad_side {
        "right" => true,
        "bottom" => false,
        _ => ui.calc.wide.get(),
    };
    let slot = if calc { &ui.calc.body } else { &ui.octave_side };
    let pad = &ui.keypad.root;
    if pad.parent().as_ref() != Some(slot.upcast_ref()) {
        if let Some(p) = pad.parent().and_downcast::<gtk::Box>() {
            p.remove(pad);
        }
        slot.append(pad);
    }
    ui.calc.body.set_orientation(if right { gtk::Orientation::Horizontal } else { gtk::Orientation::Vertical });
    pad.set_vexpand(right);
    pad.set_valign(gtk::Align::Fill);
    pad.set_visible(shown);
    // number pad beside the mode's keys when the keypad spans the width, below them when it's to the right
    ui.keypad.show(mode, c.number_pad, !calc || !(right || ui.calc.narrow.get()));
    // the on-screen keypad stands in for the touch keyboard
    ui.calc.entry.set_input_hints(if calc && shown { gtk::InputHints::INHIBIT_OSK } else { gtk::InputHints::NONE });
    let figs = ui.interp.try_borrow().map_or(true, |ip| !ip.plots.figures.is_empty());
    ui.plot_side.set_visible(!calc || figs);
    drop(c);
    ui.mode_btns[calc as usize].set_active(true);
    set_state(ui, "keypad", shown.to_variant());
    set_state(ui, "keypad-mode", if shown { mode } else { "none" }.to_variant());
}

/// State of a stateful app action (menu radio / check items).
fn set_state(ui: &Ui, action: &str, v: glib::Variant) {
    if let Some(a) = ui.win.application().and_then(|a| a.lookup_action(action)).and_downcast::<gio::SimpleAction>() {
        a.set_state(&v);
    }
}

/// Keypad `mode` (keypad::MODES or "none"). In Octave mode "none" hides only the keypad there.
fn set_keypad(ui: &Rc<Ui>, mode: &'static str) {
    {
        let mut c = ui.cfg.borrow_mut();
        if c.calc_mode || mode != "none" {
            c.keypad_mode = mode;
        }
        if mode != "none" {
            c.keypad_last = mode;
        }
        if !c.calc_mode {
            c.keypad_octave = mode != "none";
        }
    }
    apply_layout(ui);
    calc::refresh(ui); // the bases readout comes and goes with Programming
}

/// Use `o` for calculations, show it in the menu and keypad, recompute the live result.
fn apply_options(ui: &Rc<Ui>, o: &qalc::Options) {
    qalc::set_options(o);
    sync_options(ui);
    calc::refresh(ui);
}

/// Options menu, precision and the keypad's base keys from the options in use.
fn sync_options(ui: &Ui) {
    let o = qalc::options();
    for (key, _, _) in OPTIONS {
        let v = match key {
            "angle" => o.angle.clone(),
            "approx" => o.approx.clone(),
            "fraction" => o.fraction.clone(),
            "notation" => o.notation.clone(),
            "base" => o.base.to_string(),
            _ => o.bits.to_string(),
        };
        set_state(ui, &format!("calc-{key}"), v.to_variant());
    }
    set_state(ui, "calc-twos", o.twos_complement.to_variant());
    ui.precision.set_value(o.precision as f64);
    ui.keypad.set_base(o.input_base);
}

fn set_mode(ui: &Rc<Ui>, calc: bool) {
    ui.cfg.borrow_mut().calc_mode = calc;
    apply_layout(ui);
    focus_mode(ui);
}

/// Focus the mode's main input: calculator entry, or the REPL / script, whichever was used last.
fn focus_mode(ui: &Ui) {
    if ui.cfg.borrow().calc_mode {
        ui.calc.entry.grab_focus_without_selecting();
    } else if ui.octave_focus.get() == Focus::Editor && ui.editor.is_visible() {
        ui.editor.grab_focus();
    } else {
        ui.repl.grab_focus_without_selecting();
    }
}

fn key_press(ui: &Rc<Ui>, act: &keypad::Act) {
    if let keypad::Act::Base(n) = act {
        let mut o = qalc::options();
        (o.base, o.input_base) = (*n, *n);
        return apply_options(ui, &o);
    }
    let calc = ui.cfg.borrow().calc_mode;
    let (t, pop) = if calc {
        ui.calc.entry.grab_focus_without_selecting();
        (keypad::Target::Entry(&ui.calc.entry, false), &ui.calc_pop)
    } else if ui.octave_focus.get() == Focus::Editor {
        ui.editor.grab_focus();
        (keypad::Target::Text(&ui.editor), &ui.editor_pop)
    } else {
        ui.repl.grab_focus_without_selecting();
        (keypad::Target::Entry(&ui.repl, true), &ui.repl_pop)
    };
    pop.quiet(|| keypad::press(t, act));
}

/// Answer's Octave code -> REPL entry, or the end of the script if the editor was used last.
fn send_to_octave(ui: &Rc<Ui>, code: &str) {
    set_mode(ui, false);
    // several lines (a figure's code) are a script: the one-line REPL would mangle them
    if code.contains('\n') && !ui.editor.is_visible() {
        ui.cfg.borrow_mut().show_editor = true;
        apply_layout(ui);
        if let Some(a) = ui.win.application().and_then(|a| a.lookup_action("toggle-editor")).and_downcast::<gio::SimpleAction>() {
            a.set_state(&true.to_variant()); // keep the menu's check in step
        }
    }
    if (ui.octave_focus.get() == Focus::Editor || code.contains('\n')) && ui.editor.is_visible() {
        let b = ui.editor.buffer();
        let mut end = b.end_iter();
        let nl = if end.starts_line() { "" } else { "\n" };
        b.insert(&mut end, &format!("{nl}{code}\n"));
        b.place_cursor(&b.end_iter());
        ui.editor.scroll_mark_onscreen(&b.get_insert());
    } else {
        ui.repl_pop.quiet(|| ui.repl.set_text(code));
        ui.repl.set_position(-1);
    }
}

/// "→ Calculator": the focused text widget's selection (or current line / REPL text) as math.
fn to_calc(ui: &Rc<Ui>) {
    let line = |v: &gtk::TextView| {
        let b = v.buffer();
        let (s, e) = b.selection_bounds().unwrap_or_else(|| {
            let mut s = b.iter_at_mark(&b.get_insert());
            s.set_line_offset(0);
            let mut e = s;
            if !e.ends_line() {
                e.forward_to_line_end();
            }
            (s, e)
        });
        b.text(&s, &e, false).to_string()
    };
    let entry = |e: &gtk::Entry| e.selection_bounds().map_or_else(|| e.text().to_string(), |(a, b)| e.chars(a, b).to_string());
    let text = match ui.focus.get() {
        Focus::Editor => line(&ui.editor),
        Focus::Console => line(&ui.console),
        Focus::Repl => entry(&ui.repl),
        Focus::Calc => entry(&ui.calc.entry),
    };
    let code = text.trim().trim_start_matches(">>").trim().trim_end_matches(';').trim();
    if code.is_empty() {
        return;
    }
    let math = crate::qalc::from_octave(code);
    set_mode(ui, true);
    ui.calc_pop.quiet(|| ui.calc.entry.set_text(&math));
    ui.calc.entry.set_position(-1);
}

fn set_dark(ui: &Rc<Ui>, on: bool) {
    ui.cfg.borrow_mut().dark = on;
    if let Some(s) = gtk::Settings::default() {
        s.set_gtk_application_prefer_dark_theme(on);
    }
    ui.canvas.invalidate();
}

/// Switch shells without changing the remembered Advanced mode, theme, or calculator options.
fn show_simple(ui: &Rc<Ui>, simple: bool) {
    if simple && !ui.simple_mode.get() && ui.win.is_mapped() {
        let mut cfg = ui.cfg.borrow_mut();
        if ui.plot_side.is_visible() { cfg.split = ui.main.position(); }
        cfg.editor_split = ui.edit_paned.position();
    }
    ui.simple_mode.set(simple);
    let name = if simple { "simple" } else { "advanced" };
    ui.shell.set_visible_child_name(name);
    ui.headers.set_visible_child_name(name);
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_application_prefer_dark_theme(!simple && ui.cfg.borrow().dark);
    }
    if simple {
        ui.win.set_title(Some("EduCalc — Simple"));
        if let Some(s) = ui.simple.borrow().as_ref() { s.focus(); }
    } else {
        update_title(ui); apply_layout(ui); focus_mode(ui);
    }
}

/// App CSS (touch sizes, editor font) plus the user's style.css, reloaded when it changes.
fn load_css(ui: &Rc<Ui>) {
    let display = WidgetExt::display(&ui.win);
    let base = gtk::CssProvider::new();
    base.load_from_string(&format!("{BASE_CSS}{}{}", font_css(&ui.cfg.borrow().editor_font), simple::CSS));
    gtk::style_context_add_provider_for_display(&display, &base, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);

    let user = gtk::CssProvider::new();
    gtk::style_context_add_provider_for_display(&display, &user, gtk::STYLE_PROVIDER_PRIORITY_USER);
    let u = ui.clone();
    user.connect_parsing_error(move |_, section, e| {
        u.print(&format!("style.css:{}: {e}\n", section.start_location().lines() + 1), true);
    });
    let path = crate::config_dir().join("style.css");
    let file = gio::File::for_path(&path);
    let reload = move |p: &gtk::CssProvider| p.load_from_string(&std::fs::read_to_string(&path).unwrap_or_default());
    reload(&user);
    match file.monitor_file(gio::FileMonitorFlags::NONE, None::<&gio::Cancellable>) {
        Ok(m) => {
            m.connect_changed(move |_, _, _, ev| {
                use gio::FileMonitorEvent::*;
                if matches!(ev, ChangesDoneHint | Created | Deleted | MovedIn | Renamed) {
                    reload(&user);
                }
            });
            *ui.css_monitor.borrow_mut() = Some(m);
        }
        Err(e) => ui.print(&format!("style.css: not watching: {e}\n"), true),
    }
}

/// Pango font description ("Monospace 11") -> CSS for the text widgets.
fn font_css(desc: &str) -> String {
    let d = gtk::pango::FontDescription::from_string(desc);
    let mut css = String::new();
    if let Some(f) = d.family().filter(|f| !f.is_empty()) {
        css += &format!("font-family: \"{f}\"; ");
    }
    if d.size() > 0 {
        let unit = if d.is_size_absolute() { "px" } else { "pt" };
        css += &format!("font-size: {}{unit};", d.size() as f64 / gtk::pango::SCALE as f64);
    }
    format!(".editor, .console, .repl {{ {css} }}\n")
}

fn run_editor(ui: &Rc<Ui>) {
    let b = ui.editor.buffer();
    let (sel, (s, e)) = match b.selection_bounds() {
        Some(r) => ("selection", r),
        None => ("script", b.bounds()),
    };
    let src = b.text(&s, &e, false);
    ui.print(&format!(">> % run {sel}\n"), false);
    run_code(ui, &src);
}

fn run_code(ui: &Ui, src: &str) {
    if let Err(e) = eval_octave(ui, src) {
        ui.print(&format!("error: {e}\n"), true);
    }
}

/// Run Octave code, its output to the console, then refresh workspace and plots.
// ponytail: eval runs on the UI thread, so a long script freezes the window; a worker thread
// needs a Send interpreter.
fn eval_octave(ui: &Ui, src: &str) -> Result<(), String> {
    let (r, out) = {
        let mut ip = ui.interp.borrow_mut();
        // a panicking builtin must not take the unsaved editor buffer down with it
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ip.eval(src)));
        (r, ip.take_output())
    };
    ui.print(&out, false);
    sync(ui);
    r.unwrap_or_else(|_| Err("internal error (panic), see stderr".into()))
}

/// Refresh the workspace list and, if plots changed, the figure selector and canvas.
fn sync(ui: &Ui) {
    let ip = ui.interp.borrow();
    let mut vars: Vec<_> = ip.vars.iter().collect();
    vars.sort_by_key(|(n, _)| n.as_str());
    ui.vars.remove_all();
    for (name, v) in &vars {
        let row = gtk::Box::builder().spacing(12).margin_start(8).margin_end(8).margin_top(6).margin_bottom(6).build();
        row.append(&gtk::Label::builder().label(name.as_str()).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).build());
        row.append(&gtk::Label::builder().label(v.summary()).css_classes(["dim-label"]).build());
        if matches!(v, Value::Num(_) | Value::Bool(_)) {
            let b = gtk::Button::builder().icon_name("x-office-spreadsheet-symbolic").tooltip_text("Edit in Data Editor").action_name("app.edit-var").action_target(&name.to_variant()).build();
            b.add_css_class("flat");
            row.append(&b);
        }
        // math view of strings and anonymous functions, computed on hover
        let src = match v {
            Value::Str(s) => Some(s.clone()),
            Value::Func(f) => match &**f {
                Func::Anon(l) => Some(l.src.clone()),
                Func::Named(_) => None,
            },
            _ => None,
        };
        if let Some(src) = src {
            row.set_has_tooltip(true);
            row.connect_query_tooltip(move |_, _, _, _, tip| {
                let math = crate::qalc::pretty(&src);
                tip.set_text(Some(&math));
                math != src
            });
        }
        ui.vars.append(&row);
    }
    *ui.var_names.borrow_mut() = vars.iter().map(|(n, _)| n.to_string()).collect();
    if ip.plots.version != ui.version.get() {
        ui.version.set(ip.plots.version);
        let names: Vec<String> = ip.plots.figures.iter().map(|f| format!("Figure {}", f.id)).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        ui.figs.splice(0, ui.figs.n_items(), &names);
        ui.fig_select.set_selected(ip.plots.current as u32);
        ui.canvas.area.queue_draw();
        // calculator mode shows the plot pane once there's something in it
        ui.plot_side.set_visible(!ui.cfg.borrow().calc_mode || !ip.plots.figures.is_empty());
    }
}

fn update_title(ui: &Ui) {
    if ui.simple_mode.get() {
        ui.win.set_title(Some("EduCalc — Simple"));
        return;
    }
    let name = ui.file.borrow().as_ref().and_then(|p| p.file_name()).map_or("untitled".into(), |n| n.to_string_lossy().into_owned());
    let mark = if ui.editor.buffer().is_modified() { "*" } else { "" };
    ui.win.set_title(Some(&format!("{mark}{name} — EduCalc")));
}

fn save_state(ui: &Ui) {
    let mut c = ui.cfg.borrow_mut();
    // a hidden plot pane leaves the divider at the end; keep the user's split
    if !ui.simple_mode.get() && ui.plot_side.is_visible() {
        c.split = ui.main.position();
    }
    if !ui.simple_mode.get() { c.editor_split = ui.edit_paned.position(); }
    (c.width, c.height) = ui.win.default_size();
    c.maximized = ui.win.is_maximized();
    if let Err(e) = c.save_state(&ini_path()).and_then(|_| qalc::options().save_ini(&ini_path())) {
        eprintln!("qcalcp.ini: {e}");
    }
}

/// Run `then` now if the editor is clean, else after the user picks Discard or Save.
fn confirm_discard(ui: &Rc<Ui>, then: impl FnOnce(&Rc<Ui>) + 'static) {
    if !ui.editor.buffer().is_modified() {
        return then(ui);
    }
    let d = gtk::AlertDialog::builder()
        .message("Save changes to the script?")
        .detail("Unsaved changes will be lost.")
        .buttons(["Cancel", "Discard", "Save"])
        .cancel_button(0)
        .default_button(2)
        .modal(true)
        .build();
    let u = ui.clone();
    d.choose(Some(&ui.win), None::<&gio::Cancellable>, move |r| match r {
        Ok(1) => then(&u),
        Ok(2) => save(&u, false, then),
        _ => {}
    });
}

fn open(ui: &Rc<Ui>) {
    confirm_discard(ui, |ui| {
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        let f = gtk::FileFilter::new();
        f.set_name(Some("Octave scripts (*.m)"));
        f.add_pattern("*.m");
        filters.append(&f);
        let all = gtk::FileFilter::new();
        all.set_name(Some("All files"));
        all.add_pattern("*");
        filters.append(&all);
        let d = gtk::FileDialog::builder().title("Open script").filters(&filters).modal(true).build();
        let u = ui.clone();
        d.open(Some(&ui.win), None::<&gio::Cancellable>, move |r| {
            if let Some(p) = r.ok().and_then(|f| f.path()) {
                load_file(&u, p);
            }
        });
    });
}

fn load_file(ui: &Ui, path: PathBuf) -> bool {
    match std::fs::read_to_string(&path) {
        Ok(s) => {
            let b = ui.editor.buffer();
            b.set_text(&s);
            b.place_cursor(&b.start_iter());
            b.set_modified(false);
            *ui.file.borrow_mut() = Some(path);
            update_title(ui);
            true
        }
        Err(e) => {
            ui.print(&format!("{}: {e}\n", path.display()), true);
            false
        }
    }
}

/// Save to the current file (or ask for one), then run `then` if it worked.
fn save(ui: &Rc<Ui>, ask: bool, then: impl FnOnce(&Rc<Ui>) + 'static) {
    let known = ui.file.borrow().clone();
    if let (false, Some(p)) = (ask, &known) {
        if write_file(ui, p.clone()) {
            then(ui);
        }
        return;
    }
    let d = gtk::FileDialog::builder().title("Save script").initial_name("untitled.m").modal(true).build();
    if let Some(p) = known {
        d.set_initial_file(Some(&gio::File::for_path(p)));
    }
    let u = ui.clone();
    d.save(Some(&ui.win), None::<&gio::Cancellable>, move |r| {
        if let Some(p) = r.ok().and_then(|f| f.path()) {
            if write_file(&u, p) {
                then(&u);
            }
        }
    });
}

fn write_file(ui: &Ui, path: PathBuf) -> bool {
    let b = ui.editor.buffer();
    match std::fs::write(&path, b.text(&b.start_iter(), &b.end_iter(), false).as_bytes()) {
        Ok(()) => {
            *ui.file.borrow_mut() = Some(path);
            b.set_modified(false);
            update_title(ui);
            true
        }
        Err(e) => {
            ui.print(&format!("{}: {e}\n", path.display()), true);
            false
        }
    }
}

/// Current figure -> PNG/SVG (by extension) at 1600x1200.
fn export(ui: &Rc<Ui>) {
    let id = {
        let ip = ui.interp.borrow();
        ip.plots.figures.get(ip.plots.current).map(|f| f.id)
    };
    let Some(id) = id else {
        return ui.print("export: no figure yet\n", true);
    };
    let d = gtk::FileDialog::builder().title("Export plot (.png or .svg)").initial_name(format!("figure{id}.png")).modal(true).build();
    let u = ui.clone();
    d.save(Some(&ui.win), None::<&gio::Cancellable>, move |r| {
        let Some(path) = r.ok().and_then(|f| f.path()) else { return };
        let res = {
            let (ip, cfg) = (u.interp.borrow(), u.cfg.borrow());
            match ip.plots.figures.get(ip.plots.current) {
                Some(fig) => crate::plot::render::save(fig, cfg.theme(), &path, 1600, 1200),
                None => Err("figure is gone".into()),
            }
        };
        match res {
            Ok(()) => u.print(&format!("saved {}\n", path.display()), false),
            Err(e) => u.print(&format!("export: {e}\n"), true),
        }
    });
}
