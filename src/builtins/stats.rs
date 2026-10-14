//! Reductions, searching and sorting, descriptive statistics, distributions, trapz.
use std::cmp::Ordering;
use std::f64::consts::{PI, SQRT_2};

use super::math::{erfc, erfinv, lgamma};
use super::{Elem, R, bcast, boolean, chars, dim_of, is_opt, lanes, lanes2, nargs, num, on, real_arg, red};
use crate::lang::Interp;
use crate::value::{Builtin, C64, Mat, Matrix, Value};

pub fn lookup(name: &str) -> Option<Builtin> {
    let f: Builtin = match name {
        "sum" => |_, a, _| creduce("sum", a, sum, sum),
        "prod" => |_, a, _| creduce("prod", a, prod, prod),
        "mean" => |_, a, _| creduce("mean", a, mean, mean),
        "median" => |_, a, _| reduce("median", a, median),
        "range" => |_, a, _| reduce("range", a, |l| fold(l, f64::max) - fold(l, f64::min)),
        "any" => |_, a, _| {
            nargs("any", a, 1, 2)?;
            let (m, d) = red("any", nonzero(&a[0])?, a.get(1))?;
            boolean(lanes(&m, d, |l| vec![l.iter().any(|&x| x != 0.0) as u8 as f64]))
        },
        "all" => |_, a, _| {
            nargs("all", a, 1, 2)?;
            let (m, d) = red("all", nonzero(&a[0])?, a.get(1))?;
            boolean(lanes(&m, d, |l| vec![l.iter().all(|&x| x != 0.0) as u8 as f64]))
        },
        "nnz" => |_, a, _| {
            nargs("nnz", a, 1, 1)?;
            num(Matrix::scalar(nonzero(&a[0])?.data.iter().filter(|&&x| x != 0.0).count() as f64))
        },
        "cumsum" => |_, a, _| cscan("cumsum", a, |p, x| p + x, |p, x| p + x),
        "cumprod" => |_, a, _| cscan("cumprod", a, |p, x| p * x, |p, x| p * x),
        "cummax" => |_, a, _| scan("cummax", a, |p, x| if x > p || p.is_nan() { x } else { p }),
        "cummin" => |_, a, _| scan("cummin", a, |p, x| if x < p || p.is_nan() { x } else { p }),
        "max" => |_, a, _| minmax("max", a, true),
        "min" => |_, a, _| minmax("min", a, false),
        "mode" => mode,
        "std" => |_, a, _| spread("std", a, true),
        "var" => |_, a, _| spread("var", a, false),
        "find" => find,
        "sort" => sort,
        "unique" => unique,
        "histc" => histc,
        "cov" => |_, a, _| covar("cov", a, false),
        "corrcoef" => |_, a, _| covar("corrcoef", a, true),
        "corr" => corr,
        "quantile" => |_, a, _| quantile("quantile", a, 1.0),
        "prctile" => |_, a, _| quantile("prctile", a, 100.0),
        "iqr" => |_, a, _| {
            nargs("iqr", a, 1, 2)?;
            let m = real_arg("iqr", &a[0])?;
            let d = dim_of("iqr", &m, a.get(1))?;
            num(lanes(&m, d, |l| {
                let q = quantile_lane(l, &[0.25, 0.75]);
                vec![q[1] - q[0]]
            }))
        },
        "skewness" => |_, a, _| moment("skewness", a, skewness),
        "kurtosis" => |_, a, _| moment("kurtosis", a, kurtosis),
        "zscore" => |_, a, _| {
            nargs("zscore", a, 1, 3)?;
            let opt = opt01("zscore", a.get(1))?;
            let m = real_arg("zscore", &a[0])?;
            let d = dim_of("zscore", &m, a.get(2))?;
            num(lanes(&m, d, |l| {
                let mu = mean(l);
                let sd = var(l, opt).sqrt();
                let sd = if sd == 0.0 { 1.0 } else { sd };
                l.iter().map(|x| (x - mu) / sd).collect()
            }))
        },
        "mad" => |_, a, _| {
            nargs("mad", a, 1, 3)?;
            let opt = opt01("mad", a.get(1))?;
            let m = real_arg("mad", &a[0])?;
            let d = dim_of("mad", &m, a.get(2))?;
            num(lanes(&m, d, |l| {
                let c = if opt == 0.0 { mean(l) } else { median(l) };
                let dev: Vec<f64> = l.iter().map(|x| (x - c).abs()).collect();
                vec![if opt == 0.0 { mean(&dev) } else { median(&dev) }]
            }))
        },
        "normpdf" => |_, a, _| dist("normpdf", a, 1, &[0.0, 1.0], |v| {
            let (x, mu, s) = (v[0], v[1], v[2]);
            if s > 0.0 { (-(x - mu).powi(2) / (2.0 * s * s)).exp() / (s * (2.0 * PI).sqrt()) } else { f64::NAN }
        }),
        "normcdf" => |_, a, _| dist("normcdf", a, 1, &[0.0, 1.0], |v| {
            if v[2] > 0.0 { 0.5 * erfc(-(v[0] - v[1]) / (v[2] * SQRT_2)) } else { f64::NAN }
        }),
        "norminv" => |_, a, _| dist("norminv", a, 1, &[0.0, 1.0], |v| {
            if v[2] > 0.0 { v[1] + v[2] * SQRT_2 * erfinv(2.0 * v[0] - 1.0) } else { f64::NAN }
        }),
        "binopdf" => |_, a, _| dist("binopdf", a, 3, &[f64::NAN, f64::NAN], |v| {
            let (k, n, p) = (v[0], v[1], v[2]);
            if n < 0.0 || n.fract() != 0.0 || !(0.0..=1.0).contains(&p) {
                f64::NAN
            } else if k < 0.0 || k > n || k.fract() != 0.0 {
                0.0
            } else if p == 0.0 || p == 1.0 {
                (k == if p == 0.0 { 0.0 } else { n }) as u8 as f64
            } else {
                (lgamma(n + 1.0) - lgamma(k + 1.0) - lgamma(n - k + 1.0) + k * p.ln() + (n - k) * (-p).ln_1p()).exp()
            }
        }),
        "poisspdf" => |_, a, _| dist("poisspdf", a, 2, &[f64::NAN], |v| {
            let (k, l) = (v[0], v[1]);
            if !(l >= 0.0) {
                f64::NAN
            } else if k < 0.0 || k.fract() != 0.0 {
                0.0
            } else if l == 0.0 {
                (k == 0.0) as u8 as f64
            } else {
                (k * l.ln() - l - lgamma(k + 1.0)).exp()
            }
        }),
        "exppdf" => |_, a, _| dist("exppdf", a, 1, &[1.0], |v| {
            let (x, mu) = (v[0], v[1]);
            if !(mu > 0.0) { f64::NAN } else if x < 0.0 { 0.0 } else { (-x / mu).exp() / mu }
        }),
        "trapz" => |_, a, _| trapz("trapz", a, false),
        "cumtrapz" => |_, a, _| trapz("cumtrapz", a, true),
        _ => return None,
    };
    Some(f)
}

