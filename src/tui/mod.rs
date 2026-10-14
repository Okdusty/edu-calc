//! Terminal frontend (ratatui): calculator/Octave REPL, workspace and a half-block plot pane.
mod calc;

use std::f64::consts::FRAC_PI_2;
use std::io::stdout;
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::time::Instant;

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event, KeyCode, KeyEvent,
    KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Widget};
use ratatui::{DefaultTerminal, Frame};

use crate::complete::{self, Completion};
use crate::lang::Interp;
use crate::plot::cursor::{self, Hit, Mode};
use crate::plot::render::Frame as PlotFrame;
use crate::plot::{Axes, Theme, View};
use crate::qalc::{self, Answer};

use calc::{Lang, Preview};

// most useful first, narrow terminals cut the tail; the mode's own keys go in front
const HELP_CALC: &str = " F5 Octave  ^O to Octave  F6 auto-plot ";
const HELP_OCTAVE: &str = " F5 calculator  ^K to math ";
const HELP_REPL: &str = " Tab plot  F2 vars  F3 layout  F4 theme  Alt+arrows split  ^Q quit  ^L clear  ^C cancel  \
                         PgUp/PgDn scroll  ↑↓ history";
const HELP_PLOT: &str = " arrows pan/rotate  +/- zoom  k key points  o code  t trace  p/Enter pin  P unpin all  r reset  \
                         [ ] figure  mouse hover/drag/scroll  Tab REPL  F4 theme  ^Q quit";
const HELP_MENU: &str = " Tab/Enter complete  ↑↓ choose  Esc close";
/// completion popup rows
const MENU: usize = 6;
/// marks the Octave-mode view echo in the log
const VIEW: &str = "% view: ";
/// mouse reach for the data cursor, pane pixels (1 cell = 1x2)
const REACH: f64 = 4.0;

/// Terminal UI in calculator mode; with `script`, run it first (in Octave mode) so its variables
/// and figures are ready.
pub fn run(script: Option<PathBuf>) -> std::io::Result<()> {
    // before touching the terminal: addons and rc.m may print
    let mut app = App::new();
    if let Some(p) = script {
        app.lang = Lang::Octave;
        match std::fs::read_to_string(&p) {
            Ok(src) => app.eval(&src),
            Err(e) => app.push(Kind::Err, &format!("{}: {e}", p.display())),
        }
    }
    let mut term = ratatui::try_init()?;
    // ratatui's hook restores raw mode + alt screen; we also turned on mouse capture and paste mode
    let hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let _ = execute!(stdout(), DisableMouseCapture, DisableBracketedPaste);
        hook(info);
    }));
    let r = execute!(stdout(), EnableMouseCapture, EnableBracketedPaste).and_then(|_| app.run(&mut term));
    let _ = execute!(stdout(), DisableMouseCapture, DisableBracketedPaste);
    ratatui::restore();
    r
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    In,
    Out,
    Err,
    Warn,
    Dim,
}

struct App {
    interp: Interp,
    log: Vec<(Kind, String)>,
    /// log rows scrolled up from the bottom
    scroll: usize,
    log_h: usize,
    input: Vec<char>,
    cursor: usize,
    /// lines of an unfinished block (if/for/function ...) waiting for its end
    pending: String,
    lang: Lang,
    /// last calculator answer, for ^O
    last: Option<Answer>,
    preview: Preview,
    /// run `Answer::visual` after each calculation
    auto_plot: bool,
    /// shared by both modes; each entry comes back in the mode it was entered in
    history: Vec<(Lang, String)>,
    hist: usize,
    show_ws: bool,
    plot_top: bool,
    /// plot pane share, percent
    split: u16,
    dark: bool,
    plot_focus: bool,
    /// last plot render and what it was rendered for: (plots.version, w, h, dark)
    pixels: Result<Vec<u8>, String>,
    pixels_key: Option<(u64, u16, u16, bool)>,
    /// where each axes landed in `pixels`
    frames: Vec<PlotFrame>,
    /// data cursor (mouse hover or trace) and trace mode; `cursor` is the input line's
    probe: Option<Hit>,
    trace: bool,
    /// plot pane interior on screen, for mouse hit-testing
    plot_rect: Rect,
    /// mouse drag: last cell, axes index and its size in pane pixels
    drag: Option<(u16, u16, (usize, f64, f64))>,
    /// completion popup over the input line, and where it was drawn (mouse hit-testing)
    menu: Option<Menu>,
    menu_rect: Rect,
    quit: bool,
}

/// Completions of the identifier before the cursor: where it starts (char offset), the matches,
/// the selected one.
struct Menu {
    start: usize,
    items: Vec<Completion>,
    sel: usize,
}

impl App {
    fn new() -> Self {
        let (interp, msg) = crate::new_interp();
        let mut app = App::with(interp, &msg);
        let (o, warn) = qalc::Options::load_ini(&crate::ini_path());
        qalc::set_options(&o);
        warn.lines().for_each(|l| app.push(Kind::Warn, l));
        app
    }

    fn with(interp: Interp, msg: &str) -> Self {
        let mut app = App {
            interp,
            log: vec![],
            scroll: 0,
            log_h: 1,
            input: vec![],
            cursor: 0,
            pending: String::new(),
            lang: Lang::Calc,
            last: None,
            preview: Preview::default(),
            auto_plot: true,
            history: vec![],
            hist: 0,
            show_ws: true,
            plot_top: false,
            split: 50,
            dark: true,
            plot_focus: false,
            pixels: Ok(vec![]),
            pixels_key: None,
            frames: vec![],
            probe: None,
            trace: false,
            plot_rect: Rect::default(),
            drag: None,
            menu: None,
            menu_rect: Rect::default(),
            quit: false,
        };
        for l in msg.lines() {
            app.push(Kind::Out, l);
        }
        app
    }

    fn run(&mut self, term: &mut DefaultTerminal) -> std::io::Result<()> {
        while !self.quit {
            term.draw(|f| self.draw(f))?;
            // wait for input, but only until the live preview is due
            let ready = match self.preview.wait(Instant::now()) {
                Some(w) => event::poll(w)?,
                None => true,
            };
            if ready {
                match event::read()? {
                    Event::Key(k) if k.kind != KeyEventKind::Release => self.on_key(k),
                    Event::Mouse(m) => self.on_mouse(m),
                    Event::Paste(s) => self.on_paste(&s),
                    _ => {}
                }
            }
            self.tick(Instant::now());
        }
        Ok(())
    }

    fn push(&mut self, kind: Kind, s: &str) {
        self.log.push((kind, s.replace('\t', "    ")));
        self.scroll = 0;
    }

