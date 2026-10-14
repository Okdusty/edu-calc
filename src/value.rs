//! Shared value types. CONTRACT: every module builds on this; change only additively.
use std::collections::BTreeMap;
use std::rc::Rc;

use crate::lang::Interp;

/// Builtin function: (interp, args, nargout) -> outputs. Return at least one value
/// when nargout >= 1; returning more than nargout is fine (extras dropped).
pub type Builtin = fn(&mut Interp, &[Value], usize) -> Result<Vec<Value>, String>;

/// Complex scalar (nalgebra's re-export of num-complex; no extra dependency).
pub type C64 = nalgebra::Complex<f64>;
/// Real matrix, column-major (Octave layout). Scalars are 1x1, empty is 0x0.
pub type Matrix = Mat<f64>;
/// Complex matrix, same layout.
pub type CMatrix = Mat<C64>;

#[derive(Clone, Debug, PartialEq)]
pub struct Mat<T> {
    pub rows: usize,
    pub cols: usize,
    pub data: Vec<T>,
}

impl<T: Copy + Default> Mat<T> {
    pub fn new(rows: usize, cols: usize, data: Vec<T>) -> Self {
        assert_eq!(rows * cols, data.len(), "matrix data length");
        Mat { rows, cols, data }
    }
    pub fn zeros(rows: usize, cols: usize) -> Self {
        Mat::new(rows, cols, vec![T::default(); rows * cols])
    }
    pub fn filled(rows: usize, cols: usize, v: T) -> Self {
        Mat::new(rows, cols, vec![v; rows * cols])
    }
    pub fn empty() -> Self {
        Mat::zeros(0, 0)
    }
    pub fn scalar(x: T) -> Self {
        Mat::new(1, 1, vec![x])
    }
    pub fn row(v: Vec<T>) -> Self {
        Mat::new(1, v.len(), v)
    }
    pub fn col(v: Vec<T>) -> Self {
        Mat::new(v.len(), 1, v)
    }
    /// Build from row-major nested rows (all same length).
    pub fn from_rows(rows: &[Vec<T>]) -> Self {
        let r = rows.len();
        let c = rows.first().map_or(0, |x| x.len());
        Mat::new(r, c, (0..r * c).map(|k| rows[k % r][k / r]).collect())
    }
    pub fn get(&self, r: usize, c: usize) -> T {
        self.data[c * self.rows + r]
    }
    pub fn set(&mut self, r: usize, c: usize, v: T) {
        self.data[c * self.rows + r] = v;
    }
    pub fn len(&self) -> usize {
        self.data.len()
    }
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
    pub fn is_scalar(&self) -> bool {
        self.rows == 1 && self.cols == 1
    }
    pub fn is_vector(&self) -> bool {
        (self.rows == 1 || self.cols == 1) && !self.is_empty()
    }
    pub fn map<U: Copy + Default>(&self, f: impl Fn(T) -> U) -> Mat<U> {
        Mat::new(self.rows, self.cols, self.data.iter().map(|&x| f(x)).collect())
    }
    pub fn transpose(&self) -> Self {
        let mut t = Mat::zeros(self.cols, self.rows);
        for r in 0..self.rows {
            for c in 0..self.cols {
                t.set(c, r, self.get(r, c));
            }
        }
        t
    }
}

impl Matrix {
    pub fn to_complex(&self) -> CMatrix {
        self.map(|x| C64::new(x, 0.0))
    }
}

impl CMatrix {
    pub fn from_parts(re: &Matrix, im: &Matrix) -> CMatrix {
        assert_eq!((re.rows, re.cols), (im.rows, im.cols), "complex parts size");
        Mat::new(re.rows, re.cols, re.data.iter().zip(&im.data).map(|(&a, &b)| C64::new(a, b)).collect())
    }
    pub fn re(&self) -> Matrix {
        self.map(|z| z.re)
    }
    pub fn im(&self) -> Matrix {
        self.map(|z| z.im)
    }
    pub fn is_real(&self) -> bool {
        self.data.iter().all(|z| z.im == 0.0)
    }
    /// Conjugate transpose (Octave `'`).
    pub fn ctranspose(&self) -> CMatrix {
        self.transpose().map(|z| z.conj())
    }
}

#[derive(Clone, Debug)]
pub enum Func {
    /// `@sin`, or a user/builtin/addon function referenced by name.
    Named(String),
    /// `@(x) ...` — body owned by the language module.
    Anon(crate::lang::Lambda),
}