fn fold(l: &[f64], f: fn(f64, f64) -> f64) -> f64 {
    l.iter().cloned().reduce(f).unwrap_or(f64::NAN)
}

fn reduce_of<T: Elem>(name: &str, a: &[Value], m: Mat<T>, f: fn(&[T]) -> T) -> R {
    let (m, d) = red(name, m, a.get(1))?;
    Ok(vec![T::value(lanes(&m, d, |l| vec![f(l)]))])
}

fn reduce(name: &str, a: &[Value], f: fn(&[f64]) -> f64) -> R {
    nargs(name, a, 1, 2)?;
    reduce_of(name, a, real_arg(name, &a[0])?, f)
}

/// sum / prod / mean: the same lane function for real and complex input.
fn creduce(name: &str, a: &[Value], f: fn(&[f64]) -> f64, fc: fn(&[C64]) -> C64) -> R {
    nargs(name, a, 1, 2)?;
    on(&a[0], |m| reduce_of(name, a, m, f), |z| reduce_of(name, a, z, fc))
}

fn sum<T: Elem>(l: &[T]) -> T {
    l.iter().fold(T::default(), |s, &x| s + x)
}

fn prod<T: Elem>(l: &[T]) -> T {
    l.iter().fold(T::one(), |p, &x| p * x)
}

fn mean<T: Elem>(l: &[T]) -> T {
    sum(l) / T::from_real(l.len() as f64)
}

/// any / all / nnz view: complex elements become 1 when nonzero, 0 otherwise.
fn nonzero(v: &Value) -> Result<Matrix, String> {
    match v {
        Value::Complex(z) => Ok(z.map(|z| (z != C64::default()) as u8 as f64)),
        v => v.matrix(),
    }
}

fn median(l: &[f64]) -> f64 {
    if l.is_empty() || l.iter().any(|x| x.is_nan()) {
        return f64::NAN;
    }
    let mut v = l.to_vec();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 }
}

/// Variance; opt 0 normalizes by N-1, opt 1 by N.
fn var(l: &[f64], opt: f64) -> f64 {
    let n = l.len() as f64;
    match l {
        [] => f64::NAN,
        [x] => if x.is_finite() { 0.0 } else { f64::NAN },
        _ => {
            let mu = mean(l);
            l.iter().map(|x| (x - mu).powi(2)).sum::<f64>() / if opt == 1.0 { n } else { n - 1.0 }
        }
    }
}

fn opt01(name: &str, v: Option<&Value>) -> Result<f64, String> {
    let opt = match v {
        Some(v) if v.size() != (0, 0) => v.scalar()?,
        _ => return Ok(0.0),
    };
    if opt == 0.0 || opt == 1.0 { Ok(opt) } else { Err(format!("{name}: normalization OPT must be 0 or 1")) }
}

fn spread(name: &str, a: &[Value], sd: bool) -> R {
    nargs(name, a, 1, 3)?;
    let opt = opt01(name, a.get(1))?;
    let lane_var = |m: Matrix| -> Result<Matrix, String> {
        let (m, d) = red(name, m, a.get(2))?;
        Ok(lanes(&m, d, |l| vec![var(l, opt)]))
    };
    let v = match &a[0] {
        // sum |z - mean|^2 splits into the real part's plus the imaginary part's
        Value::Complex(z) => {
            let (re, im) = (lane_var(z.re())?, lane_var(z.im())?);
            Matrix::new(re.rows, re.cols, re.data.iter().zip(&im.data).map(|(x, y)| x + y).collect())
        }
        v => lane_var(v.matrix()?)?,
    };
    num(if sd { v.map(f64::sqrt) } else { v })
}

