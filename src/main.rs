mod builtins;
mod gui;
mod lang;
mod plot;
mod py;
mod qalc;
mod tui;
mod value;

use std::path::PathBuf;
use std::process::ExitCode;

use value::Builtin;

const USAGE: &str = "\
usage: qcalcp                         GTK window (terminal UI when there's no display)
       qcalcp gui [FILE.m]            GTK window, FILE loaded and run
       qcalcp tui [FILE.m]            terminal UI, FILE run first
       qcalcp FILE.m|- [opts]         run script (- = stdin), render its figures
       qcalcp -e CODE [opts]          evaluate Octave code
       qcalcp plot EXPR [A:B] [opts]  plot f(x), or surface if EXPR uses y
       qcalcp MATH... [opts]          calculate like qalc: 5 km to mi, 'integrate(x^2, 0, 3)',
                                      'solve(x^2 = 4)', 'plot(sin(x)/x, -20, 20)'
opts:  -o OUT.png|OUT.svg   save figures (figure 2 -> OUT-2.png ...); MATH also saves its visualization;
                            default draws in the terminal
       -s WxH               image size, default 800x600
       --light              light theme
config: ~/.config/qcalcp/{rc.m, addons/*.py, qcalcp.ini, style.css}
";

/// ~/.config/qcalcp
pub fn config_dir() -> PathBuf {
    gtk4::glib::user_config_dir().join("qcalcp")
}

/// Builtin lookup order, after variables and user-defined functions.
pub fn resolve_builtin(name: &str) -> Option<Builtin> {
    lang::builtin(name)
        .or_else(|| builtins::lookup(name))
        .or_else(|| plot::builtins::lookup(name))
        .or_else(|| py::builtin(name))
        .or_else(|| qalc::builtin(name))
}

const ADDONS_OK: &str = "loaded addons: ";

/// One-shot CLI runs stay quiet about successfully loaded addons; errors and rc.m output still show.
fn cli_startup(msg: &str) {
    msg.lines().filter(|l| !l.starts_with(ADDONS_OK)).for_each(|l| eprintln!("{l}"));
}

/// Interpreter with Python addons and rc.m loaded, plus startup messages.
pub fn new_interp() -> (lang::Interp, String) {
    let dir = config_dir();
    let mut interp = lang::Interp::new();
    let mut msg = String::new();
    match py::load_addons(&dir.join("addons")) {
        Ok(names) if !names.is_empty() => msg += &format!("{ADDONS_OK}{}\n", names.join(", ")),
        Ok(_) => {}
        Err(e) => msg += &format!("addons: {e}\n"),
    }
    if let Ok(src) = std::fs::read_to_string(dir.join("rc.m")) {
        let r = interp.eval(&src);
        msg += &interp.take_output();
        if let Err(e) = r {
            msg += &format!("rc.m: {e}\n");
        }
    }
    (interp, msg)
}

fn main() -> ExitCode {
    let mut out = None;
    let mut size = (800, 600);
    let mut theme = plot::Theme::dark();
    let mut pos = vec![];
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-o" => out = args.next(),
            "-s" => match args.next().as_deref().and_then(|s| s.split_once('x')).and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))) {
                Some(s) => size = s,
                None => return usage("-s wants WxH"),
            },
            "--light" => theme = plot::Theme::light(),
            "-h" | "--help" => return usage(""),
            _ => pos.push(a),
        }
    }
    let code = match pos.first().map(String::as_str) {
        None => return gui_or_tui(None),
        Some("gui") => return gui_or_tui(pos.get(1).map(PathBuf::from)),
        Some("tui") => return run_tui(pos.get(1).map(PathBuf::from)),
        Some("-e") => match pos.get(1) {
            Some(c) => c.clone(),
            None => return usage("-e wants code"),
        },
        Some("plot") => match pos.get(1) {
            Some(e) => expr_plot(e, pos.get(2).map(String::as_str)),
            None => return usage("plot wants an expression"),
        },
        Some("-") => std::io::read_to_string(std::io::stdin()).unwrap_or_default(),
        Some(f) if f.ends_with(".m") || std::path::Path::new(f).exists() => match std::fs::read_to_string(f) {
            Ok(s) => s,
            Err(e) => return fail(&format!("{f}: {e}")),
        },
        Some(_) => return calc(&pos.join(" "), out, size, &theme),
    };
    batch(&code, out, size, &theme)
}

/// Qalculate-style one-shot: `qcalcp 5 km to mi` prints `5 km → mi = 3.106855961 mi`.
fn calc(expr: &str, out: Option<String>, size: (u32, u32), theme: &plot::Theme) -> ExitCode {
    let (mut interp, msg) = new_interp();
    cli_startup(&msg);
    match qalc::eval_into(&mut interp, expr) {
        Ok(a) => {
            for m in &a.messages {
                eprintln!("warning: {m}");
            }
            println!("{} = {}", a.input, a.result);
            if let Some(x) = &a.approx {
                println!("  ≈ {x}");
            }
            // with -o, also save the answer's visualization (shaded integral, roots, matrix heatmap ...)
            if let (Some(_), Some(v)) = (&out, &a.visual)
                && let Err(e) = interp.eval(v)
            {
                eprintln!("warning: visualization failed: {e}");
            }
            figures(&interp, out, size, theme)
        }
        Err(e) => fail(&e),
    }
}

/// GUI when a display is usable, else the TUI (ssh, bare tty, broken GTK).
fn gui_or_tui(script: Option<PathBuf>) -> ExitCode {
    // GTK would otherwise find a running session's default socket (ssh, other VT) and pop a window there
    let no_display = ["WAYLAND_DISPLAY", "DISPLAY"].iter().all(|v| std::env::var_os(v).is_none_or(|s| s.is_empty()));
    if no_display {
        eprintln!("qcalcp: no WAYLAND_DISPLAY/DISPLAY, using the TUI");
        return run_tui(script);
    }
    match gtk4::init() {
        Ok(()) => gui::run(script),
        Err(e) => {
            eprintln!("qcalcp: GTK unavailable ({e}), using the TUI");
            run_tui(script)
        }
    }
}

fn run_tui(script: Option<PathBuf>) -> ExitCode {
    tui::run(script).map_or_else(|e| fail(&format!("tui: {e}")), |_| ExitCode::SUCCESS)
}

/// Run code, print its output, then save or draw every figure.
fn batch(code: &str, out: Option<String>, size: (u32, u32), theme: &plot::Theme) -> ExitCode {
    let (mut interp, msg) = new_interp();
    cli_startup(&msg);
    let r = interp.eval(code);
    print!("{}", interp.take_output());
    if let Err(e) = r {
        return fail(&e);
    }
    figures(&interp, out, size, theme)
}

/// Save (-o) or draw in the terminal every figure.
fn figures(interp: &lang::Interp, out: Option<String>, (w, h): (u32, u32), theme: &plot::Theme) -> ExitCode {
    for (k, fig) in interp.plots.figures.iter().enumerate() {
        match &out {
            Some(path) => {
                let p = numbered(path, k);
                if let Err(e) = plot::render::save(fig, theme, p.as_ref(), w, h) {
                    return fail(&format!("{p}: {e}"));
                }
                eprintln!("saved {p}");
            }
            None => {
                let (c, r) = ratatui::crossterm::terminal::size().unwrap_or((100, 30));
                print!("{}", plot::render::ansi(fig, theme, c, r.saturating_sub(2).max(10)));
            }
        }
    }
    ExitCode::SUCCESS
}

/// out.png -> out.png, out-2.png, out-3.png ...
fn numbered(path: &str, k: usize) -> String {
    match (k, path.rsplit_once('.')) {
        (0, _) => path.to_string(),
        (_, Some((stem, ext))) => format!("{stem}-{}.{ext}", k + 1),
        (_, None) => format!("{path}-{}", k + 1),
    }
}

/// `qcalcp plot EXPR [A:B]` -> Octave code: fplot for f(x), fsurf when EXPR uses y.
fn expr_plot(expr: &str, range: Option<&str>) -> String {
    let (a, b) = range.and_then(|r| r.split_once(':')).unwrap_or(("-10", "10"));
    let has_y = expr.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|t| t == "y");
    let e = expr.replace('\'', "''");
    if has_y {
        format!("fsurf('{e}', [{a} {b} {a} {b}]); title('{e}'); xlabel('x'); ylabel('y');")
    } else {
        format!("fplot('{e}', [{a} {b}]); grid on; title('{e}'); xlabel('x');")
    }
}

fn usage(err: &str) -> ExitCode {
    if err.is_empty() {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    eprint!("qcalcp: {err}\n{USAGE}");
    ExitCode::from(2)
}

fn fail(e: &str) -> ExitCode {
    eprintln!("error: {e}");
    ExitCode::FAILURE
}

#[cfg(test)]
mod tests {
    #[test]
    fn cli_helpers() {
        assert_eq!(super::numbered("a/out.png", 0), "a/out.png");
        assert_eq!(super::numbered("a/out.png", 2), "a/out-3.png");
        assert!(super::expr_plot("sin(x)*y", None).starts_with("fsurf"));
        assert!(super::expr_plot("sin(x)", Some("0:2*pi")).contains("[0 2*pi]"));
    }
}
