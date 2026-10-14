//! Octave plotting commands (plot, surf, xlabel, hold, subplot, ...) writing into `Interp::plots`.
use std::borrow::Cow;
use std::path::Path;

use super::{Axes, Data, Figure, LineStyle, Marker, Note, Rgb, Series, Source, Theme, View, analysis, calculus, cursor, render};
use crate::lang::Interp;
use crate::value::{Builtin, Func, Matrix, Value};

type R = Result<Vec<Value>, String>;

pub fn lookup(name: &str) -> Option<Builtin> {
    let f: Builtin = match name {
        "plot" => plot,
        "semilogx" => semilogx,
        "semilogy" => semilogy,
        "loglog" => loglog,
        "plot3" => plot3,
        "scatter" => scatter,
        "scatter3" => scatter3,
        "bar" => bar,
        "stairs" => stairs,
        "stem" => stem,
        "hist" | "histogram" => hist,
        "polar" | "polarplot" => polar,
        "errorbar" => errorbar,
        "surf" => surf,
        "mesh" => mesh,
        "contour" => contour,
        "contourf" => contourf,
        "imagesc" | "image" => imagesc,
        "pcolor" => pcolor,
        "quiver" => quiver,
        "compass" => compass,
        "feather" => feather,
        "datatip" => datatip,
        "keypoints" => keypoints,
        "text" => text,
        "area" => area,
        "fill" => fill,
        "intplot" => calculus::intplot,
        "tangentplot" => calculus::tangentplot,
        "riemann" => calculus::riemann,
        "slopefield" => calculus::slopefield,
        "matview" => calculus::matview,
        "eigplot" => calculus::eigplot,
        "peaks" => peaks,
        "sphere" => sphere,
        "cylinder" => cylinder,
        "fplot" | "ezplot" => fplot,
        "fsurf" | "ezsurf" => fsurf,
        "fmesh" | "ezmesh" => fmesh,
        "fcontour" | "ezcontour" => fcontour,
        "fimplicit" => fimplicit,
        "title" => title,
        "xlabel" => xlabel,
        "ylabel" => ylabel,
        "zlabel" => zlabel,
        "legend" => legend,
        "grid" => grid,
        "hold" => hold,
        "ishold" => ishold,
        "axis" => axis,
        "xlim" => xlim,
        "ylim" => ylim,
        "zlim" => zlim,
        "view" => view,
        "colormap" => colormap,
        "figure" => figure,
        "clf" => clf,
        "close" => close,
        "subplot" => subplot,
        "print" | "saveas" => print,
        // ponytail: no colorbar (renderer has none); drawnow/shg have nothing to do
        "drawnow" | "shg" | "colorbar" => |_, _, _| Ok(vec![]),
        _ => return None,
    };
    Some(f)
}

// ---- helpers ----

pub(super) fn touch(it: &mut Interp) -> R {
    it.plots.touch();
    Ok(vec![])
}

fn is_num(v: &Value) -> bool {
    matches!(v, Value::Num(_) | Value::Bool(_) | Value::Complex(_))
}

fn warn_imag(it: &mut Interp, name: &str) {
    it.print(&format!("warning: {name}: imaginary parts of complex X and/or Y arguments ignored\n"));
}

/// Complex args -> their real parts, warning once per call like Octave.
fn real_args<'a>(it: &mut Interp, name: &str, args: &'a [Value]) -> Cow<'a, [Value]> {
    if !args.iter().any(Value::is_complex) {
        return Cow::Borrowed(args);
    }
    warn_imag(it, name);
    args.iter().map(|v| if let Value::Complex(m) = v { Value::Num(m.re()) } else { v.clone() }).collect()
}

/// Leading numeric args as matrices, and the rest.
fn leading(args: &[Value]) -> Result<(Vec<Matrix>, &[Value]), String> {
    let n = args.iter().take_while(|v| is_num(v)).count();
    Ok((args[..n].iter().map(Value::matrix).collect::<Result<_, _>>()?, &args[n..]))
}

/// Numbers from a numeric value or command-syntax text like "221" or "[0 1]".
fn numbers(v: &Value) -> Result<Vec<f64>, String> {
    match v {
        Value::Str(s) => s
            .trim_matches(|c| c == '[' || c == ']')
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|t| !t.is_empty())
            .map(|t| t.parse::<f64>().map_err(|_| format!("expected a number, got '{s}'")))
            .collect(),
        v => Ok(v.matrix()?.data),
    }
}

pub(super) fn linspace(a: f64, b: f64, n: usize) -> Vec<f64> {
    if n < 2 {
        return vec![b; n];
    }
    (0..n).map(|i| a + (b - a) * i as f64 / (n - 1) as f64).collect()
}

pub(super) fn meshgrid(x: &[f64], y: &[f64]) -> (Matrix, Matrix) {
    let (r, c) = (y.len(), x.len());
    let (mut mx, mut my) = (Matrix::zeros(r, c), Matrix::zeros(r, c));
    for i in 0..r {
        for j in 0..c {
            mx.set(i, j, x[j]);
            my.set(i, j, y[i]);
        }
    }
    (mx, my)
}

pub(super) fn range(v: &[f64]) -> (f64, f64) {
    v.iter().filter(|v| v.is_finite()).fold((f64::INFINITY, f64::NEG_INFINITY), |r, &v| (r.0.min(v), r.1.max(v)))
}

fn scalar(m: &Matrix) -> Result<f64, String> {
    if m.is_scalar() { Ok(m.data[0]) } else { Err(format!("expected a scalar, got {}x{}", m.rows, m.cols)) }
}

pub(super) fn first(r: R) -> Result<Value, String> {
    r?.into_iter().next().ok_or_else(|| "function returned nothing".into())
}

/// Columns of length n (a length-n vector is one column).
fn columns(m: &Matrix, n: usize) -> Option<Vec<Vec<f64>>> {
    if n == 0 {
        Some(vec![])
    } else if m.is_vector() && m.len() == n {
        Some(vec![m.data.clone()])
    } else if m.rows == n {
        Some(m.data.chunks(n).map(<[f64]>::to_vec).collect())
    } else if m.cols == n {
        Some(m.transpose().data.chunks(n).map(<[f64]>::to_vec).collect())
    } else {
        None
    }
}

/// Octave x/y pairing: vector x with each column of Y, matrices column by column.
fn xy_pairs(x: Option<&Matrix>, y: &Matrix) -> Result<Vec<(Vec<f64>, Vec<f64>)>, String> {
    let n = |m: &Matrix| if m.is_vector() { m.len() } else { m.rows };
    let Some(x) = x else {
        let ys = columns(y, n(y)).unwrap_or_default();
        return Ok(ys.into_iter().map(|c| ((1..=c.len()).map(|i| i as f64).collect(), c)).collect());
    };
    let (Some(xs), Some(ys)) = (columns(x, n(x)), columns(y, n(x))) else {
        return Err(format!("x ({}x{}) and y ({}x{}) don't match", x.rows, x.cols, y.rows, y.cols));
    };
    Ok(match (xs.len(), ys.len()) {
        (1, _) => ys.into_iter().map(|c| (xs[0].clone(), c)).collect(),
        (_, 1) => xs.into_iter().map(|c| (c, ys[0].clone())).collect(),
        (a, b) if a == b => xs.into_iter().zip(ys).collect(),
        _ => return Err("x and y need the same number of columns".into()),
    })
}

/// Parsed format string like "r--o" or "k;label;".
#[derive(Clone, Default)]
struct Fmt {
    color: Option<Rgb>,
    line: Option<LineStyle>,
    marker: Option<Marker>,
    label: Option<String>,
}

impl Fmt {
    fn apply(&self, s: &mut Series) {
        if let Some(c) = self.color {
            s.color = Some(c);
        }
        if let Some(m) = self.marker {
            s.marker = m;
            s.line = LineStyle::None; // marker alone = no line
        }
        if let Some(l) = self.line {
            s.line = l;
        }
        if let Some(l) = &self.label {
            s.label = l.clone();
        }
    }
}

/// None if `s` isn't a format string (then it's a property name).
fn parse_fmt(s: &str) -> Option<Fmt> {
    let mut f = Fmt::default();
    let mut spec = s;
    if let Some((a, rest)) = s.split_once(';') {
        f.label = Some(rest.trim_end_matches(';').to_string());
        spec = a;
    }
    let c: Vec<char> = spec.chars().collect();
    let mut i = 0;
    while i < c.len() {
        match (c[i], c.get(i + 1)) {
            ('-', Some('-')) => (f.line, i) = (Some(LineStyle::Dashed), i + 1),
            ('-', Some('.')) => (f.line, i) = (Some(LineStyle::DashDot), i + 1),
            ('-', _) => f.line = Some(LineStyle::Solid),
            (':', _) => f.line = Some(LineStyle::Dotted),
            (ch, _) => match color_char(ch) {
                Some(col) => f.color = Some(col),
                None => f.marker = Some(marker_char(ch)?),
            },
        }
        i += 1;
    }
    Some(f)
}

fn color_char(c: char) -> Option<Rgb> {
    Some(match c {
        'r' => [220, 40, 40],
        'g' => [30, 160, 50],
        'b' => [30, 90, 220],
        'c' => [0, 180, 200],
        'm' => [200, 40, 200],
        'y' => [220, 190, 0],
        'k' => [0, 0, 0],
        'w' => [255, 255, 255],
        _ => return None,
    })
}

fn marker_char(c: char) -> Option<Marker> {
    Some(match c {
        '.' => Marker::Point,
        'o' => Marker::Circle,
        'x' => Marker::Cross,
        '+' => Marker::Plus,
        '*' | 'p' | 'h' => Marker::Star,
        's' => Marker::Square,
        'd' => Marker::Diamond,
        '^' | 'v' | '<' | '>' => Marker::Triangle, // ponytail: one triangle orientation
        _ => return None,
    })
}

/// 'r', 'red' or [r g b] in 0..1.
fn parse_color(v: &Value) -> Result<Rgb, String> {
    if let Value::Str(s) = v {
        let s = s.to_ascii_lowercase();
        let c = if s == "black" { 'k' } else { s.chars().next().unwrap_or(' ') };
        return color_char(c).ok_or_else(|| format!("unknown color '{s}'"));
    }
    match v.matrix()?.data[..] {
        [r, g, b] if [r, g, b].iter().all(|c| (0.0..=1.0).contains(c)) => Ok([r, g, b].map(|c| (c * 255.0).round() as u8)),
        _ => Err("color must be a name or [r g b] with values in 0..1".into()),
    }
}

/// Trailing 'Name', value pairs.
fn props(name: &str, series: &mut [Series], args: &[Value]) -> Result<(), String> {
    for pair in args.chunks(2) {
        let key = pair[0].as_str().map_err(|_| format!("{name}: expected a property name"))?.to_ascii_lowercase();
        let v = pair.get(1).ok_or_else(|| format!("{name}: property '{key}' needs a value"))?;
        for s in series.iter_mut() {
            match key.as_str() {
                "linewidth" => s.width = v.scalar()?,
                "displayname" => s.label = v.as_str()?.to_string(),
                "color" => s.color = Some(parse_color(v)?),
                "marker" => {
                    s.marker = match v.as_str()? {
                        "none" => Marker::None,
                        "square" => Marker::Square,
                        "diamond" => Marker::Diamond,
                        m => parse_fmt(m).and_then(|f| f.marker).ok_or_else(|| format!("{name}: unknown marker '{m}'"))?,
                    }
                }
                "linestyle" => {
                    s.line = match v.as_str()? {
                        "none" => LineStyle::None,
                        l => parse_fmt(l).and_then(|f| f.line).ok_or_else(|| format!("{name}: unknown line style '{l}'"))?,
                    }
                }
                // ponytail: not modeled, accepted so scripts keep running
                // fill/area color; surfaces' 'interp' and friends are skipped
                "facecolor" => s.color = parse_color(v).ok().or(s.color),
                "markersize" | "markerfacecolor" | "markeredgecolor" | "edgecolor" | "facealpha" | "edgealpha" | "linespec" => {}
                _ => return Err(format!("{name}: unknown property '{key}'")),
            }
        }
    }
    Ok(())
}

/// Default series color the renderer would pick at index i (light palette); for multi-part
/// series (stem, errorbar) whose parts must share one color.
pub(super) fn palette(i: usize) -> Rgb {
    let p = Theme::light().palette;
    p[i % p.len()]
}

/// Put new series on the current axes; without hold, the old plot and its limits/scales go.
pub(super) fn add(it: &mut Interp, series: Vec<Series>) -> &mut Axes {
    let a = it.plots.axes();
    if !a.hold {
        (a.series, a.xlim, a.ylim, a.zlim) = (vec![], None, None, None);
        (a.logx, a.logy, a.force3d, a.view) = (false, false, false, View::default());
        a.pins.clear();
        a.notes.clear();
    }
    a.series.extend(series);
    a
}

/// 3D plots turn the grid on, like Octave.
fn add3(it: &mut Interp, series: Vec<Series>) -> R {
    let a = add(it, series);
    if !a.hold {
        a.grid = true;
    }
    touch(it)
}

/// Limits = exact data extent (bounds() without its 2D padding).
fn tight(a: &mut Axes) {
    if a.series.is_empty() {
        return;
    }
    (a.xlim, a.ylim, a.zlim) = (None, None, None);
    let pad = if a.is_3d() { 0.0 } else { 0.05 };
    let b = a.bounds();
    let un = |(lo, hi): (f64, f64)| {
        let r = (hi - lo) / (1.0 + 2.0 * pad);
        (lo + pad * r, hi - pad * r)
    };
    (a.xlim, a.ylim) = (Some(un(b[0])), Some(un(b[1])));
    if a.is_3d() {
        a.zlim = Some(un(b[2]));
    }
}

