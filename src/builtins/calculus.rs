//! Multiple integrals, nonlinear systems (fsolve), unconstrained minimization (fminunc), the
//! numeric Jacobian, del2 / divergence / curl on meshgrid arrays, and odeset / odeget.
use std::collections::BTreeMap;

use nalgebra::{DMatrix, DVector};

use super::numeric::{coords, grad, opt, quad, quad_tols};
use super::{R, first, lanes, nargs, num, real_arg};
use crate::lang::Interp;
use crate::value::{Builtin, Matrix, Value};

pub fn lookup(name: &str) -> Option<Builtin> {
    let f: Builtin = match name {
        "integral2" => |it, a, _| nested("integral2", 2, it, a),
        "dblquad" => |it, a, _| nested("dblquad", 2, it, a),
        "integral3" => |it, a, _| nested("integral3", 3, it, a),
        "triplequad" => |it, a, _| nested("triplequad", 3, it, a),
        "fsolve" => fsolve,
        "fminunc" => fminunc,
        // numeric, by central differences (no symbolic math here): jacobian(@(v) [v(1)^2; v(1)*v(2)], [1 2])
        "jacobian" => |it, a, _| {
            nargs("jacobian", a, 2, 2)?;
            let x0 = real_arg("jacobian", &a[1])?;
            let j = fd_jacobian(&mut vfun("jacobian", it, &a[0], &x0, &mut (0, 0)), &x0.data)?;
            num(Matrix::new(j.nrows(), j.ncols(), j.as_slice().to_vec()))
        },
        "del2" => del2,
        "divergence" => |_, a, _| field("divergence", a),
        "curl" => |_, a, _| field("curl", a),
        "odeset" => odeset,
        "odeget" => |_, a, _| {
            nargs("odeget", a, 2, 3)?;
            let Value::Struct(s) = &a[0] else { return Err("odeget: ODE_OPT must be a valid ODE_OPT structure".into()) };
            let v = s.get(&ode_field(a[1].as_str()?)).filter(|v| v.size() != (0, 0));
            Ok(vec![v.or(a.get(2)).cloned().unwrap_or_else(Value::empty)])
        },
        _ => return None,
    };
    Some(f)
}

/// integral2 / integral3 (and dblquad / triplequad) as nested adaptive quadrature. Inner limits may
/// be function handles of the outer variables: integral2(f, 0, 1, 0, @(x) x).
fn nested(name: &str, dims: usize, interp: &mut Interp, a: &[Value]) -> R {
    if a.len() < 1 + 2 * dims {
        return Err(format!("Invalid call to {name}"));
    }
    let (abstol, reltol) = quad_tols(&a[1 + 2 * dims..])?;
    let (f, lim) = (&a[0], &a[1..1 + 2 * dims]);
    let call = |it: &mut Interp, v: &Value, at: &[f64]| -> Result<f64, String> {
        first(it.call(v, &at.iter().map(|&t| Value::num(t)).collect::<Vec<_>>(), 1)?)?.scalar()
    };
    let bound = |it: &mut Interp, v: &Value, outer: &[f64]| match v {
        Value::Func(_) => call(it, v, outer),
        v => v.scalar(),
    };
    // ponytail: every level uses the caller's tolerances, so error can add up across levels
    let mut fx = |x: f64| -> Result<f64, String> {
        let (y0, y1) = (bound(interp, &lim[2], &[x])?, bound(interp, &lim[3], &[x])?);
        let mut fy = |y: f64| -> Result<f64, String> {
            if dims == 2 {
                return call(interp, f, &[x, y]);
            }
            let (z0, z1) = (bound(interp, &lim[4], &[x, y])?, bound(interp, &lim[5], &[x, y])?);
            quad(&mut |z| call(interp, f, &[x, y, z]), z0, z1, abstol, reltol)
        };
        quad(&mut fy, y0, y1, abstol, reltol)
    };
    num(Matrix::scalar(quad(&mut fx, lim[0].scalar()?, lim[1].scalar()?, abstol, reltol)?))
}

type Fv<'a> = dyn FnMut(&[f64]) -> Result<Vec<f64>, String> + 'a;

