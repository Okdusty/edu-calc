//! Linear algebra on nalgebra (`DMatrix::from_vec` is column-major, same as ours).
use nalgebra::DMatrix;

use super::{Elem, R, dim_of, first, is_opt, lanes, nargs, num, on};
use crate::lang::Interp;
use crate::value::{BinOp, Builtin, C64, CMatrix, Mat, Matrix, UnOp, Value};

pub fn lookup(name: &str) -> Option<Builtin> {
    let f: Builtin = match name {
        "norm" => norm,
        "dot" => dot,
        "cross" => cross,
        "trace" => trace,
        "rank" => rank,
        "det" => det,
        "inv" | "inverse" => inv,
        "pinv" => pinv,
        "lu" => lu,
        "qr" => qr,
        "chol" => chol,
        "eig" => eig,
        "svd" => svd,
        "null" => null,
        "orth" => orth,
        "expm" => |_, a, _| {
            nargs("expm", a, 1, 1)?;
            fn of<T: Elem>(m: Mat<T>) -> R {
                square("expm", &m)?;
                finite("expm", &m)?;
                Ok(vec![T::value(expm(&m))])
            }
            on(&a[0], of, of)
        },
        "sqrtm" => |_, a, _| schur_fn("sqrtm", a, sqrt_tri),
        "logm" => |_, a, _| schur_fn("logm", a, log_tri),
        "rref" => rref,
        "cond" => cond,
        "linsolve" => |_, a, _| {
            nargs("linsolve", a, 2, 3)?;
            // ponytail: LT/UT/SYM/POSDEF are speed hints; only TRANSA changes the answer
            let t = matches!(a.get(2), Some(Value::Struct(o)) if o.get("TRANSA").is_some_and(|v| v.truthy().unwrap_or(false)));
            let am = if t { super::ops::unop(UnOp::CTranspose, &a[0])? } else { a[0].clone() };
            Ok(vec![super::ops::binop(BinOp::LDiv, &am, &a[1])?])
        },
        "hess" => hess,
        "schur" => schur,
        "vecnorm" => |_, a, _| {
            nargs("vecnorm", a, 1, 3)?;
            let m = absolute(&a[0])?;
            let p = match a.get(1) {
                Some(v) if v.size() != (0, 0) => v.scalar()?,
                _ => 2.0,
            };
            if !(p > 0.0) {
                return Err("vecnorm: P must be positive real scalar or Inf".into());
            }
            let d = dim_of("vecnorm", &m, a.get(2))?;
            num(lanes(&m, d, |l| vec![pnorm(l, p)]))
        },
        "transpose" => |_, a, _| {
            nargs("transpose", a, 1, 1)?;
            Ok(vec![super::ops::unop(UnOp::Transpose, &a[0])?])
        },
        _ => return None,
    };
    Some(f)
}

fn na<T: Elem>(m: &Mat<T>) -> DMatrix<T> {
    DMatrix::from_vec(m.rows, m.cols, m.data.clone())
}

fn mat<T: Elem>(d: &DMatrix<T>) -> Mat<T> {
    Mat::new(d.nrows(), d.ncols(), d.as_slice().to_vec())
}

fn square<T>(name: &str, m: &Mat<T>) -> Result<(), String> {
    if m.rows == m.cols { Ok(()) } else { Err(format!("{name}: A must be a square matrix")) }
}

fn finite<T: Elem>(name: &str, m: &Mat<T>) -> Result<(), String> {
    if m.data.iter().all(|x| x.is_finite()) { Ok(()) } else { Err(format!("{name}: input must not contain Inf or NaN values")) }
}

fn diag_of<T: Elem>(v: &[T]) -> Mat<T> {
    let mut d = Mat::zeros(v.len(), v.len());
    v.iter().enumerate().for_each(|(i, &x)| d.set(i, i, x));
    d
}

fn eye(n: usize) -> Matrix {
    let mut m = Matrix::zeros(n, n);
    (0..n).for_each(|i| m.set(i, i, 1.0));
    m
}

/// Thin SVD: U (m x k), singular values descending, V (n x k), k = min(m, n).
type Svd<T> = (DMatrix<T>, Vec<f64>, DMatrix<T>);

fn svd_of<T: Elem>(name: &str, m: &Mat<T>) -> Result<Svd<T>, String> {
    finite(name, m)?;
    if m.is_empty() {
        return Ok((DMatrix::zeros(m.rows, 0), vec![], DMatrix::zeros(m.cols, 0)));
    }
    let s = na(m).try_svd(true, true, f64::EPSILON, 10_000).ok_or_else(|| format!("{name}: SVD did not converge"))?;
    let (Some(u), Some(vt)) = (s.u, s.v_t) else { return Err(format!("{name}: SVD failed")) };
    Ok((u, s.singular_values.as_slice().to_vec(), vt.adjoint()))
}

/// Singular value cutoff: explicit TOL, or max(size) * s_max * eps.
fn tol_of<T>(m: &Mat<T>, s: &[f64], t: Option<&Value>) -> Result<f64, String> {
    match t {
        Some(t) => t.scalar(),
        None => Ok(m.rows.max(m.cols) as f64 * s.first().unwrap_or(&0.0) * f64::EPSILON),
    }
}

/// Pseudo-inverse V * S^-1 * U' from a thin SVD, dropping singular values <= tol.
fn pinv_of<T: Elem>((u, s, v): Svd<T>, tol: f64) -> DMatrix<T> {
    let inv_s = DMatrix::from_fn(s.len(), s.len(), |i, j| T::from_real(if i == j && s[i] > tol { 1.0 / s[i] } else { 0.0 }));
    v * inv_s * u.adjoint()
}

/// Extend orthonormal columns q (m x k) to a full m x m orthonormal basis.
fn complete<T: Elem>(q: &DMatrix<T>) -> DMatrix<T> {
    let (m, k) = q.shape();
    if k >= m {
        return q.clone();
    }
    let aug = DMatrix::from_fn(m, k + m, |i, j| if j < k { q[(i, j)] } else { T::from_real((i + k == j) as u8 as f64) });
    let mut full = aug.qr().q();
    full.columns_mut(0, k).copy_from(q);
    full
}