// ---- 2D lines ----

/// plot-style args: (y | x, y)[, fmt] repeated, then 'Name', value pairs. A lone complex y
/// plots real against imaginary part; complex x/y pairs lose their imaginary parts (with a warning).
fn line_series(it: &mut Interp, name: &str, args: &[Value]) -> Result<Vec<Series>, String> {
    if args.is_empty() {
        return Err(format!("{name}: not enough arguments"));
    }
    let mut out = vec![];
    let mut i = 0;
    let mut warned = false;
    let mut real = |it: &mut Interp, v: &Value| match v {
        Value::Complex(m) => {
            if !std::mem::replace(&mut warned, true) {
                warn_imag(it, name);
            }
            Ok(m.re())
        }
        v => v.matrix(),
    };
    while i < args.len() && is_num(&args[i]) {
        let pairs = match (&args[i], args.get(i + 1).filter(|v| is_num(v))) {
            (a, Some(b)) => {
                i += 2;
                xy_pairs(Some(&real(it, a)?), &real(it, b)?)
            }
            (Value::Complex(z), None) => {
                i += 1;
                xy_pairs(Some(&z.re()), &z.im())
            }
            (a, None) => {
                i += 1;
                xy_pairs(None, &a.matrix()?)
            }
        }
        .map_err(|e| format!("{name}: {e}"))?;
        let mut fmt = None;
        if let Some(Value::Str(s)) = args.get(i) {
            fmt = parse_fmt(s);
            i += fmt.is_some() as usize;
        }
        for (x, y) in pairs {
            let mut s = Series::new(Data::Line { x, y });
            if let Some(f) = &fmt {
                f.apply(&mut s);
            }
            out.push(s);
        }
    }
    props(name, &mut out, &args[i..])?;
    Ok(out)
}

fn lines(it: &mut Interp, name: &str, args: &[Value], logx: bool, logy: bool) -> R {
    let s = line_series(it, name, args)?;
    let a = add(it, s);
    a.logx |= logx;
    a.logy |= logy;
    touch(it)
}

fn plot(it: &mut Interp, args: &[Value], _: usize) -> R {
    lines(it, "plot", args, false, false)
}
fn semilogx(it: &mut Interp, args: &[Value], _: usize) -> R {
    lines(it, "semilogx", args, true, false)
}
fn semilogy(it: &mut Interp, args: &[Value], _: usize) -> R {
    lines(it, "semilogy", args, false, true)
}
fn loglog(it: &mut Interp, args: &[Value], _: usize) -> R {
    lines(it, "loglog", args, true, true)
}

fn plot3(it: &mut Interp, args: &[Value], _: usize) -> R {
    let args = &real_args(it, "plot3", args)[..];
    let mut out = vec![];
    let mut i = 0;
    while i < args.len() && is_num(&args[i]) {
        let g: Vec<Vec<f64>> = args[i..].iter().take(3).take_while(|v| is_num(v)).map(|v| v.matrix().map(|m| m.data)).collect::<Result<_, _>>()?;
        // ponytail: matrices are flattened, not split into columns
        let [x, y, z] = &g[..] else { return Err("plot3: expected x, y, z".into()) };
        if x.len() != y.len() || x.len() != z.len() {
            return Err("plot3: x, y and z must have the same length".into());
        }
        let mut s = Series::new(Data::Line3 { x: x.clone(), y: y.clone(), z: z.clone() });
        i += 3;
        if let Some(f) = args.get(i).and_then(|v| v.as_str().ok()).and_then(parse_fmt) {
            f.apply(&mut s);
            i += 1;
        }
        out.push(s);
    }
    if out.is_empty() {
        return Err("plot3: expected x, y, z".into());
    }
    props("plot3", &mut out, &args[i..])?;
    add3(it, out)
}

/// scatter(x, y[, s[, c]][, 'filled'][, marker]) and scatter3 with z.
fn scatter_series(it: &mut Interp, name: &str, args: &[Value], dims: usize) -> Result<Vec<Series>, String> {
    let args = &real_args(it, name, args)[..];
    let (nums, rest) = leading(args)?;
    if nums.len() < dims {
        return Err(format!("{name}: not enough arguments"));
    }
    let n = nums[0].len();
    if nums[..dims].iter().any(|m| m.len() != n) {
        return Err(format!("{name}: coordinates must have the same length"));
    }
    // ponytail: per-point sizes are averaged into one marker size
    let size = nums.get(dims).filter(|m| !m.is_empty()).map_or(36.0, |m| m.data.iter().sum::<f64>() / m.len() as f64);
    let (mut marker, mut color) = (Marker::Point, None);
    for v in rest {
        let s = v.as_str()?;
        if s != "filled" {
            let f = parse_fmt(s).ok_or_else(|| format!("{name}: unknown option '{s}'"))?;
            marker = f.marker.unwrap_or(marker);
            color = f.color.or(color);
        }
    }
    let mut groups: Vec<(Vec<usize>, Option<Rgb>)> = vec![((0..n).collect(), color)];
    match nums.get(dims + 1) {
        Some(c) if c.len() == 3 && (n != 3 || c.rows == 1) => groups[0].1 = Some(parse_color(&Value::Num(c.clone()))?),
        Some(c) if c.len() == n => {
            // per-point colors: binned through the colormap, one series per bin
            // ponytail: 16 color bins, and a later colormap() doesn't recolor them
            let cm = it.plots.fig().colormap.clone();
            let (lo, hi) = range(&c.data);
            let mut bins = vec![vec![]; 16];
            for (k, &v) in c.data.iter().enumerate() {
                let t = if hi > lo { (v - lo) / (hi - lo) } else { 0.5 };
                bins[((t * 16.0) as usize).min(15)].push(k);
            }
            groups = bins.into_iter().enumerate().filter(|(_, b)| !b.is_empty()).map(|(i, b)| (b, Some(render::cmap(&cm, (i as f64 + 0.5) / 16.0)))).collect();
        }
        Some(_) => return Err(format!("{name}: color must be [r g b] or one value per point")),
        None => {}
    }
    let pick = |k: usize, idx: &[usize]| idx.iter().map(|&i| nums[k].data[i]).collect::<Vec<f64>>();
    Ok(groups
        .iter()
        .map(|(idx, c)| {
            let data = if dims == 3 { Data::Line3 { x: pick(0, idx), y: pick(1, idx), z: pick(2, idx) } } else { Data::Line { x: pick(0, idx), y: pick(1, idx) } };
            Series { line: LineStyle::None, marker, color: *c, width: size.max(1.0).sqrt() / 3.0, ..Series::new(data) }
        })
        .collect())
}

fn scatter(it: &mut Interp, args: &[Value], _: usize) -> R {
    let s = scatter_series(it, "scatter", args, 2)?;
    add(it, s);
    touch(it)
}
fn scatter3(it: &mut Interp, args: &[Value], _: usize) -> R {
    let s = scatter_series(it, "scatter3", args, 3)?;
    add3(it, s)
}

/// bar(y | x, y[, width][, color]); matrix columns become grouped bars.
fn bar(it: &mut Interp, args: &[Value], _: usize) -> R {
    let args = &real_args(it, "bar", args)[..];
    let (nums, rest) = leading(args)?;
    let (x, y, w) = match &nums[..] {
        [y] => (None, y, 0.8),
        [y, w] if w.is_scalar() && !y.is_scalar() => (None, y, w.data[0]),
        [x, y] => (Some(x), y, 0.8),
        [x, y, w] => (Some(x), y, scalar(w)?),
        _ => return Err("bar: expected y, (x, y) or (x, y, width)".into()),
    };
    let pairs = xy_pairs(x, y).map_err(|e| format!("bar: {e}"))?;
    let fmt = rest.first().and_then(|v| v.as_str().ok()).and_then(parse_fmt);
    let m = pairs.len().max(1) as f64;
    let mut out = vec![];
    for (j, (x, y)) in pairs.into_iter().enumerate() {
        let mut xs = x.clone();
        xs.sort_by(f64::total_cmp);
        let dx = xs.windows(2).map(|w| w[1] - w[0]).filter(|d| *d > 0.0).fold(f64::INFINITY, f64::min);
        let dx = if dx.is_finite() { dx } else { 1.0 };
        // ponytail: grouped side by side only, no 'stacked'
        let bw = w * dx / m;
        let off = (j as f64 - (m - 1.0) / 2.0) * bw;
        let mut s = Series::new(Data::Bar { x: x.iter().map(|v| v + off).collect(), y, width: bw });
        if let Some(f) = &fmt {
            s.color = f.color;
        }
        out.push(s);
    }
    add(it, out);
    touch(it)
}

fn stairs(it: &mut Interp, args: &[Value], _: usize) -> R {
    let mut s = line_series(it, "stairs", args)?;
    for s in &mut s {
        if let Data::Line { x, y } = &mut s.data {
            let (mut sx, mut sy) = (vec![], vec![]);
            for i in 0..x.len() {
                sx.push(x[i]);
                sy.push(y[i]);
                if i + 1 < x.len() {
                    sx.push(x[i + 1]);
                    sy.push(y[i]);
                }
            }
            (*x, *y) = (sx, sy);
        }
    }
    add(it, s);
    touch(it)
}

/// area(y | x, y)[, props]: filled down to 0.
// ponytail: matrix columns overlap instead of stacking
fn area(it: &mut Interp, args: &[Value], _: usize) -> R {
    let args = &real_args(it, "area", args)[..];
    let (nums, rest) = leading(args)?;
    let pairs = match &nums[..] {
        [y] => xy_pairs(None, y),
        [x, y] => xy_pairs(Some(x), y),
        _ => return Err("area: expected y or (x, y)".into()),
    }
    .map_err(|e| format!("area: {e}"))?;
    let mut out = vec![];
    for (mut x, mut y) in pairs.into_iter().filter(|(x, _)| !x.is_empty()) {
        let (x0, x1) = (x[0], x[x.len() - 1]);
        x.extend([x1, x0]);
        y.extend([0.0, 0.0]);
        out.push(Series::new(Data::Fill { x, y }));
    }
    props("area", &mut out, rest)?;
    add(it, out);
    touch(it)
}

/// fill(x, y[, color][, props]): a polygon per column.
// ponytail: one x/y/color group; colormapped (scalar or per-vertex) colors use the palette
fn fill(it: &mut Interp, args: &[Value], _: usize) -> R {
    let args = &real_args(it, "fill", args)[..];
    let (nums, mut rest) = leading(args)?;
    let [x, y, c @ ..] = &nums[..] else { return Err("fill: expected x, y[, color]".into()) };
    let pairs = xy_pairs(Some(x), y).map_err(|e| format!("fill: {e}"))?;
    let mut color = c.first().and_then(|m| parse_color(&Value::Num(m.clone())).ok());
    if rest.len() % 2 == 1 {
        color = Some(parse_color(&rest[0]).map_err(|e| format!("fill: {e}"))?);
        rest = &rest[1..];
    }
    let mut out: Vec<Series> = pairs.into_iter().map(|(x, y)| Series { color, ..Series::new(Data::Fill { x, y }) }).collect();
    props("fill", &mut out, rest)?;
    add(it, out);
    touch(it)
}

/// Stems: a marker series plus a NaN-broken line series in the same color.
fn stem(it: &mut Interp, args: &[Value], _: usize) -> R {
    // ponytail: 'filled' accepted, markers stay hollow
    let args: Vec<Value> = args.iter().filter(|v| !matches!(v, Value::Str(s) if s == "filled")).cloned().collect();
    let base = line_series(it, "stem", &args)?;
    let n0 = Some(it.plots.axes()).filter(|a| a.hold).map_or(0, |a| a.series.len());
    let mut out = vec![];
    for (k, s) in base.into_iter().enumerate() {
        let Data::Line { x, y } = &s.data else { continue };
        let color = Some(s.color.unwrap_or(palette(n0 + k)));
        let (mut sx, mut sy) = (vec![], vec![]);
        for (&x, &y) in x.iter().zip(y) {
            sx.extend([x, x, f64::NAN]);
            sy.extend([0.0, y, f64::NAN]);
        }
        let marker = if s.marker == Marker::None { Marker::Circle } else { s.marker };
        let line = if s.line == LineStyle::None { LineStyle::Solid } else { s.line };
        let stems = Series { line, marker: Marker::None, color, label: String::new(), ..Series::new(Data::Line { x: sx, y: sy }) };
        out.push(Series { line: LineStyle::None, marker, color, ..s });
        out.push(Series { width: stems.width, ..stems });
    }
    add(it, out);
    touch(it)
}

/// Bin centers and counts; `bins` is a count (default 10) or a vector of centers.
fn histogram(y: &[f64], bins: Option<&Matrix>) -> Result<(Vec<f64>, Vec<f64>), String> {
    let (lo, hi) = range(y);
    let (lo, hi) = if lo > hi { (0.0, 1.0) } else if lo == hi { (lo - 0.5, hi + 0.5) } else { (lo, hi) };
    let centers = match bins {
        Some(m) if m.len() > 1 => m.data.clone(),
        b => {
            let n = b.map_or(Ok(10.0), |m| scalar(m))?;
            if !(n >= 1.0) {
                return Err("hist: number of bins must be positive".into());
            }
            let w = (hi - lo) / n.floor();
            (0..n as usize).map(|i| lo + (i as f64 + 0.5) * w).collect()
        }
    };
    let edges: Vec<f64> = centers.windows(2).map(|w| (w[0] + w[1]) / 2.0).collect();
    let mut counts = vec![0.0; centers.len()];
    for &v in y.iter().filter(|v| v.is_finite()) {
        counts[edges.partition_point(|&e| e < v)] += 1.0;
    }
    Ok((centers, counts))
}

