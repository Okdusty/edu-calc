//! Operator semantics. The evaluator calls `binop` / `unop` for every operator.
use std::ops::{AddAssign, Mul};

use crate::value::{BinOp, Builtin, C64, CMatrix, Mat, Matrix, UnOp, Value};

/// Function forms of the operators (`plus(a, b)` is `a + b`); `power` lives with the math functions.
pub fn lookup(name: &str) -> Option<Builtin> {
    use BinOp::*;
    let f: Builtin = match name {
        "plus" => |_, a, _| fold("plus", Add, a),
        "times" => |_, a, _| fold("times", EMul, a),
        "mtimes" => |_, a, _| fold("mtimes", Mul, a),
        "and" => |_, a, _| fold("and", And, a),
        "or" => |_, a, _| fold("or", Or, a),
        "minus" => |_, a, _| bin("minus", Sub, a),
        "rdivide" => |_, a, _| bin("rdivide", EDiv, a),
        "ldivide" => |_, a, _| bin("ldivide", ELDiv, a),
        "mrdivide" => |_, a, _| bin("mrdivide", Div, a),
        "mldivide" => |_, a, _| bin("mldivide", LDiv, a),
        "mpower" => |_, a, _| bin("mpower", Pow, a),
        "eq" => |_, a, _| bin("eq", Eq, a),
        "ne" => |_, a, _| bin("ne", Ne, a),
        "lt" => |_, a, _| bin("lt", Lt, a),
        "le" => |_, a, _| bin("le", Le, a),
        "gt" => |_, a, _| bin("gt", Gt, a),
        "ge" => |_, a, _| bin("ge", Ge, a),
        "uminus" => |_, a, _| un("uminus", UnOp::Neg, a),
        "not" => |_, a, _| un("not", UnOp::Not, a),
        _ => return None,
    };
    Some(f)
}

/// a1 op a2 op ... left to right; at least two operands.
fn fold(name: &str, op: BinOp, a: &[Value]) -> Result<Vec<Value>, String> {
    let (x, rest) = a.split_first().filter(|(_, r)| !r.is_empty()).ok_or_else(|| format!("Invalid call to {name}"))?;
    Ok(vec![rest.iter().try_fold(x.clone(), |acc, y| binop(op, &acc, y))?])
}

fn bin(name: &str, op: BinOp, a: &[Value]) -> Result<Vec<Value>, String> {
    match a {
        [x, y] => Ok(vec![binop(op, x, y)?]),
        _ => Err(format!("Invalid call to {name}")),
    }
}

fn un(name: &str, op: UnOp, a: &[Value]) -> Result<Vec<Value>, String> {
    match a {
        [x] => Ok(vec![unop(op, x)?]),
        _ => Err(format!("Invalid call to {name}")),
    }
}

pub fn binop(op: BinOp, a: &Value, b: &Value) -> Result<Value, String> {
    if a.is_complex() || b.is_complex() {
        return cbinop(op, &a.cmatrix()?, &b.cmatrix()?);
    }
    let (x, y) = (a.matrix()?, b.matrix()?);
    use BinOp::*;
    let f: fn(f64, f64) -> f64 = match op {
        Mul if !x.is_scalar() && !y.is_scalar() => return matmul(&x, &y).map(Value::Num),
        Div if !y.is_scalar() => return mrdivide(&x, &y, super::linalg::solve).map(Value::Num),
        LDiv if !x.is_scalar() => return super::linalg::solve(&x, &y).map(Value::Num),
        Pow if !(x.is_scalar() && y.is_scalar()) => return mpower(&x, &y).map(Value::Num),
        // negative base to a fractional power goes complex (Octave); `cpow` keeps the other elements real
        Pow | EPow if x.data.iter().any(|&a| a < 0.0) && y.data.iter().any(|&b| fractional(b)) => {
            return cbinop(op, &x.to_complex(), &y.to_complex());
        }
        Add => |a, b| a + b,
        Sub => |a, b| a - b,
        Mul | EMul => |a, b| a * b,
        Div | EDiv => |a, b| a / b,
        LDiv | ELDiv => |a, b| b / a,
        Pow | EPow => f64::powf,
        Eq => |a, b| (a == b) as u8 as f64,
        Ne => |a, b| (a != b) as u8 as f64,
        Lt => |a, b| (a < b) as u8 as f64,
        Le => |a, b| (a <= b) as u8 as f64,
        Gt => |a, b| (a > b) as u8 as f64,
        Ge => |a, b| (a >= b) as u8 as f64,
        And => |a, b| (a != 0.0 && b != 0.0) as u8 as f64,
        Or => |a, b| (a != 0.0 || b != 0.0) as u8 as f64,
    };
    let m = broadcast(&x, &y, f)?;
    Ok(if matches!(op, Eq | Ne | Lt | Le | Gt | Ge | And | Or) { Value::Bool(m) } else { Value::Num(m) })
}

