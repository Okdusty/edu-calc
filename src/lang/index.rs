//! Pure value plumbing: ranges, concatenation, indexing, indexed assignment and deletion.
use std::collections::BTreeMap;

use crate::value::{Mat, Matrix, Value};

/// Evaluated assignment accessor.
pub enum AccV {
    Paren(Vec<Value>),
    Brace(Vec<Value>),
    Field(String),
}

pub fn str_of(m: &Matrix) -> String {
    m.data.iter().map(|&x| char::from_u32(x as u32).unwrap_or('?')).collect()
}

fn is_colon(v: &Value) -> bool {
    matches!(v, Value::Str(s) if s == ":")
}

/// `x(1)`, `x(1, 1)`, `x(:)` on a 1x1 thing
fn is_one(idx: &[Value]) -> bool {
    idx.iter().all(|v| is_colon(v) || v.scalar().ok() == Some(1.0))
}

/// `a:s:b` as a row; a small tolerance so 0:0.1:1 has 11 elements.
pub fn range(a: f64, s: f64, b: f64) -> Result<Matrix, String> {
    let n = ((b - a) / s + 1e-10).floor() + 1.0;
    if n.is_nan() || n < 1.0 || s == 0.0 {
        return Ok(Matrix::zeros(1, 0));
    }
    if n > 1e9 {
        return Err("out of memory or dimension too large for Octave's index type".into());
    }
    let clamp = |x: f64| if s > 0.0 { x.min(b) } else { x.max(b) };
    Ok(Matrix::row((0..n as usize).map(|k| clamp(a + k as f64 * s)).collect()))
}

/// Matrix / cell literal: horizontal then vertical concatenation.
pub fn concat(rows: Vec<Vec<Value>>) -> Result<Value, String> {
    if rows.iter().flatten().any(|v| matches!(v, Value::Cell(_))) {
        // ponytail: cells are 1-D, so [c1, c2] and [c1; c2] both append
        let mut out = vec![];
        for v in rows.into_iter().flatten() {
            match v {
                Value::Cell(c) => out.extend(c),
                v => out.push(v),
            }
        }
        return Ok(Value::Cell(out));
    }
    let n = rows.iter().flatten().count();
    if n == 1 && matches!(rows[0][0], Value::Struct(_) | Value::Func(_)) {
        return Ok(rows.into_iter().flatten().next().unwrap_or_else(Value::empty));
    }
    if rows.iter().flatten().any(Value::is_complex) {
        return cat(&rows, Value::cmatrix).map(Value::complex);
    }
    let is_str = rows.iter().flatten().any(|v| matches!(v, Value::Str(_)));
    let is_bool = n > 0 && rows.iter().flatten().all(|v| matches!(v, Value::Bool(_)));
    let m = cat(&rows, Value::matrix)?;
    Ok(if is_str {
        if m.rows > 1 {
            return Err("vertical dimensions mismatch: multi-row char arrays are not supported".into());
        }
        Value::Str(str_of(&m))
    } else if is_bool {
        Value::Bool(m)
    } else {
        Value::Num(m)
    })
}

/// Rows of values, each converted by `conv`, joined horizontally then vertically (empties skipped).
fn cat<T: Copy + Default>(rows: &[Vec<Value>], conv: fn(&Value) -> Result<Mat<T>, String>) -> Result<Mat<T>, String> {
    let mut out: Option<Mat<T>> = None;
    for row in rows {
        let mut acc: Option<Mat<T>> = None;
        for v in row {
            let m = conv(v).map_err(|_| format!("concatenation operator not implemented for '{}' by 'matrix' operations", v.type_name()))?;
            if m.is_empty() {
                continue;
            }
            acc = Some(match acc {
                None => m,
                Some(a) if a.rows != m.rows => {
                    return Err(format!("horizontal dimensions mismatch ({}x{} vs {}x{})", a.rows, a.cols, m.rows, m.cols));
                }
                Some(mut a) => {
                    a.cols += m.cols;
                    a.data.extend(m.data);
                    a
                }
            });
        }
        let Some(m) = acc else { continue };
        out = Some(match out {
            None => m,
            Some(a) if a.cols != m.cols => {
                return Err(format!("vertical dimensions mismatch ({}x{} vs {}x{})", a.rows, a.cols, m.rows, m.cols));
            }
            Some(a) => {
                let mut v = Mat::zeros(a.rows + m.rows, a.cols);
                for j in 0..a.cols {
                    for i in 0..a.rows {
                        v.set(i, j, a.get(i, j));
                    }
                    for i in 0..m.rows {
                        v.set(a.rows + i, j, m.get(i, j));
                    }
                }
                v
            }
        });
    }
    Ok(out.unwrap_or_else(Mat::empty))
}

