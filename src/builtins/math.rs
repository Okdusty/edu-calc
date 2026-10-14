//! Element-wise math, number theory, constructors and shape functions, RNG, physics units.
use std::cell::Cell;
use std::f64::consts::{LN_2, LN_10, PI, TAU};

use super::{Elem, R, bcast, boolean, chars, is_opt, nargs, num, on, real_arg, reshaped};
use crate::lang::Interp;
use crate::value::{Builtin, C64, Mat, Matrix, Value};

// ponytail: C libm via FFI until f64::gamma / f64::erf leave nightly
unsafe extern "C" {
    pub(super) safe fn erf(x: f64) -> f64;
    pub(super) safe fn erfc(x: f64) -> f64;
    safe fn tgamma(x: f64) -> f64;
    pub(super) safe fn lgamma(x: f64) -> f64;
}

pub fn lookup(name: &str) -> Option<Builtin> {
    let f: Builtin = match name {
        "sin" => |_, a, _| cmap("sin", a, f64::sin, C64::sin, never),
        "cos" => |_, a, _| cmap("cos", a, f64::cos, C64::cos, never),
        "tan" => |_, a, _| cmap("tan", a, f64::tan, C64::tan, never),
        "sec" => |_, a, _| cmap("sec", a, |x| 1.0 / x.cos(), |z| z.cos().inv(), never),
        "csc" => |_, a, _| cmap("csc", a, |x| 1.0 / x.sin(), |z| z.sin().inv(), never),
        "cot" => |_, a, _| cmap("cot", a, |x| 1.0 / x.tan(), |z| z.tan().inv(), never),
        "asin" => |_, a, _| cmap("asin", a, f64::asin, C64::asin, |x| x.abs() > 1.0),
        "acos" => |_, a, _| cmap("acos", a, f64::acos, C64::acos, |x| x.abs() > 1.0),
        "atan" => |_, a, _| cmap("atan", a, f64::atan, C64::atan, never),
        "sinh" => |_, a, _| cmap("sinh", a, f64::sinh, C64::sinh, never),
        "cosh" => |_, a, _| cmap("cosh", a, f64::cosh, C64::cosh, never),
        "tanh" => |_, a, _| cmap("tanh", a, f64::tanh, C64::tanh, never),
        "asinh" => |_, a, _| cmap("asinh", a, f64::asinh, C64::asinh, never),
        "acosh" => |_, a, _| cmap("acosh", a, f64::acosh, C64::acosh, |x| x < 1.0),
        "atanh" => |_, a, _| cmap("atanh", a, f64::atanh, catanh, |x| x.abs() > 1.0),
        "sind" => |_, a, _| map("sind", a, sind),
        "cosd" => |_, a, _| map("cosd", a, cosd),
        "tand" => |_, a, _| map("tand", a, tand),
        "asind" => |_, a, _| map("asind", a, |x| x.asin().to_degrees()),
        "acosd" => |_, a, _| map("acosd", a, |x| x.acos().to_degrees()),
        "atand" => |_, a, _| map("atand", a, |x| x.atan().to_degrees()),
        "exp" => |_, a, _| cmap("exp", a, f64::exp, C64::exp, never),
        "expm1" => |_, a, _| map("expm1", a, f64::exp_m1),
        "log" => |_, a, _| cmap("log", a, f64::ln, C64::ln, neg),
        "log1p" => |_, a, _| map("log1p", a, f64::ln_1p),
        // via |z| and arg so log10(-10) has real part exactly 1, like Octave's rc_log10
        "log10" => |_, a, _| cmap("log10", a, f64::log10, |z| C64::new(z.norm().log10(), z.arg() / LN_10), neg),
        "log2" => |_, a, _| cmap("log2", a, f64::log2, |z| C64::new(z.norm().log2(), z.arg() / LN_2), neg),
        "sqrt" => |_, a, _| cmap("sqrt", a, f64::sqrt, C64::sqrt, neg),
        "realsqrt" => |_, a, _| real_only("realsqrt", a, f64::sqrt, neg),
        "reallog" => |_, a, _| real_only("reallog", a, f64::ln, neg),
        "cbrt" => |_, a, _| map("cbrt", a, f64::cbrt),
        "abs" => |_, a, _| part("abs", a, f64::abs, C64::norm),
        "angle" | "arg" => |_, a, _| part("angle", a, |x| 0f64.atan2(x), C64::arg),
        "sign" => |_, a, _| {
            let sign = |x: f64| if x > 0.0 { 1.0 } else if x < 0.0 { -1.0 } else { x };
            cmap("sign", a, sign, |z| if z.norm() == 0.0 { z } else { z.unscale(z.norm()) }, never)
        },
        "floor" => |_, a, _| cmap("floor", a, f64::floor, |z| C64::new(z.re.floor(), z.im.floor()), never),
        "ceil" => |_, a, _| cmap("ceil", a, f64::ceil, |z| C64::new(z.re.ceil(), z.im.ceil()), never),
        "round" => |_, a, _| cmap("round", a, f64::round, |z| C64::new(z.re.round(), z.im.round()), never),
        "fix" => |_, a, _| cmap("fix", a, f64::trunc, |z| C64::new(z.re.trunc(), z.im.trunc()), never),
        "gamma" => |_, a, _| map("gamma", a, |x| if x <= 0.0 && x.fract() == 0.0 { f64::INFINITY } else { tgamma(x) }),
        "gammaln" | "lgamma" => |_, a, _| map("gammaln", a, |x| lgamma(x)),
        "factorial" => factorial,
        "erf" => |_, a, _| map("erf", a, |x| erf(x)),
        "erfc" => |_, a, _| map("erfc", a, |x| erfc(x)),
        "erfinv" => |_, a, _| map("erfinv", a, erfinv),
        "deg2rad" => |_, a, _| map("deg2rad", a, f64::to_radians),
        "rad2deg" => |_, a, _| map("rad2deg", a, f64::to_degrees),
        "real" => |_, a, _| part("real", a, |x| x, |z| z.re),
        "imag" => |_, a, _| part("imag", a, |_| 0.0, |z| z.im),
        "conj" => |_, a, _| cmap("conj", a, |x| x, |z| z.conj(), never),
        "isnan" => |_, a, _| test("isnan", a, f64::is_nan, C64::is_nan),
        "isinf" => |_, a, _| test("isinf", a, f64::is_infinite, C64::is_infinite),
        "isfinite" => |_, a, _| test("isfinite", a, f64::is_finite, C64::is_finite),
        "isprime" => |_, a, _| {
            nargs("isprime", a, 1, 1)?;
            boolean(real_arg("isprime", &a[0])?.map(|x| is_prime(x) as u8 as f64))
        },
        "atan2" => |_, a, _| map2("atan2", a, f64::atan2),
        "hypot" => |_, a, _| map2("hypot", a, f64::hypot),
        "mod" => |_, a, _| map2("mod", a, modulo),
        "rem" => |_, a, _| map2("rem", a, rem),
        "power" => |_, a, _| power("power", a),
        "realpow" => |_, a, _| {
            let r = power("realpow", a)?;
            if r[0].is_complex() { Err("realpow: produced complex result".into()) } else { Ok(r) }
        },
        "nthroot" => nthroot,
        "gcd" => |_, a, _| int_fold("gcd", a, gcd),
        "lcm" => |_, a, _| int_fold("lcm", a, |x, y| if x == 0.0 || y == 0.0 { 0.0 } else { (x / gcd(x, y) * y).abs() }),
        "nchoosek" => nchoosek,
        "primes" => primes,
        "factor" => factor,
        "zeros" => |_, a, _| num(fill("zeros", a, || 0.0)?),
        "ones" => |_, a, _| num(fill("ones", a, || 1.0)?),
        "eye" => |_, a, _| {
            let (r, c) = dims("eye", a)?;
            let mut m = Matrix::zeros(r, c);
            (0..r.min(c)).for_each(|i| m.set(i, i, 1.0));
            num(m)
        },
        "rand" => |_, a, _| random("rand", a, uniform),
        "randn" => |_, a, _| random("randn", a, normal),
        "randi" => randi,
        "rng" => |_, a, _| {
            nargs("rng", a, 1, 2)?;
            reseed(&a[0]);
            Ok(vec![])
        },
        "linspace" => |_, a, _| {
            nargs("linspace", a, 2, 3)?;
            let n = a.get(2).map(|v| v.scalar()).transpose()?.unwrap_or(100.0);
            if a[..2].iter().any(Value::is_complex) {
                // componentwise, so both ends stay exact
                let z = |v: &Value| v.cmatrix().and_then(|m| if m.is_scalar() { Ok(m.data[0]) } else { Err("linspace: START and END must be scalars".into()) });
                let (lo, hi) = (z(&a[0])?, z(&a[1])?);
                let (re, im) = (linspace(lo.re, hi.re, n)?, linspace(lo.im, hi.im, n)?);
                return Ok(vec![Value::complex(Mat::row(re.into_iter().zip(im).map(|(r, i)| C64::new(r, i)).collect()))]);
            }
            num(Matrix::row(linspace(a[0].scalar()?, a[1].scalar()?, n)?))
        },
        "logspace" => |_, a, _| {
            nargs("logspace", a, 2, 3)?;
            let n = a.get(2).map(|v| v.scalar()).transpose()?.unwrap_or(50.0);
            num(Matrix::row(linspace(a[0].scalar()?, a[1].scalar()?, n)?).map(|x| 10f64.powf(x)))
        },
        "meshgrid" => |_, a, _| grid("meshgrid", a, false),
        "ndgrid" => |_, a, _| grid("ndgrid", a, true),
        "repmat" => |_, a, _| {
            nargs("repmat", a, 2, 3)?;
            let (m, n) = dims("repmat", &a[1..])?;
            Ok(vec![reshaped(&a[0], |x| Ok(tile(&x, m, n)))?])
        },
        "diag" => diag,
        "magic" => |_, a, _| {
            nargs("magic", a, 1, 1)?;
            let n = a[0].scalar()?.max(0.0);
            if !(n <= 3e4) {
                return Err("magic: N is too large".into());
            }
            num(magic(n as usize))
        },
        "vander" => |_, a, _| {
            nargs("vander", a, 1, 2)?;
            let c = a[0].matrix()?.data;
            let n = a.get(1).map(|v| v.as_usize()).transpose()?.unwrap_or(c.len());
            let k = c.len();
            if n > 100_000 {
                return Err("vander: N is too large".into());
            }
            num(Matrix::new(k, n, (0..k * n).map(|q| c[q % k].powi((n - 1 - q / k) as i32)).collect()))
        },
        "cat" => |_, a, _| {
            let d = a.first().ok_or("Invalid call to cat")?.as_usize().map_err(|_| "cat: DIM must be a valid dimension")?;
            Ok(vec![concat(d, &a[1..])?])
        },
        "horzcat" => |_, a, _| Ok(vec![concat(2, a)?]),
        "vertcat" => |_, a, _| Ok(vec![concat(1, a)?]),
        "reshape" => reshape,
        "fliplr" => |_, a, _| flip_b("fliplr", a, Some(2)),
        "flipud" => |_, a, _| flip_b("flipud", a, Some(1)),
        "flip" => |_, a, _| flip_b("flip", a, None),
        "circshift" => circshift,
        "rot90" => |_, a, _| {
            nargs("rot90", a, 1, 2)?;
            let k = a.get(1).map(|v| v.scalar()).transpose()?.unwrap_or(1.0) as i64;
            let v = reshaped(&a[0], |m| {
                Ok(match k.rem_euclid(4) {
                    0 => m,
                    1 => flip(m.transpose(), 1),
                    2 => flip(flip(m, 1), 2),
                    _ => flip(m.transpose(), 2),
                })
            })?;
            Ok(vec![v])
        },
        "triu" => |_, a, _| tri("triu", a, true),
        "tril" => |_, a, _| tri("tril", a, false),
        "kron" => |_, a, _| {
            nargs("kron", a, 2, 2)?;
            if a.iter().any(Value::is_complex) { kron(&a[0].cmatrix()?, &a[1].cmatrix()?) } else { kron(&a[0].matrix()?, &a[1].matrix()?) }
        },
        "colon" => |_, a, _| {
            nargs("colon", a, 2, 3)?;
            let v: Vec<f64> = a.iter().map(|x| x.scalar()).collect::<Result<_, _>>()?;
            let (b, s, l) = if v.len() == 2 { (v[0], 1.0, v[1]) } else { (v[0], v[1], v[2]) };
            num(range(b, s, l)?)
        },
        "numel" => |_, a, _| size_fn("numel", a, |r, c| (r * c) as f64),
        "length" => |_, a, _| size_fn("length", a, |r, c| if r * c == 0 { 0.0 } else { r.max(c) as f64 }),
        "ndims" => |_, a, _| size_fn("ndims", a, |_, _| 2.0),
        "rows" => |_, a, _| size_fn("rows", a, |r, _| r as f64),
        "columns" => |_, a, _| size_fn("columns", a, |_, c| c as f64),
        "size" => size,
        "isempty" => |_, a, _| shape_test("isempty", a, |r, c| r * c == 0),
        "isscalar" => |_, a, _| shape_test("isscalar", a, |r, c| r * c == 1),
        "isvector" => |_, a, _| shape_test("isvector", a, |r, c| r == 1 || c == 1),
        "ismatrix" => |_, a, _| shape_test("ismatrix", a, |_, _| true),
        "issquare" => |_, a, _| shape_test("issquare", a, |r, c| r == c),
        "isrow" => |_, a, _| shape_test("isrow", a, |r, _| r == 1),
        "iscolumn" => |_, a, _| shape_test("iscolumn", a, |_, c| c == 1),
        "isequal" => |_, a, _| {
            nargs("isequal", a, 2, usize::MAX)?;
            Ok(vec![Value::bool(a[1..].iter().all(|b| same(&a[0], b)))])
        },
        "physconst" => physconst,
        "unitconv" => unitconv,
        "db" => |_, a, _| {
            nargs("db", a, 1, 2)?;
            let power = a.get(1).is_some_and(|v| is_opt(v, "power"));
            num(a[0].matrix()?.map(|x| if power { 10.0 * x.log10() } else { 20.0 * x.abs().log10() }))
        },
        "mag2db" => |_, a, _| map("mag2db", a, |x| 20.0 * x.log10()),
        "db2mag" => |_, a, _| map("db2mag", a, |x| 10f64.powf(x / 20.0)),
        _ => return None,
    };
    Some(f)
}