/// Complex operands (reals promoted). Results narrow back to real when every imaginary part is 0.
fn cbinop(op: BinOp, x: &CMatrix, y: &CMatrix) -> Result<Value, String> {
    use BinOp::*;
    let f: fn(C64, C64) -> C64 = match op {
        Mul if !x.is_scalar() && !y.is_scalar() => return matmul(x, y).map(Value::complex),
        Div if !y.is_scalar() => return mrdivide(x, y, super::linalg::solve).map(Value::complex),
        LDiv if !x.is_scalar() => return super::linalg::solve(x, y).map(Value::complex),
        // a complex exponent becomes NaN and fails mpower's integer check
        Pow if !(x.is_scalar() && y.is_scalar()) => {
            return mpower(x, &y.map(|z| if z.im == 0.0 { z.re } else { f64::NAN })).map(Value::complex);
        }
        Add => |a, b| a + b,
        Sub => |a, b| a - b,
        Mul | EMul => cmul,
        Div | EDiv => cdiv,
        LDiv | ELDiv => |a, b| cdiv(b, a),
        Pow | EPow => cpow,
        Eq | Ne | Lt | Le | Gt | Ge | And | Or => return broadcast(x, y, |a, b| ctest(op, a, b) as u8 as f64).map(Value::Bool),
    };
    broadcast(x, y, f).map(Value::complex)
}

/// Comparisons and logic on complex values. Ordering is by abs, ties by arg (Octave).
fn ctest(op: BinOp, a: C64, b: C64) -> bool {
    let key = |z: C64| (z.norm(), z.arg());
    let nz = |z: C64| z.re != 0.0 || z.im != 0.0;
    match op {
        BinOp::Eq => a == b,
        BinOp::Ne => a != b,
        BinOp::Lt => key(a) < key(b),
        BinOp::Le => key(a) <= key(b),
        BinOp::Gt => key(a) > key(b),
        BinOp::Ge => key(a) >= key(b),
        BinOp::And => nz(a) && nz(b),
        _ => nz(a) || nz(b),
    }
}

fn fractional(b: f64) -> bool {
    b.is_finite() && b.fract() != 0.0
}

/// Real factors scale component-wise like Octave's mixed real/complex ops, so `(Inf + 1i) * 2` is `Inf + 2i`.
fn cmul(a: C64, b: C64) -> C64 {
    if b.im == 0.0 {
        a.scale(b.re)
    } else if a.im == 0.0 {
        b.scale(a.re)
    } else {
        a * b
    }
}

fn cdiv(a: C64, b: C64) -> C64 {
    if b.im == 0.0 { a.unscale(b.re) } else { a / b }
}

/// Element power. Real cases stay real and integer powers stay exact, so results narrow back.
fn cpow(a: C64, b: C64) -> C64 {
    match () {
        _ if a.im == 0.0 && b.im == 0.0 && !(a.re < 0.0 && fractional(b.re)) => C64::new(a.re.powf(b.re), 0.0),
        _ if b.im != 0.0 => a.powc(b),
        _ if b.re == 0.5 => a.sqrt(), // exact `(-1)^0.5 = 0 + 1i`; the polar form leaves 6e-17 in the real part
        _ if b.re.fract() == 0.0 && b.re.abs() < 1e9 => a.powi(b.re as i32),
        _ => a.powf(b.re),
    }
}