    fn eval(&mut self, code: &str) {
        if code.trim().is_empty() {
            return;
        }
        let r = guarded(|| self.interp.eval(code));
        self.probe = None; // the data under it may be gone
        let out = self.interp.take_output();
        for l in out.lines() {
            self.push(Kind::Out, l);
        }
        if let Err(e) = r {
            self.push(Kind::Err, &format!("error: {e}"));
        }
    }

    /// Calculator: each line on its own. Octave: echo the input line(s), evaluate once no block is left open.
    fn submit(&mut self) {
        let text: String = self.input.drain(..).collect();
        self.cursor = 0;
        let entry = (self.lang, text.clone());
        if !text.trim().is_empty() && !text.contains('\n') && self.history.last() != Some(&entry) {
            self.history.push(entry);
        }
        self.hist = self.history.len();
        if self.lang == Lang::Calc {
            text.lines().for_each(|l| self.calc(l));
            return;
        }
        for l in text.split('\n') {
            let prompt = self.prompt();
            self.push(Kind::In, &format!("{prompt}{l}"));
            self.pending += l;
            self.pending.push('\n');
        }
        if block_depth(&self.pending) <= 0 {
            let code = std::mem::take(&mut self.pending);
            self.eval(&code);
        }
    }

    /// Calculator: non-default base/angle in front ("hex deg = ").
    fn prompt(&self) -> String {
        match self.lang {
            Lang::Calc => format!("{}= ", calc::opts_tag(&qalc::options())),
            Lang::Octave if self.pending.is_empty() => ">> ".into(),
            Lang::Octave => ".. ".into(),
        }
    }

    fn help(&self) -> String {
        if self.plot_focus {
            return HELP_PLOT.into();
        }
        if self.menu.is_some() {
            return HELP_MENU.into();
        }
        if let Some(h) = self.call_hint() {
            return h;
        }
        match self.lang {
            Lang::Calc => format!("{HELP_CALC}{HELP_REPL}"),
            Lang::Octave => format!("{HELP_OCTAVE}{HELP_REPL}"),
        }
    }

    /// " integrate(f, a, b, x = x)  Integrate " while the cursor is inside a call of a documented function.
    fn call_hint(&self) -> Option<String> {
        let name = call_at(&self.input, self.cursor)?;
        let (sig, desc) = match self.lang {
            Lang::Calc => {
                let c = qalc::catalog().iter().find(|c| c.kind == qalc::ItemKind::Function && c.name == name)?;
                (c.signature.as_str(), c.desc.as_str())
            }
            Lang::Octave => crate::doc::doc(&name).map(|d| (d.1, d.2))?,
        };
        Some(format!(" {sig}  {desc} "))
    }

    /// Open (or refresh) the completion popup for the identifier before the cursor.
    fn complete(&mut self) {
        let text: String = self.input.iter().collect();
        let lang = match self.lang {
            Lang::Calc => complete::Lang::Math,
            Lang::Octave => complete::Lang::Octave,
        };
        let mut vars: Vec<(String, String)> = self.interp.vars.iter().map(|(k, v)| (k.clone(), v.summary())).collect();
        vars.extend(self.interp.func_names().into_iter().map(|f| (f, "function".to_string())));
        let found = guarded(|| Ok(complete::complete(&text, self.cursor, lang, &vars, MENU))).ok().flatten();
        self.menu = found.filter(|(start, items)| *start <= self.cursor && !items.is_empty()).map(|(start, items)| Menu { start, items, sel: 0 });
    }

    /// Popup keys; true when the key was used. Enter on a match that's already typed out falls
    /// through and submits.
    fn menu_key(&mut self, code: KeyCode) -> bool {
        let Some(m) = &mut self.menu else { return false };
        let n = m.items.len();
        match code {
            KeyCode::Up => m.sel = (m.sel + n - 1) % n,
            KeyCode::Down => m.sel = (m.sel + 1) % n,
            KeyCode::Tab => {
                let i = m.sel;
                self.accept(i);
            }
            KeyCode::Enter => {
                let i = m.sel;
                return self.accept(i);
            }
            KeyCode::Esc => self.menu = None,
            _ => return false,
        }
        true
    }

    /// Replace the typed prefix with popup item `i`; false when that changes nothing.
    fn accept(&mut self, i: usize) -> bool {
        let Some(m) = self.menu.take() else { return false };
        let Some(c) = m.items.get(i) else { return false };
        let (text, at) = insertion(&c.insert, &c.name, self.input.get(self.cursor) == Some(&'('));
        if self.input[m.start..self.cursor].iter().copied().eq(text.chars()) {
            return false;
        }
        self.input.splice(m.start..self.cursor, text.chars());
        self.cursor = m.start + at;
        true
    }

    fn set_input(&mut self, s: &str) {
        self.input = s.chars().collect();
        self.cursor = self.input.len();
    }

    fn on_paste(&mut self, s: &str) {
        let s = s.replace("\r\n", "\n").replace('\r', "\n");
        let s = s.trim_end_matches('\n');
        self.menu = None;
        self.input.splice(self.cursor..self.cursor, s.chars());
        self.cursor += s.chars().count();
        if s.contains('\n') {
            self.plot_focus = false;
            self.submit();
        }
    }