fn scan_of<T: Elem>(name: &str, a: &[Value], m: Mat<T>, f: fn(T, T) -> T) -> R {
    let d = dim_of(name, &m, a.get(1))?;
    Ok(vec![T::value(lanes(&m, d, |l| {
        let mut acc: Option<T> = None;
        l.iter().map(|&x| *acc.insert(acc.map_or(x, |p| f(p, x)))).collect()
    }))])
}

fn scan(name: &str, a: &[Value], f: fn(f64, f64) -> f64) -> R {
    nargs(name, a, 1, 2)?;
    scan_of(name, a, real_arg(name, &a[0])?, f)
}

fn cscan(name: &str, a: &[Value], f: fn(f64, f64) -> f64, fc: fn(C64, C64) -> C64) -> R {
    nargs(name, a, 1, 2)?;
    on(&a[0], |m| scan_of(name, a, m, f), |z| scan_of(name, a, z, fc))
}

/// max/min: `(x)`, `(x, [], dim)` with indices, or `(a, b)` element-wise. NaN is skipped.
/// Complex values order by abs, then by arg, like Octave.
fn minmax(name: &str, a: &[Value], is_max: bool) -> R {
    nargs(name, a, 1, 3)?;
    if a.len() == 3 && a[1].size() != (0, 0) {
        return Err(format!("{name}: second argument is ignored, use {name}(x, [], dim)"));
    }
    if a.iter().take(2).any(Value::is_complex) {
        let key = |z: C64| (z.norm(), z.arg());
        let better = |x: C64, y: C64| if is_max { key(x) > key(y) } else { key(x) < key(y) };
        let b = if a.len() == 2 { Some(a[1].cmatrix()?) } else { None };
        return pick(name, a, a[0].cmatrix()?, b, better, C64::is_nan);
    }
    let b = if a.len() == 2 { Some(a[1].matrix()?) } else { None };
    pick(name, a, a[0].matrix()?, b, |x, y| if is_max { x > y } else { x < y }, f64::is_nan)
}

fn pick<T: Elem>(name: &str, a: &[Value], m: Mat<T>, b: Option<Mat<T>>, better: impl Fn(T, T) -> bool, nan: fn(T) -> bool) -> R {
    if let Some(b) = b {
        return Ok(vec![T::value(bcast(name, &[m, b], |v| if nan(v[0]) || better(v[1], v[0]) { v[1] } else { v[0] })?)]);
    }
    if m.is_empty() && a.len() == 1 {
        return Ok(vec![Value::empty(), Value::empty()]);
    }
    let d = dim_of(name, &m, a.get(2))?;
    let (v, i) = lanes2(&m, d, |l| {
        let mut best: Option<usize> = None;
        for (k, &x) in l.iter().enumerate() {
            if !nan(x) && best.is_none_or(|b| better(x, l[b])) {
                best = Some(k);
            }
        }
        let b = best.unwrap_or(0);
        (vec![l.get(b).copied().unwrap_or(T::from_real(f64::NAN))], vec![b as f64 + 1.0])
    });
    Ok(vec![T::value(v), Value::Num(i)])
}

fn mode(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("mode", a, 1, 2)?;
    let (m, d) = red("mode", real_arg("mode", &a[0])?, a.get(1))?;
    let (v, f) = lanes2(&m, d, |l| {
        let mut s = l.to_vec();
        s.sort_by(cmp_nan);
        let (mut best, mut count, mut i) = (f64::NAN, 0, 0);
        while i < s.len() {
            let mut j = i + 1;
            while j < s.len() && s[j] == s[i] {
                j += 1;
            }
            if j - i > count {
                (best, count) = (s[i], j - i);
            }
            i = j;
        }
        (vec![best], vec![count as f64])
    });
    Ok(vec![Value::Num(v), Value::Num(f)])
}

/// Total order with NaN last (ascending), like Octave's sort.
fn cmp_nan(a: &f64, b: &f64) -> Ordering {
    a.partial_cmp(b).unwrap_or_else(|| a.is_nan().cmp(&b.is_nan()))
}

/// Octave's complex order: by abs, then by arg; NaN last.
fn cmp_c(a: &C64, b: &C64) -> Ordering {
    cmp_nan(&a.norm(), &b.norm()).then_with(|| cmp_nan(&a.arg(), &b.arg()))
}

fn find(_: &mut Interp, a: &[Value], nargout: usize) -> R {
    nargs("find", a, 1, 3)?;
    let m = a[0].cmatrix()?;
    let mut idx: Vec<usize> = (0..m.len()).filter(|&k| m.data[k] != C64::default()).collect();
    if let Some(n) = a.get(1) {
        let n = n.as_usize().map_err(|_| "find: N must be a non-negative integer")?;
        if a.get(2).is_some_and(|v| is_opt(v, "last")) {
            idx.drain(..idx.len().saturating_sub(n));
        } else {
            idx.truncate(n);
        }
    }
    fn shape<X: Copy + Default>(m: &Mat<C64>, v: Vec<X>) -> Mat<X> {
        match (m.rows, m.cols) {
            (0, 0) => Mat::empty(),
            (1, _) => Mat::row(v),
            _ => Mat::col(v),
        }
    }
    if nargout <= 1 {
        return num(shape(&m, idx.iter().map(|&k| k as f64 + 1.0).collect()));
    }
    let r = m.rows.max(1);
    Ok(vec![
        Value::Num(shape(&m, idx.iter().map(|&k| (k % r) as f64 + 1.0).collect())),
        Value::Num(shape(&m, idx.iter().map(|&k| (k / r) as f64 + 1.0).collect())),
        Value::complex(shape(&m, idx.iter().map(|&k| m.data[k]).collect())),
    ])
}

