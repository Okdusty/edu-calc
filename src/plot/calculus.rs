//! Calculus and matrix visuals: intplot, tangentplot, riemann, slopefield, matview, eigplot.
//! `f` is a function handle or an expression string, like fplot's.
use super::builtins::{add, curve, first, func, letters, linspace, meshgrid, palette, pretty_fn, range, robust_ylim, round_limits, sample, touch, vectorize};
use super::{Axes, Data, LineStyle, Marker, Note, Series, Source, cursor};
use crate::lang::Interp;
use crate::value::{Func, Matrix, Value};

type R = Result<Vec<Value>, String>;

const GRAY: [u8; 3] = [128, 128, 128];

/// Compact number for notes, with a real minus sign.
fn num(v: f64) -> String {
    cursor::num(v).replace('-', "\u{2212}")
}

/// "∫_{a}^{b} (f) dx" for notes, in f's own variable.
fn integral_text(f: &Value, a: f64, b: f64) -> String {
    let name = pretty_fn(f, "x");
    let name = if name.contains(" + ") || name.contains(" − ") { format!("({name})") } else { name };
    let var = match f {
        Value::Str(s) if letters(s).len() == 1 => letters(s)[0].to_string(),
        Value::Func(h) => match &**h {
            Func::Anon(l) if l.params.len() == 1 => l.params[0].clone(),
            _ => "x".into(),
        },
        _ => "x".into(),
    };
    format!("∫_{{{}}}^{{{}}} {name} d{var}", num(a), num(b))
}

fn bounds(name: &str, a: &Value, b: &Value) -> Result<(f64, f64), String> {
    let (a, b) = (a.scalar()?, b.scalar()?);
    if a.is_finite() && b.is_finite() && a != b { Ok((a, b)) } else { Err(format!("{name}: a and b must be finite and different")) }
}

/// Limits for a function plot over [lo, hi]: y around f's samples (poles ignored), 0 included
/// when asked, `top` of the range left free above for a note.
fn limits(a: &mut Axes, (lo, hi): (f64, f64), ys: &[f64], zero: bool, top: f64) {
    let (y0, y1) = robust_ylim(ys).unwrap_or_else(|| range(ys));
    let (y0, y1) = if zero { (y0.min(0.0), y1.max(0.0)) } else { (y0, y1) };
    let r = if y1 > y0 { y1 - y0 } else { 1.0 };
    (a.xlim, a.ylim) = (Some((lo, hi)), Some((y0 - 0.05 * r, y1 + top * r)));
}

/// A note in the top left corner of the limits.
fn corner_note(a: &mut Axes, text: String) {
    let ((x0, x1), (y0, y1)) = (a.xlim.unwrap_or((0.0, 1.0)), a.ylim.unwrap_or((0.0, 1.0)));
    a.notes.push(Note { at: [x0 + 0.02 * (x1 - x0), y1 - 0.08 * (y1 - y0), f64::NAN], text, align: 0.0 });
}

/// Positive and negative parts of y(x) as fill polygons down to 0, zero crossings included;
/// None for a part that's empty.
fn signed_parts(x: &[f64], y: &[f64]) -> [Option<(Vec<f64>, Vec<f64>)>; 2] {
    let mut pts = vec![];
    for k in 0..x.len() {
        if k > 0 && y[k - 1] * y[k] < 0.0 {
            let t = y[k - 1] / (y[k - 1] - y[k]);
            pts.push((x[k - 1] + t * (x[k] - x[k - 1]), 0.0));
        }
        pts.push((x[k], y[k]));
    }
    [1.0, -1.0].map(|sign| {
        let (mut px, mut py) = (vec![x[0]], vec![0.0]);
        for &(a, b) in &pts {
            px.push(a);
            py.push(if b * sign > 0.0 { b } else { 0.0 });
        }
        px.push(x[x.len() - 1]);
        py.push(0.0);
        py.iter().any(|v| *v != 0.0).then_some((px, py))
    })
}