    fn on_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        if !self.plot_focus && !ctrl && !alt && self.menu_key(k.code) {
            return;
        }
        let menu = self.menu.take().is_some();
        match k.code {
            KeyCode::Char('d' | 'q') if ctrl => self.quit = true,
            KeyCode::Char('l') if ctrl => {
                self.log.clear();
                self.scroll = 0;
            }
            KeyCode::F(2) => self.show_ws = !self.show_ws,
            KeyCode::F(3) => self.plot_top = !self.plot_top,
            KeyCode::F(4) => self.dark = !self.dark,
            KeyCode::F(5) => self.lang = self.lang.other(),
            KeyCode::F(6) => self.auto_plot = !self.auto_plot,
            KeyCode::Tab | KeyCode::BackTab => self.plot_focus = !self.plot_focus,
            KeyCode::PageUp => self.scroll += self.log_h.saturating_sub(1).max(1),
            KeyCode::PageDown => self.scroll = self.scroll.saturating_sub(self.log_h.saturating_sub(1).max(1)),
            // split = plot share; arrows move the divider (plot right: Left grows it, plot top: Down grows it)
            KeyCode::Left | KeyCode::Down if alt => self.split = (self.split + 5).min(90),
            KeyCode::Right | KeyCode::Up if alt => self.split = self.split.saturating_sub(5).max(10),
            _ if self.plot_focus => self.plot_key(k.code),
            _ => self.edit_key(k.code, ctrl),
        }
        // typing opens the popup and keeps filtering it; Backspace only narrows an open one
        let typed = matches!(k.code, KeyCode::Char(_)) && !ctrl || menu && k.code == KeyCode::Backspace;
        if typed && !self.plot_focus {
            self.complete();
        }
    }

    fn edit_key(&mut self, code: KeyCode, ctrl: bool) {
        let n = self.input.len();
        match code {
            KeyCode::Char('a') if ctrl => self.cursor = 0,
            KeyCode::Char('e') if ctrl => self.cursor = n,
            KeyCode::Char('u') if ctrl => {
                self.input.drain(..self.cursor);
                self.cursor = 0;
            }
            KeyCode::Char('w') if ctrl => {
                let mut i = self.cursor;
                while i > 0 && self.input[i - 1].is_whitespace() {
                    i -= 1;
                }
                while i > 0 && !self.input[i - 1].is_whitespace() {
                    i -= 1;
                }
                self.input.drain(i..self.cursor);
                self.cursor = i;
            }
            KeyCode::Char('c') if ctrl => self.cancel(),
            KeyCode::Char('o') if ctrl && self.lang == Lang::Calc => self.to_octave(),
            KeyCode::Char('k') if ctrl && self.lang == Lang::Octave => self.to_math(),
            KeyCode::Esc => self.cancel(),
            KeyCode::Char(_) if ctrl => {}
            KeyCode::Char(c) => {
                self.input.insert(self.cursor, c);
                self.cursor += 1;
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(n),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = n,
            KeyCode::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                self.input.remove(self.cursor);
            }
            KeyCode::Delete if self.cursor < n => {
                self.input.remove(self.cursor);
            }
            KeyCode::Up if self.hist > 0 => {
                self.hist -= 1;
                self.recall();
            }
            KeyCode::Down if self.hist < self.history.len() => {
                self.hist += 1;
                self.recall();
            }
            KeyCode::Enter => self.submit(),
            _ => {}
        }
    }

    /// History entry `hist` into the input line, back in the mode it was entered in.
    fn recall(&mut self) {
        match self.history.get(self.hist).cloned() {
            Some((lang, s)) => {
                self.lang = lang;
                self.set_input(&s);
            }
            // ponytail: past the newest entry gives an empty line, the unsent draft isn't kept
            None => self.set_input(""),
        }
    }

    /// Drop the input line and any half-typed block.
    fn cancel(&mut self) {
        self.input.clear();
        self.cursor = 0;
        if !std::mem::take(&mut self.pending).is_empty() {
            self.push(Kind::Err, "(block cancelled)");
        }
    }

    fn plot_key(&mut self, code: KeyCode) {
        let p = &self.interp.plots;
        let Some(ai) = p.figures.get(p.current).map(|f| f.current) else { return };
        match code {
            KeyCode::Char('t') => {
                self.trace = !self.trace;
                self.probe = if self.trace { self.probe } else { None };
                self.step(0, 0);
            }
            KeyCode::Left if self.trace => self.step(-1, 0),
            KeyCode::Right if self.trace => self.step(1, 0),
            KeyCode::Up if self.trace => self.step(0, 1),
            KeyCode::Down if self.trace => self.step(0, -1),
            KeyCode::Char('p') | KeyCode::Enter => {
                if let Some(h) = self.probe {
                    self.with_axes(h.axes, |a| a.pins.push(h.point));
                }
            }
            KeyCode::Char('P') => {
                let p = &mut self.interp.plots;
                for a in &mut p.figures[p.current].axes {
                    a.pins.clear();
                }
                p.touch();
            }
            KeyCode::Left => self.move_view(ai, |a| nudge(a, -0.1, 0.0)),
            KeyCode::Right => self.move_view(ai, |a| nudge(a, 0.1, 0.0)),
            KeyCode::Up => self.move_view(ai, |a| nudge(a, 0.0, 0.1)),
            KeyCode::Down => self.move_view(ai, |a| nudge(a, 0.0, -0.1)),
            KeyCode::Char('+' | '=') => self.move_view(ai, |a| zoom(a, 0.8)),
            KeyCode::Char('-' | '_') => self.move_view(ai, |a| zoom(a, 1.25)),
            KeyCode::Char('k') => self.with_axes(ai, |a| a.keypoints = !a.keypoints),
            KeyCode::Char('o') => self.figure_code(),
            KeyCode::Char('r') => {
                let p = &mut self.interp.plots;
                for a in &mut p.figures[p.current].axes {
                    (a.xlim, a.ylim, a.zlim, a.view) = (None, None, None, View::default());
                }
                p.touch();
            }
            KeyCode::Char(c @ ('[' | ']')) => {
                let p = &mut self.interp.plots;
                let n = p.figures.len();
                p.current = (p.current + if c == ']' { 1 } else { n - 1 }) % n;
                p.touch();
                self.probe = None;
            }
            KeyCode::Esc => self.plot_focus = false,
            _ => {}
        }
    }

    /// Trace cursor: `di` points along its series or `ds` series over; (0, 0) starts or snaps it.
    fn step(&mut self, di: isize, ds: isize) {
        if !self.trace {
            return;
        }
        let p = &self.interp.plots;
        if let Some(fig) = p.figures.get(p.current) {
            self.probe = cursor::step(fig, &self.frames, self.probe, di, ds);
        }
    }

    /// Mouse over pane cell (col, row): nearest point, or along the traced series.
    fn hover(&mut self, col: u16, row: u16) {
        let p = &self.interp.plots;
        let Some(fig) = p.figures.get(p.current) else { return };
        let (x, y) = pane_px(self.plot_rect, col, row);
        let mode = if self.trace { Mode::Trace { series: self.probe.map_or(0, |h| h.series) } } else { Mode::Nearest { radius: REACH } };
        let h = cursor::pick(fig, &self.frames, x, y, mode);
        if h.is_some() || !self.trace {
            self.probe = h;
        }
    }

    /// Pan/zoom axes `ai` with `f`, then let function curves follow the new view (2D) and, in Octave
    /// mode, echo the view as code: one log line, replaced while it's still the last.
    fn move_view(&mut self, ai: usize, f: impl FnOnce(&mut Axes)) {
        self.with_axes(ai, f);
        let p = &self.interp.plots;
        if p.figures.get(p.current).and_then(|fig| fig.axes.get(ai)).is_none_or(|a| a.is_3d()) {
            return;
        }
        if guarded(|| Ok(crate::plot::resample(&mut self.interp))) == Ok(true) {
            self.interp.plots.touch();
        }
        if self.lang != Lang::Octave {
            return;
        }
        let p = &self.interp.plots;
        let [(x0, x1), (y0, y1), _] = p.figures[p.current].axes[ai].bounds();
        let v = |x| crate::sig(x, 4);
        let line = format!("{VIEW}xlim([{} {}]); ylim([{} {}]);", v(x0), v(x1), v(y0), v(y1));
        if self.log.last().is_some_and(|(k, s)| *k == Kind::Dim && s.starts_with(VIEW)) {
            self.log.pop();
        }
        self.push(Kind::Dim, &line);
    }

    /// `o`: the current figure as Octave code into the log; in Octave mode also into the history (↑).
    fn figure_code(&mut self) {
        let p = &self.interp.plots;
        let code = p.figures.get(p.current).map(|f| f.octave()).unwrap_or_default();
        if code.trim().is_empty() {
            return self.push(Kind::Warn, "(no recorded code for this figure)");
        }
        code.lines().for_each(|l| self.push(Kind::Out, l));
        let entry = (Lang::Octave, code);
        if self.lang == Lang::Octave && self.history.last() != Some(&entry) {
            self.history.push(entry);
            self.hist = self.history.len();
        }
    }

    /// Apply `f` to axes `ai` of the current figure and mark the plots dirty.
    fn with_axes(&mut self, ai: usize, f: impl FnOnce(&mut Axes)) {
        let p = &mut self.interp.plots;
        if let Some(a) = p.figures.get_mut(p.current).and_then(|fig| fig.axes.get_mut(ai)) {
            f(a);
            p.touch();
        }
    }

    /// Axes under a screen cell of the plot pane, with that axes' size in pane pixels (1 cell = 1x2 px).
    fn hit(&self, col: u16, row: u16) -> Option<(usize, f64, f64)> {
        let r = self.plot_rect;
        if !r.contains(Position::new(col, row)) {
            return None;
        }
        let fig = self.interp.plots.figures.get(self.interp.plots.current)?;
        let (w, h) = (r.width as f64, r.height as f64 * 2.0);
        let (x, y) = pane_px(r, col, row);
        let ai = fig.axes_at(x, y, w, h)?;
        let (_, _, cw, ch) = fig.cell_rect(fig.axes[ai].cell, w, h);
        Some((ai, cw, ch))
    }

    fn on_mouse(&mut self, m: MouseEvent) {
        let (col, row) = (m.column, m.row);
        let in_plot = self.plot_rect.contains(Position::new(col, row));
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) if self.menu_rect.contains(Position::new(col, row)) => {
                self.accept((row - self.menu_rect.y) as usize);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                self.menu = None;
                self.plot_focus = in_plot;
                self.drag = self.hit(col, row).map(|h| (col, row, h));
            }
            MouseEventKind::Drag(MouseButton::Left) if self.trace => self.hover(col, row),
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some((c0, r0, (ai, cw, ch))) = self.drag {
                    // grab semantics: the content follows the pointer
                    let dx = -(col as f64 - c0 as f64) / cw;
                    let dy = (row as f64 - r0 as f64) * 2.0 / ch;
                    // ponytail: resamples on every drag event; only on release if big plots stutter
                    self.move_view(ai, |a| nudge(a, dx, dy));
                    self.drag = Some((col, row, (ai, cw, ch)));
                }
            }
            MouseEventKind::Up(_) => self.drag = None,
            MouseEventKind::Moved if in_plot => self.hover(col, row),
            MouseEventKind::Moved if !self.trace => self.probe = None,
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown if in_plot => {
                if let Some((ai, ..)) = self.hit(col, row) {
                    let k = if m.kind == MouseEventKind::ScrollUp { 0.9 } else { 1.0 / 0.9 };
                    self.move_view(ai, |a| zoom(a, k));
                }
            }
            MouseEventKind::ScrollUp => self.scroll += 3,
            MouseEventKind::ScrollDown => self.scroll = self.scroll.saturating_sub(3),
            _ => {}
        }
    }

    fn draw(&mut self, f: &mut Frame) {
        let [main, help] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(f.area());
        let (plot, rest) = if self.plot_top {
            let [p, r] = Layout::vertical([Constraint::Percentage(self.split), Constraint::Percentage(100 - self.split)]).areas(main);
            (p, r)
        } else {
            let [r, p] = Layout::horizontal([Constraint::Percentage(100 - self.split), Constraint::Percentage(self.split)]).areas(main);
            (p, r)
        };
        let repl = if self.show_ws {
            let [r, w] = Layout::horizontal([Constraint::Min(20), Constraint::Length((rest.width / 3).min(30))]).areas(rest);
            self.draw_ws(f, w);
            r
        } else {
            rest
        };
        self.draw_plot(f, plot);
        self.draw_repl(f, repl);
        f.render_widget(Line::from(self.help()).reversed(), help);
    }

    fn draw_plot(&mut self, f: &mut Frame, area: Rect) {
        let p = &self.interp.plots;
        let mut block = Block::bordered().title(if self.trace { " Plot · trace " } else { " Plot " }).border_style(border(self.plot_focus));
        if !p.figures.is_empty() {
            block = block.title(Line::from(format!(" figure {}/{} ", p.current + 1, p.figures.len())).right_aligned());
        }
        // cursor readout, Δ to the last pin of its axes
        let fig = p.figures.get(p.current);
        if let Some((h, fig)) = self.probe.zip(fig) {
            let (a, b) = cursor::readout(h.point, fig.axes.get(h.axes).map_or(&[][..], |a| &a.pins));
            block = block.title_bottom(Line::from(format!(" {a}{} ", b.map_or(String::new(), |b| format!("  {b}")))).left_aligned());
        }
        let inner = block.inner(area);
        f.render_widget(block, area);
        self.plot_rect = inner;
        let Some(fig) = p.figures.get(p.current) else {
            let ex = if self.lang == Lang::Calc { "plot(sin(x), 0, 10)" } else { "plot(sin(0:0.1:10))" };
            let hint = format!("\nno figures yet\n\ntry  {ex}\n\nTab focuses the plot");
            f.render_widget(Paragraph::new(hint).centered().dark_gray(), inner);
            return;
        };
        if inner.is_empty() {
            return;
        }
        let key = Some((p.version, inner.width, inner.height, self.dark));
        if self.pixels_key != key {
            let theme = if self.dark { Theme::dark() } else { Theme::light() };
            let (w, h) = (inner.width as u32, inner.height as u32 * 2);
            (self.pixels, self.frames) = match guarded(|| crate::plot::render::render_frames(fig, &theme, w, h)) {
                Ok((px, frames)) => (Ok(px), frames),
                Err(e) => (Err(e), vec![]),
            };
            self.pixels_key = key;
        }
        match &self.pixels {
            Ok(px) => f.render_widget(HalfBlocks { px, w: inner.width }, inner),
            Err(e) => f.render_widget(Paragraph::new(format!("render error: {e}")).red(), inner),
        }
        // cursor cell, from the data point so it stays on it while panning
        let at = self.probe.and_then(|h| Some(cursor::project(self.frames.iter().find(|f| f.axes == h.axes)?, h.point)));
        if let Some(c) = at.and_then(|(x, y)| cell_at(inner, x, y)).and_then(|p| f.buffer_mut().cell_mut(p)) {
            c.set_char('▀').set_fg(CURSOR).set_bg(CURSOR);
        }
    }

    fn draw_repl(&mut self, f: &mut Frame, area: Rect) {
        let title = match self.lang {
            Lang::Calc if !self.auto_plot => " Calculator · auto-plot off ",
            Lang::Calc => " Calculator ",
            Lang::Octave => " Octave ",
        };
        let block = Block::bordered().title(title).border_style(border(!self.plot_focus));
        let inner = block.inner(area);
        self.menu_rect = Rect::default();
        f.render_widget(block, area);
        if inner.is_empty() {
            return;
        }
        let preview_h = (self.lang == Lang::Calc) as u16;
        let [log_area, input_area, preview_area] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(1), Constraint::Length(preview_h)]).areas(inner);
        let (w, h) = (log_area.width.max(1) as usize, log_area.height as usize);
        self.log_h = h;
        // wrap bottom-up, only as far back as the view needs
        // ponytail: wraps by char count, wide (CJK) chars overflow; unicode-width if anyone cares
        let mut rows = vec![];
        'fill: for (kind, text) in self.log.iter().rev() {
            let style = match kind {
                Kind::In => Style::new().bold(),
                Kind::Out => Style::new(),
                Kind::Err => Style::new().red(),
                Kind::Warn => Style::new().yellow(),
                Kind::Dim => Style::new().dark_gray(),
            };
            let cs: Vec<char> = text.chars().collect();
            let chunks: Vec<&[char]> = if cs.is_empty() { vec![&[]] } else { cs.chunks(w).collect() };
            for c in chunks.into_iter().rev() {
                rows.push(Line::styled(c.iter().collect::<String>(), style));
                if rows.len() >= self.scroll + h {
                    break 'fill;
                }
            }
        }
        self.scroll = self.scroll.min(rows.len().saturating_sub(h));
        let view: Vec<Line> = rows.into_iter().skip(self.scroll).take(h).rev().collect();
        f.render_widget(Paragraph::new(view), log_area);

        let prompt = self.prompt();
        let avail = (input_area.width as usize).saturating_sub(prompt.len()).max(1);
        let start = self.cursor.saturating_sub(avail - 1);
        // multi-line input (a figure's code from the history) shows its line breaks as ⏎
        let shown: String = self.input[start..].iter().take(avail).map(|&c| if c == '\n' { '⏎' } else { c }).collect();
        let col = |i: usize| input_area.x + (prompt.len() + i.saturating_sub(start)) as u16;
        f.render_widget(Line::from(vec![Span::raw(prompt.as_str()).bold(), Span::raw(shown)]), input_area);
        if !self.plot_focus {
            f.set_cursor_position((col(self.cursor), input_area.y));
        }
        if let Some(p) = &self.preview.text {
            f.render_widget(Line::from(format!("{:w$}{p}", "", w = prompt.len())).dark_gray(), preview_area);
        }
        // completion popup: right above the input line, starting under the typed prefix
        let Some(m) = self.menu.as_ref().filter(|_| !self.plot_focus) else { return };
        let n = m.items.len().min(MENU).min(log_area.height as usize);
        let rows: Vec<(String, String)> = m.items[..n].iter().map(|c| (format!(" {} ", c.signature), format!(" {} ", c.desc))).collect();
        let w = rows.iter().map(|(s, d)| s.chars().count() + if d.trim().is_empty() { 0 } else { d.chars().count() }).max().unwrap_or(0) as u16;
        // it may spill over the panes to the right, like an IDE popup
        let right = f.area().right();
        let w = w.min(right - log_area.x);
        let r = Rect::new(col(m.start).min(right - w), log_area.bottom() - n as u16, w, n as u16);
        let lines: Vec<Line> = rows
            .into_iter()
            .enumerate()
            .map(|(i, (s, d))| match i == m.sel {
                true => Line::from(vec![Span::raw(s), Span::raw(d)]).black().on_cyan(),
                false => Line::from(vec![Span::raw(s).white(), Span::raw(d).gray()]),
            })
            .collect();
        f.render_widget(Clear, r);
        f.render_widget(Paragraph::new(lines).on_dark_gray(), r);
        self.menu_rect = r;
    }

    fn draw_ws(&self, f: &mut Frame, area: Rect) {
        let mut vars: Vec<_> = self.interp.vars.iter().collect();
        vars.sort_by(|a, b| a.0.cmp(b.0));
        let n = vars.iter().map(|v| v.0.chars().count()).max().unwrap_or(0).min(12);
        // ponytail: no scrolling, long workspaces are cut at the pane bottom
        let lines: Vec<Line> = vars
            .iter()
            .map(|(k, v)| Line::from(vec![Span::raw(format!("{k:<n$} ")).bold(), Span::raw(v.summary()).dark_gray()]))
            .collect();
        f.render_widget(Paragraph::new(lines).block(Block::bordered().title(" Workspace ").border_style(border(false))), area);
    }
}