/// `A \ B`: LU with partial pivoting when square, least squares (min norm) otherwise.
pub(super) fn solve<T: Elem>(a: &Mat<T>, b: &Mat<T>) -> Result<Mat<T>, String> {
    if a.rows != b.rows {
        return Err(format!("operator \\: nonconformant arguments (op1 is {}x{}, op2 is {}x{})", a.rows, a.cols, b.rows, b.cols));
    }
    if a.is_empty() || b.is_empty() {
        return Ok(Mat::zeros(a.cols, b.cols));
    }
    if a.rows == a.cols {
        // ponytail: exactly singular A falls through to least squares, Octave warns and returns Inf
        if let Some(x) = na(a).lu().solve(&na(b)) {
            return Ok(mat(&x));
        }
    }
    if !a.data.iter().chain(&b.data).all(|x| x.is_finite()) {
        return Ok(Mat::filled(a.cols, b.cols, T::from_real(f64::NAN)));
    }
    let svd = svd_of("mldivide", a)?;
    let tol = tol_of(a, &svd.1, None)?;
    Ok(mat(&(pinv_of(svd, tol) * na(b))))
}

/// Eigenvalues via Schur. Real input uses the real Schur form, so real eigenvalues come out
/// exactly real and complex ones as exact conjugate pairs.
pub(super) fn eigvals(m: &CMatrix) -> Result<Vec<C64>, String> {
    finite("eig", m)?;
    if m.is_empty() {
        return Ok(vec![]);
    }
    let fail = || "eig: did not converge".to_string();
    if m.is_real() {
        let s = na(&m.re()).try_schur(f64::EPSILON, 10_000).ok_or_else(fail)?;
        return Ok(s.complex_eigenvalues().iter().copied().collect());
    }
    let s = na(m).try_schur(f64::EPSILON, 10_000).ok_or_else(fail)?;
    Ok(s.eigenvalues().ok_or_else(fail)?.iter().copied().collect())
}

/// Inverse of a square matrix; all Inf when singular (Octave's result, minus the warning).
pub(super) fn inverse<T: Elem>(a: &Mat<T>) -> Mat<T> {
    na(a).try_inverse().map_or_else(|| Mat::filled(a.rows, a.cols, T::from_real(f64::INFINITY)), |x| mat(&x))
}

/// Matrix exponential: scaling and squaring with a degree-6 Padé approximant.
pub(super) fn expm<T: Elem>(a: &Mat<T>) -> Mat<T> {
    let n = a.rows;
    let norm = (0..n).map(|i| (0..n).map(|j| a.get(i, j).modulus()).sum::<f64>()).fold(0.0, f64::max);
    let s = if norm > 0.0 { (norm.log2().floor() as i32 + 2).max(0) } else { 0 };
    let a = na(a).map(|x| x.unscale(2f64.powi(s)));
    let id = DMatrix::<T>::identity(n, n);
    let sc = |m: &DMatrix<T>, c: f64| m.map(|x| x.scale(c));
    let (mut x, mut c, q) = (a.clone(), 0.5, 6);
    let (mut e, mut d) = (&id + sc(&a, c), &id - sc(&a, c));
    for k in 2..=q {
        c = c * (q - k + 1) as f64 / (k * (2 * q - k + 1)) as f64;
        x = &a * x;
        e += sc(&x, c);
        if k % 2 == 0 { d += sc(&x, c) } else { d -= sc(&x, c) }
    }
    let mut e = d.lu().solve(&e).unwrap_or(e);
    for _ in 0..s {
        e = &e * &e;
    }
    mat(&e)
}

/// |x| elementwise, real or complex.
fn absolute(v: &Value) -> Result<Matrix, String> {
    match v {
        Value::Complex(z) => Ok(z.map(|z| z.norm())),
        v => Ok(v.matrix()?.map(f64::abs)),
    }
}

/// p-norm of a vector of absolute values (p may be +-Inf or 0).
fn pnorm(abs: &[f64], p: f64) -> f64 {
    let big = abs.iter().cloned().fold(0.0, f64::max);
    match p {
        f64::INFINITY => big,
        f64::NEG_INFINITY => abs.iter().cloned().fold(f64::INFINITY, f64::min),
        0.0 => abs.iter().filter(|&&x| x != 0.0).count() as f64,
        2.0 if big > 0.0 && big.is_finite() => big * abs.iter().map(|x| (x / big).powi(2)).sum::<f64>().sqrt(),
        _ => abs.iter().map(|x| x.powf(p)).sum::<f64>().powf(1.0 / p),
    }
}

fn norm(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("norm", a, 1, 3)?;
    // only the matrix 2-norm needs the values themselves
    let m = absolute(&a[0])?;
    let abs = &m.data;
    let fro = a.get(1).is_some_and(|v| is_opt(v, "fro"));
    let p = match a.get(1) {
        None => 2.0,
        Some(_) if fro => 2.0,
        Some(Value::Str(s)) if s.eq_ignore_ascii_case("inf") => f64::INFINITY,
        Some(Value::Str(s)) => return Err(format!("norm: unknown norm type '{s}'")),
        Some(v) => v.scalar()?,
    };
    let n = if m.is_empty() {
        0.0
    } else if m.rows == 1 || m.cols == 1 || fro {
        pnorm(abs, p)
    } else {
        let sums = |dim| lanes(&m, dim, |l| vec![l.iter().sum::<f64>()]);
        match p {
            1.0 => sums(1).data.into_iter().fold(0.0, f64::max),
            f64::INFINITY => sums(2).data.into_iter().fold(0.0, f64::max),
            2.0 => match &a[0] {
                Value::Complex(z) => svd_of("norm", z)?.1[0],
                v => svd_of("norm", &v.matrix()?)?.1[0],
            },
            _ => return Err("norm: only p = 1, 2, Inf or 'fro' are supported for matrices".into()),
        }
    };
    num(Matrix::scalar(n))
}