/// x -> f(x) through the interpreter, x shaped like x0; `shape` gets the shape f returns.
fn vfun<'a>(name: &'a str, it: &'a mut Interp, f: &'a Value, x0: &Matrix, shape: &'a mut (usize, usize)) -> impl FnMut(&[f64]) -> Result<Vec<f64>, String> + 'a {
    let (r, c) = (x0.rows, x0.cols);
    move |x| {
        let m = real_arg(name, &first(it.call(f, &[Value::Num(Matrix::new(r, c, x.to_vec()))], 1)?)?)?;
        *shape = (m.rows, m.cols);
        Ok(m.data)
    }
}

/// J(i, j) = d f_i / d x_j by central differences, step eps^(1/3) * max(|x_j|, 1).
fn fd_jacobian(f: &mut Fv, x: &[f64]) -> Result<DMatrix<f64>, String> {
    let mut cols: Vec<Vec<f64>> = vec![];
    let mut xp = x.to_vec();
    for j in 0..x.len() {
        let h = f64::EPSILON.cbrt() * x[j].abs().max(1.0);
        xp[j] = x[j] + h;
        let fp = f(&xp)?;
        xp[j] = x[j] - h;
        let fm = f(&xp)?;
        xp[j] = x[j];
        if fp.len() != fm.len() {
            return Err("jacobian: the function must return the same number of values everywhere".into());
        }
        cols.push(fp.iter().zip(&fm).map(|(p, m)| (p - m) / (2.0 * h)).collect());
    }
    let m = cols.first().map_or(0, Vec::len);
    Ok(DMatrix::from_fn(m, x.len(), |i, j| cols[j][i]))
}

/// Levenberg–Marquardt (Nielsen's damping update) for f(x) = 0, in the least-squares sense when
/// f has more outputs than inputs. Returns (x, f(x), info, iterations); info is Octave's:
/// 1 residual <= tolf, 2 step <= tolx, 3 stuck at a nonzero residual, 0 iteration limit.
fn levmar(f: &mut Fv, x0: &[f64], tolx: f64, tolf: f64, maxit: usize) -> Result<(Vec<f64>, Vec<f64>, i32, usize), String> {
    let done = |x: DVector<f64>, fx: DVector<f64>, info, it| Ok((x.as_slice().to_vec(), fx.as_slice().to_vec(), info, it));
    let n = x0.len();
    let mut x = DVector::from_column_slice(x0);
    let mut fx = DVector::from_vec(f(x.as_slice())?);
    let (mut mu, mut nu) = (-1.0, 2.0);
    for it in 0..maxit {
        if fx.amax() <= tolf {
            return done(x, fx, 1, it);
        }
        let j = fd_jacobian(f, x.as_slice())?;
        let (a, g) = (j.tr_mul(&j), j.tr_mul(&fx));
        if g.amax() <= f64::EPSILON * fx.norm_squared() {
            return done(x, fx, 3, it);
        }
        if mu < 0.0 {
            mu = 1e-3 * a.diagonal().max().max(f64::EPSILON);
        }
        let mut accepted = false;
        for _ in 0..64 {
            let damped = &a + DMatrix::<f64>::identity(n, n) * mu;
            let h = damped.lu().solve(&-&g).unwrap_or_else(|| DVector::from_element(n, f64::NAN));
            if h.norm() <= tolx * (x.norm() + tolx) {
                return done(x, fx, 2, it);
            }
            let xn = &x + &h;
            let fnew = DVector::from_vec(f(xn.as_slice())?);
            if fnew.len() != fx.len() {
                return Err("fsolve: the function must return the same number of values everywhere".into());
            }
            // actual / predicted decrease of |f|^2
            let rho = (fx.norm_squared() - fnew.norm_squared()) / h.dot(&(&h * mu - &g));
            if rho > 0.0 {
                (x, fx) = (xn, fnew);
                mu *= (1.0 - (2.0 * rho - 1.0).powi(3)).max(1.0 / 3.0);
                nu = 2.0;
                accepted = true;
                break;
            }
            mu *= nu;
            nu *= 2.0;
        }
        if !accepted {
            return done(x, fx, 3, it);
        }
    }
    done(x, fx, 0, maxit)
}

