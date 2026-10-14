//! FFT family: iterative radix-2 Cooley–Tukey, Bluestein for other lengths; fftshift, nextpow2,
//! phase unwrap.
use std::f64::consts::{PI, TAU};

use super::{R, dim_of, lanes, nargs, num, real_arg};
use crate::lang::Interp;
use crate::value::{Builtin, C64, CMatrix, Value};

pub fn lookup(name: &str) -> Option<Builtin> {
    let f: Builtin = match name {
        "fft" => |_, a, _| fft_b("fft", a, false),
        "ifft" => |_, a, _| fft_b("ifft", a, true),
        "fft2" => |_, a, _| fft2("fft2", a, false),
        "ifft2" => |_, a, _| fft2("ifft2", a, true),
        "fftshift" => |interp, a, _| shift("fftshift", interp, a, false),
        "ifftshift" => |interp, a, _| shift("ifftshift", interp, a, true),
        "nextpow2" => |_, a, _| {
            nargs("nextpow2", a, 1, 1)?;
            num(real_arg("nextpow2", &a[0])?.map(nextpow2))
        },
        "unwrap" => unwrap,
        _ => return None,
    };
    Some(f)
}

/// exp(sign 2πi k/n), exact on quarter turns so e.g. fft([1 1 1 1]) has exact zeros.
fn cis(k: usize, n: usize, sign: f64) -> C64 {
    let k = k % n;
    match (4 * k).is_multiple_of(n).then_some(4 * k / n) {
        Some(0) => C64::new(1.0, 0.0),
        Some(1) => C64::new(0.0, sign),
        Some(2) => C64::new(-1.0, 0.0),
        Some(3) => C64::new(0.0, -sign),
        _ => C64::from_polar(1.0, sign * TAU * k as f64 / n as f64),
    }
}

/// In-place iterative radix-2 transform; the length must be a power of two (or 0, 1).
fn radix2(x: &mut [C64], sign: f64) {
    let n = x.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            x.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let w: Vec<C64> = (0..len / 2).map(|k| cis(k, len, sign)).collect();
        for s in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (p, q) = (x[s + k], x[s + k + len / 2] * w[k]);
                x[s + k] = p + q;
                x[s + k + len / 2] = p - q;
            }
        }
        len *= 2;
    }
}

/// Unnormalized DFT of any length, in place; `inverse` uses exp(+2πi jk/n).
fn dft(x: &mut [C64], inverse: bool) {
    let n = x.len();
    let sign = if inverse { 1.0 } else { -1.0 };
    if n <= 1 || n.is_power_of_two() {
        return radix2(x, sign);
    }
    // Bluestein: jk = (j² + k² - (k-j)²)/2 turns the DFT into a convolution with the chirp
    // w_k = exp(sign πi k²/n), done with power-of-two transforms of length m >= 2n - 1
    let m = (2 * n - 1).next_power_of_two();
    let w: Vec<C64> = (0..n).map(|k| cis((k as u128 * k as u128 % (2 * n) as u128) as usize, 2 * n, sign)).collect();
    let (mut a, mut b) = (vec![C64::default(); m], vec![C64::default(); m]);
    for k in 0..n {
        a[k] = x[k] * w[k];
        b[k] = w[k].conj();
        b[(m - k) % m] = w[k].conj();
    }
    radix2(&mut a, -1.0);
    radix2(&mut b, -1.0);
    a.iter_mut().zip(&b).for_each(|(p, q)| *p *= q);
    radix2(&mut a, 1.0);
    for k in 0..n {
        x[k] = (a[k] * w[k]).unscale(m as f64);
    }
}

/// 1-D transform of every lane along `dim`, zero-padded or truncated to `n`.
fn transform(x: &CMatrix, n: Option<usize>, dim: usize, inverse: bool) -> CMatrix {
    lanes(x, dim, |l| {
        let mut v = l.to_vec();
        v.resize(n.unwrap_or(l.len()), C64::default());
        dft(&mut v, inverse);
        if inverse {
            let k = v.len() as f64;
            v.iter_mut().for_each(|z| *z = z.unscale(k));
        }
        v
    })
}

/// Transform length argument: positive integer, `[]` for the default.
fn points(name: &str, v: Option<&Value>) -> Result<Option<usize>, String> {
    match v {
        Some(v) if v.size() != (0, 0) => {
            let n = v.as_usize().ok().filter(|&n| n > 0 && n <= 1 << 30);
            n.map(Some).ok_or_else(|| format!("{name}: number of points N must be greater than zero"))
        }
        _ => Ok(None),
    }
}