/// hist(y[, n | centers[, norm]]): plots bars, or returns [counts, centers] when asked.
fn hist(it: &mut Interp, args: &[Value], nargout: usize) -> R {
    let args = &real_args(it, "hist", args)[..];
    let (nums, _) = leading(args)?;
    let y = nums.first().ok_or("hist: needs data")?;
    let (centers, mut counts) = histogram(&y.data, nums.get(1))?;
    if let Some(norm) = nums.get(2) {
        let (s, k) = (counts.iter().sum::<f64>(), scalar(norm)?);
        counts.iter_mut().for_each(|c| *c *= k / s.max(1.0));
    }
    if nargout > 0 {
        return Ok(vec![Value::from(counts), Value::from(centers)]);
    }
    let width = centers.windows(2).map(|w| w[1] - w[0]).fold(f64::INFINITY, f64::min);
    let width = if width.is_finite() { width } else { 1.0 };
    add(it, vec![Series::new(Data::Bar { x: centers, y: counts, width })]);
    touch(it)
}

/// polar(theta, rho[, fmt]) as x/y lines.
// ponytail: no circular grid, just equal-ish limits
fn polar(it: &mut Interp, args: &[Value], _: usize) -> R {
    let args = &real_args(it, "polar", args)[..];
    let (nums, rest) = leading(args)?;
    let [th, r] = &nums[..] else { return Err("polar: expected theta, rho".into()) };
    if th.len() != r.len() {
        return Err("polar: theta and rho must have the same length".into());
    }
    let x = Matrix::new(r.rows, r.cols, th.data.iter().zip(&r.data).map(|(t, r)| r * t.cos()).collect());
    let y = Matrix::new(r.rows, r.cols, th.data.iter().zip(&r.data).map(|(t, r)| r * t.sin()).collect());
    let s = line_series(it, "polar", &[[Value::Num(x), Value::Num(y)].as_slice(), rest].concat())?;
    let a = add(it, s);
    if !a.hold {
        round_limits(a, &r.data);
    }
    touch(it)
}

/// Equal-ish limits around the origin for radii `r` (polar, compass).
pub(super) fn round_limits(a: &mut Axes, r: &[f64]) {
    let m = r.iter().filter(|v| v.is_finite()).fold(0.0f64, |m, v| m.max(v.abs())).max(1e-12) * 1.1;
    (a.xlim, a.ylim) = (Some((-m * 1.35, m * 1.35)), Some((-m, m)));
}

/// errorbar(y, e | x, y, e | x, y, lo, hi)[, fmt]
fn errorbar(it: &mut Interp, args: &[Value], _: usize) -> R {
    let args = &real_args(it, "errorbar", args)[..];
    let (nums, rest) = leading(args)?;
    let v = |k: usize| nums[k].data.clone();
    let (x, y, lo, hi) = match nums.len() {
        2 => ((1..=nums[0].len()).map(|i| i as f64).collect(), v(0), v(1), v(1)),
        3 => (v(0), v(1), v(2), v(2)),
        4 => (v(0), v(1), v(2), v(3)),
        _ => return Err("errorbar: expected (y, e), (x, y, e) or (x, y, lo, hi)".into()),
    };
    let n = x.len();
    if [&y, &lo, &hi].iter().any(|v| v.len() != n) {
        return Err("errorbar: arguments must have the same length".into());
    }
    let fmt = rest.first().and_then(|v| v.as_str().ok()).and_then(parse_fmt).unwrap_or_default();
    let n0 = Some(it.plots.axes()).filter(|a| a.hold).map_or(0, |a| a.series.len());
    let color = Some(fmt.color.unwrap_or(palette(n0)));
    let (x0, x1) = range(&x);
    let cap = if n > 1 { (x1 - x0) / (n - 1) as f64 * 0.15 } else { 0.1 };
    let (mut bx, mut by) = (vec![], vec![]);
    for i in 0..n {
        let (b, t) = (y[i] - lo[i], y[i] + hi[i]);
        bx.extend([x[i], x[i], f64::NAN, x[i] - cap, x[i] + cap, f64::NAN, x[i] - cap, x[i] + cap, f64::NAN]);
        by.extend([b, t, f64::NAN, b, b, f64::NAN, t, t, f64::NAN]);
    }
    let mut data = Series { color, ..Series::new(Data::Line { x, y }) };
    fmt.apply(&mut data);
    let bars = Series { color, width: 1.0, ..Series::new(Data::Line { x: bx, y: by }) };
    add(it, vec![data, bars]);
    touch(it)
}

// ---- surfaces, contours, images ----

/// Z | X, Y, Z (vectors are expanded meshgrid-style).
fn grid_args(name: &str, nums: &[Matrix]) -> Result<(Matrix, Matrix, Matrix), String> {
    let (x, y, z) = match nums {
        [z] => {
            let (x, y) = meshgrid(&(1..=z.cols).map(|i| i as f64).collect::<Vec<_>>(), &(1..=z.rows).map(|i| i as f64).collect::<Vec<_>>());
            (x, y, z.clone())
        }
        [x, y, z, ..] if nums.len() <= 4 => {
            if x.is_vector() && y.is_vector() && x.len() == z.cols && y.len() == z.rows {
                let (x, y) = meshgrid(&x.data, &y.data);
                (x, y, z.clone())
            } else {
                (x.clone(), y.clone(), z.clone())
            }
        }
        _ => return Err(format!("{name}: expected Z or X, Y, Z")),
    };
    if (x.rows, x.cols) != (z.rows, z.cols) || (y.rows, y.cols) != (z.rows, z.cols) {
        return Err(format!("{name}: X, Y and Z sizes don't match"));
    }
    Ok((x, y, z))
}

fn surface(it: &mut Interp, name: &str, args: &[Value], wire: bool) -> R {
    let args = &real_args(it, name, args)[..];
    let (nums, rest) = leading(args)?;
    let (x, y, z) = grid_args(name, &nums)?;
    let mut s = vec![Series::new(Data::Surface { x, y, z, wire })];
    props(name, &mut s, rest)?;
    add3(it, s)
}
fn surf(it: &mut Interp, args: &[Value], _: usize) -> R {
    surface(it, "surf", args, false)
}
fn mesh(it: &mut Interp, args: &[Value], _: usize) -> R {
    surface(it, "mesh", args, true)
}

/// (Z | X, Y, Z)[, n | levels][, fmt]
fn contour_args(name: &str, args: &[Value]) -> Result<(Matrix, Matrix, Matrix, Vec<f64>, Fmt), String> {
    let (nums, rest) = leading(args)?;
    let (g, lv) = match nums.len() {
        2 | 4 => (&nums[..nums.len() - 1], nums.last()),
        _ => (&nums[..], None),
    };
    let (x, y, z) = grid_args(name, g)?;
    if z.rows < 2 || z.cols < 2 {
        return Err(format!("{name}: Z must be at least 2x2"));
    }
    let levels = match lv {
        None => vec![],
        Some(m) if m.is_scalar() => {
            let (lo, hi) = range(&z.data);
            let n = m.data[0].max(1.0) as usize;
            linspace(lo, hi, n + 2)[1..=n].to_vec()
        }
        Some(m) if m.len() == 2 && m.data[0] == m.data[1] => vec![m.data[0]],
        Some(m) => m.data.clone(),
    };
    let fmt = rest.first().and_then(|v| v.as_str().ok()).and_then(parse_fmt).unwrap_or_default();
    Ok((x, y, z, levels, fmt))
}

fn contour(it: &mut Interp, args: &[Value], _: usize) -> R {
    let (x, y, z, levels, fmt) = contour_args("contour", &real_args(it, "contour", args))?;
    let mut s = Series::new(Data::Contour { x, y, z, levels });
    fmt.apply(&mut s);
    let a = add(it, vec![s]);
    if !a.hold {
        tight(a);
    }
    touch(it)
}

/// Filled contours: a band-quantized image resampled on a fine grid, plus dark contour lines.
// ponytail: assumes a roughly uniform X/Y grid (true for meshgrid input)
fn contourf(it: &mut Interp, args: &[Value], _: usize) -> R {
    let (x, y, z, mut levels, _) = contour_args("contourf", &real_args(it, "contourf", args))?;
    if levels.is_empty() {
        levels = render::auto_levels(&z);
    }
    let (zlo, zhi) = range(&z.data);
    let band = |v: f64| {
        let k = levels.partition_point(|&l| l <= v);
        let lo = if k == 0 { zlo } else { levels[k - 1] };
        let hi = if k == levels.len() { zhi } else { levels[k] };
        (lo + hi) / 2.0
    };
    let (nr, nc) = (z.rows.max(200), z.cols.max(200));
    let mut img = Matrix::zeros(nr, nc);
    for i in 0..nr {
        for j in 0..nc {
            let fi = i as f64 / (nr - 1) as f64 * (z.rows - 1) as f64;
            let fj = j as f64 / (nc - 1) as f64 * (z.cols - 1) as f64;
            img.set(i, j, band(bilinear(&z, fi, fj)));
        }
    }
    let ((x0, x1), (y0, y1)) = (range(&x.data), range(&y.data));
    let (dx, dy) = ((x1 - x0) / (nc - 1) as f64 / 2.0, (y1 - y0) / (nr - 1) as f64 / 2.0);
    let image = Series::new(Data::Image { z: img, x: (x0 - dx, x1 + dx), y: (y0 - dy, y1 + dy) });
    let lines = Series { color: Some([25, 25, 25]), width: 0.75, ..Series::new(Data::Contour { x, y, z, levels }) };
    let a = add(it, vec![image, lines]);
    if !a.hold {
        (a.xlim, a.ylim) = (Some((x0, x1)), Some((y0, y1)));
    }
    touch(it)
}

fn bilinear(z: &Matrix, fi: f64, fj: f64) -> f64 {
    let (i, j) = ((fi.floor() as usize).min(z.rows - 2), (fj.floor() as usize).min(z.cols - 2));
    let (t, u) = (fi - i as f64, fj - j as f64);
    let top = z.get(i, j) * (1.0 - u) + z.get(i, j + 1) * u;
    let bot = z.get(i + 1, j) * (1.0 - u) + z.get(i + 1, j + 1) * u;
    top * (1.0 - t) + bot * t
}

/// imagesc(C | x, y, C): row 1 at the top (y runs down, Octave's `axis ij`), axes tight around the pixels.
fn imagesc(it: &mut Interp, args: &[Value], _: usize) -> R {
    let (nums, _) = leading(&real_args(it, "imagesc", args))?;
    let (z, x, y) = match &nums[..] {
        [z] => (z.clone(), None, None),
        [x, y, z] => (z.clone(), Some(x), Some(y)),
        _ => return Err("imagesc: expected C or x, y, C".into()),
    };
    if z.is_empty() {
        return Err("imagesc: empty image".into());
    }
    // x/y give the centers of the first and last pixel
    let ext = |v: Option<&Matrix>, n: usize| {
        let (a, b) = v.filter(|m| !m.is_empty()).map_or((1.0, n as f64), |m| (m.data[0], m.data[m.len() - 1]));
        let d = if n > 1 && a != b { (b - a).abs() / (n - 1) as f64 } else { 1.0 };
        (a.min(b) - d / 2.0, a.max(b) + d / 2.0)
    };
    let (xe, ye) = (ext(x, z.cols), ext(y, z.rows));
    let a = add(it, vec![Series::new(Data::Image { z, x: xe, y: ye })]);
    if !a.hold {
        (a.xlim, a.ylim) = (Some(xe), Some((ye.1, ye.0)));
    }
    touch(it)
}

/// pcolor(Z | X, Y, Z): like imagesc but Z row 1 at the bottom.
// ponytail: cells are centered on the grid points instead of spanning between them
fn pcolor(it: &mut Interp, args: &[Value], _: usize) -> R {
    let (nums, _) = leading(&real_args(it, "pcolor", args))?;
    let (x, y, z) = grid_args("pcolor", &nums)?;
    let pad = |(lo, hi): (f64, f64)| if lo < hi { (lo, hi) } else { (lo - 0.5, hi + 0.5) };
    let (xe, ye) = (pad(range(&x.data)), pad(range(&y.data)));
    let a = add(it, vec![Series::new(Data::Image { z, x: xe, y: ye })]);
    if !a.hold {
        (a.xlim, a.ylim) = (Some(xe), Some(ye));
    }
    touch(it)
}

