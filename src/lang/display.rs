//! Octave-style value display (`format short` / `format long`).
use std::cell::Cell;

use crate::value::{C64, CMatrix, Func, Matrix, Value};

thread_local! {
    // ponytail: display() has no Interp, so the format mode is per thread; fine with one interpreter per thread
    static LONG: Cell<bool> = const { Cell::new(false) };
}

/// `format long` (16 significant digits) or `format short` (5).
pub fn set_long(on: bool) {
    LONG.with(|l| l.set(on));
}

fn prec() -> i32 {
    if LONG.with(Cell::get) { 16 } else { 5 }
}

fn digits(x: f64) -> i32 {
    if x == 0.0 { 0 } else { x.abs().log10().floor() as i32 + 1 }
}

/// Octave's integer/fraction digit split for a number with `d` integer digits.
fn ld_rd(d: i32, p: i32) -> (i32, i32) {
    match d {
        _ if d > 0 => (d, if p > d { p - d } else { p }),
        _ if d < 0 => (1, p - d),
        _ => (1, p - 1),
    }
}

enum Fmt {
    Int,
    Fixed(usize),
    Exp(usize),
}

/// Digits of the largest and smallest finite magnitudes, and whether all finite values are integers.
fn stats(data: &[f64]) -> (i32, i32, bool) {
    let fin = data.iter().copied().filter(|x| x.is_finite());
    let max = fin.clone().fold(0.0f64, |a, x| a.max(x.abs()));
    let min = fin.clone().fold(f64::INFINITY, |a, x| a.min(x.abs()));
    let min = if min.is_finite() { min } else { 0.0 };
    (digits(max), digits(min), fin.clone().all(|x| x.fract() == 0.0))
}

/// One format shared by all elements (Octave picks it from the largest and smallest magnitudes).
fn choose(data: &[f64]) -> Fmt {
    let (dmax, dmin, int) = stats(data);
    fmt_for(dmax, dmin, int)
}

/// Complex matrices: Octave takes the larger digit count of the real/imaginary maxima, and of the minima.
fn cchoose(m: &CMatrix) -> Fmt {
    let ((rmax, rmin, rint), (imax, imin, iint)) = (stats(&m.re().data), stats(&m.im().data));
    fmt_for(rmax.max(imax), rmin.max(imin), rint && iint)
}

fn fmt_for(dmax: i32, dmin: i32, int: bool) -> Fmt {
    let p = prec();
    if int {
        return if dmax > p + 5 { Fmt::Exp(p as usize - 1) } else { Fmt::Int };
    }
    let (l1, r1) = ld_rd(dmax, p);
    let (l2, r2) = ld_rd(dmin, p);
    let (ld, rd) = (l1.max(l2), r1.max(r2));
    if 1 + ld + 1 + rd > p + 4 { Fmt::Exp(p as usize - 1) } else { Fmt::Fixed(rd as usize) }
}

