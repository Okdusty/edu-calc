//! Numeric library: math, linear algebra, statistics, numerics, strings.
pub mod ops;

mod calculus;
mod fft;
mod linalg;
mod math;
mod numeric;
mod stats;
mod strings;

use crate::lang::Interp;
use crate::value::{Builtin, C64, CMatrix, Mat, Matrix, Value};

pub fn lookup(name: &str) -> Option<Builtin> {
    math::lookup(name)
        .or_else(|| stats::lookup(name))
        .or_else(|| linalg::lookup(name))
        .or_else(|| numeric::lookup(name))
        .or_else(|| fft::lookup(name))
        .or_else(|| strings::lookup(name))
        .or_else(|| calculus::lookup(name))
        .or_else(|| ops::lookup(name))
}

type R = Result<Vec<Value>, String>;

/// Element types the generic helpers run on: f64 and C64.
trait Elem: nalgebra::ComplexField<RealField = f64> + Copy + Default {
    /// Wrap a result; complex narrows to real when every imaginary part is 0.
    fn value(m: Mat<Self>) -> Value;
}

impl Elem for f64 {
    fn value(m: Matrix) -> Value {
        Value::Num(m)
    }
}

impl Elem for C64 {
    fn value(m: CMatrix) -> Value {
        Value::complex(m)
    }
}

/// Run `re` on the real view of `v`, or `cx` when it is complex.
fn on(v: &Value, re: impl FnOnce(Matrix) -> R, cx: impl FnOnce(CMatrix) -> R) -> R {
    match v {
        Value::Complex(z) => cx(z.clone()),
        v => re(v.matrix()?),
    }
}

/// Real view for functions that are real by nature: complex input gets an error naming `name`.
fn real_arg(name: &str, v: &Value) -> Result<Matrix, String> {
    if v.is_complex() { Err(format!("{name}: complex input not supported")) } else { v.matrix() }
}

/// `Invalid call to NAME` unless lo <= args.len() <= hi.
fn nargs(name: &str, args: &[Value], lo: usize, hi: usize) -> Result<(), String> {
    if args.len() < lo || args.len() > hi { Err(format!("Invalid call to {name}")) } else { Ok(()) }
}

fn num(m: Matrix) -> R {
    Ok(vec![Value::Num(m)])
}

fn boolean(m: Matrix) -> R {
    Ok(vec![Value::Bool(m)])
}

/// Case-insensitive "is this arg the string `s`".
fn is_opt(v: &Value, s: &str) -> bool {
    matches!(v, Value::Str(x) if x.eq_ignore_ascii_case(s))
}

/// Explicit DIM (1-based) or the first non-singleton dimension.
fn dim_of<T>(name: &str, m: &Mat<T>, d: Option<&Value>) -> Result<usize, String> {
    match d {
        Some(v) => match v.scalar() {
            Ok(x) if x >= 1.0 && x.fract() == 0.0 => Ok(x as usize),
            _ => Err(format!("{name}: DIM must be a valid dimension")),
        },
        None => Ok(if m.rows != 1 { 1 } else { 2 }),
    }
}

/// Reduction input: `sum([])` reduces a 0x1 so it is 0 like Octave, not 1x0. Returns (X, DIM).
fn red<T: Copy + Default>(name: &str, m: Mat<T>, d: Option<&Value>) -> Result<(Mat<T>, usize), String> {
    let m = if d.is_none() && m.rows == 0 && m.cols == 0 { Mat::zeros(0, 1) } else { m };
    let dim = dim_of(name, &m, d)?;
    Ok((m, dim))
}

/// Apply `f` to every vector along `dim` (1: columns, 2: rows, 3+: single elements) and stack
/// both results along that dim. Each call must return the same lengths.
fn lanes2<T, U, V>(m: &Mat<T>, dim: usize, mut f: impl FnMut(&[T]) -> (Vec<U>, Vec<V>)) -> (Mat<U>, Mat<V>)
where
    T: Copy + Default,
    U: Copy + Default,
    V: Copy + Default,
{
    fn back<X: Copy + Default>(x: Mat<X>, r: usize, c: usize) -> Mat<X> {
        if x.rows == 1 { Mat::new(r, c, x.data) } else { x }
    }
    fn stack<X: Copy + Default>(v: Vec<Vec<X>>, k: usize, n: usize, dim: usize) -> Mat<X> {
        if dim == 1 { Mat::new(k, n, v.concat()) } else { Mat::new(n, k, (0..n * k).map(|q| v[q % n][q / n]).collect()) }
    }
    if dim > 2 {
        let (a, b) = lanes2(&Mat::row(m.data.clone()), 1, f);
        return (back(a, m.rows, m.cols), back(b, m.rows, m.cols));
    }
    let (n, len) = if dim == 1 { (m.cols, m.rows) } else { (m.rows, m.cols) };
    let (mut xs, mut ys) = (vec![], vec![]);
    for l in 0..n {
        let lane: Vec<T> = (0..len).map(|i| if dim == 1 { m.get(i, l) } else { m.get(l, i) }).collect();
        let (x, y) = f(&lane);
        xs.push(x);
        ys.push(y);
    }
    let (kx, ky) = match (xs.first(), ys.first()) {
        (Some(x), Some(y)) => (x.len(), y.len()),
        _ => {
            let (x, y) = f(&vec![T::default(); len]);
            (x.len(), y.len())
        }
    };
    (stack(xs, kx, n, dim), stack(ys, ky, n, dim))
}

