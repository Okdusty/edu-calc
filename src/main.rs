mod builtins;
mod complete;
mod doc;
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
usage: EduCalc                         GTK window (terminal UI when there's no display)
       EduCalc gui [FILE.m]            GTK window, FILE loaded and run
       EduCalc tui [FILE.m]            terminal UI, FILE run first
       EduCalc FILE.m|- [opts]         run script (- = stdin), render its figures
       EduCalc DATA [opts]             load .csv .tsv .txt .dat .xlsx .xls .xlsm .ods: size, column stats
                                      and a plot (first column = x when there are several)
       EduCalc -e CODE [opts]          evaluate Octave code
       EduCalc plot EXPR [A:B] [opts]  plot f(x), or surface if EXPR uses y
       EduCalc MATH... [opts]          calculate like qalc: 5 km to mi, 'integrate(x^2, 0, 3)',
                                      'solve(x^2 = 4)', 'plot(sin(x)/x, -20, 20)'
opts:  -o OUT.png|OUT.svg   save figures (figure 2 -> OUT-2.png ...); MATH also saves its visualization;
                            default draws in the terminal
       -s WxH               image size, default 800x600
       --light              light theme
       --set KEY=VALUE      calculator option for this run, over qcalcp.ini [calc] (repeatable):
                            base=16, angle=deg, precision=20, approx=exact, fraction=mixed, bits=32 ...
                            (`set KEY VALUE` typed in the calculator UIs is saved to qcalcp.ini)
config: ~/.config/qcalcp/{rc.m, addons/*.py, qcalcp.ini, style.css}
";

/// ~/.config/qcalcp
pub fn config_dir() -> PathBuf {
    gtk4::glib::user_config_dir().join("qcalcp")
}

/// ~/.config/qcalcp/qcalcp.ini
fn ini_path() -> PathBuf {
    config_dir().join("qcalcp.ini")
}

/// Builtin lookup order, after variables and user-defined functions.
pub fn resolve_builtin(name: &str) -> Option<Builtin> {
    lang::builtin(name)
        .or_else(|| builtins::lookup(name))
        .or_else(|| plot::builtins::lookup(name))
        .or_else(|| py::builtin(name))
        .or_else(|| qalc::builtin(name))
        .or_else(|| doc::builtin(name))
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

/// Interpreter for one-shot runs: startup messages to stderr, calculator options from qcalcp.ini
/// then --set.
fn cli_interp(sets: &[(String, String)]) -> Result<lang::Interp, String> {
    let (interp, msg) = new_interp();
    cli_startup(&msg);
    let (mut o, warn) = qalc::Options::load_ini(&ini_path());
    warn.lines().for_each(|l| eprintln!("{l}"));
    for (k, v) in sets {
        o.set(k, v).map_err(|e| format!("--set {k}={v}: {e}"))?;
    }
    qalc::set_options(&o);
    Ok(interp)
}

/// Command line: options, then the positional words.
#[derive(Debug)]
struct Args {
    out: Option<String>,
    size: (u32, u32),
    light: bool,
    /// --set KEY=VALUE
    sets: Vec<(String, String)>,
    pos: Vec<String>,
}

/// Err is the usage complaint, "" for --help.
fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut a = Args { out: None, size: (800, 600), light: false, sets: vec![], pos: vec![] };
    while let Some(s) = args.next() {
        match s.as_str() {
            "-o" => a.out = args.next(),
            "-s" => {
                let wh = args.next().as_deref().and_then(|s| s.split_once('x')).and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)));
                a.size = wh.ok_or("-s wants WxH")?;
            }
            "--light" => a.light = true,
            "--set" => {
                let kv = args.next().and_then(|s| s.split_once('=').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())));
                a.sets.push(kv.ok_or("--set wants KEY=VALUE")?);
            }
            "-h" | "--help" => return Err(String::new()),
            _ => a.pos.push(s),
        }
    }
    Ok(a)
}

/// Spreadsheet/table files: loaded with readmatrix instead of run as scripts.
fn is_data(path: &str) -> bool {
    let ext = std::path::Path::new(path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    matches!(ext.as_deref(), Some("csv" | "tsv" | "txt" | "dat" | "xlsx" | "xls" | "xlsm" | "ods"))
}

fn main() -> ExitCode {
    bundled_python_home();
    let a = match parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => return usage(&e),
    };
    let pos = &a.pos;
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
        Some(f) if is_data(f) => return data_file(f, &a),
        Some(f) if f.ends_with(".m") || std::path::Path::new(f).exists() => match std::fs::read_to_string(f) {
            Ok(s) => s,
            Err(e) => return fail(&format!("{f}: {e}")),
        },
        Some(_) => return calc(&pos.join(" "), &a),
    };
    batch(&code, &a)
}

/// Qalculate-style one-shot: `EduCalc 5 km to mi` prints `5 km → mi = 3.106855961 mi`.
fn calc(expr: &str, args: &Args) -> ExitCode {
    let mut interp = match cli_interp(&args.sets) {
        Ok(i) => i,
        Err(e) => return fail(&e),
    };
    match qalc::eval_into(&mut interp, expr) {
        Ok(a) => {
            for m in &a.messages {
                eprintln!("warning: {m}");
            }
            println!("{}", crate::answer_head(&a));
            if let Some(x) = &a.approx {
                println!("  ≈ {x}");
            }
            if let Some(b) = bases_for(&a) {
                println!("  {}", bases_line(&b));
            }
            // with -o, also save the answer's visualization (shaded integral, roots, matrix heatmap ...)
            if let (Some(_), Some(v)) = (&args.out, &a.visual)
                && let Err(e) = interp.eval(v)
            {
                eprintln!("warning: visualization failed: {e}");
            }
            figures(&interp, args)
        }
        Err(e) => fail(&e),
    }
}

/// `qalc::bases` in a programming context only: a non-decimal base is set, or the input used bit
/// operators or a conversion. Plain integers (2 + 2) get no bases line.
// ponytail: reads the math view, which prints 0b101 as 5; pass the typed text if literals alone should count
pub fn bases_for(a: &qalc::Answer) -> Option<Vec<(&'static str, String)>> {
    let o = qalc::options();
    let prog = o.base != 10 || o.input_base != 10 || ["&", "|", "xor", "~", "shift(", "bitcmp", "bitrot", "→"].iter().any(|t| a.input.contains(t));
    if prog { qalc::bases(a) } else { None }
}

/// `input = result`; a `set KEY VALUE` answer is just its result ("angle = deg").
pub fn answer_head(a: &qalc::Answer) -> String {
    if a.input.starts_with("set ") { a.result.clone() } else { format!("{} = {}", a.input, a.result) }
}

/// An integer answer in the programming bases: "HEX FF · DEC 255 · OCT 377 · BIN 1111 1111".
fn bases_line(b: &[(&str, String)]) -> String {
    b.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join(" · ")
}

/// `x` rounded to `n` significant digits, printed short: 3.142, 12.5, -3, NaN.
// ponytail: no exponent form, 1e-9 prints as 0.000000001; a %g formatter if that shows up in data
fn sig(x: f64, n: usize) -> String {
    format!("{:.*e}", n.max(1) - 1, x).parse::<f64>().unwrap_or(x).to_string()
}

/// `EduCalc FILE.csv`: readmatrix into `data`, print its size and column statistics, plot it.
fn data_file(path: &str, args: &Args) -> ExitCode {
    let mut interp = match cli_interp(&args.sets) {
        Ok(i) => i,
        Err(e) => return fail(&e),
    };
    let r = interp.eval(&format!("data = readmatrix('{}');", path.replace('\'', "''")));
    print!("{}", interp.take_output());
    if let Err(e) = r {
        return fail(&e);
    }
    let m = match interp.vars.get("data").map(value::Value::matrix) {
        Some(Ok(m)) => m,
        Some(Err(e)) => return fail(&format!("{path}: {e}")),
        None => return fail(&format!("{path}: readmatrix gave nothing")),
    };
    print!("{path}: {}x{}\n{}", m.rows, m.cols, column_stats(&m));
    if m.is_empty() {
        return ExitCode::SUCCESS;
    }
    // one column against its index, else the first column is x for the others
    let plot = if m.cols == 1 { "plot(data); grid on;" } else { "plot(data(:, 1), data(:, 2:end)); grid on;" };
    if let Err(e) = interp.eval(plot) {
        return fail(&e);
    }
    figures(&interp, args)
}

/// n/mean/std/min/max per column, NaN (empty cells) skipped.
fn column_stats(m: &value::Matrix) -> String {
    let mut s = format!("{:>4}{:>8}{:>12}{:>12}{:>12}{:>12}\n", "col", "n", "mean", "std", "min", "max");
    for (c, col) in m.data.chunks(m.rows.max(1)).enumerate() {
        let v: Vec<f64> = col.iter().copied().filter(|x| !x.is_nan()).collect();
        let n = v.len() as f64;
        let mean = v.iter().sum::<f64>() / n;
        let std = (v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0).max(1.0)).sqrt();
        let (min, max) = (v.iter().copied().fold(f64::NAN, f64::min), v.iter().copied().fold(f64::NAN, f64::max));
        let [mean, std, min, max] = [mean, std, min, max].map(|x| sig(x, 6));
        s += &format!("{:>4}{:>8}{mean:>12}{std:>12}{min:>12}{max:>12}\n", c + 1, v.len());
    }
    s
}

/// GUI when a display is usable, else the TUI (ssh, bare tty, broken GTK).
fn gui_or_tui(script: Option<PathBuf>) -> ExitCode {
    // GTK would otherwise find a running session's default socket (ssh, other VT) and pop a window there.
    // Only X11/Wayland systems say so through these; Windows and macOS always have a display.
    let no_display = cfg!(all(unix, not(target_os = "macos"))) && ["WAYLAND_DISPLAY", "DISPLAY"].iter().all(|v| std::env::var_os(v).is_none_or(|s| s.is_empty()));
    if no_display {
        eprintln!("EduCalc: no WAYLAND_DISPLAY/DISPLAY, using the TUI");
        return run_tui(script);
    }
    match gtk4::init() {
        Ok(()) => {
            #[cfg(windows)]
            detach_console();
            gui::run(script)
        }
        Err(e) => {
            eprintln!("EduCalc: GTK unavailable ({e}), using the TUI");
            run_tui(script)
        }
    }
}

/// Started from Explorer, the console is ours alone (a terminal's would list its shell too): close
/// that console window, the GUI doesn't need it.
// ponytail: it still flashes up for a moment; a separate windows-subsystem launcher would avoid that
#[cfg(windows)]
fn detach_console() {
    unsafe extern "system" {
        fn GetConsoleProcessList(list: *mut u32, count: u32) -> u32;
        fn FreeConsole() -> i32;
    }
    let mut ids = [0u32; 2];
    if unsafe { GetConsoleProcessList(ids.as_mut_ptr(), 2) } == 1 {
        unsafe { FreeConsole() };
    }
}

/// Packaged builds (AppImage, Windows zip) carry Python's standard library next to the binary,
/// in ../lib/python3.*: point the embedded Python there unless PYTHONHOME says otherwise.
fn bundled_python_home() {
    if std::env::var_os("PYTHONHOME").is_some() {
        return;
    }
    let Some(prefix) = std::env::current_exe().ok().and_then(|e| Some(e.parent()?.parent()?.to_path_buf())) else { return };
    let stdlib = std::fs::read_dir(prefix.join("lib")).into_iter().flatten().flatten();
    if stdlib.into_iter().any(|d| d.file_name().to_string_lossy().starts_with("python3") && d.path().join("os.py").exists()) {
        // SAFETY: first thing in main, before any thread exists
        unsafe { std::env::set_var("PYTHONHOME", &prefix) };
    }
}

fn run_tui(script: Option<PathBuf>) -> ExitCode {
    tui::run(script).map_or_else(|e| fail(&format!("tui: {e}")), |_| ExitCode::SUCCESS)
}

/// Run code, print its output, then save or draw every figure.
fn batch(code: &str, args: &Args) -> ExitCode {
    let mut interp = match cli_interp(&args.sets) {
        Ok(i) => i,
        Err(e) => return fail(&e),
    };
    let r = interp.eval(code);
    print!("{}", interp.take_output());
    if let Err(e) = r {
        return fail(&e);
    }
    figures(&interp, args)
}

/// Save (-o) or draw in the terminal every figure.
fn figures(interp: &lang::Interp, args: &Args) -> ExitCode {
    let theme = if args.light { plot::Theme::light() } else { plot::Theme::dark() };
    let (w, h) = args.size;
    for (k, fig) in interp.plots.figures.iter().enumerate() {
        match &args.out {
            Some(path) => {
                let p = numbered(path, k);
                if let Err(e) = plot::render::save(fig, &theme, p.as_ref(), w, h) {
                    return fail(&format!("{p}: {e}"));
                }
                eprintln!("saved {p}");
            }
            None => {
                let (c, r) = ratatui::crossterm::terminal::size().unwrap_or((100, 30));
                print!("{}", plot::render::ansi(fig, &theme, c, r.saturating_sub(2).max(10)));
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

/// `EduCalc plot EXPR [A:B]` -> Octave code: fplot for f(x), fsurf when EXPR uses y.
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
    eprint!("EduCalc: {err}\n{USAGE}");
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
        let args = |s: &str| super::parse_args(s.split(' ').map(String::from));
        let a = args("--set base=16 --set angle=deg -o a.png 255 + 1").unwrap();
        assert_eq!(a.sets, [("base".into(), "16".into()), ("angle".into(), "deg".into())]);
        assert_eq!((a.out.as_deref(), a.pos.join(" ")), (Some("a.png"), "255 + 1".into()));
        assert_eq!(args("-s 640x480 --light x").map(|a| (a.size, a.light)), Ok(((640, 480), true)));
        assert_eq!(args("--set base").unwrap_err(), "--set wants KEY=VALUE");
        assert_eq!(args("-s 640").unwrap_err(), "-s wants WxH");
        assert_eq!(args("-h").unwrap_err(), "");
        assert!(super::is_data("dir/m.CSV") && super::is_data("t.xlsx") && super::is_data("x.ods"));
        assert!(!super::is_data("run.m") && !super::is_data("255") && !super::is_data("1.5"));
        assert_eq!((super::sig(2.0 / 3.0, 4), super::sig(-12.5, 4), super::sig(f64::NAN, 4)), ("0.6667".into(), "-12.5".into(), "NaN".into()));
        // columns [1 2 3 NaN] and [5 5 5 5]: NaN skipped, a constant column has std 0
        let m = crate::value::Matrix::new(4, 2, vec![1., 2., 3., f64::NAN, 5., 5., 5., 5.]);
        let t = super::column_stats(&m);
        let rows: Vec<Vec<&str>> = t.lines().map(|l| l.split_whitespace().collect()).collect();
        assert_eq!(rows, [vec!["col", "n", "mean", "std", "min", "max"], vec!["1", "3", "2", "1", "1", "3"], vec!["2", "4", "5", "0", "5", "5"]]);
    }
}
