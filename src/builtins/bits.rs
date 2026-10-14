//! Bits and bases for programmers: dec2bin & co, bitand/bitor/bitxor/bitshift/bitcmp, xor,
//! truthtable. Numbers are doubles, so integers are exact below 2^53 (flintmax), like Octave's
//! double versions. Text results: a string for a scalar, else a cell array of equal-width strings
//! (Octave returns a char matrix; there are no char matrices here).
use super::{R, bcast, first, nargs, num};
use crate::lang::Interp;
use crate::value::{Builtin, Matrix, Value};

/// 2^53: doubles hold every integer below it
const FLINT: f64 = 9007199254740992.0;

pub fn lookup(name: &str) -> Option<Builtin> {
    let f: Builtin = match name {
        "dec2bin" => |_, a, _| {
            nargs("dec2bin", a, 1, 2)?;
            to_text("dec2bin", &a[0], 2, a.get(1))
        },
        "dec2hex" => |_, a, _| {
            nargs("dec2hex", a, 1, 2)?;
            to_text("dec2hex", &a[0], 16, a.get(1))
        },
        "dec2base" => |_, a, _| {
            nargs("dec2base", a, 2, 3)?;
            to_text("dec2base", &a[0], base("dec2base", &a[1])?, a.get(2))
        },
        "bin2dec" => |_, a, _| {
            nargs("bin2dec", a, 1, 1)?;
            from_text("bin2dec", &a[0], 2)
        },
        "hex2dec" => |_, a, _| {
            nargs("hex2dec", a, 1, 1)?;
            from_text("hex2dec", &a[0], 16)
        },
        "base2dec" => |_, a, _| {
            nargs("base2dec", a, 2, 2)?;
            from_text("base2dec", &a[0], base("base2dec", &a[1])?)
        },
        "bitand" => |_, a, _| bitop("bitand", a, |x, y| x & y),
        "bitor" => |_, a, _| bitop("bitor", a, |x, y| x | y),
        "bitxor" => |_, a, _| bitop("bitxor", a, |x, y| x ^ y),
        // bitshift(a, n): n > 0 shifts left (bits past 53 drop out), n < 0 right
        "bitshift" => |_, a, _| {
            nargs("bitshift", a, 2, 2)?;
            let (x, n) = (a[0].matrix()?, a[1].matrix()?);
            ints("bitshift", &x)?;
            if n.data.iter().any(|s| s.fract() != 0.0) {
                return Err("bitshift: N must be an integer".into());
            }
            let mask = FLINT as u64 - 1;
            num(bcast("bitshift", &[x, n], |v| {
                let (x, s) = (v[0] as u64, v[1]);
                let r = if s >= 0.0 { x.checked_shl(s as u32).unwrap_or(0) & mask } else { x.checked_shr(-s as u32).unwrap_or(0) };
                r as f64
            })?)
        },
        // bitcmp(a[, bits]): NOT within `bits`; by default the smallest of 8, 16, 32 or 53 bits that
        // holds a (like the calculator's bitcmp), so bitcmp(5) is 250. Octave's double version uses 53.
        "bitcmp" => |_, a, _| {
            nargs("bitcmp", a, 1, 2)?;
            let bits = a.get(1).map(Value::as_usize).transpose()?;
            if bits.is_some_and(|b| !(1..=53).contains(&b)) {
                return Err("bitcmp: BITS must be 1..53".into());
            }
            let m = a[0].matrix()?;
            ints("bitcmp", &m)?;
            num(m.map(|x| {
                let n = x as u64;
                let w = bits.unwrap_or_else(|| [8, 16, 32].into_iter().find(|&w| n < 1 << w).unwrap_or(53));
                (!n & ((1u64 << w) - 1)) as f64
            }))
        },
        "xor" => |_, a, _| {
            nargs("xor", a, 2, 2)?;
            let m = bcast("xor", &[a[0].matrix()?, a[1].matrix()?], |v| ((v[0] != 0.0) != (v[1] != 0.0)) as u8 as f64)?;
            Ok(vec![Value::Bool(m)])
        },
        "truthtable" => truthtable,
        _ => return None,
    };
    Some(f)
}

/// Every element a non-negative integer below flintmax.
fn ints(name: &str, m: &Matrix) -> Result<(), String> {
    if m.data.iter().all(|&x| x >= 0.0 && x.fract() == 0.0 && x < FLINT) {
        Ok(())
    } else {
        Err(format!("{name}: arguments must be non-negative integers below flintmax"))
    }
}

fn bitop(name: &str, a: &[Value], f: fn(u64, u64) -> u64) -> R {
    nargs(name, a, 2, 2)?;
    let (x, y) = (a[0].matrix()?, a[1].matrix()?);
    ints(name, &x)?;
    ints(name, &y)?;
    num(bcast(name, &[x, y], |v| f(v[0] as u64, v[1] as u64) as f64)?)
}

