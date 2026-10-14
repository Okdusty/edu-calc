//! Calculus, polynomials, interpolation, filtering, quadrature, root finding, optimization,
//! ODE solvers and arrayfun / cellfun. Algorithms are plain Rust over closures; the builtins
//! adapt `Interp::call` into those closures.
use std::collections::BTreeMap;

use super::linalg::{eigvals, solve};
use super::{Elem, R, dim_of, first, fun1, is_opt, lanes, nargs, num, on, real_arg};
use crate::lang::Interp;
use crate::value::{Builtin, C64, CMatrix, Mat, Matrix, Value};

pub(super) type Fx<'a> = dyn FnMut(f64) -> Result<f64, String> + 'a;

pub fn lookup(name: &str) -> Option<Builtin> {
    let f: Builtin = match name {
        "diff" => diff,
        "gradient" => gradient,
        "polyfit" => polyfit,
        "polyval" => polyval,
        "polyder" => polyder,
        "polyint" => |_, a, _| {
            nargs("polyint", a, 1, 2)?;
            let p = a[0].matrix()?.data;
            let k = a.get(1).map(|v| v.scalar()).transpose()?.unwrap_or(0.0);
            let n = p.len();
            let mut out: Vec<f64> = p.iter().enumerate().map(|(i, &c)| c / (n - i) as f64).collect();
            out.push(k);
            num(Matrix::row(out))
        },
        "roots" => roots,
        "poly" => poly,
        "conv" => conv,
        "deconv" => deconv,
        "filter" => filter,
        "interp1" => interp1,
        "integral" | "quad" | "quadgk" | "quadl" => integral,
        "fzero" => |interp, a, _| {
            nargs("fzero", a, 2, 3)?;
            let x0 = real_arg("fzero", &a[1])?.data;
            let (x, fx) = fzero(&mut fun1(interp, &a[0]), &x0, opt(a.get(2), "TolX", f64::EPSILON))?;
            Ok(vec![Value::num(x), Value::num(fx), Value::num(1.0)])
        },
        "fminbnd" => |interp, a, _| {
            nargs("fminbnd", a, 3, 4)?;
            let tol = opt(a.get(3), "TolX", 1e-8);
            let (x, fx) = fminbnd(&mut fun1(interp, &a[0]), a[1].scalar()?, a[2].scalar()?, tol)?;
            Ok(vec![Value::num(x), Value::num(fx), Value::num(1.0)])
        },
        "fminsearch" => fminsearch,
        // ponytail: ode15s / ode23s are explicit ode45 (fine when mildly stiff), implicit solver if stiff problems show up
        "ode45" | "ode15s" | "ode23s" => |interp, a, n| ode_b("ode45", &DORMAND_PRINCE, 4, interp, a, n),
        "ode23" => |interp, a, n| ode_b("ode23", &BOGACKI_SHAMPINE, 1, interp, a, n),
        "arrayfun" => |interp, a, n| mapfun("arrayfun", false, interp, a, n),
        "cellfun" => |interp, a, n| mapfun("cellfun", true, interp, a, n),
        _ => return None,
    };
    Some(f)
}

/// Numeric field of an optimset/odeset-style options struct.
pub(super) fn opt(o: Option<&Value>, key: &str, def: f64) -> f64 {
    match o {
        Some(Value::Struct(s)) => s.get(key).and_then(|v| v.scalar().ok()).unwrap_or(def),
        _ => def,
    }
}

fn diff(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("diff", a, 1, 3)?;
    let k = match a.get(1) {
        Some(v) if v.size() != (0, 0) => v.as_usize()?,
        _ => 1,
    };
    fn of<T: Elem>(m: Mat<T>, k: usize, d: Option<&Value>) -> R {
        let d = dim_of("diff", &m, d)?;
        Ok(vec![T::value(lanes(&m, d, |l| {
            let mut v = l.to_vec();
            for _ in 0..k.min(v.len()) {
                v = v.windows(2).map(|w| w[1] - w[0]).collect();
            }
            v
        }))])
    }
    on(&a[0], |m| of(m, k, a.get(2)), |z| of(z, k, a.get(2)))
}

/// One-sided differences at the ends, central inside; x are the sample coordinates.
pub(super) fn grad<T: Elem>(y: &[T], x: &[f64]) -> Vec<T> {
    let n = y.len();
    if n < 2 {
        return vec![T::default(); n];
    }
    (0..n).map(|i| {
        let (p, q) = (i.saturating_sub(1), (i + 1).min(n - 1));
        (y[q] - y[p]).unscale(x[q] - x[p])
    }).collect()
}

/// Coordinates for gradient: default 1-spacing, scalar spacing h, or explicit vector.
pub(super) fn coords(v: Option<&Value>, n: usize) -> Result<Vec<f64>, String> {
    let s = match v {
        Some(v) => v.matrix()?.data,
        None => vec![1.0],
    };
    match s.len() {
        1 => Ok((0..n).map(|k| k as f64 * s[0]).collect()),
        l if l == n => Ok(s),
        _ => Err("gradient: dimensions of spacing and data must match".into()),
    }
}

fn gradient(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("gradient", a, 1, 3)?;
    fn of<T: Elem>(m: Mat<T>, a: &[Value]) -> R {
        if m.rows == 1 || m.cols == 1 {
            return Ok(vec![T::value(Mat::new(m.rows, m.cols, grad(&m.data, &coords(a.get(1), m.len())?)))]);
        }
        let hx = coords(a.get(1), m.cols)?;
        let hy = coords(a.get(2).or(a.get(1).filter(|v| v.size() == (1, 1))), m.rows)?;
        Ok(vec![T::value(lanes(&m, 2, |l| grad(l, &hx))), T::value(lanes(&m, 1, |l| grad(l, &hy)))])
    }
    on(&a[0], |m| of(m, a), |z| of(z, a))
}

fn horner<T: Elem>(p: &[T], x: T) -> T {
    p.iter().fold(T::default(), |acc, &c| acc * x + c)
}

fn polyfit(_: &mut Interp, a: &[Value], nargout: usize) -> R {
    nargs("polyfit", a, 3, 3)?;
    let (mut x, y) = (real_arg("polyfit", &a[0])?.data, real_arg("polyfit", &a[1])?.data);
    let n = a[2].as_usize().ok().filter(|&n| n < 1000).ok_or("polyfit: N must be a non-negative integer below 1000")?;
    if x.len() != y.len() {
        return Err("polyfit: X and Y must be vectors of the same size".into());
    }
    let k = x.len();
    let mu = (nargout >= 3).then(|| {
        let m = x.iter().sum::<f64>() / k as f64;
        let s = (x.iter().map(|v| (v - m).powi(2)).sum::<f64>() / (k as f64 - 1.0)).sqrt();
        x.iter_mut().for_each(|v| *v = (*v - m) / s);
        (m, s)
    });
    let v = Matrix::new(k, n + 1, (0..k * (n + 1)).map(|q| x[q % k].powi((n - q / k) as i32)).collect());
    let p = solve(&v, &Matrix::col(y.clone()))?.data;
    let mut out = vec![Value::from(p.clone())];
    if nargout >= 2 {
        let yf: Vec<f64> = x.iter().map(|&t| horner(&p, t)).collect();
        let normr = y.iter().zip(&yf).map(|(a, b)| (a - b).powi(2)).sum::<f64>().sqrt();
        let s = BTreeMap::from([
            ("normr".to_string(), Value::num(normr)),
            ("df".to_string(), Value::num(k as f64 - (n + 1) as f64)),
            ("yf".to_string(), Value::from(yf)),
        ]);
        out.push(Value::Struct(s));
    }
    if let Some((m, s)) = mu {
        out.push(Value::Num(Matrix::col(vec![m, s])));
    }
    Ok(out)
}