/// [x, fval, info, output] = fsolve(f, x0[, options]); options: TolX, TolFun, MaxIter.
fn fsolve(interp: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("fsolve", a, 2, 3)?;
    let x0 = real_arg("fsolve", &a[1])?;
    let o = a.get(2);
    let (tolx, tolf, maxit) = (opt(o, "TolX", 1e-10), opt(o, "TolFun", 1e-10), opt(o, "MaxIter", 400.0) as usize);
    let mut shape = (0, 0);
    let (x, fx, info, it) = levmar(&mut vfun("fsolve", interp, &a[0], &x0, &mut shape), &x0.data, tolx, tolf, maxit)?;
    let out = BTreeMap::from([("iterations".to_string(), Value::num(it as f64))]);
    Ok(vec![
        Value::Num(Matrix::new(x0.rows, x0.cols, x)),
        Value::Num(Matrix::new(shape.0, shape.1, fx)),
        Value::num(info as f64),
        Value::Struct(out),
    ])
}

/// BFGS with a central-difference gradient and Armijo backtracking. Returns (x, f(x), gradient,
/// info, iterations); info: 1 gradient <= tolf, 2 step <= tolx, 0 iteration limit.
type Min = (Vec<f64>, f64, Vec<f64>, i32, usize);

fn bfgs(f: &mut dyn FnMut(&[f64]) -> Result<f64, String>, x0: &[f64], tolx: f64, tolf: f64, maxit: usize) -> Result<Min, String> {
    let n = x0.len();
    let gradient = |f: &mut dyn FnMut(&[f64]) -> Result<f64, String>, x: &DVector<f64>| -> Result<DVector<f64>, String> {
        Ok(DVector::from_iterator(n, fd_jacobian(&mut |v: &[f64]| Ok(vec![f(v)?]), x.as_slice())?.iter().copied()))
    };
    let done = |x: DVector<f64>, fx, g: DVector<f64>, info, it| Ok((x.as_slice().to_vec(), fx, g.as_slice().to_vec(), info, it));
    let id = DMatrix::<f64>::identity(n, n);
    let mut x = DVector::from_column_slice(x0);
    let mut fx = f(x.as_slice())?;
    let mut g = gradient(f, &x)?;
    let mut h = id.clone(); // inverse Hessian estimate
    for it in 0..maxit {
        if g.amax() <= tolf {
            return done(x, fx, g, 1, it);
        }
        let mut p = -(&h * &g);
        if !(p.dot(&g) < 0.0) {
            h = id.clone();
            p = -g.clone();
        }
        let slope = p.dot(&g);
        let mut t = 1.0;
        let (xn, fnew) = loop {
            let xn = &x + &p * t;
            let fnew = f(xn.as_slice())?;
            if fnew <= fx + 1e-4 * t * slope {
                break (xn, fnew);
            }
            t /= 2.0;
            if t < 1e-20 {
                return done(x, fx, g, 2, it);
            }
        };
        let gn = gradient(f, &xn)?;
        let (s, y) = (&xn - &x, &gn - &g);
        let sy = s.dot(&y);
        if sy > f64::EPSILON * s.norm() * y.norm() {
            if it == 0 {
                h = &id * (sy / y.dot(&y));
            }
            let r = &id - &s * y.transpose() / sy;
            h = &r * &h * r.transpose() + &s * s.transpose() / sy;
        }
        let small = s.norm() <= tolx * (1.0 + x.norm());
        (x, fx, g) = (xn, fnew, gn);
        if small {
            return done(x, fx, g, 2, it + 1);
        }
    }
    done(x, fx, g, 0, maxit)
}

/// [x, fval, info, output, grad] = fminunc(f, x0[, options]); options: TolX, TolFun, MaxIter.
fn fminunc(interp: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("fminunc", a, 2, 3)?;
    let x0 = real_arg("fminunc", &a[1])?;
    let o = a.get(2);
    let (tolx, tolf, maxit) = (opt(o, "TolX", 1e-12), opt(o, "TolFun", 1e-8), opt(o, "MaxIter", 400.0) as usize);
    let (r, c) = (x0.rows, x0.cols);
    let mut f = |x: &[f64]| first(interp.call(&a[0], &[Value::Num(Matrix::new(r, c, x.to_vec()))], 1)?)?.scalar();
    let (x, fx, g, info, it) = bfgs(&mut f, &x0.data, tolx, tolf, maxit)?;
    let out = BTreeMap::from([("iterations".to_string(), Value::num(it as f64))]);
    Ok(vec![Value::Num(Matrix::new(r, c, x)), Value::num(fx), Value::num(info as f64), Value::Struct(out), Value::Num(Matrix::new(r, c, g))])
}

