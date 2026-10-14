//! Strings, the printf family (Octave semantics), number <-> text, timers.
use std::cell::Cell;
use std::collections::VecDeque;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use super::{R, chars, is_opt, nargs, num};
use crate::lang::Interp;
use crate::value::{Builtin, C64, CMatrix, Mat, Matrix, Value};

pub fn lookup(name: &str) -> Option<Builtin> {
    let f: Builtin = match name {
        "sprintf" => |_, a, _| {
            let fmt = template("sprintf", a)?;
            Ok(vec![Value::Str(format("sprintf", fmt, &a[1..])?)])
        },
        "printf" => |interp, a, _| {
            let s = format("printf", template("printf", a)?, &a[1..])?;
            interp.print(&s);
            Ok(vec![])
        },
        "fprintf" => fprintf,
        "num2str" => num2str,
        "int2str" => |_, a, _| {
            nargs("int2str", a, 1, 1)?;
            Ok(vec![Value::Str(match &a[0] {
                Value::Complex(z) => num2str_c(&z.map(|z| C64::new(z.re.round(), z.im.round())), None)?,
                v => num2str_of(&v.matrix()?.map(f64::round), None)?,
            })])
        },
        "mat2str" => mat2str,
        "str2double" => |_, a, _| {
            nargs("str2double", a, 1, 1)?;
            num(match &a[0] {
                Value::Str(s) => Matrix::scalar(str2double(s)),
                Value::Cell(c) => Matrix::row(c.iter().map(|v| v.as_str().map_or(f64::NAN, str2double)).collect()),
                v => {
                    let (r, c) = v.size();
                    Matrix::filled(r, c, f64::NAN)
                }
            })
        },
        "str2num" => str2num,
        "strcat" => strcat,
        "strsplit" => strsplit,
        "strjoin" => |_, a, _| {
            nargs("strjoin", a, 1, 2)?;
            let Value::Cell(c) = &a[0] else { return Err("strjoin: CSTR must be a cell array of strings".into()) };
            let parts: Vec<&str> = c.iter().map(|v| v.as_str()).collect::<Result<_, _>>()?;
            let d = a.get(1).map(|v| v.as_str().map(escapes)).transpose()?.unwrap_or_else(|| " ".into());
            Ok(vec![Value::Str(parts.join(&d))])
        },
        "strrep" => |_, a, _| {
            nargs("strrep", a, 3, 3)?;
            let (p, r) = (a[1].as_str()?, a[2].as_str()?);
            each_str("strrep", &a[0], |s| if p.is_empty() { s.to_string() } else { s.replace(p, r) })
        },
        "strtrim" => |_, a, _| {
            nargs("strtrim", a, 1, 1)?;
            each_str("strtrim", &a[0], |s| s.trim_matches([' ', '\t', '\n', '\x0b', '\x0c', '\r', '\0']).to_string())
        },
        "upper" | "toupper" => |_, a, _| {
            nargs("upper", a, 1, 1)?;
            each_str("upper", &a[0], str::to_uppercase).or_else(|_| Ok(vec![a[0].clone()]))
        },
        "lower" | "tolower" => |_, a, _| {
            nargs("lower", a, 1, 1)?;
            each_str("lower", &a[0], str::to_lowercase).or_else(|_| Ok(vec![a[0].clone()]))
        },
        "strcmp" => |_, a, _| strcmp("strcmp", a, false),
        "strcmpi" => |_, a, _| strcmp("strcmpi", a, true),
        "strfind" => |_, a, _| {
            nargs("strfind", a, 2, 2)?;
            let pat: Vec<char> = a[1].as_str()?.chars().collect();
            let find = |s: &str| {
                let s: Vec<char> = s.chars().collect();
                let n = s.len().saturating_sub(pat.len()) + 1;
                let hits = (0..n).filter(|&i| !pat.is_empty() && s.len() >= pat.len() && s[i..i + pat.len()] == pat[..]);
                Value::from(hits.map(|i| i as f64 + 1.0).collect::<Vec<_>>())
            };
            match &a[0] {
                Value::Cell(c) => Ok(vec![Value::Cell(c.iter().map(|v| v.as_str().map(find)).collect::<Result<_, _>>()?)]),
                v => Ok(vec![find(v.as_str()?)]),
            }
        },
        "blanks" => |_, a, _| {
            nargs("blanks", a, 1, 1)?;
            let n = a[0].as_usize()?;
            if n > 1 << 30 {
                return Err("blanks: N is too large".into());
            }
            Ok(vec![Value::Str(" ".repeat(n))])
        },
        "num2cell" => |_, a, _| {
            nargs("num2cell", a, 1, 1)?;
            Ok(vec![Value::Cell(match &a[0] {
                Value::Cell(c) => c.iter().map(|v| Value::Cell(vec![v.clone()])).collect(),
                Value::Str(s) => s.chars().map(|c| Value::Str(c.to_string())).collect(),
                Value::Bool(m) => m.data.iter().map(|&x| Value::bool(x != 0.0)).collect(),
                Value::Complex(m) => m.data.iter().map(|&z| Value::complex(Mat::scalar(z))).collect(),
                v => v.matrix()?.data.into_iter().map(Value::num).collect(),
            })])
        },
        "tic" => |_, _, nargout| {
            TIC.set(Some(Instant::now()));
            Ok(if nargout > 0 { vec![Value::num(now_us())] } else { vec![] })
        },
        "toc" => |interp, a, nargout| {
            let secs = match a.first() {
                Some(id) => (now_us() - id.scalar()?) / 1e6,
                None => TIC.get().ok_or("toc: function called before timer initialization with tic()")?.elapsed().as_secs_f64(),
            };
            if nargout > 0 {
                return num(Matrix::scalar(secs));
            }
            interp.print(&format!("Elapsed time is {} seconds.\n", fmt_g(secs, 6, false, false)));
            Ok(vec![])
        },
        _ => return None,
    };
    Some(f)
}