/// quiver(u, v | x, y, u, v)[, scale][, fmt]; arrows auto-scaled to the grid like Octave.
fn quiver(it: &mut Interp, args: &[Value], _: usize) -> R {
    let args = &real_args(it, "quiver", args)[..];
    let (nums, rest) = leading(args)?;
    let (x, y, u, v, scale) = match &nums[..] {
        [u, v] | [u, v, _] => {
            let (x, y) = meshgrid(&(1..=u.cols).map(|i| i as f64).collect::<Vec<_>>(), &(1..=u.rows).map(|i| i as f64).collect::<Vec<_>>());
            (x, y, u.clone(), v.clone(), nums.get(2))
        }
        [x, y, u, v] | [x, y, u, v, _] => {
            let (x, y) = if x.is_vector() && y.is_vector() && x.len() == u.cols && y.len() == u.rows && !u.is_vector() { meshgrid(&x.data, &y.data) } else { (x.clone(), y.clone()) };
            (x, y, u.clone(), v.clone(), nums.get(4))
        }
        _ => return Err("quiver: expected (u, v) or (x, y, u, v)".into()),
    };
    let n = u.len();
    if x.len() != n || y.len() != n || v.len() != n {
        return Err("quiver: x, y, u and v must have the same number of elements".into());
    }
    let s = scale.map_or(Ok(1.0), |m| scalar(m))?;
    let mut k = 1.0;
    if s != 0.0 && n > 1 {
        let ((x0, x1), (y0, y1)) = (range(&x.data), range(&y.data));
        let (dx, dy) = ((x1 - x0) / x.cols as f64, (y1 - y0) / y.rows as f64);
        let len = u.data.iter().zip(&v.data).map(|(a, b)| a.hypot(*b)).filter(|l| l.is_finite()).fold(0.0, f64::max);
        if len > 0.0 && dx.hypot(dy) > 0.0 {
            k = dx.hypot(dy) / len * 0.9 * s;
        }
    }
    let mut q = Series::new(Data::Quiver { x: x.data, y: y.data, u: u.data.iter().map(|a| a * k).collect(), v: v.data.iter().map(|a| a * k).collect() });
    if let Some(f) = rest.first().and_then(|v| v.as_str().ok()).and_then(parse_fmt) {
        q.color = f.color;
    }
    add(it, vec![q]);
    touch(it)
}

/// compass(z | u, v)[, fmt]: arrows from the origin (phasor diagram).
fn compass(it: &mut Interp, args: &[Value], _: usize) -> R {
    arrows(it, "compass", args, false)
}

/// feather(z | u, v)[, fmt]: arrows from (k, 0) along a baseline.
fn feather(it: &mut Interp, args: &[Value], _: usize) -> R {
    arrows(it, "feather", args, true)
}

fn arrows(it: &mut Interp, name: &str, args: &[Value], feather: bool) -> R {
    let n = args.iter().take_while(|v| is_num(v)).count();
    let (u, v) = match &args[..n] {
        [z] => {
            let z = z.cmatrix()?;
            (z.re().data, z.im().data)
        }
        [u, v] => (u.matrix()?.data, v.matrix()?.data),
        _ => return Err(format!("{name}: expected z or (u, v)")),
    };
    if u.len() != v.len() {
        return Err(format!("{name}: u and v must have the same number of elements"));
    }
    let k = u.len();
    let fmt = args.get(n).and_then(|v| v.as_str().ok()).and_then(parse_fmt).unwrap_or_default();
    let n0 = Some(it.plots.axes()).filter(|a| a.hold).map_or(0, |a| a.series.len());
    let color = Some(fmt.color.unwrap_or(palette(n0)));
    let x = if feather { (1..=k).map(|i| i as f64).collect() } else { vec![0.0; k] };
    let r: Vec<f64> = u.iter().zip(&v).map(|(a, b)| a.hypot(*b)).collect();
    let mut s = vec![Series { color, ..Series::new(Data::Quiver { x, y: vec![0.0; k], u, v }) }];
    if feather {
        s.push(Series { color, width: 0.75, ..Series::new(Data::Line { x: vec![1.0, k.max(1) as f64], y: vec![0.0, 0.0] }) });
    }
    let a = add(it, s);
    if !a.hold && !feather {
        round_limits(a, &r);
    }
    touch(it)
}

fn peaks_z(x: &Matrix, y: &Matrix) -> Matrix {
    let z = x.data.iter().zip(&y.data).map(|(&x, &y)| {
        3.0 * (1.0 - x).powi(2) * (-x * x - (y + 1.0).powi(2)).exp() - 10.0 * (x / 5.0 - x.powi(3) - y.powi(5)) * (-x * x - y * y).exp() - (-(x + 1.0).powi(2) - y * y).exp() / 3.0
    });
    Matrix::new(x.rows, x.cols, z.collect())
}

/// Surface data: plotted with nargout 0, else returned as Z or [X, Y, Z].
fn grid_out(it: &mut Interp, nargout: usize, (x, y, z): (Matrix, Matrix, Matrix)) -> R {
    match nargout {
        0 => add3(it, vec![Series::new(Data::Surface { x, y, z, wire: false })]),
        1 => Ok(vec![Value::Num(z)]),
        _ => Ok(vec![Value::Num(x), Value::Num(y), Value::Num(z)]),
    }
}

fn peaks(it: &mut Interp, args: &[Value], nargout: usize) -> R {
    let (x, y) = match args {
        [] => {
            let v = linspace(-3.0, 3.0, 49);
            meshgrid(&v, &v)
        }
        [n] if n.size() == (1, 1) => {
            let v = linspace(-3.0, 3.0, n.as_usize()?);
            meshgrid(&v, &v)
        }
        [n] => {
            let v = n.matrix()?.data;
            meshgrid(&v, &v)
        }
        [x, y, ..] => {
            let (x, y) = (x.matrix()?, y.matrix()?);
            if x.is_vector() && y.is_vector() { meshgrid(&x.data, &y.data) } else { (x, y) }
        }
    };
    if (x.rows, x.cols) != (y.rows, y.cols) {
        return Err("peaks: X and Y must be the same size".into());
    }
    let z = peaks_z(&x, &y);
    grid_out(it, nargout, (x, y, z))
}

fn sphere(it: &mut Interp, args: &[Value], nargout: usize) -> R {
    let n = args.first().map_or(Ok(20), Value::as_usize)?.max(3);
    let th = linspace(-std::f64::consts::PI, std::f64::consts::PI, n + 1);
    let ph = linspace(-std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2, n + 1);
    let (t, p) = meshgrid(&th, &ph);
    let f = |g: fn(f64, f64) -> f64| Matrix::new(n + 1, n + 1, t.data.iter().zip(&p.data).map(|(&t, &p)| g(t, p)).collect());
    grid_out(it, nargout, (f(|t, p| p.cos() * t.cos()), f(|t, p| p.cos() * t.sin()), f(|_, p| p.sin())))
}

fn cylinder(it: &mut Interp, args: &[Value], nargout: usize) -> R {
    let r = args.first().map_or(Ok(vec![1.0, 1.0]), |v| v.matrix().map(|m| m.data))?;
    let r = if r.len() == 1 { vec![r[0]; 2] } else { r };
    let n = args.get(1).map_or(Ok(20), Value::as_usize)?.max(3);
    let th = linspace(0.0, 2.0 * std::f64::consts::PI, n + 1);
    let m = r.len();
    let mk = |f: &dyn Fn(usize, usize) -> f64| Matrix::new(m, n + 1, (0..m * (n + 1)).map(|k| f(k % m, k / m)).collect());
    let (x, y) = (mk(&|i, j| r[i] * th[j].cos()), mk(&|i, j| r[i] * th[j].sin()));
    let z = mk(&|i, _| i as f64 / (m - 1) as f64);
    grid_out(it, nargout, (x, y, z))
}

// ---- function plots ----

/// "x^2*y" -> "x.^2.*y": element-wise operators so one call covers the whole grid.
pub(super) fn vectorize(s: &str) -> String {
    let mut out = String::new();
    let mut prev = ' ';
    for c in s.chars() {
        if matches!(c, '*' | '/' | '^') && prev != '.' {
            out.push('.');
        }
        out.push(c);
        prev = c;
    }
    out
}

/// Single-letter variables of an expression (e, i, j are constants).
pub(super) fn letters(s: &str) -> Vec<char> {
    let c: Vec<char> = s.chars().collect();
    let id = |ch: &char| ch.is_alphanumeric() || *ch == '_';
    let mut v = vec![];
    for k in 0..c.len() {
        let alone = (k == 0 || !id(&c[k - 1])) && c.get(k + 1).is_none_or(|n| !id(n) && *n != '(');
        if c[k].is_ascii_alphabetic() && alone && !"eij".contains(c[k]) && !v.contains(&c[k]) {
            v.push(c[k]);
        }
    }
    v.sort();
    v
}

/// Function argument -> callable: handles and names as-is, expressions vectorized into a handle.
pub(super) fn func(it: &mut Interp, f: &Value, two: bool) -> Result<Value, String> {
    match f {
        Value::Func(_) => Ok(f.clone()),
        Value::Str(s) if s.chars().all(|c| c.is_alphanumeric() || c == '_') && s.len() > 1 => Ok(f.clone()),
        Value::Str(s) => {
            let l = letters(s);
            let vars = match (two, l.len()) {
                (false, 1) => l[0].to_string(),
                (false, _) => "x".into(),
                (true, 2) if !(l.contains(&'x') ^ l.contains(&'y')) => format!("{},{}", l[0], l[1]),
                (true, _) => "x,y".into(),
            };
            first(it.call(&Value::from("str2func"), &[Value::Str(format!("@({vars}) {}", vectorize(s)))], 1))
        }
        _ => Err("expected a function handle or an expression string".into()),
    }
}

/// f at every point: one vectorized call, else point by point.
pub(super) fn sample(args: &[Matrix], mut f: impl FnMut(&[Value]) -> Result<Value, String>) -> Result<Vec<f64>, String> {
    let n = args[0].len();
    let whole: Vec<Value> = args.iter().map(|m| Value::Num(m.clone())).collect();
    if let Ok(m) = f(&whole).and_then(|v| v.matrix()) {
        if m.len() == n {
            return Ok(m.data);
        }
    }
    (0..n).map(|k| f(&args.iter().map(|m| Value::num(m.data[k])).collect::<Vec<_>>())?.scalar()).collect()
}

/// 5% and 95% quantiles of the finite values.
fn quantiles(v: &[f64]) -> Option<(f64, f64)> {
    let mut f: Vec<f64> = v.iter().copied().filter(|v| v.is_finite()).collect();
    f.sort_by(f64::total_cmp);
    (!f.is_empty()).then(|| (f[f.len() / 20], f[f.len() - 1 - f.len() / 20]))
}

/// Break a sampled curve at jumps (tan, 1/x, floor) so no vertical lines get drawn through
/// poles: a big step that flips sign or dwarfs both neighbouring steps.
pub(super) fn break_jumps(xs: &[f64], ys: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let Some((lo, hi)) = quantiles(ys) else { return (xs.to_vec(), ys.to_vec()) };
    let d: Vec<f64> = ys.windows(2).map(|w| (w[1] - w[0]).abs()).collect();
    let mut sorted: Vec<f64> = d.iter().copied().filter(|v| v.is_finite()).collect();
    sorted.sort_by(f64::total_cmp);
    let lim = (sorted.get(sorted.len() / 2).copied().unwrap_or(0.0) * 20.0).max((hi - lo) * 0.25);
    let (mut x, mut y) = (vec![], vec![]);
    for k in 0..xs.len() {
        if k > 0 && lim > 0.0 && d[k - 1] > lim {
            let near = d.get(k.wrapping_sub(2)).copied().unwrap_or(0.0).max(d.get(k).copied().unwrap_or(0.0));
            if ys[k] * ys[k - 1] < 0.0 || d[k - 1] > 3.0 * near {
                x.push(f64::NAN);
                y.push(f64::NAN);
            }
        }
        x.push(xs[k]);
        y.push(ys[k]);
    }
    (x, y)
}

/// y-limits that ignore poles; Some only when outliers would squash the curve.
pub(super) fn robust_ylim(ys: &[f64]) -> Option<(f64, f64)> {
    let (lo, hi) = quantiles(ys)?;
    let (min, max) = range(ys);
    let r = (hi - lo).max(1e-12);
    (max - min > 5.0 * r).then(|| ((lo - 0.5 * r).max(min), (hi + 0.5 * r).min(max)))
}

/// Math view of a function argument for titles and legends: "x.^2 + 1" -> "x² + 1", @sin -> "sin(x)".
pub(super) fn pretty_fn(f: &Value, vars: &str) -> String {
    let src = match f {
        Value::Str(s) if s.chars().all(|c| c.is_alphanumeric() || c == '_') && s.len() > 1 => format!("{s}({vars})"),
        Value::Str(s) => s.clone(),
        Value::Func(h) => match &**h {
            Func::Anon(l) => l.src.clone(),
            Func::Named(n) => format!("{n}({vars})"),
        },
        _ => return String::new(),
    };
    crate::qalc::pretty(&src)
}

/// Legend text of the last series and, unless hold is on, the title.
fn name_plot(a: &mut Axes, text: String) {
    if !a.hold {
        a.title = text.clone();
    }
    if let Some(s) = a.series.last_mut() {
        s.label = text;
    }
}

/// Callable f sampled for drawing on [lo, hi]: (x, y broken at poles, raw y).
pub(super) fn curve(it: &mut Interp, name: &str, h: &Value, lo: f64, hi: f64) -> Result<(Vec<f64>, Vec<f64>, Vec<f64>), String> {
    let xs = linspace(lo, hi, 1001);
    let ys = sample(&[Matrix::row(xs.clone())], |a| first(it.call(h, a, 1))).map_err(|e| format!("{name}: {e}"))?;
    let (x, y) = break_jumps(&xs, &ys);
    Ok((x, y, ys))
}

