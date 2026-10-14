//! Key points of a sampled curve y(x): local extrema, roots, y-intercept, inflection points, the
//! vertex of a parabola and the curve's symmetry. Pure and cheap, so the renderer recomputes them
//! from whatever samples are there and they follow resampling.
//!
//! Samples: x ascending; NaN gaps at poles (big jumps are split here too, like fplot does, and
//! runs under 3 samples dropped), so a sign flip through a pole is no root and a pole no extremum.
//! Noise: σ comes from 4th differences (≈ 0 on smooth curves, std √70 σ on white noise; small
//! data sets count as noise-free). Extrema need a retracement and roots / inflections a sign change
//! between samples well clear of the noise, so jitter makes no key points. On smooth, dense curves
//! extrema are refined through the parabola of the 3 samples around them.
use super::builtins::{break_jumps, range, robust_ylim};
use super::cursor;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct KeyPoints {
    pub max: Vec<[f64; 2]>,
    pub min: Vec<[f64; 2]>,
    pub roots: Vec<[f64; 2]>,
    pub inflection: Vec<[f64; 2]>,
    /// (0, f(0))
    pub y_int: Option<[f64; 2]>,
    /// the extremum, when the curve is a parabola
    pub vertex: Option<[f64; 2]>,
    pub symmetry: Symmetry,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Symmetry {
    #[default]
    None,
    /// f(−x) = f(x)
    Even,
    /// f(−x) = −f(x)
    Odd,
    /// mirror axis x = h through an extremum
    Axis(f64),
    /// point symmetric about an inflection point (h, k)
    Point(f64, f64),
}

impl Symmetry {
    /// "even", "odd", "x = 2", "point (0, 1)" or "none"
    pub fn text(&self) -> String {
        match *self {
            Symmetry::None => "none".into(),
            Symmetry::Even => "even".into(),
            Symmetry::Odd => "odd".into(),
            Symmetry::Axis(h) => format!("x = {}", cursor::num(h)),
            Symmetry::Point(h, k) => format!("point ({}, {})", cursor::num(h), cursor::num(k)),
        }
    }
}

/// Finite samples split into runs (at least 3 samples each) at gaps and jumps.
struct Curve {
    x: Vec<f64>,
    y: Vec<f64>,
    /// [start, end) of each run
    runs: Vec<(usize, usize)>,
}

impl Curve {
    /// Quadratic interpolation at `q` and an error bound for it (from the 2nd difference there);
    /// None in a gap.
    fn at(&self, q: f64) -> Option<(f64, f64)> {
        let (x, y) = (&self.x, &self.y);
        let &(a, b) = self.runs.iter().find(|&&(a, b)| x[a] <= q && q <= x[b - 1])?;
        // samples j - 1, j, j + 1 around q
        let j = (a + x[a..b].partition_point(|&v| v < q)).clamp(a + 1, b - 2);
        let [(x0, y0), (x1, y1), (x2, y2)] = [j - 1, j, j + 1].map(|i| (x[i], y[i]));
        let v = y0 * (q - x1) * (q - x2) / ((x0 - x1) * (x0 - x2)) + y1 * (q - x0) * (q - x2) / ((x1 - x0) * (x1 - x2)) + y2 * (q - x0) * (q - x1) / ((x2 - x0) * (x2 - x1));
        Some((v, (y2 - 2.0 * y1 + y0).abs() / 4.0))
    }
    /// Every sample maps onto the curve under x -> 2h - x, y -> y (k None) or 2k - y, within `tol`,
    /// over enough of the curve to mean something.
    fn mirrors(&self, h: f64, k: Option<f64>, tol: f64) -> bool {
        let mut n = 0;
        for (&x, &y) in self.x.iter().zip(&self.y) {
            let Some((v, e)) = self.at(2.0 * h - x) else { continue };
            if (v - k.map_or(y, |k| 2.0 * k - y)).abs() > tol + e {
                return false;
            }
            n += 1;
        }
        n >= (self.x.len() / 5).max(5)
    }
}

/// Key points of y(x); None without at least 5 usable samples in increasing x.
pub fn analyze(x: &[f64], y: &[f64]) -> Option<KeyPoints> {
    let n = x.len().min(y.len());
    // a lone NaN between samples that agree (sin(x)/x at 0) is a hole, not a pole: skip it
    let hole = |i: usize| {
        let f = |k: usize| y.get(k).copied().unwrap_or(f64::NAN);
        let (a, b) = (f(i.wrapping_sub(1)), f(i + 1));
        !f(i).is_finite() && a.is_finite() && b.is_finite() && (b - a).abs() <= 2.0 * ((a - f(i.wrapping_sub(2))).abs() + (f(i + 2) - b).abs())
    };
    let (x, y): (Vec<f64>, Vec<f64>) = (0..n).filter(|&i| !hole(i)).map(|i| (x[i], y[i])).unzip();
    let (bx, by) = break_jumps(&x, &y);
    let mut c = Curve { x: vec![], y: vec![], runs: vec![] };
    let mut start = 0;
    for (&a, &b) in bx.iter().zip(&by).chain([(&f64::NAN, &f64::NAN)]) {
        if a.is_finite() && b.is_finite() {
            c.x.push(a);
            c.y.push(b);
        } else if c.x.len() - start < 3 {
            c.x.truncate(start); // a lone sample or two next to a pole
            c.y.truncate(start);
        } else {
            c.runs.push((start, c.x.len()));
            start = c.x.len();
        }
    }
    // ponytail: parametric / unsorted lines get nothing; sorting would scramble them
    if c.x.len() < 5 || c.x.windows(2).any(|w| !(w[1] > w[0])) {
        return None;
    }
    let (xs, ys) = (&c.x, &c.y);
    let (lo, hi) = robust_ylim(ys).unwrap_or_else(|| range(ys));
    let r = hi - lo;
    let xspan = xs[xs.len() - 1] - xs[0];
    let mut d4: Vec<f64> = c.runs.iter().flat_map(|&(a, b)| (a + 2..b.saturating_sub(2)).map(|i| (ys[i - 2] - 4.0 * ys[i - 1] + 6.0 * ys[i] - 4.0 * ys[i + 1] + ys[i + 2]).abs())).collect();
    let few = d4.len() < 20;
    let sigma = if few { 0.0 } else { median(&mut d4) / 5.64 }; // median |N(0, 70σ²)| = 0.674·√70 σ
    let smooth = sigma <= r * 1e-6;
    let eps = r * 1e-9;
    let mut k = KeyPoints::default();

    // extrema: zigzag turning points, plateaus skipped
    let tol = (12.0 * sigma).max(r * 1e-6);
    let mut piv = vec![];
    for &(a, b) in &c.runs {
        for (i, up) in pivots(&ys[a..b], tol) {
            let i = a + i;
            let flat = |j: usize| j < b && (ys[j] - ys[i]).abs() <= eps;
            if !(flat(i + 1) && flat(i + 2)) {
                piv.push((i, up));
            }
        }
    }
    for &(i, up) in &piv {
        let p = if smooth && !few { peak(xs, ys, i) } else { [xs[i], ys[i]] };
        if up { k.max.push(p) } else { k.min.push(p) }
    }

    // roots: sign changes between samples clear of zero (the middle one of a noisy cluster; a
    // stretch of exact zeros is no root), plus extrema touching zero
    let tol = (6.0 * sigma).max(eps);
    for &(a, b) in &c.runs {
        let mut last: Option<usize> = None;
        for j in (a..b).filter(|&j| ys[j].abs() > tol) {
            if let Some(i) = last
                && (ys[i] > 0.0) != (ys[j] > 0.0)
                && (i..j).filter(|&m| ys[m] == 0.0).count() < 3
            {
                let cross: Vec<(usize, f64)> = (i..j).filter_map(|m| Some((m, crossing(xs, ys, m)?))).collect();
                if let Some(&(m, x)) = cross.get(cross.len() / 2) {
                    // one secant step on the quadratic interpolant
                    let slope = (ys[m + 1] - ys[m]) / (xs[m + 1] - xs[m]);
                    let x = match c.at(x) {
                        Some((v, _)) if ys[m] != 0.0 && (x - v / slope).is_finite() => (x - v / slope).clamp(xs[m], xs[m + 1]),
                        _ => x,
                    };
                    k.roots.push([x, 0.0]);
                }
            }
            last = Some(j);
        }
    }
    if smooth {
        k.roots.extend(k.max.iter().chain(&k.min).filter(|p| p[1].abs() <= tol).map(|p| [p[0], 0.0]));
    }
    k.roots.sort_by(|p, q| p[0].total_cmp(&q[0]));

    k.y_int = c.at(0.0).map(|(v, _)| [0.0, v]);

    // inflections: sign changes of the curvature, each side held for 2 confident samples (a
    // staircase's +/- spikes are no inflections)
    // ponytail: curvature measured as a 2nd difference assumes roughly even spacing for the noise bound
    let mut g = vec![f64::NAN; xs.len()]; // 2nd divided differences, f''/2
    for &(a, b) in &c.runs {
        for i in a + 1..b - 1 {
            let (sl, sr) = ((ys[i] - ys[i - 1]) / (xs[i] - xs[i - 1]), (ys[i + 1] - ys[i]) / (xs[i + 1] - xs[i]));
            g[i] = (sr - sl) / (xs[i + 1] - xs[i - 1]);
        }
    }
    let tol = (12.0 * sigma).max(eps);
    for &(a, b) in &c.runs {
        // last held sign and its latest sample; the current streak (sign, first sample, length)
        let (mut held, mut streak): (Option<(bool, usize)>, Option<(bool, usize, usize)>) = (None, None);
        for j in (a + 1..b - 1).filter(|&j| (g[j] * (xs[j + 1] - xs[j - 1]).powi(2) / 2.0).abs() > tol) {
            let s = g[j] > 0.0;
            let (_, first, len) = *streak.insert(match streak {
                Some((t, f, l)) if t == s => (t, f, l + 1),
                _ => (s, j, 1),
            });
            if len < 2 {
                continue;
            }
            if let Some((t, i)) = held
                && t != s
                && len == 2
            {
                let cross: Vec<f64> = (i..first).filter_map(|m| crossing(xs, &g, m)).collect();
                if let Some(&x) = cross.get(cross.len() / 2)
                    && let Some((v, _)) = c.at(x)
                {
                    k.inflection.push([x, v]);
                }
            }
            held = Some((s, j));
        }
    }

    // vertex: one extremum and constant curvature
    if smooth && c.runs.len() == 1 && piv.len() == 1 {
        let mut f2: Vec<f64> = g.iter().copied().filter(|v| v.is_finite()).collect();
        let med = median(&mut f2);
        if med != 0.0 && f2.iter().all(|v| (v - med).abs() <= 1e-3 * med.abs()) {
            let v = peak(xs, ys, piv[0].0); // exact for a parabola
            if piv[0].1 { k.max[0] = v } else { k.min[0] = v }
            k.vertex = Some(v);
        }
    }

    // symmetry: even, odd, a mirror axis through an extremum, a center at an inflection point
    // (candidates nearest the middle first)
    if r > eps {
        let tol = r * 1e-6 + 8.0 * sigma;
        let mid = (xs[0] + xs[xs.len() - 1]) / 2.0;
        let near = |v: &[[f64; 2]]| {
            let mut v = v.to_vec();
            v.sort_by(|p, q| (p[0] - mid).abs().total_cmp(&(q[0] - mid).abs()));
            v.truncate(3);
            v
        };
        let ext: Vec<[f64; 2]> = k.max.iter().chain(&k.min).copied().collect();
        k.symmetry = if c.mirrors(0.0, None, tol) {
            Symmetry::Even
        } else if c.mirrors(0.0, Some(0.0), tol) {
            Symmetry::Odd
        } else if let Some(p) = near(&ext).into_iter().find(|p| c.mirrors(p[0], None, tol)) {
            Symmetry::Axis(p[0])
        } else if let Some(p) = near(&k.inflection).into_iter().find(|p| c.mirrors(p[0], Some(p[1]), tol)) {
            Symmetry::Point(p[0], p[1])
        } else {
            Symmetry::None
        };
    }

    // display-ready numbers: rounding noise to 0, near-integers to integers
    let t = |p: &mut [f64; 2]| *p = [tidy(p[0], xspan), tidy(p[1], r)];
    k.max.iter_mut().chain(&mut k.min).chain(&mut k.roots).chain(&mut k.inflection).chain(&mut k.y_int).chain(&mut k.vertex).for_each(t);
    k.symmetry = match k.symmetry {
        Symmetry::Axis(h) => Symmetry::Axis(tidy(h, xspan)),
        Symmetry::Point(h, v) => Symmetry::Point(tidy(h, xspan), tidy(v, r)),
        s => s,
    };
    Some(k)
}

fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let m = v.len() / 2;
    *v.select_nth_unstable_by(m, f64::total_cmp).1
}