/// Element-wise with Octave broadcasting (scalar, row vs column, matching dims).
pub fn broadcast<T: Copy + Default, U: Copy + Default>(x: &Mat<T>, y: &Mat<T>, f: impl Fn(T, T) -> U) -> Result<Mat<U>, String> {
    let dim = |a: usize, b: usize| match (a, b) {
        _ if a == b => Some(a),
        (1, n) | (n, 1) => Some(n),
        _ => None,
    };
    let (Some(r), Some(c)) = (dim(x.rows, y.rows), dim(x.cols, y.cols)) else {
        return Err(format!("operator: nonconformant arguments (op1 is {}x{}, op2 is {}x{})", x.rows, x.cols, y.rows, y.cols));
    };
    let at = |m: &Mat<T>, i: usize, j: usize| m.get(if m.rows == 1 { 0 } else { i }, if m.cols == 1 { 0 } else { j });
    let mut out = Mat::zeros(r, c);
    for j in 0..c {
        for i in 0..r {
            out.set(i, j, f(at(x, i, j), at(y, i, j)));
        }
    }
    Ok(out)
}

pub fn matmul<T: Copy + Default + Mul<Output = T> + AddAssign>(x: &Mat<T>, y: &Mat<T>) -> Result<Mat<T>, String> {
    if x.cols != y.rows {
        return Err(format!("operator *: nonconformant arguments (op1 is {}x{}, op2 is {}x{})", x.rows, x.cols, y.rows, y.cols));
    }
    let mut out = Mat::zeros(x.rows, y.cols);
    for j in 0..y.cols {
        for k in 0..x.cols {
            let b = y.get(k, j);
            for i in 0..x.rows {
                out.data[j * x.rows + i] += x.get(i, k) * b;
            }
        }
    }
    Ok(out)
}

/// `x / y` = `(y.' \ x.').'`, with `solve` doing the `\`.
pub fn mrdivide<T: Copy + Default>(x: &Mat<T>, y: &Mat<T>, solve: fn(&Mat<T>, &Mat<T>) -> Result<Mat<T>, String>) -> Result<Mat<T>, String> {
    if x.cols != y.cols {
        return Err(format!("operator /: nonconformant arguments (op1 is {}x{}, op2 is {}x{})", x.rows, x.cols, y.rows, y.cols));
    }
    Ok(solve(&y.transpose(), &x.transpose())?.transpose())
}

/// Square matrix to an integer power (negative via the inverse), by repeated squaring.
fn mpower<T: super::Elem>(x: &Mat<T>, y: &Matrix) -> Result<Mat<T>, String> {
    if !(x.rows == x.cols && y.is_scalar()) {
        return Err("for x^y, only square matrix arguments are permitted and one argument must be scalar.  Use .^ for elementwise power.".into());
    }
    let p = y.data[0];
    if p.fract() != 0.0 || !p.is_finite() {
        return Err("operator ^: only integer powers of a square matrix are supported".into()); // ponytail: no eig-based A^p
    }
    let mut base = if p < 0.0 { super::linalg::inverse(x) } else { x.clone() };
    let mut acc = Mat::zeros(x.rows, x.rows);
    (0..x.rows).for_each(|i| acc.set(i, i, T::one()));
    let mut n = p.abs() as u64;
    while n > 0 {
        if n & 1 == 1 {
            acc = matmul(&acc, &base)?;
        }
        n >>= 1;
        if n > 0 {
            base = matmul(&base, &base)?;
        }
    }
    Ok(acc)
}

