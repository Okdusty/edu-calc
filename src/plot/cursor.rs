//! Data cursor, shared by the GUI and the TUI: the renderer's frames map data <-> pixels; pick the
//! point under a pointer, walk a trace cursor along a series, format readouts.
use plotters::coord::CoordTranslate;

use super::render::Frame;
use super::{Data, Figure};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    /// nearest data point within `radius` pixels, any series of the axes under the pointer
    Nearest { radius: f64 },
    /// follow one series: 2D lines interpolate y at the pointer's x, other kinds take the nearest point
    Trace { series: usize },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub axes: usize,
    pub series: usize,
    /// point index (interpolated trace: the segment start); grids and images: column-major element
    pub index: usize,
    /// data coordinates; z is NaN where the data has none (2D lines, bars, arrows)
    pub point: [f64; 3],
    /// frame pixels
    pub screen: (f64, f64),
}

/// Data -> frame pixels.
pub fn project(f: &Frame, p: [f64; 3]) -> (f64, f64) {
    let b = &f.bounds;
    match &f.cube {
        Some((cs, u)) => {
            // the renderer's clamping; a missing z sits on 0 like 2D data in 3D axes
            let p = [p[0], p[1], if p[2].is_nan() { 0.0 } else { p[2] }];
            let q: [f64; 3] = std::array::from_fn(|k| p[k].max(b[k].0).min(b[k].1));
            let (x, y) = cs.translate(&(q[1], q[2], q[0]));
            (x as f64 / u, y as f64 / u)
        }
        None => {
            let [x0, y0, x1, y1] = f.rect;
            (to_px(b[0], f.logx, x0, x1, p[0]), to_px(b[1], f.logy, y1, y0, p[1]))
        }
    }
}

/// Frame pixels -> data (x, y) on a 2D frame; NaN on 3D ones (no unique inverse).
pub fn unproject(f: &Frame, (px, py): (f64, f64)) -> (f64, f64) {
    if f.cube.is_some() {
        return (f64::NAN, f64::NAN);
    }
    let [x0, y0, x1, y1] = f.rect;
    (from_px(f.bounds[0], f.logx, x0, x1, px), from_px(f.bounds[1], f.logy, y1, y0, py))
}

fn lg(v: f64, log: bool) -> f64 {
    if !log { v } else if v > 0.0 { v.log10() } else { f64::NAN }
}
fn to_px((lo, hi): (f64, f64), log: bool, p0: f64, p1: f64, v: f64) -> f64 {
    p0 + (lg(v, log) - lg(lo, log)) / (lg(hi, log) - lg(lo, log)) * (p1 - p0)
}
fn from_px((lo, hi): (f64, f64), log: bool, p0: f64, p1: f64, p: f64) -> f64 {
    let v = lg(lo, log) + (p - p0) / (p1 - p0) * (lg(hi, log) - lg(lo, log));
    if log { 10f64.powf(v) } else { v }
}

/// Number of pickable points of a series.
fn count(d: &Data) -> usize {
    match d {
        Data::Line { x, y } | Data::Fill { x, y } | Data::Bar { x, y, .. } => x.len().min(y.len()),
        Data::Line3 { x, y, z } => x.len().min(y.len()).min(z.len()),
        Data::Surface { z, .. } | Data::Contour { z, .. } | Data::Image { z, .. } => z.len(),
        Data::Quiver { x, y, u, v } => x.len().min(y.len()).min(u.len()).min(v.len()),
    }
}