fn map(name: &str, a: &[Value], f: impl Fn(f64) -> f64) -> R {
    nargs(name, a, 1, 1)?;
    num(real_arg(name, &a[0])?.map(f))
}

fn never(_: f64) -> bool {
    false
}

fn neg(x: f64) -> bool {
    x < 0.0
}

/// Element-wise with a complex twin `fc`. Real input stays on `f` unless some element leaves
/// the real domain (`promote`, e.g. sqrt(-4)); those elements then go through `fc`, like Octave.
fn cmap(name: &str, a: &[Value], f: fn(f64) -> f64, fc: fn(C64) -> C64, promote: fn(f64) -> bool) -> R {
    nargs(name, a, 1, 1)?;
    let m = match &a[0] {
        Value::Complex(z) => return Ok(vec![Value::complex(z.map(fc))]),
        v => v.matrix()?,
    };
    if !m.data.iter().any(|&x| promote(x)) {
        return num(m.map(f));
    }
    Ok(vec![Value::complex(m.map(|x| if promote(x) { fc(C64::new(x, 0.0)) } else { C64::new(f(x), 0.0) }))])
}

/// realsqrt / reallog: error where the result would be complex.
fn real_only(name: &str, a: &[Value], f: fn(f64) -> f64, bad: fn(f64) -> bool) -> R {
    nargs(name, a, 1, 1)?;
    let m = real_arg(name, &a[0])?;
    if m.data.iter().any(|&x| bad(x)) {
        return Err(format!("{name}: produced complex result"));
    }
    num(m.map(f))
}

/// num-complex puts atanh(x > 1) on the lower branch; Octave (C99 catanh) uses the upper one.
fn catanh(z: C64) -> C64 {
    if z.im == 0.0 && z.re > 1.0 { z.atanh().conj() } else { z.atanh() }
}

/// Complex -> real element-wise (abs, angle, real, imag).
fn part(name: &str, a: &[Value], f: fn(f64) -> f64, fc: fn(C64) -> f64) -> R {
    nargs(name, a, 1, 1)?;
    on(&a[0], |m| num(m.map(f)), |z| num(z.map(fc)))
}

fn test(name: &str, a: &[Value], f: fn(f64) -> bool, fc: fn(C64) -> bool) -> R {
    nargs(name, a, 1, 1)?;
    on(&a[0], |m| boolean(m.map(|x| f(x) as u8 as f64)), |z| boolean(z.map(|x| fc(x) as u8 as f64)))
}

fn map2(name: &str, a: &[Value], f: impl Fn(f64, f64) -> f64) -> R {
    nargs(name, a, 2, 2)?;
    num(bcast(name, &[real_arg(name, &a[0])?, real_arg(name, &a[1])?], |v| f(v[0], v[1]))?)
}

