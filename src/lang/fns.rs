//! Interpreter-level builtins: constants, eval/feval, workspace, errors, structs, cells, type tests.
use std::collections::BTreeMap;
use std::rc::Rc;

use super::display::{self, disp};
use super::lexer::KEYWORDS;
use super::{Interp, is_ident};
use crate::value::{Builtin, C64, Func, Matrix, Value};

type Out = Result<Vec<Value>, String>;

pub fn lookup(name: &str) -> Option<Builtin> {
    let f: Builtin = match name {
        "pi" => |_, a, _| fill(a, std::f64::consts::PI),
        "e" => |_, a, _| fill(a, std::f64::consts::E),
        "Inf" | "inf" => |_, a, _| fill(a, f64::INFINITY),
        "NaN" | "nan" | "NA" => |_, a, _| fill(a, f64::NAN),
        "i" | "j" | "I" | "J" => |_, a, _| Ok(vec![Value::complex(sized(a, 0.0)?.map(|_| C64::new(0.0, 1.0)))]),
        // ponytail: narrows like every complex result, so complex(1, 0) is real (Octave keeps it complex)
        "complex" => |_, a, _| match a {
            [x] => Ok(vec![Value::complex(x.cmatrix()?)]),
            [re, im] => {
                let z = crate::builtins::ops::broadcast(&re.matrix()?, &im.matrix()?, C64::new).map_err(|_| "complex: dimension mismatch")?;
                Ok(vec![Value::complex(z)])
            }
            _ => Err("Invalid call to complex".into()),
        },
        "eps" => eps,
        "realmax" => |_, a, _| fill(a, f64::MAX),
        "realmin" => |_, a, _| fill(a, f64::MIN_POSITIVE),
        "flintmax" => |_, a, _| fill(a, 2f64.powi(53)),
        "true" => |_, a, _| Ok(vec![Value::Bool(sized(a, 1.0)?)]),
        "false" => |_, a, _| Ok(vec![Value::Bool(sized(a, 0.0)?)]),
        "disp" | "display" => |it, a, _| {
            let s = disp(a.first().ok_or("Invalid call to disp")?);
            it.print(&s);
            Ok(vec![])
        },
        "eval" => eval,
        "feval" => |it, a, n| {
            let (f, rest) = a.split_first().ok_or("Invalid call to feval")?;
            it.call(f, rest, n)
        },
        "str2func" => |it, a, _| {
            let s = arg_str(a, 0, "str2func")?;
            if s.starts_with('@') { it.eval_str(s, 1) } else { Ok(vec![Value::Func(Rc::new(Func::Named(s.to_string())))]) }
        },
        "func2str" => |_, a, _| match a.first() {
            Some(Value::Func(f)) => Ok(vec![Value::Str(match &**f {
                Func::Named(n) => n.clone(),
                Func::Anon(l) => l.src.clone(),
            })]),
            _ => Err("func2str: FCN_HANDLE argument must be a valid function handle".into()),
        },
        "exist" => |it, a, _| {
            let n = arg_str(a, 0, "exist")?;
            let code = match () {
                _ if it.vars.contains_key(n) => 1,
                _ if it.funcs.contains_key(n) => 103,
                _ if is_ident(n) && std::path::Path::new(&format!("{n}.m")).exists() => 2,
                _ if crate::resolve_builtin(n).is_some() || crate::py::has(n) => 5,
                _ => 0,
            };
            Ok(vec![Value::num(code as f64)])
        },
        "isvarname" => |_, a, _| Ok(vec![Value::bool(matches!(a.first(), Some(Value::Str(s)) if is_ident(s) && !KEYWORDS.contains(&s.as_str())))]),
        "clear" | "clearvars" => clear,
        "clc" | "pkg" | "more" => |_, _, _| Ok(vec![]),
        "who" => |it, _, _| {
            let mut names: Vec<&String> = it.vars.keys().collect();
            if names.is_empty() {
                return Ok(vec![]);
            }
            names.sort();
            let s = format!("Variables visible from the current scope:\n\n{}\n\n", names.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("  "));
            it.print(&s);
            Ok(vec![])
        },
        "whos" => |it, _, _| {
            let mut names: Vec<&String> = it.vars.keys().collect();
            names.sort();
            let mut s =
                format!("Variables visible from the current scope:\n\n  {:<12}{:<12}{}\n  {:<12}{:<12}{}\n", "Name", "Size", "Class", "====", "====", "=====");
            for n in names {
                let (r, c) = it.vars[n].size();
                s += &format!("  {:<12}{:<12}{}\n", n, format!("{r}x{c}"), it.vars[n].type_name());
            }
            it.print(&(s + "\n"));
            Ok(vec![])
        },
        "format" => |_, a, _| {
            let words: Vec<&str> = a.iter().filter_map(|v| v.as_str().ok()).collect();
            if words.is_empty() || words.contains(&"short") {
                display::set_long(false);
            } else if words.contains(&"long") {
                display::set_long(true);
            }
            Ok(vec![])
        },
        "class" => |_, a, _| Ok(vec![Value::Str(a.first().ok_or("Invalid call to class")?.type_name().into())]),
        "isa" => |_, a, _| {
            let (v, c) = (a.first().ok_or("Invalid call to isa")?, arg_str(a, 1, "isa")?);
            let num = matches!(v, Value::Num(_) | Value::Complex(_));
            Ok(vec![Value::bool(v.type_name() == c || (num && matches!(c, "numeric" | "float")))])
        },
        "error" | "rethrow" => error,
        "warning" => |it, a, _| {
            if a.is_empty() || matches!(a[0].as_str(), Ok("on" | "off" | "query" | "error")) {
                return Ok(vec![]);
            }
            let msg = message(it, strip_id(a))?;
            it.print(&format!("warning: {msg}\n"));
            Ok(vec![])
        },
        "struct" => |_, a, _| {
            if a.len() % 2 != 0 {
                return Err("struct: additional arguments must occur as \"field\", VALUE pairs".into());
            }
            let mut s = BTreeMap::new();
            for p in a.chunks(2) {
                let v = match &p[1] {
                    Value::Cell(c) if c.len() == 1 => c[0].clone(),
                    Value::Cell(c) if c.len() > 1 => return Err("struct: struct arrays are not supported".into()),
                    v => v.clone(),
                };
                s.insert(p[0].as_str()?.to_string(), v);
            }
            Ok(vec![Value::Struct(s)])
        },
        "isfield" => |_, a, _| {
            let has = |f: &Value| matches!((a.first(), f), (Some(Value::Struct(s)), Value::Str(f)) if s.contains_key(f));
            Ok(vec![match a.get(1) {
                Some(Value::Cell(fs)) => Value::Bool(Matrix::row(fs.iter().map(|f| has(f) as u8 as f64).collect())),
                Some(f) => Value::bool(has(f)),
                None => return Err("Invalid call to isfield".into()),
            }])
        },
        "fieldnames" => |_, a, _| match a.first() {
            Some(Value::Struct(s)) => Ok(vec![Value::Cell(s.keys().map(|k| Value::Str(k.clone())).collect())]),
            _ => Err("fieldnames: Invalid input argument".into()),
        },
        "rmfield" => |_, a, _| {
            let (Some(Value::Struct(s)), f) = (a.first(), arg_str(a, 1, "rmfield")?) else {
                return Err("rmfield: first argument must be a struct".into());
            };
            let mut s = s.clone();
            s.remove(f).ok_or_else(|| format!("rmfield: structure does not contain remaining field {f}"))?;
            Ok(vec![Value::Struct(s)])
        },
        "cell" => |_, a, _| {
            let m = sized(a, 0.0)?;
            Ok(vec![Value::Cell(vec![Value::empty(); m.len()])]) // ponytail: 1-D cells, r*c elements
        },
        "deal" => |_, a, n| match a.len() {
            1 => Ok(vec![a[0].clone(); n.max(1)]),
            k if k == n.max(1) => Ok(a.to_vec()),
            _ => Err("deal: nargin > 1 and nargin != nargout".into()),
        },
        "iscell" => |_, a, _| Ok(vec![Value::bool(matches!(a.first(), Some(Value::Cell(_))))]),
        "isstruct" => |_, a, _| Ok(vec![Value::bool(matches!(a.first(), Some(Value::Struct(_))))]),
        "ischar" => |_, a, _| Ok(vec![Value::bool(matches!(a.first(), Some(Value::Str(_))))]),
        "isnumeric" => |_, a, _| Ok(vec![Value::bool(matches!(a.first(), Some(Value::Num(_) | Value::Complex(_))))]),
        "iscomplex" => |_, a, _| Ok(vec![Value::bool(matches!(a.first(), Some(Value::Complex(_))))]),
        "islogical" | "isbool" => |_, a, _| Ok(vec![Value::bool(matches!(a.first(), Some(Value::Bool(_))))]),
        "is_function_handle" => |_, a, _| Ok(vec![Value::bool(matches!(a.first(), Some(Value::Func(_))))]),
        "isreal" => |_, a, _| Ok(vec![Value::bool(matches!(a.first(), Some(Value::Num(_) | Value::Bool(_) | Value::Str(_))))]),
        _ => return None,
    };
    Some(f)
}