/// fplot(f[, [a b]][, fmt])
fn fplot(it: &mut Interp, args: &[Value], _: usize) -> R {
    let f = args.first().ok_or("fplot: needs a function")?;
    let (lo, hi) = match args.get(1).filter(|v| is_num(v)).map(Value::matrix).transpose()? {
        Some(m) if m.len() >= 2 && m.data[0] < m.data[1] => (m.data[0], m.data[1]),
        Some(_) => return Err("fplot: range must be [a b] with a < b".into()),
        None => (-5.0, 5.0),
    };
    let h = func(it, f, false)?;
    let (x, y, ys) = curve(it, "fplot", &h, lo, hi)?;
    let mut s = Series { source: Some(Source::Func(h)), ..Series::new(Data::Line { x, y }) };
    if let Some(fm) = args.iter().skip(1).find_map(|v| v.as_str().ok().and_then(parse_fmt)) {
        fm.apply(&mut s);
    }
    let a = add(it, vec![s]);
    if !a.hold {
        (a.xlim, a.ylim) = (Some((lo, hi)), robust_ylim(&ys));
    }
    name_plot(a, pretty_fn(f, "x"));
    touch(it)
}

/// f(x, y) on an n x n grid over [a b] or [xa xb ya yb] (default [-5 5]).
fn fgrid(it: &mut Interp, name: &str, args: &[Value], n: usize) -> Result<(Matrix, Matrix, Matrix), String> {
    let f = args.first().ok_or_else(|| format!("{name}: needs a function"))?;
    let r = args.get(1).filter(|v| is_num(v)).map_or(Ok(vec![-5.0, 5.0]), numbers)?;
    let (xa, xb, ya, yb) = match r[..] {
        [a, b] => (a, b, a, b),
        [xa, xb, ya, yb] => (xa, xb, ya, yb),
        _ => return Err(format!("{name}: range must be [a b] or [xa xb ya yb]")),
    };
    let h = func(it, f, true)?;
    let (x, y) = meshgrid(&linspace(xa, xb, n), &linspace(ya, yb, n));
    let z = sample(&[x.clone(), y.clone()], |a| first(it.call(&h, a, 1))).map_err(|e| format!("{name}: {e}"))?;
    Ok((x, y, Matrix::new(n, n, z)))
}

fn fsurf(it: &mut Interp, args: &[Value], _: usize) -> R {
    let (x, y, z) = fgrid(it, "fsurf", args, 60)?;
    add3(it, vec![Series::new(Data::Surface { x, y, z, wire: false })])?;
    name_plot(it.plots.axes(), pretty_fn(&args[0], "x, y"));
    Ok(vec![])
}
fn fmesh(it: &mut Interp, args: &[Value], _: usize) -> R {
    let (x, y, z) = fgrid(it, "fmesh", args, 40)?;
    add3(it, vec![Series::new(Data::Surface { x, y, z, wire: true })])?;
    name_plot(it.plots.axes(), pretty_fn(&args[0], "x, y"));
    Ok(vec![])
}
fn fcontour(it: &mut Interp, args: &[Value], _: usize) -> R {
    let (x, y, z) = fgrid(it, "fcontour", args, 100)?;
    let a = add(it, vec![Series::new(Data::Contour { x, y, z, levels: vec![] })]);
    tight(a);
    name_plot(a, pretty_fn(&args[0], "x, y"));
    touch(it)
}
fn fimplicit(it: &mut Interp, args: &[Value], _: usize) -> R {
    let (x, y, z) = fgrid(it, "fimplicit", args, 150)?;
    let a = add(it, vec![Series::new(Data::Contour { x, y, z, levels: vec![0.0] })]);
    tight(a);
    name_plot(a, format!("{} = 0", pretty_fn(&args[0], "x, y")));
    touch(it)
}

// ---- annotations and axes state ----

fn text_arg(args: &[Value]) -> Result<String, String> {
    Ok(match args.first() {
        None => String::new(),
        Some(Value::Str(s)) => s.clone(),
        // ponytail: multi-line titles are joined on one line
        Some(Value::Cell(c)) => c.iter().map(|v| v.as_str().map(str::to_string)).collect::<Result<Vec<_>, _>>()?.join(" "),
        Some(v) => v.scalar()?.to_string(),
    })
}

fn title(it: &mut Interp, args: &[Value], _: usize) -> R {
    it.plots.axes().title = text_arg(args)?;
    touch(it)
}
fn xlabel(it: &mut Interp, args: &[Value], _: usize) -> R {
    it.plots.axes().xlabel = text_arg(args)?;
    touch(it)
}
fn ylabel(it: &mut Interp, args: &[Value], _: usize) -> R {
    it.plots.axes().ylabel = text_arg(args)?;
    touch(it)
}
fn zlabel(it: &mut Interp, args: &[Value], _: usize) -> R {
    it.plots.axes().zlabel = text_arg(args)?;
    touch(it)
}

/// legend('a', 'b' | {'a', 'b'} | 'show' | 'off' | 'toggle')
fn legend(it: &mut Interp, args: &[Value], _: usize) -> R {
    let mut names = vec![];
    for v in args {
        match v {
            Value::Cell(c) => names.extend(c.iter().map(|v| v.as_str().map(str::to_string)).collect::<Result<Vec<_>, _>>()?),
            v => names.push(v.as_str()?.to_string()),
        }
    }
    // ponytail: placement is automatic ("best"); 'Location', ... is accepted and ignored
    if let Some(p) = names.iter().position(|s| s.eq_ignore_ascii_case("location")) {
        names.truncate(p);
    }
    let a = it.plots.axes();
    match names.first().map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("off" | "hide") if names.len() == 1 => a.legend = false,
        Some("toggle") if names.len() == 1 => a.legend = !a.legend,
        None | Some("show" | "on") if names.len() <= 1 => {
            a.legend = true;
            for (i, s) in a.series.iter_mut().enumerate().filter(|(_, s)| s.label.is_empty()) {
                s.label = format!("data{}", i + 1);
            }
        }
        _ => {
            for (s, n) in a.series.iter_mut().zip(names) {
                s.label = n;
            }
            a.legend = true;
        }
    }
    touch(it)
}

/// datatip(x, y[, z]) pins a data tip on the current axes; `datatip clear` removes them all.
fn datatip(it: &mut Interp, args: &[Value], _: usize) -> R {
    let a = it.plots.axes();
    match args {
        [Value::Str(s)] if s == "clear" => a.pins.clear(),
        [x, y] | [x, y, _] => {
            let z = args.get(2).map_or(Ok(f64::NAN), Value::scalar)?;
            a.pins.push([x.scalar()?, y.scalar()?, z]);
        }
        _ => return Err("datatip: expected (x, y[, z]) or 'clear'".into()),
    }
    touch(it)
}

/// keypoints [on | off]: mark the key points of the current axes' lines (no argument toggles).
/// p = keypoints() | keypoints(x, y): struct of them for the axes' first line, or the data: max,
/// min, roots, inflection (N x 2 [x y]), vertex ([] or [h k]), yintercept ([] or f(0)), symmetry
/// ('even', 'odd', 'x = 2', 'point (0, 1)' or 'none').
fn keypoints(it: &mut Interp, args: &[Value], nargout: usize) -> R {
    let k = match args {
        [x, y] if is_num(x) && is_num(y) => {
            let (x, y) = (x.matrix()?.data, y.matrix()?.data);
            if x.len() != y.len() {
                return Err("keypoints: x and y must have the same length".into());
            }
            analysis::analyze(&x, &y).ok_or("keypoints: needs at least 5 finite points in increasing x")?
        }
        [] if nargout > 0 => {
            let p = &it.plots;
            let a = p.figures.get(p.current).and_then(|f| f.axes.get(f.current));
            let lines = a.into_iter().flat_map(|a| &a.series).filter_map(|s| if let Data::Line { x, y } = &s.data { Some((x, y)) } else { None });
            lines.filter_map(|(x, y)| analysis::analyze(x, y)).next().ok_or("keypoints: no line to analyze in the current axes")?
        }
        _ => {
            let a = it.plots.axes();
            a.keypoints = switch("keypoints", args, a.keypoints, &[])?;
            return touch(it);
        }
    };
    let pts = |v: &[[f64; 2]]| Value::Num(Matrix::new(v.len(), 2, v.iter().map(|p| p[0]).chain(v.iter().map(|p| p[1])).collect()));
    let fields = [
        ("max", pts(&k.max)),
        ("min", pts(&k.min)),
        ("roots", pts(&k.roots)),
        ("inflection", pts(&k.inflection)),
        ("vertex", k.vertex.map_or_else(Value::empty, |v| Value::from(v.to_vec()))),
        ("yintercept", k.y_int.map_or_else(Value::empty, |p| Value::num(p[1]))),
        ("symmetry", Value::Str(k.symmetry.text())),
    ];
    Ok(vec![Value::Struct(fields.into_iter().map(|(n, v)| (n.to_string(), v)).collect())])
}

/// text(x, y[, z], str[, 'HorizontalAlignment', h]): notes at data coords. Vector positions take
/// one string each from a cell or share one string; a cell for one position is multi-line.
// ponytail: other properties (FontSize, Color, Rotation, ...) are accepted and ignored
fn text(it: &mut Interp, args: &[Value], _: usize) -> R {
    let (nums, rest) = leading(args)?;
    if !(2..=3).contains(&nums.len()) || nums.iter().any(|m| m.len() != nums[0].len()) {
        return Err("text: expected x, y[, z] of the same length, then the text".into());
    }
    let (txt, props) = rest.split_first().ok_or("text: no text")?;
    let one = |v: &Value| match v {
        Value::Str(s) => Ok(s.clone()),
        v => v.scalar().map(cursor::num),
    };
    let n = nums[0].len();
    let texts = match txt {
        Value::Cell(c) if c.len() == n && n > 1 => c.iter().map(one).collect::<Result<Vec<_>, _>>()?,
        Value::Cell(c) => vec![c.iter().map(one).collect::<Result<Vec<_>, _>>()?.join("\n"); n],
        v => vec![one(v)?; n],
    };
    let mut align = 0.0;
    for p in props.chunks(2) {
        if p[0].as_str()?.eq_ignore_ascii_case("horizontalalignment") {
            let h = p.get(1).ok_or("text: HorizontalAlignment needs a value")?.as_str()?;
            align = match h.to_ascii_lowercase().as_str() {
                "center" | "middle" => 0.5,
                "right" => 1.0,
                _ => 0.0,
            };
        }
    }
    let a = it.plots.axes();
    for (k, text) in texts.into_iter().enumerate() {
        let at = [nums[0].data[k], nums[1].data[k], nums.get(2).map_or(f64::NAN, |z| z.data[k])];
        a.notes.push(Note { at, text, align });
    }
    touch(it)
}

/// on | off | toggle (no argument) for grid and hold.
fn switch(name: &str, args: &[Value], cur: bool, extra: &[&str]) -> Result<bool, String> {
    match args.first().map(|v| v.as_str().map(str::to_ascii_lowercase)).transpose()?.as_deref() {
        None => Ok(!cur),
        Some("on") => Ok(true),
        Some("off") => Ok(false),
        Some(s) if extra.contains(&s) => Ok(true),
        Some(s) => Err(format!("{name}: expected on or off, got '{s}'")),
    }
}

fn grid(it: &mut Interp, args: &[Value], _: usize) -> R {
    let a = it.plots.axes();
    // ponytail: 'minor' just turns the grid on; log axes get minor lines anyway
    a.grid = switch("grid", args, a.grid, &["minor"])?;
    touch(it)
}

fn hold(it: &mut Interp, args: &[Value], _: usize) -> R {
    let a = it.plots.axes();
    a.hold = switch("hold", args, a.hold, &["all"])?;
    touch(it)
}

fn ishold(it: &mut Interp, _: &[Value], _: usize) -> R {
    Ok(vec![Value::bool(it.plots.axes().hold)])
}

/// Equal data units on both axes.
// ponytail: assumes the usual ~4:3 plot box; exact needs the renderer to know pixel aspect
fn equal(a: &mut Axes) {
    let [bx, by, _] = a.bounds();
    let (cx, cy) = ((bx.0 + bx.1) / 2.0, (by.0 + by.1) / 2.0);
    let (mut wx, mut wy) = ((bx.1 - bx.0) / 2.0, (by.1 - by.0) / 2.0);
    if wx < wy * 1.35 {
        wx = wy * 1.35;
    } else {
        wy = wx / 1.35;
    }
    (a.xlim, a.ylim) = (Some((cx - wx, cx + wx)), Some((cy - wy, cy + wy)));
    a.equal = true; // the renderer matches the plot box's actual shape
}