/// power(x, y) is exactly `x .^ y`, so both spellings agree (incl. the complex promotion).
fn power(name: &str, a: &[Value]) -> R {
    nargs(name, a, 2, 2)?;
    Ok(vec![super::ops::binop(crate::value::BinOp::EPow, &a[0], &a[1])?])
}

fn kron<T: Elem>(x: &Mat<T>, y: &Mat<T>) -> R {
    let (r, c) = (x.rows * y.rows, x.cols * y.cols);
    Ok(vec![T::value(Mat::new(r, c, (0..r * c).map(|k| {
        let (i, j) = (k % r, k / r);
        x.get(i / y.rows, j / y.cols) * y.get(i % y.rows, j % y.cols)
    }).collect()))])
}

fn sind(x: f64) -> f64 {
    let r = x % 360.0;
    if r % 180.0 == 0.0 { 0.0 } else { r.to_radians().sin() }
}

fn cosd(x: f64) -> f64 {
    let r = x % 360.0;
    if (r - 90.0) % 180.0 == 0.0 { 0.0 } else { r.to_radians().cos() }
}

fn tand(x: f64) -> f64 {
    let r = x % 360.0;
    if r % 180.0 == 0.0 {
        0.0
    } else if (r - 90.0) % 180.0 == 0.0 {
        f64::INFINITY
    } else {
        r.to_radians().tan()
    }
}

/// Octave mod: result has the sign of y; mod(x, 0) = x.
fn modulo(x: f64, y: f64) -> f64 {
    if y == 0.0 {
        return x;
    }
    let q = x / y;
    if y.fract() != 0.0 && (q - q.round()).abs() < f64::EPSILON * q.round().abs() {
        return 0.0;
    }
    x - q.floor() * y
}

/// Octave rem: result has the sign of x; rem(x, 0) = x.
fn rem(x: f64, y: f64) -> f64 {
    if y == 0.0 {
        return x;
    }
    let q = x / y;
    if y.fract() != 0.0 && (q - q.round()).abs() < f64::EPSILON * q.round().abs() {
        return 0.0;
    }
    x - q.trunc() * y
}

fn gcd(a: f64, b: f64) -> f64 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b > 0.0 {
        (a, b) = (b, a % b);
    }
    a
}

fn int_fold(name: &str, a: &[Value], f: fn(f64, f64) -> f64) -> R {
    nargs(name, a, 2, usize::MAX)?;
    let mut acc = real_arg(name, &a[0])?;
    for v in &a[1..] {
        acc = bcast(name, &[acc, real_arg(name, v)?], |v| f(v[0], v[1]))?;
    }
    if acc.data.iter().any(|x| x.fract() != 0.0) {
        return Err(format!("{name}: all values must be integers"));
    }
    num(acc)
}

/// Inverse error function: Giles' approximation polished by Newton on erf / erfc.
pub(super) fn erfinv(y: f64) -> f64 {
    if !(-1.0..=1.0).contains(&y) {
        return f64::NAN;
    }
    let (s, y) = (y.signum(), y.abs());
    if y == 1.0 {
        return s * f64::INFINITY;
    }
    let w = -((1.0 - y) * (1.0 + y)).ln();
    let (w, c): (f64, &[f64]) = if w < 5.0 {
        (w - 2.5, &[2.81022636e-08, 3.43273939e-07, -3.5233877e-06, -4.39150654e-06, 0.00021858087, -0.00125372503, -0.00417768164, 0.246640727, 1.50140941])
    } else {
        (w.sqrt() - 3.0, &[-0.000200214257, 0.000100950558, 0.00134934322, -0.00367342844, 0.00573950773, -0.0076224613, 0.00943887047, 1.00167406, 2.83297682])
    };
    let mut x = c.iter().fold(0.0, |p, &k| p * w + k) * y;
    for _ in 0..3 {
        // erf(x) - y, written via erfc near 1 where 1 - y is exact
        let err = if y < 0.5 { erf(x) - y } else { (1.0 - y) - erfc(x) };
        x -= err / (2.0 / PI.sqrt() * (-x * x).exp());
    }
    s * x
}

fn factorial(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("factorial", a, 1, 1)?;
    let m = a[0].matrix()?;
    if m.data.iter().any(|&x| x < 0.0 || x.fract() != 0.0 || x.is_nan()) {
        return Err("factorial: all N must be real non-negative integers".into());
    }
    num(m.map(|x| tgamma(x + 1.0).round()))
}

fn nthroot(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("nthroot", a, 2, 2)?;
    let x = real_arg("nthroot", &a[0])?;
    let n = a[1].scalar().map_err(|_| "nthroot: N must be a nonzero scalar")?;
    if n == 0.0 {
        return Err("nthroot: N must be a nonzero scalar".into());
    }
    let odd = n.fract() == 0.0 && n % 2.0 != 0.0;
    if !odd && x.data.iter().any(|&v| v < 0.0) {
        return Err("nthroot: N must be an odd integer if X contains negative values".into());
    }
    num(x.map(|v| {
        let y = if v < 0.0 { -(-v).powf(1.0 / n) } else { v.powf(1.0 / n) };
        // one Newton step makes exact roots exact: nthroot(27, 3) == 3
        if n.fract() == 0.0 && y.is_finite() && y != 0.0 { y - (y.powf(n) - v) / (n * y.powf(n - 1.0)) } else { y }
    }))
}

fn nchoosek(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("nchoosek", a, 2, 2)?;
    let v = a[0].matrix()?;
    let k = a[1].as_usize().map_err(|_| "nchoosek: K must be a non-negative integer")?;
    if v.len() == 1 {
        let n = v.data[0];
        if n < 0.0 || n.fract() != 0.0 {
            return Err("nchoosek: N must be a non-negative integer".into());
        }
        let k = k as f64;
        if k > n {
            return num(Matrix::scalar(0.0));
        }
        let k = k.min(n - k);
        if k > 1000.0 {
            return num(Matrix::scalar((lgamma(n + 1.0) - lgamma(k + 1.0) - lgamma(n - k + 1.0)).exp().round()));
        }
        let c = (1..=k as u64).fold(1.0, |c, i| c * (n - k + i as f64) / i as f64);
        return num(Matrix::scalar(c.round()));
    }
    // all k-combinations of the elements, one per row
    let n = v.len();
    if k > n {
        return num(Matrix::zeros(0, k));
    }
    let count = (1..=k).fold(1.0, |c, i| c * (n - k + i) as f64 / i as f64);
    if count > 1e7 {
        return Err("nchoosek: result is too large".into());
    }
    let mut rows = vec![];
    let mut idx: Vec<usize> = (0..k).collect();
    loop {
        rows.push(idx.iter().map(|&i| v.data[i]).collect::<Vec<f64>>());
        let mut i = k;
        while i > 0 && idx[i - 1] == n - k + i - 1 {
            i -= 1;
        }
        if i == 0 {
            break;
        }
        idx[i - 1] += 1;
        for j in i..k {
            idx[j] = idx[j - 1] + 1;
        }
    }
    num(Matrix::from_rows(&rows))
}

fn primes(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("primes", a, 1, 1)?;
    let n = a[0].scalar()?;
    if n > 1e8 {
        return Err("primes: N is too large".into()); // ponytail: plain sieve, segmented if anyone needs more
    }
    let n = if n >= 2.0 { n as usize } else { return num(Matrix::zeros(1, 0)) };
    let mut sieve = vec![true; n + 1];
    let mut out = vec![];
    for i in 2..=n {
        if sieve[i] {
            out.push(i as f64);
            (i * i..=n).step_by(i).for_each(|j| sieve[j] = false);
        }
    }
    num(Matrix::row(out))
}

/// Deterministic Miller–Rabin for every u64.
fn is_prime(x: f64) -> bool {
    if x < 2.0 || x.fract() != 0.0 || x >= 18_446_744_073_709_551_616.0 {
        return false;
    }
    let n = x as u64;
    const BASES: [u64; 12] = [2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37];
    if let Some(&p) = BASES.iter().find(|&&p| n % p == 0) {
        return n == p;
    }
    let mul = |a: u64, b: u64| (a as u128 * b as u128 % n as u128) as u64;
    let pow = |mut b: u64, mut e: u64| {
        let mut r = 1;
        while e > 0 {
            if e & 1 == 1 {
                r = mul(r, b);
            }
            b = mul(b, b);
            e >>= 1;
        }
        r
    };
    let s = (n - 1).trailing_zeros();
    let d = (n - 1) >> s;
    'witness: for a in BASES {
        let mut x = pow(a, d);
        if x == 1 || x == n - 1 {
            continue;
        }
        for _ in 1..s {
            x = mul(x, x);
            if x == n - 1 {
                continue 'witness;
            }
        }
        return false;
    }
    true
}

