//! Calculator mode: math input through libqalculate, live preview, conversion to and from Octave.
use std::time::{Duration, Instant};

use super::{App, Kind, guarded};
use crate::qalc::{self, Answer};

/// Input language of the REPL.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Lang {
    Calc,
    Octave,
}

impl Lang {
    pub(super) fn other(self) -> Lang {
        match self {
            Lang::Calc => Lang::Octave,
            Lang::Octave => Lang::Calc,
        }
    }
}

/// How long the input must stand still before the preview is computed.
const DEBOUNCE: Duration = Duration::from_millis(200);

/// Result-as-you-type, computed once the input has stood still for `DEBOUNCE`.
#[derive(Default)]
pub(super) struct Preview {
    /// input that `text` is (or, once `due`, will be) the preview of
    input: String,
    due: Option<Instant>,
    pub(super) text: Option<String>,
}

impl Preview {
    /// Note the input at `now`; true when it's time to compute its preview. A change drops the
    /// old preview and restarts the wait.
    fn update(&mut self, input: &str, now: Instant) -> bool {
        if input != self.input {
            (self.input, self.text, self.due) = (input.to_string(), None, Some(now + DEBOUNCE));
        }
        let ready = self.due.is_some_and(|d| now >= d);
        if ready {
            self.due = None;
        }
        ready
    }

    /// How long the event loop may block before the preview is due; None = as long as it likes.
    pub(super) fn wait(&self, now: Instant) -> Option<Duration> {
        self.due.map(|d| d.saturating_duration_since(now))
    }
}

/// Log lines for an answer: `input = result`, then the approximation, other forms and warnings.
fn answer_lines(a: &Answer) -> Vec<(Kind, String)> {
    let mut v = vec![(Kind::In, format!("{} = {}", a.input, a.result))];
    if let Some(x) = &a.approx {
        v.push((Kind::Dim, format!("  ≈ {x}")));
    }
    if !a.alts.is_empty() {
        let alts: Vec<String> = a.alts.iter().map(|(l, t)| format!("{l} {t}")).collect();
        v.push((Kind::Dim, format!("  {}", alts.join(" · "))));
    }
    v.extend(a.messages.iter().map(|m| (Kind::Warn, format!("  {m}"))));
    v
}

impl App {
    /// One calculator line: answer into the log (and Octave's workspace), its visual into the plot pane.
    pub(super) fn calc(&mut self, line: &str) {
        if line.trim().is_empty() {
            return;
        }
        let r = guarded(|| qalc::eval_into(&mut self.interp, line));
        self.probe = None; // plot(...) may have replaced the figure under it
        match r {
            Ok(a) => self.show(a),
            Err(e) => {
                self.push(Kind::In, line);
                self.push(Kind::Err, &format!("error: {e}"));
            }
        }
    }

    fn show(&mut self, a: Answer) {
        for (k, s) in answer_lines(&a) {
            self.push(k, &s);
        }
        if self.auto_plot
            && let Some(code) = &a.visual
        {
            self.eval(code);
        }
        self.last = Some(a);
    }

    /// ^O: the last answer as Octave code in the input line, in Octave mode.
    pub(super) fn to_octave(&mut self) {
        match self.last.as_ref().and_then(|a| a.octave.clone().or_else(|| a.visual.clone())) {
            Some(code) => {
                self.lang = Lang::Octave;
                self.set_input(&code);
            }
            None => self.push(Kind::Warn, "(no Octave form for the last answer)"),
        }
    }

    /// ^K: the Octave input line in math notation, in calculator mode.
    pub(super) fn to_math(&mut self) {
        let code: String = self.input.iter().collect();
        self.lang = Lang::Calc;
        self.set_input(&qalc::from_octave(&code));
    }

    /// Live preview upkeep: after every event, and when its wait runs out.
    pub(super) fn tick(&mut self, now: Instant) {
        if self.lang == Lang::Octave {
            self.preview = Preview::default();
            return;
        }
        let input: String = self.input.iter().collect();
        if self.preview.update(&input, now) {
            self.preview.text = preview_of(&input);
        }
    }
}