fn base(name: &str, v: &Value) -> Result<u32, String> {
    // ponytail: numeric BASE only; Octave also takes a string of digit symbols
    match v.scalar() {
        Ok(b) if (2.0..=36.0).contains(&b) && b.fract() == 0.0 => Ok(b as u32),
        _ => Err(format!("{name}: BASE must be an integer from 2 to 36")),
    }
}

/// Integer x in `base`, uppercase digits; a negative x in two's complement of the smallest of
/// 8, 16, 32, 64 bits that holds it (as Octave's dec2bin(-5) = 11111011).
fn digits(name: &str, x: f64, base: u32) -> Result<String, String> {
    if x.fract() != 0.0 || x.abs() >= FLINT {
        return Err(format!("{name}: input must be integers below flintmax"));
    }
    let mut n = if x < 0.0 {
        let k = -x as u64;
        let w = [8, 16, 32, 64].into_iter().find(|&w| k <= 1u64 << (w - 1)).unwrap_or(64);
        (x as i64 as u64) & (u64::MAX >> (64 - w))
    } else {
        x as u64
    };
    let mut s = vec![];
    loop {
        s.push(char::from_digit((n % base as u64) as u32, base).unwrap_or('?').to_ascii_uppercase());
        n /= base as u64;
        if n == 0 {
            break;
        }
    }
    Ok(s.iter().rev().collect())
}

/// dec2bin & co: a string for a scalar, else a cell of strings padded to one width (at least `len`).
fn to_text(name: &str, v: &Value, base: u32, len: Option<&Value>) -> R {
    let m = v.matrix()?;
    let len = len.map(Value::as_usize).transpose()?.unwrap_or(0);
    let s: Vec<String> = m.data.iter().map(|&x| digits(name, x, base)).collect::<Result<_, _>>()?;
    let w = s.iter().map(String::len).max().unwrap_or(0).max(len);
    let mut s: Vec<Value> = s.into_iter().map(|d| Value::Str(format!("{d:0>w$}"))).collect();
    Ok(vec![if m.is_scalar() { s.swap_remove(0) } else { Value::Cell(s) }])
}

/// bin2dec & co: a string -> number, a cell of strings -> column; spaces ignored, NaN for bad digits.
fn from_text(name: &str, v: &Value, base: u32) -> R {
    let parse = |s: &str| {
        let d = s.chars().filter(|c| !c.is_whitespace());
        d.map(|c| c.to_digit(base)).try_fold(0.0, |acc, d| d.map(|d| acc * base as f64 + d as f64)).unwrap_or(f64::NAN)
    };
    match v {
        Value::Str(s) => num(Matrix::scalar(parse(s))),
        Value::Cell(c) => num(Matrix::col(c.iter().map(|x| x.as_str().map(parse)).collect::<Result<_, _>>()?)),
        _ => Err(format!("{name}: S must be a string or a cell array of strings")),
    }
}

/// Names in `expr` that aren't variables or functions, sorted: the truth table's inputs.
fn free_names(it: &Interp, expr: &str) -> Vec<String> {
    let funcs = it.func_names();
    let mut names: Vec<String> = expr
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| w.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_'))
        .filter(|w| !it.vars.contains_key(*w) && !funcs.iter().any(|f| f == w) && crate::resolve_builtin(w).is_none())
        .map(String::from)
        .collect();
    names.sort();
    names.dedup();
    names
}

/// truthtable('a & ~b | c'): the expression over every 0/1 combination of its free names (sorted,
/// the first is the most significant bit) as the matrix [inputs output]; printed as a table
/// instead when the result isn't assigned. && and || work element-wise here.
fn truthtable(it: &mut Interp, a: &[Value], nargout: usize) -> R {
    nargs("truthtable", a, 1, 1)?;
    let src = a[0].as_str()?;
    let expr = src.replace("&&", "&").replace("||", "|");
    let names = free_names(it, &expr);
    if names.len() > 16 {
        return Err(format!("truthtable: {} inputs is too many (at most 16)", names.len()));
    }
    let (k, rows) = (names.len(), 1usize << names.len());
    let cols: Vec<Vec<f64>> = (0..k).map(|j| (0..rows).map(|r| ((r >> (k - 1 - j)) & 1) as f64).collect()).collect();
    let f = first(it.call(&Value::from("str2func"), &[Value::Str(format!("@({}) {expr}", names.join(", ")))], 1)?)?;
    let args: Vec<Value> = cols.iter().map(|c| Value::Num(Matrix::col(c.clone()))).collect();
    let y = first(it.call(&f, &args, 1)?)?.matrix()?;
    let out: Vec<f64> = match y.len() {
        1 => vec![(y.data[0] != 0.0) as u8 as f64; rows],
        n if n == rows => y.data.iter().map(|&v| (v != 0.0) as u8 as f64).collect(),
        n => return Err(format!("truthtable: expected {rows} results, got {n}")),
    };
    if nargout == 0 {
        let head = names.join(" ");
        let mut t = format!("{head} | {src}\n{}-+-{}\n", "-".repeat(head.len()), "-".repeat(src.chars().count()));
        for r in 0..rows {
            let ins: Vec<String> = names.iter().zip(&cols).map(|(n, c)| format!("{:>w$}", c[r], w = n.len())).collect();
            t += &format!("{} | {}\n", ins.join(" "), out[r]);
        }
        it.print(&t);
        return Ok(vec![]);
    }
    let mut data = cols.concat();
    data.extend(out);
    num(Matrix::new(rows, k + 1, data))
}