fn polyval(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("polyval", a, 2, 4)?;
    let (m, s) = match a.get(3) {
        Some(mu) => match mu.matrix()?.data[..] {
            [m, s] => (m, s),
            _ => return Err("polyval: MU must be a 2-element vector".into()),
        },
        None => (0.0, 1.0),
    };
    if a[..2].iter().any(Value::is_complex) {
        let p = a[0].cmatrix()?.data;
        return Ok(vec![Value::complex(a[1].cmatrix()?.map(|t| horner(&p, (t - m) / s)))]);
    }
    let p = a[0].matrix()?.data;
    num(a[1].matrix()?.map(|t| horner(&p, (t - m) / s)))
}

/// Drop leading zeros, keep at least one coefficient.
fn strip(v: Vec<f64>) -> Vec<f64> {
    let k = v.iter().position(|&c| c != 0.0).unwrap_or(v.len().saturating_sub(1));
    v[k..].to_vec()
}

fn pder(p: &[f64]) -> Vec<f64> {
    let n = p.len();
    if n <= 1 {
        return vec![0.0];
    }
    strip(p[..n - 1].iter().enumerate().map(|(i, &c)| c * (n - 1 - i) as f64).collect())
}

/// a + s*b with right-aligned coefficients.
fn padd(a: &[f64], b: &[f64], s: f64) -> Vec<f64> {
    let n = a.len().max(b.len());
    let at = |v: &[f64], i: usize| if i + v.len() >= n { v[i + v.len() - n] } else { 0.0 };
    (0..n).map(|i| at(a, i) + s * at(b, i)).collect()
}

fn convolve<T: Elem>(a: &[T], b: &[T]) -> Vec<T> {
    if a.is_empty() || b.is_empty() {
        return vec![];
    }
    let mut out = vec![T::default(); a.len() + b.len() - 1];
    for (i, &x) in a.iter().enumerate() {
        for (j, &y) in b.iter().enumerate() {
            out[i + j] += x * y;
        }
    }
    out
}

/// polyder(p); polyder(a, b) = (a*b)'; [q, d] = polyder(b, a) = (b/a)'.
fn polyder(_: &mut Interp, a: &[Value], nargout: usize) -> R {
    nargs("polyder", a, 1, 2)?;
    let p = a[0].matrix()?.data;
    let Some(b) = a.get(1) else { return Ok(vec![Value::from(pder(&p))]) };
    let b = b.matrix()?.data;
    if nargout < 2 {
        return Ok(vec![Value::from(strip(padd(&convolve(&pder(&p), &b), &convolve(&p, &pder(&b)), 1.0)))]);
    }
    let q = padd(&convolve(&pder(&p), &b), &convolve(&p, &pder(&b)), -1.0);
    Ok(vec![Value::from(strip(q)), Value::from(strip(convolve(&b, &b)))])
}

fn roots(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("roots", a, 1, 1)?;
    let p = a[0].cmatrix()?.data;
    if p.iter().any(|x| !x.is_finite()) {
        return Err("roots: inputs must not contain Inf or NaN".into());
    }
    let zero = C64::default();
    let (Some(s), Some(e)) = (p.iter().position(|&c| c != zero), p.iter().rposition(|&c| c != zero)) else {
        return num(Matrix::empty());
    };
    let core = &p[s..=e];
    let n = core.len() - 1;
    // companion matrix: first row -p(2:end)/p(1), ones on the subdiagonal; a real leading
    // coefficient divides part-wise so real polynomials keep an exactly real companion
    let div = |z: C64| if core[0].im == 0.0 { z.unscale(core[0].re) } else { z.fdiv(core[0]) };
    let mut comp = CMatrix::zeros(n, n);
    for j in 0..n {
        comp.set(0, j, -div(core[j + 1]));
        if j + 1 < n {
            comp.set(j + 1, j, C64::new(1.0, 0.0));
        }
    }
    let mut r = eigvals(&comp)?;
    r.extend(std::iter::repeat_n(zero, p.len() - 1 - e));
    r.sort_by(|x, y| y.re.total_cmp(&x.re).then(y.im.total_cmp(&x.im)));
    Ok(vec![Value::complex(Mat::col(r))])
}

/// poly(r): coefficients of the polynomial with roots r; poly(A): characteristic polynomial.
fn poly(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("poly", a, 1, 1)?;
    let x = a[0].cmatrix()?;
    let r = match (x.rows, x.cols) {
        (0, _) | (_, 0) => return num(Matrix::scalar(1.0)),
        (1, _) | (_, 1) => x.data,
        (m, n) if m == n => eigvals(&x)?,
        _ => return Err("poly: input must be a vector or a square matrix".into()),
    };
    let c = r.iter().fold(vec![C64::new(1.0, 0.0)], |c, &z| convolve(&c, &[C64::new(1.0, 0.0), -z]));
    // like Octave's poly.m: roots in exact conjugate pairs give real coefficients
    let by_parts = |x: &C64, y: &C64| x.re.total_cmp(&y.re).then(x.im.total_cmp(&y.im));
    let mut up: Vec<C64> = r.iter().filter(|z| z.im > 0.0).copied().collect();
    let mut down: Vec<C64> = r.iter().filter(|z| z.im < 0.0).map(|z| z.conj()).collect();
    up.sort_by(by_parts);
    down.sort_by(by_parts);
    let c = Mat::row(c);
    Ok(vec![if up == down { Value::Num(c.re()) } else { Value::complex(c) }])
}

fn conv(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("conv", a, 2, 3)?;
    if a[..2].iter().any(Value::is_complex) {
        return conv_of(a[0].cmatrix()?, a[1].cmatrix()?, a.get(2));
    }
    conv_of(a[0].matrix()?, a[1].matrix()?, a.get(2))
}

fn conv_of<T: Elem>(x: Mat<T>, y: Mat<T>, shape: Option<&Value>) -> R {
    if !(x.rows == 1 || x.cols == 1) || !(y.rows == 1 || y.cols == 1) {
        return Err("conv: both arguments A and B must be vectors".into());
    }
    let (la, lb) = (x.len(), y.len());
    if la == 0 || lb == 0 {
        return num(Matrix::empty());
    }
    let full = convolve(&x.data, &y.data);
    let v = match shape {
        None => full,
        Some(s) if is_opt(s, "full") => full,
        Some(s) if is_opt(s, "same") => {
            let i = lb / 2; // ceil((lb - 1) / 2)
            full[i..i + la].to_vec()
        }
        Some(s) if is_opt(s, "valid") => if la >= lb { full[lb - 1..la].to_vec() } else { vec![] },
        Some(_) => return Err("conv: SHAPE argument must be \"full\", \"same\", or \"valid\"".into()),
    };
    // Octave's conv.m: orientation follows the longer vector (b on ties)
    let col = if la > lb { x.cols == 1 && x.rows > 1 } else { y.cols == 1 && y.rows > 1 };
    Ok(vec![T::value(if col { Mat::col(v) } else { Mat::row(v) })])
}

fn deconv(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("deconv", a, 2, 2)?;
    if a.iter().any(Value::is_complex) {
        return deconv_of(a[0].cmatrix()?, a[1].cmatrix()?.data);
    }
    deconv_of(a[0].matrix()?, a[1].matrix()?.data)
}