thread_local! {
    static TIC: Cell<Option<Instant>> = const { Cell::new(None) };
}

fn now_us() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_micros() as f64)
}

fn template<'a>(who: &str, a: &'a [Value]) -> Result<&'a str, String> {
    match a.first() {
        Some(Value::Str(s)) => Ok(s),
        Some(_) => Err(format!("{who}: format TEMPLATE must be a string")),
        None => Err(format!("Invalid call to {who}")),
    }
}

/// Backslash escapes as Octave's printf processes them in the template.
pub(super) fn escapes(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let digits = |it: &mut std::iter::Peekable<std::str::Chars>, radix: u32, max: usize, mut v: u32| {
            for _ in 0..max {
                match it.peek().and_then(|d| d.to_digit(radix)) {
                    Some(d) => {
                        v = v * radix + d;
                        it.next();
                    }
                    None => break,
                }
            }
            char::from_u32(v).unwrap_or('\0')
        };
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('a') => out.push('\x07'),
            Some('b') => out.push('\x08'),
            Some('f') => out.push('\x0c'),
            Some('v') => out.push('\x0b'),
            Some('x') => out.push(digits(&mut it, 16, 2, 0)),
            Some(d @ '0'..='7') => out.push(digits(&mut it, 8, 2, d as u32 - '0' as u32)),
            Some(o) => out.push(o), // \\ \" \' and unknown escapes
            None => out.push('\\'),
        }
    }
    out
}

/// One `%[flags][width][.prec]conv` directive.
#[derive(Clone, Debug, Default)]
struct Spec {
    flags: String,
    width: Option<usize>,
    prec: Option<usize>,
    conv: char,
}

/// Split a template Octave-style: literal text belongs to the conversion that follows it,
/// trailing text is its own element.
fn parse(fmt: &str) -> Vec<(String, Option<Spec>)> {
    let mut out = vec![];
    let mut text = String::new();
    let mut it = fmt.chars().peekable();
    let number = |it: &mut std::iter::Peekable<std::str::Chars>| {
        let mut s = String::new();
        while let Some(&d) = it.peek()
            && d.is_ascii_digit()
        {
            s.push(d);
            it.next();
        }
        // ponytail: width/precision capped, a typo like %.1e9f should not eat all memory
        s.parse::<usize>().ok().map(|n| n.min(1000))
    };
    while let Some(c) = it.next() {
        if c != '%' {
            text.push(c);
            continue;
        }
        if it.peek() == Some(&'%') {
            it.next();
            text.push('%');
            continue;
        }
        let mut spec = Spec::default();
        while let Some(&f) = it.peek()
            && "-+ 0#".contains(f)
        {
            spec.flags.push(f);
            it.next();
        }
        spec.width = number(&mut it);
        if it.peek() == Some(&'.') {
            it.next();
            spec.prec = Some(number(&mut it).unwrap_or(0));
        }
        while it.peek().is_some_and(|m| "lhLqjzt".contains(*m)) {
            it.next();
        }
        match it.next() {
            Some(conv) if "diouxXfFeEgGcs".contains(conv) => {
                spec.conv = conv;
                out.push((std::mem::take(&mut text), Some(spec)));
            }
            Some(other) => {
                text.push('%');
                text.push(other);
            }
            None => text.push('%'),
        }
    }
    if !text.is_empty() {
        out.push((text, None));
    }
    out
}

