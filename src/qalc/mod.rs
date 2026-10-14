//! Calculator mode over libqalculate (C++ shim in shim.cpp, built by build.rs): math notation,
//! exact results, units, symbolic calculus, and conversion both ways with Octave.
//! One global calculator; every call into it holds `PUSHED`'s lock.
use std::collections::BTreeMap;
use std::ffi::{CStr, CString, c_char, c_int};
use std::sync::{Mutex, MutexGuard};

use crate::lang::Interp;
use crate::plot::{Data, Figure, Series};
use crate::value::{Builtin, C64, CMatrix, Matrix, Value};

unsafe extern "C" {
    fn qs_calc(expr: *const c_char, msecs: c_int, mode: c_int) -> *mut c_char;
    fn qs_pretty(expr: *const c_char) -> *mut c_char;
    fn qs_set_var(name: *const c_char, val: *const c_char) -> *mut c_char;
    fn qs_sample(
        expr: *const c_char,
        xvar: *const c_char,
        yvar: *const c_char,
        xs: *const f64,
        ys: *const f64,
        n: usize,
        out: *mut f64,
        msecs: c_int,
    ) -> *mut c_char;
    fn qs_free(s: *mut c_char);
}

const TIMEOUT_MS: i32 = 5000;
/// result-as-you-type budget
const PREVIEW_MS: i32 = 300;

/// qs_calc modes
const PLAIN: c_int = 0;
const SET_ANS: c_int = 1;
const PURE: c_int = 2;

#[derive(Clone, Debug, Default)]
pub struct Answer {
    /// parsed input, math view ("∫(x²)")
    pub input: String,
    /// main result, exact when possible ("1 / √(2)", "x³ / 3 + C", "8.04672 km")
    pub result: String,
    /// decimal approximation when it differs from `result`
    pub approx: Option<String>,
    /// other forms: ("fraction", "1/2"), ("hex", "0xFF"), ("base units", "5000 m") ...
    #[allow(dead_code)] // read by the calculator views (next)
    pub alts: Vec<(String, String)>,
    /// numeric result for Octave (units dropped: the magnitude in the shown unit)
    pub value: Option<Value>,
    /// result as vectorized Octave code, when expressible
    pub octave: Option<String>,
    /// unknowns left in the result, sorted ("x", "y")
    pub free_vars: Vec<String>,
    /// libqalculate warnings
    pub messages: Vec<String>,
    /// Octave code that visualizes this answer (e.g. `intplot(@(x) x.^2, 0, 3)` for a definite
    /// integral); frontends run it with `Interp::eval` when auto-visualize is on. Also the
    /// "convert to Octave" artifact for plots.
    #[allow(dead_code)] // read by the calculator views
    pub visual: Option<String>,
}

/// Last value pushed per Octave variable (unchanged ones aren't pushed again, so calculator-side
/// assignments like `a := x^2` survive). Its lock also serializes every call into libqalculate.
static PUSHED: Mutex<BTreeMap<String, CMatrix>> = Mutex::new(BTreeMap::new());

fn lock() -> MutexGuard<'static, BTreeMap<String, CMatrix>> {
    PUSHED.lock().unwrap_or_else(|e| e.into_inner())
}

fn cstr(s: &str) -> Result<CString, String> {
    CString::new(s).map_err(|_| "expression contains a NUL character".to_string())
}

/// Shim string -> String (freed); None for NULL.
fn take(p: *mut c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
    unsafe { qs_free(p) };
    Some(s)
}

fn list(s: &str) -> Vec<String> {
    s.split('\x1e').filter(|x| !x.is_empty()).map(String::from).collect()
}

fn some(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
}

/// "rows cols re im re im ..." (row-major) -> Value.
fn parse_value(s: &str) -> Option<Value> {
    let mut it = s.split(' ');
    let r: usize = it.next()?.parse().ok()?;
    let c: usize = it.next()?.parse().ok()?;
    let v: Vec<f64> = it.map(|t| t.parse().ok()).collect::<Option<_>>()?;
    if v.len() != 2 * r * c {
        return None;
    }
    let data = (0..r * c).map(|k| 2 * ((k % r) * c + k / r)).map(|q| C64::new(v[q], v[q + 1])).collect();
    Some(Value::complex(CMatrix::new(r, c, data)))
}