fn sort(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("sort", a, 1, 3)?;
    let desc = a[1..].iter().any(|v| is_opt(v, "descend"));
    let order = |o: Ordering| if desc { o.reverse() } else { o };
    if let Value::Cell(c) = &a[0] {
        let s: Vec<&str> = c.iter().map(|v| v.as_str()).collect::<Result<_, _>>().map_err(|_| "sort: only cell arrays of character strings may be sorted")?;
        let mut idx: Vec<usize> = (0..c.len()).collect();
        idx.sort_by(|&i, &j| order(s[i].cmp(s[j])));
        let pos: Vec<f64> = idx.iter().map(|&i| i as f64 + 1.0).collect();
        return Ok(vec![Value::Cell(idx.iter().map(|&i| c[i].clone()).collect()), Value::from(pos)]);
    }
    let dim = a[1..].iter().find(|v| !matches!(v, Value::Str(_)));
    if let Value::Complex(z) = &a[0] {
        let (s, i) = lanes2(z, dim_of("sort", z, dim)?, |l| sorted(l, desc, cmp_c));
        return Ok(vec![Value::complex(s), Value::Num(i)]);
    }
    let m = a[0].matrix()?;
    let (s, i) = lanes2(&m, dim_of("sort", &m, dim)?, |l| sorted(l, desc, cmp_nan));
    let s = match &a[0] {
        Value::Bool(_) => Value::Bool(s),
        Value::Str(_) => Value::Str(chars(&s.data)),
        _ => Value::Num(s),
    };
    Ok(vec![s, Value::Num(i)])
}

/// Stable sort of one lane (ties keep their order): values and 1-based source indices.
fn sorted<T: Copy>(l: &[T], desc: bool, cmp: fn(&T, &T) -> Ordering) -> (Vec<T>, Vec<f64>) {
    let mut idx: Vec<usize> = (0..l.len()).collect();
    idx.sort_by(|&p, &q| if desc { cmp(&l[p], &l[q]).reverse() } else { cmp(&l[p], &l[q]) });
    (idx.iter().map(|&k| l[k]).collect(), idx.iter().map(|&k| k as f64 + 1.0).collect())
}

/// `[u, i, j] = unique(x)`: sorted (or "stable") unique values, u = x(i), x = u(j).
fn unique(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("unique", a, 1, 4)?;
    let stable = a[1..].iter().any(|v| is_opt(v, "stable"));
    let last = a[1..].iter().any(|v| is_opt(v, "last"));
    // sort indices, group equal neighbours
    let (n, row, groups) = match &a[0] {
        Value::Cell(c) => {
            let s: Vec<&str> = c.iter().map(|v| v.as_str()).collect::<Result<_, _>>().map_err(|_| "unique: X must be an array or cell array of strings")?;
            let mut idx: Vec<usize> = (0..s.len()).collect();
            idx.sort_by(|&p, &q| s[p].cmp(s[q]));
            (s.len(), true, group(&idx, |p, q| s[p] == s[q]))
        }
        Value::Complex(z) => {
            let mut idx: Vec<usize> = (0..z.len()).collect();
            idx.sort_by(|&p, &q| cmp_c(&z.data[p], &z.data[q]));
            (z.len(), z.rows == 1, group(&idx, |p, q| z.data[p] == z.data[q]))
        }
        v => {
            let m = v.matrix()?;
            let mut idx: Vec<usize> = (0..m.len()).collect();
            idx.sort_by(|&p, &q| cmp_nan(&m.data[p], &m.data[q]));
            (m.len(), m.rows == 1, group(&idx, |p, q| m.data[p] == m.data[q]))
        }
    };
    let mut groups = groups;
    if stable {
        groups.sort_by_key(|g| g[0]);
    }
    let shape = |v: Vec<f64>| if row { Matrix::row(v) } else { Matrix::col(v) };
    let mut j = vec![0.0; n];
    for (gi, g) in groups.iter().enumerate() {
        g.iter().for_each(|&k| j[k] = gi as f64 + 1.0);
    }
    let i = shape(groups.iter().map(|g| (if last { g[g.len() - 1] } else { g[0] }) as f64 + 1.0).collect());
    let u = match &a[0] {
        Value::Cell(c) => Value::Cell(groups.iter().map(|g| c[g[0]].clone()).collect()),
        Value::Complex(z) => {
            let u: Vec<C64> = groups.iter().map(|g| z.data[g[0]]).collect();
            Value::complex(if row { Mat::row(u) } else { Mat::col(u) })
        }
        v => {
            let m = v.matrix()?;
            let u = shape(groups.iter().map(|g| m.data[g[0]]).collect());
            match v {
                Value::Str(_) => Value::Str(chars(&u.data)),
                Value::Bool(_) => Value::Bool(u),
                _ if n == 0 => Value::empty(),
                _ => Value::Num(u),
            }
        }
    };
    Ok(vec![u, Value::Num(i), Value::Num(shape(j))])
}