/// Point `i` (< count) in data coordinates, None where the data has a gap.
/// Images: the cell center and its value.
// ponytail: arrows report their tip (compass/feather: the phasor itself), not (u, v)
fn vertex(d: &Data, i: usize) -> Option<[f64; 3]> {
    let nan = f64::NAN;
    let (p, has_z) = match d {
        Data::Line { x, y } | Data::Fill { x, y } | Data::Bar { x, y, .. } => ([x[i], y[i], nan], false),
        Data::Quiver { x, y, u, v } => ([x[i] + u[i], y[i] + v[i], nan], false),
        Data::Line3 { x, y, z } => ([x[i], y[i], z[i]], true),
        Data::Surface { x, y, z, .. } | Data::Contour { x, y, z, .. } => ([x.data[i], y.data[i], z.data[i]], true),
        Data::Image { z, x, y } => {
            let (r, c) = (i % z.rows, i / z.rows);
            let (dx, dy) = ((x.1 - x.0) / z.cols as f64, (y.1 - y.0) / z.rows as f64);
            ([x.0 + (c as f64 + 0.5) * dx, y.1 - (r as f64 + 0.5) * dy, z.data[i]], true)
        }
    };
    (p[0].is_finite() && p[1].is_finite() && (!has_z || p[2].is_finite())).then_some(p)
}

/// Cursor on point `index` of a series; None if it's missing, a gap or off screen (log axes).
fn at(fig: &Figure, frames: &[Frame], axes: usize, series: usize, index: usize) -> Option<Hit> {
    let f = frames.iter().find(|f| f.axes == axes)?;
    let d = &fig.axes.get(axes)?.series.get(series)?.data;
    let point = vertex(d, index).filter(|_| index < count(d))?;
    let screen = project(f, point);
    (screen.0.is_finite() && screen.1.is_finite()).then_some(Hit { axes, series, index, point, screen })
}

fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// Pixels from (x, y) to the frame's rect, 0 inside.
fn off(f: &Frame, x: f64, y: f64) -> f64 {
    let [x0, y0, x1, y1] = f.rect;
    (x0 - x).max(x - x1).max(0.0).hypot((y0 - y).max(y - y1).max(0.0))
}

/// What's under frame pixel (px, py), see `Mode`.
// ponytail: O(points) per call and no hidden-surface test in 3D; fine for hover rates
pub fn pick(fig: &Figure, frames: &[Frame], px: f64, py: f64, mode: Mode) -> Option<Hit> {
    // the axes under the pointer, or the nearest one (a trace keeps following past the edge)
    let f = frames.iter().min_by(|a, b| off(a, px, py).total_cmp(&off(b, px, py)))?;
    let a = fig.axes.get(f.axes)?;
    let last = a.series.len().checked_sub(1)?;
    let (series, radius) = match mode {
        Mode::Nearest { radius } if off(f, px, py) <= radius => (0..=last, radius),
        Mode::Nearest { .. } => return None,
        Mode::Trace { series } => (series.min(last)..=series.min(last), f64::INFINITY),
    };
    let flat = f.cube.is_none();
    let (x, y) = unproject(f, (px, py));
    let mut best: Option<(f64, Hit)> = None;
    for si in series {
        let d = &a.series[si].data;
        if !flat && matches!(d, Data::Bar { .. } | Data::Image { .. } | Data::Quiver { .. }) {
            continue; // not drawn in 3D axes
        }
        let hit = |i| at(fig, frames, f.axes, si, i);
        let special = match d {
            Data::Line { x: lx, y: ly } if flat && matches!(mode, Mode::Trace { .. }) => trace_line(f, lx, ly, px, py).map(|(index, screen)| {
                // on a point: its exact values
                let on = hit(index).filter(|h| dist(h.screen, screen) < 0.5);
                let (x, y) = unproject(f, screen);
                (0.0, on.unwrap_or(Hit { axes: f.axes, series: si, index, point: [x, y, f64::NAN], screen }))
            }),
            // the cell under the pointer; any point within reach wins over it
            Data::Image { z, x: xr, y: yr } if flat => {
                let c = ((x - xr.0) / (xr.1 - xr.0) * z.cols as f64).floor();
                let r = ((yr.1 - y) / (yr.1 - yr.0) * z.rows as f64).floor();
                let inside = c >= 0.0 && r >= 0.0 && c < z.cols as f64 && r < z.rows as f64;
                inside.then(|| r as usize + c as usize * z.rows).and_then(hit).map(|h| (radius, h))
            }
            Data::Bar { x: bx, y: by, width } if flat => {
                let base = if f.logy { f.bounds[1].0 } else { 0.0 };
                (0..count(d)).find(|&i| (x - bx[i]).abs() <= width / 2.0 && (y - base) * (y - by[i]) <= 0.0).and_then(hit).map(|h| (0.0, h))
            }
            _ => None,
        };
        let cand = special.or_else(|| (0..count(d)).filter_map(hit).map(|h| (dist(h.screen, (px, py)), h)).min_by(|a, b| a.0.total_cmp(&b.0)));
        if let Some(c) = cand
            && best.as_ref().is_none_or(|b| c.0 < b.0)
        {
            best = Some(c);
        }
    }
    best.filter(|(d, _)| *d <= radius).map(|(_, h)| h)
}