/// One calculation (lock held) in a qs_calc mode; also returns the variable name for `a := ...` / `a = ...`.
fn raw_calc(expr: &str, ms: i32, mode: c_int) -> Result<(Answer, Option<String>), String> {
    let c = cstr(expr)?;
    let r = take(unsafe { qs_calc(c.as_ptr(), ms, mode) }).ok_or("qalculate: out of memory")?;
    let f: Vec<&str> = r.split('\x1f').collect();
    if !f[0].is_empty() {
        return Err(list(f[0]).join("; "));
    }
    let [_, input, result, approx, alts, value, octave, vars, msgs, assigned, visual] = f[..] else {
        return Err("qalculate: malformed reply".into());
    };
    let alts = list(alts).iter().filter_map(|a| a.split_once('\x1d')).map(|(l, t)| (l.into(), t.into())).collect();
    let a = Answer {
        input: input.into(),
        result: result.into(),
        approx: some(approx),
        alts,
        value: parse_value(value),
        octave: some(octave),
        free_vars: list(vars),
        messages: list(msgs),
        visual: some(visual),
    };
    Ok((a, some(assigned)))
}

fn raw_pretty(expr: &str) -> String {
    let Ok(c) = cstr(&from_octave(expr)) else { return expr.to_string() };
    match take(unsafe { qs_pretty(c.as_ptr()) }) {
        Some(s) if !s.is_empty() => s,
        _ => expr.to_string(),
    }
}

fn raw_set(name: &str, lit: &str) -> Result<(), String> {
    let (n, l) = (cstr(name)?, cstr(lit)?);
    take(unsafe { qs_set_var(n.as_ptr(), l.as_ptr()) }).map_or(Ok(()), Err)
}

/// expr at (xs[i], ys[i]); yvar "" = one variable (ys unused but read, pass xs).
fn raw_sample(expr: &str, xvar: &str, yvar: &str, xs: &[f64], ys: &[f64]) -> Result<Vec<f64>, String> {
    let (e, x, y) = (cstr(expr)?, cstr(xvar)?, cstr(yvar)?);
    let n = xs.len().min(ys.len());
    let mut out = vec![f64::NAN; n];
    let r = unsafe { qs_sample(e.as_ptr(), x.as_ptr(), y.as_ptr(), xs.as_ptr(), ys.as_ptr(), n, out.as_mut_ptr(), TIMEOUT_MS) };
    take(r).map_or(Ok(out), Err)
}

/// Result-as-you-type: "result" or "result ≈ approx". None on error, after ~300 ms, or when the
/// expression would change anything (assignments, plot, export, command); ans stays as it was.
#[allow(dead_code)] // used by the calculator views
pub fn preview(expr: &str) -> Option<String> {
    let _g = lock();
    let (a, _) = raw_calc(expr, PREVIEW_MS, PURE).ok()?;
    Some(match a.approx {
        Some(x) => format!("{} ≈ {x}", a.result),
        None => a.result,
    })
}

/// Calculate in math notation; handles qalc's "expr to unit/base/fraction/hex ..." conversions.
#[allow(dead_code)] // API for the calculator views; the GUI/TUI wire it up next
pub fn calc(expr: &str) -> Result<Answer, String> {
    let _g = lock();
    raw_calc(expr, TIMEOUT_MS, SET_ANS).map(|(a, _)| a)
}

/// Math view of an expression (Octave or math syntax) for labels; input unchanged on failure.
pub fn pretty(expr: &str) -> String {
    let _g = lock();
    raw_pretty(expr)
}

/// Math expression -> vectorized Octave code, after evaluating it ("diff(x^3, x)" -> "3.*(x.^2)").
#[allow(dead_code)] // API for the calculator views; the GUI/TUI wire it up next
pub fn to_octave(expr: &str) -> Result<String, String> {
    let _g = lock();
    let (a, _) = raw_calc(expr, TIMEOUT_MS, PLAIN)?;
    a.octave.ok_or_else(|| format!("'{}' has no Octave equivalent", a.result))
}