/// C-style %e: mantissa plus a signed exponent of at least two digits.
fn fmt_e(x: f64, prec: usize, upper: bool) -> String {
    let s = format!("{x:.prec$e}");
    let (m, e) = s.split_once('e').unwrap_or((&s, "0"));
    let e: i32 = e.parse().unwrap_or(0);
    format!("{m}{}{}{:02}", if upper { 'E' } else { 'e' }, if e < 0 { '-' } else { '+' }, e.abs())
}

/// C-style %g.
fn fmt_g(x: f64, prec: usize, alt: bool, upper: bool) -> String {
    let p = prec.max(1);
    let e: i32 = format!("{:.*e}", p - 1, x).split_once('e').map_or(0, |(_, e)| e.parse().unwrap_or(0));
    let s = if e < -4 || e >= p as i32 { fmt_e(x, p - 1, upper) } else { format!("{:.*}", (p as i32 - 1 - e) as usize, x) };
    if alt {
        return s;
    }
    let trim = |m: &str| if m.contains('.') { m.trim_end_matches('0').trim_end_matches('.').to_string() } else { m.to_string() };
    match s.find(['e', 'E']) {
        Some(k) => format!("{}{}", trim(&s[..k]), &s[k..]),
        None => trim(&s),
    }
}

fn pad(spec: &Spec, body: String, zeros: bool) -> String {
    let n = body.chars().count();
    let w = spec.width.unwrap_or(0);
    if n >= w {
        return body;
    }
    if spec.flags.contains('-') {
        return body + &" ".repeat(w - n);
    }
    if zeros && spec.flags.contains('0') {
        let k = if body.starts_with(['+', '-', ' ']) { 1 } else { 0 };
        return format!("{}{}{}", &body[..k], "0".repeat(w - n), &body[k..]);
    }
    " ".repeat(w - n) + &body
}

/// One number through one conversion. Non-integers given to %d/%i/%u/%x/%o/%c switch to %g,
/// like Octave; Inf/NaN print as Inf/NaN.
fn fmt_num(spec: &Spec, x: f64) -> String {
    let signed = |s: String| match () {
        _ if s.starts_with('-') => s,
        _ if spec.flags.contains('+') => format!("+{s}"),
        _ if spec.flags.contains(' ') => format!(" {s}"),
        _ => s,
    };
    if !x.is_finite() {
        let s = if x.is_nan() { "NaN" } else if x > 0.0 { "Inf" } else { "-Inf" };
        return pad(spec, signed(s.into()), false);
    }
    let int = x.fract() == 0.0 && x.abs() < 9.2e18;
    let p = spec.prec;
    let s = match spec.conv {
        'd' | 'i' | 'u' if int => format!("{}", x as i64),
        'x' if int && x >= 0.0 => format!("{:x}", x as u64),
        'X' if int && x >= 0.0 => format!("{:X}", x as u64),
        'o' if int && x >= 0.0 => format!("{:o}", x as u64),
        'c' | 's' if int && x >= 0.0 => return pad(spec, char::from_u32(x as u32).map(String::from).unwrap_or_default(), false),
        'f' | 'F' => format!("{:.*}", p.unwrap_or(6), x),
        'e' | 'E' => fmt_e(x, p.unwrap_or(6), spec.conv == 'E'),
        'G' => fmt_g(x, p.unwrap_or(6), spec.flags.contains('#'), true),
        _ => fmt_g(x, p.unwrap_or(6), spec.flags.contains('#'), false),
    };
    pad(spec, signed(s), true)
}

enum Item {
    Num(f64),
    Str(String),
}