/// fft / ifft (x [, n [, dim]]): down columns, or along a row vector.
fn fft_b(name: &str, a: &[Value], inverse: bool) -> R {
    nargs(name, a, 1, 3)?;
    let x = a[0].cmatrix()?;
    let n = points(name, a.get(1))?;
    let d = dim_of(name, &x, a.get(2))?;
    Ok(vec![Value::complex(transform(&x, n, d, inverse))])
}

/// fft2 / ifft2 (x [, m, n]): down the columns, then along the rows.
fn fft2(name: &str, a: &[Value], inverse: bool) -> R {
    if a.len() != 1 && a.len() != 3 {
        return Err(format!("Invalid call to {name}"));
    }
    let x = a[0].cmatrix()?;
    let (m, n) = (points(name, a.get(1))?, points(name, a.get(2))?);
    Ok(vec![Value::complex(transform(&transform(&x, m, 1, inverse), n, 2, inverse))])
}

/// fftshift / ifftshift: swap the halves along every dimension (or only DIM) via circshift.
fn shift(name: &str, interp: &mut Interp, a: &[Value], inverse: bool) -> R {
    nargs(name, a, 1, 2)?;
    let (r, c) = a[0].size();
    let half = |k: usize| (if inverse { k.div_ceil(2) } else { k / 2 }) as f64;
    let (sr, sc) = match a.get(1).map(|d| d.as_usize()) {
        None => (half(r), half(c)),
        Some(Ok(1)) => (half(r), 0.0),
        Some(Ok(2)) => (0.0, half(c)),
        Some(Ok(d)) if d > 2 => (0.0, 0.0),
        _ => return Err(format!("{name}: DIM must be a valid dimension")),
    };
    super::math::circshift(interp, &[a[0].clone(), Value::from(vec![sr, sc])], 1)
}

/// Smallest n with 2^n >= |x|; nextpow2(0) = 0.
fn nextpow2(x: f64) -> f64 {
    let x = x.abs();
    if x == 0.0 || !x.is_finite() {
        return if x == 0.0 { 0.0 } else { x };
    }
    let n = x.log2().ceil();
    // log2 may be an ulp off next to exact powers of two
    if 2f64.powf(n - 1.0) >= x {
        n - 1.0
    } else if 2f64.powf(n) < x {
        n + 1.0
    } else {
        n
    }
}