/// One subscript as 0-based positions; `extent` is what `:` covers.
fn positions(v: &Value, extent: usize) -> Result<Vec<usize>, String> {
    match v {
        _ if is_colon(v) => Ok((0..extent).collect()),
        Value::Bool(m) => Ok(m.data.iter().enumerate().filter(|(_, x)| **x != 0.0).map(|(i, _)| i).collect()),
        v => v
            .matrix()?
            .data
            .iter()
            .map(|&x| match x {
                _ if x >= 1.0 && x.fract() == 0.0 && x < 9e15 => Ok(x as usize - 1),
                _ if x.fract() == 0.0 && x < 1.0 => Err(format!("index ({x}): out of bound; value {x} out of bound {extent}")),
                _ => Err(format!("index ({x}): subscripts must be either integers 1 to (2^63)-1 or logicals")),
            })
            .collect(),
    }
}

fn check(p: &[usize], n: usize, what: impl Fn(usize) -> String) -> Result<(), String> {
    match p.iter().find(|&&k| k >= n) {
        Some(&k) => Err(format!("index ({}): out of bound {n}", what(k + 1))),
        None => Ok(()),
    }
}

pub fn index_matrix<T: Copy + Default>(m: &Mat<T>, idx: &[Value]) -> Result<Mat<T>, String> {
    let dims = format!(" (dimensions are {}x{})", m.rows, m.cols);
    match idx {
        [] => Ok(m.clone()),
        [i] => {
            let p = positions(i, m.len())?;
            check(&p, m.len(), |k| k.to_string()).map_err(|e| e + &dims)?;
            let k = p.len();
            let (ir, ic) = match i {
                Value::Bool(b) if b.rows == 1 => (1, k),
                Value::Bool(_) => (k, 1),
                v => v.size(),
            };
            // A(:) is a column; vector source + vector index keeps the source orientation
            let (r, c) = match () {
                _ if is_colon(i) => (k, 1),
                _ if (ir == 1 || ic == 1) && (m.rows == 1) != (m.cols == 1) => {
                    if m.rows == 1 {
                        (1, k)
                    } else {
                        (k, 1)
                    }
                }
                _ => (ir, ic),
            };
            Ok(Mat::new(r, c, p.iter().map(|&k| m.data[k]).collect()))
        }
        [i, j] => {
            let rp = positions(i, m.rows)?;
            let cp = positions(j, m.cols)?;
            check(&rp, m.rows, |k| format!("{k},_")).map_err(|e| e + &dims)?;
            check(&cp, m.cols, |k| format!("_,{k}")).map_err(|e| e + &dims)?;
            let mut out = Mat::zeros(rp.len(), cp.len());
            for (jj, &c) in cp.iter().enumerate() {
                for (ii, &r) in rp.iter().enumerate() {
                    out.set(ii, jj, m.get(r, c));
                }
            }
            Ok(out)
        }
        [i, j, rest @ ..] if rest.iter().all(|v| is_colon(v) || v.scalar().ok() == Some(1.0)) => index_matrix(m, &[i.clone(), j.clone()]),
        _ => Err("index: only 2-D indexing is supported".into()),
    }
}

/// Cells are 1-D (1xN): positions for `c(...)` / `c{...}`.
pub fn cell_positions(n: usize, idx: &[Value]) -> Result<Vec<usize>, String> {
    let m = Matrix::row((0..n).map(|k| k as f64).collect());
    Ok(index_matrix(&m, idx)?.data.iter().map(|&x| x as usize).collect())
}