/// Octave sprintf: numeric args are flattened column-major (complex as real, imag pairs), a
/// char arg is one item for %s and a run of char codes otherwise; the template is recycled
/// until the data runs out.
pub(super) fn format(who: &str, fmt: &str, args: &[Value]) -> Result<String, String> {
    let elts = parse(&escapes(fmt));
    let mut items = VecDeque::new();
    for a in args {
        match a {
            Value::Str(s) => items.push_back(Item::Str(s.clone())),
            Value::Num(m) | Value::Bool(m) => items.extend(m.data.iter().map(|&x| Item::Num(x))),
            Value::Complex(m) => items.extend(m.data.iter().flat_map(|z| [Item::Num(z.re), Item::Num(z.im)])),
            v => return Err(format!("{who}: wrong type argument '{}'", v.type_name())),
        }
    }
    let mut out = String::new();
    if items.is_empty() || elts.iter().all(|e| e.1.is_none()) {
        elts.iter().for_each(|e| out += &e.0);
        return Ok(out);
    }
    loop {
        for (text, spec) in &elts {
            let Some(spec) = spec else {
                out += text;
                continue;
            };
            let s = match (spec.conv, items.pop_front()) {
                (_, None) => return Ok(out),
                ('s', Some(Item::Str(s))) => pad(spec, if let Some(p) = spec.prec { s.chars().take(p).collect() } else { s }, false),
                (_, Some(Item::Str(s))) => {
                    let mut cs = s.chars();
                    let first = cs.next();
                    let rest: String = cs.collect();
                    if !rest.is_empty() {
                        items.push_front(Item::Str(rest));
                    }
                    first.map_or_else(String::new, |c| fmt_num(spec, c as u32 as f64))
                }
                (_, Some(Item::Num(x))) => fmt_num(spec, x),
            };
            out += text;
            out += &s;
        }
        if items.is_empty() {
            return Ok(out);
        }
    }
}

fn fprintf(interp: &mut Interp, a: &[Value], nargout: usize) -> R {
    let (fid, rest) = match a.first() {
        Some(Value::Str(_)) => (1.0, a),
        Some(v) => (v.scalar()?, &a[1..]),
        None => return Err("Invalid call to fprintf".into()),
    };
    if fid != 1.0 && fid != 2.0 {
        return Err(format!("fprintf: invalid stream number = {fid}")); // ponytail: no file I/O
    }
    let s = if rest.is_empty() { String::new() } else { format("fprintf", template("fprintf", rest)?, &rest[1..])? };
    interp.print(&s);
    Ok(if nargout > 0 { vec![Value::num(s.len() as f64)] } else { vec![] })
}

/// Octave's num2str: "%Nd" for integers, "%N.Mg" with 4 significant decimals otherwise;
/// rows are formatted with the format repeated per column, common leading blanks trimmed.
fn num2str_of(x: &Matrix, arg: Option<&Value>) -> Result<String, String> {
    if x.is_empty() {
        return Ok(String::new());
    }
    let neg = x.data.iter().any(|&v| v < 0.0) as usize;
    let big = x.data.iter().filter(|v| v.is_finite()).fold(0.0f64, |m, v| m.max(v.abs()));
    let dgt = if big > 0.0 { big.log10().floor() as i64 } else { 0 };
    let fmt = match arg {
        Some(Value::Str(f)) => f.clone(),
        Some(p) => {
            let p = p.as_usize().map_err(|_| "num2str: PRECISION must be a scalar integer >= 0")?.min(100);
            format!("%{}.{}g", p + 7 + neg, p)
        }
        None if x.data.iter().all(|v| *v == v.trunc()) => format!("%{}d", dgt.max(0) as usize + 3 + neg),
        None => {
            let ndgt = (dgt + 5).clamp(5, 16) as usize;
            format!("%{}.{}g", ndgt + 7 + neg, ndgt)
        }
    };
    rows_text(&fmt, x.cols, (0..x.rows).map(|i| (0..x.cols).map(|j| x.get(i, j)).collect()), |s| s)
}

/// Each row's values through `fmt` repeated per column, then `fix`; trailing blanks and the
/// blanks common to all rows' starts are trimmed, like Octave's num2str.
fn rows_text(fmt: &str, cols: usize, rows: impl Iterator<Item = Vec<f64>>, fix: fn(String) -> String) -> Result<String, String> {
    let rowfmt = fmt.repeat(cols).trim_end().to_string();
    let rows = rows.map(|r| format("num2str", &rowfmt, &[Value::from(r)]).map(fix)).collect::<Result<Vec<_>, _>>()?;
    let rows: Vec<&str> = rows.iter().map(|r| r.trim_end()).collect();
    let lead = rows.iter().map(|r| r.len() - r.trim_start_matches(' ').len()).min().unwrap_or(0);
    Ok(rows.iter().map(|r| &r[lead..]).collect::<Vec<_>>().join("\n"))
}