/// Trace a 2D line at screen x `px`, interpolating in screen space (so log axes work too). Where the
/// line passes that x several times, the crossing nearest `py`; past the ends, the nearest point.
/// Returns (point index at or before the spot, spot).
fn trace_line(f: &Frame, x: &[f64], y: &[f64], px: f64, py: f64) -> Option<(usize, (f64, f64))> {
    let s: Vec<(f64, f64)> = x.iter().zip(y).map(|(&a, &b)| project(f, [a, b, 0.0])).collect();
    let ok = |p: (f64, f64)| p.0.is_finite() && p.1.is_finite();
    let mut best: Option<(f64, usize, (f64, f64))> = None;
    for (i, w) in s.windows(2).enumerate() {
        let (p, q) = (w[0], w[1]);
        if !(ok(p) && ok(q)) || px < p.0.min(q.0) || px > p.0.max(q.0) {
            continue;
        }
        let t = if q.0 == p.0 { 0.0 } else { (px - p.0) / (q.0 - p.0) };
        let c = (px, p.1 + t * (q.1 - p.1));
        // sitting on the next point counts as that point, so stepping from here moves
        let i = if dist(c, q) < 0.5 { i + 1 } else { i };
        if best.is_none_or(|b| (c.1 - py).abs() < b.0) {
            best = Some(((c.1 - py).abs(), i, c));
        }
    }
    best.map(|(_, i, c)| (i, c)).or_else(|| {
        let near = (0..s.len()).filter(|&i| ok(s[i])).min_by(|&i, &j| (s[i].0 - px).abs().total_cmp(&(s[j].0 - px).abs()))?;
        Some((near, s[near]))
    })
}

/// Trace cursor keys: move `di` points along the series or `ds` series over (re-traced at the same
/// x). No cursor yet: the middle point of the first series of the current axes. At an end it stays.
pub fn step(fig: &Figure, frames: &[Frame], cur: Option<Hit>, di: isize, ds: isize) -> Option<Hit> {
    let Some(h) = cur else {
        let n = count(&fig.axes.get(fig.current)?.series.first()?.data);
        return (n / 2..n).chain((0..n / 2).rev()).find_map(|i| at(fig, frames, fig.current, 0, i));
    };
    let a = fig.axes.get(h.axes)?;
    let f = frames.iter().find(|f| f.axes == h.axes)?;
    if ds != 0 {
        let s = (h.series as isize + ds).rem_euclid(a.series.len().max(1) as isize) as usize;
        let (x, y) = project(f, h.point);
        return pick(fig, frames, x, y, Mode::Trace { series: s }).or(cur);
    }
    let d = &a.series.get(h.series)?.data;
    // an interpolated cursor sits between index and index + 1: the first step left lands on index
    let on = vertex(d, h.index).is_some_and(|p| dist(project(f, p), project(f, h.point)) < 0.5);
    let dir = if di < 0 { -1 } else { 1 };
    let mut i = h.index as isize + if di < 0 && !on { di + 1 } else { di };
    while (0..count(d) as isize).contains(&i) {
        if let Some(hit) = at(fig, frames, h.axes, h.series, i as usize) {
            return Some(hit);
        }
        i += dir; // skip gaps
    }
    cur
}