/// `v(idx)`
pub fn index(v: &Value, idx: &[Value]) -> Result<Value, String> {
    match v {
        Value::Cell(c) => Ok(Value::Cell(cell_positions(c.len(), idx)?.into_iter().map(|k| c[k].clone()).collect())),
        Value::Struct(_) | Value::Func(_) => {
            cell_positions(1, idx)?;
            Ok(v.clone())
        }
        Value::Num(m) => Ok(Value::Num(index_matrix(m, idx)?)),
        Value::Bool(m) => Ok(Value::Bool(index_matrix(m, idx)?)),
        Value::Str(_) => Ok(Value::Str(str_of(&index_matrix(&v.matrix()?, idx)?))),
        Value::Complex(m) => Ok(Value::complex(index_matrix(m, idx)?)),
    }
}

/// Linear subscript of a 1-D cell: `c(i)` or `c(1, i)`.
fn cell_lin(idx: &[Value]) -> Result<&Value, String> {
    match idx {
        [i] => Ok(i),
        [one, i] if is_colon(one) || one.scalar().ok() == Some(1.0) => Ok(i),
        _ => Err("only 1-D cell arrays (1xN) are supported".into()),
    }
}

/// Grow to r x c with zero padding. Column-major, so keeping the row count (or growing a
/// column vector) just extends the data, amortized O(1) for `x(end+1) = v` loops.
fn resize<T: Copy + Default>(m: &mut Mat<T>, r: usize, c: usize) {
    if r == m.rows || (m.cols == 1 && c == 1) {
        m.data.resize(r * c, T::default());
        (m.rows, m.cols) = (r, c);
        return;
    }
    let mut out = Mat::zeros(r, c);
    for j in 0..m.cols {
        for i in 0..m.rows {
            out.set(i, j, m.get(i, j));
        }
    }
    *m = out;
}

fn nonconformant<T>(k: usize, rhs: &Mat<T>) -> String {
    format!("=: nonconformant arguments (op1 is 1x{k}, op2 is {}x{})", rhs.rows, rhs.cols)
}

fn assign_matrix<T: Copy + Default>(m: &mut Mat<T>, idx: &[Value], rhs: &Mat<T>) -> Result<(), String> {
    let val = |k: usize| if rhs.len() == 1 { rhs.data[0] } else { rhs.data[k] };
    match idx {
        [i] => {
            let n = m.len();
            let p = positions(i, n)?;
            if rhs.len() != 1 && rhs.len() != p.len() {
                return Err(nonconformant(p.len(), rhs));
            }
            if let Some(&mx) = p.iter().max().filter(|&&mx| mx >= n) {
                match (m.rows, m.cols) {
                    _ if n == 0 => *m = Mat::zeros(1, mx + 1),
                    (1, _) => resize(m, 1, mx + 1),
                    (_, 1) => resize(m, mx + 1, 1),
                    _ => return Err("resize: Invalid resizing operation or ambiguous assignment to an out-of-bounds array element".into()),
                };
            }
            for (k, &pos) in p.iter().enumerate() {
                m.data[pos] = val(k);
            }
        }
        [i, j] => {
            // `:` over an empty dimension takes its extent from the right-hand side
            let rp = positions(i, if m.rows == 0 { rhs.rows } else { m.rows })?;
            let cp = positions(j, if m.cols == 0 { rhs.cols } else { m.cols })?;
            if rhs.len() != 1 && rhs.len() != rp.len() * cp.len() {
                return Err(format!("=: nonconformant arguments (op1 is {}x{}, op2 is {}x{})", rp.len(), cp.len(), rhs.rows, rhs.cols));
            }
            let nr = rp.iter().map(|&r| r + 1).max().unwrap_or(0).max(m.rows);
            let nc = cp.iter().map(|&c| c + 1).max().unwrap_or(0).max(m.cols);
            if (nr, nc) != (m.rows, m.cols) {
                resize(m, nr, nc);
            }
            let mut k = 0;
            for &c in &cp {
                for &r in &rp {
                    m.set(r, c, val(k));
                    k += 1;
                }
            }
        }
        [i, j, rest @ ..] if rest.iter().all(|v| is_colon(v) || v.scalar().ok() == Some(1.0)) => assign_matrix(m, &[i.clone(), j.clone()], rhs)?,
        _ => return Err("index: only 2-D indexing is supported".into()),
    }
    Ok(())
}

