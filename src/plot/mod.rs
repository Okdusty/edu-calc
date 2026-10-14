//! Figure model. CONTRACT shared by plot builtins, renderer, GUI, TUI, CLI; change only additively.
mod analysis;
pub mod builtins;
mod calculus;
pub mod cursor;
mod mathtext;
pub mod render;

use crate::lang::Interp;
use crate::value::{Matrix, Value};

pub type Rgb = [u8; 3];

/// All figures of one interpreter. Frontends redraw when `version` changes.
#[derive(Clone, Debug, Default)]
pub struct Session {
    pub figures: Vec<Figure>,
    pub current: usize,
    pub version: u64,
}

impl Session {
    /// Current figure, created on demand.
    pub fn fig(&mut self) -> &mut Figure {
        if self.figures.is_empty() {
            self.figures.push(Figure::new(1));
            self.current = 0;
        }
        &mut self.figures[self.current]
    }
    /// Current axes of the current figure, created on demand.
    pub fn axes(&mut self) -> &mut Axes {
        self.fig().axes()
    }
    pub fn touch(&mut self) {
        self.version += 1;
    }
}

#[derive(Clone, Debug)]
pub struct Figure {
    /// Octave figure number (1-based)
    pub id: usize,
    /// subplot grid
    pub rows: usize,
    pub cols: usize,
    pub axes: Vec<Axes>,
    /// index into `axes`
    pub current: usize,
    /// "viridis" | "jet" | "hot" | "gray" | "cool" | ...
    pub colormap: String,
    /// Octave statements that built this figure, in order (recorded by the interpreter for
    /// top-level statements that changed it; calculator plots store their fplot/fsurf code).
    pub code: Vec<String>,
    /// each axes' view when `code` was last logged: `octave()` adds lines for what changed since
    logged: Vec<ViewState>,
}

/// What `Figure::octave` tracks per axes: xlim, ylim, zlim, camera, key points.
type ViewState = (Option<(f64, f64)>, Option<(f64, f64)>, Option<(f64, f64)>, View, bool);

fn view_state(a: &Axes) -> ViewState {
    (a.xlim, a.ylim, a.zlim, a.view, a.keypoints)
}

/// `xlim([-12.5 12.5]);` with the limits rounded to 4 significant digits of their span;
/// `xlim('auto');` for None.
fn lim_code(name: &str, l: Option<(f64, f64)>) -> String {
    let Some((a, b)) = l else { return format!("{name}('auto');") };
    let e = ((b - a).abs().log10().floor() as i32 - 3).clamp(-20, 300);
    let r = |v: f64| {
        let s = format!("{:.*}", (-e).max(0) as usize, (v / 10f64.powi(e)).round() * 10f64.powi(e));
        let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.') } else { &s };
        if s == "-0" { "0".to_string() } else { s.to_string() }
    };
    format!("{name}([{} {}]);", r(a), r(b))
}