fn arg_str<'a>(a: &'a [Value], k: usize, f: &str) -> Result<&'a str, String> {
    a.get(k).ok_or_else(|| format!("Invalid call to {f}"))?.as_str().map_err(|e| format!("{f}: {e}"))
}

/// Size arguments: (), (n), ([r c]), (r, c).
fn sized(a: &[Value], x: f64) -> Result<Matrix, String> {
    let (r, c) = match a {
        [] => (1, 1),
        [n] => match n.matrix()?.data[..] {
            [r, c] if r >= 0.0 && c >= 0.0 => (r as usize, c as usize),
            _ => (n.as_usize()?, n.as_usize()?),
        },
        [r, c, ..] => (r.as_usize()?, c.as_usize()?),
    };
    Ok(Matrix::filled(r, c, x))
}

fn fill(a: &[Value], x: f64) -> Out {
    Ok(vec![Value::Num(sized(a, x)?)])
}

/// `eps` or `eps(x)`: spacing of doubles at x.
fn eps(_: &mut Interp, a: &[Value], _: usize) -> Out {
    match a {
        [x] if !matches!(x, Value::Str(_)) => Ok(vec![Value::Num(x.matrix()?.map(|v| {
            let v = v.abs();
            v.next_up() - v
        }))]),
        _ => fill(a, f64::EPSILON),
    }
}