const CURSOR: Color = Color::Rgb(230, 0, 230);

/// Center of pane cell (col, row) in pane pixels (1 cell = 1x2 px), the grid frames live on.
fn pane_px(r: Rect, col: u16, row: u16) -> (f64, f64) {
    (col.saturating_sub(r.x) as f64 + 0.5, row.saturating_sub(r.y) as f64 * 2.0 + 1.0)
}

/// Screen cell holding pane pixel (x, y), if inside the pane.
fn cell_at(r: Rect, x: f64, y: f64) -> Option<Position> {
    let (c, w) = ((x.floor(), (y / 2.0).floor()), (r.width as f64, r.height as f64));
    (c.0 >= 0.0 && c.1 >= 0.0 && c.0 < w.0 && c.1 < w.1).then(|| Position::new(r.x + c.0 as u16, r.y + c.1 as u16))
}

fn border(focused: bool) -> Style {
    Style::new().fg(if focused { Color::Cyan } else { Color::DarkGray })
}

/// Run `f`, turning a panic into an Err: a bug in a builtin or the renderer must not take
/// the session (and the user's workspace) down with it.
fn guarded<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let hook = panic::take_hook();
    panic::set_hook(Box::new(|_| {})); // the installed hook would restore the terminal and print over the UI
    let r = panic::catch_unwind(AssertUnwindSafe(f));
    panic::set_hook(hook);
    r.unwrap_or_else(|e| {
        let msg = e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()));
        Err(format!("internal error: {}", msg.unwrap_or_default()))
    })
}