/// dot(x, y[, dim]) = sum(conj(x) .* y).
fn dot(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("dot", a, 2, 3)?;
    fn of<T: Elem>(x: Mat<T>, y: Mat<T>, dim: Option<&Value>) -> R {
        let sum = |l: &[T]| l.iter().fold(T::default(), |s, &v| s + v);
        let prod: Vec<T> = x.data.iter().zip(&y.data).map(|(&p, &q)| p * q).collect();
        if x.is_vector() && y.is_vector() && x.len() == y.len() {
            return Ok(vec![T::value(Mat::scalar(sum(&prod)))]);
        }
        if (x.rows, x.cols) != (y.rows, y.cols) {
            return Err("dot: sizes of X and Y must match".into());
        }
        let prod = Mat::new(x.rows, x.cols, prod);
        let dim = super::dim_of("dot", &prod, dim)?;
        Ok(vec![T::value(lanes(&prod, dim, |l| vec![sum(l)]))])
    }
    if a[..2].iter().any(Value::is_complex) {
        return of(a[0].cmatrix()?.map(|z| z.conj()), a[1].cmatrix()?, a.get(2));
    }
    of(a[0].matrix()?, a[1].matrix()?, a.get(2))
}

fn cross(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("cross", a, 2, 2)?;
    fn of<T: Elem>(x: Mat<T>, y: Mat<T>) -> R {
        if x.len() != 3 || y.len() != 3 {
            return Err("cross: both X and Y must have 3 elements".into()); // ponytail: no 3xN column-wise form
        }
        let (u, v) = (&x.data, &y.data);
        let c = vec![u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        Ok(vec![T::value(Mat::new(x.rows, x.cols, c))])
    }
    if a.iter().any(Value::is_complex) { of(a[0].cmatrix()?, a[1].cmatrix()?) } else { of(a[0].matrix()?, a[1].matrix()?) }
}

fn trace(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("trace", a, 1, 1)?;
    fn of<T: Elem>(m: Mat<T>) -> R {
        if m.rows != m.cols {
            return Err("trace: only valid on square matrix".into());
        }
        Ok(vec![T::value(Mat::scalar((0..m.rows).fold(T::default(), |s, i| s + m.get(i, i))))])
    }
    on(&a[0], of, of)
}

fn rank(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("rank", a, 1, 2)?;
    fn of<T: Elem>(m: Mat<T>, t: Option<&Value>) -> R {
        let s = svd_of("rank", &m)?.1;
        let tol = tol_of(&m, &s, t)?;
        num(Matrix::scalar(s.iter().filter(|&&x| x > tol).count() as f64))
    }
    on(&a[0], |m| of(m, a.get(1)), |z| of(z, a.get(1)))
}

fn det(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("det", a, 1, 1)?;
    fn of<T: Elem>(m: Mat<T>) -> R {
        square("det", &m)?;
        Ok(vec![T::value(Mat::scalar(if m.is_empty() { T::one() } else { na(&m).lu().determinant() }))])
    }
    on(&a[0], of, of)
}

fn inv(interp: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("inv", a, 1, 1)?;
    // (inverse, reciprocal condition estimate)
    fn of<T: Elem>(m: Mat<T>) -> Result<(Value, f64), String> {
        square("inverse", &m)?;
        if m.is_empty() {
            return Ok((T::value(m), f64::INFINITY));
        }
        let x = inverse(&m);
        let norm1 = |m: &Mat<T>| lanes(&m.map(|z| z.modulus()), 1, |l| vec![l.iter().sum::<f64>()]).data.into_iter().fold(0.0, f64::max);
        let rcond = 1.0 / (norm1(&m) * norm1(&x));
        Ok((T::value(x), rcond))
    }
    let (x, rcond) = match &a[0] {
        Value::Complex(z) => of(z.clone())?,
        v => of(v.matrix()?)?,
    };
    if !(rcond >= f64::EPSILON) {
        interp.print("warning: matrix singular to machine precision\n");
    }
    Ok(vec![x, Value::num(if rcond.is_nan() { 0.0 } else { rcond })])
}

fn pinv(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("pinv", a, 1, 2)?;
    fn of<T: Elem>(m: Mat<T>, t: Option<&Value>) -> R {
        let svd = svd_of("pinv", &m)?;
        let tol = tol_of(&m, &svd.1, t)?;
        Ok(vec![T::value(mat(&pinv_of(svd, tol)))])
    }
    on(&a[0], |m| of(m, a.get(1)), |z| of(z, a.get(1)))
}

fn lu(_: &mut Interp, a: &[Value], nargout: usize) -> R {
    nargs("lu", a, 1, 1)?;
    let m = a[0].matrix()?;
    if m.is_empty() {
        return Ok(vec![Value::Num(m.clone()), Value::Num(m.clone()), Value::Num(Matrix::empty())]);
    }
    let lu = na(&m).lu();
    let (mut l, u) = (lu.l(), lu.u());
    if nargout <= 1 {
        // packed form Y = L + U - I, like LAPACK
        let k = m.rows.min(m.cols);
        let y = DMatrix::from_fn(m.rows, m.cols, |i, j| if i > j && j < k { l[(i, j)] } else if i < k { u[(i, j)] } else { 0.0 });
        return num(mat(&y));
    }
    if nargout == 2 {
        lu.p().inv_permute_rows(&mut l);
        return Ok(vec![Value::Num(mat(&l)), Value::Num(mat(&u))]);
    }
    let mut p = DMatrix::<f64>::identity(m.rows, m.rows);
    lu.p().permute_rows(&mut p);
    Ok(vec![Value::Num(mat(&l)), Value::Num(mat(&u)), Value::Num(mat(&p))])
}

fn qr(_: &mut Interp, a: &[Value], nargout: usize) -> R {
    nargs("qr", a, 1, 2)?;
    let m = a[0].matrix()?;
    let econ = a.get(1).is_some_and(|v| is_opt(v, "econ") || v.scalar().is_ok_and(|x| x == 0.0));
    if m.is_empty() {
        return Ok(vec![Value::Num(eye(m.rows)), Value::Num(m)]);
    }
    let (q, r) = if econ || m.rows <= m.cols {
        let d = na(&m).qr();
        (d.q(), d.r())
    } else {
        // full Q: QR of [A I] shares A's reflectors, its first n columns of R are A's R
        let aug = DMatrix::from_fn(m.rows, m.cols + m.rows, |i, j| if j < m.cols { m.get(i, j) } else { (i + m.cols == j) as u8 as f64 });
        let d = aug.qr();
        (d.q(), d.r().columns(0, m.cols).into_owned())
    };
    if nargout <= 1 {
        return num(mat(&r));
    }
    Ok(vec![Value::Num(mat(&q)), Value::Num(mat(&r))])
}

/// Upper R with R'R = A (upper triangle of A is used), or the 1-based failing column.
fn cholesky(a: &Matrix) -> Result<Matrix, usize> {
    let n = a.rows;
    let mut r = Matrix::zeros(n, n);
    for j in 0..n {
        let s = a.get(j, j) - (0..j).map(|k| r.get(k, j).powi(2)).sum::<f64>();
        if !(s > 0.0) {
            return Err(j + 1);
        }
        r.set(j, j, s.sqrt());
        for i in j + 1..n {
            let t = a.get(j, i) - (0..j).map(|k| r.get(k, j) * r.get(k, i)).sum::<f64>();
            r.set(j, i, t / r.get(j, j));
        }
    }
    Ok(r)
}

fn chol(_: &mut Interp, a: &[Value], nargout: usize) -> R {
    nargs("chol", a, 1, 1)?;
    let m = a[0].matrix()?;
    square("chol", &m)?;
    match cholesky(&m) {
        Ok(r) => Ok(vec![Value::Num(r), Value::num(0.0)]),
        Err(p) if nargout >= 2 => {
            let q = p - 1;
            let full = cholesky(&Matrix::new(q, q, (0..q * q).map(|k| m.get(k % q, k / q)).collect())).unwrap_or(Matrix::empty());
            Ok(vec![Value::Num(full), Value::num(p as f64)])
        }
        Err(_) => Err("chol: input matrix must be positive definite".into()),
    }
}

fn eig(_: &mut Interp, a: &[Value], nargout: usize) -> R {
    if a.len() == 2 {
        return Err("eig: generalized eigenvalue problem is not supported".into());
    }
    nargs("eig", a, 1, 1)?;
    let m = a[0].cmatrix()?;
    square("eig", &m)?;
    finite("eig", &m)?;
    let n = m.rows;
    if n == 0 {
        return Ok(vec![Value::Num(Matrix::zeros(0, 1)), Value::Num(Matrix::empty())]);
    }
    let big = m.data.iter().fold(0.0f64, |s, z| s.max(z.norm()));
    if (0..n).all(|i| (0..=i).all(|j| (m.get(i, j) - m.get(j, i).conj()).norm() <= 1e-14 * big)) {
        return if m.is_real() { hermitian_eig(&m.re(), nargout) } else { hermitian_eig(&m, nargout) };
    }
    // sorted by real part, +imag first, so conjugate pairs stay adjacent
    let mut vals = eigvals(&m)?;
    vals.sort_by(|x, y| x.re.total_cmp(&y.re).then(y.im.total_cmp(&x.im)));
    if nargout <= 1 {
        return Ok(vec![Value::complex(Mat::col(vals))]);
    }
    // eigenvector = null vector of A - λI; repeated λ take the next-smallest singular vectors
    // while those are null too, a defective λ reuses its null vector (dependent columns, like LAPACK)
    let mut vecs = CMatrix::zeros(n, n);
    for (j, &l) in vals.iter().enumerate() {
        let seen = vals[..j].iter().filter(|&&x| (x - l).norm() <= 1e-8 * (1.0 + l.norm())).count();
        let mut shifted = m.clone();
        (0..n).for_each(|i| shifted.set(i, i, m.get(i, i) - l));
        let pick = |s: &[f64]| {
            let k = n - 1 - seen.min(n - 1);
            if s[k] <= 1e-8 * s[0] { k } else { n - 1 }
        };
        // a real A - λI keeps its eigenvector exactly real
        let v: Vec<C64> = if shifted.is_real() {
            let (_, s, v) = svd_of("eig", &shifted.re())?;
            v.column(pick(&s)).iter().map(|&x| C64::new(x, 0.0)).collect()
        } else {
            let (_, s, v) = svd_of("eig", &shifted)?;
            v.column(pick(&s)).iter().copied().collect()
        };
        // unit norm already; rotate so the (first) largest component is real and positive
        let big = v.iter().fold(0.0f64, |b, z| b.max(z.norm()));
        let top = v.iter().copied().find(|z| z.norm() >= big * (1.0 - 1e-12)).unwrap_or_default();
        let phase = top.conj().unscale(top.norm());
        (0..n).for_each(|i| vecs.set(i, j, v[i] * phase));
    }
    Ok(vec![Value::complex(vecs), Value::complex(diag_of(&vals))])
}

/// Symmetric / Hermitian: real eigenvalues ascending, orthonormal eigenvectors.
fn hermitian_eig<T: Elem>(m: &Mat<T>, nargout: usize) -> R {
    let n = m.rows;
    let e = na(m).try_symmetric_eigen(f64::EPSILON, 10_000).ok_or("eig: did not converge")?;
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&i, &j| e.eigenvalues[i].total_cmp(&e.eigenvalues[j]));
    let vals: Vec<f64> = idx.iter().map(|&i| e.eigenvalues[i]).collect();
    if nargout <= 1 {
        return num(Matrix::col(vals));
    }
    let vecs = DMatrix::from_fn(n, n, |i, j| e.eigenvectors[(i, idx[j])]);
    Ok(vec![T::value(mat(&vecs)), Value::Num(diag_of(&vals))])
}