fn axis(it: &mut Interp, args: &[Value], nargout: usize) -> R {
    let a = it.plots.axes();
    for v in args {
        match v {
            Value::Str(s) => match s.to_ascii_lowercase().as_str() {
                "auto" | "normal" => (a.xlim, a.ylim, a.zlim, a.equal) = (None, None, None, false),
                "tight" => tight(a),
                "equal" | "image" => {
                    tight(a);
                    if !a.is_3d() {
                        equal(a);
                    }
                }
                "manual" => {
                    let b = a.bounds();
                    (a.xlim, a.ylim, a.zlim) = (Some(b[0]), Some(b[1]), Some(b[2]));
                }
                // ponytail: box shape and axis visibility are not modeled
                // y direction: "ij" runs down like a matrix (imagesc's default), "xy" up
                "xy" => {
                    if let Some((lo, hi)) = a.ylim.filter(|(lo, hi)| lo > hi) {
                        a.ylim = Some((hi, lo));
                    }
                }
                "ij" => {
                    let (lo, hi) = a.bounds()[1];
                    a.ylim = Some((lo.max(hi), lo.min(hi)));
                }
                "square" | "on" | "off" | "fill" | "vis3d" => {}
                o => return Err(format!("axis: unknown option '{o}'")),
            },
            v => {
                let m = numbers(v)?;
                if !(m.len() == 4 || m.len() == 6) || m.chunks(2).any(|p| !(p[0] < p[1])) {
                    return Err("axis: limits must be increasing [x0 x1 y0 y1] or [x0 x1 y0 y1 z0 z1]".into());
                }
                (a.xlim, a.ylim) = (Some((m[0], m[1])), Some((m[2], m[3])));
                if m.len() == 6 {
                    a.zlim = Some((m[4], m[5]));
                }
            }
        }
    }
    let b = a.bounds();
    let three = a.is_3d();
    if !args.is_empty() {
        it.plots.touch();
        super::resample(it);
        if nargout == 0 {
            return Ok(vec![]);
        }
    }
    let mut v = vec![b[0].0, b[0].1, b[1].0, b[1].1];
    if three {
        v.extend([b[2].0, b[2].1]);
    }
    Ok(vec![Value::from(v)])
}

/// xlim/ylim/zlim: set [lo hi] (Inf = keep current), 'auto', 'manual', or return current.
fn lim(it: &mut Interp, args: &[Value], nargout: usize, k: usize) -> R {
    let name = ["xlim", "ylim", "zlim"][k];
    let a = it.plots.axes();
    let cur = a.bounds()[k];
    let slot = match k {
        0 => &mut a.xlim,
        1 => &mut a.ylim,
        _ => &mut a.zlim,
    };
    match args.first() {
        None => return Ok(vec![Value::from(vec![cur.0, cur.1])]),
        Some(Value::Str(s)) if s == "auto" => *slot = None,
        Some(Value::Str(s)) if s == "manual" => *slot = Some(cur),
        Some(v) => {
            let [lo, hi] = numbers(v)?[..] else { return Err(format!("{name}: expected [lo hi]")) };
            let (lo, hi) = (if lo.is_finite() { lo } else { cur.0 }, if hi.is_finite() { hi } else { cur.1 });
            if !(lo < hi) {
                return Err(format!("{name}: limits must be increasing"));
            }
            *slot = Some((lo, hi));
        }
    }
    let b = a.bounds()[k];
    it.plots.touch();
    if k == 0 {
        super::resample(it); // the curve follows a wider view
    }
    Ok(if nargout > 0 { vec![Value::from(vec![b.0, b.1])] } else { vec![] })
}
fn xlim(it: &mut Interp, args: &[Value], n: usize) -> R {
    lim(it, args, n, 0)
}
fn ylim(it: &mut Interp, args: &[Value], n: usize) -> R {
    lim(it, args, n, 1)
}
fn zlim(it: &mut Interp, args: &[Value], n: usize) -> R {
    lim(it, args, n, 2)
}

/// view(az, el) | view([az el]) | view(2) | view(3), degrees; returns [az el].
fn view(it: &mut Interp, args: &[Value], nargout: usize) -> R {
    let nums = args.iter().map(numbers).collect::<Result<Vec<_>, _>>()?.concat();
    let a = it.plots.axes();
    match nums[..] {
        [] => {}
        [2.0] => (a.view, a.force3d) = (View { yaw: 0.0, pitch: 90f64.to_radians(), zoom: 1.0 }, false),
        [3.0] => (a.view, a.force3d) = (View::default(), true),
        [az, el] => {
            a.view = View { yaw: az.to_radians(), pitch: el.to_radians(), zoom: a.view.zoom };
            a.force3d = !(az == 0.0 && el == 90.0);
        }
        _ => return Err("view: expected (az, el), [az el], 2 or 3".into()),
    }
    let v = a.view;
    if !nums.is_empty() {
        it.plots.touch();
    }
    Ok(if nargout > 0 || args.is_empty() { vec![Value::from(vec![v.yaw.to_degrees(), v.pitch.to_degrees()])] } else { vec![] })
}

/// colormap(name) sets the figure's map; returns it as a 64x3 matrix.
fn colormap(it: &mut Interp, args: &[Value], nargout: usize) -> R {
    let f = it.plots.fig();
    if let Some(v) = args.first() {
        // ponytail: named maps only, no custom Nx3 matrices
        let name = v.as_str().map_err(|_| "colormap: expected a name (viridis, jet, hot, gray, cool, parula, turbo)")?.to_ascii_lowercase();
        let name = if name == "default" { "viridis".to_string() } else { name };
        if render::colormap_table(&name).is_none() {
            return Err(format!("colormap: unknown colormap '{name}'"));
        }
        f.colormap = name;
    }
    let name = f.colormap.clone();
    if !args.is_empty() {
        it.plots.touch();
        if nargout == 0 {
            return Ok(vec![]);
        }
    }
    let rows: Vec<Vec<f64>> = (0..64).map(|i| render::cmap(&name, i as f64 / 63.0).iter().map(|&c| c as f64 / 255.0).collect()).collect();
    Ok(vec![Value::Num(Matrix::from_rows(&rows))])
}

/// figure (new) | figure(n) (select or create); returns n.
fn figure(it: &mut Interp, args: &[Value], nargout: usize) -> R {
    let s = &mut it.plots;
    let id = match args.first().and_then(|v| numbers(v).ok()).and_then(|v| v.first().copied()) {
        Some(n) if n >= 1.0 && n.fract() == 0.0 => n as usize,
        Some(n) => return Err(format!("figure: invalid figure number {n}")),
        None => s.figures.iter().map(|f| f.id).max().unwrap_or(0) + 1,
    };
    let old = s.figures.iter().position(|f| f.id == id);
    s.current = old.unwrap_or_else(|| {
        s.figures.push(Figure::new(id));
        s.figures.len() - 1
    });
    s.touch();
    if old.is_some() {
        it.skip_log(); // just switching figures; a new one starts its log with this statement
    }
    Ok(if nargout > 0 { vec![Value::num(id as f64)] } else { vec![] })
}

fn clf(it: &mut Interp, _: &[Value], _: usize) -> R {
    let f = it.plots.fig();
    *f = Figure::new(f.id); // its Octave log too
    it.plots.touch();
    it.skip_log();
    Ok(vec![])
}

/// close | close all | close(n)
fn close(it: &mut Interp, args: &[Value], _: usize) -> R {
    let s = &mut it.plots;
    match args.first() {
        Some(Value::Str(a)) if a == "all" => s.figures.clear(),
        Some(v) => {
            let id = numbers(v)?.first().copied().ok_or("close: expected a figure number")?;
            let i = s.figures.iter().position(|f| f.id as f64 == id).ok_or_else(|| format!("close: no figure {id}"))?;
            s.figures.remove(i);
        }
        None if !s.figures.is_empty() => {
            s.figures.remove(s.current.min(s.figures.len() - 1));
        }
        None => {}
    }
    s.current = s.figures.len().saturating_sub(1);
    s.touch();
    it.skip_log();
    Ok(vec![])
}

/// subplot(r, c, i) | subplot(rci), 1-based row-major i.
fn subplot(it: &mut Interp, args: &[Value], _: usize) -> R {
    // ponytail: spanning subplot(2, 2, [1 2]) uses the first cell
    let v: Vec<f64> = args.iter().map(|a| numbers(a).map(|n| n.first().copied().unwrap_or(0.0))).collect::<Result<_, _>>()?;
    let (r, c, i) = match v[..] {
        [r, c, i] => (r, c, i),
        [n] => ((n / 100.0).floor(), (n / 10.0).floor() % 10.0, n % 10.0),
        _ => return Err("subplot: expected (rows, cols, index)".into()),
    };
    if [r, c, i].iter().any(|v| v.fract() != 0.0 || *v < 1.0) || i > r * c {
        return Err(format!("subplot: index {i} out of range for {r}x{c}"));
    }
    let f = it.plots.fig();
    // ponytail: one grid per figure; changing the grid shape drops the old axes
    if (f.rows, f.cols) != (r as usize, c as usize) {
        f.axes.clear();
    }
    f.subplot(r as usize, c as usize, i as usize - 1);
    touch(it)
}