impl Figure {
    pub fn new(id: usize) -> Self {
        Figure { id, rows: 1, cols: 1, axes: vec![], current: 0, colormap: "viridis".into(), code: vec![], logged: vec![] }
    }
    pub fn axes(&mut self) -> &mut Axes {
        if self.axes.is_empty() {
            self.axes.push(Axes::new(0));
            self.current = 0;
        }
        &mut self.axes[self.current]
    }
    /// `subplot(rows, cols, cell)` with 0-based row-major cell; selects (or creates) that axes.
    pub fn subplot(&mut self, rows: usize, cols: usize, cell: usize) -> &mut Axes {
        self.rows = rows.max(1);
        self.cols = cols.max(1);
        self.current = match self.axes.iter().position(|a| a.cell == cell) {
            Some(i) => i,
            None => {
                self.axes.push(Axes::new(cell));
                self.axes.len() - 1
            }
        };
        &mut self.axes[self.current]
    }
    /// Pixel rect (x, y, w, h) of an axes cell in a w x h canvas. Renderer and hit-testing both use this.
    pub fn cell_rect(&self, cell: usize, w: f64, h: f64) -> (f64, f64, f64, f64) {
        let (cw, ch) = (w / self.cols as f64, h / self.rows as f64);
        ((cell % self.cols) as f64 * cw, (cell / self.cols) as f64 * ch, cw, ch)
    }
    /// Octave code that rebuilds this figure as it looks now: `code`, then the current view
    /// (limits, camera, key points) of every axes. Follows pans/zooms without the user editing code.
    /// Only what changed since the last logged statement is added (none of it accumulates), so
    /// `fplot(f, [-5 5]); xlim([-20 20])` stays as typed until the view moves.
    pub fn octave(&self) -> String {
        let mut out = self.code.clone();
        let several = self.axes.len() > 1;
        let mut moved = None;
        for (i, a) in self.axes.iter().enumerate() {
            let (xl, yl, zl, v, kp) = self.logged.get(i).copied().unwrap_or((None, None, None, View::default(), false));
            let mut l = vec![];
            for (name, now, was) in [("xlim", a.xlim, xl), ("ylim", a.ylim, yl), ("zlim", a.zlim, zl)] {
                if now != was {
                    l.push(lim_code(name, now));
                }
            }
            let deg = |v: View| [v.yaw, v.pitch].map(|r| (r.to_degrees() * 10.0).round() / 10.0);
            if a.is_3d() && deg(a.view) != deg(v) {
                let [az, el] = deg(a.view);
                l.push(format!("view({az}, {el});"));
            }
            if a.keypoints != kp {
                l.push(format!("keypoints {};", if a.keypoints { "on" } else { "off" }));
            }
            if l.is_empty() {
                continue;
            }
            if several {
                l.insert(0, format!("subplot({}, {}, {});", self.rows, self.cols, a.cell + 1));
                moved = Some(i);
            }
            out.push(l.join(" "));
        }
        // leave the current axes current
        if moved.is_some_and(|i| i != self.current)
            && let Some(a) = self.axes.get(self.current)
        {
            out.push(format!("subplot({}, {}, {});", self.rows, self.cols, a.cell + 1));
        }
        out.join("\n")
    }
    /// One axes' limits as Octave, e.g. `xlim([-12.5 12.5]); ylim([-3 30]);` (with its
    /// `subplot(...)` when the figure has several); empty when nothing is set.
    pub fn view_code(&self, i: usize) -> String {
        let Some(a) = self.axes.get(i) else { return String::new() };
        let mut l: Vec<String> = [("xlim", a.xlim), ("ylim", a.ylim)].into_iter().filter(|(_, v)| v.is_some()).map(|(n, v)| lim_code(n, v)).collect();
        if self.axes.len() > 1 && !l.is_empty() {
            l.insert(0, format!("subplot({}, {}, {});", self.rows, self.cols, a.cell + 1));
        }
        l.join(" ")
    }
    /// Append a statement (the interpreter's log of what built this figure) and remember the view
    /// it leaves, so `octave()` only adds what changes after it.
    pub(crate) fn log(&mut self, stmt: &str) {
        let s = stmt.trim().trim_end_matches([';', ',']).trim_end();
        if !s.is_empty() {
            self.code.push(format!("{s};"));
        }
        self.logged = self.axes.iter().map(view_state).collect();
    }
    /// Index into `axes` under pixel (x, y) of a w x h canvas.
    pub fn axes_at(&self, x: f64, y: f64, w: f64, h: f64) -> Option<usize> {
        self.axes.iter().position(|a| {
            let (ax, ay, aw, ah) = self.cell_rect(a.cell, w, h);
            x >= ax && x < ax + aw && y >= ay && y < ay + ah
        })
    }
}

/// 3D camera, radians. Octave default `view(-37.5, 30)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub yaw: f64,
    pub pitch: f64,
    pub zoom: f64,
}

impl Default for View {
    fn default() -> Self {
        View { yaw: (-37.5f64).to_radians(), pitch: 30f64.to_radians(), zoom: 1.0 }
    }
}

#[derive(Clone, Debug)]
pub struct Axes {
    /// 0-based row-major cell in the figure's subplot grid
    pub cell: usize,
    pub series: Vec<Series>,
    pub title: String,
    pub xlabel: String,
    pub ylabel: String,
    pub zlabel: String,
    pub xlim: Option<(f64, f64)>,
    pub ylim: Option<(f64, f64)>,
    pub zlim: Option<(f64, f64)>,
    pub logx: bool,
    pub logy: bool,
    pub hold: bool,
    pub grid: bool,
    pub legend: bool,
    /// force 3D even without 3D series (`view(3)`)
    pub force3d: bool,
    pub view: View,
    /// pinned data tips (data coords; z ignored in 2D), drawn by the renderer so exports keep them
    pub pins: Vec<[f64; 3]>,
    /// text annotations (`text(x, y, str)`)
    pub notes: Vec<Note>,
    /// mark key points of line series (extrema, roots, intercepts, inflections, vertex and
    /// symmetry): computed from the data at render time, so they follow resampling (`keypoints on`)
    pub keypoints: bool,
}