fn svd(_: &mut Interp, a: &[Value], nargout: usize) -> R {
    nargs("svd", a, 1, 2)?;
    let econ = a.get(1).is_some_and(|v| is_opt(v, "econ") || v.scalar().is_ok_and(|x| x == 0.0));
    fn of<T: Elem>(m: Mat<T>, nargout: usize, econ: bool) -> R {
        let (u, s, v) = svd_of("svd", &m)?;
        if nargout <= 1 {
            return num(Matrix::col(s));
        }
        let (u, v) = if econ { (u, v) } else { (complete(&u), complete(&v)) };
        let mut sm = Matrix::zeros(u.ncols(), v.ncols());
        s.iter().enumerate().for_each(|(i, &x)| sm.set(i, i, x));
        Ok(vec![T::value(mat(&u)), Value::Num(sm), T::value(mat(&v))])
    }
    on(&a[0], |m| of(m, nargout, econ), |z| of(z, nargout, econ))
}

fn rank_tol<T>(m: &Mat<T>, s: &[f64]) -> usize {
    let tol = tol_of(m, s, None).unwrap_or(0.0);
    s.iter().filter(|&&x| x > tol).count()
}

fn null(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("null", a, 1, 1)?;
    fn of<T: Elem>(m: Mat<T>) -> R {
        let (_, s, v) = svd_of("null", &m)?;
        let v = complete(&v);
        let r = rank_tol(&m, &s);
        let out = mat(&v.columns(r, m.cols - r).into_owned());
        Ok(vec![T::value(out.map(|x| if x.modulus() < f64::EPSILON { T::zero() } else { x }))])
    }
    on(&a[0], of, of)
}