fn factor(_: &mut Interp, a: &[Value], nargout: usize) -> R {
    nargs("factor", a, 1, 1)?;
    let q = a[0].scalar()?;
    if q < 0.0 || q.fract() != 0.0 || !q.is_finite() {
        return Err("factor: Q must be a real non-negative integer".into());
    }
    if q > 2f64.powi(53) {
        return Err("factor: Q too large to factor (> flintmax)".into());
    }
    let mut n = q as u64;
    let mut p = vec![];
    if n < 4 {
        p.push(n);
    } else {
        let mut d = 2;
        while d * d <= n {
            while n % d == 0 {
                p.push(d);
                n /= d;
            }
            d += if d == 2 { 1 } else { 2 };
        }
        if n > 1 {
            p.push(n);
        }
    }
    if nargout < 2 {
        return num(Matrix::row(p.into_iter().map(|x| x as f64).collect()));
    }
    let mut u: Vec<f64> = p.iter().map(|&x| x as f64).collect();
    u.dedup();
    let k: Vec<f64> = u.iter().map(|&x| p.iter().filter(|&&y| y as f64 == x).count() as f64).collect();
    Ok(vec![Value::from(u), Value::from(k)])
}

/// Size args of zeros/ones/rand...: (), (n), (r, c), ([r c]); class names like "double" ignored.
fn dims(name: &str, a: &[Value]) -> Result<(usize, usize), String> {
    let a: Vec<&Value> = a.iter().filter(|v| !matches!(v, Value::Str(_))).collect();
    let v: Vec<f64> = match a.as_slice() {
        [] => vec![1.0, 1.0],
        [x] => {
            let m = x.matrix()?;
            if m.len() == 1 { vec![m.data[0]; 2] } else { m.data }
        }
        many => many.iter().map(|x| x.scalar()).collect::<Result<_, _>>()?,
    };
    if v.len() < 2 || v[2..].iter().any(|&d| d != 1.0) {
        return Err(format!("{name}: only 2-D sizes are supported"));
    }
    if v[0].is_nan() || v[1].is_nan() {
        return Err(format!("{name}: NaN is invalid as size specification"));
    }
    let (r, c) = (v[0].max(0.0).floor(), v[1].max(0.0).floor());
    if !(r <= 1e9 && c <= 1e9 && r * c <= 1e9) {
        return Err(format!("{name}: out of memory or dimension too large for Octave's index type"));
    }
    Ok((r as usize, c as usize))
}

fn fill(name: &str, a: &[Value], mut f: impl FnMut() -> f64) -> Result<Matrix, String> {
    let (r, c) = dims(name, a)?;
    Ok(Matrix::new(r, c, (0..r * c).map(|_| f()).collect()))
}

thread_local! {
    static RNG: Cell<u64> = Cell::new(
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64),
    );
}

/// splitmix64
fn next_u64() -> u64 {
    RNG.with(|r| {
        let z = r.get().wrapping_add(0x9E37_79B9_7F4A_7C15);
        r.set(z);
        let z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        let z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    })
}

/// Uniform on the open interval (0, 1).
pub(super) fn uniform() -> f64 {
    ((next_u64() >> 11) as f64 + 0.5) / (1u64 << 53) as f64
}

fn normal() -> f64 {
    (-2.0 * uniform().ln()).sqrt() * (TAU * uniform()).cos()
}

/// `rng(n)`, `rng('default')`, `rng('shuffle')`.
fn reseed(v: &Value) {
    let s = match v {
        Value::Str(s) if s == "shuffle" => next_u64() ^ std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64),
        Value::Str(_) => 0,
        v => v.scalar().unwrap_or(0.0).to_bits(),
    };
    RNG.with(|r| r.set(s));
}

/// rand / randn, including the `rand('seed'|'state'|'twister', n)` reseed forms.
fn random(name: &str, a: &[Value], f: fn() -> f64) -> R {
    if let Some(Value::Str(s)) = a.first()
        && ["seed", "state", "twister"].contains(&s.as_str())
    {
        reseed(a.get(1).unwrap_or(&Value::num(0.0)));
        return Ok(vec![]);
    }
    num(fill(name, a, f)?)
}

fn randi(_: &mut Interp, a: &[Value], _: usize) -> R {
    let lim = a.first().ok_or("Invalid call to randi")?.matrix()?;
    let (lo, hi) = match lim.data[..] {
        [hi] => (1.0, hi),
        [lo, hi] => (lo, hi),
        _ => return Err("randi: IMAX must be a scalar or a 2-element vector".into()),
    };
    if lo > hi || lo.fract() != 0.0 || hi.fract() != 0.0 {
        return Err("randi: require IMIN <= IMAX and both integer".into());
    }
    num(fill("randi", &a[1..], || lo + (uniform() * (hi - lo + 1.0)).floor())?)
}

fn linspace(lo: f64, hi: f64, n: f64) -> Result<Vec<f64>, String> {
    if !(n <= 1e9) {
        return Err("linspace: N is too large".into());
    }
    let n = n.floor().max(1.0) as usize;
    Ok((0..n).map(|i| if i + 1 == n { hi } else { lo + (hi - lo) * i as f64 / (n - 1) as f64 }).collect())
}

/// meshgrid: X varies along columns; ndgrid: X varies along rows.
fn grid(name: &str, a: &[Value], nd: bool) -> R {
    nargs(name, a, 1, 2)?;
    let x = a[0].matrix()?.data;
    let y = match a.get(1) {
        Some(v) => v.matrix()?.data,
        None => x.clone(),
    };
    let (u, v) = if nd { (&y, &x) } else { (&x, &y) };
    let (r, c) = (v.len(), u.len());
    let gx = Matrix::new(r, c, (0..r * c).map(|k| u[k / r]).collect());
    let gy = Matrix::new(r, c, (0..r * c).map(|k| v[k % r]).collect());
    Ok(if nd { vec![gy.into(), gx.into()] } else { vec![gx.into(), gy.into()] })
}

fn tile(x: &Matrix, m: usize, n: usize) -> Matrix {
    let (r, c) = (x.rows * m, x.cols * n);
    Matrix::new(r, c, (0..r * c).map(|k| x.get((k % r) % x.rows, (k / r) % x.cols)).collect())
}

fn diag(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("diag", a, 1, 2)?;
    let k = a.get(1).map(|v| v.scalar()).transpose()?.unwrap_or(0.0);
    if k.fract() != 0.0 || k.abs() > 1e5 {
        return Err("diag: K must be an integer".into());
    }
    let k = k as i64;
    let v = reshaped(&a[0], |m| {
        if m.rows == 1 || m.cols == 1 {
            let n = m.len() + k.unsigned_abs() as usize;
            let mut out = Matrix::zeros(n, n);
            for (i, &v) in m.data.iter().enumerate() {
                let (r, c) = if k >= 0 { (i, i + k as usize) } else { (i + k.unsigned_abs() as usize, i) };
                out.set(r, c, v);
            }
            return Ok(out);
        }
        let v = (0..m.rows).filter_map(|i| {
            let j = i as i64 + k;
            (j >= 0 && (j as usize) < m.cols).then(|| m.get(i, j as usize))
        });
        Ok(Matrix::col(v.collect()))
    })?;
    Ok(vec![v])
}

/// Octave's magic.m: odd (siamese), doubly even, singly even (LUX-style swaps).
fn magic(n: usize) -> Matrix {
    let mut a = Matrix::zeros(n, n);
    if n <= 1 {
        return Matrix::filled(n, n, 1.0);
    }
    if n % 2 == 1 {
        for k in 0..n * n {
            let shift = k / n;
            let c = (k + 1 - shift + (n - 3) / 2) % n;
            let r = (n * n - k + 2 * shift) % n;
            a.data[c * n + r] = (k + 1) as f64;
        }
    } else if n % 4 == 0 {
        let edge = |t: usize| t % 4 == 0 || t % 4 == 3;
        for i in 0..n {
            for j in 0..n {
                let v = (i * n + j + 1) as f64;
                a.set(i, j, if edge(i) == edge(j) { (n * n + 1) as f64 - v } else { v });
            }
        }
    } else {
        let m = n / 2;
        let b = magic(m);
        let q = (m * m) as f64;
        for i in 0..m {
            for j in 0..m {
                let v = b.get(i, j);
                a.set(i, j, v);
                a.set(i, j + m, v + 2.0 * q);
                a.set(i + m, j, v + 3.0 * q);
                a.set(i + m, j + m, v + q);
            }
        }
        let swap = |a: &mut Matrix, i: usize, j: usize| {
            let t = a.get(i, j);
            a.set(i, j, a.get(i + m, j));
            a.set(i + m, j, t);
        };
        let k = (m - 1) / 2;
        if k > 1 {
            for j in (1..k).chain(n - k + 1..n) {
                (0..m).for_each(|i| swap(&mut a, i, j));
            }
        }
        (0..k).chain(k + 1..m).for_each(|i| swap(&mut a, i, 0));
        swap(&mut a, k, k);
    }
    a
}