fn lanes<T: Copy + Default, U: Copy + Default>(m: &Mat<T>, dim: usize, mut f: impl FnMut(&[T]) -> Vec<U>) -> Mat<U> {
    lanes2(m, dim, |l| (f(l), Vec::<f64>::new())).0
}

/// Element-wise over any number of broadcast-compatible matrices.
fn bcast<T: Copy + Default, U: Copy + Default>(name: &str, ms: &[Mat<T>], f: impl Fn(&[T]) -> U) -> Result<Mat<U>, String> {
    let fit = |a: usize, b: usize| if a == b || b == 1 { Some(a) } else if a == 1 { Some(b) } else { None };
    let (mut r, mut c) = (1, 1);
    for m in ms {
        (r, c) = fit(r, m.rows).zip(fit(c, m.cols)).ok_or_else(|| format!("{name}: nonconformant arguments"))?;
    }
    let at = |m: &Mat<T>, i: usize, j: usize| m.get(if m.rows == 1 { 0 } else { i }, if m.cols == 1 { 0 } else { j });
    let mut out = Vec::with_capacity(r * c);
    let mut buf = vec![T::default(); ms.len()];
    for j in 0..c {
        for i in 0..r {
            for (k, m) in ms.iter().enumerate() {
                buf[k] = at(m, i, j);
            }
            out.push(f(&buf));
        }
    }
    Ok(Mat::new(r, c, out))
}

/// Apply a shape operation to `v`, keeping logical / char / cell / complex type where it still
/// fits. Cells and complex go through 1-based element indices, 0 marks a zero fill (triu, diag).
// ponytail: char results with >1 row become double (no char matrices), cells stay 1xN.
fn reshaped(v: &Value, f: impl FnOnce(Matrix) -> Result<Matrix, String>) -> Result<Value, String> {
    let idx = |r: usize, c: usize| Matrix::new(r, c, (1..=r * c).map(|i| i as f64).collect());
    match v {
        Value::Cell(c) => {
            let ix = f(idx(1, c.len()))?;
            return Ok(Value::Cell(ix.data.iter().map(|&i| if i == 0.0 { Value::empty() } else { c[i as usize - 1].clone() }).collect()));
        }
        Value::Complex(z) => {
            let ix = f(idx(z.rows, z.cols))?;
            return Ok(Value::complex(ix.map(|i| if i == 0.0 { C64::default() } else { z.data[i as usize - 1] })));
        }
        _ => {}
    }
    let m = f(v.matrix()?)?;
    Ok(match v {
        Value::Bool(_) => Value::Bool(m),
        Value::Str(_) if m.rows <= 1 => Value::Str(chars(&m.data)),
        _ => Value::Num(m),
    })
}

fn chars(codes: &[f64]) -> String {
    codes.iter().map(|&c| char::from_u32(c as u32).unwrap_or('\u{fffd}')).collect()
}

/// Scalar callback `x -> f(x)` through the interpreter.
fn fun1<'a>(interp: &'a mut Interp, f: &'a Value) -> impl FnMut(f64) -> Result<f64, String> + 'a {
    move |x| first(interp.call(f, &[Value::num(x)], 1)?)?.scalar()
}

fn first(v: Vec<Value>) -> Result<Value, String> {
    v.into_iter().next().ok_or_else(|| "value on right hand side of assignment is undefined".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lanes_shapes() {
        let m = Matrix::from_rows(&[vec![1., 2., 3.], vec![4., 5., 6.]]);
        let sum = |l: &[f64]| vec![l.iter().sum::<f64>()];
        assert_eq!(lanes(&m, 1, sum), Matrix::row(vec![5., 7., 9.]));
        assert_eq!(lanes(&m, 2, sum), Matrix::col(vec![6., 15.]));
        assert_eq!(lanes(&m, 3, sum), m);
        assert_eq!(lanes(&Matrix::zeros(3, 0), 1, sum), Matrix::zeros(1, 0));
        let (v, i) = lanes2(&m, 2, |l| (vec![l[2]], vec![3.0]));
        let z = CMatrix::from_parts(&m, &m);
        let tr = reshaped(&Value::Complex(z.clone()), |x| Ok(x.transpose())).unwrap();
        assert!(matches!(tr, Value::Complex(t) if t == z.transpose()));
        let tri = reshaped(&Value::Complex(z), |x| Ok(x.map(|i| if i == 2.0 { 0.0 } else { i }))).unwrap();
        assert_eq!(tri.cmatrix().unwrap().data[1], C64::default());
        assert_eq!((v, i), (Matrix::col(vec![3., 6.]), Matrix::col(vec![3., 3.])));
        assert_eq!(bcast("f", &[Matrix::zeros(2, 1), Matrix::zeros(1, 3), Matrix::scalar(1.)], |_| 1.).unwrap().len(), 6);
        assert!(bcast("f", &[Matrix::zeros(2, 1), Matrix::zeros(3, 1)], |_| 1.).is_err());
    }
}