/// Second differences / 2 along a lane with coordinates x, ends extrapolated linearly (Octave's del2.m).
fn second(l: &[f64], x: &[f64]) -> Vec<f64> {
    let n = l.len();
    if n < 3 {
        return vec![0.0; n];
    }
    let mut d = vec![0.0; n];
    for i in 1..n - 1 {
        let (h1, h2) = (x[i] - x[i - 1], x[i + 1] - x[i]);
        d[i] = ((l[i - 1] - l[i]) / h1 + (l[i + 1] - l[i]) / h2) / (h1 + h2);
    }
    let k = n - 1;
    let (h1, h2) = (x[1] - x[0], x[2] - x[1]);
    d[0] = (h1 + h2) / h2 * d[1] - h1 / h2 * d[2];
    let (h1, h2) = (x[k] - x[k - 1], x[k - 1] - x[k - 2]);
    d[k] = (h1 + h2) / h2 * d[k - 1] - h1 / h2 * d[k - 2];
    d
}

/// del2(M[, h | hx, hy]): discrete Laplacian / 4, MATLAB's scaling.
fn del2(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("del2", a, 1, 3)?;
    let m = real_arg("del2", &a[0])?;
    let (hx, hy) = if a.len() == 3 { (a.get(1), a.get(2)) } else { (a.get(1), a.get(1)) };
    // a coordinate vector only applies along a dimension long enough to use it
    let hx = if m.cols >= 3 { coords(hx, m.cols)? } else { vec![] };
    let hy = if m.rows >= 3 { coords(hy, m.rows)? } else { vec![] };
    let (dx, dy) = (lanes(&m, 2, |l| second(l, &hx)), lanes(&m, 1, |l| second(l, &hy)));
    num(Matrix::new(m.rows, m.cols, dx.data.iter().zip(&dy.data).map(|(p, q)| (p + q) / 2.0).collect()))
}

/// divergence / curl of a 2-D field: (X, Y, U, V) from meshgrid, or (U, V) on unit spacing.
/// curl returns [curlz, angular velocity curlz/2].
// ponytail: 2-D only (values are matrices, no 3-D arrays)
fn field(name: &str, a: &[Value]) -> R {
    let (xy, u, v) = match a {
        [u, v] => (None, u, v),
        [x, y, u, v] => (Some((x, y)), u, v),
        _ => return Err(format!("{name}: only 2-D fields are supported: {name}(X, Y, U, V) or {name}(U, V)")),
    };
    let (u, v) = (real_arg(name, u)?, real_arg(name, v)?);
    if (u.rows, u.cols) != (v.rows, v.cols) {
        return Err(format!("{name}: U and V must be the same size"));
    }
    // meshgrid X varies along rows, Y down columns; plain vectors work too
    let (hx, hy) = match xy {
        Some((x, y)) => {
            let (x, y) = (real_arg(name, x)?, real_arg(name, y)?);
            let xs = if x.rows == 1 || x.cols == 1 { x.data } else { (0..x.cols).map(|j| x.get(0, j)).collect() };
            let ys = if y.rows == 1 || y.cols == 1 { y.data } else { y.data[..y.rows].to_vec() };
            (coords(Some(&Value::from(xs)), u.cols)?, coords(Some(&Value::from(ys)), u.rows)?)
        }
        None => (coords(None, u.cols)?, coords(None, u.rows)?),
    };
    let ddx = |m: &Matrix| lanes(m, 2, |l| grad(l, &hx));
    let ddy = |m: &Matrix| lanes(m, 1, |l| grad(l, &hy));
    let comb = |p: Matrix, q: Matrix, s: f64| Matrix::new(p.rows, p.cols, p.data.iter().zip(&q.data).map(|(a, b)| a + s * b).collect());
    if name == "divergence" {
        return num(comb(ddx(&u), ddy(&v), 1.0));
    }
    let cz = comb(ddx(&v), ddy(&u), -1.0);
    Ok(vec![Value::Num(cz.clone()), Value::Num(cz.map(|z| z / 2.0))])
}

