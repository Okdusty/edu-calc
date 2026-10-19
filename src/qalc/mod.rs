//! Calculator mode over libqalculate (C++ shim in shim.cpp, built by build.rs): math notation,
//! exact results, units, symbolic calculus, and conversion both ways with Octave.
//! One global calculator; every call into it holds `PUSHED`'s lock.
use std::collections::BTreeMap;
use std::ffi::{CStr, CString, c_char, c_int};
use std::path::Path;
use std::sync::{Mutex, MutexGuard, OnceLock};

use gtk4::glib;

use crate::lang::Interp;
use crate::plot::{Data, Figure, Series, Source};
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
    fn qs_options(angle: c_int, approx: c_int, precision: c_int, fraction: c_int, notation: c_int, base: c_int, input_base: c_int, twos: c_int, bits: c_int);
    fn qs_catalog() -> *mut c_char;
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

/// What a catalog entry is.
#[allow(dead_code)] // used by complete.rs and the frontends
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Function,
    /// variables and constants (pi, e, c, ...)
    Variable,
    Unit,
}

/// One libqalculate name for completion (each alternative name gets its own entry).
#[allow(dead_code)] // used by complete.rs and the frontends
#[derive(Clone, Debug)]
pub struct CatalogItem {
    pub name: String,
    pub kind: ItemKind,
    /// text that replaces the typed prefix: "integrate(□, □, □)" (required arguments as □), "pi", "km"
    pub insert: String,
    /// "integrate(f, a, b, x = x)" for functions, "km (kilometer, length)" style for units
    pub signature: String,
    /// one short line: "Integrate", "Archimedes' constant (3.14159…)"
    pub desc: String,
}

/// Every active function, variable/constant and unit name of libqalculate, built once (lazily).
#[allow(dead_code)] // used by complete.rs and the frontends
pub fn catalog() -> &'static [CatalogItem] {
    static CATALOG: OnceLock<Vec<CatalogItem>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let s = {
            let _g = lock();
            take(unsafe { qs_catalog() }).unwrap_or_default()
        };
        list(&s)
            .iter()
            .filter_map(|r| {
                let [kind, name, insert, signature, desc] = r.split('\x1f').collect::<Vec<_>>()[..] else { return None };
                let kind = match kind {
                    "f" => ItemKind::Function,
                    "v" => ItemKind::Variable,
                    _ => ItemKind::Unit,
                };
                Some(CatalogItem { name: name.into(), kind, insert: insert.into(), signature: signature.into(), desc: desc.into() })
            })
            .collect()
    })
}

/// Calculator settings (qcalcp.ini [calc]; `set KEY VALUE` in the calculator entry changes them
/// live, like qalc's `set`). Defaults are qalc's.
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// "rad" | "deg" | "gra"
    pub angle: String,
    /// "exact" | "try" (try exact, the default) | "approx"
    pub approx: String,
    /// significant digits of approximate results
    pub precision: u32,
    /// "auto" | "decimal" | "fraction" | "mixed"
    pub fraction: String,
    /// "auto" | "normal" | "sci" | "eng"
    pub notation: String,
    /// result base 2..=36 (2 bin, 8 oct, 10 dec, 16 hex)
    pub base: u32,
    /// base numbers are typed in (A-F digits need 16)
    pub input_base: u32,
    /// negative numbers in bin (and in hex when `bits` is set) as two's complement
    pub twos_complement: bool,
    /// bit width for bin/hex display and bitcmp/bitrot: 0 = automatic, else 8/16/32/64/128
    pub bits: u32,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            angle: "rad".into(),
            approx: "try".into(),
            precision: 10,
            fraction: "auto".into(),
            notation: "auto".into(),
            base: 10,
            input_base: 10,
            twos_complement: true,
            bits: 0,
        }
    }
}

/// The Options keys, as in qcalcp.ini [calc].
const KEYS: [&str; 9] = ["angle", "approx", "precision", "fraction", "notation", "base", "input_base", "twos_complement", "bits"];