/// Runs of equal elements in a sorted index list; each group keeps original order.
fn group(idx: &[usize], eq: impl Fn(usize, usize) -> bool) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = vec![];
    for &k in idx {
        match groups.last_mut() {
            Some(g) if eq(g[0], k) => g.push(k),
            _ => groups.push(vec![k]),
        }
    }
    groups
}

fn histc(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("histc", a, 2, 3)?;
    let edges = real_arg("histc", &a[1])?.data;
    let m = real_arg("histc", &a[0])?;
    let d = dim_of("histc", &m, a.get(2))?;
    num(lanes(&m, d, |l| {
        let mut n = vec![0.0; edges.len()];
        for &x in l {
            let k = edges.partition_point(|&e| e <= x);
            if edges.last() == Some(&x) {
                n[edges.len() - 1] += 1.0;
            } else if k > 0 && k < edges.len() {
                n[k - 1] += 1.0;
            }
        }
        n
    }))
}

/// Covariance of the columns of x (rows are observations).
fn cov_m(x: &Matrix, opt: f64) -> Matrix {
    let (n, p) = (x.rows, x.cols);
    let mu: Vec<f64> = (0..p).map(|j| (0..n).map(|i| x.get(i, j)).sum::<f64>() / n as f64).collect();
    let den = if opt == 1.0 || n == 1 { n as f64 } else { n as f64 - 1.0 };
    let mut c = Matrix::zeros(p, p);
    for s in 0..p {
        for t in 0..p {
            c.set(s, t, (0..n).map(|i| (x.get(i, s) - mu[s]) * (x.get(i, t) - mu[t])).sum::<f64>() / den);
        }
    }
    c
}

fn corr_of(c: &Matrix) -> Matrix {
    let mut r = c.clone();
    for k in 0..c.len() {
        let (i, j) = (k % c.rows, k / c.rows);
        r.data[k] = if i == j && c.get(i, i) > 0.0 { 1.0 } else { c.data[k] / (c.get(i, i) * c.get(j, j)).sqrt() };
    }
    r
}

/// Row vectors become columns: one variable, many observations.
fn obs(v: &Value) -> Result<Matrix, String> {
    let m = v.matrix()?;
    Ok(if m.rows == 1 { m.transpose() } else { m })
}

/// cov(x), cov(x, opt), cov(x, y[, opt]) -> cov([x(:) y(:)]) as in Octave 8+ / MATLAB.
fn covar(name: &str, a: &[Value], corr: bool) -> R {
    nargs(name, a, 1, 3)?;
    let x = obs(&a[0])?;
    let two = a.len() == 3 || a.get(1).is_some_and(|v| v.size() != (1, 1) || x.len() == 1);
    let (x, opt) = if two {
        let y = a[1].matrix()?;
        if y.len() != x.len() {
            return Err(format!("{name}: X and Y must have the same number of observations"));
        }
        (Matrix::new(x.len(), 2, x.data.iter().chain(&y.data).cloned().collect()), a.get(2))
    } else {
        (x, a.get(1))
    };
    let opt = if corr { 0.0 } else { opt01(name, opt)? };
    let c = cov_m(&x, opt);
    num(if corr { corr_of(&c) } else if c.len() == 1 || x.cols == 1 { Matrix::scalar(c.data[0]) } else { c })
}

fn corr(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("corr", a, 1, 2)?;
    let x = obs(&a[0])?;
    let Some(y) = a.get(1) else { return num(corr_of(&cov_m(&x, 0.0))) };
    let y = obs(y)?;
    if x.rows != y.rows {
        return Err("corr: X and Y must have the same number of observations".into());
    }
    let (p, q) = (x.cols, y.cols);
    let r = corr_of(&cov_m(&Matrix::new(x.rows, p + q, x.data.iter().chain(&y.data).cloned().collect()), 0.0));
    num(Matrix::new(p, q, (0..p * q).map(|k| r.get(k % p, p + k / p)).collect()))
}

/// Octave's default quantile method 5: piecewise linear with p(k) = (k - 0.5) / n. NaN skipped.
fn quantile_lane(l: &[f64], p: &[f64]) -> Vec<f64> {
    let mut v: Vec<f64> = l.iter().cloned().filter(|x| !x.is_nan()).collect();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    p.iter()
        .map(|&p| {
            let pos = n as f64 * p + 0.5;
            match n {
                0 => f64::NAN,
                _ if pos < 1.0 => v[0],
                _ if pos >= n as f64 => v[n - 1],
                _ => {
                    let i = pos.floor() as usize - 1;
                    v[i] + (pos - pos.floor()) * (v[i + 1] - v[i])
                }
            }
        })
        .collect()
}