fn orth(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("orth", a, 1, 1)?;
    fn of<T: Elem>(m: Mat<T>) -> R {
        let (u, s, _) = svd_of("orth", &m)?;
        Ok(vec![T::value(mat(&u.columns(0, rank_tol(&m, &s)).into_owned()))])
    }
    on(&a[0], of, of)
}

/// [R, k] = rref(A[, tol]): Gauss-Jordan with partial pivoting (Octave's rref.m); k lists the pivot columns.
fn rref(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("rref", a, 1, 2)?;
    fn of<T: Elem>(mut m: Mat<T>, tol: Option<&Value>) -> R {
        let (rows, cols) = (m.rows, m.cols);
        let tol = match tol {
            Some(t) => t.scalar()?,
            None => f64::EPSILON * rows.max(cols) as f64 * lanes(&m.map(|x| x.modulus()), 2, |l| vec![l.iter().sum()]).data.into_iter().fold(0.0, f64::max),
        };
        let (mut used, mut r) = (vec![], 0);
        for c in 0..cols {
            if r == rows {
                break;
            }
            let (p, big) = (r..rows).map(|i| (i, m.get(i, c).modulus())).fold((r, -1.0), |b, x| if x.1 > b.1 { x } else { b });
            if big <= tol {
                (r..rows).for_each(|i| m.set(i, c, T::zero()));
                continue;
            }
            used.push(c as f64 + 1.0);
            for j in c..cols {
                let t = m.get(p, j);
                m.set(p, j, m.get(r, j));
                m.set(r, j, t);
            }
            let piv = m.get(r, c);
            (c..cols).for_each(|j| m.set(r, j, m.get(r, j) / piv));
            for i in (0..rows).filter(|&i| i != r) {
                let f = m.get(i, c);
                (c..cols).for_each(|j| m.set(i, j, m.get(i, j) - f * m.get(r, j)));
            }
            r += 1;
        }
        Ok(vec![T::value(m), Value::Num(Matrix::row(used))])
    }
    on(&a[0], |m| of(m, a.get(1)), |z| of(z, a.get(1)))
}

/// cond(A) = s_max / s_min (any shape); cond(A, p) = norm(A, p) * norm(inv(A), p).
fn cond(interp: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("cond", a, 1, 2)?;
    if let Some(p) = a.get(1).filter(|p| p.scalar().map_or(true, |p| p != 2.0)) {
        let ai = first(inv(interp, &a[..1], 1)?)?;
        let x = first(norm(interp, &[a[0].clone(), p.clone()], 1)?)?.scalar()?;
        return num(Matrix::scalar(x * first(norm(interp, &[ai, p.clone()], 1)?)?.scalar()?));
    }
    fn of<T: Elem>(m: Mat<T>) -> R {
        let s = svd_of("cond", &m)?.1;
        num(Matrix::scalar(match (s.first(), s.last()) {
            (Some(hi), Some(lo)) => hi / lo,
            _ => 0.0,
        }))
    }
    on(&a[0], of, of)
}

/// [P, H] = hess(A): A = P*H*P' with H upper Hessenberg.
fn hess(_: &mut Interp, a: &[Value], nargout: usize) -> R {
    nargs("hess", a, 1, 1)?;
    fn of<T: Elem>(m: Mat<T>, nargout: usize) -> R {
        square("hess", &m)?;
        finite("hess", &m)?;
        if m.is_empty() {
            return Ok(vec![T::value(m.clone()), T::value(m)]);
        }
        let (p, h) = na(&m).hessenberg().unpack();
        let h = T::value(mat(&h));
        Ok(if nargout <= 1 { vec![h] } else { vec![T::value(mat(&p)), h] })
    }
    on(&a[0], |m| of(m, nargout), |z| of(z, nargout))
}

/// [U, S] = schur(A[, "real" | "complex"]): A = U*S*U'; real input gives the real (quasi-triangular) form.
fn schur(_: &mut Interp, a: &[Value], nargout: usize) -> R {
    nargs("schur", a, 1, 2)?;
    let complex = match a.get(1) {
        None => false,
        Some(v) if is_opt(v, "complex") => true,
        Some(v) if is_opt(v, "real") => false,
        Some(_) => return Err("schur: second argument must be \"real\" or \"complex\"".into()),
    };
    fn of<T: Elem>(m: Mat<T>, nargout: usize) -> R {
        square("schur", &m)?;
        finite("schur", &m)?;
        if m.is_empty() {
            return Ok(vec![T::value(m.clone()), T::value(m)]);
        }
        let (u, mut s) = na(&m).try_schur(f64::EPSILON, 10_000).ok_or("schur: did not converge")?.unpack();
        // below the (block) diagonal is zero by construction; drop the round-off nalgebra leaves there
        for j in 0..m.cols {
            for i in j + 2..m.rows {
                s[(i, j)] = T::zero();
            }
        }
        let s = T::value(mat(&s));
        Ok(if nargout <= 1 { vec![s] } else { vec![T::value(mat(&u)), s] })
    }
    if complex || a[0].is_complex() { of(a[0].cmatrix()?, nargout) } else { of(a[0].matrix()?, nargout) }
}