/// A key or one of qalc's names for it ("inbase", "input base", "two's", "exp" ...) -> the Options key.
fn canon(key: &str) -> Option<&'static str> {
    let k = key.trim().to_ascii_lowercase().replace('\'', "").replace([' ', '-'], "_");
    Some(match k.as_str() {
        "angle" | "angle_unit" => "angle",
        "approx" | "approximation" => "approx",
        "precision" | "prec" => "precision",
        "fraction" | "fractions" | "frac" => "fraction",
        "notation" | "exp" | "exp_mode" => "notation",
        "base" | "outbase" | "out_base" | "output_base" => "base",
        "input_base" | "inbase" | "in_base" => "input_base",
        "twos" | "twos_complement" => "twos_complement",
        "bits" | "binary_bits" => "bits",
        _ => return None,
    })
}

impl Options {
    /// `set KEY VALUE` / ini keys: angle, approx, precision, fraction, notation, base, input_base,
    /// twos_complement, bits. Values as in the field docs, plus qalc's aliases (deg, degrees,
    /// hex, bin, on/off ...). Err names the problem.
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        let k = canon(key).ok_or_else(|| format!("unknown setting '{}' (one of {})", key.trim(), KEYS.join(", ")))?;
        let v = value.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase();
        let err = |want: &str| format!("{k}: expected {want} (got '{}')", value.trim());
        // (aliases, value) table lookup
        let pick = |t: &[(&[&str], &str)], want: &str| {
            t.iter().find(|(names, _)| names.contains(&v.as_str())).map(|(_, c)| c.to_string()).ok_or_else(|| err(want))
        };
        let base = || match v.as_str() {
            "bin" | "binary" => Ok(2),
            "oct" | "octal" => Ok(8),
            "dec" | "decimal" => Ok(10),
            "hex" | "hexadecimal" => Ok(16),
            n => n.parse().ok().filter(|b| (2..=36).contains(b)).ok_or_else(|| err("bin, oct, dec, hex or 2..36")),
        };
        match k {
            "angle" => {
                let t: &[(&[&str], &str)] = &[(&["rad", "radian", "radians"], "rad"), (&["deg", "degree", "degrees"], "deg"), (&["gra", "grad", "gradian", "gradians"], "gra")];
                self.angle = pick(t, "rad, deg or gra")?
            }
            "approx" => {
                let t: &[(&[&str], &str)] = &[(&["try", "try exact", "auto", "off"], "try"), (&["exact", "always exact"], "exact"), (&["approx", "approximate", "on"], "approx")];
                self.approx = pick(t, "exact, try or approx")?
            }
            "precision" => self.precision = v.parse().ok().filter(|p| (1..=1000).contains(p)).ok_or_else(|| err("1..1000"))?,
            "fraction" => {
                let t: &[(&[&str], &str)] = &[(&["auto"], "auto"), (&["decimal", "dec", "off"], "decimal"), (&["fraction", "fractional", "on"], "fraction"), (&["mixed", "combined"], "mixed")];
                self.fraction = pick(t, "auto, decimal, fraction or mixed")?
            }
            "notation" => {
                let t: &[(&[&str], &str)] = &[(&["auto"], "auto"), (&["normal", "off", "none"], "normal"), (&["sci", "scientific"], "sci"), (&["eng", "engineering"], "eng")];
                self.notation = pick(t, "auto, normal, sci or eng")?
            }
            "base" => self.base = base()?,
            "input_base" => self.input_base = base()?,
            "twos_complement" => {
                let t: &[(&[&str], &str)] = &[(&["on", "yes", "true", "1"], "on"), (&["off", "no", "false", "0"], "off")];
                self.twos_complement = pick(t, "on or off")? == "on"
            }
            _ => {
                let b = if v == "auto" { Some(0) } else { v.parse().ok().filter(|b| [0, 8, 16, 32, 64, 128].contains(b)) };
                self.bits = b.ok_or_else(|| err("auto, 8, 16, 32, 64 or 128"))?
            }
        }
        Ok(())
    }

    /// Value of a key (as `canon` names it) in the form `set` and the ini take.
    fn get(&self, key: &str) -> String {
        let base = |b: u32| match b {
            2 => "bin".into(),
            8 => "oct".into(),
            10 => "dec".into(),
            16 => "hex".into(),
            b => b.to_string(),
        };
        match key {
            "angle" => self.angle.clone(),
            "approx" => self.approx.clone(),
            "precision" => self.precision.to_string(),
            "fraction" => self.fraction.clone(),
            "notation" => self.notation.clone(),
            "base" => base(self.base),
            "input_base" => base(self.input_base),
            "twos_complement" => (if self.twos_complement { "on" } else { "off" }).into(),
            _ if self.bits == 0 => "auto".into(),
            _ => self.bits.to_string(),
        }
    }

    /// The [calc] keys of `path` (missing file or keys = defaults) and warnings for bad values.
    #[allow(dead_code)] // used by the frontends
    pub fn load_ini(path: &Path) -> (Options, String) {
        let mut o = Options::default();
        let Ok(text) = std::fs::read_to_string(path) else { return (o, String::new()) };
        let kf = glib::KeyFile::new();
        if let Err(e) = kf.load_from_data(&text, glib::KeyFileFlags::NONE) {
            return (o, format!("qcalcp.ini: {e}; default calculator settings\n"));
        }
        let mut warn = String::new();
        for k in KEYS {
            if let Ok(v) = kf.string("calc", k)
                && let Err(e) = o.set(k, &v)
            {
                warn += &format!("qcalcp.ini: [calc] {e}; using the default\n");
            }
        }
        (o, warn)
    }

    /// Write these keys into [calc] of `path`, keeping every other key and comment.
    #[allow(dead_code)] // used by the frontends
    pub fn save_ini(&self, path: &Path) -> Result<(), String> {
        let kf = glib::KeyFile::new();
        match std::fs::read_to_string(path) {
            // a file we can't parse is the user's; never clobber it
            Ok(text) => kf.load_from_data(&text, glib::KeyFileFlags::KEEP_COMMENTS).map_err(|e| format!("{}: {e}; not saving calculator settings", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
        for k in KEYS {
            kf.set_string("calc", k, &self.get(k));
        }
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        kf.save_to_file(path).map_err(|e| e.to_string())
    }
}

/// Settings in use (None = defaults, nothing set yet).
static OPTIONS: Mutex<Option<Options>> = Mutex::new(None);

/// Settings in use.
pub fn options() -> Options {
    OPTIONS.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default()
}

/// Use `o` for every calculation from now on (all frontends share the one calculator).
#[allow(dead_code)] // used by the frontends
pub fn set_options(o: &Options) {
    let _g = lock();
    apply(o);
}

/// set_options with the lock held.
fn apply(o: &Options) {
    apply_raw(o);
    *OPTIONS.lock().unwrap_or_else(|e| e.into_inner()) = Some(o.clone());
}

/// Change only the shim while holding PUSHED; scoped worker options must not be saved
/// by a simultaneous GUI close or exposed to the Advanced options menu.
fn apply_raw(o: &Options) {
    // index in the shim's tables, where 0 is the default
    let ix = |v: &str, all: &[&str]| all.iter().position(|a| *a == v).unwrap_or(0) as c_int;
    unsafe {
        qs_options(
            ix(&o.angle, &["rad", "deg", "gra"]),
            ix(&o.approx, &["try", "exact", "approx"]),
            o.precision as c_int,
            ix(&o.fraction, &["auto", "decimal", "fraction", "mixed"]),
            ix(&o.notation, &["auto", "normal", "sci", "eng"]),
            o.base as c_int,
            o.input_base as c_int,
            o.twos_complement as c_int,
            o.bits as c_int,
        )
    };
}

/// `set KEY VALUE` (qalc's settings command; KEY may be several words: `set input base hex`) ->
/// (key, value). None when `expr` isn't one.
fn set_command(expr: &str) -> Option<(String, String)> {
    let w: Vec<&str> = expr.split_whitespace().collect();
    if w.len() < 2 || !w[0].eq_ignore_ascii_case("set") {
        return None;
    }
    let k = (2..w.len()).find(|&k| canon(&w[1..k].join(" ")).is_some()).unwrap_or(2);
    Some((w[1..k].join(" "), w[k..].join(" ")))
}

/// Apply a `set KEY VALUE` command (lock held): an Answer like "set angle deg" = "angle = deg".
fn raw_set_command(expr: &str) -> Option<Result<Answer, String>> {
    let (key, value) = set_command(expr)?;
    let mut o = options();
    Some(o.set(&key, &value).map(|()| {
        let k = canon(&key).unwrap_or_default();
        apply(&o);
        Answer { input: format!("set {k} {value}"), result: format!("{k} = {}", o.get(k)), ..Answer::default() }
    }))
}

/// An integer answer in the four programming bases, in this order: ("HEX", "FF"), ("DEC", "255"),
/// ("OCT", "377"), ("BIN", "1111 1111"); honors twos_complement and bits. None when the answer
/// isn't a single integer.
#[allow(dead_code)] // used by the frontends
pub fn bases(a: &Answer) -> Option<Vec<(&'static str, String)>> {
    // exact digits: the decimal form is the result or, in another output base, an alternative
    let dec = a.alts.iter().find(|(l, _)| l == "decimal").map_or(&a.result, |(_, t)| t);
    let n: i128 = dec.replace('−', "-").parse().ok()?;
    let o = options();
    // smallest byte-multiple width that holds n (two's complement for negatives), at least `bits`
    let fits = |w: u32| if n < 0 { w >= 128 || n >= -(1i128 << (w - 1)) } else { w >= 128 || n < 1i128 << w };
    let width = [8, 16, 32, 64, 128].into_iter().find(|&w| w >= o.bits && fits(w)).unwrap_or(128);
    let (sign, u) = if n < 0 && !o.twos_complement { ("-", n.unsigned_abs()) } else { ("", (n as u128) & (u128::MAX >> (128 - width))) };
    let w = width as usize;
    let hex = if o.bits > 0 || (n < 0 && o.twos_complement) { format!("{u:0w$X}", w = w / 4) } else { format!("{u:X}") };
    // BIN padded to the width, in groups of 4 like qalc
    let bin = format!("{u:0w$b}");
    let bin = bin.as_bytes().chunks(4).map(String::from_utf8_lossy).collect::<Vec<_>>().join(" ");
    Some(vec![("HEX", format!("{sign}{hex}")), ("DEC", n.to_string()), ("OCT", format!("{sign}{u:o}")), ("BIN", format!("{sign}{bin}"))])
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
    if set_command(expr).is_some() {
        return None;
    }
    let _g = lock();
    let (a, _) = raw_calc(expr, PREVIEW_MS, PURE).ok()?;
    Some(match a.approx {
        Some(x) => format!("{} ≈ {x}", a.result),
        None => a.result,
    })
}

/// Calculate in math notation; handles qalc's "expr to unit/base/fraction/hex ..." conversions
/// and `set KEY VALUE`.
#[allow(dead_code)] // API for the calculator views; the GUI/TUI wire it up next
pub fn calc(expr: &str) -> Result<Answer, String> {
    let _g = lock();
    if let Some(r) = raw_set_command(expr) {
        return r;
    }
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

/// Plain, sendable answer for the classroom worker. PURE evaluation never changes ans,
/// variables or files. Options are scoped to the call, including on an error.
#[derive(Clone, Debug)]
pub struct ClassroomAnswer {
    pub input: String,
    pub result: String,
    pub approx: String,
    pub free_vars: Vec<String>,
}

pub fn classroom_eval(expr: &str, opts: &Options) -> Result<ClassroomAnswer, String> {
    let _g = lock();
    let previous = options();
    apply_raw(opts);
    let result = raw_calc(expr, TIMEOUT_MS, PURE).map(|(a, _)| ClassroomAnswer {
        input: a.input, result: a.result, approx: a.approx.unwrap_or_default(), free_vars: a.free_vars,
    });
    apply_raw(&previous);
    result
}

/// Explicit y=f(x), sampled without changing the advanced calculator's options.
pub fn classroom_curve(expr: &str, opts: &Options, lo: f64, hi: f64) -> Result<(Vec<f64>, Vec<f64>), String> {
    let _g = lock();
    let previous = options();
    apply_raw(opts);
    let result = (|| {
        let (answer, _) = raw_calc(expr, PREVIEW_MS, PURE)?;
        if answer.free_vars.iter().any(|v| v != "x") {
            return Err("Use a function of x, for example y = 2x + 1.".into());
        }
        // the Simple graph asks for twice the view: 1000 samples across what's shown
        let xs = linspace(lo, hi, 2001);
        let ys = raw_sample(expr, "x", "", &xs, &xs)?;
        Ok(crate::plot::builtins::break_jumps(&xs, &ys))
    })();
    apply_raw(&previous);
    result
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
            // the source lets zooming out extend the curve
            let s = Series { source: Some(Source::Qalc { expr: f.to_string(), var: x.to_string() }), ..Series::new(Data::Line { x: xs, y: ys }) };
            (s, code.map(|c| format!("fplot(@({x}) {c}, [{lo} {hi}])")))
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
        p.fig().code.extend(octave.clone());
    } else {
        // each calculator plot gets its own figure, so earlier answers keep theirs
        let id = p.figures.iter().map(|f| f.id).max().unwrap_or(0) + 1;
        p.figures.push(Figure::new(id));
        p.current = p.figures.len() - 1;
        p.fig().code = octave.iter().cloned().collect();
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
/// draws into `interp.plots`. `set KEY VALUE` changes the settings (persisting them is the frontend's job).
pub fn eval_into(interp: &mut Interp, expr: &str) -> Result<Answer, String> {
    let mut pushed = lock();
    if let Some(r) = raw_set_command(expr) {
        return r;
    }
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

    #[test]
    fn classroom_options_and_curves() {
        let previous = options();
        let degrees = Options { angle: "deg".into(), ..Default::default() };
        assert_eq!(classroom_eval("sin(30)", &degrees).unwrap().result, "1/2");
        assert_eq!(classroom_eval("sin(pi/2)", &Options::default()).unwrap().result, "1");
        assert_eq!(classroom_eval("3/4 + 1/2", &degrees).unwrap().result, "5/4");
        assert!(classroom_eval("sin(", &degrees).is_err());
        assert_eq!(options(), previous);
        let (xs, ys) = classroom_curve("x²", &degrees, -2.0, 2.0).unwrap();
        assert_eq!(xs.len(), ys.len());
        assert!((ys[0] - 4.0).abs() < 1e-9);
        let (_, ys) = classroom_curve("sin(x)", &degrees, 0.0, 90.0).unwrap();
        assert!((ys.last().unwrap() - 1.0).abs() < 1e-9);
        let (_, ys) = classroom_curve("1/x", &degrees, -1.0, 1.0).unwrap();
        assert!(ys.iter().any(|y| y.is_nan()));
        assert!(classroom_curve("x+y", &degrees, -1.0, 1.0).is_err());
        assert!(classroom_curve("3", &degrees, -1.0, 1.0).unwrap().1.iter().all(|y| *y == 3.0));
        assert_eq!(options(), previous);
    }

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
        // zooming out can resample it; the figure records its Octave code
        assert!(matches!(&ax.series[0].source, Some(Source::Qalc { expr, var }) if expr == "x^2" && var == "x"));
        assert_eq!(it.plots.figures[0].code, ["fplot(@(x) x.^2, [-2 2])"]);
        it.eval("hold on").unwrap();
        eval_into(&mut it, "plot(2x, -1, 1)").unwrap();
        it.eval("hold off").unwrap();
        assert_eq!(it.plots.figures[0].code, ["fplot(@(x) x.^2, [-2 2])", "hold on;", "fplot(@(x) 2.*x, [-1 1])", "hold off;"]);
        eval_into(&mut it, "plot(x*y)").unwrap();
        let f = &it.plots.figures[1];
        assert!(matches!(f.axes[0].series[0].data, Data::Surface { .. }) && f.axes[0].series[0].source.is_none());
        assert_eq!(f.code, ["fsurf(@(x, y) x.*y, [-10 10 -10 10])"]);
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
        for e in ["qt_pv := 7", "qt_pv = 7", "plot(x^2)", "1 + plot(x)", "sin(", "set angle deg"] {
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
            format!("figure; fplot(@({x}) {f}, [-2 2]); hold on; fplot(@({x}) {g}, [-2 2]); hold off; legend('{lf}', '{lg}'); keypoints on")
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
        assert_eq!(vis("x^2 + 1"), "fplot(@(x) x.^2 + 1, [-2 2]); keypoints on"); // around the vertex
        assert_eq!(vis("x^2 - 4x + 3"), "fplot(@(x) x.^2 - (4.*x) + 3, [0 4]); keypoints on"); // roots 1, 3, vertex 2
        assert_eq!(vis("sin(x)"), "fplot(@(x) sin(x), [-5 5]); keypoints on"); // not a polynomial: the default range
        assert_eq!(vis("diff(sin(x)*y, x)"), "fsurf(@(x, y) cos(x).*y, [-5 5 -5 5])"); // two unknowns: a surface
        assert!(ok("5 km to mi").visual.is_none() && ok("2 + 2").visual.is_none());
        for e in ["integrate(x^2)", "diff(\\t^3, \\t)", "solve(x^2 = 4)", "[1 2; 3 4]^-1", "∫(sin(x)^2, 0, pi)"] {
            assert!(ok(e).octave.is_some(), "{e}"); // still convertible to Octave
        }
        let mut it = Interp::new();
        assert!(eval_into(&mut it, "plot(x^2)").unwrap().visual.is_none()); // already drawn
        // the ones built from existing Octave plot functions run (keypoints is the plot side's)
        for e in ["integrate(x^2)", "diff(\\t^3, \\t)", "solve(x^2 = 4)", "solve(x^2 = -1)", "x^2 + 1", "diff(sin(x)*y, x)"] {
            it.eval(vis(e).trim_end_matches("; keypoints on")).unwrap_or_else(|err| panic!("{e}: {err}"));
        }
        assert_eq!(it.plots.figures.len(), 5);
    }

    #[test]
    fn extrema_and_key_points() {
        let a = ok("extrema(x^3 - 3x)");
        assert_eq!(a.result, "[1  −1]");
        assert_eq!(a.visual.as_deref(), Some("figure; fplot(@(x) x.^3 - (3.*x), [-3 3]); keypoints on"));
        let a = ok("inflections(x^4 - x^2)");
        assert_eq!(a.approx.as_deref(), Some("[0.4082482905  −0.4082482905]"));
        assert_eq!(a.visual.as_deref(), Some("figure; fplot(@(x) x.^4 - (x.^2), [-2 2]); keypoints on"));
        let a = ok("extrema(\\t^2 - 2\\t, \\t)");
        assert_eq!((a.result.as_str(), a.visual.as_deref()), ("1", Some("figure; fplot(@(t) t.^2 - (2.*t), [-1 3]); keypoints on")));
        let mut it = Interp::new();
        it.eval(a.visual.unwrap().trim_end_matches("; keypoints on")).unwrap();
    }

    #[test]
    fn programming() {
        for (e, want) in [
            ("5 & 3", "1"),
            ("5 | 3", "7"),
            ("12 xor 10", "6"),
            ("~5", "−6"),
            ("1 << 4", "16"),
            ("256 >> 2", "64"),
            ("bitcmp(5)", "250"),
            ("bitrot(1, 1)", "2"),
            ("17 mod 5", "2"),
            ("17 // 5", "3"),
            ("log2(1024)", "10"),
            ("1 && 0", "0"),
            ("1 || 0", "1"),
            ("!0", "1"),
        ] {
            assert_eq!(ok(e).result, want, "{e}");
            assert_eq!(preview(e).as_deref(), Some(want), "{e}");
        }
        // bases() reads the global settings: hold the lock so no test changes them meanwhile
        let _g = lock();
        let b = |e: &str| bases(&raw_calc(e, TIMEOUT_MS, PLAIN).unwrap().0);
        let s = |v: &[&str; 4]| Some(["HEX", "DEC", "OCT", "BIN"].into_iter().zip(v.map(String::from)).collect::<Vec<_>>());
        assert_eq!(b("255"), s(&["FF", "255", "377", "1111 1111"]));
        assert_eq!(b("-5 to hex"), s(&["FB", "-5", "373", "1111 1011"])); // two's complement
        assert_eq!(b("2^64 - 1").unwrap()[0].1, "FFFFFFFFFFFFFFFF");
        for e in ["1/3", "2.5", "5 m", "x", "[1 2]"] {
            assert_eq!(b(e), None, "{e}");
        }
        // other bases stay on offer whatever the result is shown in
        let a = raw_calc("255 to bin", TIMEOUT_MS, PLAIN).unwrap().0;
        assert!(["decimal", "hex", "oct"].iter().all(|l| a.alts.iter().any(|(k, _)| k == l)), "{:?}", a.alts);
    }

    #[test]
    fn settings() {
        // settings are global: one lock hold, and the defaults back even if an assert fails
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                apply(&Options::default());
            }
        }
        let _g = lock();
        let _r = Reset;
        let run = |e: &str| match raw_set_command(e) {
            Some(r) => r.unwrap_or_else(|err| panic!("{e}: {err}")),
            None => raw_calc(e, TIMEOUT_MS, PLAIN).unwrap_or_else(|err| panic!("{e}: {err}")).0,
        };
        let a = run("set angle deg");
        assert_eq!((a.input.as_str(), a.result.as_str(), a.value.is_none(), a.visual.is_none()), ("set angle deg", "angle = deg", true, true));
        assert_eq!(run("sin(30)").result, "1/2");
        assert_eq!(run("asin(x)").octave.as_deref(), Some("asind(x)")); // Octave code keeps degrees
        assert_eq!(run("set angle radians").result, "angle = rad");
        run("set approx on");
        assert_eq!(run("sqrt(8)").result, "2.828427125");
        run("set approx try");
        run("set fraction mixed");
        assert_eq!(run("7/3").result, "2 + 1/3");
        run("set fractions off");
        assert_eq!(run("1/4").result, "0.25");
        run("set fraction auto");
        run("set precision 20");
        assert_eq!(run("pi").approx.as_deref(), Some("3.1415926535897932385"));
        run("set prec 10");
        run("set exp eng");
        assert_eq!(run("123456.0").result, "123.456E3");
        run("set notation auto");
        assert_eq!(run("set input base hex").result, "input_base = hex");
        assert_eq!(run("FF + 1").result, "256");
        assert_eq!(run("A & B").result, "10");
        run("set inbase dec");
        run("set base hex");
        let a = run("-1");
        assert_eq!((a.result.as_str(), run("255").result.as_str()), ("−0x1", "0xFF"));
        assert!(a.alts.contains(&("decimal".into(), "−1".into())), "{:?}", a.alts);
        run("set bits 16");
        assert_eq!(run("-1").result, "0xFFFF"); // two's complement at a fixed width
        assert_eq!(run("set bits auto").result, "bits = auto");
        run("set base bin");
        assert_eq!(run("-1").result, "1111 1111");
        run("set twos off");
        assert_eq!(run("-1").result, "−0000 0001");
        assert_eq!(run("5").result, "0000 0101");
        run("set bits 16");
        assert_eq!(run("bitcmp(5)").result, "1111 1111 1111 1010"); // bitcmp's width follows bits
        assert_eq!(bases(&run("5")).unwrap()[0].1, "0005");
        assert_eq!(options(), Options { base: 2, twos_complement: false, bits: 16, ..Options::default() });
        for bad in ["set foo bar", "set angle sideways", "set precision 0", "set base 1", "set bits 12", "set angle"] {
            assert!(raw_set_command(bad).unwrap().is_err(), "{bad}");
        }
        assert!(raw_set_command("set angle sideways").unwrap().unwrap_err().contains("sideways"));
        assert!(raw_set_command("settle 5").is_none() && raw_set_command("set").is_none());
    }

    #[test]
    fn options_ini() {
        let mut o = Options::default();
        for (k, v) in [("angle", "degrees"), ("approx", "approximate"), ("precision", "20"), ("fraction", "on"), ("exp", "engineering")] {
            o.set(k, v).unwrap();
        }
        for (k, v) in [("outbase", "hex"), ("input base", "bin"), ("two's", "off"), ("bits", "32")] {
            o.set(k, v).unwrap();
        }
        let want = Options { angle: "deg".into(), approx: "approx".into(), precision: 20, fraction: "fraction".into(), notation: "eng".into(), base: 16, input_base: 2, twos_complement: false, bits: 32 };
        assert_eq!(o, want);
        // [calc] round-trips; the user's other keys and comments stay; bad values are named
        let dir = std::env::temp_dir().join(format!("qcalcp-ini-{}", std::process::id()));
        let path = dir.join("qcalcp.ini");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "# mine\n[calc]\n# keep me\nauto_visual=false\nangle=gradians\nbits=7\n").unwrap();
        let (l, w) = Options::load_ini(&path);
        assert!(l.angle == "gra" && l.bits == 0 && w.contains("bits") && w.contains("'7'"), "{w}");
        o.save_ini(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(["# mine", "# keep me", "auto_visual=false", "base=hex", "twos_complement=off"].iter().all(|k| text.contains(k)), "{text}");
        assert_eq!(Options::load_ini(&path), (want, String::new()));
        // never clobber a file that doesn't parse
        std::fs::write(&path, "[calc\nbroken").unwrap();
        assert!(o.save_ini(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[calc\nbroken");
        assert_eq!(Options::load_ini(&dir.join("missing.ini")), (Options::default(), String::new()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn catalog_entries() {
        let c = catalog();
        let get = |n: &str| c.iter().find(|i| i.name == n).unwrap_or_else(|| panic!("{n}"));
        let i = get("integrate");
        assert_eq!((i.kind, i.insert.as_str(), i.desc.as_str()), (ItemKind::Function, "integrate(□)", "Integrate"));
        assert!(i.signature.starts_with("integrate(function, [lower limit], [upper limit], "), "{}", i.signature);
        assert_eq!(get("integral").insert, "integral(□)"); // alternative names too
        assert_eq!(get("parallel").insert, "parallel(□, □)");
        assert_eq!(get("diff").signature, "diff(function, [with respect to], order = 1, [variable value])");
        assert_eq!((get("eig").signature.as_str(), get("eig").desc.as_str()), ("eig(matrix)", "Eigenvalues"));
        assert_eq!(get("extrema").signature, "extrema(function, variable = x)");
        assert!(get("inflections").kind == ItemKind::Function && get("bitcmp").insert == "bitcmp(□)");
        assert_eq!(get("log_2").insert, "log2(□)"); // completion-only alias inserts the real name
        let pi = get("pi");
        assert_eq!((pi.kind, pi.insert.as_str(), pi.desc.as_str()), (ItemKind::Variable, "pi", "Archimedes' Constant (pi) ≈ 3.14159"));
        assert!(get("speed_of_light").desc.ends_with("≈ 2.99792E8 m/s"));
        let m = get("meter");
        assert_eq!((m.kind, m.signature.as_str(), m.desc.as_str()), (ItemKind::Unit, "meter", "Meter (length)"));
        assert_eq!(get("USD").desc, "U.S. Dollar (currency)");
        // typeable names only: no π, ∫, plurals or 1-letter names
        assert!(c.iter().all(|i| i.name.len() >= 2 && i.name.is_ascii()) && !c.iter().any(|i| i.name == "meters" || i.name == "ans"));
        assert!(c.len() > 1000, "{}", c.len());
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