/// intplot(f, a, b): f over [a, b] padded 20%, the signed area shaded, a note with the integral.
pub(super) fn intplot(it: &mut Interp, args: &[Value], nargout: usize) -> R {
    let [f, a, b] = args else { return Err("intplot: expected (f, a, b)".into()) };
    let (a, b) = bounds("intplot", a, b)?;
    let h = func(it, f, false)?;
    let val = first(it.call(&Value::from("integral"), &[h.clone(), Value::num(a), Value::num(b)], 1))?.scalar()?;
    let (lo, hi) = (a.min(b), a.max(b));
    let pad = 0.2 * (hi - lo);
    let (x, y, ys) = curve(it, "intplot", &h, lo - pad, hi + pad)?;
    let sx = linspace(lo, hi, 400);
    let sy = sample(&[Matrix::row(sx.clone())], |v| first(it.call(&h, v, 1))).map_err(|e| format!("intplot: {e}"))?;
    let mut out = vec![];
    for (k, part) in signed_parts(&sx, &sy).into_iter().enumerate() {
        if let Some((x, y)) = part {
            out.push(Series { color: Some(palette(k)), line: LineStyle::None, ..Series::new(Data::Fill { x, y }) });
        }
    }
    out.push(Series { color: Some(palette(0)), width: 2.0, source: Some(Source::Func(h)), ..Series::new(Data::Line { x, y }) });
    let ax = add(it, out);
    if !ax.hold {
        limits(ax, (lo - pad, hi + pad), &ys, true, 0.25);
        ax.title = pretty_fn(f, "x");
        ax.grid = true;
    }
    let (y0, y1) = ax.ylim.unwrap_or_else(|| range(&ys));
    ax.series.push(Series { color: Some(GRAY), width: 1.0, line: LineStyle::Dashed, ..Series::new(Data::Line { x: vec![a, a, f64::NAN, b, b], y: vec![y0, y1, f64::NAN, y0, y1] }) });
    // ponytail: |result| below 1e-12 of the area scale reads as roundoff and shows as 0
    let scale = sy.iter().filter(|v| v.is_finite()).fold(0.0f64, |m, v| m.max(v.abs())) * (hi - lo);
    let shown = if val.abs() < 1e-12 * scale.max(1e-300) { 0.0 } else { val };
    corner_note(ax, format!("{} = {}", integral_text(f, a, b), num(shown)));
    touch(it)?;
    Ok(if nargout > 0 { vec![Value::num(val)] } else { vec![] })
}

/// tangentplot(f, x0[, [a b]]): f, its tangent at x0 (central difference), the point, the slope.
pub(super) fn tangentplot(it: &mut Interp, args: &[Value], nargout: usize) -> R {
    let [f, x0, rest @ ..] = args else { return Err("tangentplot: expected (f, x0[, [a b]])".into()) };
    let x0 = x0.scalar()?;
    let (lo, hi) = match rest.first().map(Value::matrix).transpose()? {
        Some(m) if m.len() == 2 && m.data[0] < m.data[1] => (m.data[0], m.data[1]),
        Some(_) => return Err("tangentplot: range must be [a b] with a < b".into()),
        None => (x0 - 3.0, x0 + 3.0),
    };
    let h = func(it, f, false)?;
    let mut at = |x: f64| first(it.call(&h, &[Value::num(x)], 1))?.scalar();
    let d = 1e-5 * x0.abs().max(1.0);
    let (y0, slope) = (at(x0)?, (at(x0 + d)? - at(x0 - d)?) / (2.0 * d));
    if !(y0.is_finite() && slope.is_finite()) {
        return Err(format!("tangentplot: f has no tangent at x = {x0}"));
    }
    let (x, y, ys) = curve(it, "tangentplot", &h, lo, hi)?;
    let tan = Data::Line { x: vec![lo, hi], y: vec![y0 + slope * (lo - x0), y0 + slope * (hi - x0)] };
    // the tangent as a function too, so zooming out extends it with the curve
    let line = first(it.call(&Value::from("str2func"), &[Value::Str(format!("@(x) {y0} + {slope} * (x - {x0})"))], 1))?;
    let out = vec![
        Series { color: Some(palette(0)), width: 2.0, source: Some(Source::Func(h)), ..Series::new(Data::Line { x, y }) },
        Series { color: Some(palette(1)), source: Some(Source::Func(line)), ..Series::new(tan) },
        Series { color: Some(palette(1)), line: LineStyle::None, marker: Marker::Point, width: 3.0, ..Series::new(Data::Line { x: vec![x0], y: vec![y0] }) },
    ];
    let name = pretty_fn(f, "x");
    let ax = add(it, out);
    if !ax.hold {
        limits(ax, (lo, hi), &ys, false, 0.2);
        ax.title = name;
        ax.grid = true;
    }
    corner_note(ax, format!("f'({}) = {}", num(x0), num(slope)));
    touch(it)?;
    Ok(if nargout > 0 { vec![Value::num(slope)] } else { vec![] })
}