/// cat along dim 1 or 2. Empty arrays are skipped; char wins over numbers, cells concat flat.
pub(super) fn concat(dim: usize, vals: &[Value]) -> Result<Value, String> {
    let parts: Vec<&Value> = vals.iter().filter(|v| v.size().0 * v.size().1 > 0).collect();
    if vals.iter().any(|v| matches!(v, Value::Cell(_))) {
        if dim != 2 && parts.len() > 1 {
            return Err("vertcat: 2-D cell arrays are not supported".into());
        }
        let mut out = vec![];
        for v in parts {
            match v {
                Value::Cell(c) => out.extend(c.iter().cloned()),
                v => return Err(format!("concatenation operator not implemented for 'cell' by '{}' operations", v.type_name())),
            }
        }
        return Ok(Value::Cell(out));
    }
    if parts.iter().any(|v| v.is_complex()) {
        let ms: Vec<_> = parts.iter().map(|v| v.cmatrix()).collect::<Result<_, _>>()?;
        return Ok(Value::complex(cat(dim, &ms)?));
    }
    let ms: Vec<Matrix> = parts.iter().map(|v| v.matrix()).collect::<Result<_, _>>()?;
    let m = cat(dim, &ms)?;
    if vals.iter().any(|v| matches!(v, Value::Str(_))) {
        if m.rows > 1 {
            return Err("vertcat: char matrices are not supported".into()); // ponytail: strings are rows
        }
        return Ok(Value::Str(chars(&m.data)));
    }
    if !parts.is_empty() && parts.iter().all(|v| matches!(v, Value::Bool(_))) {
        return Ok(Value::Bool(m));
    }
    Ok(Value::Num(m))
}

fn cat<T: Copy + Default>(dim: usize, ms: &[Mat<T>]) -> Result<Mat<T>, String> {
    let mismatch = |w: &str, a: &Mat<T>, b: &Mat<T>| Err(format!("{w} dimensions mismatch ({}x{} vs {}x{})", a.rows, a.cols, b.rows, b.cols));
    Ok(match (dim, ms.first()) {
        (_, None) => Mat::empty(),
        (_, Some(m0)) if ms.len() == 1 => m0.clone(),
        (1, Some(m0)) => {
            if let Some(b) = ms.iter().find(|m| m.cols != m0.cols) {
                return mismatch("vertical", m0, b);
            }
            let rows = ms.iter().map(|m| m.rows).sum();
            let data = (0..m0.cols).flat_map(|j| ms.iter().flat_map(move |m| (0..m.rows).map(move |i| m.get(i, j)))).collect();
            Mat::new(rows, m0.cols, data)
        }
        (2, Some(m0)) => {
            if let Some(b) = ms.iter().find(|m| m.rows != m0.rows) {
                return mismatch("horizontal", m0, b);
            }
            Mat::new(m0.rows, ms.iter().map(|m| m.cols).sum(), ms.iter().flat_map(|m| m.data.iter().cloned()).collect())
        }
        _ => return Err("cat: N-d arrays are not supported".into()),
    })
}

fn reshape(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("reshape", a, 2, usize::MAX)?;
    let (r0, c0) = a[0].size();
    let mut d: Vec<Option<f64>> = if a.len() == 2 {
        a[1].matrix()?.data.into_iter().map(Some).collect()
    } else {
        a[1..].iter().map(|v| if v.size() == (0, 0) { Ok(None) } else { v.scalar().map(Some) }).collect::<Result<_, _>>()?
    };
    while d.len() > 2 && d.last() == Some(&Some(1.0)) {
        d.pop();
    }
    if d.len() != 2 || d.iter().any(|x| x.is_some_and(|x| x < 0.0 || x.fract() != 0.0)) {
        return Err("reshape: SIZE must be 2-D with non-negative integers".into());
    }
    let total = r0 * c0;
    if d.iter().flatten().any(|&x| x > 1e9) {
        return Err("reshape: out of memory or dimension too large for Octave's index type".into());
    }
    let (r, c) = match (d[0], d[1]) {
        (Some(r), Some(c)) => (r as usize, c as usize),
        (None, Some(k)) | (Some(k), None) => {
            let k = k as usize;
            if k == 0 || total % k != 0 {
                return Err(format!("reshape: SIZE is not divisible by the product of known dimensions (= {k})"));
            }
            if d[0].is_none() { (total / k, k) } else { (k, total / k) }
        }
        _ => return Err("reshape: only a single dimension can be unknown".into()),
    };
    if r * c != total {
        return Err(format!("reshape: can't reshape {r0}x{c0} array to {r}x{c} array"));
    }
    Ok(vec![reshaped(&a[0], |m| Ok(Matrix::new(r, c, m.data)))?])
}

fn flip(m: Matrix, dim: usize) -> Matrix {
    let (r, c) = (m.rows, m.cols);
    let at = |k: usize| match (k % r, k / r) {
        (i, j) if dim == 1 => m.get(r - 1 - i, j),
        (i, j) if dim == 2 => m.get(i, c - 1 - j),
        (i, j) => m.get(i, j),
    };
    Matrix::new(r, c, (0..r * c).map(at).collect())
}

fn flip_b(name: &str, a: &[Value], dim: Option<usize>) -> R {
    nargs(name, a, 1, if dim.is_some() { 1 } else { 2 })?;
    let (r, _) = a[0].size();
    let dim = match (dim, a.get(1)) {
        (Some(d), _) => d,
        (None, Some(d)) => d.as_usize().map_err(|_| format!("{name}: DIM must be a valid dimension"))?,
        (None, None) => if r != 1 { 1 } else { 2 },
    };
    Ok(vec![reshaped(&a[0], |m| Ok(flip(m, dim)))?])
}

pub(super) fn circshift(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("circshift", a, 2, 3)?;
    let (r, c) = a[0].size();
    let n = a[1].matrix()?.data;
    let along = |d: usize, k: f64| if d == 1 { (k, 0.0) } else if d == 2 { (0.0, k) } else { (0.0, 0.0) };
    let (sr, sc) = match (a.get(2), n.as_slice()) {
        (Some(d), [k]) => along(d.as_usize()?, *k),
        (None, [k]) => along(if r != 1 { 1 } else { 2 }, *k),
        (None, [kr, kc]) => (*kr, *kc),
        _ => return Err("circshift: N must be a scalar or a 2-element vector".into()),
    };
    let rot = |k: f64, n: usize| if n == 0 { 0 } else { (k as i64).rem_euclid(n as i64) as usize };
    let (sr, sc) = (rot(sr, r), rot(sc, c));
    let v = reshaped(&a[0], |m| Ok(Matrix::new(r, c, (0..r * c).map(|k| m.get((k % r + r - sr) % r, (k / r + c - sc) % c)).collect())))?;
    Ok(vec![v])
}

fn tri(name: &str, a: &[Value], upper: bool) -> R {
    nargs(name, a, 1, 2)?;
    let k = a.get(1).map(|v| v.scalar()).transpose()?.unwrap_or(0.0) as i64;
    let v = reshaped(&a[0], |m| {
        let keep = |q: usize| {
            let d = (q / m.rows) as i64 - (q % m.rows) as i64;
            if upper { d >= k } else { d <= k }
        };
        Ok(Matrix::new(m.rows, m.cols, (0..m.len()).map(|q| if keep(q) { m.data[q] } else { 0.0 }).collect()))
    })?;
    Ok(vec![v])
}

/// base:inc:limit with Octave's tolerance on the element count and a clamped last element.
pub(super) fn range(base: f64, inc: f64, limit: f64) -> Result<Matrix, String> {
    if inc == 0.0 || [base, inc, limit].iter().any(|x| x.is_nan()) || (limit - base) * inc < 0.0 {
        return Ok(Matrix::zeros(1, 0));
    }
    let q = (limit - base) / inc;
    let n = (q * (1.0 + 3.0 * f64::EPSILON) + 3.0 * f64::EPSILON).floor() + 1.0;
    if !(n <= 1e9) {
        return Err("colon: out of memory or dimension too large for Octave's index type".into());
    }
    let n = n as usize;
    let v = (0..n).map(|i| {
        let x = base + i as f64 * inc;
        if (inc > 0.0 && x > limit) || (inc < 0.0 && x < limit) { limit } else { x }
    });
    Ok(Matrix::row(v.collect()))
}