/// Turning points of `y` that retrace by more than `tol` (zigzag): (index, is max). The ends
/// never count.
fn pivots(y: &[f64], tol: f64) -> Vec<(usize, bool)> {
    let mut out = vec![];
    let (mut dir, mut ext) = (0.0, 0);
    for i in 1..y.len() {
        if dir == 0.0 {
            if (y[i] - y[0]).abs() > tol {
                (dir, ext) = ((y[i] - y[0]).signum(), i);
            }
        } else if (y[i] - y[ext]) * dir > 0.0 {
            ext = i;
        } else if (y[ext] - y[i]) * dir > tol {
            out.push((ext, dir > 0.0));
            (dir, ext) = (-dir, i);
        }
    }
    out
}

/// Vertex of the parabola through samples i - 1, i, i + 1 (kept within them).
fn peak(x: &[f64], y: &[f64], i: usize) -> [f64; 2] {
    let (a, b) = (x[i - 1] - x[i], x[i + 1] - x[i]);
    let (fa, fb) = (y[i - 1] - y[i], y[i + 1] - y[i]);
    let c2 = (fa / a - fb / b) / (a - b);
    let c1 = fa / a - c2 * a;
    let t = -c1 / (2.0 * c2);
    if t.is_finite() && t >= a && t <= b { [x[i] + t, y[i] + c1 * t / 2.0] } else { [x[i], y[i]] }
}