fn exp_fmt(x: f64, decimals: usize) -> String {
    let s = format!("{x:.decimals$e}");
    let (m, e) = s.split_once('e').unwrap_or((&s, "0"));
    let e: i32 = e.parse().unwrap_or(0);
    format!("{m}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs())
}

fn num(x: f64, f: &Fmt) -> String {
    match *f {
        _ if x.is_nan() => "NaN".into(),
        _ if x.is_infinite() => (if x > 0.0 { "Inf" } else { "-Inf" }).into(),
        _ if x == 0.0 => "0".into(), // also -0, in every format (Octave)
        Fmt::Int => format!("{x:.0}"),
        Fmt::Fixed(rd) => format!("{x:.rd$}"),
        Fmt::Exp(d) => exp_fmt(x, d),
    }
}

/// Scalar as Octave prints it: 5, 3.1416, -0.5000, 1.0000e+10.
pub fn scalar(x: f64) -> String {
    num(x, &choose(&[x]))
}

/// Sign shown between the parts of a complex number (signbit, so `0 - 0i` keeps its minus).
fn isign(im: f64) -> char {
    if im.is_sign_negative() { '-' } else { '+' }
}

/// Complex scalar: 3 + 4i, 0.5000 - 1.2500i, 1.0000e+10 + 2.0000e-03i.
fn cscalar(z: C64) -> String {
    let f = choose(&[z.re, z.im]);
    format!("{} {} {}i", num(z.re, &f), isign(z.im), num(z.im.abs(), &f))
}

/// Complex matrix body: both parts share one width, the real part gets a sign slot.
fn crows(m: &CMatrix) -> String {
    let f = cchoose(m);
    let parts: Vec<(String, String)> = m.data.iter().map(|z| (num(z.re, &f), num(z.im.abs(), &f))).collect();
    let w = parts.iter().map(|(r, i)| r.trim_start_matches('-').len().max(i.len())).max().unwrap_or(1);
    let mut out = String::new();
    for r in 0..m.rows {
        for c in 0..m.cols {
            let k = c * m.rows + r;
            out += &format!("  {:>rw$} {} {:>w$}i", parts[k].0, isign(m.data[k].im), parts[k].1, rw = w + 1);
        }
        out.push('\n');
    }
    out
}

/// Matrix body: right-aligned columns, two-space gutter plus a sign slot (none for logicals).
fn rows(m: &Matrix, logical: bool) -> String {
    let f = choose(&m.data);
    let cells: Vec<String> = m.data.iter().map(|&x| num(x, &f)).collect();
    let w = cells.iter().map(|s| s.trim_start_matches('-').len()).max().unwrap_or(1) + !logical as usize;
    let mut out = String::new();
    for r in 0..m.rows {
        for c in 0..m.cols {
            out += &format!("  {:>w$}", cells[c * m.rows + r]);
        }
        out.push('\n');
    }
    out
}

fn indent(s: &str, n: usize) -> String {
    let pad = " ".repeat(n);
    s.lines().map(|l| if l.is_empty() { "\n".to_string() } else { format!("{pad}{l}\n") }).collect()
}

pub fn func_str(f: &Func) -> String {
    match f {
        Func::Named(n) => format!("@{n}"),
        Func::Anon(l) => l.src.clone(),
    }
}

/// `name = value` as the REPL prints it.
pub fn display(name: &str, v: &Value) -> String {
    match v {
        Value::Num(m) | Value::Bool(m) if m.is_empty() => format!("{name} = []({}x{})\n", m.rows, m.cols),
        Value::Num(m) | Value::Bool(m) if m.is_scalar() => format!("{name} = {}\n", scalar(m.data[0])),
        Value::Num(m) => format!("{name} =\n\n{}\n", rows(m, false)),
        Value::Bool(m) => format!("{name} =\n\n{}\n", rows(m, true)),
        Value::Str(s) => format!("{name} = {s}\n"),
        Value::Complex(m) if m.is_scalar() => format!("{name} = {}\n", cscalar(m.data[0])),
        Value::Complex(m) => format!("{name} =\n\n{}\n", crows(m)),
        Value::Cell(c) if c.is_empty() => format!("{name} = {{}}(0x0)\n"),
        Value::Cell(_) => format!("{name} =\n{}\n", disp(v)),
        Value::Struct(_) => format!("{name} =\n\n  scalar structure containing the fields:\n\n{}\n", disp(v)),
        Value::Func(f) => match &**f {
            Func::Named(n) => format!("{name} = @{n}\n"),
            Func::Anon(l) => format!("{name} =\n\n{}\n\n", l.src),
        },
    }
}

/// `disp(value)`: the value without its name.
pub fn disp(v: &Value) -> String {
    match v {
        Value::Num(m) | Value::Bool(m) if m.is_empty() => String::new(),
        Value::Num(m) | Value::Bool(m) if m.is_scalar() => format!("{}\n", scalar(m.data[0])),
        Value::Num(m) => rows(m, false),
        Value::Bool(m) => rows(m, true),
        Value::Str(s) => format!("{s}\n"),
        Value::Complex(m) if m.is_scalar() => format!("{}\n", cscalar(m.data[0])),
        Value::Complex(m) => crows(m),
        Value::Cell(c) if c.is_empty() => "{}(0x0)\n".into(),
        Value::Cell(c) => {
            let items: String = c.iter().enumerate().map(|(k, e)| indent(&display(&format!("[1,{}]", k + 1), e), 2)).collect();
            format!("{{\n{items}}}\n")
        }
        Value::Struct(s) => s.iter().map(|(k, e)| indent(&display(k, e), 4)).collect(),
        Value::Func(f) => format!("{}\n", func_str(f)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numbers() {
        assert_eq!(scalar(5.0), "5");
        assert_eq!(scalar(std::f64::consts::PI), "3.1416");
        assert_eq!(scalar(-0.5), "-0.5000");
        assert_eq!(scalar(1e10), "1.0000e+10");
        assert_eq!(scalar(0.001), "1.0000e-03");
        assert_eq!(scalar(12.5), "12.500");
        assert_eq!(scalar(1234.5), "1234.5");
        assert_eq!(scalar(f64::NEG_INFINITY), "-Inf");
        set_long(true);
        assert_eq!(scalar(std::f64::consts::PI), "3.141592653589793");
        set_long(false);
        let m = Matrix::from_rows(&[vec![1., 2.], vec![3., 4.]]);
        assert_eq!(display("A", &Value::Num(m)), "A =\n\n   1   2\n   3   4\n\n");
        assert_eq!(disp(&Value::from(vec![0.5, 1.25])), "   0.5000   1.2500\n");
        assert_eq!(disp(&Value::from(vec![1., f64::NAN, -3.])), "     1   NaN    -3\n");
    }
}