fn size_fn(name: &str, a: &[Value], f: fn(usize, usize) -> f64) -> R {
    let v = a.first().ok_or_else(|| format!("Invalid call to {name}"))?;
    let (r, c) = v.size();
    num(Matrix::scalar(f(r, c)))
}

fn shape_test(name: &str, a: &[Value], f: fn(usize, usize) -> bool) -> R {
    nargs(name, a, 1, 1)?;
    let (r, c) = a[0].size();
    Ok(vec![Value::bool(f(r, c))])
}

fn size(_: &mut Interp, a: &[Value], nargout: usize) -> R {
    nargs("size", a, 1, 2)?;
    let (r, c) = a[0].size();
    if let Some(d) = a.get(1) {
        let d = d.as_usize().ok().filter(|&d| d >= 1).ok_or("size: requested dimension DIM out of range")?;
        return num(Matrix::scalar(match d {
            1 => r,
            2 => c,
            _ => 1,
        } as f64));
    }
    if nargout <= 1 {
        return num(Matrix::row(vec![r as f64, c as f64]));
    }
    let mut out = vec![Value::num(r as f64), Value::num(c as f64)];
    out.resize(nargout, Value::num(1.0));
    Ok(out)
}

fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Cell(x), Value::Cell(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q)),
        (Value::Struct(x), Value::Struct(y)) => x.len() == y.len() && x.iter().zip(y).all(|((k, p), (l, q))| k == l && same(p, q)),
        (Value::Func(x), Value::Func(y)) => format!("{x:?}") == format!("{y:?}"),
        _ => match (a.cmatrix(), b.cmatrix()) {
            (Ok(x), Ok(y)) => (x.rows, x.cols) == (y.rows, y.cols) && x.data == y.data,
            _ => false,
        },
    }
}

/// CODATA 2018 (SI exact where defined).
const PHYS: &[(&str, f64)] = &[
    ("LightSpeed", 299_792_458.0),
    ("Planck", 6.626_070_15e-34),
    ("ReducedPlanck", 1.054_571_817e-34),
    ("Boltzmann", 1.380_649e-23),
    ("ElementaryCharge", 1.602_176_634e-19),
    ("ElectronMass", 9.109_383_701_5e-31),
    ("ProtonMass", 1.672_621_923_69e-27),
    ("NeutronMass", 1.674_927_498_04e-27),
    ("Avogadro", 6.022_140_76e23),
    ("Gravitational", 6.674_30e-11),
    ("Gravity", 9.806_65),
    ("VacuumPermittivity", 8.854_187_812_8e-12),
    ("VacuumPermeability", 1.256_637_062_12e-6),
    ("StefanBoltzmann", 5.670_374_419e-8),
    ("GasConstant", 8.314_462_618_153_24),
    ("Faraday", 96_485.332_123_310_02),
    ("BohrRadius", 5.291_772_109_03e-11),
    ("FineStructure", 7.297_352_569_3e-3),
    ("AtomicMassUnit", 1.660_539_066_60e-27),
    ("EarthRadius", 6.371e6),
    ("EarthMass", 5.972_2e24),
];

fn physconst(_: &mut Interp, a: &[Value], _: usize) -> R {
    let names = || PHYS.iter().map(|p| p.0).collect::<Vec<_>>().join(", ");
    let name = a.first().and_then(|v| v.as_str().ok()).ok_or_else(|| format!("physconst: NAME must be one of: {}", names()))?;
    match PHYS.iter().find(|p| p.0.eq_ignore_ascii_case(name)) {
        Some(p) => num(Matrix::scalar(p.1)),
        None => Err(format!("physconst: unknown constant '{name}'; valid names: {}", names())),
    }
}

/// (unit, quantity, SI factor). Temperature is affine and handled separately.
const UNITS: &[(&str, &str, f64)] = &[
    ("m", "length", 1.0),
    ("km", "length", 1e3),
    ("cm", "length", 1e-2),
    ("mm", "length", 1e-3),
    ("um", "length", 1e-6),
    ("nm", "length", 1e-9),
    ("in", "length", 0.0254),
    ("ft", "length", 0.3048),
    ("yd", "length", 0.9144),
    ("mi", "length", 1609.344),
    ("nmi", "length", 1852.0),
    ("au", "length", 1.495_978_707e11),
    ("ly", "length", 9.460_730_472_580_8e15),
    ("pc", "length", 3.085_677_581_491_367e16),
    ("kg", "mass", 1.0),
    ("g", "mass", 1e-3),
    ("mg", "mass", 1e-6),
    ("t", "mass", 1e3),
    ("lb", "mass", 0.453_592_37),
    ("oz", "mass", 0.028_349_523_125),
    ("slug", "mass", 14.593_902_937_206_364),
    ("s", "time", 1.0),
    ("ms", "time", 1e-3),
    ("us", "time", 1e-6),
    ("ns", "time", 1e-9),
    ("min", "time", 60.0),
    ("h", "time", 3600.0),
    ("day", "time", 86_400.0),
    ("week", "time", 604_800.0),
    ("yr", "time", 31_557_600.0),
    ("N", "force", 1.0),
    ("kN", "force", 1e3),
    ("dyn", "force", 1e-5),
    ("lbf", "force", 4.448_221_615_260_5),
    ("kgf", "force", 9.806_65),
    ("J", "energy", 1.0),
    ("kJ", "energy", 1e3),
    ("MJ", "energy", 1e6),
    ("cal", "energy", 4.184),
    ("kcal", "energy", 4184.0),
    ("Wh", "energy", 3600.0),
    ("kWh", "energy", 3.6e6),
    ("eV", "energy", 1.602_176_634e-19),
    ("BTU", "energy", 1055.055_852_62),
    ("erg", "energy", 1e-7),
    ("W", "power", 1.0),
    ("kW", "power", 1e3),
    ("MW", "power", 1e6),
    ("hp", "power", 745.699_871_582_270_2),
    ("Pa", "pressure", 1.0),
    ("kPa", "pressure", 1e3),
    ("MPa", "pressure", 1e6),
    ("bar", "pressure", 1e5),
    ("mbar", "pressure", 100.0),
    ("atm", "pressure", 101_325.0),
    ("psi", "pressure", 6894.757_293_168_361),
    ("torr", "pressure", 101_325.0 / 760.0),
    ("mmHg", "pressure", 133.322_387_415),
    ("rad", "angle", 1.0),
    ("deg", "angle", PI / 180.0),
    ("grad", "angle", PI / 200.0),
    ("rev", "angle", TAU),
    ("arcmin", "angle", PI / 10_800.0),
    ("arcsec", "angle", PI / 648_000.0),
    ("m/s", "speed", 1.0),
    ("km/h", "speed", 1.0 / 3.6),
    ("mph", "speed", 0.447_04),
    ("knot", "speed", 1852.0 / 3600.0),
    ("ft/s", "speed", 0.3048),
];

/// Temperature unit -> (to kelvin, from kelvin).
fn temp(u: &str) -> Option<(fn(f64) -> f64, fn(f64) -> f64)> {
    Some(match u {
        "C" | "degC" => (|x| x + 273.15, |k| k - 273.15),
        "F" | "degF" => (|x| (x - 32.0) * 5.0 / 9.0 + 273.15, |k| (k - 273.15) * 9.0 / 5.0 + 32.0),
        "K" => (|x| x, |k| k),
        "R" | "degR" => (|x| x * 5.0 / 9.0, |k| k * 9.0 / 5.0),
        _ => return None,
    })
}