/// `eval(try)` / `eval(try, catch)`
fn eval(it: &mut Interp, a: &[Value], n: usize) -> Out {
    let src = arg_str(a, 0, "eval")?.to_string();
    match (it.eval_str(&src, n), a.get(1)) {
        (Err(_), Some(c)) => {
            it.err_line = None;
            let c = c.as_str()?.to_string();
            it.eval_str(&c, n)
        }
        (r, _) => r,
    }
}

fn clear(it: &mut Interp, a: &[Value], _: usize) -> Out {
    let names: Vec<&str> = a.iter().filter_map(|v| v.as_str().ok()).filter(|s| !s.starts_with('-')).collect();
    if names.is_empty() || names.iter().any(|n| matches!(*n, "all" | "variables")) {
        it.vars.clear();
    }
    for n in names {
        match n.strip_suffix('*') {
            Some(p) => it.vars.retain(|k, _| !k.starts_with(p)),
            None => drop(it.vars.remove(n)),
        }
    }
    Ok(vec![])
}

/// Drop a leading message identifier: error('Pkg:id', 'fmt', ...).
fn strip_id(a: &[Value]) -> &[Value] {
    match a.first().and_then(|v| v.as_str().ok()) {
        Some(s) if a.len() > 1 && s.contains(':') && !s.contains(char::is_whitespace) && !s.ends_with(':') => &a[1..],
        _ => a,
    }
}

/// Format a message: sprintf when there are arguments (and sprintf exists), else the plain text.
fn message(it: &mut Interp, a: &[Value]) -> Result<String, String> {
    match a {
        [] => Ok("unspecified error".into()),
        [m] => Ok(m.as_str()?.to_string()),
        _ => match crate::resolve_builtin("sprintf") {
            Some(f) => Ok(f(it, a, 1)?.first().and_then(|v| v.as_str().ok()).unwrap_or_default().to_string()),
            None => Ok(a
                .iter()
                .map(|v| v.as_str().map(String::from).unwrap_or_else(|_| format!("{}", v.scalar().unwrap_or(f64::NAN))))
                .collect::<Vec<_>>()
                .join(" ")),
        },
    }
}

fn error(it: &mut Interp, a: &[Value], _: usize) -> Out {
    if let Some(Value::Struct(s)) = a.first() {
        return Err(s.get("message").and_then(|m| m.as_str().ok()).unwrap_or("unspecified error").to_string());
    }
    Err(message(it, strip_id(a))?)
}