/// riemann(f, a, b, n[, 'left' | 'right' | 'mid' | 'trap']): the strips under f, their sum and
/// the integral.
pub(super) fn riemann(it: &mut Interp, args: &[Value], nargout: usize) -> R {
    let [f, a, b, n, rest @ ..] = args else { return Err("riemann: expected (f, a, b, n[, method])".into()) };
    let (a, b) = bounds("riemann", a, b)?;
    let n = n.as_usize()?;
    if !(1..=100_000).contains(&n) {
        return Err("riemann: n must be 1 to 100000".into());
    }
    let rule = rest.first().map(Value::as_str).transpose()?.unwrap_or("left").to_ascii_lowercase();
    // where each strip's left and right heights are sampled, as fractions of the strip
    let (tag, at) = match rule.as_str() {
        "left" => ("L", (0.0, 0.0)),
        "right" => ("R", (1.0, 1.0)),
        "mid" | "middle" | "midpoint" => ("M", (0.5, 0.5)),
        "trap" | "trapezoid" | "trapezoidal" => ("T", (0.0, 1.0)),
        r => return Err(format!("riemann: unknown method '{r}', use left, right, mid or trap")),
    };
    let h = func(it, f, false)?;
    let dx = (b - a) / n as f64;
    let xs: Vec<f64> = (0..n).flat_map(|i| [a + (i as f64 + at.0) * dx, a + (i as f64 + at.1) * dx]).collect();
    let fs = sample(&[Matrix::row(xs)], |v| first(it.call(&h, v, 1))).map_err(|e| format!("riemann: {e}"))?;
    let (mut px, mut py, mut sum) = (vec![], vec![], 0.0);
    for i in 0..n {
        let (x0, x1, h0, h1) = (a + i as f64 * dx, a + (i + 1) as f64 * dx, fs[2 * i], fs[2 * i + 1]);
        px.extend([x0, x0, x1, x1, f64::NAN]);
        py.extend([0.0, h0, h1, 0.0, f64::NAN]);
        sum += (h0 + h1) / 2.0 * dx;
    }
    let exact = first(it.call(&Value::from("integral"), &[h.clone(), Value::num(a), Value::num(b)], 1))?.scalar()?;
    let (lo, hi) = (a.min(b), a.max(b));
    let pad = 0.1 * (hi - lo);
    let (x, y, ys) = curve(it, "riemann", &h, lo - pad, hi + pad)?;
    let out = vec![
        Series { color: Some(palette(1)), width: 1.0, ..Series::new(Data::Fill { x: px, y: py }) },
        Series { color: Some(palette(0)), width: 2.0, source: Some(Source::Func(h)), ..Series::new(Data::Line { x, y }) },
    ];
    let ax = add(it, out);
    if !ax.hold {
        limits(ax, (lo - pad, hi + pad), &ys, true, 0.3);
        ax.title = pretty_fn(f, "x");
        ax.grid = true;
    }
    corner_note(ax, format!("{tag}_{{{n}}} = {}\n{} = {}", num(sum), integral_text(f, a, b), num(exact)));
    touch(it)?;
    Ok(if nargout > 0 { vec![Value::num(sum), Value::num(exact)] } else { vec![] })
}