/// Text placed at data coords (z ignored in 2D). Like all plot text it may use math markup.
#[derive(Clone, Debug)]
pub struct Note {
    pub at: [f64; 3],
    pub text: String,
    /// horizontal anchor: 0 = text starts at `at` (Octave's default), 0.5 = centered, 1 = ends there
    pub align: f64,
}

impl Axes {
    pub fn new(cell: usize) -> Self {
        Axes {
            cell,
            series: vec![],
            title: String::new(),
            xlabel: String::new(),
            ylabel: String::new(),
            zlabel: String::new(),
            xlim: None,
            ylim: None,
            zlim: None,
            logx: false,
            logy: false,
            hold: false,
            grid: false,
            legend: false,
            force3d: false,
            view: View::default(),
            pins: vec![],
            notes: vec![],
            keypoints: false,
        }
    }
    /// Add a series, replacing existing ones unless `hold` is on.
    pub fn add(&mut self, s: Series) {
        if !self.hold {
            self.series.clear();
            self.pins.clear();
            self.notes.clear();
        }
        self.series.push(s);
    }
    pub fn is_3d(&self) -> bool {
        self.force3d || self.series.iter().any(|s| matches!(s.data, Data::Line3 { .. } | Data::Surface { .. }))
    }
    /// Effective x/y/z ranges: explicit limits win, else finite data extent (+5% pad in 2D),
    /// never degenerate. Renderer draws exactly these; GUI pans/zooms by writing xlim/ylim/zlim.
    pub fn bounds(&self) -> [(f64, f64); 3] {
        let mut ext = [(f64::INFINITY, f64::NEG_INFINITY); 3];
        let mut eat = |k: usize, v: &[f64]| {
            for &x in v.iter().filter(|x| x.is_finite()) {
                ext[k] = (ext[k].0.min(x), ext[k].1.max(x));
            }
        };
        for s in &self.series {
            match &s.data {
                Data::Line { x, y } | Data::Fill { x, y } => {
                    eat(0, x);
                    eat(1, y);
                }
                Data::Bar { x, y, width } => {
                    let lo: Vec<f64> = x.iter().map(|v| v - width / 2.0).collect();
                    let hi: Vec<f64> = x.iter().map(|v| v + width / 2.0).collect();
                    eat(0, &lo);
                    eat(0, &hi);
                    eat(1, y);
                    eat(1, &[0.0]);
                }
                Data::Line3 { x, y, z } => {
                    eat(0, x);
                    eat(1, y);
                    eat(2, z);
                }
                Data::Surface { x, y, z, .. } | Data::Contour { x, y, z, .. } => {
                    eat(0, &x.data);
                    eat(1, &y.data);
                    eat(2, &z.data);
                }
                Data::Image { z, x, y } => {
                    eat(0, &[x.0, x.1]);
                    eat(1, &[y.0, y.1]);
                    eat(2, &z.data);
                }
                Data::Quiver { x, y, u, v } => {
                    let tx: Vec<f64> = x.iter().zip(u).map(|(a, b)| a + b).collect();
                    let ty: Vec<f64> = y.iter().zip(v).map(|(a, b)| a + b).collect();
                    eat(0, x);
                    eat(0, &tx);
                    eat(1, y);
                    eat(1, &ty);
                }
            }
        }
        let pad = if self.is_3d() { 0.0 } else { 0.05 };
        let lims = [self.xlim, self.ylim, self.zlim];
        std::array::from_fn(|k| {
            if let Some(l) = lims[k] {
                return l;
            }
            let (lo, hi) = ext[k];
            if lo > hi {
                (0.0, 1.0)
            } else if lo == hi {
                (lo - 1.0, hi + 1.0)
            } else {
                let p = (hi - lo) * pad;
                (lo - p, hi + p)
            }
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LineStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
    DashDot,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Marker {
    #[default]
    None,
    Point,
    Circle,
    Cross,
    Plus,
    Star,
    Square,
    Diamond,
    Triangle,
}

#[derive(Clone, Debug)]
pub struct Series {
    pub data: Data,
    /// legend text; empty = not in legend
    pub label: String,
    /// None = theme palette by series index
    pub color: Option<Rgb>,
    pub line: LineStyle,
    pub marker: Marker,
    pub width: f64,
    /// where a `Data::Line` came from when it samples a function of x: lets `resample` redraw it
    /// over whatever x-range is visible (zoom out = the curve keeps going)
    pub source: Option<Source>,
}

impl Series {
    pub fn new(data: Data) -> Self {
        Series { data, label: String::new(), color: None, line: LineStyle::Solid, marker: Marker::None, width: 1.5, source: None }
    }
}

/// Function behind a sampled curve.
#[derive(Clone, Debug)]
pub enum Source {
    /// Octave callable of one argument (fplot, ezplot, intplot's curve ...)
    Func(Value),
    /// libqalculate expression in `var` (calculator `plot(...)`), sampled with `qalc::sample`
    Qalc { expr: String, var: String },
}

/// Re-sample every sourced line of the current figure over its axes' visible x-range (plus a
/// margin), keeping its style. True when anything changed (frontends then redraw). Frontends call
/// it after pans/zooms; `xlim` calls it too.
/// Sampled like fplot (1001 points) over the view plus half a span each side, or just the view when
/// ylim is automatic (the y-range then follows the data, so it should be the visible part). Skipped
/// while the samples still cover the view with at least 250 points; log x axes are left alone.
pub fn resample(it: &mut Interp) -> bool {
    let cur = it.plots.current;
    let Some(fig) = it.plots.figures.get(cur) else { return false };
    let mut jobs = vec![];
    for (i, a) in fig.axes.iter().enumerate() {
        let Some((v0, v1)) = a.xlim.filter(|_| !a.is_3d() && !a.logx) else { continue };
        let span = v1 - v0;
        if !(span > 0.0 && span.is_finite()) {
            continue; // reversed (matview) or broken limits
        }
        let m = if a.ylim.is_some() { span / 2.0 } else { 0.0 };
        for (j, s) in a.series.iter().enumerate() {
            let (Some(src), Data::Line { x, .. }) = (&s.source, &s.data) else { continue };
            let (lo, hi) = builtins::range(x);
            if lo <= v0 && hi >= v1 && x.iter().filter(|v| (v0..=v1).contains(*v)).count() >= 250 {
                continue;
            }
            jobs.push((i, j, src.clone(), v0 - m, v1 + m));
        }
    }
    let mut changed = false;
    for (i, j, src, lo, hi) in jobs {
        let data = match &src {
            Source::Func(h) => builtins::curve(it, "resample", h, lo, hi).map(|(x, y, _)| (x, y)),
            Source::Qalc { expr, var } => crate::qalc::sample(expr, var, lo, hi, 1001).map(|(x, y)| builtins::break_jumps(&x, &y)),
        };
        // ponytail: a function that fails over the new range keeps its old samples
        let slot = it.plots.figures.get_mut(cur).and_then(|f| f.axes.get_mut(i)).and_then(|a| a.series.get_mut(j));
        if let (Ok((x, y)), Some(s)) = (data, slot) {
            s.data = Data::Line { x, y };
            changed = true;
        }
    }
    if changed {
        it.plots.touch();
    }
    changed
}

#[derive(Clone, Debug)]
pub enum Data {
    /// plot / scatter (scatter = LineStyle::None + marker) / stairs / polar (converted)
    Line { x: Vec<f64>, y: Vec<f64> },
    /// bar / hist
    Bar { x: Vec<f64>, y: Vec<f64>, width: f64 },
    /// plot3 / scatter3
    Line3 { x: Vec<f64>, y: Vec<f64>, z: Vec<f64> },
    /// surf / mesh: X, Y, Z all the same size (meshgrid-style, so parametric surfaces work)
    Surface { x: Matrix, y: Matrix, z: Matrix, wire: bool },
    /// contour: X, Y, Z same size; empty levels = auto
    Contour { x: Matrix, y: Matrix, z: Matrix, levels: Vec<f64> },
    /// imagesc / heatmap: z(row, col) spanning x and y ranges
    Image { z: Matrix, x: (f64, f64), y: (f64, f64) },
    /// quiver: arrows from (x, y) to (x + u, y + v)
    Quiver { x: Vec<f64>, y: Vec<f64>, u: Vec<f64>, v: Vec<f64> },
    /// closed polygon drawn translucent with its outline: fill / area / shaded integrals;
    /// NaN points split it into several polygons
    Fill { x: Vec<f64>, y: Vec<f64> },
}

/// Plot colors and fonts; frontends load user overrides into this.
/// All plot text (title, labels, legend, notes) may carry math markup: `x^2`, `x^{10}`, `v_0`,
/// `\alpha`, `\int`, `\sqrt` (TeX-like, as Octave's default text interpreter) or Unicode math.
#[derive(Clone, Debug)]
pub struct Theme {
    pub bg: Rgb,
    pub fg: Rgb,
    pub grid: Rgb,
    pub palette: Vec<Rgb>,
    pub font: String,
    pub font_size: u32,
}

impl Theme {
    pub fn light() -> Self {
        Theme {
            bg: [255, 255, 255],
            fg: [30, 30, 30],
            grid: [220, 220, 220],
            // Octave/MATLAB default line colors
            palette: vec![[0, 114, 189], [217, 83, 25], [237, 177, 32], [126, 47, 142], [119, 172, 48], [77, 190, 238], [162, 20, 47]],
            // covers superscripts and math symbols (∫ √ ⁻ ²), unlike the usual sans default
            font: "DejaVu Sans".into(),
            font_size: 14,
        }
    }
    pub fn dark() -> Self {
        Theme {
            bg: [30, 30, 34],
            fg: [220, 220, 220],
            grid: [65, 65, 72],
            palette: vec![[77, 166, 255], [255, 120, 70], [250, 200, 60], [190, 120, 230], [140, 210, 80], [100, 220, 240], [240, 80, 110]],
            ..Theme::light()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounds_and_hit() {
        let mut f = Figure::new(1);
        f.subplot(2, 2, 3).add(Series::new(Data::Line { x: vec![0., 10.], y: vec![5., 5.] }));
        let a = &f.axes[0];
        assert_eq!(a.bounds()[0], (-0.5, 10.5));
        assert_eq!(a.bounds()[1], (4.0, 6.0));
        assert_eq!(f.axes_at(150., 150., 200., 200.), Some(0));
        assert_eq!(f.axes_at(50., 50., 200., 200.), None);
    }

    fn xs(it: &mut Interp) -> Vec<f64> {
        let Data::Line { x, .. } = &it.plots.axes().series[0].data else { panic!() };
        x.clone()
    }

    #[test]
    fn resample_follows_the_view() {
        let mut it = Interp::new();
        it.eval("fplot(@(x) x.^2, [-5 5], 'r--')").unwrap();
        let v = it.plots.version;
        assert!(!resample(&mut it) && it.plots.version == v); // fplot's samples cover its view
        // a wider xlim: sampled over just the view (ylim is automatic), style and label kept
        it.eval("xlim([-20 20])").unwrap();
        let x = xs(&mut it);
        assert_eq!((x[0], x[x.len() - 1], x.len()), (-20.0, 20.0, 1001));
        let s = &it.plots.axes().series[0];
        assert!(s.line == LineStyle::Dashed && s.label == "x²" && s.source.is_some());
        assert_eq!(it.plots.axes().bounds()[1].1, 420.0); // y follows: 400 + 5% padding
        // with a fixed ylim (as GUI gestures leave it) half a span of margin on each side
        it.eval("ylim([0 100]); xlim([0 10])").unwrap();
        assert_eq!(xs(&mut it)[0], -20.0); // still 251 samples in view: dense enough
        it.eval("xlim([0 4])").unwrap();
        let x = xs(&mut it);
        assert_eq!((x[0], x[x.len() - 1], it.plots.axes().ylim), (-2.0, 6.0, Some((0.0, 100.0))));
        // panning a little keeps the samples, panning past them resamples
        it.plots.axes().xlim = Some((1.0, 5.0));
        assert!(!resample(&mut it));
        it.plots.axes().xlim = Some((30.0, 34.0));
        assert!(resample(&mut it));
        let x = xs(&mut it);
        assert_eq!((x[0], x[x.len() - 1]), (28.0, 36.0));
        // tan: poles stay broken; unsourced, 3D and log axes are left alone
        it.eval("figure; fplot(@tan, [-1 1]); xlim([-5 5])").unwrap();
        let Data::Line { y, .. } = &it.plots.axes().series[0].data else { panic!() };
        assert!(y.iter().filter(|v| v.is_nan()).count() >= 3);
        it.eval("figure; plot(1:5); xlim([0 50]); figure; semilogx(1:5); xlim([1 100])").unwrap();
        assert!(!resample(&mut it));
        // a function that stops working keeps its samples
        it.eval("figure; f = @(x) x; fplot(f)").unwrap();
        it.plots.axes().series[0].source = Some(Source::Func(Value::from("nosuchfunction")));
        it.plots.axes().xlim = Some((100.0, 200.0));
        assert!(!resample(&mut it));
    }

    #[test]
    fn octave_code_rebuilds_the_figure() {
        let mut it = Interp::new();
        it.eval("t = linspace(0, 2, 40);").unwrap();
        it.eval("x = 3; figure(4); plot(t, t.^2, 'o'); hold on\nfplot(@sin, [0 2]); title('demo') % note\nfor k = 1:2, plot(t, t + k); end").unwrap();
        it.eval("keypoints on; y = x + 1;").unwrap();
        let f = it.plots.fig();
        assert_eq!(f.code, ["figure(4);", "plot(t, t.^2, 'o');", "hold on;", "fplot(@sin, [0 2]);", "title('demo');", "for k = 1:2, plot(t, t + k); end;", "keypoints on;"]);
        assert_eq!(f.octave(), f.code.join("\n")); // nothing moved since
        // the view follows gestures (limits rounded to 4 digits of their span), regenerated each time
        f.axes[0].xlim = Some((-0.123456, 2.5));
        f.axes[0].ylim = Some((-1.0, 4.0000001));
        let code = f.octave();
        assert!(code.ends_with("title('demo');\nfor k = 1:2, plot(t, t + k); end;\nkeypoints on;\nxlim([-0.123 2.5]); ylim([-1 4]);"), "{code}");
        assert_eq!(f.octave(), code);
        // run in a fresh interpreter with the same variables: same series, labels, view
        let mut fresh = Interp::new();
        fresh.eval("t = linspace(0, 2, 40);").unwrap();
        fresh.eval(&code).unwrap();
        let (a, b) = (&it.plots.fig().axes[0], &fresh.plots.fig().axes[0]);
        assert_eq!((b.series.len(), b.title.as_str(), b.keypoints, b.xlim, b.ylim), (a.series.len(), "demo", true, Some((-0.123, 2.5)), Some((-1.0, 4.0))));
        for (p, q) in a.series.iter().zip(&b.series) {
            let (Data::Line { y: y1, .. }, Data::Line { y: y2, .. }) = (&p.data, &q.data) else { panic!() };
            assert!(p.marker == q.marker && y1.len() == y2.len());
        }
        assert_eq!(fresh.plots.fig().id, 4);
        // what isn't logged: switching figures, clf and close; nested evals log once
        it.eval("figure(9); plot(1:3); figure(4); eval('grid on')").unwrap();
        assert_eq!(it.plots.fig().code.last().unwrap(), "eval('grid on');");
        assert_eq!(it.plots.figures[1].code, ["figure(9);", "plot(1:3);"]);
        it.eval("close(9); clf").unwrap();
        assert!(it.plots.fig().code.is_empty() && it.plots.fig().octave().is_empty());
        it.eval("plot(1:3); clf; plot(2:4)").unwrap();
        assert_eq!(it.plots.fig().code, ["plot(2:4);"]);
        // a failing statement isn't logged
        assert!(it.eval("plot(1:3); title(nosuch)").is_err());
        assert_eq!(it.plots.fig().code, ["plot(2:4);", "plot(1:3);"]);
    }

    #[test]
    fn octave_code_with_subplots_and_3d() {
        let mut it = Interp::new();
        it.eval("subplot(1, 2, 1); plot(1:4); subplot(1, 2, 2); surf(peaks(5)); subplot(1, 2, 1)").unwrap();
        let f = it.plots.fig();
        f.axes[1].view.yaw = 0.5;
        f.axes[1].view.pitch = 0.25;
        f.axes[0].xlim = Some((0.0, 2.0));
        let code = f.octave();
        let tail: Vec<&str> = code.lines().rev().take(3).collect();
        assert_eq!(tail, ["subplot(1, 2, 1);", "subplot(1, 2, 2); view(28.6, 14.3);", "subplot(1, 2, 1); xlim([0 2]);"], "{code}");
        assert_eq!(f.view_code(0), "subplot(1, 2, 1); xlim([0 2]);");
        let mut fresh = Interp::new();
        fresh.eval(&code).unwrap();
        let g = fresh.plots.fig();
        assert_eq!((g.axes[0].xlim, g.current, (g.axes[1].view.yaw.to_degrees() * 10.0).round()), (Some((0.0, 2.0)), 0, 286.0));
        // a GUI reset back to automatic limits
        f.axes[0].xlim = None;
        it.eval("xlim([1 3])").unwrap();
        it.plots.fig().axes[0].xlim = None;
        assert!(it.plots.fig().octave().ends_with("xlim('auto');"));
    }
}