/// Where v crosses zero from sample m to m + 1: an exact zero at m, or a sign change interpolated.
fn crossing(x: &[f64], v: &[f64], m: usize) -> Option<f64> {
    let (a, b) = (v[m], *v.get(m + 1)?);
    if a == 0.0 {
        Some(x[m])
    } else if a * b < 0.0 {
        Some(x[m] + a / (a - b) * (x[m + 1] - x[m]))
    } else {
        None
    }
}

/// |v| below span·1e-9 -> 0, within span·1e-5 of an integer (well under a pixel) -> that integer.
fn tidy(v: f64, span: f64) -> f64 {
    let s = span.abs();
    if v.abs() < s * 1e-9 {
        0.0
    } else if (v - v.round()).abs() < s * 1e-5 {
        v.round() + 0.0 // no -0
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plot::builtins::linspace;

    fn of(f: impl Fn(f64) -> f64, lo: f64, hi: f64, n: usize) -> KeyPoints {
        let x = linspace(lo, hi, n);
        let y: Vec<f64> = x.iter().map(|&v| f(v)).collect();
        analyze(&x, &y).unwrap()
    }
    fn near(p: &[[f64; 2]], q: &[[f64; 2]], tol: f64) -> bool {
        p.len() == q.len() && p.iter().zip(q).all(|(a, b)| (a[0] - b[0]).abs() < tol && (a[1] - b[1]).abs() < tol)
    }

    #[test]
    fn parabola() {
        // off-grid sampling: refined, then snapped to integers
        let k = of(|x| x * x - 4.0 * x + 3.0, -5.3, 7.1, 777);
        assert_eq!(k.vertex, Some([2.0, -1.0]));
        assert_eq!((k.min.clone(), k.max.len()), (vec![[2.0, -1.0]], 0));
        assert_eq!(k.roots, [[1.0, 0.0], [3.0, 0.0]]);
        assert_eq!(k.y_int, Some([0.0, 3.0]));
        assert!(k.inflection.is_empty());
        assert_eq!(k.symmetry, Symmetry::Axis(2.0));
        assert_eq!(k.symmetry.text(), "x = 2");
        // a touching root, and an even parabola
        let k = of(|x| (x - 1.0) * (x - 1.0), -4.0, 4.0, 1000);
        assert_eq!(k.roots, [[1.0, 0.0]]);
        assert_eq!(of(|x| x * x, -3.0, 3.0, 300).symmetry, Symmetry::Even);
        // a coarse parabola still gets its vertex off the samples
        let k = analyze(&[-3., -2., -1., 0., 1., 2., 3.], &[12.25, 6.25, 2.25, 0.25, 0.25, 2.25, 6.25]).unwrap();
        assert_eq!(k.vertex, Some([0.5, 0.0]));
    }

    #[test]
    fn trig_and_cubic() {
        use std::f64::consts::{FRAC_PI_2, PI};
        let k = of(f64::sin, -5.0, 5.0, 1001);
        let (a, b) = (3.0 * FRAC_PI_2, FRAC_PI_2);
        assert!(near(&k.max, &[[-a, 1.0], [b, 1.0]], 1e-5) && near(&k.min, &[[-b, -1.0], [a, -1.0]], 1e-5), "{k:?}");
        assert!(near(&k.roots, &[[-PI, 0.0], [0.0, 0.0], [PI, 0.0]], 1e-6), "{:?}", k.roots);
        assert!(near(&k.inflection, &[[-PI, 0.0], [0.0, 0.0], [PI, 0.0]], 1e-3), "{:?}", k.inflection);
        assert_eq!((k.vertex, k.symmetry), (None, Symmetry::Odd));
        assert_eq!(of(f64::cos, -4.0, 6.0, 500).symmetry, Symmetry::Even);
        // x³ − 3x: extrema at ±1, roots 0 and ±√3, inflection at the origin, odd
        let k = of(|x| x * x * x - 3.0 * x, -2.5, 2.5, 1001);
        assert!(near(&k.max, &[[-1.0, 2.0]], 1e-9) && near(&k.min, &[[1.0, -2.0]], 1e-9), "{k:?}");
        assert!(near(&k.roots, &[[-3f64.sqrt(), 0.0], [0.0, 0.0], [3f64.sqrt(), 0.0]], 1e-6), "{:?}", k.roots);
        assert_eq!((k.inflection.clone(), k.symmetry, k.vertex, k.y_int), (vec![[0.0, 0.0]], Symmetry::Odd, None, Some([0.0, 0.0])));
        // shifted cubic: point symmetric about its inflection
        let k = of(|x| (x - 1.0).powi(3) - 3.0 * (x - 1.0) + 2.0, -2.0, 3.5, 900);
        assert_eq!(k.symmetry, Symmetry::Point(1.0, 2.0));
        assert_eq!(k.symmetry.text(), "point (1, 2)");
        // shifted cosine: mirror axis through its maximum
        assert_eq!(of(|x| (x - 1.0).cos() + 0.3 * (x - 1.0).powi(2), -3.0, 6.0, 900).symmetry, Symmetry::Axis(1.0));
    }

    #[test]
    fn poles() {
        // 1/x: no roots or extrema through the pole, no y-intercept, odd
        for n in [1000, 1001] {
            let k = of(|x| 1.0 / x, -5.0, 5.0, n);
            assert!(k.roots.is_empty() && k.max.is_empty() && k.min.is_empty() && k.y_int.is_none(), "{n}: {k:?}");
            assert_eq!(k.symmetry, Symmetry::Odd);
        }
        // tan: roots and inflections at kπ only, nothing at the poles
        use std::f64::consts::PI;
        let k = of(f64::tan, -4.0, 4.0, 1001);
        assert!(near(&k.roots, &[[-PI, 0.0], [0.0, 0.0], [PI, 0.0]], 1e-4), "{:?}", k.roots);
        assert!(near(&k.inflection, &[[-PI, 0.0], [0.0, 0.0], [PI, 0.0]], 1e-3), "{:?}", k.inflection);
        assert!(k.max.is_empty() && k.min.is_empty(), "{k:?}");
        assert_eq!(k.symmetry, Symmetry::Odd);
        // sin(x)/x: the 0/0 sample is a hole, its maximum is still there
        let k = of(|x| x.sin() / x, -10.0, 10.0, 1001);
        assert!(k.max.len() == 3 && k.max[1] == [0.0, 1.0] && k.y_int == Some([0.0, 1.0]) && k.symmetry == Symmetry::Even, "{k:?}");
        // 1/x² is even, with a pole and no extremum at 0
        let k = of(|x| 1.0 / (x * x), -3.0, 3.0, 600);
        assert!(k.max.is_empty() && k.min.is_empty() && k.roots.is_empty() && k.y_int.is_none(), "{k:?}");
        assert_eq!(k.symmetry, Symmetry::Even);
    }

    #[test]
    fn noise_steps_constants_and_bad_input() {
        // deterministic noise of ±0.05 on sin: its two extrema and three roots, no inflections
        let mut s = 12345u64;
        let mut rnd = || {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (s >> 11) as f64 / (1u64 << 53) as f64 - 0.5
        };
        let x = linspace(-5.0, 5.0, 400);
        let y: Vec<f64> = x.iter().map(|&v| v.sin() + 0.1 * rnd()).collect();
        let k = analyze(&x, &y).unwrap();
        assert_eq!((k.max.len(), k.min.len(), k.roots.len(), k.inflection.len()), (1, 1, 3, 0), "{k:?}");
        assert!((k.max[0][0] - 1.57).abs() < 0.3 && (k.roots[1][0]).abs() < 0.2, "{k:?}");
        // constants: nothing but the intercept
        for c in [0.0, 3.0] {
            let k = of(|_| c, -2.0, 2.0, 50);
            assert_eq!(k, KeyPoints { y_int: Some([0.0, c]), ..KeyPoints::default() });
        }
        // a line: its root and intercept
        let k = of(|x| 2.0 * x + 1.0, -3.0, 3.0, 61);
        assert_eq!((k.roots.clone(), k.y_int), (vec![[-0.5, 0.0]], Some([0.0, 1.0])));
        assert!(k.inflection.is_empty() && k.max.is_empty() && k.symmetry == Symmetry::None);
        // plateau tops and staircases: no extrema, roots or inflections
        let k = of(|x| (2.0 - x.abs()).min(1.0), -3.0, 3.0, 601);
        assert!(k.max.is_empty() && k.min.is_empty(), "{k:?}");
        let k = of(f64::floor, -3.0, 3.0, 601);
        assert!(k.roots.is_empty() && k.inflection.is_empty() && k.max.is_empty(), "{k:?}");
        // a few data points: every turn counts, unrefined
        let k = analyze(&[1., 2., 3., 4., 5., 6., 7.], &[1., 3., 5., 7., 5., 3., 1.]).unwrap();
        assert_eq!((k.max.clone(), k.vertex, k.symmetry), (vec![[4.0, 7.0]], None, Symmetry::Axis(4.0)));
        // too few, unsorted, NaN-only, mismatched lengths, infinities
        assert!(analyze(&[1., 2., 3.], &[1., 2., 1.]).is_none());
        assert!(analyze(&[0., 1., 2., 1., 0., 3.], &[0., 1., 0., 1., 0., 1.]).is_none());
        assert!(analyze(&[f64::NAN; 9], &[1.0; 9]).is_none());
        assert!(analyze(&linspace(0., 1., 9), &[1.0; 4]).is_none());
        let k = analyze(&linspace(0., 1., 9), &[f64::INFINITY, 0., 1., 2., 1., 0., f64::NAN, 5., 6.]).unwrap();
        assert_eq!(k.max.len(), 1);
    }
}