fn deconv_of<T: Elem>(ym: Mat<T>, d: Vec<T>) -> R {
    if d.first().is_none_or(|&c| c == T::default()) {
        return Err("deconv: divisor cannot be zero".into());
    }
    if ym.len() < d.len() {
        return Ok(vec![Value::num(0.0), T::value(ym)]);
    }
    let mut r = ym.data.clone();
    let n = r.len() - d.len() + 1;
    let mut b = vec![T::default(); n];
    for i in 0..n {
        b[i] = r[i] / d[0];
        for (j, &c) in d.iter().enumerate() {
            r[i + j] -= b[i] * c;
        }
        r[i] = T::default();
    }
    Ok(vec![T::value(Mat::row(b)), T::value(Mat::new(ym.rows, ym.cols, r))])
}

/// filter(b, a, x[, zi[, dim]]): direct form II transposed, normalized by a(1).
fn filter(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("filter", a, 3, 5)?;
    // ponytail: real filters and signals only; complex would need the same loop over C64
    let (mut b, mut den) = (real_arg("filter", &a[0])?.data, real_arg("filter", &a[1])?.data);
    let x = real_arg("filter", &a[2])?;
    let a0 = *den.first().ok_or("filter: A must be a non-empty vector")?;
    if a0 == 0.0 {
        return Err("filter: a(1) must be nonzero".into());
    }
    b.iter_mut().for_each(|c| *c /= a0);
    den.iter_mut().for_each(|c| *c /= a0);
    let n = b.len().max(den.len());
    b.resize(n, 0.0);
    den.resize(n, 0.0);
    let zi = match a.get(3) {
        Some(v) if v.size() != (0, 0) => v.matrix()?.data,
        _ => vec![],
    };
    let d = dim_of("filter", &x, a.get(4))?;
    num(lanes(&x, d, |l| {
        let mut z = zi.clone();
        z.resize(n, 0.0);
        l.iter().map(|&xi| {
            let yi = b[0] * xi + z[0];
            for k in 1..n {
                z[k - 1] = b[k] * xi + z[k] - den[k] * yi;
            }
            yi
        }).collect()
    }))
}

/// Natural cubic spline second derivatives (Thomas algorithm).
fn spline_m2(x: &[f64], y: &[f64]) -> Vec<f64> {
    let n = x.len();
    let mut m = vec![0.0; n];
    let (mut c, mut d) = (vec![0.0; n], vec![0.0; n]);
    for i in 1..n.saturating_sub(1) {
        let (h0, h1) = (x[i] - x[i - 1], x[i + 1] - x[i]);
        let r = (y[i + 1] - y[i]) / h1 - (y[i] - y[i - 1]) / h0;
        let den = (h0 + h1) / 3.0 - h0 / 6.0 * c[i - 1];
        c[i] = h1 / 6.0 / den;
        d[i] = (r - h0 / 6.0 * d[i - 1]) / den;
    }
    for i in (1..n.saturating_sub(1)).rev() {
        m[i] = d[i] - c[i] * m[i + 1];
    }
    m
}

/// Interpolate sorted (xs, ys) at t. `extrap` None extrapolates, Some(v) is the outside value.
fn interp_at(xs: &[f64], ys: &[f64], m2: &[f64], method: &str, t: f64, extrap: Option<f64>) -> f64 {
    let n = xs.len();
    if t.is_nan() {
        return f64::NAN;
    }
    if let Some(v) = extrap
        && (t < xs[0] || t > xs[n - 1])
    {
        return v;
    }
    let i = xs.partition_point(|&v| v <= t).clamp(1, n - 1) - 1;
    let (x0, x1, y0, y1) = (xs[i], xs[i + 1], ys[i], ys[i + 1]);
    let (h, s) = (x1 - x0, (t - x0) / (x1 - x0));
    match method {
        "nearest" => if t < (x0 + x1) / 2.0 { y0 } else { y1 },
        "spline" => {
            let a = 1.0 - s;
            a * y0 + s * y1 + ((a.powi(3) - a) * m2[i] + (s.powi(3) - s) * m2[i + 1]) * h * h / 6.0
        }
        _ => y0 + s * (y1 - y0),
    }
}

/// interp1(x, y, xi[, method][, 'extrap' | value]) or interp1(y, xi, ...).
fn interp1(_: &mut Interp, a: &[Value], _: usize) -> R {
    let nnum = a.iter().take_while(|v| !matches!(v, Value::Str(_))).count();
    let (x, y, xi) = match nnum {
        2 => {
            let y = real_arg("interp1", &a[0])?;
            let n = if y.rows == 1 { y.cols } else { y.rows };
            ((1..=n).map(|k| k as f64).collect::<Vec<_>>(), y, real_arg("interp1", &a[1])?)
        }
        3 | 4 => (real_arg("interp1", &a[0])?.data, real_arg("interp1", &a[1])?, real_arg("interp1", &a[2])?),
        _ => return Err("Invalid call to interp1".into()),
    };
    let (mut method, mut extrap) = ("linear".to_string(), Some(f64::NAN));
    for v in &a[nnum.min(3)..] {
        match v {
            Value::Str(s) if s.eq_ignore_ascii_case("extrap") => extrap = None,
            Value::Str(s) => method = s.trim_start_matches('*').to_lowercase(),
            v => extrap = Some(v.scalar()?),
        }
    }
    if !["linear", "nearest", "spline"].contains(&method.as_str()) {
        return Err(format!("interp1: invalid METHOD '{method}'")); // ponytail: no pchip/previous/next yet
    }
    let cols: Vec<Vec<f64>> = if y.rows <= 1 || y.cols <= 1 {
        vec![y.data.clone()]
    } else {
        (0..y.cols).map(|j| (0..y.rows).map(|i| y.get(i, j)).collect()).collect()
    };
    let n = x.len();
    if cols[0].len() != n {
        return Err("interp1: X and Y must have the same length".into());
    }
    if n < 2 {
        return Err("interp1: minimum of 2 points required".into());
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&i, &j| x[i].total_cmp(&x[j]));
    let xs: Vec<f64> = order.iter().map(|&i| x[i]).collect();
    if xs.windows(2).any(|w| w[0] == w[1]) {
        return Err("interp1: X must be strictly monotonic".into());
    }
    let mut out = vec![];
    for c in &cols {
        let ys: Vec<f64> = order.iter().map(|&i| c[i]).collect();
        let m2 = if method == "spline" { spline_m2(&xs, &ys) } else { vec![] };
        out.extend(xi.data.iter().map(|&t| interp_at(&xs, &ys, &m2, &method, t, extrap)));
    }
    num(if cols.len() == 1 { Matrix::new(xi.rows, xi.cols, out) } else { Matrix::new(xi.len(), cols.len(), out) })
}

const XGK: [f64; 8] = [
    0.991_455_371_120_812_6, 0.949_107_912_342_758_5, 0.864_864_423_359_769_1, 0.741_531_185_599_394_4,
    0.586_087_235_467_691_1, 0.405_845_151_377_397_2, 0.207_784_955_007_898_5, 0.0,
];
const WGK: [f64; 8] = [
    0.022_935_322_010_529_22, 0.063_092_092_629_978_55, 0.104_790_010_322_250_18, 0.140_653_259_715_525_92,
    0.169_004_726_639_267_9, 0.190_350_578_064_785_4, 0.204_432_940_075_298_9, 0.209_482_141_084_727_83,
];
const WG: [f64; 4] = [0.129_484_966_168_869_7, 0.279_705_391_489_276_7, 0.381_830_050_505_118_9, 0.417_959_183_673_469_4];

/// 15-point Gauss–Kronrod on [a, b]: (integral, error estimate).
fn gk15(f: &mut Fx, a: f64, b: f64) -> Result<(f64, f64), String> {
    let (c, h) = ((a + b) / 2.0, (b - a) / 2.0);
    let fc = f(c)?;
    let (mut k, mut g) = (fc * WGK[7], fc * WG[3]);
    for j in 0..7 {
        let s = f(c - h * XGK[j])? + f(c + h * XGK[j])?;
        k += WGK[j] * s;
        if j % 2 == 1 {
            g += WG[j / 2] * s;
        }
    }
    Ok((k * h, ((k - g) * h).abs()))
}