/// slopefield(f, [x0 x1], [y0 y1][, y0s]): direction field of y' = f(x, y), plus ode45 solutions
/// from (x0, y) for each y in y0s. A string may use t instead of x.
pub(super) fn slopefield(it: &mut Interp, args: &[Value], _: usize) -> R {
    let [f, xr, yr, rest @ ..] = args else { return Err("slopefield: expected (f, [x0 x1], [y0 y1][, y0s])".into()) };
    let pair = |v: &Value| match v.matrix()?.data[..] {
        [a, b] if a < b && a.is_finite() && b.is_finite() => Ok((a, b)),
        _ => Err("slopefield: ranges must be [lo hi] with lo < hi".to_string()),
    };
    let ((x0, x1), (y0, y1)) = (pair(xr)?, pair(yr)?);
    let h = match f {
        Value::Str(s) if letters(s).contains(&'t') && !letters(s).contains(&'x') => {
            first(it.call(&Value::from("str2func"), &[Value::Str(format!("@(t, y) {}", vectorize(s)))], 1))?
        }
        f => func(it, f, true)?,
    };
    // segments centered in a 20 x 15 grid, all the same length on screen (cell units)
    let (nx, ny) = (20, 15);
    let (dx, dy) = ((x1 - x0) / nx as f64, (y1 - y0) / ny as f64);
    let (gx, gy) = meshgrid(&linspace(x0 + dx / 2.0, x1 - dx / 2.0, nx), &linspace(y0 + dy / 2.0, y1 - dy / 2.0, ny));
    let s = sample(&[gx.clone(), gy.clone()], |v| first(it.call(&h, v, 1))).map_err(|e| format!("slopefield: {e}"))?;
    let (mut sx, mut sy) = (vec![], vec![]);
    for k in 0..s.len() {
        let (u, v): (f64, f64) = if s[k].is_infinite() { (0.0, 1.0) } else { (1.0, s[k] * dx / dy) };
        let l = u.hypot(v) / 0.36;
        if l.is_finite() {
            let (hx, hy) = (u / l * dx, v / l * dy);
            sx.extend([gx.data[k] - hx, gx.data[k] + hx, f64::NAN]);
            sy.extend([gy.data[k] - hy, gy.data[k] + hy, f64::NAN]);
        }
    }
    let mut out = vec![Series { color: Some(GRAY), width: 1.0, ..Series::new(Data::Line { x: sx, y: sy }) }];
    let starts = rest.first().map(Value::matrix).transpose()?.map_or(vec![], |m| m.data);
    for (k, &y) in starts.iter().enumerate() {
        let sol = it.call(&Value::from("ode45"), &[h.clone(), Value::from(vec![x0, x1]), Value::num(y)], 2);
        match sol.and_then(|r| Ok((r.first().ok_or("no t")?.matrix()?.data, r.get(1).ok_or("no y")?.matrix()?.data))) {
            Ok((t, v)) => out.push(Series { color: Some(palette(k)), width: 2.0, ..Series::new(Data::Line { x: t, y: v }) }),
            // ponytail: a solution that fails (blow-up) is skipped whole, not drawn up to the failure
            Err(e) => it.print(&format!("warning: slopefield: no solution from y = {y}: {e}\n")),
        }
    }
    if !starts.is_empty() {
        let n = starts.len();
        out.push(Series { color: Some(GRAY), line: LineStyle::None, marker: Marker::Point, width: 2.5, ..Series::new(Data::Line { x: vec![x0; n], y: starts }) });
    }
    let name = format!("y' = {}", pretty_fn(f, "x, y"));
    let ax = add(it, out);
    if !ax.hold {
        (ax.xlim, ax.ylim, ax.title) = (Some((x0, x1)), Some((y0, y1)), name);
    }
    touch(it)
}

/// matview(A): heatmap with each value written in its cell, row 1 on top; complex cells colored
/// by magnitude.
pub(super) fn matview(it: &mut Interp, args: &[Value], _: usize) -> R {
    let [v] = args else { return Err("matview: expected a matrix".into()) };
    let m = v.cmatrix()?;
    let (r, c) = (m.rows, m.cols);
    if r == 0 || c == 0 {
        return Err("matview: empty matrix".into());
    }
    // y runs down (reversed ylim), so image row 0 (drawn at the high y end) is A's last row
    let z = Matrix::new(r, c, (0..r * c).map(|k| m.get(r - 1 - k % r, k / r)).map(|v| if m.is_real() { v.re } else { v.norm() }).collect());
    let ax = add(it, vec![Series::new(Data::Image { z, x: (0.5, c as f64 + 0.5), y: (0.5, r as f64 + 0.5) })]);
    if !ax.hold {
        (ax.xlim, ax.ylim) = (Some((0.5, c as f64 + 0.5)), Some((r as f64 + 0.5, 0.5)));
        (ax.xlabel, ax.ylabel) = ("column".into(), "row".into());
        ax.title = if m.is_real() { format!("{r}×{c}") } else { format!("{r}×{c}, color = |a_{{ij}}|") };
    }
    // ponytail: values only up to 16x16, bigger matrices are just the heatmap
    if r <= 16 && c <= 16 {
        for k in 0..r * c {
            let (i, j, v) = (k % r, k / r, m.data[k]);
            let text = if v.im == 0.0 {
                num(v.re)
            } else if v.re == 0.0 {
                format!("{}i", num(v.im))
            } else {
                format!("{}{}{}i", num(v.re), if v.im < 0.0 { "\u{2212}" } else { "+" }, num(v.im.abs()))
            };
            ax.notes.push(Note { at: [j as f64 + 1.0, i as f64 + 1.0, f64::NAN], text, align: 0.5 });
        }
    }
    touch(it)
}