/// RGB8 pixels `w` wide, two pixel rows per cell: `▀` with fg = upper pixel, bg = lower pixel.
struct HalfBlocks<'a> {
    px: &'a [u8],
    w: u16,
}

impl Widget for HalfBlocks<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let rgb = |x: u16, y: u16| {
            let i = (y as usize * self.w as usize + x as usize) * 3;
            self.px.get(i..i + 3).map_or(Color::Reset, |p| Color::Rgb(p[0], p[1], p[2]))
        };
        for y in 0..area.height {
            for x in 0..area.width.min(self.w) {
                if let Some(c) = buf.cell_mut((area.x + x, area.y + y)) {
                    c.set_char('▀').set_fg(rgb(x, 2 * y)).set_bg(rgb(x, 2 * y + 1));
                }
            }
        }
    }
}

/// Pan (2D, by fractions of the axis range) or rotate (3D, by fractions of 90°).
fn nudge(a: &mut Axes, dx: f64, dy: f64) {
    if a.is_3d() {
        a.view.yaw += dx * FRAC_PI_2;
        a.view.pitch = (a.view.pitch + dy * FRAC_PI_2).clamp(-FRAC_PI_2, FRAC_PI_2);
        return;
    }
    // ponytail: linear-space pan, log axes pan unevenly; do it in log10 space if that bothers anyone
    let [x, y, _] = a.bounds();
    if dx != 0.0 {
        a.xlim = Some(shift(x, dx));
    }
    if dy != 0.0 {
        a.ylim = Some(shift(y, dy));
    }
}

