//! Figure model. CONTRACT shared by plot builtins, renderer, GUI, TUI, CLI; change only additively.
pub mod builtins;
mod calculus;
pub mod cursor;
mod mathtext;
pub mod render;

use crate::value::Matrix;

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
}

impl Figure {
    pub fn new(id: usize) -> Self {
        Figure { id, rows: 1, cols: 1, axes: vec![], current: 0, colormap: "viridis".into() }
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
}

impl Series {
    pub fn new(data: Data) -> Self {
        Series { data, label: String::new(), color: None, line: LineStyle::Solid, marker: Marker::None, width: 1.5 }
    }
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
}