/// unwrap(x [, tol [, dim]]): jumps bigger than tol (default π) lose their multiples of 2π.
fn unwrap(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("unwrap", a, 1, 3)?;
    let x = real_arg("unwrap", &a[0])?;
    let tol = match a.get(1) {
        Some(v) if v.size() != (0, 0) => v.scalar()?.abs(),
        _ => PI,
    };
    let d = dim_of("unwrap", &x, a.get(2))?;
    num(lanes(&x, d, |l| {
        let mut off = 0.0;
        let mut out = l.to_vec();
        for i in 1..l.len() {
            let step = l[i] - l[i - 1];
            if step.abs() > tol {
                off -= (step.abs() / TAU).round() * TAU * step.signum();
            }
            out[i] = l[i] + off;
        }
        out
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{Mat, Matrix};

    fn run(name: &str, args: &[Value]) -> Value {
        lookup(name).unwrap()(&mut Interp::new(), args, 1).unwrap().remove(0)
    }

    fn close(a: &[C64], b: &[C64], tol: f64) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).norm() <= tol)
    }

    fn naive(x: &[C64]) -> Vec<C64> {
        let n = x.len();
        (0..n).map(|k| (0..n).map(|j| x[j] * C64::from_polar(1.0, -TAU * (j * k % n) as f64 / n as f64)).sum()).collect()
    }

    fn signal(n: usize) -> Vec<C64> {
        (0..n).map(|j| C64::new((j as f64 * 0.7).sin() + 0.1 * j as f64, (j as f64 * 1.3).cos())).collect()
    }

    #[test]
    fn fft_matches_dft() {
        for n in [1, 2, 3, 7, 8, 12, 64, 100] {
            let x = signal(n);
            let y = run("fft", &[Value::complex(Mat::row(x.clone()))]).cmatrix().unwrap();
            assert_eq!((y.rows, y.cols), (1, n));
            assert!(close(&y.data, &naive(&x), 1e-10 * n as f64), "fft length {n}");
            let back = run("ifft", &[Value::Complex(y)]).cmatrix().unwrap();
            assert!(close(&back.data, &x, 1e-12), "ifft length {n}");
        }
        // real input, exact quarter-turn twiddles: exact zeros narrow to a real result
        assert_eq!(run("fft", &[Value::from(vec![1., 1., 1., 1.])]).matrix().unwrap().data, vec![4., 0., 0., 0.]);
        let y = run("fft", &[Value::from(vec![1., 2., 3., 4.])]).cmatrix().unwrap();
        assert_eq!(y.data, vec![C64::new(10., 0.), C64::new(-2., 2.), C64::new(-2., 0.), C64::new(-2., -2.)]);
    }

    #[test]
    fn fft_padding_columns() {
        let x = Value::from(vec![1., 2., 3.]);
        let y = run("fft", &[x.clone(), Value::num(5.)]).cmatrix().unwrap();
        let padded: Vec<C64> = [1., 2., 3., 0., 0.].iter().map(|&v| C64::new(v, 0.)).collect();
        assert!(close(&y.data, &naive(&padded), 1e-12) && y.cols == 5);
        let y = run("fft", &[x, Value::num(2.)]).cmatrix().unwrap();
        assert!(close(&y.data, &[C64::new(3., 0.), C64::new(-1., 0.)], 1e-15));
        // matrices go column by column; dim 2 goes along rows
        let m: Value = Matrix::from_rows(&[vec![1., 2.], vec![3., 5.]]).into();
        assert_eq!(run("fft", &[m.clone()]).matrix().unwrap(), Matrix::from_rows(&[vec![4., 7.], vec![-2., -3.]]));
        assert_eq!(run("fft", &[m.clone(), Value::empty(), Value::num(2.)]).matrix().unwrap(), Matrix::from_rows(&[vec![3., -1.], vec![8., -2.]]));
        assert_eq!(run("fft2", &[m.clone()]).matrix().unwrap(), Matrix::from_rows(&[vec![11., -3.], vec![-5., 1.]]));
        assert_eq!(run("ifft2", &[run("fft2", &[m.clone()])]).matrix().unwrap(), m.matrix().unwrap());
        assert!(fft_b("fft", &[m, Value::num(0.)], false).is_err());
    }

    #[test]
    fn shifts_pow2_unwrap() {
        let v = |x: Vec<f64>| Value::from(x);
        let s = |name: &str, x: Vec<f64>| run(name, &[v(x)]).matrix().unwrap().data;
        assert_eq!(s("fftshift", vec![1., 2., 3., 4., 5.]), vec![4., 5., 1., 2., 3.]);
        assert_eq!(s("ifftshift", vec![1., 2., 3., 4., 5.]), vec![3., 4., 5., 1., 2.]);
        assert_eq!(s("fftshift", vec![1., 2., 3., 4.]), vec![3., 4., 1., 2.]);
        assert_eq!(s("ifftshift", s("fftshift", vec![1., 2., 3., 4., 5., 6., 7.])), vec![1., 2., 3., 4., 5., 6., 7.]);
        let m: Value = Matrix::from_rows(&[vec![1., 2.], vec![3., 4.]]).into();
        assert_eq!(run("fftshift", &[m]).matrix().unwrap(), Matrix::from_rows(&[vec![4., 3.], vec![2., 1.]]));
        let z = run("fftshift", &[Value::complex(Mat::row(vec![C64::new(1., 1.), C64::new(2., 0.)]))]).cmatrix().unwrap();
        assert_eq!(z.data, vec![C64::new(2., 0.), C64::new(1., 1.)]);
        assert_eq!(s("nextpow2", vec![0., 1., 2., 3., 1000., 1024., 0.3, -5.]), vec![0., 0., 1., 2., 10., 10., -1., 3.]);
        let ramp: Vec<f64> = (0..8).map(|k| 2.0 * k as f64).collect();
        let wrapped: Vec<f64> = ramp.iter().map(|t| t.sin().atan2(t.cos())).collect();
        let u = s("unwrap", wrapped);
        assert!(u.iter().zip(&ramp).all(|(a, b)| (a - b).abs() < 1e-12), "{u:?}");
        assert_eq!(s("unwrap", vec![0., 3., 7.]), vec![0., 3., 7. - TAU]);
    }
}