fn quantile(name: &str, a: &[Value], scale: f64) -> R {
    nargs(name, a, 1, 3)?;
    let p: Vec<f64> = match a.get(1) {
        Some(v) if v.size() != (0, 0) => v.matrix()?.data.iter().map(|x| x / scale).collect(),
        _ => vec![0.0, 0.25, 0.5, 0.75, 1.0],
    };
    if p.iter().any(|x| !(0.0..=1.0).contains(x)) {
        return Err(format!("{name}: P must be in the range [0, {scale}]"));
    }
    let m = real_arg(name, &a[0])?;
    let d = dim_of(name, &m, a.get(2))?;
    num(lanes(&m, d, |l| quantile_lane(l, &p)))
}

fn moment(name: &str, a: &[Value], f: fn(&[f64], bool) -> f64) -> R {
    nargs(name, a, 1, 3)?;
    let flag = match a.get(1) {
        Some(v) if v.size() != (0, 0) => v.scalar()?,
        _ => 1.0,
    };
    let m = real_arg(name, &a[0])?;
    let d = dim_of(name, &m, a.get(2))?;
    num(lanes(&m, d, |l| vec![f(l, flag == 0.0)]))
}

/// Central moments m2, mk of a lane.
fn central(l: &[f64], k: i32) -> (f64, f64) {
    let mu = mean(l);
    let n = l.len() as f64;
    (l.iter().map(|x| (x - mu).powi(2)).sum::<f64>() / n, l.iter().map(|x| (x - mu).powi(k)).sum::<f64>() / n)
}

fn skewness(l: &[f64], unbiased: bool) -> f64 {
    let (m2, m3) = central(l, 3);
    let n = l.len() as f64;
    let y = m3 / m2.powf(1.5);
    if unbiased { y * (n * (n - 1.0)).sqrt() / (n - 2.0) } else { y }
}

fn kurtosis(l: &[f64], unbiased: bool) -> f64 {
    let (m2, m4) = central(l, 4);
    let n = l.len() as f64;
    let y = m4 / (m2 * m2);
    if unbiased { 3.0 + (n - 1.0) / ((n - 2.0) * (n - 3.0)) * ((n + 1.0) * y - 3.0 * (n - 1.0)) } else { y }
}

/// Distribution with `lo` required args and defaults for the rest, broadcast element-wise.
fn dist(name: &str, a: &[Value], lo: usize, defs: &[f64], f: fn(&[f64]) -> f64) -> R {
    nargs(name, a, lo, 1 + defs.len())?;
    let mut ms = vec![real_arg(name, &a[0])?];
    for (i, &d) in defs.iter().enumerate() {
        ms.push(match a.get(i + 1) {
            Some(v) => real_arg(name, v)?,
            None => Matrix::scalar(d),
        });
    }
    num(bcast(name, &ms, f)?)
}

/// trapz(y), trapz(x, y), trapz(..., dim); x is a spacing scalar or a coordinate vector.
fn trapz(name: &str, a: &[Value], cumulative: bool) -> R {
    nargs(name, a, 1, 3)?;
    let y_is_dim = a.len() == 2 && a[1].size() == (1, 1) && a[0].size() != (1, 1);
    let (x, y, d) = match a.len() {
        1 => (None, &a[0], None),
        2 if y_is_dim => (None, &a[0], a.get(1)),
        _ => (Some(real_arg(name, &a[0])?.data), &a[1], a.get(2)),
    };
    on(y, |m| trapz_of(name, x.as_deref(), m, d, cumulative), |z| trapz_of(name, x.as_deref(), z, d, cumulative))
}