/// Preview text for calculator input; none for blank input or plots (drawing is the slow part).
// ponytail: runs on the UI thread, a slow expression stalls typing up to qalc's preview timeout; thread it if that bites
fn preview_of(input: &str) -> Option<String> {
    if input.trim().is_empty() || input.contains("plot(") {
        return None;
    }
    guarded(|| Ok(qalc::preview(input))).ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::Interp;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(app: &mut App, c: KeyCode) {
        app.on_key(KeyEvent::new(c, KeyModifiers::NONE));
    }
    fn ctrl(app: &mut App, c: char) {
        app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
    }
    fn enter(app: &mut App, s: &str) {
        app.set_input(s);
        key(app, KeyCode::Enter);
    }
    fn input(app: &App) -> String {
        app.input.iter().collect()
    }

    #[test]
    fn mode_toggle_and_prompt() {
        let mut app = App::with(Interp::new(), "");
        assert_eq!((app.lang, app.prompt()), (Lang::Calc, "= "));
        assert!(app.help().contains("^O"));
        key(&mut app, KeyCode::F(5));
        assert_eq!((app.lang, app.prompt()), (Lang::Octave, ">> "));
        assert!(app.help().contains("^K"));
        enter(&mut app, "if 1");
        assert_eq!(app.prompt(), ".. ");
        enter(&mut app, "end");
        assert_eq!(app.prompt(), ">> ");
        key(&mut app, KeyCode::F(5));
        assert_eq!(app.prompt(), "= ");
        key(&mut app, KeyCode::F(6));
        assert!(!app.auto_plot);
    }

    #[test]
    fn history_keeps_modes() {
        let mut app = App::with(Interp::new(), "");
        enter(&mut app, "1 + 1");
        assert!(app.log.contains(&(Kind::In, "1 + 1 = 2".into())), "{:?}", app.log);
        key(&mut app, KeyCode::F(5));
        enter(&mut app, "tq_h = 3;");
        key(&mut app, KeyCode::Up);
        assert_eq!((app.lang, input(&app)), (Lang::Octave, "tq_h = 3;".into()));
        key(&mut app, KeyCode::Up);
        assert_eq!((app.lang, input(&app)), (Lang::Calc, "1 + 1".into()));
        key(&mut app, KeyCode::Down);
        assert_eq!((app.lang, input(&app)), (Lang::Octave, "tq_h = 3;".into()));
        key(&mut app, KeyCode::Down);
        assert_eq!((app.lang, input(&app)), (Lang::Octave, String::new()));
    }

    #[test]
    fn preview_debounce() {
        let t = Instant::now();
        let ms = |n| t + Duration::from_millis(n);
        let mut p = Preview::default();
        assert!(!p.update("", t) && p.wait(t).is_none()); // nothing typed, nothing due
        assert!(!p.update("1+", t));
        assert_eq!(p.wait(t), Some(DEBOUNCE));
        assert!(!p.update("1+2", ms(150))); // still typing: the wait restarts
        assert_eq!(p.wait(ms(300)), Some(Duration::from_millis(50)));
        assert!(!p.update("1+2", ms(300)));
        assert!(p.update("1+2", ms(350)));
        p.text = Some("3".into());
        assert!(!p.update("1+2", ms(900)) && p.wait(ms(900)).is_none()); // computed once
        assert!(!p.update("1+", ms(1000)) && p.text.is_none()); // a change drops the stale one
    }

    #[test]
    fn preview_in_app() {
        let mut app = App::with(Interp::new(), "");
        let t = Instant::now();
        app.set_input("1/3 + 1/6");
        app.tick(t);
        assert!(app.preview.text.is_none());
        app.tick(t + DEBOUNCE);
        assert!(app.preview.text.as_ref().is_some_and(|p| p.starts_with("1/2")), "{:?}", app.preview.text);
        app.set_input("plot(x^2)");
        app.tick(t + DEBOUNCE * 2);
        app.tick(t + DEBOUNCE * 3);
        assert!(app.preview.text.is_none() && app.preview.wait(t).is_none()); // due, but plots aren't previewed
        key(&mut app, KeyCode::F(5)); // Octave mode: no preview, nothing to wait for
        app.tick(t + DEBOUNCE * 4);
        assert!(app.preview.wait(t).is_none());
    }

    #[test]
    fn answer_formatting() {
        let a = Answer {
            input: "1/3 + 1/6".into(),
            result: "1/2".into(),
            approx: Some("0.5".into()),
            alts: vec![("fraction".into(), "1/2".into()), ("hex".into(), "0x1".into())],
            messages: vec!["careful".into()],
            ..Answer::default()
        };
        let want = [
            (Kind::In, "1/3 + 1/6 = 1/2"),
            (Kind::Dim, "  ≈ 0.5"),
            (Kind::Dim, "  fraction 1/2 · hex 0x1"),
            (Kind::Warn, "  careful"),
        ];
        assert_eq!(answer_lines(&a), want.map(|(k, s)| (k, s.to_string())));
        let a = Answer { input: "2".into(), result: "2".into(), ..Answer::default() };
        assert_eq!(answer_lines(&a), [(Kind::In, "2 = 2".to_string())]);
    }

    #[test]
    fn conversions() {
        let mut app = App::with(Interp::new(), "");
        ctrl(&mut app, 'o'); // nothing to convert yet
        assert_eq!((app.lang, app.log.last().unwrap().0), (Lang::Calc, Kind::Warn));
        enter(&mut app, "diff(x^3, x)");
        ctrl(&mut app, 'o');
        assert_eq!((app.lang, input(&app)), (Lang::Octave, "3.*(x.^2)".into()));
        ctrl(&mut app, 'k');
        assert_eq!((app.lang, input(&app)), (Lang::Calc, "3*(x^2)".into()));
        // no Octave form: the visual stands in
        app.last = Some(Answer { visual: Some("fplot(@(x) x, [0 1])".into()), ..Answer::default() });
        ctrl(&mut app, 'o');
        assert_eq!(input(&app), "fplot(@(x) x, [0 1])");
        enter(&mut app, "sin(");
        assert_eq!(app.log.last().unwrap().0, Kind::Err);
    }

    #[test]
    fn auto_plot() {
        let mut app = App::with(Interp::new(), "");
        let a = Answer { visual: Some("plot([1 2 3]);".into()), ..Answer::default() };
        app.auto_plot = false;
        app.show(a.clone());
        assert!(app.interp.plots.figures.is_empty());
        app.auto_plot = true;
        app.show(a);
        assert_eq!(app.interp.plots.figures.len(), 1);
        enter(&mut app, "plot(x^2, 0, 1)"); // calculator plots land in the same session
        assert_eq!(app.interp.plots.figures.len(), 2);
    }
}