/// Pin within `radius` pixels of (px, py), nearest first: (axes, pin index).
pub fn pin_near(fig: &Figure, frames: &[Frame], px: f64, py: f64, radius: f64) -> Option<(usize, usize)> {
    let pins = frames.iter().filter_map(|f| Some((f, fig.axes.get(f.axes)?)));
    let all = pins.flat_map(|(f, a)| a.pins.iter().enumerate().map(move |(k, &p)| (dist(project(f, p), (px, py)), f.axes, k)));
    all.filter(|c| c.0 <= radius).min_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, a, k)| (a, k))
}

/// Readout: "x = 1.5, y = 0.9975[, z = …]" and, against the last of `pins` not at the cursor,
/// "Δx = …, Δy = …[, Δz = …], Δy/Δx = …" (oscilloscope-style cursor pair).
pub fn readout(p: [f64; 3], pins: &[[f64; 3]]) -> (String, Option<String>) {
    let mut s = format!("x = {}, y = {}", num(p[0]), num(p[1]));
    if p[2].is_finite() {
        s += &format!(", z = {}", num(p[2]));
    }
    let rel = pins.iter().rev().find(|q| (q[0], q[1]) != (p[0], p[1])).map(|q| {
        let (dx, dy) = (p[0] - q[0], p[1] - q[1]);
        let mut r = format!("Δx = {}, Δy = {}", num(dx), num(dy));
        if p[2].is_finite() && q[2].is_finite() {
            r += &format!(", Δz = {}", num(p[2] - q[2]));
        }
        if dx != 0.0 {
            r += &format!(", Δy/Δx = {}", num(dy / dx));
        }
        r
    });
    (s, rel)
}