#[derive(Clone, Debug)]
pub enum Value {
    /// double array
    Num(Matrix),
    /// complex double array. Build with `Value::complex`, which narrows to Num when every
    /// imaginary part is 0 (Octave semantics), so a Complex value always has some im != 0.
    Complex(CMatrix),
    /// logical array (entries 0/1); matters for logical indexing `x(x > 0)`
    Bool(Matrix),
    /// char row vector
    Str(String),
    /// 1xN cell array. ponytail: 2-D cells skipped.
    Cell(Vec<Value>),
    Struct(BTreeMap<String, Value>),
    Func(Rc<Func>),
}

impl Value {
    pub fn num(x: f64) -> Value {
        Value::Num(Matrix::scalar(x))
    }
    pub fn bool(b: bool) -> Value {
        Value::Bool(Matrix::scalar(b as u8 as f64))
    }
    pub fn empty() -> Value {
        Value::Num(Matrix::empty())
    }

    /// Complex result, narrowed to real when all imaginary parts are 0.
    pub fn complex(m: CMatrix) -> Value {
        if m.is_real() { Value::Num(m.re()) } else { Value::Complex(m) }
    }

    /// Real numeric view: Num/Bool as-is, Str as char codes. Complex is an error, so
    /// real-only code never silently drops imaginary parts; complex-aware code uses `cmatrix`.
    pub fn matrix(&self) -> Result<Matrix, String> {
        match self {
            Value::Num(m) | Value::Bool(m) => Ok(m.clone()),
            Value::Str(s) => Ok(Matrix::row(s.chars().map(|c| c as u32 as f64).collect())),
            Value::Complex(_) => Err("expected real value, got complex".into()),
            v => Err(format!("expected numeric value, got {}", v.type_name())),
        }
    }
    /// Complex numeric view of any numeric value.
    pub fn cmatrix(&self) -> Result<CMatrix, String> {
        match self {
            Value::Complex(m) => Ok(m.clone()),
            v => Ok(v.matrix()?.to_complex()),
        }
    }
    pub fn is_complex(&self) -> bool {
        matches!(self, Value::Complex(_))
    }
    pub fn scalar(&self) -> Result<f64, String> {
        let m = self.matrix()?;
        if m.is_scalar() { Ok(m.data[0]) } else { Err(format!("expected scalar, got {}x{}", m.rows, m.cols)) }
    }
    pub fn as_usize(&self) -> Result<usize, String> {
        let x = self.scalar()?;
        if x >= 0.0 && x.fract() == 0.0 { Ok(x as usize) } else { Err(format!("expected non-negative integer, got {x}")) }
    }
    pub fn as_str(&self) -> Result<&str, String> {
        match self {
            Value::Str(s) => Ok(s),
            v => Err(format!("expected string, got {}", v.type_name())),
        }
    }
    /// Octave truth: non-empty and all elements nonzero.
    pub fn truthy(&self) -> Result<bool, String> {
        let m = self.cmatrix()?;
        Ok(!m.is_empty() && m.data.iter().all(|z| z.re != 0.0 || z.im != 0.0))
    }
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Num(_) | Value::Complex(_) => "double",
            Value::Bool(_) => "logical",
            Value::Str(_) => "char",
            Value::Cell(_) => "cell",
            Value::Struct(_) => "struct",
            Value::Func(_) => "function_handle",
        }
    }
    pub fn size(&self) -> (usize, usize) {
        match self {
            Value::Num(m) | Value::Bool(m) => (m.rows, m.cols),
            Value::Complex(m) => (m.rows, m.cols),
            Value::Str(s) => (1, s.chars().count()),
            Value::Cell(c) => (1, c.len()),
            Value::Struct(_) | Value::Func(_) => (1, 1),
        }
    }
    /// Short workspace summary, e.g. "1x100 double".
    pub fn summary(&self) -> String {
        let (r, c) = self.size();
        let cx = if self.is_complex() { "complex " } else { "" };
        format!("{r}x{c} {cx}{}", self.type_name())
    }
}

impl From<f64> for Value {
    fn from(x: f64) -> Self {
        Value::num(x)
    }
}
impl From<CMatrix> for Value {
    fn from(m: CMatrix) -> Self {
        Value::complex(m)
    }
}
impl From<Matrix> for Value {
    fn from(m: Matrix) -> Self {
        Value::Num(m)
    }
}
impl From<Vec<f64>> for Value {
    fn from(v: Vec<f64>) -> Self {
        Value::Num(Matrix::row(v))
    }
}
impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Str(s.to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add, Sub,
    Mul, Div, LDiv, Pow,     // matrix: *  /  \  ^
    EMul, EDiv, ELDiv, EPow, // element-wise: .*  ./  .\  .^
    Eq, Ne, Lt, Le, Gt, Ge,
    And, Or,                 // element-wise & | (&& || short-circuit lives in the evaluator)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg, Plus, Not,
    Transpose,  // .'
    CTranspose, // '  (conjugate transpose)
}