/// f(A) = Q f(T) Q' through the complex Schur form A = Q T Q'. Real input whose result is real
/// up to round-off comes back real.
fn schur_fn(name: &str, a: &[Value], f: fn(&DMatrix<C64>) -> Result<DMatrix<C64>, String>) -> R {
    nargs(name, a, 1, 1)?;
    let m = a[0].cmatrix()?;
    square(name, &m)?;
    finite(name, &m)?;
    if m.is_empty() {
        return Ok(vec![Value::Num(Matrix::empty())]);
    }
    let (q, t) = na(&m).try_schur(f64::EPSILON, 10_000).ok_or_else(|| format!("{name}: did not converge"))?.unpack();
    let x = mat(&(&q * f(&t).map_err(|e| format!("{name}: {e}"))? * q.adjoint()));
    let big = x.data.iter().fold(0.0f64, |b, z| b.max(z.norm()));
    // ponytail: 1e-10 relative imaginary round-off threshold; a real-Schur algorithm would make it exact
    if m.is_real() && x.data.iter().all(|z| z.im.abs() <= 1e-10 * big) {
        return num(x.re());
    }
    Ok(vec![Value::complex(x)])
}

/// Principal square root of an upper triangular matrix (Björck–Hammarling recurrence).
fn sqrt_tri(t: &DMatrix<C64>) -> Result<DMatrix<C64>, String> {
    let n = t.nrows();
    let mut r = DMatrix::<C64>::zeros(n, n);
    for j in 0..n {
        r[(j, j)] = t[(j, j)].sqrt();
        for i in (0..j).rev() {
            let s = t[(i, j)] - (i + 1..j).map(|k| r[(i, k)] * r[(k, j)]).sum::<C64>();
            r[(i, j)] = if s == C64::default() { s } else { s / (r[(i, i)] + r[(j, j)]) };
        }
    }
    Ok(r)
}