/// Globally adaptive Gauss–Kronrod (QUADPACK QAG style). Infinite limits are mapped onto
/// finite ones; finite ones get Octave quadgk's t(3 - t^2) map to soften endpoint singularities.
pub(super) fn quad(f: &mut Fx, a: f64, b: f64, abstol: f64, reltol: f64) -> Result<f64, String> {
    if a == b {
        return Ok(0.0);
    }
    if a > b {
        return Ok(-quad(f, b, a, abstol, reltol)?);
    }
    let (lo, hi, mut g): (f64, f64, Box<Fx>) = match (a.is_finite(), b.is_finite()) {
        (true, true) => {
            let (m, r) = ((a + b) / 2.0, (b - a) / 4.0);
            (-1.0, 1.0, Box::new(move |t: f64| Ok(f(m + r * t * (3.0 - t * t))? * 3.0 * r * (1.0 - t * t))))
        }
        (true, false) => (0.0, 1.0, Box::new(move |t: f64| Ok(f(a + t / (1.0 - t))? / (1.0 - t).powi(2)))),
        (false, true) => (0.0, 1.0, Box::new(move |t: f64| Ok(f(b - t / (1.0 - t))? / (1.0 - t).powi(2)))),
        (false, false) => (-1.0, 1.0, Box::new(move |t: f64| Ok(f(t / (1.0 - t * t))? * (1.0 + t * t) / (1.0 - t * t).powi(2)))),
    };
    let (i0, e0) = gk15(&mut g, lo, hi)?;
    let mut parts = vec![(lo, hi, i0, e0)];
    loop {
        let total: f64 = parts.iter().map(|p| p.2).sum();
        let err: f64 = parts.iter().map(|p| p.3).sum();
        // ponytail: Octave warns when MaxIntervalCount (650) is hit; we return the estimate
        if err <= abstol.max(reltol * total.abs()) || parts.len() >= 650 || !total.is_finite() {
            return Ok(total);
        }
        let k = (0..parts.len()).max_by(|&i, &j| parts[i].3.total_cmp(&parts[j].3)).unwrap_or(0);
        let (a, b, _, _) = parts.swap_remove(k);
        let m = (a + b) / 2.0;
        let (l, el) = gk15(&mut g, a, m)?;
        let (r, er) = gk15(&mut g, m, b)?;
        parts.push((a, m, l, el));
        parts.push((m, b, r, er));
    }
}

fn integral(interp: &mut Interp, a: &[Value], _: usize) -> R {
    if a.len() < 3 {
        return Err("Invalid call to integral".into());
    }
    let (lo, hi) = (a[1].scalar()?, a[2].scalar()?);
    let (abstol, reltol) = quad_tols(&a[3..])?;
    num(Matrix::scalar(quad(&mut fun1(interp, &a[0]), lo, hi, abstol, reltol)?))
}