/// Compact number: 4 significant digits, trailing zeros dropped, scientific outside 1e-4..1e6.
pub fn num(v: f64) -> String {
    if v == 0.0 {
        return "0".into(); // also -0
    }
    if !v.is_finite() {
        return format!("{v}");
    }
    let e = v.abs().log10().floor() as i32;
    let trim = |s: String| if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
    if (-4..6).contains(&e) {
        let dec = (3 - e).max(0) as usize;
        trim(format!("{v:.dec$}"))
    } else {
        let s = format!("{v:.3e}");
        let (m, x) = s.split_once('e').unwrap_or((&s, "0"));
        format!("{}e{x}", trim(m.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plot::render::render_frames;
    use crate::plot::{Series, Theme};
    use crate::value::Matrix;

    fn fig(series: Vec<Data>) -> Figure {
        let mut f = Figure::new(1);
        let a = f.axes();
        a.hold = true;
        series.into_iter().for_each(|d| a.add(Series::new(d)));
        f
    }
    fn frames(f: &Figure) -> Vec<Frame> {
        render_frames(f, &Theme::light(), 400, 300).unwrap().1
    }
    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn numbers() {
        assert_eq!(num(1.23456), "1.235");
        assert_eq!(num(0.944), "0.944");
        assert_eq!(num(-2.5), "-2.5");
        assert_eq!(num(1234567.0), "1.235e6");
        assert_eq!(num(0.00001234), "1.234e-5");
        assert_eq!(num(100.0), "100");
        assert_eq!((num(0.0), num(-0.0), num(f64::NAN)), ("0".into(), "0".into(), "NaN".into()));
        // Δ to the last pin that isn't the cursor itself (just pinned)
        let (a, b) = readout([3.0, 2.0, f64::NAN], &[[1.0, 1.0, f64::NAN], [3.0, 2.0, f64::NAN]]);
        assert_eq!((a.as_str(), b.as_deref()), ("x = 3, y = 2", Some("Δx = 2, Δy = 1, Δy/Δx = 0.5")));
        assert_eq!(readout([1.0, 2.0, 3.0], &[]), ("x = 1, y = 2, z = 3".into(), None));
    }

    #[test]
    fn project_round_trips() {
        let mut f = fig(vec![Data::Line { x: vec![1.0, 1000.0], y: vec![-2.0, 5.0] }]);
        for logx in [false, true] {
            f.axes[0].logx = logx;
            let fr = &frames(&f)[0];
            assert!(fr.cube.is_none() && fr.rect[0] > 0.0 && fr.rect[2] < 400.0);
            for p in [[1.0, -2.0], [31.6, 0.5], [1000.0, 5.0]] {
                let s = project(fr, [p[0], p[1], f64::NAN]);
                let (x, y) = unproject(fr, s);
                assert!(close(x, p[0], 1e-9 * p[0].abs()) && close(y, p[1], 1e-9), "{logx} {p:?} -> {s:?} -> {x} {y}");
            }
            // log: equal ratios, equal distances
            let px = |x| project(fr, [x, 0.0, 0.0]).0;
            assert_eq!(close(px(10.0) - px(1.0), px(100.0) - px(10.0), 1e-9), logx);
        }
    }

    #[test]
    fn nearest_and_trace_on_lines() {
        let f = fig(vec![Data::Line { x: vec![0.0, 1.0, 2.0, 3.0], y: vec![0.0, 10.0, 0.0, 10.0] }, Data::Line { x: vec![0.0, 3.0], y: vec![5.0, 5.0] }]);
        let fr = frames(&f);
        // nearest point near (1, 10), within reach only
        let (x, y) = project(&fr[0], [1.0, 10.0, 0.0]);
        let h = pick(&f, &fr, x + 3.0, y + 2.0, Mode::Nearest { radius: 24.0 }).unwrap();
        assert_eq!((h.series, h.index, h.point[..2].to_vec()), (0, 1, vec![1.0, 10.0]));
        assert!(pick(&f, &fr, x + 60.0, y + 60.0, Mode::Nearest { radius: 24.0 }).is_none());
        // trace: y interpolated at the pointer's x, whatever the pointer's y
        let (x, _) = project(&fr[0], [0.25, 0.0, 0.0]);
        let h = pick(&f, &fr, x, 0.0, Mode::Trace { series: 0 }).unwrap();
        assert!(close(h.point[0], 0.25, 1e-9) && close(h.point[1], 2.5, 1e-6), "{h:?}");
        assert_eq!(h.index, 0);
        // series 1 at the same x; past the end: the last point
        let h1 = step(&f, &fr, Some(h), 0, 1).unwrap();
        assert!(h1.series == 1 && close(h1.point[1], 5.0, 1e-9));
        let end = pick(&f, &fr, 399.0, 10.0, Mode::Trace { series: 0 }).unwrap();
        assert_eq!((end.index, end.point[..2].to_vec()), (3, vec![3.0, 10.0]));
        // stepping: from between points 0 and 1, right lands on 1 and left on 0; ends stay put
        assert_eq!(step(&f, &fr, Some(h), 1, 0).unwrap().index, 1);
        let left = step(&f, &fr, Some(h), -1, 0).unwrap();
        assert_eq!((left.index, left.point[1]), (0, 0.0));
        assert_eq!(step(&f, &fr, Some(left), -1, 0).unwrap().index, 0);
        assert_eq!(step(&f, &fr, Some(end), 1, 0).unwrap().index, 3);
        // no cursor yet: middle of the first series
        assert_eq!(step(&f, &fr, None, 0, 0).unwrap().index, 2);
    }

    #[test]
    fn image_bar_and_3d_picks() {
        let img = Matrix::from_rows(&[vec![1.0, 2.0, 3.0], vec![4.0, 5.0, 6.0]]);
        let f = fig(vec![Data::Image { z: img, x: (0.5, 3.5), y: (0.5, 2.5) }]);
        let fr = frames(&f);
        // row 1 is on top: (3, 2) is the top-right cell
        let (x, y) = project(&fr[0], [2.9, 2.1, 0.0]);
        let h = pick(&f, &fr, x, y, Mode::Nearest { radius: 24.0 }).unwrap();
        assert_eq!(h.point, [3.0, 2.0, 3.0]);
        let (x, y) = project(&fr[0], [1.2, 0.7, 0.0]);
        assert_eq!(pick(&f, &fr, x, y, Mode::Nearest { radius: 24.0 }).unwrap().point[2], 4.0);

        let f = fig(vec![Data::Bar { x: vec![1.0, 2.0, 3.0], y: vec![3.0, -1.0, 2.0], width: 0.8 }]);
        let fr = frames(&f);
        let (x, y) = project(&fr[0], [2.2, -0.3, 0.0]); // inside the negative bar, far from its end
        let h = pick(&f, &fr, x, y, Mode::Nearest { radius: 5.0 }).unwrap();
        assert_eq!((h.index, h.point[1]), (1, -1.0));
        let (x, y) = project(&fr[0], [2.2, 2.5, 0.0]); // above it: nothing
        assert!(pick(&f, &fr, x, y, Mode::Nearest { radius: 5.0 }).is_none());

        let (n, g) = (7, |i: usize| -3.0 + i as f64);
        let x = Matrix::new(n, n, (0..n * n).map(|k| g(k / n)).collect());
        let y = Matrix::new(n, n, (0..n * n).map(|k| g(k % n)).collect());
        let z = Matrix::new(n, n, x.data.iter().zip(&y.data).map(|(a, b)| (a * b) / 3.0).collect());
        let mut f = fig(vec![Data::Surface { x, y, z, wire: false }]);
        f.axes[0].view.yaw = 0.7;
        let fr = frames(&f);
        assert!(fr[0].cube.is_some());
        for k in [0, 10, 24, 48] {
            let Data::Surface { x, y, z, .. } = &f.axes[0].series[0].data else { unreachable!() };
            let p = [x.data[k], y.data[k], z.data[k]];
            let (sx, sy) = project(&fr[0], p);
            let h = pick(&f, &fr, sx + 1.0, sy, Mode::Nearest { radius: 24.0 }).unwrap();
            assert_eq!((h.index, h.point), (k, p));
        }
        assert!(unproject(&fr[0], (1.0, 1.0)).0.is_nan());
    }

    #[test]
    fn pins_draw_and_are_found() {
        let th = Theme::dark();
        let mut f = fig(vec![Data::Line { x: vec![0.0, 10.0], y: vec![0.0, 0.0] }]);
        f.axes[0].ylim = Some((-1.0, 9.0));
        let (bare, fr) = render_frames(&f, &th, 400, 300).unwrap();
        f.axes[0].pins.push([5.0, 6.0, f64::NAN]);
        let (px, _) = render_frames(&f, &th, 400, 300).unwrap();
        let (x, y) = project(&fr[0], [5.0, 6.0, 0.0]);
        // the dot and its tip box (up and right of it) add ink where there was none
        let changed = |x0: f64, y0: f64, w: f64, h: f64| {
            let mut n = 0;
            for j in y0 as usize..(y0 + h) as usize {
                for i in x0 as usize..(x0 + w) as usize {
                    let k = (j * 400 + i) * 3;
                    n += (px[k..k + 3] != bare[k..k + 3]) as usize;
                }
            }
            n
        };
        assert!(changed(x - 3.0, y - 3.0, 6.0, 6.0) > 15);
        assert!(changed(x + 4.0, y - 40.0, 50.0, 36.0) > 100);
        assert_eq!(pin_near(&f, &fr, x + 5.0, y, 24.0), Some((0, 0)));
        assert_eq!(pin_near(&f, &fr, x + 50.0, y, 24.0), None);
    }
}