/// Octave expression -> math syntax: "@(x) x.^2 + 2*x" -> "x^2 + 2x", "log(x)" -> "ln(x)",
/// "exp(1)" -> "e", "A'" -> "transpose(A)", "2j" -> "2i".
pub fn from_octave(code: &str) -> String {
    let s = code.trim();
    let s = match s.strip_prefix('@').and_then(|r| r.split_once(')')) {
        Some((_, body)) => body.trim(),
        None => s,
    };
    let cs: Vec<char> = s.chars().collect();
    let at = |i: usize| cs.get(i).copied().unwrap_or('\0');
    let (mut out, mut i) = (String::new(), 0);
    while i < cs.len() {
        let c = cs[i];
        if c.is_ascii_alphabetic() || c == '_' {
            let j = (i..cs.len()).find(|&k| !(cs[k].is_ascii_alphanumeric() || cs[k] == '_')).unwrap_or(cs.len());
            let w: String = cs[i..j].iter().collect();
            if w == "exp" && cs[j..].starts_with(&['(', '1', ')']) {
                out.push('e');
                i = j + 3;
                continue;
            }
            out += match w.as_str() {
                "log" => "ln",
                "j" => "i",
                "sign" => "sgn",
                "fix" => "trunc",
                "real" => "re",
                "imag" => "im",
                "angle" => "arg",
                "nthroot" => "root",
                "Inf" | "inf" => "∞",
                "NaN" | "nan" => "undefined",
                w => w,
            };
            i = j;
        } else if c.is_ascii_digit() || (c == '.' && at(i + 1).is_ascii_digit()) {
            // number; a '.' before * / ^ ' belongs to the element-wise operator
            let mut j = i;
            while at(j).is_ascii_digit() || (at(j) == '.' && !matches!(at(j + 1), '*' | '/' | '^' | '\'')) {
                j += 1;
            }
            if matches!(at(j), 'e' | 'E') && (at(j + 1).is_ascii_digit() || matches!(at(j + 1), '+' | '-') && at(j + 2).is_ascii_digit()) {
                j += 2;
                while at(j).is_ascii_digit() {
                    j += 1;
                }
            }
            out.extend(&cs[i..j]);
            // imaginary literal: 2i, 3.5j
            if matches!(at(j), 'i' | 'j') && !(at(j + 1).is_ascii_alphanumeric() || at(j + 1) == '_') {
                out.push('i');
                j += 1;
            }
            i = j;
            // 2*x -> 2x (not 2*e3: that would read as 2000)
            let star = if at(i) == '.' { i + 1 } else { i };
            if at(star) == '*' && at(star + 1).is_ascii_alphabetic() && !matches!(at(star + 1), 'e' | 'E') {
                i = star + 1;
            }
        } else if c == '.' && matches!(at(i + 1), '*' | '/' | '^') {
            i += 1;
        } else if (c == '\'' || c == '.' && at(i + 1) == '\'') && out.ends_with(|p: char| p.is_ascii_alphanumeric() || "_)]".contains(p)) {
            // transpose of what was just written (a quote anywhere else starts a string)
            // ponytail: ' is plain transpose; the conjugate part only matters for complex matrices
            let k = operand_start(&out);
            out = format!("{}transpose({})", &out[..k], &out[k..]);
            i += if c == '.' { 2 } else { 1 };
        } else if c == '~' && at(i + 1) == '=' {
            out += "!=";
            i += 2;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// Start of the operand that ends `s`: a name or number, or a bracketed group with the function
/// name in front of it.
fn operand_start(s: &str) -> usize {
    let word = |end: usize| s[..end].trim_end_matches(|c: char| c.is_ascii_alphanumeric() || c == '_' || c == '.').len();
    if !s.ends_with([')', ']']) {
        return word(s.len());
    }
    let mut depth = 0;
    for (k, c) in s.char_indices().rev() {
        match c {
            ')' | ']' => depth += 1,
            '(' | '[' => {
                depth -= 1;
                if depth == 0 {
                    return if c == '(' { word(k) } else { k };
                }
            }
            _ => {}
        }
    }
    0
}

/// qalc literal for a numeric matrix: exact decimals ("1.5E0"), [[row], [row]] for matrices.
fn literal(m: &CMatrix) -> String {
    let real = |x: f64| match x {
        x if x.is_nan() => "undefined".to_string(),
        x if x.is_infinite() => (if x > 0.0 { "infinity" } else { "(-infinity)" }).to_string(),
        x => format!("{x:E}"),
    };
    let num = |z: C64| if z.im == 0.0 { real(z.re) } else { format!("({}+({})*i)", real(z.re), real(z.im)) };
    let row = |r: usize| (0..m.cols).map(|c| num(m.get(r, c))).collect::<Vec<_>>().join(", ");
    match (m.rows, m.cols) {
        (1, 1) => num(m.data[0]),
        (1, _) => format!("[{}]", row(0)),
        _ => format!("[{}]", (0..m.rows).map(|r| format!("[{}]", row(r))).collect::<Vec<_>>().join(", ")),
    }
}

fn numeric(v: &Value) -> Option<CMatrix> {
    match v {
        Value::Num(_) | Value::Bool(_) | Value::Complex(_) => v.cmatrix().ok().filter(|m| !m.is_empty()),
        _ => None,
    }
}

/// Push a numeric Octave value into the calculator as variable `name`.
/// Built-in qalculate names (units like `m` or `s`, constants, `x`/`y`/`z`) are refused, not shadowed.
#[allow(dead_code)] // API for the calculator views; the GUI/TUI wire it up next
pub fn set_var(name: &str, v: &Value) -> Result<(), String> {
    let m = numeric(v).ok_or_else(|| format!("{name}: only non-empty numeric values go to the calculator"))?;
    let _g = lock();
    raw_set(name, &literal(&m))
}

/// Numeric workspace variables that changed since their last push.
fn push_vars(pushed: &mut BTreeMap<String, CMatrix>, interp: &Interp) {
    for (name, v) in &interp.vars {
        // ponytail: >10k elements skipped (qalc parses a text literal), pass big data by hand if needed
        let Some(m) = numeric(v).filter(|m| m.len() <= 10_000) else { continue };
        if pushed.get(name) != Some(&m) {
            // refused names (units, constants, x/y/z) stay qalculate's; remembered so they aren't retried
            let _ = raw_set(name, &literal(&m));
            pushed.insert(name.clone(), m);
        }
    }
}

fn linspace(a: f64, b: f64, n: usize) -> Vec<f64> {
    (0..n).map(|i| a + (b - a) * i as f64 / (n.max(2) - 1) as f64).collect()
}

/// expr sampled at n points of [lo, hi] for plotting; NaN where it isn't real.
#[allow(dead_code)] // API for the calculator views; the GUI/TUI wire it up next
pub fn sample(expr: &str, var: &str, lo: f64, hi: f64, n: usize) -> Result<(Vec<f64>, Vec<f64>), String> {
    let xs = linspace(lo, hi, n);
    let _g = lock();
    let ys = raw_sample(expr, var, "", &xs, &xs)?;
    Ok((xs, ys))
}

/// `plot(...)` at top level -> its arguments.
fn plot_args(expr: &str) -> Option<Vec<String>> {
    let inner = expr.trim().strip_prefix("plot")?.trim_start().strip_prefix('(')?.strip_suffix(')')?;
    let (mut depth, mut args, mut cur) = (0, vec![], String::new());
    for c in inner.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' if depth == 0 => return None, // plot(a) + plot(b)
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                args.push(std::mem::take(&mut cur).trim().to_string());
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    args.push(cur.trim().to_string());
    (depth == 0).then_some(args)
}

/// qalc's plot(expr[, min, max]) drawn into the interpreter's figures instead of gnuplot:
/// a curve, or a 50x50 surface for two unknowns.
fn plot(interp: &mut Interp, args: &[String]) -> Result<Answer, String> {
    let num = |s: &str| -> Result<f64, String> {
        let (a, _) = raw_calc(s, TIMEOUT_MS, PLAIN)?;
        a.value.and_then(|v| v.scalar().ok()).ok_or_else(|| format!("plot: '{s}' is not a real number"))
    };
    let (f, lo, hi) = match args {
        [f] => (f.as_str(), -10.0, 10.0),
        [f, a, b, ..] => (f.as_str(), num(a)?, num(b)?),
        _ => return Err("plot: expected plot(expr) or plot(expr, min, max)".into()),
    };
    if !(lo < hi) {
        return Err("plot: min must be less than max".into());
    }
    let (mut a, _) = raw_calc(f, TIMEOUT_MS, PLAIN)?;
    let label = raw_pretty(f);
    let code = a.octave.take();
    let (series, octave) = match &a.free_vars[..] {
        [] | [_] => {
            let x = a.free_vars.first().map_or("x", String::as_str);
            let xs = linspace(lo, hi, 501);
            let ys = raw_sample(f, x, "", &xs, &xs)?;
            (Series::new(Data::Line { x: xs, y: ys }), code.map(|c| format!("fplot(@({x}) {c}, [{lo} {hi}])")))
        }
        [x, y] => {
            let n = 50;
            let g = linspace(lo, hi, n);
            let mx = Matrix::new(n, n, (0..n * n).map(|k| g[k / n]).collect());
            let my = Matrix::new(n, n, (0..n * n).map(|k| g[k % n]).collect());
            let z = raw_sample(f, x, y, &mx.data, &my.data)?;
            let s = Series::new(Data::Surface { x: mx, y: my, z: Matrix::new(n, n, z), wire: false });
            (s, code.map(|c| format!("fsurf(@({x}, {y}) {c}, [{lo} {hi} {lo} {hi}])")))
        }
        _ => return Err(format!("plot: too many unknowns ({})", a.free_vars.join(", "))),
    };
    let p = &mut interp.plots;
    if !p.figures.is_empty() && p.axes().hold {
        p.axes().add(Series { label: label.clone(), ..series });
    } else {
        // each calculator plot gets its own figure, so earlier answers keep theirs
        let id = p.figures.iter().map(|f| f.id).max().unwrap_or(0) + 1;
        p.figures.push(Figure::new(id));
        p.current = p.figures.len() - 1;
        let ax = p.axes();
        ax.add(Series { label: label.clone(), ..series });
        ax.title = label.clone();
        ax.grid = true;
    }
    let ax = p.axes();
    ax.xlabel = a.free_vars.first().cloned().unwrap_or_else(|| "x".into());
    ax.ylabel = a.free_vars.get(1).cloned().unwrap_or_default();
    ax.legend = ax.series.len() > 1;
    p.touch();
    a.input = raw_pretty(&format!("plot({})", args.join(", ")));
    a.value = None;
    a.octave = octave;
    a.visual = None; // already drawn
    Ok(a)
}

/// Calculator-mode entry shared by GUI/TUI/CLI: Octave's numeric variables are visible to the
/// calculator, numeric results land in Octave (`ans`, or the assigned name), and `plot(...)`
/// draws into `interp.plots`.
pub fn eval_into(interp: &mut Interp, expr: &str) -> Result<Answer, String> {
    let mut pushed = lock();
    push_vars(&mut pushed, interp);
    if let Some(args) = plot_args(expr) {
        return plot(interp, &args);
    }
    let (a, assigned) = raw_calc(expr, TIMEOUT_MS, SET_ANS)?;
    if let Some(v) = &a.value {
        let name = assigned.unwrap_or_else(|| "ans".into());
        if let Some(m) = numeric(v) {
            pushed.insert(name.clone(), m);
        }
        interp.vars.insert(name, v.clone());
    }
    Ok(a)
}

/// Octave-side bridge: qalc, qalc2oct, qalcfn, pretty.
pub fn builtin(name: &str) -> Option<Builtin> {
    let f: Builtin = match name {
        // numeric result, or the result text when symbolic
        "qalc" => |it, a, _| {
            let (a, _) = synced(it, str_arg(a, "qalc")?, |e| raw_calc(e, TIMEOUT_MS, PLAIN))?;
            Ok(vec![a.value.unwrap_or(Value::Str(a.result))])
        },
        "qalc2oct" => |it, a, _| {
            let (a, _) = synced(it, str_arg(a, "qalc2oct")?, |e| raw_calc(e, TIMEOUT_MS, PLAIN))?;
            Ok(vec![Value::Str(a.octave.ok_or_else(|| format!("qalc2oct: '{}' has no Octave equivalent", a.result))?)])
        },
        // f = qalcfn('diff(x^3*sin(x), x)'); fplot(f, [0 5])
        "qalcfn" => |it, a, _| {
            let (a, _) = synced(it, str_arg(a, "qalcfn")?, |e| raw_calc(e, TIMEOUT_MS, PLAIN))?;
            let code = a.octave.ok_or_else(|| format!("qalcfn: '{}' has no Octave equivalent", a.result))?;
            let h = format!("@({}) {code}", a.free_vars.join(", "));
            it.call(&Value::from("str2func"), &[Value::Str(h)], 1)
        },
        "pretty" => |_, a, _| Ok(vec![Value::Str(pretty(str_arg(a, "pretty")?))]),
        _ => return None,
    };
    Some(f)
}

fn str_arg<'a>(args: &'a [Value], f: &str) -> Result<&'a str, String> {
    match args {
        [Value::Str(s)] => Ok(s),
        _ => Err(format!("{f}: expected one string argument")),
    }
}

/// `f(expr)` under the lock, with the interpreter's numeric variables pushed first.
fn synced<T>(it: &Interp, expr: &str, f: impl FnOnce(&str) -> Result<T, String>) -> Result<T, String> {
    let mut pushed = lock();
    push_vars(&mut pushed, it);
    f(expr)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(e: &str) -> Answer {
        calc(e).unwrap_or_else(|err| panic!("{e}: {err}"))
    }
    fn num(a: &Answer) -> f64 {
        a.value.as_ref().unwrap().scalar().unwrap()
    }

    #[test]
    fn exact_results() {
        let a = ok("1/3 + 1/6");
        assert_eq!((a.result.as_str(), a.approx.as_deref()), ("1/2", Some("0.5")));
        assert_eq!(num(&a), 0.5);
        let a = ok("sqrt(8)");
        assert!(a.result.contains('√') && a.approx.as_deref() == Some("2.828427125"), "{a:?}");
        assert_eq!(a.octave.as_deref(), Some("2.*sqrt(2)"));
        assert_eq!(ok("2^100").result, "1267650600228229401496703205376");
        assert_eq!(ok("integrate(sin(x)^2, 0, pi)").result, "π / 2");
    }

    #[test]
    fn units_and_bases() {
        let a = ok("5 km to mi");
        assert_eq!(a.result, "3.106855961 mi");
        assert!((num(&a) - 3.106855961).abs() < 1e-9 && a.octave.is_none());
        assert!(a.alts.contains(&("base units".into(), "5000 m".into())), "{:?}", a.alts);
        let a = ok("255 to hex");
        assert_eq!((a.result.as_str(), num(&a)), ("0xFF", 255.0));
        assert!(a.alts.iter().any(|(l, t)| l == "decimal" && t == "255"));
    }

    #[test]
    fn symbolic() {
        let a = ok("diff(x^3, x)");
        assert_eq!((a.result.as_str(), a.free_vars.clone()), ("3x²", vec!["x".to_string()]));
        assert!(a.value.is_none() && a.approx.is_none());
        let a = ok("integrate(x^2)");
        assert_eq!((a.result.as_str(), a.octave.as_deref()), ("x³ / 3 + C", Some("(x.^3)./3")));
        assert_eq!(a.free_vars, ["x"]); // the integration constant isn't an unknown to plot over
        let a = ok("solve(x^2 = 4)");
        let v = a.value.unwrap().matrix().unwrap();
        assert_eq!((v.rows, v.cols, v.data.iter().sum::<f64>()), (1, 2, 0.0));
    }

    #[test]
    fn matrix_and_complex_values() {
        let v = ok("[1 2; 3 4]^-1").value.unwrap().matrix().unwrap();
        assert_eq!(v, Matrix::from_rows(&[vec![-2., 1.], vec![1.5, -0.5]]));
        let a = ok("sqrt(-4)");
        assert!(matches!(&a.value, Some(Value::Complex(m)) if m.data == [C64::new(0., 2.)]), "{a:?}");
    }

    #[test]
    fn octave_round_trip() {
        assert_eq!(to_octave("x² + 2x").unwrap(), "x.^2 + 2.*x");
        assert_eq!(from_octave("@(x) x.^2 + 2*x"), "x^2 + 2x");
        assert_eq!(from_octave("2./x + 3.*y.^2 - x2*y + 2*e3"), "2/x + 3y^2 - x2*y + 2*e3");
        assert_eq!(from_octave("sign(x) ~= fix(1.5e-3*x)"), "sgn(x) != trunc(1.5e-3x)");
        let code = to_octave("diff(x^3*sin(x), x)").unwrap();
        assert_eq!(to_octave(&from_octave(&code)).unwrap(), code);
        assert!(to_octave("5 m").is_err());
    }

    #[test]
    fn pretty_labels() {
        assert_eq!(pretty("x.^2 + 2*x"), "x² + 2x");
        assert_eq!(pretty("sqrt(2)/2"), "√(2) / 2");
        assert_eq!(pretty("2*pi*r"), "2πr");
        assert_eq!(pretty("Temperature (K)"), "Temperature (K)"); // prose stays prose
        assert_eq!(pretty("1 +* 2"), "1 +* 2");
    }

    #[test]
    fn eval_into_syncs_with_octave() {
        let mut it = Interp::new();
        it.eval("qt_k = 4; qt_m = [1 2 3];").unwrap();
        let a = eval_into(&mut it, "qt_k^2 + total(qt_m)").unwrap();
        assert_eq!(a.result, "22");
        assert_eq!(it.vars["ans"].scalar(), Ok(22.0));
        eval_into(&mut it, "qt_r := sqrt(2)").unwrap();
        assert!((it.vars["qt_r"].scalar().unwrap() - 2f64.sqrt()).abs() < 1e-15);
        assert_eq!(it.vars["ans"].scalar(), Ok(22.0)); // assignments leave ans alone
        // the calculator kept the exact value, not the pushed-back double
        assert_eq!(eval_into(&mut it, "qt_r^2").unwrap().result, "2");
        it.eval("qt_k = 5;").unwrap();
        assert_eq!(eval_into(&mut it, "qt_k").unwrap().result, "5");
        assert!(set_var("m", &Value::num(1.0)).is_err()); // metre stays a unit
        assert!(eval_into(&mut it, "sin(").is_err());
    }

    #[test]
    fn plot_draws_into_figures() {
        let mut it = Interp::new();
        let a = eval_into(&mut it, "plot(x^2, -2, 2)").unwrap();
        assert_eq!(a.octave.as_deref(), Some("fplot(@(x) x.^2, [-2 2])"));
        let ax = &it.plots.figures[0].axes[0];
        assert_eq!((ax.title.as_str(), ax.xlabel.as_str()), ("x²", "x"));
        let Data::Line { x, y } = &ax.series[0].data else { panic!() };
        assert_eq!((x[0], y[0], y[250], x.len()), (-2.0, 4.0, 0.0, 501));
        eval_into(&mut it, "plot(x*y)").unwrap();
        assert!(matches!(it.plots.figures[1].axes[0].series[0].data, Data::Surface { .. }));
        let (_, ys) = sample("diff(x^3, x)", "x", 0.0, 2.0, 3).unwrap();
        assert_eq!(ys, [0.0, 3.0, 12.0]);
    }

    #[test]
    fn octave_builtins() {
        let mut it = Interp::new();
        it.eval("f = qalcfn('diff(x^3, x)'); y = f([1 2]); n = qalc('1/4 + 1/4'); s = qalc2oct('x^2'); p = pretty('x.^2');").unwrap();
        assert_eq!(it.vars["y"].matrix().unwrap().data, [3.0, 12.0]);
        assert_eq!(it.vars["n"].scalar(), Ok(0.5));
        assert_eq!(it.vars["s"].as_str(), Ok("x.^2"));
        assert_eq!(it.vars["p"].as_str(), Ok("x²"));
    }

    #[test]
    fn input_reads_like_qalc() {
        assert_eq!(ok("1/3 + 1/6").input, "1/3 + 1/6");
        assert_eq!(ok("0.5 + 1/3").input, "0.5 + 1/3");
        assert_eq!(ok("2/3*x").input, "(2/3)x");
        assert_eq!(pretty("x/(y+1)"), "x / (y + 1)");
    }

    #[test]
    fn preview_is_fast_and_pure() {
        assert_eq!(preview("1/3 + 1/6").as_deref(), Some("1/2 ≈ 0.5"));
        assert_eq!(preview("3 + 3").as_deref(), Some("6"));
        for e in ["qt_pv := 7", "qt_pv = 7", "plot(x^2)", "1 + plot(x)", "sin("] {
            assert_eq!(preview(e), None, "{e}");
        }
        let t = std::time::Instant::now();
        assert_eq!(preview("(10^7)!"), None);
        assert!(t.elapsed() < std::time::Duration::from_millis(1500), "{:?}", t.elapsed());
        // under one lock hold, so parallel tests can't move ans in between
        let _g = lock();
        raw_calc("41 + 1", TIMEOUT_MS, SET_ANS).unwrap();
        assert_eq!(raw_calc("5 + 5", PREVIEW_MS, PURE).unwrap().0.result, "10");
        assert!(raw_calc("qt_pv := 7", PREVIEW_MS, PURE).is_err());
        assert_eq!(raw_calc("ans", TIMEOUT_MS, PLAIN).unwrap().0.result, "42");
        assert!(raw_calc("qt_pv", TIMEOUT_MS, PLAIN).map_or(true, |(a, _)| a.result != "7"));
    }

    #[test]
    fn visuals() {
        let vis = |e: &str| ok(e).visual.unwrap_or_default();
        assert_eq!(vis("integrate(x^2, 0, 3)"), "intplot(@(x) x.^2, 0, 3)");
        assert_eq!(vis("integrate(x^2, x, 0, 3)"), "intplot(@(x) x.^2, 0, 3)"); // Octave/SymPy order works too
        assert_eq!(ok("integrate(x^2, x, 0, 3)").octave.as_deref(), Some("9"));
        assert_eq!(vis("∫(sin(x)^2, 0, pi)"), "intplot(@(x) sin(x).^2, 0, pi)");
        assert_eq!(vis("integrate(\\t^2, 0, 3)"), "intplot(@(t) t.^2, 0, 3)");
        let both = |x: &str, f: &str, g: &str, lf: &str, lg: &str| {
            format!("figure; fplot(@({x}) {f}, [-5 5]); hold on; fplot(@({x}) {g}, [-5 5]); hold off; legend('{lf}', '{lg}')")
        };
        assert_eq!(vis("integrate(x^2)"), both("x", "x.^2", "(x.^3)./3", "x²", "x³ / 3 + C"));
        assert_eq!(vis("diff(\\t^3, \\t)"), both("t", "t.^3", "3.*(t.^2)", "t³", "3t²"));
        let roots = "figure; fplot(@(x) x.^2 - 4, [-4 4]); hold on; plot([2 -2], 0*[2 -2], 'o'); hold off; grid on";
        assert_eq!(vis("solve(x^2 = 4)"), roots);
        assert!(vis("solve(z^2 - 2z = 3, z)").starts_with("figure; fplot(@(z) z.^2 - (2.*z) - 3, [-3 5]); hold on; plot([3 -1]"));
        assert_eq!(vis("solve(x^2 = -1)"), "figure; fplot(@(x) x.^2 - (-1), [-5 5]); grid on"); // no real roots
        let a = ok("eig([1 2; 3 4])");
        assert_eq!(a.visual.as_deref(), Some("eigplot([1, 2; 3, 4])"));
        assert!(a.approx.as_deref() == Some("[5.372281323  −0.3722813233]") && a.octave.is_some(), "{a:?}");
        assert_eq!(vis("[1 2; 3 4]^-1"), "matview([-2, 1; 3./2, -(1./2)])");
        assert_eq!(vis("x^2 + 1"), "fplot(@(x) x.^2 + 1, [-5 5])");
        assert_eq!(vis("diff(sin(x)*y, x)"), "fsurf(@(x, y) cos(x).*y, [-5 5 -5 5])"); // two unknowns: a surface
        assert!(ok("5 km to mi").visual.is_none() && ok("2 + 2").visual.is_none());
        for e in ["integrate(x^2)", "diff(\\t^3, \\t)", "solve(x^2 = 4)", "[1 2; 3 4]^-1", "∫(sin(x)^2, 0, pi)"] {
            assert!(ok(e).octave.is_some(), "{e}"); // still convertible to Octave
        }
        let mut it = Interp::new();
        assert!(eval_into(&mut it, "plot(x^2)").unwrap().visual.is_none()); // already drawn
        // the ones built from existing Octave plot functions run
        for e in ["integrate(x^2)", "diff(\\t^3, \\t)", "solve(x^2 = 4)", "solve(x^2 = -1)", "x^2 + 1", "diff(sin(x)*y, x)"] {
            it.eval(&vis(e)).unwrap_or_else(|err| panic!("{e}: {err}"));
        }
        assert_eq!(it.plots.figures.len(), 5);
    }

    #[test]
    fn octave_forms_to_math() {
        assert_eq!(from_octave("exp(1) + log(x) - log10(100)"), "e + ln(x) - log10(100)");
        assert_eq!(from_octave("[1 2; 3 4]' + A.'"), "transpose([1 2; 3 4]) + transpose(A)");
        assert_eq!(from_octave("2j + 3.5i*x + abs(f(x)')"), "2i + 3.5ix + abs(transpose(f(x)))");
        assert_eq!(from_octave("@(t) sqrt(t).*pi./1e-3"), "sqrt(t)*pi/1e-3");
        let v = |e: &str| ok(&from_octave(e)).value.unwrap_or_else(|| panic!("{e}"));
        assert_eq!(v("[1 2; 3 4]'").matrix().unwrap(), Matrix::from_rows(&[vec![1., 3.], vec![2., 4.]]));
        assert_eq!(v("log(exp(1)) + log10(1e3) + sqrt(abs(-16))").scalar(), Ok(8.0));
        assert!(matches!(v("2j + 1i"), Value::Complex(m) if m.data == [C64::new(0., 3.)]));
        assert!((v("pi*1e-3").scalar().unwrap() - std::f64::consts::PI / 1000.0).abs() < 1e-15);
        for m in ["x^2 + 2x", "ln(x)/x", "sin(x)e^(-x)", "[1 2; 3 4]^-1", "sqrt(-4) + abs(x)"] {
            let code = to_octave(m).unwrap();
            assert_eq!(to_octave(&from_octave(&code)).unwrap(), code, "{m}");
        }
    }

    #[test]
    fn timeout_and_garbage() {
        let _g = lock();
        let e = raw_calc("(10^7)!", 200, PLAIN).unwrap_err();
        assert!(e.contains("timed out"), "{e}");
        assert_eq!(raw_calc("1+1", TIMEOUT_MS, PLAIN).unwrap().0.result, "2"); // still alive
        for g in ["sin(", ")(", "1/", "\u{1}\u{2}", "[[1, 2], [3]]"] {
            let _ = raw_calc(g, TIMEOUT_MS, PLAIN); // must not crash
        }
        assert!(raw_calc("sin(", TIMEOUT_MS, PLAIN).is_err());
        assert!(raw_calc("a\0b", TIMEOUT_MS, PLAIN).is_err());
    }
}