fn trapz_of<T: Elem>(name: &str, x: Option<&[f64]>, y: Mat<T>, d: Option<&Value>, cumulative: bool) -> R {
    let d = dim_of(name, &y, d)?;
    let len = match d {
        1 => y.rows,
        2 => y.cols,
        _ => 1,
    };
    if let Some(x) = &x
        && x.len() != 1
        && x.len() != len
    {
        return Err(format!("{name}: length of X and length of Y along DIM must match"));
    }
    Ok(vec![T::value(lanes(&y, d, |l| {
        let dx = |i: usize| match &x {
            Some(x) if x.len() == 1 => x[0],
            Some(x) => x[i + 1] - x[i],
            None => 1.0,
        };
        let mut acc = T::default();
        let mut out = vec![T::default(); l.len()];
        for i in 1..l.len() {
            acc += (l[i - 1] + l[i]).scale(dx(i - 1) / 2.0);
            out[i] = acc;
        }
        if cumulative { out } else { vec![acc] }
    }))])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(name: &str, args: &[Value], nargout: usize) -> Vec<Matrix> {
        lookup(name).unwrap()(&mut Interp::new(), args, nargout).unwrap().iter().map(|v| v.matrix().unwrap()).collect()
    }

    fn m(name: &str, args: &[Value]) -> Matrix {
        run(name, args, 1).remove(0)
    }

    fn a23() -> Value {
        Matrix::from_rows(&[vec![1., 2., 3.], vec![4., 5., 6.]]).into()
    }

    #[test]
    fn reductions() {
        assert_eq!(m("sum", &[a23()]).data, vec![5., 7., 9.]);
        assert_eq!(m("sum", &[a23(), Value::num(2.)]), Matrix::col(vec![6., 15.]));
        assert_eq!(m("sum", &[Value::empty()]).data, vec![0.]);
        assert_eq!(m("prod", &[Value::from(vec![1., 2., 3., 4.])]).data, vec![24.]);
        assert_eq!(m("cumsum", &[a23()]), Matrix::from_rows(&[vec![1., 2., 3.], vec![5., 7., 9.]]));
        assert_eq!(m("cumsum", &[a23(), Value::num(2.)]), Matrix::from_rows(&[vec![1., 3., 6.], vec![4., 9., 15.]]));
        assert_eq!(m("cumprod", &[Value::from(vec![1., 2., 3.])]).data, vec![1., 2., 6.]);
        assert_eq!(m("mean", &[a23()]).data, vec![2.5, 3.5, 4.5]);
        assert_eq!(m("median", &[Value::from(vec![3., 1., 4., 1., 5.])]).data, vec![3.]);
        assert_eq!(m("median", &[Value::from(vec![3., 1., 4., 2.])]).data, vec![2.5]);
        let v = Value::from(vec![2., 4., 4., 4., 5., 5., 7., 9.]);
        assert!((m("var", &[v.clone()]).data[0] - 32.0 / 7.0).abs() < 1e-12);
        assert_eq!(m("std", &[v.clone(), Value::num(1.)]).data, vec![2.]);
        assert_eq!(m("mode", &[v.clone()]).data, vec![4.]);
        assert_eq!(m("range", &[v.clone()]).data, vec![7.]);
        let mi = run("max", &[Value::from(vec![3., 9., f64::NAN, 9.])], 2);
        assert_eq!((mi[0].data[0], mi[1].data[0]), (9., 2.));
        assert_eq!(m("min", &[a23(), Value::empty(), Value::num(2.)]), Matrix::col(vec![1., 4.]));
        assert_eq!(m("max", &[Value::from(vec![1., 5.]), Value::num(3.)]).data, vec![3., 5.]);
        assert_eq!(m("any", &[Value::from(vec![0., 0., 1.])]).data, vec![1.]);
        assert_eq!(m("all", &[a23()]).data, vec![1., 1., 1.]);
        assert_eq!(m("cummax", &[Value::from(vec![1., 3., 2., 5.])]).data, vec![1., 3., 3., 5.]);
    }

    #[test]
    fn search_sort() {
        let si = run("sort", &[Value::from(vec![3., 1., 2., 1.]), "descend".into()], 2);
        assert_eq!(si[0].data, vec![3., 2., 1., 1.]);
        assert_eq!(si[1].data, vec![1., 3., 2., 4.]);
        let si = run("sort", &[Value::from(vec![3., f64::NAN, 1.])], 2);
        assert_eq!(si[1].data, vec![3., 1., 2.]);
        assert_eq!(m("find", &[Value::from(vec![0., 1., 0., 2.])]).data, vec![2., 4.]);
        assert_eq!(m("find", &[Value::from(vec![0., 1., 0., 2.]), Value::num(1.)]).data, vec![2.]);
        let rc = run("find", &[Matrix::from_rows(&[vec![0., 1.], vec![1., 0.]]).into()], 2);
        assert_eq!((rc[0].data.clone(), rc[1].data.clone()), (vec![2., 1.], vec![1., 2.]));
        assert_eq!((rc[0].rows, rc[0].cols), (2, 1));
        let u = run("unique", &[Value::from(vec![3., 1., 3., 2.])], 3);
        assert_eq!(u[0].data, vec![1., 2., 3.]);
        assert_eq!(u[1].data, vec![2., 4., 1.]);
        assert_eq!(u[2].data, vec![3., 1., 3., 2.]);
        assert_eq!(m("nnz", &[a23()]).data, vec![6.]);
        assert_eq!(m("histc", &[Value::from(vec![0.5, 1., 1.5, 2., 3.]), Value::from(vec![0., 1., 2.])]).data, vec![1., 2., 1.]);
    }

    fn cv(v: &[(f64, f64)]) -> Value {
        Value::complex(Mat::row(v.iter().map(|&(re, im)| C64::new(re, im)).collect()))
    }

    fn cm(name: &str, args: &[Value], nargout: usize) -> Vec<Value> {
        lookup(name).unwrap()(&mut Interp::new(), args, nargout).unwrap()
    }

    fn data(v: &Value) -> Vec<C64> {
        v.cmatrix().unwrap().data
    }

    #[test]
    fn complex_reductions() {
        let c = C64::new;
        let z = cv(&[(1., 1.), (-2., 0.), (1., -1.), (0., 0.5)]);
        assert_eq!(data(&cm("sum", &[z.clone()], 1)[0]), vec![c(0., 0.5)]);
        assert_eq!(data(&cm("mean", &[z.clone()], 1)[0]), vec![c(0., 0.125)]);
        assert_eq!(data(&cm("prod", &[cv(&[(1., 1.), (1., -1.)])], 1)[0]), vec![c(2., 0.)]);
        assert!(matches!(cm("prod", &[cv(&[(1., 1.), (1., -1.)])], 1)[0], Value::Num(_)));
        assert_eq!(data(&cm("cumsum", &[z.clone()], 1)[0]), vec![c(1., 1.), c(-1., 1.), c(0., 0.), c(0., 0.5)]);
        // sort / max / min order by abs, then arg
        let si = cm("sort", &[z.clone()], 2);
        assert_eq!(data(&si[0]), vec![c(0., 0.5), c(1., -1.), c(1., 1.), c(-2., 0.)]);
        assert_eq!(si[1].matrix().unwrap().data, vec![4., 3., 1., 2.]);
        assert_eq!(data(&cm("sort", &[z.clone(), "descend".into()], 1)[0])[0], c(-2., 0.));
        let mi = cm("max", &[z.clone()], 2);
        assert_eq!((data(&mi[0]), mi[1].matrix().unwrap().data), (vec![c(-2., 0.)], vec![2.]));
        let mi = cm("min", &[z.clone()], 2);
        assert_eq!((data(&mi[0]), mi[1].matrix().unwrap().data), (vec![c(0., 0.5)], vec![4.]));
        assert_eq!(data(&cm("max", &[cv(&[(1., 1.), (1., -1.)]), Value::num(1.)], 1)[0]), vec![c(1., 1.), c(1., -1.)]);
        // |z - mean|^2: var([i -i]) = (1 + 1) / 1
        assert_eq!(cm("var", &[cv(&[(0., 1.), (0., -1.)])], 1)[0].matrix().unwrap().data, vec![2.]);
        assert_eq!(cm("std", &[cv(&[(0., 1.), (0., -1.)]), Value::num(1.)], 1)[0].matrix().unwrap().data, vec![1.]);
        // nonzero tests count either part
        let nz = cv(&[(0., 1.), (0., 0.), (2., 0.)]);
        assert_eq!(cm("nnz", &[nz.clone()], 1)[0].matrix().unwrap().data, vec![2.]);
        assert_eq!(cm("any", &[nz.clone()], 1)[0].matrix().unwrap().data, vec![1.]);
        assert_eq!(cm("all", &[nz.clone()], 1)[0].matrix().unwrap().data, vec![0.]);
        let f = cm("find", &[nz.clone()], 3);
        assert_eq!(f[1].matrix().unwrap().data, vec![1., 3.]);
        assert_eq!(data(&f[2]), vec![c(0., 1.), c(2., 0.)]);
        let u = cm("unique", &[cv(&[(1., 1.), (0., 1.), (1., 1.)])], 1);
        assert_eq!(data(&u[0]), vec![c(0., 1.), c(1., 1.)]);
        let err = lookup("normpdf").unwrap()(&mut Interp::new(), &[z.clone()], 1).unwrap_err();
        assert_eq!(err, "normpdf: complex input not supported");
        assert_eq!(lookup("median").unwrap()(&mut Interp::new(), &[z], 1).unwrap_err(), "median: complex input not supported");
    }

    #[test]
    fn statistics() {
        let x = Value::from(vec![1., 2., 3., 4.]);
        assert_eq!(m("quantile", &[x.clone(), Value::num(0.5)]).data, vec![2.5]);
        assert_eq!(m("prctile", &[x.clone(), Value::from(vec![25., 75.])]).data, vec![1.5, 3.5]);
        assert_eq!(m("iqr", &[x.clone()]).data, vec![2.]);
        let y = Value::from(vec![2., 4., 6., 8.1]);
        let r = m("corrcoef", &[x.clone(), y.clone()]);
        assert!((r.get(0, 0) - 1.0).abs() < 1e-15 && r.get(0, 1) > 0.99 && r.get(0, 1) < 1.0);
        assert!((m("corr", &[x.clone(), y.clone()]).data[0] - r.get(0, 1)).abs() < 1e-15);
        assert!((m("cov", &[x.clone()]).data[0] - 5.0 / 3.0).abs() < 1e-12);
        assert_eq!(m("cov", &[x.clone(), x.clone()]).len(), 4);
        assert!(m("skewness", &[x.clone()]).data[0].abs() < 1e-15);
        assert!((m("kurtosis", &[x.clone()]).data[0] - 1.64).abs() < 1e-12);
        assert!(m("zscore", &[x.clone()]).data.iter().sum::<f64>().abs() < 1e-12);
        assert_eq!(m("mad", &[x.clone()]).data, vec![1.]);
        assert!((m("normpdf", &[Value::num(0.)]).data[0] - 0.398_942_280_401_432_7).abs() < 1e-15);
        assert!((m("normcdf", &[Value::num(1.96)]).data[0] - 0.975_002_104_851_780).abs() < 1e-12);
        assert!((m("norminv", &[Value::num(0.975)]).data[0] - 1.959_963_984_540_054).abs() < 1e-12);
        assert!((m("binopdf", &[Value::num(2.), Value::num(4.), Value::num(0.5)]).data[0] - 0.375).abs() < 1e-14);
        assert!((m("poisspdf", &[Value::num(2.), Value::num(3.)]).data[0] - 4.5 * (-3f64).exp()).abs() < 1e-14);
        assert!((m("exppdf", &[Value::num(1.), Value::num(2.)]).data[0] - 0.5 * (-0.5f64).exp()).abs() < 1e-15);
        assert_eq!(m("trapz", &[Value::from(vec![1., 2., 3.])]).data, vec![4.]);
        assert_eq!(m("trapz", &[Value::from(vec![0., 1., 3.]), Value::from(vec![1., 1., 1.])]).data, vec![3.]);
        assert_eq!(m("cumtrapz", &[Value::from(vec![1., 2., 3.])]).data, vec![0., 1.5, 4.]);
    }
}