/// Complex num2str: Octave's "%W.Pg%-+W.Pgi" per element, padding moved behind the "i".
fn num2str_c(z: &CMatrix, arg: Option<&Value>) -> Result<String, String> {
    let parts = || z.data.iter().flat_map(|z| [z.re, z.im]);
    let fmt = match arg {
        Some(Value::Str(f)) => format!("{}%-+{}i", f.trim_end(), f.get(1..).unwrap_or("")),
        Some(p) => {
            let p = p.as_usize().map_err(|_| "num2str: PRECISION must be a scalar integer >= 0")?.min(100);
            format!("%{w}.{p}g%-+{w}.{p}gi", w = p + 7)
        }
        None => {
            let big = parts().filter(|v| v.is_finite()).fold(0.0f64, |m, v| m.max(v.abs()));
            let dgt = if big > 0.0 { big.log10().floor() as i64 } else { 0 };
            if parts().all(|v| v == v.trunc()) {
                let n = (dgt + 3).max(1);
                format!("%{n}.{n}g%-+{n}.{n}gi")
            } else {
                let n = (dgt + 5).clamp(5, 16);
                format!("%{w}.{n}g%-+{w}.{n}gi", w = n + 7)
            }
        }
    };
    // "  3+4  i" -> "  3+4i  ": blanks before an "i" move behind it
    let move_i = |s: String| {
        let (mut out, mut blanks) = (String::new(), 0);
        for c in s.chars() {
            match c {
                ' ' => blanks += 1,
                'i' => out += &format!("i{}", " ".repeat(std::mem::take(&mut blanks))),
                c => out += &format!("{}{c}", " ".repeat(std::mem::take(&mut blanks))),
            }
        }
        out
    };
    let row = |i: usize| (0..z.cols).flat_map(|j| [z.get(i, j).re, z.get(i, j).im]).collect();
    rows_text(&fmt, z.cols, (0..z.rows).map(row), move_i)
}

fn num2str(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("num2str", a, 1, 2)?;
    Ok(vec![Value::Str(match &a[0] {
        Value::Str(s) => s.clone(),
        Value::Complex(z) => num2str_c(z, a.get(1))?,
        v => num2str_of(&v.matrix()?, a.get(1))?,
    })])
}

fn mat2str(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("mat2str", a, 1, 3)?;
    if let Value::Str(s) = &a[0] {
        return Ok(vec![Value::Str(format!("\"{s}\""))]);
    }
    let p = match a.get(1) {
        Some(v) if !matches!(v, Value::Str(_)) => v.as_usize()?.min(100),
        _ => 15,
    };
    let spec = Spec { conv: 'g', prec: Some(p), ..Default::default() };
    let signed = Spec { flags: "+".into(), ..spec.clone() };
    // element texts, column-major
    let (r, c, items): (usize, usize, Vec<String>) = match &a[0] {
        Value::Complex(z) => (z.rows, z.cols, z.data.iter().map(|z| format!("{}{}i", fmt_num(&spec, z.re), fmt_num(&signed, z.im))).collect()),
        Value::Bool(m) => (m.rows, m.cols, m.data.iter().map(|&v| (if v != 0.0 { "true" } else { "false" }).to_string()).collect()),
        v => {
            let x = v.matrix()?;
            (x.rows, x.cols, x.data.iter().map(|&v| fmt_num(&spec, v)).collect())
        }
    };
    let s = match (r, c) {
        (1, 1) => items[0].clone(),
        (r, c) if r * c == 0 => format!("zeros({r},{c})"),
        (r, c) => {
            let rows: Vec<String> = (0..r).map(|i| (0..c).map(|j| items[j * r + i].as_str()).collect::<Vec<_>>().join(" ")).collect();
            format!("[{}]", rows.join(";"))
        }
    };
    Ok(vec![Value::Str(s)])
}

fn str2double(s: &str) -> f64 {
    s.trim().parse().unwrap_or(f64::NAN)
}

/// str2num evaluates the text, like Octave's eval-based version.
fn str2num(interp: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("str2num", a, 1, 1)?;
    let s = a[0].as_str()?;
    let var = "__str2num_tmp__";
    let ok = interp.eval(&format!("{var} = [{s}];")).is_ok();
    match interp.vars.remove(var) {
        Some(v) if ok => Ok(vec![v, Value::bool(true)]),
        _ => Ok(vec![Value::empty(), Value::bool(false)]),
    }
}