/// eigplot(A), A real 2x2: the unit circle and its image under A, the eigenvectors scaled by
/// their eigenvalues. Complex eigenvalues: circle and ellipse plus a note.
pub(super) fn eigplot(it: &mut Interp, args: &[Value], _: usize) -> R {
    let a = match args {
        [v] => v.matrix().ok().filter(|m| (m.rows, m.cols) == (2, 2) && m.data.iter().all(|x| x.is_finite())),
        _ => None,
    }
    .ok_or("eigplot: A must be a real 2x2 matrix")?;
    let th = linspace(0.0, std::f64::consts::TAU, 201);
    let (cx, cy): (Vec<f64>, Vec<f64>) = th.iter().map(|t| (t.cos(), t.sin())).unzip();
    let ex: Vec<f64> = cx.iter().zip(&cy).map(|(x, y)| a.get(0, 0) * x + a.get(0, 1) * y).collect();
    let ey: Vec<f64> = cx.iter().zip(&cy).map(|(x, y)| a.get(1, 0) * x + a.get(1, 1) * y).collect();
    let mut rmax = ex.iter().zip(&ey).fold(1.0f64, |m, (x, y)| m.max(x.hypot(*y)));
    let mut out = vec![
        Series { color: Some(GRAY), width: 1.0, line: LineStyle::Dashed, label: "|x| = 1".into(), ..Series::new(Data::Line { x: cx, y: cy }) },
        Series { color: Some(palette(0)), width: 2.0, label: "A x".into(), ..Series::new(Data::Line { x: ex, y: ey }) },
    ];
    let mut notes = vec![];
    let r = it.call(&Value::from("eig"), &[Value::Num(a.clone())], 2)?;
    match (r.first(), r.get(1)) {
        (Some(Value::Num(v)), Some(Value::Num(d))) => {
            for k in 0..2 {
                let l = d.get(k, k);
                let tip = [l * v.get(0, k), l * v.get(1, k)];
                rmax = rmax.max(l.abs());
                let label = format!("\\lambda_{} v_{}", k + 1, k + 1);
                out.push(Series { color: Some(palette(k + 1)), width: 2.0, label, ..Series::new(Data::Quiver { x: vec![0.0], y: vec![0.0], u: vec![tip[0]], v: vec![tip[1]] }) });
                let align = if tip[0] < 0.0 { 1.0 } else { 0.0 };
                notes.push(Note { at: [tip[0], tip[1], f64::NAN], text: format!("\\lambda_{} = {}", k + 1, num(l)), align });
            }
        }
        (Some(_), Some(Value::Complex(d))) => {
            let l = d.get(0, 0);
            notes.push(Note { at: [f64::NAN; 3], text: format!("\\lambda = {} \\pm {}i: no real eigenvectors", num(l.re), num(l.im.abs())), align: 0.0 });
        }
        _ => return Err("eigplot: eig returned no eigenvectors".into()),
    }
    let ax = add(it, out);
    if !ax.hold {
        round_limits(ax, &[rmax]);
        ax.grid = true;
        ax.legend = true;
        let g = |i, j| num(a.get(i, j));
        ax.title = format!("A = [{} {}; {} {}]", g(0, 0), g(0, 1), g(1, 0), g(1, 1));
    }
    for n in notes {
        if n.at[0].is_nan() {
            corner_note(ax, n.text);
        } else {
            ax.notes.push(n);
        }
    }
    touch(it)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plot::{Theme, render};

    fn s(x: &str) -> Value {
        Value::from(x)
    }
    fn n(x: f64) -> Value {
        Value::num(x)
    }

    #[test]
    fn shaded_parts() {
        let [p, m] = signed_parts(&[0.0, 1.0, 2.0], &[1.0, -1.0, -1.0]);
        let (p, m) = (p.unwrap(), m.unwrap());
        // crossing at 0.5 inserted, closed down to 0 at both ends
        assert_eq!(p.0, [0.0, 0.0, 0.5, 1.0, 2.0, 2.0]);
        assert_eq!(p.1, [0.0, 1.0, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(m.1, [0.0, 0.0, 0.0, -1.0, -1.0, 0.0]);
        assert!(signed_parts(&[0.0, 1.0], &[1.0, 2.0])[1].is_none());
    }

    #[test]
    fn calculus_builtins() {
        let mut it = Interp::new();
        // intplot: value returned, both signs shaded, note with the integral
        let v = intplot(&mut it, &[s("x.^2 - 1"), n(0.0), n(3.0)], 1).unwrap();
        assert!((v[0].scalar().unwrap() - 6.0).abs() < 1e-9);
        let a = it.plots.axes();
        assert_eq!(a.series.iter().filter(|s| matches!(s.data, Data::Fill { .. })).count(), 2);
        assert!(a.notes[0].text.contains("= 6") && a.title.contains('²'), "{} | {}", a.notes[0].text, a.title);
        // riemann: midpoint sum of x^2 on [0, 1], n = 4 is 0.328125
        let r = riemann(&mut it, &[s("x^2"), n(0.0), n(1.0), n(4.0), s("mid")], 2).unwrap();
        assert!((r[0].scalar().unwrap() - 0.328125).abs() < 1e-12 && (r[1].scalar().unwrap() - 1.0 / 3.0).abs() < 1e-9);
        assert!(it.plots.axes().notes[0].text.starts_with("M_{4} = 0.3281"));
        let r = riemann(&mut it, &[s("x"), n(0.0), n(2.0), n(3.0), s("trap")], 1).unwrap();
        assert!((r[0].scalar().unwrap() - 2.0).abs() < 1e-12);
        assert!(riemann(&mut it, &[s("x"), n(0.0), n(1.0), n(0.0)], 0).is_err());
        assert!(riemann(&mut it, &[s("x"), n(0.0), n(1.0), n(2.0), s("up")], 0).is_err());
        // tangent of x^2 at 1: slope 2, note, point marked
        let t = tangentplot(&mut it, &[s("x^2"), n(1.0)], 1).unwrap();
        assert!((t[0].scalar().unwrap() - 2.0).abs() < 1e-6);
        assert_eq!(it.plots.axes().notes[0].text, "f'(1) = 2");
        // slope field with t, two solutions plus their start markers
        slopefield(&mut it, &[s("t - y"), Value::from(vec![0.0, 3.0]), Value::from(vec![-2.0, 2.0]), Value::from(vec![-1.0, 1.0])], 0).unwrap();
        let a = it.plots.axes();
        assert_eq!((a.series.len(), a.title.as_str()), (4, "y' = t − y"));
        // matview: reversed y, one centered note per cell, complex as a+bi
        let z = Value::complex(crate::value::CMatrix::from_parts(&Matrix::row(vec![1.0, 0.0]), &Matrix::row(vec![-2.0, 3.0])));
        matview(&mut it, &[z], 0).unwrap();
        let a = it.plots.axes();
        assert_eq!(a.notes.iter().map(|n| n.text.as_str()).collect::<Vec<_>>(), ["1\u{2212}2i", "3i"]);
        assert_eq!((a.ylim, a.notes[1].at[0], a.notes[1].align), (Some((1.5, 0.5)), 2.0, 0.5));
        // eigplot: real eigenvalues of a symmetric matrix as arrows; complex ones as a note
        let m = Value::Num(Matrix::from_rows(&[vec![2.0, 1.0], vec![1.0, 3.0]]));
        eigplot(&mut it, &[m], 0).unwrap();
        let a = it.plots.axes();
        assert_eq!((a.series.len(), a.notes.len()), (4, 2));
        assert!(a.notes[1].text.contains("3.618"), "{}", a.notes[1].text);
        eigplot(&mut it, &[Value::Num(Matrix::from_rows(&[vec![0.0, -1.0], vec![1.0, 0.0]]))], 0).unwrap();
        assert!(it.plots.axes().notes[0].text.contains("\\pm 1i"));
        assert!(eigplot(&mut it, &[Value::Num(Matrix::zeros(3, 3))], 0).is_err());
        // each renders
        for th in [Theme::light(), Theme::dark()] {
            assert!(render::render_rgb(it.plots.fig(), &th, 320, 240).is_ok());
        }
    }

    /// Visual QA: `cargo test calculus_samples -- --ignored`, then look at samples/*.png.
    #[test]
    #[ignore]
    fn calculus_samples() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("samples");
        std::fs::create_dir_all(&dir).unwrap();
        let mut it = Interp::new();
        let mut run = |code: &str, name: &str| {
            it.eval(&format!("figure; {code}")).unwrap_or_else(|e| panic!("{name}: {e}"));
            for (th, t) in [(Theme::light(), "light"), (Theme::dark(), "dark")] {
                render::save(it.plots.fig(), &th, &dir.join(format!("{name}-{t}.png")), 800, 600).unwrap();
            }
        };
        run(
            "t = linspace(0, 5, 200); plot(t, exp(-t/1.5), t, t.^2/25, t, 0.5*exp(-t).*cos(4*t)); grid on; \
             title('Decay e^{-t/\\tau}, \\tau = 1.5 and x^2'); xlabel('time t_0 + \\Delta t [s]'); ylabel('v_0 \\cdot \\alpha^{n_k}'); \
             legend('e^{-t/\\tau}', 'x^2/25', '\\alpha_1 e^{-t} cos(\\omega t)')",
            "mathtext",
        );
        run("intplot('sin(x)', 0, 2*pi)", "intplot");
        run("intplot(@(x) x.^3 - 2*x, -1, 2)", "intplot-cubic");
        run("tangentplot('x.^3 - 3*x', 1.5)", "tangentplot");
        run("riemann(@(x) sin(x) + 1.2, 0, pi, 8, 'mid')", "riemann");
        run("riemann('exp(-x/2)', 0, 4, 6, 'trap')", "riemann-trap");
        run("slopefield('x - y', [-3 3], [-3 3], [-2 0 2])", "slopefield");
        run("matview(magic(4))", "matview");
        run("matview([1+2i, -0.5; 3, 1e-5])", "matview-complex");
        run("eigplot([2 1; 1 3])", "eigplot");
        run("eigplot([1 -2; 1 1])", "eigplot-complex");
        run(
            "x = linspace(0, 2*pi, 100); plot(x, sin(x)); hold on; area(x, 0.5*cos(x)); fill([1 2 2.5 1.5], [-0.5 -0.8 -0.2 0], 'g'); \
             text(pi/2, 1, 'max: sin(\\pi/2) = 1'); text([3*pi/2 2*pi], [-1 0], {'min', 'end'}); \
             text(4, 0.6, {'two', 'lines'}, 'HorizontalAlignment', 'center'); text(6.5, 0.3, 'clamped at the right edge'); \
             legend('sin', 'area', 'fill'); title('text, area, fill')",
            "notes",
        );
        run("fplot('exp(-x.^2/2).*cos(3*x)', [-4 4])", "fplot-title");
        run("fplot(@(x) x.^3 - 3*x, [-3 3]); hold on; fplot(@(x) x.^2 - 4*x + 3); grid on; keypoints on", "keypoints");
        run("x = linspace(0, 10, 300); plot(x, 3*exp(-(x-4).^2/3) - 1 + 0.15*sin(37*x.^2)); keypoints on", "keypoints-noisy");
        run("fimplicit('x.^2 + y.^2/4 - 1', [-3 3])", "fimplicit-title");
        run("polarplot(linspace(0, 2*pi, 200), 1 + cos(linspace(0, 2*pi, 200)))", "polarplot");
        render::save(it.plots.fig(), &Theme::light(), &dir.join("polarplot.svg"), 800, 600).unwrap();
        it.eval("figure; intplot('sin(x)', 0, 2*pi)").unwrap();
        render::save(it.plots.fig(), &Theme::light(), &dir.join("intplot.svg"), 800, 600).unwrap();
        it.eval("figure; surf(peaks(20)); hold on; text(5, 13, 8, 'peak \\alpha'); title('3D note')").unwrap();
        render::save(it.plots.fig(), &Theme::dark(), &dir.join("note3d-dark.png"), 800, 600).unwrap();
    }
}