/// Zoom by `k` (< 1 = in): 2D scales the limits about their center, 3D the camera.
fn zoom(a: &mut Axes, k: f64) {
    if a.is_3d() {
        a.view.zoom = (a.view.zoom / k).clamp(0.05, 50.0);
        return;
    }
    let [x, y, _] = a.bounds();
    a.xlim = Some(scale(x, k));
    a.ylim = Some(scale(y, k));
}

fn shift((lo, hi): (f64, f64), frac: f64) -> (f64, f64) {
    let d = (hi - lo) * frac;
    sane((lo + d, hi + d), (lo, hi))
}

fn scale((lo, hi): (f64, f64), k: f64) -> (f64, f64) {
    let (c, r) = ((lo + hi) / 2.0, (hi - lo) / 2.0 * k);
    sane((c - r, c + r), (lo, hi))
}

/// New limits unless they collapsed (float precision) or overflowed.
fn sane(new: (f64, f64), old: (f64, f64)) -> (f64, f64) {
    if new.0 < new.1 && (new.1 - new.0).is_finite() { new } else { old }
}

/// Accepting a completion: the text for the typed prefix and the cursor offset in it. □ slots
/// go, the cursor lands on the first (`integrate(□, □, □)` -> `integrate(|)`, the signature
/// shows in the status line); with a `(` already after the cursor only the name goes in.
fn insertion(insert: &str, name: &str, paren_follows: bool) -> (String, usize) {
    if paren_follows {
        return (name.to_string(), name.chars().count());
    }
    match insert.split_once('□') {
        Some((head, tail)) => (format!("{head}{}", tail.replace("□, ", "").replace(", □", "").replace('□', "")), head.chars().count()),
        None => (insert.to_string(), insert.chars().count()),
    }
}

/// Name of the call whose parentheses hold the cursor: `integrate(x^2, 0|` -> integrate.
// ponytail: parentheses inside strings count too
fn call_at(input: &[char], cursor: usize) -> Option<String> {
    let mut depth = 0;
    let open = (0..cursor.min(input.len())).rev().find(|&i| {
        depth += match input[i] {
            ')' => 1,
            '(' => -1,
            _ => 0,
        };
        depth < 0
    })?;
    let start = input[..open].iter().rposition(|c| !(c.is_alphanumeric() || *c == '_')).map_or(0, |i| i + 1);
    (start < open).then(|| input[start..open].iter().collect())
}