/// Map a string or each string of a cell.
fn each_str(who: &str, v: &Value, f: impl Fn(&str) -> String) -> R {
    match v {
        Value::Str(s) => Ok(vec![Value::Str(f(s))]),
        Value::Cell(c) => Ok(vec![Value::Cell(c.iter().map(|v| v.as_str().map(|s| Value::Str(f(s)))).collect::<Result<_, _>>()?)]),
        _ => Err(format!("{who}: S must be a string or cell array of strings")),
    }
}

fn strcat(_: &mut Interp, a: &[Value], _: usize) -> R {
    let text = |v: &Value| -> Result<String, String> {
        match v {
            Value::Str(s) => Ok(s.clone()),
            v => Ok(chars(&v.matrix()?.data)),
        }
    };
    if let Some(n) = a.iter().find_map(|v| if let Value::Cell(c) = v { Some(c.len()) } else { None }) {
        // cell inputs: element-wise, whitespace kept
        let out = (0..n).map(|k| {
            let parts = a.iter().map(|v| match v {
                Value::Cell(c) if c.len() == n => text(&c[k]),
                Value::Cell(c) if c.len() == 1 => text(&c[0]),
                Value::Cell(_) => Err("strcat: nonconformant arguments".to_string()),
                v => text(v),
            });
            parts.collect::<Result<Vec<_>, _>>().map(|p| Value::Str(p.concat()))
        });
        return Ok(vec![Value::Cell(out.collect::<Result<_, _>>()?)]);
    }
    // char inputs lose trailing whitespace
    let parts = a.iter().map(|v| text(v).map(|s| s.trim_end_matches([' ', '\t', '\n', '\0']).to_string()));
    Ok(vec![Value::Str(parts.collect::<Result<Vec<_>, _>>()?.concat())])
}

fn strsplit(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("strsplit", a, 1, 4)?;
    let s: Vec<char> = a[0].as_str()?.chars().collect();
    let mut delims: Vec<String> = [" ", "\x0c", "\n", "\r", "\t", "\x0b"].map(String::from).to_vec();
    let mut i = 1;
    if let Some(d) = a.get(1)
        && !is_opt(d, "collapsedelimiters")
    {
        delims = match d {
            Value::Str(d) => vec![escapes(d)],
            Value::Cell(c) => c.iter().map(|v| v.as_str().map(escapes)).collect::<Result<_, _>>()?,
            _ => return Err("strsplit: DELIMITER must be a string or cell array of strings".into()),
        };
        i = 2;
    }
    let mut collapse = true;
    while i + 1 < a.len() {
        if is_opt(&a[i], "collapsedelimiters") {
            collapse = a[i + 1].truthy()?;
        }
        i += 2;
    }
    let mut delims: Vec<Vec<char>> = delims.iter().map(|d| d.chars().collect()).filter(|d: &Vec<char>| !d.is_empty()).collect();
    delims.sort_by_key(|d| std::cmp::Reverse(d.len())); // longest match first
    let (mut parts, mut cur, mut k) = (vec![], String::new(), 0);
    'scan: while k < s.len() {
        for d in &delims {
            if s[k..].starts_with(d) {
                parts.push(std::mem::take(&mut cur));
                k += d.len();
                continue 'scan;
            }
        }
        cur.push(s[k]);
        k += 1;
    }
    parts.push(cur);
    if collapse {
        // runs of delimiters count as one: drop empty pieces between them
        let n = parts.len();
        parts = parts.into_iter().enumerate().filter(|(i, p)| !p.is_empty() || *i == 0 || *i == n - 1).map(|(_, p)| p).collect();
    }
    Ok(vec![Value::Cell(parts.into_iter().map(Value::Str).collect())])
}