pub fn unop(op: UnOp, a: &Value) -> Result<Value, String> {
    if let Value::Complex(m) = a {
        return Ok(match op {
            UnOp::Transpose => Value::complex(m.transpose()),
            UnOp::CTranspose => Value::complex(m.ctranspose()),
            UnOp::Not => Value::Bool(m.map(|z| (z.re == 0.0 && z.im == 0.0) as u8 as f64)),
            UnOp::Neg => Value::complex(m.map(|z| -z)),
            UnOp::Plus => a.clone(),
        });
    }
    Ok(match (op, a) {
        (UnOp::Transpose | UnOp::CTranspose, Value::Bool(m)) => Value::Bool(m.transpose()),
        (UnOp::Transpose | UnOp::CTranspose, Value::Cell(c)) => Value::Cell(c.clone()), // ponytail: 1-D cells only
        (UnOp::Transpose | UnOp::CTranspose, v) => Value::Num(v.matrix()?.transpose()),
        (UnOp::Not, v) => Value::Bool(v.matrix()?.map(|x| (x == 0.0) as u8 as f64)),
        (UnOp::Neg, v) => Value::Num(v.matrix()?.map(|x| -x)),
        (UnOp::Plus, v) => Value::Num(v.matrix()?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ops() {
        let a = Value::Num(Matrix::from_rows(&[vec![1., 2.], vec![3., 4.]]));
        let r = binop(BinOp::Mul, &a, &a).unwrap().matrix().unwrap();
        assert_eq!(r, Matrix::from_rows(&[vec![7., 10.], vec![15., 22.]]));
        let row = Value::from(vec![10., 20.]);
        let r = binop(BinOp::Add, &a, &row).unwrap().matrix().unwrap();
        assert_eq!(r, Matrix::from_rows(&[vec![11., 22.], vec![13., 24.]]));
        assert!(matches!(binop(BinOp::Gt, &a, &Value::num(2.)).unwrap(), Value::Bool(_)));
    }

    #[test]
    fn functional_forms() {
        let mut it = crate::lang::Interp::new();
        it.eval("A = [1 2; 3 4]; B = [2 0; 1 -1]; z = 1 + 2i;").unwrap();
        let pairs = [
            ("plus(A, B, z)", "A + B + z"), ("minus(A, B)", "A - B"), ("times(A, B)", "A .* B"), ("rdivide(A, B)", "A ./ B"),
            ("ldivide(A, B)", "A .\\ B"), ("mtimes(A, B, A)", "A * B * A"), ("mrdivide(A, B)", "A / B"), ("mldivide(A, z*B)", "A \\ (z*B)"),
            ("mpower(A, 3)", "A ^ 3"), ("power(A, z)", "A .^ z"), ("uminus(z)", "-z"), ("not(B)", "!B"), ("eq(A, B)", "A == B"),
            ("ne(A, 2)", "A != 2"), ("lt(A, B)", "A < B"), ("le(A, B)", "A <= B"), ("gt(z, A)", "z > A"), ("ge(A, B)", "A >= B"),
            ("and(A, B, 1)", "A & B & 1"), ("or(B, 0)", "B | 0"),
        ];
        for (f, op) in pairs {
            it.eval(&format!("p = {f}; q = {op};")).unwrap_or_else(|e| panic!("{f}: {e}"));
            assert_eq!(format!("{:?}", it.vars["p"]), format!("{:?}", it.vars["q"]), "{f}");
        }
        assert!(it.eval("minus(1, 2, 3)").is_err() && it.eval("plus(1)").is_err() && it.eval("not(1, 2)").is_err());
    }

    #[test]
    fn matrix_division_power() {
        let close = |a: &Matrix, b: &Matrix| a.data.iter().zip(&b.data).all(|(x, y)| (x - y).abs() < 1e-12) && a.rows == b.rows && a.cols == b.cols;
        let a = Value::Num(Matrix::from_rows(&[vec![1., 2.], vec![3., 4.]]));
        let b = Value::Num(Matrix::col(vec![5., 11.]));
        let x = binop(BinOp::LDiv, &a, &b).unwrap().matrix().unwrap();
        assert!(close(&x, &Matrix::col(vec![1., 2.])));
        let r = binop(BinOp::Div, &Value::from(vec![5., 11.]), &a).unwrap().matrix().unwrap(); // x*A = b
        assert!(close(&matmul(&r, &a.matrix().unwrap()).unwrap(), &Matrix::row(vec![5., 11.])));
        let a3 = binop(BinOp::Pow, &a, &Value::num(3.)).unwrap().matrix().unwrap();
        assert!(close(&a3, &Matrix::from_rows(&[vec![37., 54.], vec![81., 118.]])));
        let ai = binop(BinOp::Pow, &a, &Value::num(-1.)).unwrap().matrix().unwrap();
        assert!(close(&ai, &Matrix::from_rows(&[vec![-2., 1.], vec![1.5, -0.5]])));
        let i = binop(BinOp::Pow, &a, &Value::num(0.)).unwrap().matrix().unwrap();
        assert!(close(&i, &Matrix::from_rows(&[vec![1., 0.], vec![0., 1.]])));
        assert!(binop(BinOp::Pow, &a, &Value::num(0.5)).is_err());
        assert!(binop(BinOp::LDiv, &a, &Value::Num(Matrix::col(vec![1., 2., 3.]))).is_err());
    }

    fn c(re: f64, im: f64) -> Value {
        Value::complex(CMatrix::scalar(C64::new(re, im)))
    }
    fn cm(v: &Value) -> CMatrix {
        v.cmatrix().unwrap()
    }
    fn near(a: &CMatrix, b: &[C64]) -> bool {
        a.data.len() == b.len() && a.data.iter().zip(b).all(|(x, y)| (x - y).norm() < 1e-12)
    }

    #[test]
    fn complex_arith() {
        let z = c(3., 4.);
        let i = c(0., 1.);
        // i*i narrows to real -1
        assert!(matches!(binop(BinOp::Mul, &i, &i).unwrap(), Value::Num(m) if m.data == [-1.]));
        assert_eq!(cm(&binop(BinOp::Add, &z, &Value::num(1.)).unwrap()).data, [C64::new(4., 4.)]);
        assert!(near(&cm(&binop(BinOp::Div, &z, &i).unwrap()), &[C64::new(4., -3.)]));
        assert_eq!(cm(&binop(BinOp::Div, &c(f64::INFINITY, 1.), &Value::num(2.)).unwrap()).data, [C64::new(f64::INFINITY, 0.5)]);
        // broadcast: complex row + real column
        let r = binop(BinOp::Add, &Value::complex(CMatrix::row(vec![C64::new(0., 1.), C64::new(0., 2.)])), &Value::Num(Matrix::col(vec![10., 20.]))).unwrap();
        assert_eq!((cm(&r).rows, cm(&r).cols), (2, 2));
        assert_eq!(cm(&r).get(1, 1), C64::new(20., 2.));
        // matmul, solve, mrdivide, mpower
        let a = Value::complex(CMatrix::from_rows(&[vec![C64::new(1., 1.), C64::new(2., 0.)], vec![C64::new(0., 0.), C64::new(0., 1.)]]));
        let b = Value::complex(CMatrix::col(vec![C64::new(1., 0.), C64::new(0., 2.)]));
        let x = binop(BinOp::LDiv, &a, &b).unwrap();
        assert!(near(&cm(&binop(BinOp::Mul, &a, &x).unwrap()), &cm(&b).data));
        let bt = unop(UnOp::Transpose, &b).unwrap();
        let y = binop(BinOp::Div, &bt, &a).unwrap(); // y*a = b.'
        assert!(near(&cm(&binop(BinOp::Mul, &y, &a).unwrap()), &cm(&bt).data));
        let a2 = binop(BinOp::Pow, &a, &Value::num(2.)).unwrap();
        assert!(near(&cm(&a2), &cm(&binop(BinOp::Mul, &a, &a).unwrap()).data));
        let ai = binop(BinOp::Pow, &a, &Value::num(-1.)).unwrap();
        assert!(near(&cm(&binop(BinOp::Mul, &a, &ai).unwrap()), &[C64::new(1., 0.), C64::new(0., 0.), C64::new(0., 0.), C64::new(1., 0.)]));
        assert!(binop(BinOp::Pow, &a, &i).is_err());
        // least squares: overdetermined, exact solution exists
        let tall = Value::complex(CMatrix::col(vec![C64::new(1., 0.), C64::new(0., 1.)]));
        let rhs = binop(BinOp::Mul, &tall, &c(2., 3.)).unwrap();
        assert!(near(&cm(&binop(BinOp::LDiv, &tall, &rhs).unwrap()), &[C64::new(2., 3.)]));
    }

    #[test]
    fn complex_powers() {
        let p = binop(BinOp::Pow, &Value::num(-8.), &Value::num(1. / 3.)).unwrap();
        assert!(near(&cm(&p), &[C64::new(1., 3f64.sqrt())]));
        assert!(matches!(binop(BinOp::Pow, &Value::num(-1.), &Value::num(0.5)).unwrap(), Value::Complex(m) if m.data == [C64::new(0., 1.)]));
        // elementwise: only the negative/fractional element goes complex
        let v = binop(BinOp::EPow, &Value::from(vec![-4., 4.]), &Value::num(0.5)).unwrap();
        assert_eq!(cm(&v).data, [C64::new(0., 2.), C64::new(2., 0.)]);
        // real results stay real
        assert!(matches!(binop(BinOp::EPow, &Value::from(vec![-1., 4.]), &Value::from(vec![2., 0.5])).unwrap(), Value::Num(m) if m.data == [1., 2.]));
        assert!(matches!(binop(BinOp::Pow, &Value::num(-2.), &Value::num(3.)).unwrap(), Value::Num(m) if m.data == [-8.]));
        assert!(matches!(binop(BinOp::Pow, &c(1., 1.), &Value::num(2.)).unwrap(), Value::Complex(m) if m.data == [C64::new(0., 2.)]));
        assert!(matches!(binop(BinOp::Pow, &c(0., 1.), &Value::num(4.)).unwrap(), Value::Num(m) if m.data == [1.]));
        // i^i = exp(-pi/2)
        let ii = binop(BinOp::Pow, &c(0., 1.), &c(0., 1.)).unwrap().scalar().unwrap();
        assert!((ii - (-std::f64::consts::FRAC_PI_2).exp()).abs() < 1e-12);
    }

    #[test]
    fn complex_compare_unop() {
        let t = |op, a: &Value, b: &Value| binop(op, a, b).unwrap().matrix().unwrap().data;
        let (z, w) = (c(3., 4.), c(0., 5.));
        assert_eq!(t(BinOp::Eq, &z, &z), [1.]);
        assert_eq!(t(BinOp::Ne, &z, &c(3., -4.)), [1.]);
        assert_eq!(t(BinOp::Eq, &z, &Value::num(3.)), [0.]);
        // equal abs (5): ordered by arg, atan2(4,3) < pi/2
        assert_eq!(t(BinOp::Lt, &z, &w), [1.]);
        assert_eq!(t(BinOp::Ge, &w, &z), [1.]);
        assert_eq!(t(BinOp::Gt, &Value::num(-5.), &w), [1.]); // arg(-5) = pi
        assert_eq!(t(BinOp::Lt, &c(0., 1.), &Value::num(2.)), [1.]);
        assert_eq!(t(BinOp::And, &c(0., 1.), &Value::num(1.)), [1.]);
        assert_eq!(t(BinOp::Or, &c(0., 1.), &Value::num(0.)), [1.]);
        assert!(matches!(unop(UnOp::Not, &z).unwrap(), Value::Bool(m) if m.data == [0.]));
        let row = Value::complex(CMatrix::row(vec![C64::new(1., 2.), C64::new(3., -1.)]));
        assert_eq!(cm(&unop(UnOp::CTranspose, &row).unwrap()), CMatrix::col(vec![C64::new(1., -2.), C64::new(3., 1.)]));
        assert_eq!(cm(&unop(UnOp::Transpose, &row).unwrap()), CMatrix::col(vec![C64::new(1., 2.), C64::new(3., -1.)]));
        assert_eq!(cm(&unop(UnOp::Neg, &row).unwrap()).data[0], C64::new(-1., -2.));
    }
}