/// Open-block count of Octave source: > 0 means an if/for/function/... still lacks its end.
/// Skips strings, comments and `end` inside brackets (`x(end)`).
fn block_depth(src: &str) -> i32 {
    const OPEN: &[&str] = &["if", "for", "parfor", "while", "function", "switch", "try", "do", "unwind_protect"];
    const CLOSE: &[&str] = &[
        "end", "endif", "endfor", "endparfor", "endwhile", "endfunction", "endswitch", "end_try_catch",
        "end_unwind_protect", "until",
    ];
    let (mut depth, mut brackets, mut block_comment) = (0, 0, false);
    for line in src.lines() {
        match line.trim() {
            "%{" | "#{" => block_comment = true, // ponytail: nested block comments not tracked
            "%}" | "#}" => block_comment = false,
            _ if block_comment => {}
            _ => {
                let cs: Vec<char> = line.chars().collect();
                let mut i = 0;
                while i < cs.len() {
                    match cs[i] {
                        '%' | '#' => break,
                        '"' => {
                            i += 1;
                            while i < cs.len() && cs[i] != '"' {
                                i += if cs[i] == '\\' { 2 } else { 1 };
                            }
                        }
                        // a quote right after a value is transpose, anywhere else it opens a string
                        '\'' if i == 0 || !(cs[i - 1].is_alphanumeric() || "_)]}'.\"".contains(cs[i - 1])) => {
                            i += 1;
                            while i < cs.len() {
                                if cs[i] == '\'' {
                                    if cs.get(i + 1) != Some(&'\'') {
                                        break;
                                    }
                                    i += 1; // '' is an escaped quote
                                }
                                i += 1;
                            }
                        }
                        '(' | '[' | '{' => brackets += 1,
                        ')' | ']' | '}' => brackets = (brackets - 1).max(0),
                        c if c.is_alphabetic() || c == '_' => {
                            let s = i;
                            while i < cs.len() && (cs[i].is_alphanumeric() || cs[i] == '_') {
                                i += 1;
                            }
                            let w: String = cs[s..i].iter().collect();
                            if brackets == 0 && OPEN.contains(&w.as_str()) {
                                depth += 1;
                            } else if brackets == 0 && CLOSE.contains(&w.as_str()) {
                                depth -= 1;
                            }
                            continue;
                        }
                        _ => {}
                    }
                    i += 1;
                }
            }
        }
    }
    depth
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plot::{Data, Series};

    #[test]
    fn half_blocks() {
        // 2x2 pixels: row 0 = (1,2,3) (4,5,6), row 1 = (7,8,9) (10,11,12)
        let px: Vec<u8> = (1..=12).collect();
        let area = Rect::new(0, 0, 2, 1);
        let mut buf = Buffer::empty(area);
        HalfBlocks { px: &px, w: 2 }.render(area, &mut buf);
        let c = &buf[(1, 0)];
        assert_eq!(c.symbol(), "▀");
        assert_eq!((c.fg, c.bg), (Color::Rgb(4, 5, 6), Color::Rgb(10, 11, 12)));
        assert_eq!(buf[(0, 0)].fg, Color::Rgb(1, 2, 3));
        // short buffer: no panic, missing pixels reset
        let area = Rect::new(0, 0, 2, 2);
        let mut buf = Buffer::empty(area);
        HalfBlocks { px: &px, w: 2 }.render(area, &mut buf);
        assert_eq!(buf[(0, 1)].fg, Color::Reset);
    }

    #[test]
    fn blocks() {
        assert_eq!(block_depth("x = 1"), 0);
        assert_eq!(block_depth("for i = 1:3"), 1);
        assert_eq!(block_depth("for i = 1:3\n  x(end+1) = i;"), 1);
        assert_eq!(block_depth("for i = 1:3\n  x(end+1) = i;\nend"), 0);
        assert_eq!(block_depth("y = x(end)'; z = a{end}"), 0);
        assert_eq!(block_depth("if a, b, end"), 0);
        assert_eq!(block_depth("if a\n if b\n end"), 1);
        assert_eq!(block_depth("s = 'for if while'; % end end"), 0);
        assert_eq!(block_depth("s = 'don''t end'; t = \"say \\\"end\\\"\""), 0);
        assert_eq!(block_depth("disp(\"end\"); while 1 # end"), 1);
        assert_eq!(block_depth("function y = f(x)\n  y = x';\nendfunction"), 0);
        assert_eq!(block_depth("do\n x += 1;\nuntil x > 3"), 0);
        assert_eq!(block_depth("try\n a\ncatch e\n b\nend_try_catch"), 0);
        assert_eq!(block_depth("%{\nfor this\n%}\nx = a'"), 0);
        assert_eq!(block_depth("endpoint = 3; iffy = 1"), 0);
    }

    #[test]
    fn data_cursor_and_trace() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut app = App::with(Interp::new(), "");
        app.interp.plots.axes().add(Series::new(Data::Line { x: vec![0., 1., 2., 3.], y: vec![0., 10., 0., 10.] }));
        app.interp.plots.touch();
        let mut term = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut draw = |app: &mut App| term.draw(|f| app.draw(f)).unwrap().buffer.clone();
        draw(&mut app);
        let key = |app: &mut App, c: KeyCode| app.on_key(KeyEvent::new(c, KeyModifiers::NONE));
        let index = |app: &App| app.probe.map(|h| h.index);
        // hover near the point (1, 10): nearest point, read out in the footer
        let (x, y) = cursor::project(&app.frames[0], [1., 10., 0.]);
        let at = cell_at(app.plot_rect, x + 1.0, y).unwrap();
        app.on_mouse(MouseEvent { kind: MouseEventKind::Moved, column: at.x, row: at.y, modifiers: KeyModifiers::NONE });
        assert_eq!(index(&app), Some(1));
        let screen = draw(&mut app);
        let on = cell_at(app.plot_rect, x, y).unwrap(); // highlighted where the point is
        assert_eq!((screen[(on.x, on.y)].fg, screen[(on.x, on.y)].bg), (CURSOR, CURSOR));
        let text: String = screen.content().iter().map(|c| c.symbol()).collect();
        assert!(text.contains("x = 1, y = 10"), "{text}");
        // trace: starts mid-series, arrows step, p pins, P unpins, t again leaves
        app.probe = None;
        app.plot_focus = true;
        key(&mut app, KeyCode::Char('t'));
        assert_eq!(index(&app), Some(2));
        key(&mut app, KeyCode::Right);
        key(&mut app, KeyCode::Right); // stays at the end
        assert_eq!(index(&app), Some(3));
        key(&mut app, KeyCode::Left);
        key(&mut app, KeyCode::Char('p'));
        assert_eq!(app.interp.plots.axes().pins.len(), 1);
        assert_eq!(&app.interp.plots.axes().pins[0][..2], &[2., 0.]);
        key(&mut app, KeyCode::Right);
        let text: String = draw(&mut app).content().iter().map(|c| c.symbol()).collect();
        assert!(text.contains("Δx = 1, Δy = 10, Δy/Δx = 10"), "{text}");
        key(&mut app, KeyCode::Char('P'));
        assert!(app.interp.plots.axes().pins.is_empty());
        let xlim = app.interp.plots.axes().xlim;
        key(&mut app, KeyCode::Left); // tracing: arrows don't pan
        assert_eq!(app.interp.plots.axes().xlim, xlim);
        key(&mut app, KeyCode::Char('t'));
        assert!(!app.trace && app.probe.is_none());
    }

    #[test]
    fn pan_zoom() {
        assert_eq!(shift((0., 10.), 0.1), (1., 11.));
        assert_eq!(shift((0., 10.), -0.1), (-1., 9.));
        assert_eq!(scale((0., 10.), 0.5), (2.5, 7.5));
        assert_eq!(scale((0., 10.), 0.0), (0., 10.)); // collapse refused
        assert_eq!(shift((0., f64::MAX), 1.0), (0., f64::MAX)); // overflow refused

        let mut a = Axes::new(0);
        a.add(Series::new(Data::Line { x: vec![0., 10.], y: vec![0., 1.] }));
        a.xlim = Some((0., 10.));
        nudge(&mut a, 0.1, 0.0);
        assert_eq!(a.xlim, Some((1., 11.)));
        assert_eq!(a.ylim, None); // untouched axis keeps auto limits
        zoom(&mut a, 0.5);
        assert_eq!(a.xlim, Some((3.5, 8.5)));
        assert!(a.ylim.is_some());

        a.force3d = true;
        zoom(&mut a, 0.5);
        assert_eq!(a.view.zoom, 2.0);
        nudge(&mut a, 0.0, 10.0);
        assert_eq!(a.view.pitch, FRAC_PI_2); // clamped
    }

    #[test]
    fn completion_helpers() {
        assert_eq!(insertion("integrate(□, □, □)", "integrate", false), ("integrate()".into(), 10));
        assert_eq!(insertion("linspace(□)", "linspace", false), ("linspace()".into(), 9));
        assert_eq!(insertion("rand()", "rand", false), ("rand()".into(), 6));
        assert_eq!(insertion("pi", "pi", false), ("pi".into(), 2));
        assert_eq!(insertion("sin(□)", "sin", true), ("sin".into(), 3)); // `sni|(x)` -> `sin|(x)`
        let at = |s: &str| call_at(&s.chars().collect::<Vec<_>>(), s.chars().count());
        assert_eq!(at("integrate(x^2, 0").as_deref(), Some("integrate"));
        assert_eq!(at("f(g(x), [1, 2").as_deref(), Some("f"));
        assert_eq!(at("y = a_1(").as_deref(), Some("a_1"));
        assert_eq!(at("sin(x) + 1"), None);
        assert_eq!(at("(1 + 2"), None);
        assert_eq!(call_at(&['f', '('], 9), Some("f".into())); // cursor past the end: no panic
    }

    #[test]
    fn completion_popup() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let item = |name: &str, insert: &str, signature: &str, desc: &str| Completion {
            name: name.into(),
            insert: insert.into(),
            signature: signature.into(),
            desc: desc.into(),
            kind: complete::Kind::Function,
        };
        let items = vec![
            item("integrate", "integrate(□, □, □)", "integrate(f, a, b, x = x)", "definite or indefinite integral"),
            item("integer", "integer", "integer", ""),
        ];
        let mut app = App::with(Interp::new(), "");
        let key = |app: &mut App, c: KeyCode| app.on_key(KeyEvent::new(c, KeyModifiers::NONE));
        let input = |app: &App| app.input.iter().collect::<String>();
        let open = |app: &mut App, s: &str| {
            app.set_input(s);
            app.menu = Some(Menu { start: 0, items: items.clone(), sel: 0 });
        };
        // arrows move the selection (wrapping), Tab accepts: □ slots dropped, cursor on the first
        open(&mut app, "integ");
        key(&mut app, KeyCode::Up);
        assert_eq!(app.menu.as_ref().unwrap().sel, 1);
        key(&mut app, KeyCode::Down);
        assert_eq!(app.help(), HELP_MENU);
        key(&mut app, KeyCode::Tab);
        assert_eq!((input(&app), app.cursor, app.plot_focus), ("integrate()".into(), 10, false));
        assert!(app.menu.is_none() && app.history.is_empty());
        // Enter accepts too; on a match that's already typed out it submits
        open(&mut app, "integ");
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Enter);
        assert_eq!(input(&app), "integer");
        open(&mut app, "integer");
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Enter);
        assert_eq!((input(&app), app.history.len()), (String::new(), 1));
        // Esc only closes it; closed, the keys are theirs again
        open(&mut app, "integ");
        key(&mut app, KeyCode::Esc);
        assert_eq!((input(&app), app.menu.is_none()), ("integ".into(), true));
        key(&mut app, KeyCode::Up);
        assert_eq!(input(&app), "integer"); // history
        key(&mut app, KeyCode::Tab);
        assert!(app.plot_focus);
        key(&mut app, KeyCode::Tab);
        // drawn above the input line, under the prefix; a click accepts a row
        let mut term = Terminal::new(TestBackend::new(160, 30)).unwrap();
        open(&mut app, "y = integ");
        app.menu.as_mut().unwrap().start = 4;
        let screen = term.draw(|f| app.draw(f)).unwrap().buffer.clone();
        let r = app.menu_rect;
        let row = |y: u16| (r.x..r.right()).map(|x| screen[(x, y)].symbol()).collect::<String>();
        assert_eq!(r.height, 2);
        assert!(row(r.y).starts_with(" integrate(f, a, b, x = x)  definite or"), "{}", row(r.y));
        assert!(row(r.y + 1).starts_with(" integer "));
        app.on_mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: r.x + 2, row: r.y + 1, modifiers: KeyModifiers::NONE });
        assert_eq!(input(&app), "y = integer");
    }

    #[test]
    fn plot_keys() {
        let mut app = App::with(Interp::new(), "");
        app.interp.plots.axes().add(Series::new(Data::Line { x: vec![0., 1., 2.], y: vec![0., 1., 4.] }));
        app.interp.plots.figures[0].code = vec!["plot([0 1 2], [0 1 4]);".into()];
        app.interp.plots.touch();
        app.plot_focus = true;
        let key = |app: &mut App, c: char| app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        key(&mut app, 'k');
        assert!(app.interp.plots.axes().keypoints);
        // calculator mode: code into the log only, no view echo
        key(&mut app, 'o');
        key(&mut app, '-');
        // the code, then the view (key points now on)
        assert!(app.log.iter().rev().take(2).any(|l| l.1.starts_with("plot([0 1 2]")), "{:?}", app.log);
        assert_eq!(app.log.last().unwrap().1, "keypoints on;");
        assert!(app.history.is_empty());
        // Octave mode: code also into the history; pan/zoom echo one view line, replaced in place
        app.lang = Lang::Octave;
        key(&mut app, 'o');
        assert_eq!(app.history.last().map(|h| h.0), Some(Lang::Octave));
        key(&mut app, '-');
        key(&mut app, '+');
        let views = app.log.iter().filter(|(_, s)| s.starts_with(VIEW)).count();
        assert_eq!(views, 1);
        let (x0, x1) = app.interp.plots.axes().xlim.unwrap();
        assert_eq!(app.log.last().unwrap().1, format!("% view: xlim([{} {}]); ylim([{} {}]);", crate::sig(x0, 4), crate::sig(x1, 4), crate::sig(app.interp.plots.axes().ylim.unwrap().0, 4), crate::sig(app.interp.plots.axes().ylim.unwrap().1, 4)));
        app.push(Kind::Out, "something else");
        key(&mut app, '-');
        assert_eq!(app.log.iter().filter(|(_, s)| s.starts_with(VIEW)).count(), 2);
        key(&mut app, 'k');
        assert!(!app.interp.plots.axes().keypoints);
    }
}