/// `A(idx) = []`
fn delete<T: Copy + Default>(m: &Mat<T>, idx: &[Value]) -> Result<Mat<T>, String> {
    let keep = |n: usize, p: &[usize]| {
        let mut k = vec![true; n];
        p.iter().filter(|&&i| i < n).for_each(|&i| k[i] = false);
        k
    };
    match idx {
        [i] => {
            let k = keep(m.len(), &positions(i, m.len())?);
            let data: Vec<T> = m.data.iter().zip(&k).filter(|(_, k)| **k).map(|(x, _)| *x).collect();
            Ok(if is_colon(i) && data.is_empty() {
                Mat::empty()
            } else if m.cols == 1 && m.rows != 1 {
                Mat::col(data)
            } else {
                Mat::row(data)
            })
        }
        [i, j] => {
            let (rp, cp) = (positions(i, m.rows)?, positions(j, m.cols)?);
            let all = |p: &[usize], n: usize| (0..n).all(|x| p.contains(&x));
            if all(&rp, m.rows) {
                let k = keep(m.cols, &cp);
                let cols: Vec<usize> = (0..m.cols).filter(|&c| k[c]).collect();
                Ok(Mat::new(m.rows, cols.len(), cols.iter().flat_map(|&c| (0..m.rows).map(move |r| (r, c))).map(|(r, c)| m.get(r, c)).collect()))
            } else if all(&cp, m.cols) {
                let k = keep(m.rows, &rp);
                let rows: Vec<usize> = (0..m.rows).filter(|&r| k[r]).collect();
                let mut out = Mat::zeros(rows.len(), m.cols);
                for c in 0..m.cols {
                    for (ri, &r) in rows.iter().enumerate() {
                        out.set(ri, c, m.get(r, c));
                    }
                }
                Ok(out)
            } else {
                Err("a null assignment can only have one non-colon index".into())
            }
        }
        _ => Err("a null assignment can only have one non-colon index".into()),
    }
}

/// `target(idx) = rhs`, in place.
pub fn assign(target: &mut Value, idx: &[Value], rhs: Value) -> Result<(), String> {
    let del = matches!(&rhs, Value::Num(m) if m.rows == 0 && m.cols == 0);
    match target {
        Value::Cell(c) => {
            let p = positions(cell_lin(idx)?, c.len())?;
            if del {
                let mut k = 0;
                c.retain(|_| {
                    k += 1;
                    !p.contains(&(k - 1))
                });
                return Ok(());
            }
            let vals = match rhs {
                Value::Cell(v) => v,
                v => vec![v],
            };
            if vals.len() != 1 && vals.len() != p.len() {
                return Err(format!("=: nonconformant arguments (op1 is 1x{}, op2 is 1x{})", p.len(), vals.len()));
            }
            if let Some(&mx) = p.iter().max().filter(|&&mx| mx >= c.len()) {
                c.resize(mx + 1, Value::empty());
            }
            for (k, &pos) in p.iter().enumerate() {
                c[pos] = vals[if vals.len() == 1 { 0 } else { k }].clone();
            }
            Ok(())
        }
        Value::Struct(_) | Value::Func(_) => {
            if !is_one(idx) {
                return Err("struct and function handle arrays are not supported".into());
            }
            *target = rhs;
            Ok(())
        }
        // complex stays in place; narrows back to real when the last imaginary part goes
        Value::Complex(m) => {
            if del {
                *m = delete(m, idx)?;
            } else {
                assign_matrix(m, idx, &rhs.cmatrix()?)?;
            }
            if m.is_real() {
                *target = Value::Num(m.re());
            }
            Ok(())
        }
        // complex into real promotes the whole array
        _ if rhs.is_complex() => {
            let mut m = target.cmatrix()?;
            assign_matrix(&mut m, idx, &rhs.cmatrix()?)?;
            *target = Value::complex(m);
            Ok(())
        }
        // in place: the common `x(i) = v` in a loop must not copy x
        Value::Num(m) if !del && !m.is_empty() && !matches!(rhs, Value::Str(_)) => assign_matrix(m, idx, &rhs.matrix()?),
        Value::Bool(m) if !del && matches!(rhs, Value::Bool(_)) => assign_matrix(m, idx, &rhs.matrix()?),
        _ => {
            let mut m = target.matrix()?;
            if del {
                m = delete(&m, idx)?;
            } else {
                assign_matrix(&mut m, idx, &rhs.matrix()?)?;
            }
            let fresh = matches!(target, Value::Num(t) if t.is_empty());
            *target = match (&*target, &rhs) {
                (Value::Str(_), _) => Value::Str(str_of(&m)),
                (_, Value::Str(_)) if fresh => Value::Str(str_of(&m)),
                (Value::Bool(_), _) if del => Value::Bool(m),
                (Value::Bool(_), Value::Bool(_)) => Value::Bool(m),
                (_, Value::Bool(_)) if fresh => Value::Bool(m),
                _ => Value::Num(m),
            };
            Ok(())
        }
    }
}