#[cfg(test)]
mod tests {
    use crate::lang::Interp;
    use crate::value::{Matrix, Value};

    fn run(src: &str) -> Interp {
        let mut it = Interp::new();
        it.eval(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        it
    }
    fn strs(v: &Value) -> Vec<String> {
        match v {
            Value::Cell(c) => c.iter().map(|s| s.as_str().unwrap().to_string()).collect(),
            v => vec![v.as_str().unwrap().to_string()],
        }
    }

    #[test]
    fn bases() {
        let it = run("a = dec2bin(5); b = dec2bin(5, 8); c = dec2hex(255); d = dec2bin([1 10]); e = dec2base(35, 36); f = dec2bin(-5);");
        for (k, want) in [("a", vec!["101"]), ("b", vec!["00000101"]), ("c", vec!["FF"]), ("d", vec!["0001", "1010"]), ("e", vec!["Z"]), ("f", vec!["11111011"])] {
            assert_eq!(strs(&it.vars[k]), want, "{k}");
        }
        let it = run("a = bin2dec('1111 0000'); b = hex2dec('ff'); c = base2dec('zz', 36); d = hex2dec({'A', '10'}); e = bin2dec('102');");
        assert_eq!(it.vars["a"].scalar(), Ok(240.0));
        assert_eq!(it.vars["b"].scalar(), Ok(255.0));
        assert_eq!(it.vars["c"].scalar(), Ok(1295.0));
        assert_eq!(it.vars["d"].matrix().unwrap(), Matrix::col(vec![10.0, 16.0]));
        assert!(it.vars["e"].scalar().unwrap().is_nan());
        let mut it = Interp::new();
        for bad in ["dec2bin(1.5)", "dec2base(5, 1)", "bitand(-1, 3)", "bitcmp(5, 60)", "truthtable(3)"] {
            assert!(it.eval(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn bit_ops() {
        let it = run("a = bitand(12, [10 6]); b = bitor(12, 3); c = bitxor(12, 10); d = bitshift(1, [4 -1]); e = bitshift(16, -2); f = bitcmp(5); g = bitcmp(5, 4); h = xor([1 0 1], [1 1 0]); k = bitshift(1, 60);");
        assert_eq!(it.vars["a"].matrix().unwrap().data, [8.0, 4.0]);
        assert_eq!(it.vars["b"].scalar(), Ok(15.0));
        assert_eq!(it.vars["c"].scalar(), Ok(6.0));
        assert_eq!(it.vars["d"].matrix().unwrap().data, [16.0, 0.0]);
        assert_eq!(it.vars["e"].scalar(), Ok(4.0));
        assert_eq!(it.vars["f"].scalar(), Ok(250.0)); // same as the calculator's bitcmp(5)
        assert_eq!(it.vars["g"].scalar(), Ok(10.0));
        assert!(matches!(&it.vars["h"], Value::Bool(m) if m.data == [0.0, 1.0, 1.0]));
        assert_eq!(it.vars["k"].scalar(), Ok(0.0)); // past 53 bits
    }

    #[test]
    fn truth_tables() {
        let mut it = run("t = truthtable('a & ~b | c'); u = truthtable('p && q');");
        let t = it.vars["t"].matrix().unwrap();
        assert_eq!((t.rows, t.cols), (8, 4));
        // rows abc = 000..111; output a & ~b | c
        assert_eq!((0..8).map(|r| t.get(r, 3)).collect::<Vec<_>>(), [0., 1., 0., 1., 1., 1., 0., 1.]);
        assert_eq!((0..8).map(|r| t.get(r, 0)).collect::<Vec<_>>(), [0., 0., 0., 0., 1., 1., 1., 1.]);
        assert_eq!(it.vars["u"].matrix().unwrap().data, [0., 0., 1., 1., 0., 1., 0., 1., 0., 0., 0., 1.]);
        it.eval("x = 1; truthtable('x & y')").unwrap(); // x is a variable now: only y varies
        let out = it.take_output();
        assert!(out.starts_with("y | x & y\n--+-") && out.contains("0 | 0\n1 | 1\n"), "{out}");
        assert!(!it.vars.contains_key("ans"));
    }
}