/// Principal logarithm of an upper triangular matrix: inverse scaling and squaring, i.e. square
/// roots until T is near I, then the series of log(I + X), times 2^k.
fn log_tri(t: &DMatrix<C64>) -> Result<DMatrix<C64>, String> {
    let n = t.nrows();
    if t.diagonal().iter().any(|z| z.norm() == 0.0) {
        return Err("matrix is singular, its logarithm is not finite".into());
    }
    let id = DMatrix::<C64>::identity(n, n);
    let (mut t, mut k) = (t.clone(), 0);
    while (&t - &id).iter().map(|z| z.norm()).fold(0.0, f64::max) * n as f64 > 0.25 && k < 64 {
        t = sqrt_tri(&t)?;
        k += 1;
    }
    let x = &t - &id;
    let (mut term, mut sum) = (x.clone(), x.clone());
    for p in 2..=40 {
        term = &term * &x;
        sum += term.map(|z| z.scale(if p % 2 == 0 { -1.0 } else { 1.0 } / p as f64));
    }
    Ok(sum.map(|z| z.scale(2f64.powi(k))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(f: Builtin, args: &[Value], nargout: usize) -> Vec<Matrix> {
        let mut interp = Interp::new();
        f(&mut interp, args, nargout).unwrap().iter().map(|v| v.matrix().unwrap()).collect()
    }

    fn close(a: &Matrix, b: &Matrix, tol: f64) -> bool {
        (a.rows, a.cols) == (b.rows, b.cols) && a.data.iter().zip(&b.data).all(|(x, y)| (x - y).abs() <= tol)
    }

    fn mm(a: &Matrix, b: &Matrix) -> Matrix {
        super::super::ops::matmul(a, b).unwrap()
    }

    #[test]
    fn solve_det_inv() {
        let a = Matrix::from_rows(&[vec![4., -2., 1.], vec![-2., 4., -2.], vec![1., -2., 4.]]);
        let b = Matrix::col(vec![11., -16., 17.]);
        let x = solve(&a, &b).unwrap();
        assert!(close(&x, &Matrix::col(vec![1., -2., 3.]), 1e-12));
        assert!((call(det, &[a.clone().into()], 1)[0].data[0] - 36.0).abs() < 1e-12);
        let ai = call(inv, &[a.clone().into()], 1).remove(0);
        assert!(close(&mm(&a, &ai), &eye(3), 1e-12));
        // least squares: fit y = 1 + 2x through exact points
        let a = Matrix::from_rows(&[vec![1., 0.], vec![1., 1.], vec![1., 2.]]);
        let x = solve(&a, &Matrix::col(vec![1., 3., 5.])).unwrap();
        assert!(close(&x, &Matrix::col(vec![1., 2.]), 1e-12));
    }

    #[test]
    fn decompositions() {
        let a = Matrix::from_rows(&[vec![2., 1., 1.], vec![4., -6., 0.], vec![-2., 7., 2.]]);
        let r = call(lu, &[a.clone().into()], 3);
        assert!(close(&mm(&r[0], &r[1]), &mm(&r[2], &a), 1e-12));
        let r = call(lu, &[a.clone().into()], 2);
        assert!(close(&mm(&r[0], &r[1]), &a, 1e-12));
        let t = Matrix::from_rows(&[vec![1., 2.], vec![3., 4.], vec![5., 6.]]);
        let r = call(qr, &[t.clone().into()], 2);
        assert_eq!((r[0].rows, r[0].cols, r[1].rows, r[1].cols), (3, 3, 3, 2));
        assert!(close(&mm(&r[0], &r[1]), &t, 1e-12));
        assert!(close(&mm(&r[0].transpose(), &r[0]), &eye(3), 1e-12));
        assert!(r[1].get(1, 0).abs() < 1e-12 && r[1].get(2, 1).abs() < 1e-12);
        let s = Matrix::from_rows(&[vec![4., 2.], vec![2., 3.]]);
        let c = call(chol, &[s.clone().into()], 1).remove(0);
        assert!(close(&mm(&c.transpose(), &c), &s, 1e-12) && c.get(1, 0) == 0.0);
        let svd3 = call(svd, &[t.clone().into()], 3);
        assert!(close(&mm(&mm(&svd3[0], &svd3[1]), &svd3[2].transpose()), &t, 1e-12));
        assert_eq!(call(rank, &[Matrix::from_rows(&[vec![1., 2.], vec![2., 4.]]).into()], 1)[0].data[0], 1.0);
        let n = call(null, &[Matrix::from_rows(&[vec![1., 1.]]).into()], 1).remove(0);
        assert!(n.cols == 1 && (n.data[0] + n.data[1]).abs() < 1e-12);
    }

    #[test]
    fn eigen() {
        let s = Matrix::from_rows(&[vec![2., 1.], vec![1., 2.]]);
        assert!(close(&call(eig, &[s.clone().into()], 1)[0], &Matrix::col(vec![1., 3.]), 1e-12));
        let vd = call(eig, &[s.clone().into()], 2);
        assert!(close(&mm(&s, &vd[0]), &mm(&vd[0], &vd[1]), 1e-12));
        let ns = Matrix::from_rows(&[vec![2., 0.], vec![1., 3.]]);
        let vd = call(eig, &[ns.clone().into()], 2);
        assert!(close(&vd[1], &Matrix::from_rows(&[vec![2., 0.], vec![0., 3.]]), 1e-10));
        assert!(close(&mm(&ns, &vd[0]), &mm(&vd[0], &vd[1]), 1e-10));
        let e = expm(&Matrix::from_rows(&[vec![0., 1.], vec![0., 0.]]));
        assert!(close(&e, &Matrix::from_rows(&[vec![1., 1.], vec![0., 1.]]), 1e-14));
        assert!((expm(&Matrix::scalar(1.0)).data[0] - std::f64::consts::E).abs() < 1e-14);
    }

    fn ccall(f: Builtin, args: &[Value], nargout: usize) -> Vec<CMatrix> {
        f(&mut Interp::new(), args, nargout).unwrap().iter().map(|v| v.cmatrix().unwrap()).collect()
    }

    fn cclose(a: &CMatrix, b: &CMatrix, tol: f64) -> bool {
        (a.rows, a.cols) == (b.rows, b.cols) && a.data.iter().zip(&b.data).all(|(x, y)| (x - y).norm() <= tol)
    }

    fn cmm(a: &CMatrix, b: &CMatrix) -> CMatrix {
        mat(&(na(a) * na(b)))
    }

    fn cx(re: &[Vec<f64>], im: &[Vec<f64>]) -> CMatrix {
        CMatrix::from_parts(&Matrix::from_rows(re), &Matrix::from_rows(im))
    }

    #[test]
    fn complex_eigen() {
        let i = C64::new(0., 1.);
        let rot = Matrix::from_rows(&[vec![0., 1.], vec![-1., 0.]]);
        let e = ccall(eig, &[rot.clone().into()], 1).remove(0);
        assert!(cclose(&e, &Mat::col(vec![i, -i]), 1e-15));
        let vd = ccall(eig, &[rot.clone().into()], 2);
        assert!(cclose(&cmm(&rot.to_complex(), &vd[0]), &cmm(&vd[0], &vd[1]), 1e-12));
        assert!(vd[0].data.iter().any(|z| z.im != 0.0));
        // non-Hermitian complex matrix, then a defective one (double eigenvalue 2, one eigenvector)
        let a = cx(&[vec![1., 2.], vec![0., 3.]], &[vec![1., 0.], vec![0., 0.]]);
        let vd = ccall(eig, &[Value::complex(a.clone())], 2);
        assert!(cclose(&vd[1], &cx(&[vec![1., 0.], vec![0., 3.]], &[vec![1., 0.], vec![0., 0.]]), 1e-12));
        assert!(cclose(&cmm(&a, &vd[0]), &cmm(&vd[0], &vd[1]), 1e-12));
        let a = cx(&[vec![1., 2.], vec![0., 3.]], &[vec![1., 0.], vec![1., -1.]]);
        let vd = ccall(eig, &[Value::complex(a.clone())], 2);
        assert!(cclose(&cmm(&a, &vd[0]), &cmm(&vd[0], &vd[1]), 1e-7));
        // Hermitian: real eigenvalues 2 -+ sqrt(2), unitary vectors
        let h = cx(&[vec![2., 1.], vec![1., 2.]], &[vec![0., 1.], vec![-1., 0.]]);
        let r = eig(&mut Interp::new(), &[Value::complex(h.clone())], 2).unwrap();
        assert!(matches!(&r[1], Value::Num(d) if close(d, &Matrix::from_rows(&[vec![2. - 2f64.sqrt(), 0.], vec![0., 2. + 2f64.sqrt()]]), 1e-12)));
        let (v, d) = (r[0].cmatrix().unwrap(), r[1].cmatrix().unwrap());
        assert!(cclose(&cmm(&h, &v), &cmm(&v, &d), 1e-12));
        // real eigenvalues of a real non-symmetric matrix stay real, vectors too
        let r = eig(&mut Interp::new(), &[Matrix::from_rows(&[vec![2., 0.], vec![1., 3.]]).into()], 2).unwrap();
        assert!(r.iter().all(|v| matches!(v, Value::Num(_))));
    }

    #[test]
    fn complex_matrices() {
        // [1+i 2; 3 4-i]: det = (1+i)(4-i) - 6 = -1 + 3i
        let a = cx(&[vec![1., 2.], vec![3., 4.]], &[vec![1., 0.], vec![0., -1.]]);
        let av = Value::complex(a.clone());
        assert!(cclose(&ccall(det, &[av.clone()], 1)[0], &Mat::scalar(C64::new(-1., 3.)), 1e-12));
        let ai = ccall(inv, &[av.clone()], 1).remove(0);
        let id = eye(2).to_complex();
        assert!(cclose(&cmm(&a, &ai), &id, 1e-12));
        assert!(cclose(&ccall(pinv, &[av.clone()], 1)[0], &ai, 1e-12));
        assert!(cclose(&solve(&a, &id).unwrap(), &ai, 1e-12));
        assert_eq!(call(rank, &[av.clone()], 1)[0].data, vec![2.]);
        assert_eq!(call(trace, &[av.clone()], 1)[0].data, vec![5.]);
        assert!((call(norm, &[av.clone(), "fro".into()], 1)[0].data[0] - 32f64.sqrt()).abs() < 1e-12);
        let usv = ccall(svd, &[av.clone()], 3);
        assert!(cclose(&cmm(&cmm(&usv[0], &usv[1]), &usv[2].ctranspose()), &a, 1e-12));
        let s = call(svd, &[av.clone()], 1).remove(0);
        assert!((s.data[0] - call(norm, &[av], 1)[0].data[0]).abs() < 1e-12 && s.data[1] > 0.0);
        // dot conjugates its first argument
        let v = Value::complex(Mat::row(vec![C64::new(0., 1.), C64::new(2., 0.)]));
        assert_eq!(call(dot, &[v.clone(), v.clone()], 1)[0].data, vec![5.]);
        assert_eq!(call(norm, &[v], 1)[0].data, vec![5f64.sqrt()]);
    }

    /// Run Octave code; each listed variable must be a real scalar below tol (residual norms).
    fn residuals(src: &str, names: &[&str], tol: f64) -> Interp {
        let mut it = Interp::new();
        it.eval(src).unwrap_or_else(|e| panic!("{e}"));
        for n in names {
            let r = it.vars[*n].scalar().unwrap_or_else(|e| panic!("{n}: {e}"));
            assert!(r.abs() <= tol, "{n} = {r}");
        }
        it
    }

    #[test]
    fn matrix_functions() {
        let it = residuals(
            "A = [4 1; 2 3]; B = [1 2; 3 4]; [R, k] = rref([1 2 3; 4 5 6; 7 8 10]); [R2, k2] = rref([1 2 1; 2 4 0]);
             r1 = norm(R - eye(3)); r2 = norm(R2 - [1 2 0; 0 0 1]); r3 = norm(rref([1i 2; 1 -2i]) - [1 -2i; 0 0]);
             c1 = cond(B) - 14.933034373659268; c2 = cond(B, 1) - 21; c3 = cond(eye(3)) - 1;
             S = sqrtm(A); s1 = norm(S*S - A); T = sqrtm(B); s2 = norm(T*T - B); s3 = norm(sqrtm(zeros(2)));
             L = logm(expm([0.1 0.2; 0.3 0.4])); l1 = norm(L - [0.1 0.2; 0.3 0.4]); l2 = norm(logm(eye(3)));
             l3 = norm(expm(logm(B)) - B); l4 = norm(logm(-eye(2)) - pi*1i*eye(2));
             [P, H] = hess(magic(4)); h1 = norm(P*H*P' - magic(4)); h2 = norm(H(3:4, 1)) + abs(H(4, 2)); h3 = norm(P'*P - eye(4));
             [U, Q] = schur(B); q1 = norm(U*Q*U' - B); q2 = abs(Q(2, 1));
             [U2, Q2] = schur([0 1; -1 0]); q3 = norm(U2*Q2*U2' - [0 1; -1 0]);
             [U3, Q3] = schur([0 1; -1 0], 'complex'); q4 = abs(Q3(2, 1)) + norm(U3*Q3*U3' - [0 1; -1 0]);
             v1 = norm(vecnorm([3 4; 6 8]) - [sqrt(45) sqrt(80)]); v2 = norm(vecnorm([3 4; 6 8], Inf, 2) - [4; 8]);
             v3 = vecnorm([3 4], 1) - 7; v4 = vecnorm([3i 4]) - 5;
             x1 = norm(linsolve(B, [5; 11]) - [1; 2]); o.TRANSA = true; x2 = norm(linsolve(B, [7; 10], o) - [1; 2]);",
            &["r1", "r2", "r3", "c1", "c2", "c3", "s1", "s2", "s3", "l1", "l2", "l3", "l4", "h1", "h2", "h3", "q1", "q2", "q3", "q4", "v1", "v2", "v3", "v4", "x1", "x2"],
            1e-12,
        );
        assert_eq!((it.vars["k"].matrix().unwrap().data, it.vars["k2"].matrix().unwrap().data), (vec![1., 2., 3.], vec![1., 3.]));
        // sqrtm of a matrix with positive eigenvalues is real; of one with a negative eigenvalue, complex
        assert!(matches!(it.vars["S"], Value::Num(_)) && matches!(it.vars["T"], Value::Complex(_)));
        assert!(matches!(it.vars["Q2"], Value::Num(_)) && matches!(it.vars["Q3"], Value::Complex(_)));
        assert!(Interp::new().eval("sqrtm([1 2 3])").is_err() && Interp::new().eval("logm(zeros(2))").is_err());
    }

    #[test]
    fn complex_gaps() {
        residuals(
            "d = norm(diff([1+1i, 3, 2i]) - [2-1i, -3+2i]); g = norm(gradient([1i 2i 4i]) - [1i 1.5i 2i]);
             t = abs(trapz([1i 2i 3i]) - 4i) + norm(cumtrapz([1i 2i]) - [0 1.5i]) + abs(trapz([0 2], [1i 3i]) - 4i);
             c = norm(cross([1i 0 0], [0 1 0]) - [0 0 1i]); l = norm(linspace(0, 1+2i, 3) - [0, 0.5+1i, 1+2i]);
             e = norm(expm([0 1i; 1i 0]) - [cos(1) 1i*sin(1); 1i*sin(1) cos(1)]);
             n = norm([1 1i] * null([1 1i])) + abs(size(null([1 1i]), 2) - 1); o = norm(orth([1i 0; 0 2])' * orth([1i 0; 0 2]) - eye(2));",
            &["d", "g", "t", "c", "l", "e", "n", "o"],
            1e-14,
        );
    }

    #[test]
    fn norms() {
        let v: Value = Matrix::row(vec![3., 4.]).into();
        assert_eq!(call(norm, &[v.clone()], 1)[0].data[0], 5.0);
        assert_eq!(call(norm, &[v.clone(), Value::num(1.)], 1)[0].data[0], 7.0);
        assert_eq!(call(norm, &[v, "inf".into()], 1)[0].data[0], 4.0);
        let a: Value = Matrix::from_rows(&[vec![1., -2.], vec![3., 4.]]).into();
        assert_eq!(call(norm, &[a.clone(), Value::num(1.)], 1)[0].data[0], 6.0);
        assert_eq!(call(norm, &[a.clone(), "inf".into()], 1)[0].data[0], 7.0);
        assert!((call(norm, &[a, "fro".into()], 1)[0].data[0] - 30f64.sqrt()).abs() < 1e-12);
    }
}