/// print('f.png') | print -dsvg f | saveas(h, 'f.png'): current figure, light theme, 800x600.
fn print(it: &mut Interp, args: &[Value], _: usize) -> R {
    let (mut file, mut dev, mut id) = (None, None, None);
    for v in args {
        match v {
            Value::Str(s) if s.starts_with("-d") => dev = Some(s[2..].to_ascii_lowercase()),
            Value::Str(s) if s.starts_with('-') => {} // -r300 and friends
            Value::Str(s) if file.is_none() => file = Some(s.clone()),
            Value::Str(s) => dev = Some(s.to_ascii_lowercase()),
            v => id = Some(v.scalar()?),
        }
    }
    let mut file = file.ok_or("print: no file name")?;
    if Path::new(&file).extension().is_none() {
        file = format!("{file}.{}", dev.as_deref().unwrap_or("png"));
    }
    let s = &mut it.plots;
    let fig: &Figure = match id {
        Some(id) => s.figures.iter().find(|f| f.id as f64 == id).ok_or_else(|| format!("print: no figure {id}"))?,
        None => s.fig(),
    };
    render::save(fig, &Theme::light(), Path::new(&file), 800, 600).map_err(|e| format!("print: {e}"))?;
    Ok(vec![])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(it: &mut Interp, name: &str, args: &[Value], nargout: usize) -> R {
        lookup(name).unwrap_or_else(|| panic!("no builtin {name}"))(it, args, nargout)
    }
    fn s(x: &str) -> Value {
        Value::from(x)
    }
    fn row(v: &[f64]) -> Value {
        Value::from(v.to_vec())
    }

    #[test]
    fn format_strings() {
        let f = parse_fmt("r--o").unwrap();
        assert_eq!((f.color, f.line, f.marker), (color_char('r'), Some(LineStyle::Dashed), Some(Marker::Circle)));
        let f = parse_fmt("-.k;sin x;").unwrap();
        assert_eq!((f.line, f.label.as_deref()), (Some(LineStyle::DashDot), Some("sin x")));
        let mut sr = Series::new(Data::Line { x: vec![], y: vec![] });
        parse_fmt("x").unwrap().apply(&mut sr);
        assert_eq!((sr.marker, sr.line), (Marker::Cross, LineStyle::None));
        for prop in ["Color", "LineWidth", "DisplayName", "linestyle", "marker"] {
            assert!(parse_fmt(prop).is_none(), "{prop}");
        }
        assert_eq!(vectorize("x^2*sin(x)/3 + x.^2"), "x.^2.*sin(x)./3 + x.^2");
        assert_eq!(letters("sin(t) + exp(a*t)"), ['a', 't']);
        assert_eq!(letters("e^x + pi"), ['x']);
    }

    #[test]
    fn plot_populates_session() {
        let mut it = Interp::new();
        let y = Value::Num(Matrix::from_rows(&[vec![1., 2., 3.], vec![4., 5., 6.]]));
        call(&mut it, "plot", &[y.clone()], 0).unwrap(); // 3 columns of length 2
        assert_eq!(it.plots.axes().series.len(), 3);
        call(&mut it, "plot", &[row(&[1., 2.]), y, s("r:"), s("LineWidth"), Value::num(3.)], 0).unwrap();
        let a = it.plots.axes();
        assert_eq!(a.series.len(), 3); // no hold: replaced
        assert_eq!((a.series[0].width, a.series[0].line, a.series[0].color), (3.0, LineStyle::Dotted, color_char('r')));
        call(&mut it, "hold", &[s("on")], 0).unwrap();
        call(&mut it, "semilogy", &[row(&[1., 2.]), row(&[3., 4.]), s("DisplayName"), s("d")], 0).unwrap();
        let a = it.plots.axes();
        assert_eq!((a.series.len(), a.logy, a.series[3].label.as_str()), (4, true, "d"));
        assert!(call(&mut it, "plot", &[row(&[1., 2.]), row(&[1., 2., 3.])], 0).is_err());
        assert!(call(&mut it, "plot", &[row(&[1., 2.]), s("Bogus"), Value::num(1.)], 0).is_err());
        call(&mut it, "legend", &[Value::Cell(vec![s("a"), s("b")])], 0).unwrap();
        let a = it.plots.axes();
        assert!(a.legend && a.series[1].label == "b");
        assert!(it.plots.version > 0);
    }

    #[test]
    fn figures_and_subplots() {
        let mut it = Interp::new();
        call(&mut it, "subplot", &[Value::num(2.), Value::num(2.), Value::num(3.)], 0).unwrap();
        call(&mut it, "title", &[s("c3")], 0).unwrap();
        assert_eq!((it.plots.fig().axes[0].cell, it.plots.axes().title.as_str()), (2, "c3"));
        call(&mut it, "subplot", &[s("224")], 0).unwrap();
        assert_eq!(it.plots.axes().cell, 3);
        assert!(call(&mut it, "subplot", &[Value::num(2.), Value::num(2.), Value::num(5.)], 0).is_err());
        let r = call(&mut it, "figure", &[Value::num(3.)], 1).unwrap();
        assert_eq!(r[0].scalar().unwrap(), 3.0);
        assert_eq!((it.plots.figures.len(), it.plots.fig().id), (2, 3));
        call(&mut it, "figure", &[], 0).unwrap();
        assert_eq!(it.plots.fig().id, 4);
        call(&mut it, "close", &[Value::num(3.)], 0).unwrap();
        assert_eq!(it.plots.figures.iter().map(|f| f.id).collect::<Vec<_>>(), [1, 4]);
        call(&mut it, "close", &[s("all")], 0).unwrap();
        assert!(it.plots.figures.is_empty());
    }

    #[test]
    fn axes_state() {
        let mut it = Interp::new();
        call(&mut it, "plot", &[row(&[0., 10.]), row(&[0., 5.])], 0).unwrap();
        call(&mut it, "axis", &[s("tight")], 0).unwrap();
        let r = call(&mut it, "xlim", &[], 1).unwrap()[0].matrix().unwrap();
        assert!((r.data[0] - 0.0).abs() < 1e-9 && (r.data[1] - 10.0).abs() < 1e-9, "{r:?}");
        call(&mut it, "ylim", &[row(&[-1., f64::INFINITY])], 0).unwrap();
        assert_eq!(it.plots.axes().ylim, Some((-1.0, 5.0)));
        assert!(call(&mut it, "xlim", &[row(&[2., 1.])], 0).is_err());
        call(&mut it, "grid", &[], 0).unwrap();
        assert!(it.plots.axes().grid);
        call(&mut it, "view", &[Value::num(30.), Value::num(45.)], 0).unwrap();
        assert!(it.plots.axes().is_3d());
        call(&mut it, "view", &[s("2")], 0).unwrap();
        assert!(!it.plots.axes().is_3d());
        call(&mut it, "colormap", &[s("jet")], 0).unwrap();
        assert_eq!(it.plots.fig().colormap, "jet");
        assert!(call(&mut it, "colormap", &[s("nope")], 0).is_err());
        let m = call(&mut it, "colormap", &[], 1).unwrap()[0].matrix().unwrap();
        assert_eq!((m.rows, m.cols), (64, 3));
    }

    #[test]
    fn surfaces_and_data_helpers() {
        let mut it = Interp::new();
        let z = call(&mut it, "peaks", &[Value::num(5.)], 1).unwrap();
        assert_eq!(z[0].size(), (5, 5));
        assert!((z[0].matrix().unwrap().get(2, 2) - 0.981).abs() < 1e-3); // peaks(0, 0)
        call(&mut it, "peaks", &[], 0).unwrap();
        assert!(it.plots.axes().is_3d() && it.plots.axes().grid);
        // vectors expand meshgrid-style: 2 rows (y) x 3 cols (x)
        let zz = Value::Num(Matrix::from_rows(&[vec![1., 2., 3.], vec![4., 5., 6.]]));
        call(&mut it, "mesh", &[row(&[1., 2., 3.]), row(&[10., 20.]), zz.clone()], 0).unwrap();
        let Data::Surface { x, y, wire, .. } = &it.plots.axes().series[0].data else { panic!() };
        assert!(*wire && x.get(1, 2) == 3.0 && y.get(1, 2) == 20.0);
        call(&mut it, "contour", &[zz.clone(), Value::num(3.)], 0).unwrap();
        let Data::Contour { levels, .. } = &it.plots.axes().series[0].data else { panic!() };
        assert_eq!(levels, &vec![2.25, 3.5, 4.75]);
        call(&mut it, "contourf", &[zz.clone()], 0).unwrap();
        assert!(matches!(it.plots.axes().series[0].data, Data::Image { .. }));
        call(&mut it, "imagesc", &[zz], 0).unwrap();
        // row 1 on top: y runs down (Octave's axis ij); axis xy turns it up, axis ij back
        assert_eq!((it.plots.axes().xlim, it.plots.axes().ylim), (Some((0.5, 3.5)), Some((2.5, 0.5))));
        call(&mut it, "axis", &[Value::from("xy")], 0).unwrap();
        assert_eq!(it.plots.axes().ylim, Some((0.5, 2.5)));
        call(&mut it, "axis", &[Value::from("ij")], 0).unwrap();
        assert_eq!(it.plots.axes().ylim, Some((2.5, 0.5)));
        let (c, k) = (call(&mut it, "hist", &[row(&[1., 2., 2., 3., 3., 3.]), Value::num(3.)], 2).unwrap(), 0);
        assert_eq!(c[k].matrix().unwrap().data, [1., 2., 3.]);
        let sph = call(&mut it, "sphere", &[Value::num(4.)], 3).unwrap();
        assert_eq!(sph[2].size(), (5, 5));
        call(&mut it, "quiver", &[row(&[0., 1.]), row(&[0., 0.]), row(&[1., 1.]), row(&[0., 2.])], 0).unwrap();
        let Data::Quiver { u, v, .. } = &it.plots.axes().series[0].data else { panic!() };
        assert!((v[1] / u[1] - 2.0).abs() < 1e-12 && u[0] > 0.0);
        call(&mut it, "stem", &[row(&[1., 2.])], 0).unwrap();
        let a = it.plots.axes();
        assert_eq!((a.series.len(), a.series[0].color, a.series[0].color), (2, a.series[1].color, Some(palette(0))));
    }

    #[test]
    fn complex_arrows_and_datatips() {
        use crate::value::CMatrix;
        let mut it = Interp::new();
        let z = Value::complex(CMatrix::from_parts(&Matrix::row(vec![1., 0., -3.]), &Matrix::row(vec![0., 2., 4.])));
        // plot(z): real against imaginary, no warning
        call(&mut it, "plot", &[z.clone()], 0).unwrap();
        let Data::Line { x, y } = &it.plots.axes().series[0].data else { panic!() };
        assert_eq!((x.as_slice(), y.as_slice()), (&[1., 0., -3.][..], &[0., 2., 4.][..]));
        assert_eq!(it.take_output(), "");
        // complex x/y pairs: real parts, Octave's warning once per call
        let t = row(&[1., 2., 3.]);
        call(&mut it, "plot", &[t.clone(), z.clone(), t, z.clone()], 0).unwrap();
        let Data::Line { y, .. } = &it.plots.axes().series[1].data else { panic!() };
        assert_eq!(y, &[1., 0., -3.]);
        assert_eq!(it.take_output(), "warning: plot: imaginary parts of complex X and/or Y arguments ignored\n");
        call(&mut it, "bar", &[z.clone()], 0).unwrap();
        assert!(it.take_output().starts_with("warning: bar: imaginary"));
        // compass: phasors from the origin, limits around it; feather: from (k, 0) plus a baseline
        call(&mut it, "compass", &[z.clone()], 0).unwrap();
        let a = it.plots.axes();
        let Data::Quiver { x, y, u, v } = &a.series[0].data else { panic!() };
        assert_eq!((x, y, u, v), (&vec![0.; 3], &vec![0.; 3], &vec![1., 0., -3.], &vec![0., 2., 4.]));
        let (xl, yl) = (a.xlim.unwrap(), a.ylim.unwrap());
        assert!(xl.0 == -xl.1 && yl.0 == -yl.1 && yl.1 > 5.0);
        call(&mut it, "feather", &[row(&[1., 1.]), row(&[-1., 2.])], 0).unwrap();
        let a = it.plots.axes();
        let Data::Quiver { x, v, .. } = &a.series[0].data else { panic!() };
        assert_eq!((x.as_slice(), v.as_slice(), a.series.len()), (&[1., 2.][..], &[-1., 2.][..], 2));
        assert!(call(&mut it, "compass", &[row(&[1.]), row(&[1., 2.])], 0).is_err());
        // datatips: pinned, cleared by `datatip clear` and by a new plot
        call(&mut it, "datatip", &[Value::num(1.), Value::num(2.)], 0).unwrap();
        call(&mut it, "datatip", &[Value::num(3.), Value::num(4.), Value::num(5.)], 0).unwrap();
        let pins = &it.plots.axes().pins;
        assert!(pins.len() == 2 && pins[0][2].is_nan() && pins[1] == [3., 4., 5.]);
        call(&mut it, "datatip", &[s("clear")], 0).unwrap();
        assert!(it.plots.axes().pins.is_empty());
        call(&mut it, "datatip", &[Value::num(1.), Value::num(2.)], 0).unwrap();
        call(&mut it, "plot", &[row(&[1., 2.])], 0).unwrap();
        assert!(it.plots.axes().pins.is_empty());
        assert!(call(&mut it, "datatip", &[Value::num(1.)], 0).is_err());
    }

    #[test]
    fn function_sampling() {
        // vectorized call is used when the size matches
        let xs = Matrix::row(vec![1., 2., 3.]);
        let mut calls = 0;
        let y = sample(&[xs.clone()], |a| {
            calls += 1;
            Ok(Value::Num(a[0].matrix()?.map(|x| x * x)))
        })
        .unwrap();
        assert_eq!((y, calls), (vec![1., 4., 9.], 1));
        // scalar-only function: falls back to point by point
        let y = sample(&[xs], |a| a[0].scalar().map(|x| Value::num(x + 1.0))).unwrap();
        assert_eq!(y, [2., 3., 4.]);
        // tan: the pole gets a NaN break and robust limits ignore it
        let xs = linspace(-1.5, 1.6, 300);
        let ys: Vec<f64> = xs.iter().map(|x| x.tan()).collect();
        let (bx, by) = break_jumps(&xs, &ys);
        assert_eq!(by.iter().filter(|v| v.is_nan()).count(), 1);
        assert_eq!(bx.len(), xs.len() + 1);
        let (lo, hi) = robust_ylim(&ys).unwrap();
        assert!(lo > -50.0 && hi < 50.0, "{lo} {hi}");
        let saw: Vec<f64> = xs.iter().map(|x| x - x.floor()).collect();
        assert_eq!(break_jumps(&xs, &saw).1.iter().filter(|v| v.is_nan()).count(), 3);
        let sin: Vec<f64> = xs.iter().map(|x| x.sin()).collect();
        assert!(robust_ylim(&sin).is_none());
        assert_eq!(break_jumps(&xs, &sin).0.len(), xs.len());
    }

    #[test]
    fn notes_and_fills() {
        let mut it = Interp::new();
        let n = Value::num;
        call(&mut it, "text", &[row(&[1., 2.]), row(&[3., 4.]), Value::Cell(vec![s("a"), s("b_1")])], 0).unwrap();
        call(&mut it, "text", &[n(0.), n(0.), Value::Cell(vec![s("two"), s("lines")]), s("HorizontalAlignment"), s("center")], 0).unwrap();
        let a = it.plots.axes();
        assert_eq!(a.notes.iter().map(|n| n.text.as_str()).collect::<Vec<_>>(), ["a", "b_1", "two\nlines"]);
        assert!(a.notes[1].at[..2] == [2., 4.] && a.notes[1].at[2].is_nan() && a.notes[2].align == 0.5);
        assert!(call(&mut it, "text", &[n(1.), s("x")], 0).is_err());
        // area closes down to 0; a new plot drops the notes
        call(&mut it, "area", &[row(&[1., 2., 3.])], 0).unwrap();
        let a = it.plots.axes();
        let Data::Fill { x, y } = &a.series[0].data else { panic!() };
        assert_eq!((x.as_slice(), y.as_slice()), (&[1., 2., 3., 3., 1.][..], &[1., 2., 3., 0., 0.][..]));
        assert!(a.notes.is_empty());
        call(&mut it, "fill", &[row(&[0., 1., 0.]), row(&[0., 0., 1.]), s("r")], 0).unwrap();
        assert_eq!(it.plots.axes().series[0].color, color_char('r'));
        call(&mut it, "fill", &[row(&[0., 1., 0.]), row(&[0., 0., 1.]), row(&[0., 1., 0.]), s("FaceColor"), s("b")], 0).unwrap();
        assert_eq!(it.plots.axes().series[0].color, color_char('b'));
        // string-expression plots are titled and labeled in math view
        call(&mut it, "fplot", &[s("x.^2 + 2*x")], 0).unwrap();
        let a = it.plots.axes();
        assert_eq!((a.title.as_str(), a.series[0].label.as_str()), ("x² + 2x", "x² + 2x"));
        call(&mut it, "polarplot", &[row(&[0., 1.]), row(&[1., 1.])], 0).unwrap();
    }

    #[test]
    fn keypoints_builtin() {
        let mut it = Interp::new();
        it.eval("x = linspace(-1, 5, 301); p = keypoints(x, x.^2 - 4*x + 3);").unwrap();
        let Some(Value::Struct(p)) = it.vars.get("p") else { panic!() };
        assert_eq!(p["vertex"].matrix().unwrap().data, [2.0, -1.0]);
        assert_eq!(p["roots"].matrix().unwrap().data, [1.0, 3.0, 0.0, 0.0]);
        assert_eq!((p["max"].size(), p["inflection"].size()), ((0, 2), (0, 2)));
        assert_eq!((p["symmetry"].as_str().unwrap(), p["yintercept"].scalar().unwrap()), ("x = 2", 3.0));
        // toggle on the current axes, query its first line
        it.eval("fplot(@(x) x.^3 - 3*x, [-3 3]); keypoints").unwrap();
        assert!(it.plots.axes().keypoints);
        it.eval("q = keypoints(); keypoints off").unwrap();
        let Some(Value::Struct(q)) = it.vars.get("q") else { panic!() };
        assert_eq!((q["max"].matrix().unwrap().data.as_slice(), q["symmetry"].as_str().unwrap()), (&[-1.0, 2.0][..], "odd"));
        assert!(!it.plots.axes().keypoints);
        assert!(it.eval("keypoints(1:3, 1:3)").is_err() && it.eval("keypoints maybe").is_err());
        assert!(it.eval("close all; q = keypoints()").is_err());
    }

    #[test]
    fn print_writes_files() {
        let mut it = Interp::new();
        call(&mut it, "bar", &[row(&[3., 1., 2.])], 0).unwrap();
        let p = std::env::temp_dir().join("qcalcp-print-test");
        let f = p.to_string_lossy().to_string();
        call(&mut it, "print", &[s("-dsvg"), s(&f)], 0).unwrap();
        assert!(std::fs::metadata(format!("{f}.svg")).unwrap().len() > 100);
        call(&mut it, "saveas", &[Value::num(1.), s(&format!("{f}.png"))], 0).unwrap();
        assert!(std::fs::metadata(format!("{f}.png")).unwrap().len() > 100);
        let _ = std::fs::remove_file(format!("{f}.svg"));
        let _ = std::fs::remove_file(format!("{f}.png"));
        assert!(call(&mut it, "print", &[s("x.pdf")], 0).is_err());
    }

    /// Visual QA: `cargo test samples -- --ignored`, then look at samples/*.png.
    #[test]
    #[ignore]
    fn samples() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples");
        std::fs::create_dir_all(&dir).unwrap();
        let n = |x: f64| Value::num(x);
        let xs = linspace(0.0, 10.0, 400);
        let f = |g: fn(f64) -> f64| row(&xs.iter().map(|&x| g(x)).collect::<Vec<_>>());
        let save = |it: &mut Interp, name: &str, light: bool| {
            let th = if light { Theme::light() } else { Theme::dark() };
            render::save(it.plots.fig(), &th, &dir.join(name), 800, 600).unwrap();
        };
        let mut it = Interp::new();
        let c = |it: &mut Interp, name: &str, args: &[Value]| {
            call(it, name, args, 0).unwrap();
        };

        // 2D lines, legend, grid
        c(&mut it, "plot", &[row(&xs), f(f64::sin), s("-"), row(&xs), f(f64::cos), s("--"), row(&xs), f(|x| (2.0 * x).sin() / 2.0), s("-.")]);
        c(&mut it, "hold", &[s("on")]);
        let k: Vec<f64> = (0..11).map(|i| i as f64).collect();
        c(&mut it, "plot", &[row(&k), row(&k.iter().map(|x| (x * 0.7).sin() * 0.8).collect::<Vec<_>>()), s("o")]);
        c(&mut it, "legend", &[s("sin(x)"), s("cos(x)"), s("sin(2x)/2"), s("samples")]);
        c(&mut it, "grid", &[s("on")]);
        c(&mut it, "title", &[s("Trig functions")]);
        c(&mut it, "xlabel", &[s("x [rad]")]);
        c(&mut it, "ylabel", &[s("amplitude")]);
        save(&mut it, "lines-light.png", true);
        save(&mut it, "lines-dark.png", false);
        save(&mut it, "lines.svg", true);

        // tan with poles (what fplot produces)
        c(&mut it, "figure", &[]);
        let tx = linspace(-5.0, 5.0, 1001);
        let ty: Vec<f64> = tx.iter().map(|x| x.tan()).collect();
        let (bx, by) = break_jumps(&tx, &ty);
        c(&mut it, "plot", &[row(&bx), row(&by)]);
        let (lo, hi) = robust_ylim(&ty).unwrap();
        c(&mut it, "ylim", &[row(&[lo, hi])]);
        c(&mut it, "xlim", &[row(&[-5.0, 5.0])]);
        c(&mut it, "grid", &[s("on")]);
        c(&mut it, "title", &[s("tan(x)")]);
        save(&mut it, "fplot-tan-dark.png", false);

        // surfaces
        c(&mut it, "figure", &[]);
        c(&mut it, "peaks", &[]);
        c(&mut it, "title", &[s("surf(peaks)")]);
        c(&mut it, "xlabel", &[s("x")]);
        c(&mut it, "ylabel", &[s("y")]);
        c(&mut it, "zlabel", &[s("z")]);
        save(&mut it, "surf-light.png", true);
        c(&mut it, "view", &[n(50.0), n(25.0)]);
        save(&mut it, "surf-rotated-dark.png", false);
        c(&mut it, "view", &[n(150.0), n(-20.0)]);
        save(&mut it, "surf-below-light.png", true);
        let pk = call(&mut it, "peaks", &[n(30.0)], 3).unwrap();
        c(&mut it, "mesh", &pk);
        c(&mut it, "title", &[s("mesh")]);
        save(&mut it, "mesh-light.png", true);
        save(&mut it, "mesh-dark.png", false);
        c(&mut it, "figure", &[]);
        c(&mut it, "sphere", &[]);
        c(&mut it, "colormap", &[s("turbo")]);
        c(&mut it, "title", &[s("sphere, turbo")]);
        save(&mut it, "sphere-dark.png", false);

        // contour / contourf / imagesc / pcolor
        let pk = call(&mut it, "peaks", &[], 3).unwrap();
        c(&mut it, "figure", &[]);
        c(&mut it, "contour", &pk);
        c(&mut it, "title", &[s("contour(peaks)")]);
        save(&mut it, "contour-light.png", true);
        save(&mut it, "contour-dark.png", false);
        c(&mut it, "figure", &[]);
        c(&mut it, "contourf", &pk);
        c(&mut it, "title", &[s("contourf(peaks)")]);
        save(&mut it, "contourf-light.png", true);
        c(&mut it, "figure", &[]);
        c(&mut it, "imagesc", &pk[2..]);
        c(&mut it, "colormap", &[s("jet")]);
        c(&mut it, "title", &[s("imagesc(peaks), jet")]);
        save(&mut it, "imagesc-light.png", true);

        // quiver
        c(&mut it, "figure", &[]);
        let g = linspace(-2.0, 2.0, 15);
        let (qx, qy) = meshgrid(&g, &g);
        let e = |a: f64, b: f64| (-a * a - b * b).exp();
        let qu = Matrix::new(15, 15, qx.data.iter().zip(&qy.data).map(|(&a, &b)| -2.0 * a * e(a, b) - b * 0.3).collect());
        let qv = Matrix::new(15, 15, qx.data.iter().zip(&qy.data).map(|(&a, &b)| -2.0 * b * e(a, b) + a * 0.3).collect());
        c(&mut it, "quiver", &[Value::Num(qx), Value::Num(qy), Value::Num(qu), Value::Num(qv)]);
        c(&mut it, "title", &[s("quiver")]);
        save(&mut it, "quiver-light.png", true);

        // subplots: mixed kinds
        c(&mut it, "figure", &[]);
        c(&mut it, "subplot", &[n(2.0), n(2.0), n(1.0)]);
        c(&mut it, "plot", &[row(&xs), f(|x| (-x / 3.0).exp() * (3.0 * x).sin())]);
        c(&mut it, "grid", &[s("on")]);
        c(&mut it, "title", &[s("damped")]);
        c(&mut it, "subplot", &[n(2.0), n(2.0), n(2.0)]);
        let data: Vec<f64> = (0..2000).map(|i| ((i as f64 * 0.37).sin() + (i as f64 * 0.11).cos() + (i as f64 * 1.3).sin() * 0.5) * 1.5).collect();
        c(&mut it, "hist", &[row(&data), n(25.0)]);
        c(&mut it, "title", &[s("hist")]);
        c(&mut it, "subplot", &[n(2.0), n(2.0), n(3.0)]);
        c(&mut it, "surf", &pk);
        c(&mut it, "title", &[s("surf")]);
        c(&mut it, "subplot", &[n(2.0), n(2.0), n(4.0)]);
        let lx: Vec<f64> = (1..=50).map(|i| i as f64).collect();
        c(&mut it, "loglog", &[row(&lx), row(&lx.iter().map(|x| x.powf(2.5)).collect::<Vec<_>>()), s(".-")]);
        c(&mut it, "grid", &[s("on")]);
        c(&mut it, "title", &[s("loglog")]);
        save(&mut it, "subplots-light.png", true);
        save(&mut it, "subplots-dark.png", false);
        std::fs::write(dir.join("ansi.txt"), render::ansi(it.plots.fig(), &Theme::dark(), 100, 30)).unwrap();

        // the rest: scatter, bar groups, stairs, stem, errorbar, polar, plot3, semilogx
        c(&mut it, "figure", &[]);
        c(&mut it, "subplot", &[n(2.0), n(3.0), n(1.0)]);
        let sx: Vec<f64> = (0..150).map(|i| ((i * 37 % 101) as f64 / 101.0) * 4.0 - 2.0).collect();
        let sy: Vec<f64> = sx.iter().enumerate().map(|(i, x)| x * 0.8 + ((i * 53 % 97) as f64 / 97.0 - 0.5)).collect();
        c(&mut it, "scatter", &[row(&sx), row(&sy), n(30.0), row(&sy)]);
        c(&mut it, "title", &[s("scatter")]);
        c(&mut it, "subplot", &[n(2.0), n(3.0), n(2.0)]);
        c(&mut it, "bar", &[Value::Num(Matrix::from_rows(&[vec![3., 2., 1.], vec![4., 5., 2.], vec![2., 3., 4.], vec![5., 1., 3.]]))]);
        c(&mut it, "title", &[s("bar groups")]);
        c(&mut it, "subplot", &[n(2.0), n(3.0), n(3.0)]);
        let st: Vec<f64> = (0..20).map(|i| i as f64).collect();
        c(&mut it, "stairs", &[row(&st), row(&st.iter().map(|x| (x * 0.4).sin()).collect::<Vec<_>>())]);
        c(&mut it, "hold", &[s("on")]);
        c(&mut it, "stem", &[row(&st), row(&st.iter().map(|x| (x * 0.4).cos() * 0.7).collect::<Vec<_>>())]);
        c(&mut it, "title", &[s("stairs + stem")]);
        c(&mut it, "subplot", &[n(2.0), n(3.0), n(4.0)]);
        c(&mut it, "errorbar", &[row(&[1., 2., 3., 4., 5.]), row(&[2., 3., 2.5, 4., 3.5]), row(&[0.3, 0.5, 0.2, 0.6, 0.4]), s("o")]);
        c(&mut it, "title", &[s("errorbar")]);
        c(&mut it, "subplot", &[n(2.0), n(3.0), n(5.0)]);
        let th = linspace(0.0, 2.0 * std::f64::consts::PI, 300);
        c(&mut it, "polar", &[row(&th), row(&th.iter().map(|t| (4.0 * t).cos().abs() + 0.2).collect::<Vec<_>>())]);
        c(&mut it, "title", &[s("polar")]);
        c(&mut it, "subplot", &[n(2.0), n(3.0), n(6.0)]);
        let t = linspace(0.0, 20.0, 500);
        c(&mut it, "plot3", &[row(&t.iter().map(|t| t.cos()).collect::<Vec<_>>()), row(&t.iter().map(|t| t.sin()).collect::<Vec<_>>()), row(&t)]);
        c(&mut it, "title", &[s("plot3")]);
        save(&mut it, "misc-light.png", true);
        save(&mut it, "misc-dark.png", false);

        // data tips: a cursor pair on a 2D line (one at the edge to test flipping), a surface, an image
        c(&mut it, "figure", &[]);
        let g = |x: f64| (-x / 4.0).exp() * (2.0 * x).sin();
        c(&mut it, "plot", &[row(&xs), f(g)]);
        c(&mut it, "grid", &[s("on")]);
        c(&mut it, "title", &[s("datatip pair")]);
        for x in [0.785, 3.927, 9.95] {
            c(&mut it, "datatip", &[n(x), n(g(x))]);
        }
        save(&mut it, "datatips-light.png", true);
        save(&mut it, "datatips-dark.png", false);
        c(&mut it, "figure", &[]);
        c(&mut it, "peaks", &[]);
        let pz = |x: f64, y: f64| peaks_z(&Matrix::scalar(x), &Matrix::scalar(y)).data[0];
        c(&mut it, "datatip", &[n(0.0), n(1.625), n(pz(0.0, 1.625))]);
        c(&mut it, "datatip", &[n(0.25), n(-1.625), n(pz(0.25, -1.625))]);
        c(&mut it, "title", &[s("surf datatips")]);
        save(&mut it, "datatips-surf-dark.png", false);
        save(&mut it, "datatips-surf-light.png", true);
        c(&mut it, "figure", &[]);
        c(&mut it, "imagesc", &pk[2..]);
        // row 1 is drawn on top: y = 33 is row 17 of 49
        c(&mut it, "datatip", &[n(25.0), n(33.0), n(pk[2].matrix().unwrap().get(16, 24))]);
        save(&mut it, "datatips-image-light.png", true);

        // phasors
        use crate::value::CMatrix;
        let th: Vec<f64> = (0..5).map(|k| k as f64 * 1.3).collect();
        let r = Matrix::row(th.iter().map(|t| 1.0 + t / 3.0).collect());
        let zc = Value::complex(CMatrix::from_parts(
            &Matrix::row(th.iter().zip(&r.data).map(|(t, r)| r * t.cos()).collect()),
            &Matrix::row(th.iter().zip(&r.data).map(|(t, r)| r * t.sin()).collect()),
        ));
        c(&mut it, "figure", &[]);
        c(&mut it, "subplot", &[n(1.0), n(2.0), n(1.0)]);
        c(&mut it, "compass", &[zc.clone()]);
        c(&mut it, "title", &[s("compass")]);
        c(&mut it, "subplot", &[n(1.0), n(2.0), n(2.0)]);
        c(&mut it, "feather", &[zc]);
        c(&mut it, "title", &[s("feather")]);
        save(&mut it, "phasors-light.png", true);
        save(&mut it, "phasors-dark.png", false);
    }
}