const ODE_FIELDS: [&str; 21] = [
    "AbsTol", "BDF", "Events", "InitialSlope", "InitialStep", "Jacobian", "JConstant", "JPattern", "Mass", "MassSingular",
    "MaxOrder", "MaxStep", "MStateDependence", "MvPattern", "NonNegative", "NormControl", "OutputFcn", "OutputSel",
    "Refine", "RelTol", "Stats",
];

/// Canonical spelling of an odeset field ("reltol" -> "RelTol"); unknown names stay as given.
fn ode_field(k: &str) -> String {
    ODE_FIELDS.iter().find(|f| f.eq_ignore_ascii_case(k)).map_or(k, |f| *f).to_string()
}

/// odeset([old, ...,] 'Name', value, ...): every known field present, [] when unset.
fn odeset(_: &mut Interp, a: &[Value], _: usize) -> R {
    let mut s: BTreeMap<String, Value> = ODE_FIELDS.iter().map(|k| (k.to_string(), Value::empty())).collect();
    let mut rest = a;
    while let Some((Value::Struct(o), more)) = rest.split_first() {
        s.extend(o.iter().map(|(k, v)| (ode_field(k), v.clone())));
        rest = more;
    }
    if rest.len() % 2 != 0 {
        return Err("odeset: FIELD/VALUE arguments must occur in pairs".into());
    }
    for p in rest.chunks(2) {
        let k = p[0].as_str().map_err(|_| "odeset: FIELD names must be strings".to_string())?;
        s.insert(ode_field(k), p[1].clone());
    }
    Ok(vec![Value::Struct(s)])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> Interp {
        let mut it = Interp::new();
        it.eval(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        it
    }

    fn get(it: &Interp, k: &str) -> Vec<f64> {
        it.vars[k].matrix().unwrap().data
    }

    fn close(a: &[f64], b: &[f64], tol: f64) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
    }

    #[test]
    fn multiple_integrals() {
        let it = run(
            "a = integral2(@(x, y) x.*y, 0, 1, 0, 1); b = integral2(@(x, y) x.*y, 0, 1, 0, @(x) x);
             c = dblquad(@(x, y) x.^2 + y, 0, 2, -1, 1); d = integral3(@(x, y, z) x.*y.*z, 0, 1, 0, 1, 0, 1);
             e = triplequad(@(x, y, z) ones(size(x)), 0, 1, 0, 1, 0, 1); f = integral2(@(x, y) exp(-x.^2 - y.^2), -Inf, Inf, -Inf, Inf);
             g = integral3(@(x, y, z) 1, -1, 1, @(x) -sqrt(1 - x^2), @(x) sqrt(1 - x^2), @(x, y) -sqrt(1 - x^2 - y^2), @(x, y) sqrt(1 - x^2 - y^2), 'RelTol', 1e-8);",
        );
        assert!((get(&it, "a")[0] - 0.25).abs() < 1e-14);
        assert!((get(&it, "b")[0] - 0.125).abs() < 1e-14);
        assert!((get(&it, "c")[0] - 16.0 / 3.0).abs() < 1e-12);
        assert!((get(&it, "d")[0] - 0.125).abs() < 1e-14);
        assert!((get(&it, "e")[0] - 1.0).abs() < 1e-14);
        assert!((get(&it, "f")[0] - std::f64::consts::PI).abs() < 1e-6);
        assert!((get(&it, "g")[0] - 4.0 / 3.0 * std::f64::consts::PI).abs() < 1e-5, "{:?}", get(&it, "g"));
        assert!(Interp::new().eval("integral2(@(x, y) x, 0, 1, 0)").is_err());
    }

    #[test]
    fn nonlinear_systems() {
        // x^2 + y^2 = 4, x*y = 1 near (2, 0.5); and a scalar root
        let it = run(
            "[x, fv, info] = fsolve(@(v) [v(1)^2 + v(2)^2 - 4; v(1)*v(2) - 1], [2; 0.5]);
             r = fsolve(@(x) x^3 - 2, 1); [~, ~, i2] = fsolve(@(x) x^2 + 1, 0.5);",
        );
        let x = get(&it, "x");
        assert!((x[0] * x[0] + x[1] * x[1] - 4.0).abs() < 1e-10 && (x[0] * x[1] - 1.0).abs() < 1e-10, "{x:?}");
        assert!(get(&it, "fv").iter().all(|v| v.abs() <= 1e-10) && get(&it, "info") == [1.0]);
        assert_eq!(it.vars["x"].size(), (2, 1));
        assert!((get(&it, "r")[0] - 2f64.cbrt()).abs() < 1e-12);
        assert_ne!(get(&it, "i2"), [1.0]); // x^2 + 1 has no real root
        // Rosenbrock and a quadratic bowl
        let it = run(
            "[x, fv, info, out, g] = fminunc(@(v) (1 - v(1))^2 + 100*(v(2) - v(1)^2)^2, [-1.2 1]);
             y = fminunc(@(v) (v(1) - 3)^2 + 2*(v(2) + 1)^2 + 1, [0 0]);",
        );
        assert!(close(&get(&it, "x"), &[1.0, 1.0], 1e-5) && get(&it, "fv")[0] < 1e-10, "{:?}", get(&it, "x"));
        assert_eq!(it.vars["x"].size(), (1, 2));
        assert!(get(&it, "info")[0] > 0.0 && get(&it, "g").iter().all(|v| v.abs() < 1e-6));
        assert!(close(&get(&it, "y"), &[3.0, -1.0], 1e-7));
    }

    #[test]
    fn jacobian_and_fields() {
        let it = run("J = jacobian(@(v) [v(1)^2*v(2); 5*v(1) + sin(v(2))], [1 2]);");
        assert_eq!(it.vars["J"].size(), (2, 2));
        assert!(close(&get(&it, "J"), &[4.0, 5.0, 1.0, 2f64.cos()], 1e-9), "{:?}", get(&it, "J"));
        // f'' / 4 for vectors, (f_xx + f_yy) / 4 for matrices
        let it = run(
            "a = del2([1 4 9 16 25]); b = del2((0:0.5:2).^3, 0.5); [X, Y] = meshgrid(0:4, 0:3); c = del2(X.^2 + Y.^2);
             d = divergence(X, Y, X.^2, Y.^2); [cz, cav] = curl(X, Y, -Y, X); e = divergence(-Y, X);",
        );
        assert_eq!(get(&it, "a"), [0.5; 5]);
        assert!(close(&get(&it, "b"), &[0.0, 0.75, 1.5, 2.25, 3.0], 1e-12), "{:?}", get(&it, "b")); // 6x/4
        assert!(get(&it, "c").iter().all(|&v| (v - 1.0).abs() < 1e-12));
        let (d, x, y) = (it.vars["d"].matrix().unwrap(), it.vars["X"].matrix().unwrap(), it.vars["Y"].matrix().unwrap());
        assert!((1..3).all(|i| (1..4).all(|j| d.get(i, j) == 2.0 * x.get(i, j) + 2.0 * y.get(i, j))));
        assert!(get(&it, "cz").iter().all(|&v| v == 2.0) && get(&it, "cav").iter().all(|&v| v == 1.0));
        assert!(get(&it, "e").iter().all(|&v| v == 0.0));
        assert!(Interp::new().eval("divergence(1, 2, 3, 4, 5, 6)").is_err());
    }

    #[test]
    fn ode_options() {
        let it = run(
            "o = odeset('reltol', 1e-8, 'AbsTol', 1e-10); o2 = odeset(o, 'Refine', 1); r = odeget(o2, 'RelTol');
             m = odeget(o, 'MaxStep', 7); [t, y] = ode45(@(t, y) -y, [0 1], 1, o); [t2, y2] = ode15s(@(t, y) -y, [0 1], 1);",
        );
        let Value::Struct(o) = &it.vars["o2"] else { panic!() };
        assert!(o.contains_key("RelTol") && o.contains_key("Mass") && o["Refine"].scalar() == Ok(1.0));
        assert_eq!((get(&it, "r"), get(&it, "m")), (vec![1e-8], vec![7.0]));
        assert!((get(&it, "y").last().unwrap() - (-1f64).exp()).abs() < 1e-8);
        assert!((get(&it, "y2").last().unwrap() - (-1f64).exp()).abs() < 1e-4);
        assert!(Interp::new().eval("odeset('RelTol')").is_err());
    }
}