fn strcmp(who: &str, a: &[Value], ci: bool) -> R {
    nargs(who, a, 2, 2)?;
    let eq = |x: &Value, y: &Value| match (x, y) {
        (Value::Str(p), Value::Str(q)) => if ci { p.to_lowercase() == q.to_lowercase() } else { p == q },
        _ => false,
    };
    let row = |v: Vec<bool>| Ok(vec![Value::Bool(Matrix::row(v.into_iter().map(|b| b as u8 as f64).collect()))]);
    match (&a[0], &a[1]) {
        (Value::Cell(p), Value::Cell(q)) if p.len() == q.len() => row(p.iter().zip(q).map(|(x, y)| eq(x, y)).collect()),
        (Value::Cell(p), Value::Cell(q)) if q.len() == 1 => row(p.iter().map(|x| eq(x, &q[0])).collect()),
        (Value::Cell(p), Value::Cell(q)) if p.len() == 1 => row(q.iter().map(|y| eq(&p[0], y)).collect()),
        (Value::Cell(_), Value::Cell(_)) => Err(format!("{who}: nonconformant cell arrays")),
        (Value::Cell(p), v) | (v, Value::Cell(p)) => row(p.iter().map(|x| eq(x, v)).collect()),
        (x, y) => Ok(vec![Value::bool(eq(x, y))]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sp(fmt: &str, args: &[Value]) -> String {
        format("sprintf", fmt, args).unwrap()
    }

    fn n2s(args: &[Value]) -> String {
        match &num2str(&mut Interp::new(), args, 1).unwrap()[0] {
            Value::Str(s) => s.clone(),
            v => panic!("{v:?}"),
        }
    }

    #[test]
    fn sprintf_cases() {
        assert_eq!(sp("%d", &[Value::num(42.)]), "42");
        assert_eq!(sp("%5.2f|%-5d|%05d", &[Value::num(3.14159), Value::num(7.), Value::num(-42.)]), " 3.14|7    |-0042");
        assert_eq!(sp("%e", &[Value::num(12345.678)]), "1.234568e+04");
        assert_eq!(sp("%g %g %g", &[Value::num(0.0001), Value::num(1e-5), Value::num(123456789.)]), "0.0001 1e-05 1.23457e+08");
        assert_eq!(sp("%d", &[Value::num(1.5)]), "1.5");
        assert_eq!(sp("%d", &[Value::num(std::f64::consts::PI)]), "3.14159");
        assert_eq!(sp("%d, ", &[Value::from(vec![1., 2., 3.])]), "1, 2, 3, ");
        assert_eq!(sp("%d and %d\\n", &[Value::num(1.), Value::num(2.), Value::num(3.)]), "1 and 2\n3");
        assert_eq!(sp("%s=%d\n", &["x".into(), Value::num(5.)]), "x=5\n");
        assert_eq!(sp("%d", &["ab".into()]), "9798");
        assert_eq!(sp("%c-", &["ab".into()]), "a-b-");
        assert_eq!(sp("%s", &[Value::num(65.)]), "A");
        assert_eq!(sp("%x %o %X", &[Value::num(255.), Value::num(8.), Value::num(255.)]), "ff 10 FF");
        assert_eq!(sp("%+d % d %5s|%-4s|", &[Value::num(3.), Value::num(4.), "ab".into(), "c".into()]), "+3  4    ab|c   |");
        assert_eq!(sp("%d %f", &[Value::num(f64::INFINITY), Value::num(f64::NAN)]), "Inf NaN");
        assert_eq!(sp("100%% done\\t[%d]", &[]), "100% done\t[]");
        assert_eq!(sp("value: %d\n", &[Value::empty()]), "value: \n");
        assert_eq!(sp("%.3s", &["abcdef".into()]), "abc");
        assert_eq!(sp("\\x41\\101", &[]), "AA");
    }

    #[test]
    fn num2str_cases() {
        assert_eq!(n2s(&[Value::num(std::f64::consts::PI)]), "3.1416");
        assert_eq!(n2s(&[Value::num(42.)]), "42");
        assert_eq!(n2s(&[Value::num(-5.)]), "-5");
        assert_eq!(n2s(&[Value::from(vec![1., 2., 3.])]), "1  2  3");
        assert_eq!(n2s(&[Value::num(123.456)]), "123.456");
        assert_eq!(n2s(&[Value::num(0.1)]), "0.1");
        assert_eq!(n2s(&[Value::num(1e-5)]), "1e-05");
        assert_eq!(n2s(&[Value::num(f64::INFINITY)]), "Inf");
        assert_eq!(n2s(&[Value::num(f64::NAN)]), "NaN");
        assert_eq!(n2s(&[Value::num(std::f64::consts::PI), Value::num(8.)]), "3.1415927");
        assert_eq!(n2s(&[Value::num(std::f64::consts::PI), "%10.5f".into()]), "3.14159");
        assert_eq!(n2s(&[Matrix::from_rows(&[vec![1., 10.], vec![100., 2.]]).into()]), "  1   10\n100    2");
        assert_eq!(n2s(&[Value::bool(true)]), "1");
        let m = mat2str(&mut Interp::new(), &[Matrix::from_rows(&[vec![1., 2.5], vec![-3., 4.]]).into()], 1).unwrap();
        assert!(matches!(&m[0], Value::Str(s) if s == "[1 2.5;-3 4]"));
    }

    #[test]
    fn complex_text() {
        let z = |v: &[(f64, f64)]| Value::complex(Mat::row(v.iter().map(|&(re, im)| C64::new(re, im)).collect()));
        assert_eq!(n2s(&[z(&[(3., 4.)])]), "3+4i");
        assert_eq!(n2s(&[z(&[(0.5, -1.25)])]), "0.5-1.25i");
        assert_eq!(n2s(&[z(&[(1., 2.), (3., -4.)])]), "1+2i   3-4i");
        assert_eq!(n2s(&[z(&[(std::f64::consts::PI, 1.)]), Value::num(3.)]), "3.14+1i");
        assert_eq!(n2s(&[z(&[(1.5, 2.)]), "%.2f".into()]), "1.50+2.00i");
        let m2s = |v: Value| match &mat2str(&mut Interp::new(), &[v], 1).unwrap()[0] {
            Value::Str(s) => s.clone(),
            v => panic!("{v:?}"),
        };
        assert_eq!(m2s(z(&[(1., 2.), (3., 0.)])), "[1+2i 3+0i]");
        assert_eq!(m2s(z(&[(0.5, -1.)])), "0.5-1i");
        assert_eq!(sp("%d %d|", &[z(&[(1., 2.), (3., -4.)])]), "1 2|3 -4|");
        assert_eq!(sp("%g%+gi", &[z(&[(1.5, -2.)])]), "1.5-2i");
        let cells = lookup("num2cell").unwrap()(&mut Interp::new(), &[z(&[(1., 1.), (2., 0.)])], 1).unwrap();
        assert!(matches!(&cells[0], Value::Cell(c) if c[0].is_complex() && matches!(c[1], Value::Num(_))));
    }

    #[test]
    fn string_utils() {
        let run = |name: &str, args: &[Value]| lookup(name).unwrap()(&mut Interp::new(), args, 1).unwrap().remove(0);
        let cell = |v: Value| match v {
            Value::Cell(c) => c.into_iter().map(|s| s.as_str().unwrap().to_string()).collect::<Vec<_>>(),
            v => panic!("{v:?}"),
        };
        assert_eq!(cell(run("strsplit", &["a,b,,c".into(), ",".into()])), ["a", "b", "c"]);
        assert_eq!(cell(run("strsplit", &["a,,b".into(), ",".into(), "CollapseDelimiters".into(), Value::bool(false)])), ["a", "", "b"]);
        assert_eq!(cell(run("strsplit", &["hello  world".into()])), ["hello", "world"]);
        let parts = Value::Cell(vec!["a".into(), "b".into()]);
        assert!(matches!(run("strjoin", &[parts.clone(), "-".into()]), Value::Str(s) if s == "a-b"));
        assert!(matches!(run("strcat", &["a ".into(), "b".into()]), Value::Str(s) if s == "ab"));
        assert!(matches!(run("strrep", &["hello".into(), "l".into(), "L".into()]), Value::Str(s) if s == "heLLo"));
        assert!(matches!(run("strtrim", &["  x \n".into()]), Value::Str(s) if s == "x"));
        assert!(matches!(run("upper", &["abc".into()]), Value::Str(s) if s == "ABC"));
        assert!(run("strcmp", &["a".into(), "a".into()]).truthy().unwrap());
        assert!(!run("strcmp", &["a".into(), "A".into()]).truthy().unwrap());
        assert!(run("strcmpi", &["a".into(), "A".into()]).truthy().unwrap());
        assert_eq!(run("strcmp", &[parts, "b".into()]).matrix().unwrap().data, vec![0., 1.]);
        assert_eq!(run("strfind", &["abcabc".into(), "bc".into()]).matrix().unwrap().data, vec![2., 5.]);
        assert_eq!(run("strfind", &["aaa".into(), "aa".into()]).matrix().unwrap().data, vec![1., 2.]);
        assert_eq!(run("str2double", &[" 2.5e3 ".into()]).scalar().unwrap(), 2500.0);
        assert!(run("str2double", &["abc".into()]).scalar().unwrap().is_nan());
        assert_eq!(run("str2double", &["-Inf".into()]).scalar().unwrap(), f64::NEG_INFINITY);
        assert!(matches!(run("blanks", &[Value::num(3.)]), Value::Str(s) if s == "   "));
        assert!(matches!(run("int2str", &[Value::num(2.7)]), Value::Str(s) if s == "3"));
    }
}