fn unitconv(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("unitconv", a, 3, 3)?;
    let x = a[0].matrix()?;
    let (from, to) = (a[1].as_str()?, a[2].as_str()?);
    if let (Some((to_k, _)), Some((_, from_k))) = (temp(from), temp(to)) {
        return num(x.map(|v| from_k(to_k(v))));
    }
    let find = |u: &str| {
        UNITS.iter().find(|e| e.0 == u).or_else(|| UNITS.iter().find(|e| e.0.eq_ignore_ascii_case(u))).ok_or_else(|| {
            let known: Vec<&str> = UNITS.iter().map(|e| e.0).chain(["C", "F", "K", "R"]).collect();
            format!("unitconv: unknown unit '{u}' (known: {})", known.join(" "))
        })
    };
    let (f, t) = (find(from)?, find(to)?);
    if f.1 != t.1 {
        return Err(format!("unitconv: cannot convert {} '{from}' to {} '{to}'", f.1, t.1));
    }
    num(x.map(|v| v * f.2 / t.2))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::CMatrix;

    fn run(name: &str, args: &[Value], nargout: usize) -> Vec<Value> {
        lookup(name).unwrap()(&mut Interp::new(), args, nargout).unwrap()
    }

    fn m(name: &str, args: &[Value]) -> Matrix {
        run(name, args, 1)[0].matrix().unwrap()
    }

    #[test]
    fn magic_squares() {
        assert_eq!(magic(3), Matrix::from_rows(&[vec![8., 1., 6.], vec![3., 5., 7.], vec![4., 9., 2.]]));
        assert_eq!(magic(4), Matrix::from_rows(&[vec![16., 2., 3., 13.], vec![5., 11., 10., 8.], vec![9., 7., 6., 12.], vec![4., 14., 15., 1.]]));
        assert_eq!(magic(6).get(0, 0), 35.0);
        for n in 3..=12 {
            let a = magic(n);
            let s = (n * (n * n + 1) / 2) as f64;
            for i in 0..n {
                assert_eq!((0..n).map(|j| a.get(i, j)).sum::<f64>(), s, "row {i} of magic({n})");
                assert_eq!((0..n).map(|j| a.get(j, i)).sum::<f64>(), s, "col {i} of magic({n})");
            }
            assert_eq!((0..n).map(|i| a.get(i, i)).sum::<f64>(), s);
            let mut v = a.data.clone();
            v.sort_by(f64::total_cmp);
            assert_eq!(v, (1..=n * n).map(|x| x as f64).collect::<Vec<_>>());
        }
    }

    #[test]
    fn elementwise() {
        assert_eq!(m("mod", &[Value::from(vec![-7., 7., 5.]), Value::num(3.)]).data, vec![2., 1., 2.]);
        assert_eq!(m("rem", &[Value::from(vec![-7., 7.]), Value::num(3.)]).data, vec![-1., 1.]);
        assert_eq!(m("mod", &[Value::num(5.), Value::num(0.)]).data, vec![5.]);
        assert_eq!(m("sind", &[Value::from(vec![0., 90., 180., 360.])]).data, vec![0., 1., 0., 0.]);
        assert_eq!(m("cosd", &[Value::from(vec![0., 90., 270.])]).data, vec![1., 0., 0.]);
        assert_eq!(m("nthroot", &[Value::num(-27.), Value::num(3.)]).data, vec![-3.]);
        assert_eq!(m("factorial", &[Value::from(vec![0., 5.])]).data, vec![1., 120.]);
        assert!((m("gamma", &[Value::num(0.5)]).data[0] - PI.sqrt()).abs() < 1e-14);
        for &y in &[-0.999, -0.3, 0.0, 0.2, 0.7, 0.999999] {
            assert!((erf(erfinv(y)) - y).abs() < 1e-14, "erfinv({y})");
        }
        assert_eq!(m("gcd", &[Value::from(vec![12., 18.]), Value::num(8.)]).data, vec![4., 2.]);
        assert_eq!(m("lcm", &[Value::num(4.), Value::num(6.)]).data, vec![12.]);
        assert_eq!(m("primes", &[Value::num(20.)]).data, vec![2., 3., 5., 7., 11., 13., 17., 19.]);
        assert_eq!(m("isprime", &[Value::from(vec![1., 2., 9., 97.])]).data, vec![0., 1., 0., 1.]);
        assert_eq!(m("factor", &[Value::num(360.)]).data, vec![2., 2., 2., 3., 3., 5.]);
        let pk = run("factor", &[Value::num(360.)], 2);
        assert_eq!(pk[1].matrix().unwrap().data, vec![3., 2., 1.]);
        assert_eq!(m("nchoosek", &[Value::num(5.), Value::num(2.)]).data, vec![10.]);
        assert_eq!(m("nchoosek", &[Value::from(vec![1., 2., 3.]), Value::num(2.)]), Matrix::from_rows(&[vec![1., 2.], vec![1., 3.], vec![2., 3.]]));
    }

    #[test]
    fn shapes() {
        let a: Value = Matrix::from_rows(&[vec![1., 2., 3.], vec![4., 5., 6.]]).into();
        assert_eq!(m("size", &[a.clone()]).data, vec![2., 3.]);
        let rc = run("size", &[a.clone()], 2);
        assert_eq!((rc[0].scalar().unwrap(), rc[1].scalar().unwrap()), (2., 3.));
        assert_eq!(m("size", &[a.clone(), Value::num(2.)]).data, vec![3.]);
        assert_eq!(m("numel", &[a.clone()]).data, vec![6.]);
        assert_eq!(m("reshape", &[a.clone(), Value::num(3.), Value::empty()]), Matrix::from_rows(&[vec![1., 5.], vec![4., 3.], vec![2., 6.]]));
        assert_eq!(m("fliplr", &[a.clone()]), Matrix::from_rows(&[vec![3., 2., 1.], vec![6., 5., 4.]]));
        assert_eq!(m("rot90", &[a.clone()]), Matrix::from_rows(&[vec![3., 6.], vec![2., 5.], vec![1., 4.]]));
        assert_eq!(m("circshift", &[Value::from(vec![1., 2., 3.]), Value::num(1.)]).data, vec![3., 1., 2.]);
        assert_eq!(m("triu", &[a.clone()]), Matrix::from_rows(&[vec![1., 2., 3.], vec![0., 5., 6.]]));
        assert_eq!(m("repmat", &[Value::from(vec![1., 2.]), Value::num(2.), Value::num(2.)]), Matrix::from_rows(&[vec![1., 2., 1., 2.], vec![1., 2., 1., 2.]]));
        assert_eq!(m("kron", &[Value::from(vec![1., 2.]), Matrix::col(vec![1., 1.]).into()]), Matrix::from_rows(&[vec![1., 2.], vec![1., 2.]]));
        assert_eq!(m("diag", &[Value::from(vec![1., 2.])]), Matrix::from_rows(&[vec![1., 0.], vec![0., 2.]]));
        assert_eq!(m("diag", &[a.clone(), Value::num(1.)]).data, vec![2., 6.]);
        assert_eq!(m("vertcat", &[Value::from(vec![1., 2.]), Value::from(vec![3., 4.])]), Matrix::from_rows(&[vec![1., 2.], vec![3., 4.]]));
        assert!(matches!(&run("fliplr", &["abc".into()], 1)[0], Value::Str(s) if s == "cba"));
        assert!(matches!(&run("horzcat", &["ab".into(), Value::num(67.)], 1)[0], Value::Str(s) if s == "abC"));
        let g = run("meshgrid", &[Value::from(vec![1., 2., 3.]), Value::from(vec![4., 5.])], 2);
        assert_eq!(g[0].matrix().unwrap(), Matrix::from_rows(&[vec![1., 2., 3.], vec![1., 2., 3.]]));
        assert_eq!(g[1].matrix().unwrap(), Matrix::from_rows(&[vec![4., 4., 4.], vec![5., 5., 5.]]));
        let g = run("ndgrid", &[Value::from(vec![1., 2.]), Value::from(vec![3., 4., 5.])], 2);
        assert_eq!(g[0].matrix().unwrap(), Matrix::from_rows(&[vec![1., 1., 1.], vec![2., 2., 2.]]));
        assert_eq!(m("linspace", &[Value::num(0.), Value::num(1.), Value::num(5.)]).data, vec![0., 0.25, 0.5, 0.75, 1.]);
        assert_eq!(m("colon", &[Value::num(0.), Value::num(0.1), Value::num(0.3)]).len(), 4);
        assert_eq!(m("colon", &[Value::num(0.), Value::num(0.1), Value::num(0.3)]).data[3], 0.3);
        assert_eq!(m("colon", &[Value::num(5.), Value::num(1.)]).len(), 0);
        assert!(run("isequal", &[a.clone(), a.clone()], 1)[0].truthy().unwrap());
        assert_eq!(m("eye", &[Value::num(2.), Value::num(3.)]).data, vec![1., 0., 0., 1., 0., 0.]);
    }

    fn c(re: f64, im: f64) -> C64 {
        C64::new(re, im)
    }

    fn z(name: &str, args: &[Value]) -> Vec<C64> {
        match &run(name, args, 1)[0] {
            Value::Complex(m) => m.data.clone(),
            v => panic!("{name}: expected complex, got {v:?}"),
        }
    }

    fn near(a: &[C64], b: &[C64]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).norm() < 1e-12)
    }

    #[test]
    fn complex_promotion() {
        let x = |v: f64| Value::num(v);
        assert_eq!(z("sqrt", &[x(-4.)]), vec![c(0., 2.)]);
        assert_eq!(z("sqrt", &[Value::from(vec![-4., 9.])]), vec![c(0., 2.), c(3., 0.)]);
        assert!(matches!(run("sqrt", &[Value::from(vec![4., 9.])], 1)[0], Value::Num(_)));
        assert_eq!(z("log", &[x(-1.)]), vec![c(0., PI)]);
        assert_eq!(z("log10", &[x(-10.)]), vec![c(1., PI / LN_10)]);
        assert_eq!(z("log2", &[x(-8.)]), vec![c(3., PI / LN_2)]);
        // Octave / MATLAB branches for real arguments off the real domain
        let k = 2f64.acosh();
        assert!(near(&z("asin", &[x(2.)]), &[c(PI / 2., -k)]));
        assert!(near(&z("asin", &[x(-2.)]), &[c(-PI / 2., k)]));
        assert!(near(&z("acos", &[x(2.)]), &[c(0., k)]));
        assert!(near(&z("acos", &[x(-2.)]), &[c(PI, -k)]));
        assert!(near(&z("acosh", &[x(0.5)]), &[c(0., PI / 3.)]));
        assert!(near(&z("atanh", &[x(2.)]), &[c(3f64.ln() / 2., PI / 2.)]));
        assert!(near(&z("atanh", &[x(-2.)]), &[c(-3f64.ln() / 2., PI / 2.)]));
        assert_eq!(m("asin", &[x(1.)]).data, vec![PI / 2.]);
        assert!(near(&z("power", &[x(-8.), x(1. / 3.)]), &[c(1., 3f64.sqrt())]));
        assert_eq!(m("power", &[Value::from(vec![-8., 4.]), Value::from(vec![2., 0.5])]).data, vec![64., 2.]);
        assert_eq!(z("power", &[Value::complex(Mat::scalar(c(1., 2.))), x(2.)]), vec![c(-3., 4.)]);
        assert_eq!(m("realsqrt", &[x(4.)]).data, vec![2.]);
        for (f, a) in [("realsqrt", vec![x(-1.)]), ("reallog", vec![x(-1.)]), ("realpow", vec![x(-8.), x(1. / 3.)])] {
            assert_eq!(lookup(f).unwrap()(&mut Interp::new(), &a, 1).unwrap_err(), format!("{f}: produced complex result"));
        }
        assert_eq!(lookup("gamma").unwrap()(&mut Interp::new(), &[Value::complex(Mat::scalar(c(1., 1.)))], 1).unwrap_err(), "gamma: complex input not supported");
    }

    #[test]
    fn complex_elementwise_and_shapes() {
        let w = Value::complex(Mat::row(vec![c(3., 4.), c(0., -1.), c(-1.5, 2.5)]));
        assert_eq!(m("abs", &[w.clone()]).data, vec![5., 1., 2.5f64.hypot(1.5)]);
        assert_eq!(m("angle", &[w.clone()]).data[1], -PI / 2.);
        assert_eq!(m("arg", &[Value::num(-1.)]).data, vec![PI]);
        assert_eq!(m("real", &[w.clone()]).data, vec![3., 0., -1.5]);
        assert_eq!(m("imag", &[w.clone()]).data, vec![4., -1., 2.5]);
        assert_eq!(z("conj", &[w.clone()]), vec![c(3., -4.), c(0., 1.), c(-1.5, -2.5)]);
        assert_eq!(z("sign", &[w.clone()])[0], c(0.6, 0.8));
        assert_eq!(z("round", &[w.clone()])[2], c(-2., 3.));
        assert_eq!(z("fix", &[w.clone()])[2], c(-1., 2.));
        assert!(near(&z("exp", &[Value::complex(Mat::scalar(c(0., PI)))]), &[c(-1., 0.)]));
        assert!(near(&z("sin", &[Value::complex(Mat::scalar(c(0., 1.)))]), &[c(0., 1f64.sinh())]));
        let bad = Value::complex(Mat::row(vec![c(f64::NAN, 1.), c(1., f64::INFINITY), c(1., 1.)]));
        assert_eq!(m("isnan", &[bad.clone()]).data, vec![1., 0., 0.]);
        assert_eq!(m("isinf", &[bad.clone()]).data, vec![0., 1., 0.]);
        assert_eq!(m("isfinite", &[bad]).data, vec![0., 0., 1.]);
        // shape functions keep the imaginary parts
        let a = Value::complex(CMatrix::from_parts(&Matrix::from_rows(&[vec![1., 2.], vec![3., 4.]]), &Matrix::from_rows(&[vec![1., 0.], vec![0., 1.]])));
        assert_eq!(m("size", &[a.clone()]).data, vec![2., 2.]);
        assert_eq!(m("numel", &[a.clone()]).data, vec![4.]);
        assert_eq!(z("fliplr", &[a.clone()]), vec![c(2., 0.), c(4., 1.), c(1., 1.), c(3., 0.)]);
        assert_eq!(z("reshape", &[a.clone(), Value::num(1.), Value::num(4.)]), vec![c(1., 1.), c(3., 0.), c(2., 0.), c(4., 1.)]);
        assert_eq!(z("diag", &[a.clone()]), vec![c(1., 1.), c(4., 1.)]);
        assert_eq!(z("triu", &[a.clone()]), vec![c(1., 1.), c(0., 0.), c(2., 0.), c(4., 1.)]);
        assert_eq!(m("tril", &[a.clone(), Value::num(-1.)]).data, vec![0., 3., 0., 0.]);
        assert_eq!(z("repmat", &[w.clone(), Value::num(1.), Value::num(2.)]).len(), 6);
        assert_eq!(z("horzcat", &[w.clone(), Value::num(7.)])[3], c(7., 0.));
        assert_eq!(z("vertcat", &[Value::complex(Mat::scalar(c(0., 1.))), Value::num(2.)]), vec![c(0., 1.), c(2., 0.)]);
        assert_eq!(z("kron", &[Value::from(vec![1., 2.]), Value::complex(Mat::scalar(c(0., 1.)))]), vec![c(0., 1.), c(0., 2.)]);
        assert!(run("isequal", &[a.clone(), a.clone()], 1)[0].truthy().unwrap());
        assert!(!run("isequal", &[a.clone(), Value::Num(Matrix::from_rows(&[vec![1., 2.], vec![3., 4.]]))], 1)[0].truthy().unwrap());
    }

    #[test]
    fn random_numbers() {
        run("rng", &[Value::num(7.)], 0);
        let a = m("rand", &[Value::num(3.)]);
        run("rand", &["seed".into(), Value::num(7.)], 0);
        assert_eq!(m("rand", &[Value::num(3.)]), a);
        assert!(a.data.iter().all(|&x| x > 0.0 && x < 1.0));
        let r = m("randi", &[Value::from(vec![5., 7.]), Value::num(1.), Value::num(200.)]);
        assert!(r.data.iter().all(|&x| (5.0..=7.0).contains(&x) && x.fract() == 0.0));
        assert!([5., 6., 7.].iter().all(|v| r.data.contains(v)));
        let n = m("randn", &[Value::num(1.), Value::num(20000.)]);
        let mean = n.data.iter().sum::<f64>() / n.len() as f64;
        assert!(mean.abs() < 0.05);
    }

    #[test]
    fn physics() {
        assert_eq!(m("physconst", &["lightspeed".into()]).data, vec![299_792_458.0]);
        assert!(physconst(&mut Interp::new(), &["nope".into()], 1).unwrap_err().contains("Boltzmann"));
        let c = |v: f64, f: &str, t: &str| m("unitconv", &[Value::num(v), f.into(), t.into()]).data[0];
        assert!((c(1.0, "mi", "km") - 1.609344).abs() < 1e-12);
        assert!((c(100.0, "C", "F") - 212.0).abs() < 1e-9);
        assert!((c(-40.0, "F", "C") + 40.0).abs() < 1e-9);
        assert!((c(0.0, "C", "K") - 273.15).abs() < 1e-9);
        assert!((c(180.0, "deg", "rad") - PI).abs() < 1e-12);
        assert!((c(1.0, "atm", "psi") - 14.695_948_8).abs() < 1e-6);
        assert!(unitconv(&mut Interp::new(), &[Value::num(1.), "m".into(), "s".into()], 1).is_err());
        assert_eq!(m("mag2db", &[Value::num(10.)]).data, vec![20.]);
        assert_eq!(m("db2mag", &[Value::num(40.)]).data, vec![100.]);
    }
}