/// Walk an assignment path (`s.a{2}(3) = v`), creating structs/cells on the way.
pub fn assign_path(target: &mut Value, path: &[AccV], rhs: Value) -> Result<(), String> {
    let fresh = matches!(target, Value::Num(m) if m.is_empty());
    match path {
        [] => {
            *target = rhs;
            Ok(())
        }
        [AccV::Paren(idx)] => assign(target, idx, rhs),
        [AccV::Paren(idx), rest @ ..] => {
            if !is_one(idx) {
                return Err("struct arrays are not supported".into());
            }
            assign_path(target, rest, rhs)
        }
        [AccV::Brace(idx), rest @ ..] => {
            if fresh {
                *target = Value::Cell(vec![]);
            }
            let Value::Cell(c) = target else {
                return Err(format!("matrix cannot be indexed with {{ (value is {})", target.type_name()));
            };
            let p = positions(cell_lin(idx)?, c.len())?;
            let &[pos] = &p[..] else {
                return Err("invalid number of elements in cs-list assignment".into());
            };
            if pos >= c.len() {
                c.resize(pos + 1, Value::empty());
            }
            assign_path(&mut c[pos], rest, rhs)
        }
        [AccV::Field(f), rest @ ..] => {
            if fresh {
                *target = Value::Struct(BTreeMap::new());
            }
            let Value::Struct(s) = target else {
                return Err(format!("invalid use of a {} to assign field '{f}'", target.type_name()));
            };
            assign_path(s.entry(f.clone()).or_insert_with(Value::empty), rest, rhs)
        }
    }
}

/// Value at a partial assignment path (for `end` inside the next subscript).
pub fn peek<'a>(mut v: &'a Value, path: &[AccV]) -> Option<&'a Value> {
    for a in path {
        v = match (a, v) {
            (AccV::Field(f), Value::Struct(s)) => s.get(f)?,
            (AccV::Brace(i), Value::Cell(c)) => c.get(*cell_positions(c.len(), i).ok()?.first()?)?,
            (AccV::Paren(_), v) => v,
            _ => return None,
        };
    }
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ranges_and_concat() {
        assert_eq!(range(0., 0.1, 1.).unwrap().len(), 11);
        assert_eq!(range(0., 0.1, 0.3).unwrap().len(), 4);
        assert_eq!(range(5., 1., 1.).unwrap().len(), 0);
        assert_eq!(range(5., -2., 1.).unwrap().data, vec![5., 3., 1.]);
        let m = concat(vec![vec![Value::num(1.), Value::from(vec![2., 3.])], vec![Value::from(vec![4., 5., 6.])]]).unwrap();
        assert_eq!(m.matrix().unwrap(), Matrix::from_rows(&[vec![1., 2., 3.], vec![4., 5., 6.]]));
        assert!(concat(vec![vec![Value::from(vec![1., 2.])], vec![Value::num(1.)]]).is_err());
    }
}