/// (AbsTol, RelTol) from integral-style trailing args: 'AbsTol', x / 'RelTol', x pairs or quad's
/// TOL / [ABSTOL RELTOL]. Other options and function args (dblquad's QUADF) are ignored.
pub(super) fn quad_tols(a: &[Value]) -> Result<(f64, f64), String> {
    let (mut abstol, mut reltol) = (1e-10, 1e-6);
    let mut i = 0;
    while i < a.len() {
        match &a[i] {
            Value::Str(k) if i + 1 < a.len() => {
                match k.to_lowercase().as_str() {
                    "abstol" => abstol = a[i + 1].scalar()?,
                    "reltol" => reltol = a[i + 1].scalar()?,
                    _ => {} // ArrayValued, Waypoints, MaxIntervalCount: ignored
                }
                i += 2;
            }
            // quad(f, a, b, tol): tol or [abstol reltol]
            Value::Num(t) => {
                if let Some(&x) = t.data.first() {
                    abstol = x;
                }
                if let Some(&x) = t.data.get(1) {
                    reltol = x;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    Ok((abstol, reltol))
}

/// Root of f from a bracket [a b] or a starting point (bracket searched outward), via Brent.
pub(super) fn fzero(f: &mut Fx, x0: &[f64], tolx: f64) -> Result<(f64, f64), String> {
    let (a, b, fa, fb) = match *x0 {
        [a, b] => {
            let (fa, fb) = (f(a)?, f(b)?);
            if fa != 0.0 && fb != 0.0 && fa.signum() == fb.signum() {
                return Err("fzero: not a valid initial bracketing".into());
            }
            (a, b, fa, fb)
        }
        [x] => {
            let fx = f(x)?;
            let mut found = None;
            'search: for k in 0..80 {
                let d = 0.01 * x.abs().max(1.0) * 1.5f64.powi(k);
                for t in [x - d, x + d] {
                    let ft = f(t)?;
                    if ft.is_finite() && (ft == 0.0 || fx == 0.0 || ft.signum() != fx.signum()) {
                        found = Some((x, t, fx, ft));
                        break 'search;
                    }
                }
            }
            found.ok_or("fzero: unable to find a valid initial bracketing")?
        }
        _ => return Err("fzero: X0 must be a scalar or a 2-element vector".into()),
    };
    let x = brent(f, a, b, fa, fb, tolx)?;
    Ok((x, f(x)?))
}

/// Brent's zeroin (Numerical Recipes zbrent layout).
fn brent(f: &mut Fx, mut a: f64, mut b: f64, mut fa: f64, mut fb: f64, tolx: f64) -> Result<f64, String> {
    if fa == 0.0 {
        return Ok(a);
    }
    let (mut c, mut fc) = (b, fb);
    let (mut d, mut e) = (b - a, b - a);
    for _ in 0..500 {
        if fb != 0.0 && fb.signum() == fc.signum() {
            (c, fc) = (a, fa);
            d = b - a;
            e = d;
        }
        if fc.abs() < fb.abs() {
            (a, b, c) = (b, c, b);
            (fa, fb, fc) = (fb, fc, fb);
        }
        let tol1 = 2.0 * f64::EPSILON * b.abs() + 0.5 * tolx;
        let xm = 0.5 * (c - b);
        if xm.abs() <= tol1 || fb == 0.0 {
            return Ok(b);
        }
        if e.abs() >= tol1 && fa.abs() > fb.abs() {
            let s = fb / fa;
            let (mut p, mut q) = if a == c {
                (2.0 * xm * s, 1.0 - s)
            } else {
                let (q, r) = (fa / fc, fb / fc);
                (s * (2.0 * xm * q * (q - r) - (b - a) * (r - 1.0)), (q - 1.0) * (r - 1.0) * (s - 1.0))
            };
            if p > 0.0 {
                q = -q;
            }
            p = p.abs();
            if 2.0 * p < (3.0 * xm * q - (tol1 * q).abs()).min((e * q).abs()) {
                e = d;
                d = p / q;
            } else {
                d = xm;
                e = d;
            }
        } else {
            d = xm;
            e = d;
        }
        (a, fa) = (b, fb);
        b += if d.abs() > tol1 { d } else { tol1.copysign(xm) };
        fb = f(b)?;
    }
    Ok(b)
}

/// Minimum of f on [a, b]: Brent's golden section + parabolic interpolation (FMM fmin).
pub(super) fn fminbnd(f: &mut Fx, a: f64, b: f64, tolx: f64) -> Result<(f64, f64), String> {
    let g = 0.5 * (3.0 - 5f64.sqrt());
    let (mut a, mut b) = (a.min(b), a.max(b));
    let mut x = a + g * (b - a);
    let (mut v, mut w) = (x, x);
    let (mut d, mut e) = (0.0f64, 0.0f64);
    let mut fx = f(x)?;
    let (mut fv, mut fw) = (fx, fx);
    for _ in 0..500 {
        let xm = 0.5 * (a + b);
        let tol1 = f64::EPSILON.sqrt() * x.abs() + tolx / 3.0;
        let tol2 = 2.0 * tol1;
        if (x - xm).abs() <= tol2 - 0.5 * (b - a) {
            break;
        }
        let mut golden = true;
        if e.abs() > tol1 {
            let r = (x - w) * (fx - fv);
            let mut q = (x - v) * (fx - fw);
            let mut p = (x - v) * q - (x - w) * r;
            q = 2.0 * (q - r);
            if q > 0.0 {
                p = -p;
            } else {
                q = -q;
            }
            let r = e;
            e = d;
            if p.abs() < (0.5 * q * r).abs() && p > q * (a - x) && p < q * (b - x) {
                d = p / q;
                let u = x + d;
                if u - a < tol2 || b - u < tol2 {
                    d = tol1.copysign(xm - x);
                }
                golden = false;
            }
        }
        if golden {
            e = if x >= xm { a - x } else { b - x };
            d = g * e;
        }
        let u = if d.abs() >= tol1 { x + d } else { x + tol1.copysign(d) };
        let fu = f(u)?;
        if fu <= fx {
            if u >= x { a = x } else { b = x }
            (v, fv, w, fw, x, fx) = (w, fw, x, fx, u, fu);
        } else {
            if u < x { a = u } else { b = u }
            if fu <= fw || w == x {
                (v, fv, w, fw) = (w, fw, u, fu);
            } else if fu <= fv || v == x || v == w {
                (v, fv) = (u, fu);
            }
        }
    }
    Ok((x, fx))
}

/// Nelder–Mead simplex (standard coefficients, MATLAB-style initial simplex).
pub(super) fn nelder_mead(
    f: &mut dyn FnMut(&[f64]) -> Result<f64, String>,
    x0: &[f64],
    tolx: f64,
    tolf: f64,
    maxit: usize,
) -> Result<(Vec<f64>, f64), String> {
    let n = x0.len();
    let mut s = vec![(x0.to_vec(), f(x0)?)];
    for i in 0..n {
        let mut x = x0.to_vec();
        x[i] = if x[i] != 0.0 { 1.05 * x[i] } else { 0.00025 };
        let fx = f(&x)?;
        s.push((x, fx));
    }
    for _ in 0..maxit {
        s.sort_by(|p, q| p.1.total_cmp(&q.1));
        let spread_f = s.iter().all(|p| (p.1 - s[0].1).abs() <= tolf);
        let spread_x = s.iter().all(|p| p.0.iter().zip(&s[0].0).all(|(a, b)| (a - b).abs() <= tolx));
        if spread_f && spread_x {
            break;
        }
        let cen: Vec<f64> = (0..n).map(|j| s[..n].iter().map(|p| p.0[j]).sum::<f64>() / n as f64).collect();
        let worst = s[n].clone();
        // point on the line centroid -> worst: 1 reflect, 2 expand, 0.5 / -0.5 contract
        let along = |t: f64| -> Vec<f64> { cen.iter().zip(&worst.0).map(|(c, w)| c + t * (c - w)).collect() };
        let xr = along(1.0);
        let fr = f(&xr)?;
        if fr < s[0].1 {
            let xe = along(2.0);
            let fe = f(&xe)?;
            s[n] = if fe < fr { (xe, fe) } else { (xr, fr) };
        } else if fr < s[n - 1].1 {
            s[n] = (xr, fr);
        } else {
            let outside = fr < worst.1;
            let xc = along(if outside { 0.5 } else { -0.5 });
            let fc = f(&xc)?;
            if (outside && fc <= fr) || (!outside && fc < worst.1) {
                s[n] = (xc, fc);
            } else {
                let best = s[0].0.clone();
                for p in s.iter_mut().skip(1) {
                    p.0 = best.iter().zip(&p.0).map(|(b, x)| b + 0.5 * (x - b)).collect();
                    p.1 = f(&p.0)?;
                }
            }
        }
    }
    s.sort_by(|p, q| p.1.total_cmp(&q.1));
    let (x, fx) = s.swap_remove(0);
    Ok((x, fx))
}

fn fminsearch(interp: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("fminsearch", a, 2, 3)?;
    let x0 = real_arg("fminsearch", &a[1])?;
    let (r, c) = (x0.rows, x0.cols);
    let o = a.get(2);
    let n = x0.len();
    let (tolx, tolf, maxit) = (opt(o, "TolX", 1e-4), opt(o, "TolFun", 1e-4), opt(o, "MaxIter", 200.0 * n as f64) as usize);
    let f = &a[0];
    let mut g = |x: &[f64]| first(interp.call(f, &[Value::Num(Matrix::new(r, c, x.to_vec()))], 1)?)?.scalar();
    let (x, fx) = nelder_mead(&mut g, &x0.data, tolx, tolf, maxit)?;
    Ok(vec![Value::Num(Matrix::new(r, c, x)), Value::num(fx), Value::num(1.0)])
}

/// Embedded Runge–Kutta pair; the last stage is evaluated at (t + h, y_new) (FSAL).
pub(super) struct Tableau {
    c: &'static [f64],
    a: &'static [&'static [f64]],
    b: &'static [f64],
    /// b - b_hat, the local error weights
    e: &'static [f64],
    /// exponent for step control: lower order + 1
    order: f64,
}

pub(super) const DORMAND_PRINCE: Tableau = Tableau {
    c: &[0.0, 0.2, 0.3, 0.8, 8.0 / 9.0, 1.0, 1.0],
    a: &[
        &[],
        &[0.2],
        &[3.0 / 40.0, 9.0 / 40.0],
        &[44.0 / 45.0, -56.0 / 15.0, 32.0 / 9.0],
        &[19372.0 / 6561.0, -25360.0 / 2187.0, 64448.0 / 6561.0, -212.0 / 729.0],
        &[9017.0 / 3168.0, -355.0 / 33.0, 46732.0 / 5247.0, 49.0 / 176.0, -5103.0 / 18656.0],
        &[35.0 / 384.0, 0.0, 500.0 / 1113.0, 125.0 / 192.0, -2187.0 / 6784.0, 11.0 / 84.0],
    ],
    b: &[35.0 / 384.0, 0.0, 500.0 / 1113.0, 125.0 / 192.0, -2187.0 / 6784.0, 11.0 / 84.0, 0.0],
    e: &[71.0 / 57600.0, 0.0, -71.0 / 16695.0, 71.0 / 1920.0, -17253.0 / 339200.0, 22.0 / 525.0, -1.0 / 40.0],
    order: 5.0,
};

pub(super) const BOGACKI_SHAMPINE: Tableau = Tableau {
    c: &[0.0, 0.5, 0.75, 1.0],
    a: &[&[], &[0.5], &[0.0, 0.75], &[2.0 / 9.0, 1.0 / 3.0, 4.0 / 9.0]],
    b: &[2.0 / 9.0, 1.0 / 3.0, 4.0 / 9.0, 0.0],
    e: &[-5.0 / 72.0, 1.0 / 12.0, 1.0 / 9.0, -1.0 / 8.0],
    order: 3.0,
};

/// Cubic Hermite between (y0, f0) and (y1, f1) over a step h, at fraction s.
// ponytail: Hermite dense output instead of the solver's own continuous extension
fn hermite(y0: &[f64], y1: &[f64], f0: &[f64], f1: &[f64], h: f64, s: f64) -> Vec<f64> {
    let (s2, s3) = (s * s, s * s * s);
    let (h00, h10, h01, h11) = (2.0 * s3 - 3.0 * s2 + 1.0, s3 - 2.0 * s2 + s, 3.0 * s2 - 2.0 * s3, s3 - s2);
    (0..y0.len()).map(|j| h00 * y0[j] + h10 * h * f0[j] + h01 * y1[j] + h11 * h * f1[j]).collect()
}

type Rhs<'a> = dyn FnMut(f64, &[f64]) -> Result<Vec<f64>, String> + 'a;

/// Adaptive embedded RK. tspan = [t0 tf] returns every step (`refine` points each),
/// longer tspan returns exactly those times. Rows of the result are time points.
pub(super) fn ode(f: &mut Rhs, tspan: &[f64], y0: &[f64], tab: &Tableau, rtol: f64, atol: f64, refine: usize) -> Result<(Vec<f64>, Vec<Vec<f64>>), String> {
    let (Some(&t0), Some(&tf)) = (tspan.first(), tspan.last()) else {
        return Err("ode: TSPAN must contain at least 2 distinct finite times".into());
    };
    if tspan.len() < 2 || t0 == tf || !(t0.is_finite() && tf.is_finite()) {
        return Err("ode: TSPAN must contain at least 2 distinct finite times".into());
    }
    let n = y0.len();
    let mut rhs = |t: f64, y: &[f64]| -> Result<Vec<f64>, String> {
        let v = f(t, y)?;
        if v.len() != n { Err(format!("ode: the function must return a vector with {n} elements")) } else { Ok(v) }
    };
    let dir = (tf - t0).signum();
    let hmax = (tf - t0).abs() / 10.0;
    let (mut t, mut y) = (t0, y0.to_vec());
    let mut k1 = rhs(t, &y)?;
    let rms = |v: &[f64]| (v.iter().zip(&y).map(|(a, b)| (a / (atol + rtol * b.abs())).powi(2)).sum::<f64>() / n.max(1) as f64).sqrt();
    let (d0, d1) = (rms(&y), rms(&k1));
    let mut h = (if d0 < 1e-5 || d1 < 1e-5 { 1e-6 } else { 0.01 * d0 / d1 }).min(hmax);
    let (mut ts, mut ys) = (vec![t0], vec![y0.to_vec()]);
    let mut next_out = 1;
    let s = tab.b.len();
    for _ in 0..1_000_000 {
        if (tf - t) * dir <= 0.0 {
            return Ok((ts, ys));
        }
        let last = h >= (tf - t).abs();
        let hs = if last { tf - t } else { h * dir };
        let mut k = vec![k1.clone()];
        for i in 1..s {
            let yi: Vec<f64> = (0..n).map(|j| y[j] + hs * tab.a[i].iter().zip(&k).map(|(al, kl)| al * kl[j]).sum::<f64>()).collect();
            k.push(rhs(t + tab.c[i] * hs, &yi)?);
        }
        let ynew: Vec<f64> = (0..n).map(|j| y[j] + hs * (0..s).map(|l| tab.b[l] * k[l][j]).sum::<f64>()).collect();
        let err = (0..n)
            .map(|j| (hs * (0..s).map(|l| tab.e[l] * k[l][j]).sum::<f64>()).abs() / (atol + rtol * y[j].abs().max(ynew[j].abs())))
            .fold(0.0, f64::max);
        if err <= 1.0 {
            let tnew = if last { tf } else { t + hs };
            let knew = k.pop().unwrap_or_default();
            if tspan.len() > 2 {
                while next_out < tspan.len() && (tspan[next_out] - tnew) * dir <= 0.0 {
                    ys.push(hermite(&y, &ynew, &k1, &knew, hs, (tspan[next_out] - t) / hs));
                    ts.push(tspan[next_out]);
                    next_out += 1;
                }
            } else {
                for r in 1..refine {
                    let th = r as f64 / refine as f64;
                    ts.push(t + th * hs);
                    ys.push(hermite(&y, &ynew, &k1, &knew, hs, th));
                }
                ts.push(tnew);
                ys.push(ynew.clone());
            }
            (t, y, k1) = (tnew, ynew, knew);
        }
        let fac = if err == 0.0 { 5.0 } else if err.is_nan() { 0.2 } else { (0.9 * err.powf(-1.0 / tab.order)).clamp(0.2, 5.0) };
        h = (hs.abs() * if err <= 1.0 { fac } else { fac.min(1.0) }).min(hmax);
        if h <= 16.0 * f64::EPSILON * t.abs().max(f64::MIN_POSITIVE) {
            return Err(format!("ode: step size became too small at t = {t}"));
        }
    }
    Err("ode: too many steps".into())
}

fn ode_b(name: &str, tab: &Tableau, refine: usize, interp: &mut Interp, a: &[Value], nargout: usize) -> R {
    nargs(name, a, 3, 4)?;
    let tspan = real_arg(name, &a[1])?.data;
    let y0 = real_arg(name, &a[2])?.data;
    let o = a.get(3);
    let (rtol, atol) = (opt(o, "RelTol", 1e-3), opt(o, "AbsTol", 1e-6));
    let refine = opt(o, "Refine", refine as f64).max(1.0) as usize;
    let f = &a[0];
    let mut g = |t: f64, y: &[f64]| Ok(first(interp.call(f, &[Value::num(t), Value::Num(Matrix::col(y.to_vec()))], 1)?)?.matrix()?.data);
    let (t, y) = ode(&mut g, &tspan, &y0, tab, rtol, atol, refine).map_err(|e| e.replacen("ode", name, 1))?;
    let (nt, n) = (t.len(), y0.len());
    let ym = Matrix::new(nt, n, (0..nt * n).map(|k| y[k % nt][k / nt]).collect());
    if nargout == 1 {
        let s = BTreeMap::from([
            ("x".to_string(), Value::from(t)),
            ("y".to_string(), Value::Num(ym.transpose())),
            ("solver".to_string(), Value::from(name)),
        ]);
        return Ok(vec![Value::Struct(s)]);
    }
    Ok(vec![Value::Num(Matrix::col(t)), Value::Num(ym)])
}

/// arrayfun / cellfun with 'UniformOutput' and 'ErrorHandler'.
fn mapfun(name: &str, cells: bool, interp: &mut Interp, a: &[Value], nargout: usize) -> R {
    if a.len() < 2 {
        return Err(format!("Invalid call to {name}"));
    }
    let (mut uniform, mut handler, mut inputs) = (true, None, vec![]);
    let mut i = 1;
    while i < a.len() {
        if i > 1 && i + 1 < a.len() && is_opt(&a[i], "UniformOutput") {
            uniform = a[i + 1].truthy()?;
            i += 2;
        } else if i > 1 && i + 1 < a.len() && is_opt(&a[i], "ErrorHandler") {
            handler = Some(&a[i + 1]);
            i += 2;
        } else {
            inputs.push(&a[i]);
            i += 1;
        }
    }
    let shape = inputs[0].size();
    if inputs.iter().any(|v| v.size() != shape) {
        return Err(format!("{name}: all the input arguments must have the same size and shape"));
    }
    if cells && inputs.iter().any(|v| !matches!(v, Value::Cell(_))) {
        return Err("cellfun: C must be a cell array".into());
    }
    let elem = |v: &Value, k: usize| -> Result<Value, String> {
        Ok(match v {
            Value::Cell(c) if cells => c[k].clone(),
            Value::Cell(c) => Value::Cell(vec![c[k].clone()]),
            Value::Bool(m) => Value::bool(m.data[k] != 0.0),
            Value::Str(s) => Value::Str(s.chars().nth(k).map(String::from).unwrap_or_default()),
            Value::Num(m) => Value::num(m.data[k]),
            Value::Complex(m) => Value::complex(Mat::scalar(m.data[k])),
            v => v.clone(),
        })
    };
    let n = shape.0 * shape.1;
    let nout = nargout.max(1);
    let mut outs: Vec<Vec<Value>> = vec![vec![]; nout];
    for k in 0..n {
        let args: Vec<Value> = inputs.iter().map(|v| elem(v, k)).collect::<Result<_, _>>()?;
        let r = match (interp.call(&a[0], &args, nargout), handler) {
            (Ok(r), _) => r,
            (Err(e), Some(h)) => {
                let s = BTreeMap::from([("message".to_string(), Value::from(e.as_str())), ("index".to_string(), Value::num(k as f64 + 1.0))]);
                let mut hargs = vec![Value::Struct(s)];
                hargs.extend(args);
                interp.call(h, &hargs, nargout)?
            }
            (Err(e), None) => return Err(e),
        };
        if r.is_empty() && nargout == 0 {
            continue; // function with no outputs, e.g. printf
        }
        if r.len() < nout {
            return Err(format!("{name}: function returned fewer than nargout values"));
        }
        outs.iter_mut().zip(r).for_each(|(o, v)| o.push(v));
    }
    if n > 0 && outs[0].is_empty() {
        return Ok(vec![]);
    }
    outs.into_iter()
        .map(|o| {
            if !uniform {
                return Ok(Value::Cell(o));
            }
            if o.len() != n {
                return Err(format!("{name}: function returned no value for some elements"));
            }
            let mut data = Vec::with_capacity(n);
            for v in &o {
                match v {
                    Value::Num(m) | Value::Bool(m) if m.len() == 1 => data.push(C64::new(m.data[0], 0.0)),
                    Value::Complex(m) if m.len() == 1 => data.push(m.data[0]),
                    _ => return Err(format!("{name}: all values must be scalars when UniformOutput = true; use the 'UniformOutput', false options")),
                }
            }
            let m = CMatrix::new(shape.0, shape.1, data);
            Ok(if n > 0 && o.iter().all(|v| matches!(v, Value::Bool(_))) { Value::Bool(m.re()) } else { Value::complex(m) })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{E, PI};

    fn run(name: &str, args: &[Value], nargout: usize) -> Vec<Value> {
        lookup(name).unwrap()(&mut Interp::new(), args, nargout).unwrap()
    }

    fn m(name: &str, args: &[Value]) -> Matrix {
        run(name, args, 1)[0].matrix().unwrap()
    }

    fn close(a: &[f64], b: &[f64], tol: f64) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
    }

    #[test]
    fn polynomials() {
        let x = Value::from(vec![0., 1., 2., 3., 4.]);
        let y = Value::from(vec![1., 3., 9., 19., 33.]); // 2x^2 + 1
        let p = m("polyfit", &[x.clone(), y.clone(), Value::num(2.)]);
        assert!(close(&p.data, &[2., 0., 1.], 1e-10));
        assert_eq!(m("polyval", &[p.into(), Value::num(5.)]).data.iter().map(|v| v.round()).collect::<Vec<_>>(), vec![51.]);
        let r = m("roots", &[Value::from(vec![1., -6., 11., -6.])]);
        assert!(close(&r.data, &[3., 2., 1.], 1e-10) && r.cols == 1);
        assert!(close(&m("roots", &[Value::from(vec![1., -2., 1., 0.])]).data, &[1., 1., 0.], 1e-7));
        assert_eq!(m("conv", &[Value::from(vec![1., 1.]), Value::from(vec![1., -1.])]).data, vec![1., 0., -1.]);
        assert_eq!(m("conv", &[Value::from(vec![1., 2., 3.]), Value::from(vec![1., 1.]), "same".into()]).data, vec![3., 5., 3.]);
        let qr = run("deconv", &[Value::from(vec![1., 0., -1.]), Value::from(vec![1., 1.])], 2);
        assert_eq!(qr[0].matrix().unwrap().data, vec![1., -1.]);
        assert_eq!(qr[1].matrix().unwrap().data, vec![0., 0., 0.]);
        assert_eq!(m("polyder", &[Value::from(vec![1., 2., 3.])]).data, vec![2., 2.]);
        assert_eq!(m("polyder", &[Value::from(vec![1., 0.]), Value::from(vec![1., 0.])]).data, vec![2., 0.]);
        assert_eq!(m("polyint", &[Value::from(vec![3., 2., 1.])]).data, vec![1., 1., 1., 0.]);
        assert_eq!(m("filter", &[Value::num(1.), Value::from(vec![1., -0.5]), Value::from(vec![1., 0., 0.])]).data, vec![1., 0.5, 0.25]);
        assert_eq!(m("filter", &[Value::from(vec![1., 1.]), Value::num(1.), Value::from(vec![1., 2., 3.])]).data, vec![1., 3., 5.]);
    }

    fn c(re: f64, im: f64) -> C64 {
        C64::new(re, im)
    }

    fn cv(v: &[C64]) -> Value {
        Value::complex(Mat::row(v.to_vec()))
    }

    fn cclose(a: &[C64], b: &[C64], tol: f64) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).norm() <= tol)
    }

    #[test]
    fn complex_polynomials() {
        let r = run("roots", &[Value::from(vec![1., 0., 1.])], 1).remove(0).cmatrix().unwrap();
        assert!(cclose(&r.data, &[c(0., 1.), c(0., -1.)], 1e-15) && r.cols == 1);
        // distinct real roots stay a real value
        assert!(matches!(run("roots", &[Value::from(vec![1., -6., 11., -6.])], 1)[0], Value::Num(_)));
        // roots of a complex polynomial: (x - i)(x - 2) = x^2 - (2 + i)x + 2i
        let r = run("roots", &[cv(&[c(1., 0.), c(-2., -1.), c(0., 2.)])], 1).remove(0).cmatrix().unwrap();
        assert!(cclose(&r.data, &[c(2., 0.), c(0., 1.)], 1e-12));
        // poly undoes roots; conjugate pairs give real coefficients
        for p in [vec![1., -6., 11., -6.], vec![1., 2., 5.], vec![2., 0., 0., -16.]] {
            let r = run("roots", &[Value::from(p.clone())], 1).remove(0);
            let q = run("poly", &[r], 1).remove(0);
            let lead = p[0];
            assert!(matches!(&q, Value::Num(q) if close(&q.data, &p.iter().map(|x| x / lead).collect::<Vec<_>>(), 1e-12)), "{p:?} -> {q:?}");
        }
        assert!(close(&m("poly", &[Matrix::from_rows(&[vec![1., 2.], vec![3., 4.]]).into()]).data, &[1., -5., -2.], 1e-12));
        assert_eq!(m("poly", &[Value::empty()]).data, vec![1.]);
        let q = run("poly", &[cv(&[c(0., 1.)])], 1).remove(0).cmatrix().unwrap();
        assert_eq!(q.data, vec![c(1., 0.), c(0., -1.)]);
        // polyval / conv / deconv
        assert_eq!(run("polyval", &[Value::from(vec![1., 0., 1.]), cv(&[c(0., 1.), c(0., 2.)])], 1)[0].matrix().unwrap().data, vec![0., -3.]);
        assert!(cclose(&run("polyval", &[cv(&[c(0., 1.), c(1., 0.)]), Value::num(2.)], 1)[0].cmatrix().unwrap().data, &[c(1., 2.)], 0.0));
        let p = run("conv", &[cv(&[c(1., 0.), c(0., 1.)]), cv(&[c(1., 0.), c(0., -1.)])], 1).remove(0);
        assert!(matches!(&p, Value::Num(p) if p.data == vec![1., 0., 1.]));
        let qr = run("deconv", &[p, cv(&[c(1., 0.), c(0., 1.)])], 2);
        assert!(cclose(&qr[0].cmatrix().unwrap().data, &[c(1., 0.), c(0., -1.)], 1e-15));
        assert!(qr[1].cmatrix().unwrap().data.iter().all(|z| z.norm() < 1e-15));
    }

    #[test]
    fn complex_callbacks_and_rejections() {
        let r = run("arrayfun", &["sqrt".into(), Value::from(vec![-4., 4.])], 1).remove(0).cmatrix().unwrap();
        assert_eq!(r.data, vec![c(0., 2.), c(2., 0.)]);
        let r = run("arrayfun", &["abs".into(), cv(&[c(3., 4.), c(0., -1.)])], 1).remove(0);
        assert!(matches!(&r, Value::Num(m) if m.data == vec![5., 1.]));
        let z = cv(&[c(1., 1.), c(2., 0.)]);
        let err = |name: &str, args: &[Value]| lookup(name).unwrap()(&mut Interp::new(), args, 1).unwrap_err();
        assert_eq!(err("polyfit", &[z.clone(), z.clone(), Value::num(1.)]), "polyfit: complex input not supported");
        assert_eq!(err("interp1", &[Value::from(vec![1., 2.]), z.clone(), Value::num(1.5)]), "interp1: complex input not supported");
        assert_eq!(err("fzero", &["sin".into(), z.clone()]), "fzero: complex input not supported");
        assert_eq!(err("filter", &[Value::num(1.), Value::num(1.), z]), "filter: complex input not supported");
    }

    #[test]
    fn calculus() {
        assert_eq!(m("diff", &[Value::from(vec![1., 4., 9., 16.])]).data, vec![3., 5., 7.]);
        assert_eq!(m("diff", &[Value::from(vec![1., 4., 9., 16.]), Value::num(2.)]).data, vec![2., 2.]);
        assert_eq!(m("gradient", &[Value::from(vec![1., 4., 9., 16.])]).data, vec![3., 4., 6., 7.]);
        assert_eq!(m("gradient", &[Value::from(vec![1., 4., 9.]), Value::num(0.5)]).data, vec![6., 8., 10.]);
        let g = run("gradient", &[Matrix::from_rows(&[vec![1., 2., 4.], vec![2., 4., 8.]]).into()], 2);
        assert_eq!(g[0].matrix().unwrap(), Matrix::from_rows(&[vec![1., 1.5, 2.], vec![2., 3., 4.]]));
        assert_eq!(g[1].matrix().unwrap(), Matrix::from_rows(&[vec![1., 2., 4.], vec![1., 2., 4.]]));
    }

    #[test]
    fn interpolation() {
        let (x, y) = (Value::from(vec![0., 1., 2.]), Value::from(vec![0., 1., 0.]));
        let xi = Value::from(vec![0.5, 1.5, 3.0]);
        let lin = m("interp1", &[x.clone(), y.clone(), xi.clone()]);
        assert!(lin.data[..2] == [0.5, 0.5] && lin.data[2].is_nan());
        assert_eq!(m("interp1", &[x.clone(), y.clone(), xi.clone(), "linear".into(), "extrap".into()]).data, vec![0.5, 0.5, -1.0]);
        assert_eq!(m("interp1", &[x.clone(), y.clone(), Value::from(vec![0.4, 0.5, 1.6]), "nearest".into()]).data, vec![0., 1., 0.]);
        let sp = m("interp1", &[x.clone(), y.clone(), Value::num(0.5), "spline".into()]);
        assert!((sp.data[0] - 0.6875).abs() < 1e-12);
        assert_eq!(m("interp1", &[x.clone(), y.clone(), Value::num(5.), "linear".into(), Value::num(0.)]).data, vec![0.]);
        // unsorted x is fine; matrix y interpolates each column
        let ym: Value = Matrix::from_rows(&[vec![0., 10.], vec![2., 20.]]).into();
        assert_eq!(m("interp1", &[Value::from(vec![1., 0.]), ym, Value::num(0.5)]).data, vec![1., 15.]);
    }

    #[test]
    fn quadrature_roots_minima() {
        let i = quad(&mut |x| Ok(x.sin()), 0.0, PI, 1e-10, 1e-6).unwrap();
        assert!((i - 2.0).abs() < 1e-10);
        let i = quad(&mut |x| Ok((-x * x).exp()), f64::NEG_INFINITY, f64::INFINITY, 1e-10, 1e-8).unwrap();
        assert!((i - PI.sqrt()).abs() < 1e-8);
        let i = quad(&mut |x| Ok(1.0 / x.sqrt()), 0.0, 1.0, 1e-10, 1e-8).unwrap();
        assert!((i - 2.0).abs() < 1e-6);
        assert!((quad(&mut |x| Ok(1.0 / (x * x)), 1.0, f64::INFINITY, 1e-10, 1e-8).unwrap() - 1.0).abs() < 1e-8);
        let (x, _) = fzero(&mut |x| Ok(x.cos()), &[1.0], f64::EPSILON).unwrap();
        assert!((x - PI / 2.0).abs() < 1e-12);
        let (x, _) = fzero(&mut |x| Ok(x * x - 2.0), &[0.0, 2.0], f64::EPSILON).unwrap();
        assert!((x - 2f64.sqrt()).abs() < 1e-14);
        assert!(fzero(&mut |x| Ok(x * x + 1.0), &[0.0, 2.0], f64::EPSILON).is_err());
        let (x, fx) = fminbnd(&mut |x| Ok((x - 2.0).powi(2) + 1.0), 0.0, 5.0, 1e-8).unwrap();
        assert!((x - 2.0).abs() < 1e-6 && (fx - 1.0).abs() < 1e-12);
        let (x, fx) = nelder_mead(&mut |v| Ok((v[0] - 1.0).powi(2) + 10.0 * (v[1] + 2.0).powi(2)), &[0.0, 0.0], 1e-8, 1e-10, 2000).unwrap();
        assert!(close(&x, &[1.0, -2.0], 1e-4) && fx < 1e-8);
        let (x, _) = nelder_mead(&mut |v| Ok((1.0 - v[0]).powi(2) + 100.0 * (v[1] - v[0] * v[0]).powi(2)), &[-1.2, 1.0], 1e-8, 1e-10, 5000).unwrap();
        assert!(close(&x, &[1.0, 1.0], 1e-4));
    }

    #[test]
    fn ode_solvers() {
        let (t, y) = ode(&mut |_, y| Ok(vec![-y[0]]), &[0.0, 1.0], &[1.0], &DORMAND_PRINCE, 1e-3, 1e-6, 4).unwrap();
        assert_eq!(*t.last().unwrap(), 1.0);
        assert!((y.last().unwrap()[0] - (-1f64).exp()).abs() < 1e-4);
        assert!(t.windows(2).all(|w| w[1] > w[0]));
        let (t, y) = ode(&mut |_, y| Ok(vec![-y[0]]), &[0.0, 0.5, 1.0], &[1.0], &BOGACKI_SHAMPINE, 1e-6, 1e-9, 1).unwrap();
        assert_eq!(t, vec![0.0, 0.5, 1.0]);
        assert!((y[2][0] - 1.0 / E).abs() < 1e-5);
        // harmonic oscillator, two components, backwards in time
        let (t, y) = ode(&mut |_, y| Ok(vec![y[1], -y[0]]), &[0.0, -PI], &[0.0, 1.0], &DORMAND_PRINCE, 1e-8, 1e-10, 1).unwrap();
        assert_eq!(*t.last().unwrap(), -PI);
        assert!(close(y.last().unwrap(), &[0.0, -1.0], 1e-6));
    }
}
