//! Figure -> pixels / files / terminal. One generic `draw` over any plotters backend.
use std::fmt::Write as _;
use std::path::Path;

use plotters::coord::Shift;
use plotters::coord::cartesian::Cartesian3d;
use plotters::coord::ranged3d::ProjectionMatrixBuilder;
use plotters::coord::types::RangedCoordf64;
use plotters::prelude::*;
use plotters::style::text_anchor::{HPos, Pos, VPos};

use super::analysis::{self, Symmetry};
use super::{Axes, Data, Figure, LineStyle, Marker, Rgb, Series, Theme, cursor, mathtext};
use crate::value::Matrix;

type Res<T = ()> = Result<T, Box<dyn std::error::Error>>;
type Pt = (f64, f64);

/// Where one axes landed in a rendered image, in output pixels: what `plot::cursor` needs to map
/// data <-> screen.
#[derive(Clone)]
pub struct Frame {
    /// index into `Figure::axes`
    pub axes: usize,
    /// [x0, y0, x1, y1]: 2D the plot area, 3D the axes cell
    pub rect: [f64; 4],
    /// x/y/z ranges as drawn (log axes: the padded log range)
    pub bounds: [(f64, f64); 3],
    pub logx: bool,
    pub logy: bool,
    /// 3D: plotters' projection (device pixels) and device pixels per frame pixel; None = 2D
    pub(crate) cube: Option<(Cartesian3d<RangedCoordf64, RangedCoordf64, RangedCoordf64>, f64)>,
}

impl Frame {
    /// The same frame in units `k` times larger, e.g. widget pixels of a 2x HiDPI render.
    pub fn scaled(mut self, k: f64) -> Frame {
        self.rect = self.rect.map(|v| v / k);
        if let Some((_, u)) = &mut self.cube {
            *u *= k;
        }
        self
    }
}

/// RGB8 pixels, row-major, len = w * h * 3.
pub fn render_rgb(fig: &Figure, theme: &Theme, w: u32, h: u32) -> Result<Vec<u8>, String> {
    render_frames(fig, theme, w, h).map(|(px, _)| px)
}

/// RGB8 pixels plus where each axes landed (for cursors and gestures).
pub fn render_frames(fig: &Figure, theme: &Theme, w: u32, h: u32) -> Result<(Vec<u8>, Vec<Frame>), String> {
    // big (HiDPI) GUI buffers skip supersampling to stay fast while dragging; their pixels are small anyway
    // ponytail: the 2x box filter is ~16 ms per 800x600 frame in unoptimized dev builds (~2 ms optimized);
    // a dev-profile opt-level for this crate fixes that if GUI drags feel slow
    raster(fig, theme, w, h, if w as u64 * h as u64 <= 1_500_000 { 2 } else { 1 })
}

/// Render at `ss`x and box-filter down: 2x supersampling is the anti-aliasing.
fn raster(fig: &Figure, theme: &Theme, w: u32, h: u32, ss: u32) -> Result<(Vec<u8>, Vec<Frame>), String> {
    let (sw, sh) = (w * ss, h * ss);
    let mut buf = vec![0u8; sw as usize * sh as usize * 3];
    let mut frames = vec![];
    if w > 0 && h > 0 {
        let root = BitMapBackend::with_buffer(&mut buf, (sw, sh)).into_drawing_area();
        frames = draw(&root, fig, theme, ss as f64).map_err(|e| e.to_string())?;
        root.present().map_err(|e| e.to_string())?;
    }
    Ok((if ss == 1 { buf } else { halve(&buf, w as usize, h as usize) }, frames))
}

/// 2x2 box filter: (2w x 2h) RGB -> (w x h).
fn halve(src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let row = w * 6;
    let mut out = vec![0u8; w * h * 3];
    for y in 0..h {
        let (a, b) = (&src[2 * y * row..(2 * y + 1) * row], &src[(2 * y + 1) * row..(2 * y + 2) * row]);
        let o = &mut out[y * w * 3..(y + 1) * w * 3];
        for x in 0..w * 3 {
            let i = x / 3 * 6 + x % 3;
            o[x] = ((a[i] as u16 + a[i + 3] as u16 + b[i] as u16 + b[i + 3] as u16 + 2) / 4) as u8;
        }
    }
    out
}

/// Save by extension: .png or .svg.
pub fn save(fig: &Figure, theme: &Theme, path: &Path, w: u32, h: u32) -> Result<(), String> {
    let (w, h) = (w.max(1), h.max(1));
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let r = match ext.as_str() {
        "png" => {
            let (px, _) = raster(fig, theme, w, h, if w as u64 * h as u64 <= 16_000_000 { 2 } else { 1 })?;
            let root = BitMapBackend::new(path, (w, h)).into_drawing_area();
            let img = BitMapElement::with_owned_buffer((0, 0), (w, h), px).ok_or("bitmap size")?;
            root.draw(&img).map_err(|e| e.into()).and_then(|_| Ok(root.present()?))
        }
        "svg" => {
            let root = SVGBackend::new(path, (w, h)).into_drawing_area();
            draw(&root, fig, theme, 1.0).and_then(|_| Ok(root.present()?))
        }
        _ => return Err(format!("can't save '.{ext}' files, use .png or .svg")),
    };
    r.map_err(|e| e.to_string())
}

/// Truecolor half-block (▀) text for terminals, `cols` x `rows` character cells.
pub fn ansi(fig: &Figure, theme: &Theme, cols: u16, rows: u16) -> String {
    let (w, h) = (cols as usize, rows as usize * 2);
    let Ok(px) = render_rgb(fig, theme, w as u32, h as u32) else { return String::new() };
    let at = |x: usize, y: usize| &px[(y * w + x) * 3..(y * w + x) * 3 + 3];
    let mut s = String::new();
    for r in 0..rows as usize {
        let mut last = None;
        for c in 0..w {
            let (up, lo) = (at(c, 2 * r), at(c, 2 * r + 1));
            if last != Some((up, lo)) {
                let _ = write!(s, "\x1b[38;2;{};{};{}m\x1b[48;2;{};{};{}m", up[0], up[1], up[2], lo[0], lo[1], lo[2]);
                last = Some((up, lo));
            }
            s.push('▀');
        }
        s.push_str("\x1b[0m\n");
    }
    s
}

/// Colormap lookup, t in 0..1 (NaN -> middle). Unknown names fall back to viridis.
pub fn cmap(name: &str, t: f64) -> Rgb {
    let tab = colormap_table(name).unwrap_or(VIRIDIS);
    let t = if t.is_nan() { 0.5 } else { t.clamp(0.0, 1.0) } * (tab.len() - 1) as f64;
    let (i, f) = (t.floor() as usize, t.fract());
    lerp(tab[i], tab[(i + 1).min(tab.len() - 1)], f)
}

/// Control points of a named colormap, None if unknown.
pub fn colormap_table(name: &str) -> Option<&'static [Rgb]> {
    Some(match name {
        "viridis" | "default" => VIRIDIS,
        "jet" => &[[0, 0, 128], [0, 0, 255], [0, 128, 255], [0, 255, 255], [128, 255, 128], [255, 255, 0], [255, 128, 0], [255, 0, 0], [128, 0, 0]],
        "hot" => &[[0, 0, 0], [85, 0, 0], [170, 0, 0], [255, 0, 0], [255, 85, 0], [255, 170, 0], [255, 255, 0], [255, 255, 128], [255, 255, 255]],
        "gray" | "grey" => &[[0, 0, 0], [255, 255, 255]],
        "cool" => &[[0, 255, 255], [255, 0, 255]],
        "parula" => &[[53, 42, 135], [15, 92, 221], [18, 125, 216], [7, 156, 207], [21, 177, 180], [89, 189, 140], [165, 190, 107], [225, 185, 82], [252, 206, 46], [249, 251, 14]],
        "turbo" => &[[48, 18, 59], [70, 107, 227], [40, 187, 236], [49, 242, 153], [162, 252, 60], [237, 208, 58], [251, 128, 34], [208, 48, 4], [122, 4, 3]],
        _ => return None,
    })
}

const VIRIDIS: &[Rgb] = &[[68, 1, 84], [72, 40, 120], [62, 74, 137], [49, 104, 142], [38, 130, 142], [31, 158, 137], [53, 183, 121], [109, 205, 89], [180, 222, 44], [253, 231, 37]];

/// Draw `fig` on `root`; `u` = device pixels per output pixel (supersampling).
fn draw<DB: DrawingBackend>(root: &DrawingArea<DB, Shift>, fig: &Figure, theme: &Theme, u: f64) -> Res<Vec<Frame>>
where
    DB::ErrorType: 'static,
{
    root.fill(&rgb(theme.bg))?;
    let (w, h) = root.dim_in_pixel();
    let fs = theme.font_size.max(4) as f64 * u;
    let mut frames = vec![];
    for (i, a) in fig.axes.iter().enumerate().filter(|(_, a)| a.cell < fig.rows * fig.cols) {
        let (x, y, cw, ch) = fig.cell_rect(a.cell, w as f64, h as f64);
        // tiny = terminal-sized: no text, thin lines
        let tiny = ch < fs * 7.0 || cw < fs * 9.0;
        let k = if tiny { 0.6 * u } else { fs / 14.0 };
        let p = Painter { root, theme, cmap: &fig.colormap, fs, k, u, tiny, clim: clim(a) };
        let cell = Rect::new((x, y), (x + cw, y + ch)).round();
        if a.is_3d() { p.axes3d(i, a, cell, &mut frames)? } else { p.axes2d(i, a, cell, &mut frames)? }
    }
    Ok(frames)
}

/// Drawing context for one axes; all coordinates are absolute pixels on `root`.
struct Painter<'a, DB: DrawingBackend> {
    root: &'a DrawingArea<DB, Shift>,
    theme: &'a Theme,
    cmap: &'a str,
    /// font size in px
    fs: f64,
    /// line/marker scale
    k: f64,
    /// device pixels per output pixel; hairlines are this wide and snapped to it
    u: f64,
    tiny: bool,
    /// color range of colormapped data
    clim: (f64, f64),
}

impl<'a, DB: DrawingBackend> Painter<'a, DB>
where
    DB::ErrorType: 'static,
{
    fn font(&self, scale: f64, c: Rgb) -> TextStyle<'a> {
        (self.theme.font.as_str(), self.fs * scale).into_font().color(&rgb(c))
    }
    fn title_font(&self) -> TextStyle<'a> {
        (self.theme.font.as_str(), self.fs * 1.15, FontStyle::Bold).into_font().color(&rgb(self.theme.fg))
    }
    /// Width of `s`, math markup included.
    fn text_w(&self, s: &str, st: &TextStyle) -> f64 {
        self.layout(s, st).1
    }
    /// Math runs of `s` with their x offsets, and the total width.
    fn layout(&self, s: &str, st: &TextStyle) -> (Vec<(mathtext::Run, f64)>, f64) {
        let (mut out, mut end, mut prev) = (vec![], 0.0f64, (0.0, 0.0));
        for mut r in mathtext::parse(s) {
            let w = self.run_w(&r, st);
            let x = if r.stack { prev.0 } else { end };
            end = if r.stack { end.max(x + w) } else { x + w };
            prev = (x, x + w);
            // leading spaces after the first run become an offset: SVG viewers strip them
            let mut lead = 0.0;
            if r.text.starts_with(' ') && !out.is_empty() {
                r.text = r.text.trim_start().into();
                lead = w - self.run_w(&r, st);
            }
            out.push((r, x + lead));
        }
        (out, end)
    }
    fn run_style<'b>(&self, r: &mathtext::Run, st: &TextStyle<'b>) -> TextStyle<'b> {
        TextStyle { font: st.font.resize(st.font.get_size() * r.scale), ..st.clone() }
    }
    fn run_w(&self, r: &mathtext::Run, st: &TextStyle) -> f64 {
        let st = self.run_style(r, st);
        self.root.estimate_text_size(&r.text, &st).map_or(r.text.chars().count() as f64 * st.font.get_size() * 0.55, |(w, _)| w as f64)
    }
    /// Text with math markup (`x^2`, `v_0`, `\alpha`), anchored on its main line like plain text.
    /// ponytail: text errors (no usable font) are dropped so the plot still renders.
    fn text(&self, s: &str, (x, y): Pt, st: &TextStyle, h: HPos, v: VPos) {
        let (runs, w) = self.layout(s, st);
        if let [(r, _)] = &runs[..]
            && r.scale == 1.0
            && r.shift == 0.0
        {
            let _ = self.root.draw_text(&r.text, &st.pos(Pos::new(h, v)), ipt((x, y)));
            return;
        }
        // runs go top-anchored at their own baseline; plotters' line box (bitmap and SVG alike) is
        // size / 1.24 tall with the baseline 0.76 of it down
        let em = st.font.get_size() / 1.24;
        let x0 = match h {
            HPos::Left => 0.0,
            HPos::Center => -w / 2.0,
            HPos::Right => -w,
        };
        let base = em * match v {
            VPos::Top => 0.76,
            VPos::Center => 0.26,
            VPos::Bottom => -0.24,
        };
        let rot = st.font.get_transform();
        for (r, rx) in &runs {
            let top = base - (r.shift + 0.76 * r.scale) * em;
            let (dx, dy) = rot.transform((x0 + rx).round() as i32, top.round() as i32);
            let rs = self.run_style(r, st).pos(Pos::new(HPos::Left, VPos::Top));
            let _ = self.root.draw_text(&r.text, &rs, (x.round() as i32 + dx, y.round() as i32 + dy));
        }
    }
    fn mix(&self, t: f64) -> Rgb {
        lerp(self.theme.bg, self.theme.fg, t)
    }
    fn cm(&self, v: f64) -> Rgb {
        cmap(self.cmap, (v - self.clim.0) / (self.clim.1 - self.clim.0))
    }
    /// Series color: explicit or palette; pure black/white flipped to fg when invisible on bg.
    /// Explicit light-palette colors (stem/errorbar parts pin them) follow the theme's palette.
    fn color(&self, i: usize, s: &Series) -> Rgb {
        let t = self.theme;
        let pal = |j: usize| if t.palette.is_empty() { t.fg } else { t.palette[j % t.palette.len()] };
        let c = match s.color {
            None => pal(i),
            Some(c) => Theme::light().palette.iter().position(|p| *p == c).map_or(c, pal),
        };
        let dark = t.bg.iter().map(|&v| v as u32).sum::<u32>() < 384;
        if (dark && c == [0, 0, 0]) || (!dark && c == [255, 255, 255]) { t.fg } else { c }
    }
    /// Line width in points -> device px.
    fn lw(&self, width: f64) -> u32 {
        (width * 1.33 * self.k).round().max(1.0) as u32
    }
    fn path(&self, pts: &[Pt], c: Rgb, a: f64, w: u32) -> Res {
        let mut v: Vec<(i32, i32)> = pts.iter().map(|&p| ipt(p)).collect();
        v.dedup(); // ponytail: decimation = drop repeated pixels; per-column min/max if 1e7-point series matter
        // plotters' thick-line joins spike at sharp turns, so split the path there
        let mut start = 0;
        for k in 1..v.len().saturating_sub(1) {
            let (d1, d2) = ((v[k].0 - v[k - 1].0, v[k].1 - v[k - 1].1), (v[k + 1].0 - v[k].0, v[k + 1].1 - v[k].1));
            if w > 1 && d1.0 * d2.0 + d1.1 * d2.1 <= 0 {
                self.root.draw(&PathElement::new(v[start..=k].to_vec(), stroke(c, a, w)))?;
                start = k;
            }
        }
        if v.len() - start > 1 {
            self.root.draw(&PathElement::new(v[start..].to_vec(), stroke(c, a, w)))?;
        }
        Ok(())
    }
    fn fill(&self, r: Rect, c: Rgb, a: f64) -> Res {
        let (x0, y0, x1, y1) = (r.x0.round() as i32, r.y0.round() as i32, r.x1.round() as i32, r.y1.round() as i32);
        if x1 > x0 && y1 > y0 {
            self.root.draw(&Rectangle::new([(x0, y0), (x1, y1)], fill(c, a)))?; // bottom-right is exclusive
        }
        Ok(())
    }
    fn snap(&self, v: f64) -> f64 {
        (v / self.u).floor() * self.u
    }
    /// Round to whole output pixels so filled shapes stay crisp after supersampling.
    fn whole(&self, v: f64) -> f64 {
        (v / self.u).round() * self.u
    }
    /// Crisp one-output-pixel horizontal / vertical lines.
    fn hline(&self, x0: f64, x1: f64, y: f64, c: Rgb) -> Res {
        let y = self.snap(y);
        self.fill(Rect::new((self.snap(x0), y), (self.snap(x1) + self.u, y + self.u)), c, 1.0)
    }
    fn vline(&self, x: f64, y0: f64, y1: f64, c: Rgb) -> Res {
        let x = self.snap(x);
        self.fill(Rect::new((x, self.snap(y0)), (x + self.u, self.snap(y1) + self.u)), c, 1.0)
    }
    fn outline(&self, r: Rect, c: Rgb) -> Res {
        let (x1, y1) = (r.x1 - self.u, r.y1 - self.u);
        self.hline(r.x0, x1, r.y0, c)?;
        self.hline(r.x0, x1, y1, c)?;
        self.vline(r.x0, r.y0, y1, c)?;
        self.vline(x1, r.y0, y1, c)
    }
    fn hair(&self) -> u32 {
        self.u as u32
    }
    /// Polyline with a line style's dash pattern.
    fn stroke(&self, pts: &[Pt], c: Rgb, w: u32, ls: LineStyle) -> Res {
        let u = (w as f64).max(1.5);
        let pat: &[f64] = match ls {
            LineStyle::None => return Ok(()),
            LineStyle::Solid => return self.path(pts, c, 1.0, w),
            LineStyle::Dashed => &[3.7 * u, 1.9 * u],
            LineStyle::Dotted => &[1.0 * u, 1.65 * u],
            LineStyle::DashDot => &[6.4 * u, 1.6 * u, 1.0 * u, 1.6 * u],
        };
        for d in dash(pts, pat) {
            self.path(&d, c, 1.0, w)?;
        }
        Ok(())
    }
    fn marker(&self, (x, y): Pt, m: Marker, c: Rgb, width: f64) -> Res {
        let r = (1.5 + 1.5 * width) * self.k;
        let st = stroke(c, 1.0, (width * self.k * 0.7).round().max(1.0) as u32);
        let p = |dx: f64, dy: f64| ((x + dx * r).round() as i32, (y + dy * r).round() as i32);
        let lines = |segs: &[[Pt; 2]]| -> Res {
            for [a, b] in segs {
                self.root.draw(&PathElement::new(vec![p(a.0, a.1), p(b.0, b.1)], st))?;
            }
            Ok(())
        };
        let closed = |v: &[Pt]| PathElement::new(v.iter().chain(&v[..1]).map(|q| p(q.0, q.1)).collect::<Vec<_>>(), st);
        match m {
            Marker::None => {}
            Marker::Point => self.root.draw(&Circle::new(p(0.0, 0.0), (2.0 * width * self.k).max(1.5), fill(c, 1.0)))?,
            Marker::Circle => self.root.draw(&Circle::new(p(0.0, 0.0), r, st))?,
            Marker::Square => self.root.draw(&closed(&[(-0.8, -0.8), (0.8, -0.8), (0.8, 0.8), (-0.8, 0.8)]))?,
            Marker::Diamond => self.root.draw(&closed(&[(0.0, -1.1), (1.0, 0.0), (0.0, 1.1), (-1.0, 0.0)]))?,
            Marker::Triangle => self.root.draw(&closed(&[(0.0, -1.1), (0.95, 0.65), (-0.95, 0.65)]))?,
            Marker::Plus => lines(&[[(-1.0, 0.0), (1.0, 0.0)], [(0.0, -1.0), (0.0, 1.0)]])?,
            Marker::Cross => lines(&[[(-0.75, -0.75), (0.75, 0.75)], [(-0.75, 0.75), (0.75, -0.75)]])?,
            Marker::Star => lines(&[[(-1.0, 0.0), (1.0, 0.0)], [(0.0, -1.0), (0.0, 1.0)], [(-0.7, -0.7), (0.7, 0.7)], [(-0.7, 0.7), (0.7, -0.7)]])?,
        }
        Ok(())
    }

    fn axes2d(&self, i: usize, a: &Axes, cell: Rect, frames: &mut Vec<Frame>) -> Res {
        let fs = self.fs;
        let [bx, by, bz] = a.bounds();
        let bx = if a.logx { log_range(bx, a.xlim.is_some(), pos_extent(a, 0)) } else { bx };
        let by = if a.logy { log_range(by, a.ylim.is_some(), pos_extent(a, 1)) } else { by };
        let tick_font = self.font(0.88, self.mix(0.9));
        let tl = (0.35 * fs).round();
        let (plot, xt, yt) = if self.tiny {
            let r = Rect { x0: cell.x0 + self.u, y0: cell.y0 + self.u, x1: cell.x1 - self.u, y1: cell.y1 - self.u };
            (r, ticks(bx, a.logx, 4), ticks(by, a.logy, 3))
        } else {
            let top = cell.y0 + fs * if a.title.is_empty() { 0.9 } else { 2.3 };
            let bottom = cell.y1 - fs * if a.xlabel.is_empty() { 2.0 } else { 3.4 } - tl;
            let yt = ticks(by, a.logy, ((bottom - top) / (fs * 2.6)).max(2.0) as usize);
            let yw = yt.0.iter().map(|(_, s)| self.text_w(s, &tick_font)).fold(0.0, f64::max);
            let left = cell.x0 + fs * if a.ylabel.is_empty() { 0.8 } else { 2.2 } + yw + tl + 0.35 * fs;
            let right = cell.x1 - fs * 1.4;
            let mut n = ((right - left) / (fs * 3.5)).max(2.0) as usize;
            let xt = loop {
                let t = ticks(bx, a.logx, n);
                let need: f64 = t.0.iter().map(|(_, s)| self.text_w(s, &tick_font) + fs * 1.2).sum();
                if n <= 2 || need <= right - left {
                    break t;
                }
                n -= 1;
            };
            (Rect { x0: self.snap(left), y0: self.snap(top), x1: self.snap(right), y1: self.snap(bottom) }, xt, yt)
        };
        if plot.w() < 4.0 || plot.h() < 4.0 {
            return Ok(());
        }
        let sx = Scale::new(bx, a.logx, plot.x0, plot.x1);
        let sy = Scale::new(by, a.logy, plot.y1, plot.y0);
        let rect = [plot.x0, plot.y0, plot.x1, plot.y1].map(|v| v / self.u);
        frames.push(Frame { axes: i, rect, bounds: [bx, by, bz], logx: a.logx, logy: a.logy, cube: None });
        if a.grid {
            let (major, minor) = (self.theme.grid, lerp(self.theme.bg, self.theme.grid, 0.55));
            for (t, c) in xt.1.iter().map(|&v| (v, minor)).chain(xt.0.iter().map(|(v, _)| (*v, major))) {
                self.vline(sx.px(t), plot.y0, plot.y1 - self.u, c)?;
            }
            for (t, c) in yt.1.iter().map(|&v| (v, minor)).chain(yt.0.iter().map(|(v, _)| (*v, major))) {
                self.hline(plot.x0, plot.x1 - self.u, sy.px(t), c)?;
            }
        }
        let mut occupied = vec![];
        for (i, s) in a.series.iter().enumerate() {
            self.series2d(i, s, &sx, &sy, plot, &mut occupied)?;
        }
        if a.keypoints {
            self.keypoints(a, &sx, &sy, plot, &mut occupied)?;
        }
        self.notes(a, plot, |p| (sx.px(p[0]), sy.px(p[1])), &mut occupied)?;
        let axis = self.mix(0.6);
        self.outline(plot, axis)?;
        self.pins(a, plot, |p| (sx.px(p[0]), sy.px(p[1])), &mut occupied)?;
        if self.tiny {
            return Ok(());
        }
        for (v, s) in &xt.0 {
            let x = sx.px(*v);
            self.vline(x, plot.y1, plot.y1 + tl, axis)?;
            self.text(s, (self.snap(x) + self.u / 2.0, plot.y1 + tl + 0.3 * fs), &tick_font, HPos::Center, VPos::Top);
        }
        for v in &xt.1 {
            self.vline(sx.px(*v), plot.y1, plot.y1 + tl * 0.55, axis)?;
        }
        for (v, s) in &yt.0 {
            let y = sy.px(*v);
            self.hline(plot.x0 - tl, plot.x0, y, axis)?;
            self.text(s, (plot.x0 - tl - 0.35 * fs, self.snap(y) + self.u / 2.0), &tick_font, HPos::Right, VPos::Center);
        }
        for v in &yt.1 {
            self.hline(plot.x0 - tl * 0.55, plot.x0, sy.px(*v), axis)?;
        }
        let mid = ((plot.x0 + plot.x1) / 2.0, (plot.y0 + plot.y1) / 2.0);
        if !a.title.is_empty() {
            self.text(&a.title, (mid.0, cell.y0 + 0.7 * fs), &self.title_font(), HPos::Center, VPos::Top);
        }
        if !a.xlabel.is_empty() {
            self.text(&a.xlabel, (mid.0, cell.y1 - 0.5 * fs), &self.font(1.0, self.theme.fg), HPos::Center, VPos::Bottom);
        }
        if !a.ylabel.is_empty() {
            let st = self.font(1.0, self.theme.fg).transform(FontTransform::Rotate270);
            self.text(&a.ylabel, (cell.x0 + 1.1 * fs, mid.1), &st, HPos::Center, VPos::Center);
        }
        if a.legend {
            self.legend(a, plot, &occupied)?;
        }
        Ok(())
    }

    fn series2d(&self, i: usize, s: &Series, sx: &Scale, sy: &Scale, plot: Rect, occ: &mut Vec<Pt>) -> Res {
        let c = self.color(i, s);
        let lw = self.lw(s.width);
        match &s.data {
            Data::Line { x, y } => {
                let pts: Vec<Pt> = x.iter().zip(y).map(|(&x, &y)| (sx.px(x), sy.px(y))).collect();
                for run in clip_polyline(&pts, &plot) {
                    self.stroke(&run, c, lw, s.line)?;
                }
                if s.marker != Marker::None {
                    for &p in pts.iter().filter(|&&p| plot.contains(p)) {
                        self.marker(p, s.marker, c, s.width)?;
                    }
                }
                occ.extend(pts.iter().step_by(pts.len() / 400 + 1));
            }
            Data::Fill { x, y } => {
                let pts: Vec<Pt> = x.iter().zip(y).map(|(&x, &y)| (sx.px(x), sy.px(y))).collect();
                for poly in pts.split(|p| !(p.0.is_finite() && p.1.is_finite())).filter(|p| p.len() > 1) {
                    let inside = clip_polygon(poly, &plot);
                    if inside.len() > 2 {
                        self.root.draw(&Polygon::new(inside.into_iter().map(ipt).collect::<Vec<_>>(), fill(c, 0.3)))?;
                    }
                    let ring: Vec<Pt> = poly.iter().chain(&poly[..1]).copied().collect();
                    for run in clip_polyline(&ring, &plot) {
                        self.stroke(&run, c, lw, s.line)?;
                    }
                }
                occ.extend(pts.iter().step_by(pts.len() / 400 + 1));
            }
            Data::Bar { x, y, width } => {
                let base = if sy.log { plot.y1 } else { sy.px(0.0) };
                for (&x, &y) in x.iter().zip(y) {
                    let g = |v: f64| self.whole(v);
                    let r = Rect::new((g(sx.px(x - width / 2.0)), g(sy.px(y))), (g(sx.px(x + width / 2.0)), g(base)));
                    if !(r.x0.is_finite() && r.x1.is_finite() && sy.px(y).is_finite()) {
                        continue;
                    }
                    if let Some(r) = r.clip(&plot) {
                        self.fill(r, c, 1.0)?;
                        if r.w() > 5.0 * self.u {
                            self.outline(r, self.theme.bg)?;
                        }
                        occ.extend([(r.x0, r.y0), (r.x1, r.y0), ((r.x0 + r.x1) / 2.0, r.y0)]);
                    }
                }
            }
            Data::Image { z, x, y } => {
                // ponytail: row 1 on top (imagesc) but y ticks still ascend; Axes has no reversed-y flag
                let (dx, dy) = ((x.1 - x.0) / z.cols as f64, (y.1 - y.0) / z.rows as f64);
                let xe: Vec<f64> = (0..=z.cols).map(|j| self.whole(sx.px(x.0 + j as f64 * dx))).collect();
                let ye: Vec<f64> = (0..=z.rows).map(|i| self.whole(sy.px(y.1 - i as f64 * dy))).collect();
                for i in 0..z.rows {
                    for j in 0..z.cols {
                        let v = z.get(i, j);
                        if let Some(r) = Rect::new((xe[j], ye[i]), (xe[j + 1], ye[i + 1])).clip(&plot).filter(|_| v.is_finite()) {
                            self.fill(r, self.cm(v), 1.0)?;
                        }
                    }
                }
            }
            Data::Contour { x, y, z, levels } => {
                let lv = if levels.is_empty() { auto_levels(z) } else { levels.clone() };
                for &l in &lv {
                    let lc = if s.color.is_some() || lv.len() == 1 { c } else { self.cm(l) };
                    for [p, q] in contour_segments(x, y, z, l) {
                        for run in clip_polyline(&[(sx.px(p.0), sy.px(p.1)), (sx.px(q.0), sy.px(q.1))], &plot) {
                            self.path(&run, lc, 1.0, lw)?;
                        }
                    }
                }
            }
            Data::Quiver { x, y, u, v } => {
                for k in 0..x.len().min(y.len()).min(u.len()).min(v.len()) {
                    let (p, q) = ((sx.px(x[k]), sy.px(y[k])), (sx.px(x[k] + u[k]), sy.px(y[k] + v[k])));
                    let len = (q.0 - p.0).hypot(q.1 - p.1);
                    // ponytail: arrows leaving the plot box are skipped, not clipped
                    if !(len > 0.5 && plot.contains(p) && plot.contains(q)) {
                        continue;
                    }
                    let (ux, uy) = ((q.0 - p.0) / len, (q.1 - p.1) / len);
                    let hl = (len * 0.3).min(9.0 * self.k).max(2.0);
                    let hw = hl * 0.45;
                    self.path(&[p, (q.0 - ux * hl * 0.8, q.1 - uy * hl * 0.8)], c, 1.0, lw)?;
                    let head = [q, (q.0 - ux * hl - uy * hw, q.1 - uy * hl + ux * hw), (q.0 - ux * hl + uy * hw, q.1 - uy * hl - ux * hw)];
                    self.root.draw(&Polygon::new(head.map(ipt).to_vec(), fill(c, 1.0)))?;
                    occ.extend([p, q]);
                }
            }
            Data::Line3 { .. } | Data::Surface { .. } => {} // 3D axes only
        }
        Ok(())
    }

    /// Key points of the line series (`keypoints on`), in each series' color: a marker per point
    /// (shape by kind, the vertex emphasized; one per spot), labels like "max (1.2, 3.4)" that dodge
    /// each other, the markers and the plot edges (a label that fits nowhere is dropped, its marker
    /// stays), a dashed mirror axis "x = h", and corner notes for even / odd / point symmetry.
    fn keypoints(&self, a: &Axes, sx: &Scale, sy: &Scale, plot: Rect, occ: &mut Vec<Pt>) -> Res {
        let num = |v: f64| cursor::num(v).replace('-', "\u{2212}");
        // (pixel, kind, label, color, label below the point first)
        let mut pts: Vec<(Pt, Kp, String, Rgb, bool)> = vec![];
        let (mut axes, mut notes) = (vec![], vec![]);
        for (i, s) in a.series.iter().enumerate() {
            let Data::Line { x, y } = &s.data else { continue };
            let Some(k) = analysis::analyze(x, y) else { continue };
            let c = self.color(i, s);
            let low = k.vertex.is_some_and(|v| k.min.contains(&v));
            let kinds = [(Kp::Vertex, "vertex", k.vertex.as_slice(), low), (Kp::Max, "max", &k.max, false), (Kp::Min, "min", &k.min, true)];
            let more = [(Kp::Root, "root", &k.roots[..], true), (Kp::YInt, "y-int", k.y_int.as_slice(), false), (Kp::Infl, "inflection", &k.inflection, false)];
            for (kind, name, ps, below) in kinds.into_iter().chain(more) {
                for p in ps {
                    let q = (sx.px(p[0]), sy.px(p[1]));
                    // one marker per spot: the vertex beats its min, a root at the origin its intercept
                    if plot.contains(q) && !pts.iter().any(|(o, ..)| (o.0 - q.0).abs() < 2.0 * self.k && (o.1 - q.1).abs() < 2.0 * self.k) {
                        pts.push((q, kind, format!("{name} ({}, {})", num(p[0]), num(p[1])), c, below));
                    }
                }
            }
            match k.symmetry {
                Symmetry::Axis(h) if plot.contains((sx.px(h), plot.y0)) => axes.push((sx.px(h), format!("x = {}", num(h)), c)),
                Symmetry::Even => notes.push(("even: f(\u{2212}x) = f(x)".to_string(), c)),
                Symmetry::Odd => notes.push(("odd: f(\u{2212}x) = \u{2212}f(x)".to_string(), c)),
                Symmetry::Point(h, v) => notes.push((format!("symmetric about ({}, {})", num(h), num(v)), c)),
                _ => {}
            }
        }
        // extrema of every series first; ponytail: fixed caps (60 markers, 12 labels), so a dense
        // curve (sin over 100 periods) shows its first ones
        pts.sort_by_key(|p| p.1 as u8);
        pts.truncate(60);
        for &(x, _, c) in &axes {
            self.stroke(&[(x, plot.y0), (x, plot.y1)], c, self.hair(), LineStyle::Dashed)?;
        }
        for (q, kind, _, c, _) in &pts {
            self.kp_marker(*q, *kind, *c)?;
        }
        if self.tiny {
            return Ok(());
        }
        let font = self.font(0.8, self.theme.fg);
        let (pad, lh, gap) = (0.3 * self.fs, 1.05 * self.fs, 0.6 * self.fs);
        let area = Rect { x0: plot.x0 + self.u, y0: plot.y0 + self.u, x1: plot.x1 - self.u, y1: plot.y1 - self.u };
        let r = 6.0 * self.k;
        let mut taken: Vec<Rect> = pts.iter().map(|((x, y), ..)| Rect::new((x - r, y - r), (x + r, y + r))).collect();
        // corner notes top left, stacked
        let mut y = area.y0 + 0.3 * self.fs;
        for (text, c) in &notes {
            let w = self.text_w(text, &font) + 2.0 * pad;
            let b = Rect { x0: area.x0 + 0.3 * self.fs, y0: y, x1: area.x0 + 0.3 * self.fs + w, y1: y + lh }.round();
            if !b.inside(&area) {
                break;
            }
            self.fill(b, self.theme.bg, 0.85)?;
            self.text(text, (b.x0 + pad, (b.y0 + b.y1) / 2.0), &self.font(0.8, *c), HPos::Left, VPos::Center);
            occ.extend([(b.x0, b.y0), (b.x1, b.y0), (b.x0, b.y1), (b.x1, b.y1)]);
            taken.push(b);
            y = b.y1 + 0.2 * self.fs;
        }
        // a label box at the first candidate spot that is free and inside the plot
        let mut place = |text: &str, cands: &[(f64, f64)], taken: &mut Vec<Rect>| -> Res<bool> {
            let w = self.text_w(text, &font) + 2.0 * pad;
            let Some(b) = cands.iter().map(|&(x, y)| Rect { x0: x, y0: y, x1: x + w, y1: y + lh }.round()).find(|b| b.inside(&area) && !taken.iter().any(|t| t.overlaps(b))) else {
                return Ok(false);
            };
            self.fill(b, self.theme.bg, 0.85)?;
            self.text(text, (b.x0 + pad, (b.y0 + b.y1) / 2.0), &font, HPos::Left, VPos::Center);
            occ.extend([(b.x0, b.y0), (b.x1, b.y0), (b.x0, b.y1), (b.x1, b.y1), ((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0)]);
            taken.push(b);
            Ok(true)
        };
        // the mirror axes' labels at the top, then the points'
        for (x, text, _) in &axes {
            let w = self.text_w(text, &font) + 2.0 * pad;
            let top = area.y0 + 0.3 * self.fs;
            place(text, &[(x + 0.3 * self.fs, top), (x - 0.3 * self.fs - w, top), (x + 0.3 * self.fs, area.y1 - lh - 0.3 * self.fs)], &mut taken)?;
        }
        let mut shown = 0;
        for ((x, y), _, text, _, below) in &pts {
            if shown == 12 {
                break;
            }
            let w = self.text_w(text, &font) + 2.0 * pad;
            let (up, dn, mid) = (y - gap - lh, y + gap, y - lh / 2.0);
            let (l, c, rt) = (x - gap - w, x - w / 2.0, x + gap);
            let (first, second) = if *below { (dn, up) } else { (up, dn) };
            let cands = [(c, first), (rt, first), (l, first), (rt, mid), (l, mid), (c, second), (rt, second), (l, second)];
            shown += place(text, &cands, &mut taken)? as usize;
        }
        Ok(())
    }

    /// Key point marker with a background halo: ▲ max, ▼ min, ○ root, ◇ inflection, □ y-intercept,
    /// a big ringed dot for the vertex.
    fn kp_marker(&self, (x, y): Pt, kind: Kp, c: Rgb) -> Res {
        let (r, halo, bg) = (3.5 * self.k, 1.5 * self.k, self.theme.bg);
        let lw = (1.5 * self.k).round().max(1.0) as u32;
        if matches!(kind, Kp::Root | Kp::Vertex) {
            let rr = if kind == Kp::Vertex { 1.6 * r } else { r };
            self.root.draw(&Circle::new(ipt((x, y)), rr + halo, fill(bg, 1.0)))?;
            if kind == Kp::Vertex {
                self.root.draw(&Circle::new(ipt((x, y)), rr, fill(c, 1.0)))?;
                return Ok(self.root.draw(&Circle::new(ipt((x, y)), rr * 0.45, fill(bg, 1.0)))?);
            }
            return Ok(self.root.draw(&Circle::new(ipt((x, y)), rr, stroke(c, 1.0, lw)))?);
        }
        let shape: &[Pt] = match kind {
            Kp::Max => &[(0.0, -1.3), (1.15, 0.7), (-1.15, 0.7)],
            Kp::Min => &[(0.0, 1.3), (1.15, -0.7), (-1.15, -0.7)],
            Kp::Infl => &[(0.0, -1.35), (1.1, 0.0), (0.0, 1.35), (-1.1, 0.0)],
            _ => &[(-0.95, -0.95), (0.95, -0.95), (0.95, 0.95), (-0.95, 0.95)],
        };
        let at = |s: f64| shape.iter().map(|&(dx, dy)| ipt((x + dx * s, y + dy * s))).collect::<Vec<_>>();
        self.root.draw(&Polygon::new(at(r + 1.6 * halo), fill(bg, 1.0)))?;
        if matches!(kind, Kp::Max | Kp::Min) {
            return Ok(self.root.draw(&Polygon::new(at(r), fill(c, 1.0)))?);
        }
        let mut ring = at(r);
        ring.push(ring[0]);
        Ok(self.root.draw(&PathElement::new(ring, stroke(c, 1.0, lw)))?)
    }

    /// Legend box in the plot corner covering the fewest data points (matplotlib "best").
    fn legend(&self, a: &Axes, plot: Rect, occ: &[Pt]) -> Res {
        let items: Vec<(usize, &Series)> = a.series.iter().enumerate().filter(|(_, s)| !s.label.is_empty()).collect();
        if items.is_empty() || self.tiny {
            return Ok(());
        }
        let (fs, font) = (self.fs, self.font(0.88, self.theme.fg));
        let (row, sw, pad) = (fs * 1.3, fs * 2.0, fs * 0.5);
        let tw = items.iter().map(|(_, s)| self.text_w(&s.label, &font)).fold(0.0, f64::max);
        let (w, h) = (pad * 3.0 + sw + tw, pad * 1.4 + row * items.len() as f64);
        let m = fs * 0.5;
        let corners = [(plot.x1 - m - w, plot.y0 + m), (plot.x0 + m, plot.y0 + m), (plot.x1 - m - w, plot.y1 - m - h), (plot.x0 + m, plot.y1 - m - h)];
        let hits = |&(x, y): &Pt| occ.iter().filter(|p| p.0 >= x && p.0 <= x + w && p.1 >= y && p.1 <= y + h).count();
        let (x0, y0) = corners.into_iter().min_by_key(hits).unwrap_or(corners[0]);
        let r = Rect { x0, y0, x1: x0 + w, y1: y0 + h }.round();
        self.fill(r, self.theme.bg, 0.85)?;
        self.outline(r, self.mix(0.3))?;
        for (j, (i, s)) in items.into_iter().enumerate() {
            let cy = (r.y0 + pad * 0.7 + row * (j as f64 + 0.5)).round();
            let c = self.color(i, s);
            let x = r.x0 + pad;
            let swatch = Rect::new((x + sw * 0.15, cy - fs * 0.35), (x + sw * 0.85, cy + fs * 0.35));
            match s.data {
                Data::Line { .. } | Data::Line3 { .. } => {
                    self.stroke(&[(x, cy), (x + sw, cy)], c, self.lw(s.width), s.line)?;
                    if s.marker != Marker::None {
                        self.marker((x + sw / 2.0, cy), s.marker, c, s.width)?;
                    }
                }
                Data::Bar { .. } | Data::Quiver { .. } => self.fill(swatch, c, 1.0)?,
                Data::Fill { .. } => {
                    self.fill(swatch, c, 0.3)?;
                    if s.line != LineStyle::None {
                        self.outline(swatch.round(), c)?;
                    }
                }
                _ => self.fill(swatch, if s.color.is_some() { c } else { cmap(self.cmap, 0.65) }, 1.0)?,
            }
            self.text(&s.label, (x + sw + pad, cy), &font, HPos::Left, VPos::Center);
        }
        Ok(())
    }

    /// Text notes at data coords like Octave's text(): the text starts at the point (see
    /// `Note::align`), vertically centered, on a translucent box that leaves the point itself
    /// visible; kept inside `area`, hidden once the point is out of view.
    fn notes(&self, a: &Axes, area: Rect, at: impl Fn([f64; 3]) -> Pt, occ: &mut Vec<Pt>) -> Res {
        if self.tiny {
            return Ok(());
        }
        let font = self.font(1.0, self.theme.fg);
        let (lh, pad) = (1.2 * self.fs, 0.3 * self.fs);
        for n in &a.notes {
            let q = at(n.at);
            if n.text.is_empty() || !area.contains(q) {
                continue;
            }
            let lines: Vec<&str> = n.text.lines().collect();
            let w = lines.iter().map(|l| self.text_w(l, &font)).fold(0.0, f64::max);
            let h = lh * lines.len() as f64;
            let al = n.align.clamp(0.0, 1.0);
            let x = (q.0 - al * w + (1.0 - 2.0 * al) * pad).min(area.x1 - pad - w).max(area.x0 + pad);
            let y = (q.1 - h / 2.0).min(area.y1 - pad - h).max(area.y0 + pad);
            let b = Rect { x0: x - pad, y0: y - pad / 2.0, x1: x + w + pad, y1: y + h + pad / 2.0 }.round();
            self.fill(b, self.theme.bg, 0.85)?;
            for (j, l) in lines.iter().enumerate() {
                let lx = x + al * (w - self.text_w(l, &font));
                self.text(l, (lx, y + lh * (j as f64 + 0.5)), &font, HPos::Left, VPos::Center);
            }
            occ.extend([(b.x0, b.y0), (b.x1, b.y0), (b.x0, b.y1), (b.x1, b.y1), ((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0)]);
        }
        Ok(())
    }

    /// Pinned data tips: a dot plus a small "x: …" box kept inside `area`; tiny (terminal) plots get the dot.
    // ponytail: tip boxes don't avoid each other; nudge them apart if dense pins get common
    fn pins(&self, a: &Axes, area: Rect, at: impl Fn([f64; 3]) -> Pt, occ: &mut Vec<Pt>) -> Res {
        // z where it means something: 3D, or the value of colormapped 2D data
        let zs = a.is_3d() || a.series.iter().any(|s| matches!(s.data, Data::Image { .. } | Data::Contour { .. }));
        let font = self.font(0.85, self.theme.fg);
        let (pad, lh, off) = (0.45 * self.fs, 0.95 * self.fs, 0.5 * self.fs);
        for &p in &a.pins {
            let q = at(p);
            if !area.contains(q) {
                continue; // panned out of view (or not finite)
            }
            let r = 3.0 * self.k;
            self.root.draw(&Circle::new(ipt(q), r + 1.5 * self.k, fill(self.theme.bg, 1.0)))?;
            self.root.draw(&Circle::new(ipt(q), r, fill(self.theme.fg, 1.0)))?;
            occ.push(q);
            if self.tiny {
                continue;
            }
            let n = if zs && p[2].is_finite() { 3 } else { 2 };
            let lines: Vec<String> = ["x", "y", "z"][..n].iter().zip(p).map(|(k, v)| format!("{k}: {}", cursor::num(v).replace('-', "\u{2212}"))).collect();
            let (w, h) = (lines.iter().map(|l| self.text_w(l, &font)).fold(0.0, f64::max) + 2.0 * pad, lh * n as f64 + 1.2 * pad);
            // upper right of the dot, flipped at the edges, then clamped inside
            let x = if q.0 + off + w > area.x1 { q.0 - off - w } else { q.0 + off };
            let y = if q.1 - off - h < area.y0 { q.1 + off } else { q.1 - off - h };
            let (x, y) = (x.min(area.x1 - w).max(area.x0), y.min(area.y1 - h).max(area.y0));
            let b = Rect { x0: x, y0: y, x1: x + w, y1: y + h }.round();
            self.fill(b, self.theme.bg, 0.9)?;
            self.outline(b, self.mix(0.45))?;
            for (j, l) in lines.iter().enumerate() {
                self.text(l, (b.x0 + pad, b.y0 + 0.6 * pad + lh * (j as f64 + 0.5)), &font, HPos::Left, VPos::Center);
            }
            occ.extend([(b.x0, b.y0), (b.x1, b.y0), (b.x0, b.y1), (b.x1, b.y1), ((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0)]);
        }
        Ok(())
    }

    fn axes3d(&self, i: usize, a: &Axes, cell: Rect, frames: &mut Vec<Frame>) -> Res {
        let fs = self.fs;
        let b = a.bounds();
        let mut reg = cell;
        if !self.tiny && !a.title.is_empty() {
            self.text(&a.title, ((cell.x0 + cell.x1) / 2.0, cell.y0 + 0.7 * fs), &self.title_font(), HPos::Center, VPos::Top);
            reg.y0 += 1.6 * fs;
        }
        let sub = self.root.clone().shrink((reg.x0 as i32, reg.y0 as i32), (reg.w().max(1.0) as u32, reg.h().max(1.0) as u32));
        // Octave (x, y, z) -> plotters (y, z, x): plotters' vertical axis is its y, and the cyclic swap keeps handedness
        let mut chart = ChartBuilder::on(&sub).build_cartesian_3d(b[1].0..b[1].1, b[2].0..b[2].1, b[0].0..b[0].1)?;
        let (az, el) = (a.view.yaw, a.view.pitch);
        let proj = move |scale: f64| {
            move |mut pb: ProjectionMatrixBuilder| {
                pb.yaw = az - std::f64::consts::FRAC_PI_2;
                pb.pitch = el;
                pb.scale = scale;
                pb.into_matrix()
            }
        };
        let at = |cs: &Cartesian3d<_, _, _>, p: [f64; 3]| {
            let q = cs.translate(&(p[1], p[2], p[0]));
            (q.0 as f64, q.1 as f64)
        };
        let mid = [0, 1, 2].map(|k| (b[k].0 + b[k].1) / 2.0);
        let corner = |m: usize| -> [f64; 3] { std::array::from_fn(|k| if m >> k & 1 == 0 { b[k].0 } else { b[k].1 }) };
        // fit the projected box into the cell, leaving room for tick and axis labels
        chart.with_projection(proj(1.0));
        let c0 = at(chart.as_coord_spec(), mid);
        let (mut ex, mut ey) = (1.0f64, 1.0f64);
        for m in 0..8 {
            let p = at(chart.as_coord_spec(), corner(m));
            (ex, ey) = (ex.max((p.0 - c0.0).abs()), ey.max((p.1 - c0.1).abs()));
        }
        let (mx, my) = if self.tiny { (1.0, 1.0) } else { (fs * 3.8, fs * 2.4) };
        let fit = ((reg.w() / 2.0 - mx) / ex).min((reg.h() / 2.0 - my) / ey).max(0.05);
        chart.with_projection(proj(fit * a.view.zoom.max(1e-3)));
        let cs = chart.as_coord_spec();
        let rect = [reg.x0, reg.y0, reg.x1, reg.y1].map(|v| v / self.u);
        frames.push(Frame { axes: i, rect, bounds: b, logx: false, logy: false, cube: Some((cs.clone(), self.u)) });
        let px = |p: [f64; 3]| at(cs, std::array::from_fn(|k| p[k].max(b[k].0).min(b[k].1)));
        // unit vector toward the viewer; depth grows away from it (box is a cube in normalized coords)
        let v = [el.cos() * az.sin(), -el.cos() * az.cos(), el.sin()];
        let depth = |p: [f64; 3]| -(0..3).map(|k| v[k] * (p[k] - b[k].0) / (b[k].1 - b[k].0)).sum::<f64>();
        let back: [f64; 3] = std::array::from_fn(|k| if v[k] > 1e-9 { b[k].0 } else { b[k].1 });
        let front: [f64; 3] = std::array::from_fn(|k| if back[k] == b[k].0 { b[k].1 } else { b[k].0 });
        let base = [front[0], front[1], back[2]];
        // ticks per axis from its projected length
        let tk: [Vec<(f64, String)>; 3] = std::array::from_fn(|k| {
            let mut q = base;
            q[k] = if base[k] == b[k].0 { b[k].1 } else { b[k].0 };
            let (s, e) = (px(base), px(q));
            ticks(b[k], false, ((s.0 - e.0).hypot(s.1 - e.1) / (fs * 3.2)).clamp(3.0, 6.0) as usize).0
        });

        // back panes with grid and edges
        let axis = self.mix(0.45);
        let pane = lerp(self.theme.bg, self.theme.grid, 0.3);
        for k in 0..3 {
            let (i, j) = ((k + 1) % 3, (k + 2) % 3);
            let pt = |vi: f64, vj: f64| {
                let mut p = [0.0; 3];
                (p[k], p[i], p[j]) = (back[k], vi, vj);
                p
            };
            let quad = [pt(b[i].0, b[j].0), pt(b[i].1, b[j].0), pt(b[i].1, b[j].1), pt(b[i].0, b[j].1)].map(px);
            self.root.draw(&Polygon::new(quad.map(ipt).to_vec(), fill(pane, 1.0)))?;
            if a.grid {
                for (t, _) in &tk[i] {
                    self.path(&[px(pt(*t, b[j].0)), px(pt(*t, b[j].1))], self.theme.grid, 1.0, self.hair())?;
                }
                for (t, _) in &tk[j] {
                    self.path(&[px(pt(b[i].0, *t)), px(pt(b[i].1, *t))], self.theme.grid, 1.0, self.hair())?;
                }
            }
            self.path(&[quad[0], quad[1], quad[2], quad[3], quad[0]], axis, 1.0, self.hair())?;
        }

        // data, painter's algorithm
        let mut items: Vec<(f64, Prim)> = vec![];
        let fin = |p: &[f64; 3]| p.iter().all(|v| v.is_finite());
        let avg = |p: &[[f64; 3]]| depth(std::array::from_fn(|k| p.iter().map(|q| q[k]).sum::<f64>() / p.len() as f64));
        for (i, s) in a.series.iter().enumerate() {
            let c = self.color(i, s);
            let lw = self.lw(s.width);
            let line = |pts: Vec<[f64; 3]>, items: &mut Vec<(f64, Prim)>| {
                if s.line != LineStyle::None {
                    // ponytail: 3D lines are drawn solid, dash patterns need whole polylines
                    items.extend(pts.windows(2).filter(|w| fin(&w[0]) && fin(&w[1])).map(|w| (avg(w), Prim::Seg(px(w[0]), px(w[1]), c, lw))));
                }
                if s.marker != Marker::None {
                    items.extend(pts.iter().filter(|p| fin(p)).map(|&p| (depth(p), Prim::Mark(px(p), s.marker, c, s.width))));
                }
            };
            match &s.data {
                Data::Line3 { x, y, z } => line(x.iter().zip(y).zip(z).map(|((&x, &y), &z)| [x, y, z]).collect(), &mut items),
                // ponytail: 3D fills are outlines only; filled polygons need splitting into depth-sorted pieces
                Data::Line { x, y } | Data::Fill { x, y } => line(x.iter().zip(y).map(|(&x, &y)| [x, y, 0.0]).collect(), &mut items),
                Data::Contour { x, y, z, levels } => {
                    let lv = if levels.is_empty() { auto_levels(z) } else { levels.clone() };
                    for &l in &lv {
                        let lc = if s.color.is_some() || lv.len() == 1 { c } else { self.cm(l) };
                        for [p, q] in contour_segments(x, y, z, l) {
                            let (p, q) = ([p.0, p.1, l], [q.0, q.1, l]);
                            items.push((avg(&[p, q]), Prim::Seg(px(p), px(q), lc, lw)));
                        }
                    }
                }
                Data::Surface { x, y, z, wire } => {
                    let dense = self.tiny || z.rows > 50 || z.cols > 50;
                    for r in 0..z.rows.saturating_sub(1) {
                        for q in 0..z.cols.saturating_sub(1) {
                            let p = [(r, q), (r, q + 1), (r + 1, q + 1), (r + 1, q)].map(|(i, j)| [x.get(i, j), y.get(i, j), z.get(i, j)]);
                            if !p.iter().all(fin) {
                                continue;
                            }
                            let zc = self.cm(p.iter().map(|p| p[2]).sum::<f64>() / 4.0);
                            let (f, e) = match (wire, dense) {
                                (true, _) => (self.theme.bg, (zc, 1.0)),
                                (false, true) => (zc, (zc, 1.0)),
                                (false, false) => (zc, ([0, 0, 0], 0.45)),
                            };
                            items.push((avg(&p), Prim::Quad(p.map(px), f, e)));
                        }
                    }
                }
                Data::Bar { .. } | Data::Image { .. } | Data::Quiver { .. } => {} // ponytail: 2D-only kinds are skipped in 3D axes
            }
        }
        items.sort_by(|p, q| q.0.total_cmp(&p.0));
        for (_, p) in items {
            match p {
                Prim::Quad(q, f, (e, ea)) => {
                    self.root.draw(&Polygon::new(q.map(ipt).to_vec(), fill(f, 1.0)))?;
                    self.path(&[q[0], q[1], q[2], q[3], q[0]], e, ea, self.hair())?;
                }
                Prim::Seg(p, q, c, w) => self.path(&[p, q], c, 1.0, w)?,
                Prim::Mark(p, m, c, w) => self.marker(p, m, c, w)?,
            }
        }
        // a pin or note without z (2D) sits on z = 0, like 2D data in 3D axes; cursor::project agrees
        let flat = |p: [f64; 3]| px([p[0], p[1], if p[2].is_nan() { 0.0 } else { p[2] }]);
        self.notes(a, reg, flat, &mut vec![])?;
        self.pins(a, reg, flat, &mut vec![])?;
        if self.tiny {
            return Ok(());
        }

        // ticks: x and y along the front floor edges, z on the leftmost vertical edge
        let tick_font = self.font(0.85, self.mix(0.9));
        let tl = 0.35 * fs;
        let center = px(mid);
        let zc = (0..4).map(|m| [corner(m)[0], corner(m)[1], back[2]]).min_by(|p, q| px(*p).0.total_cmp(&px(*q).0)).unwrap_or(base);
        let labels = [&a.xlabel, &a.ylabel, &a.zlabel];
        for k in 0..3 {
            let start = if k == 2 { zc } else { base };
            let mut end = start;
            end[k] = if start[k] == b[k].0 { b[k].1 } else { b[k].0 };
            let (s, e) = (px(start), px(end));
            if (s.0 - e.0).hypot(s.1 - e.1) < 1.5 * fs {
                continue; // looking along this axis
            }
            let dir = if k == 2 {
                (if s.0 <= center.0 { -1.0 } else { 1.0 }, 0.0)
            } else {
                // outward = away from the box along the other floor axis
                let o = 1 - k;
                let mut q = start;
                q[o] += (front[o] - back[o]) * 0.1;
                let p1 = at(cs, q);
                let n = (p1.0 - s.0).hypot(p1.1 - s.1).max(1e-9);
                ((p1.0 - s.0) / n, (p1.1 - s.1) / n)
            };
            let (h, v) = anchor_for(dir);
            let mut wmax: f64 = 0.0;
            for (t, txt) in &tk[k] {
                let mut p = start;
                p[k] = *t;
                let p = px(p);
                self.path(&[p, (p.0 + dir.0 * tl, p.1 + dir.1 * tl)], axis, 1.0, self.hair())?;
                self.text(txt, (p.0 + dir.0 * (tl + 0.3 * fs), p.1 + dir.1 * (tl + 0.3 * fs)), &tick_font, h, v);
                wmax = wmax.max(self.text_w(txt, &tick_font));
            }
            if labels[k].is_empty() {
                continue;
            }
            let m = ((s.0 + e.0) / 2.0, (s.1 + e.1) / 2.0);
            let font = self.font(1.0, self.theme.fg);
            if k == 2 {
                let d = tl + 0.3 * fs + wmax + 0.9 * fs;
                self.text(labels[k], (m.0 + dir.0 * d, m.1), &font.transform(FontTransform::Rotate270), HPos::Center, VPos::Center);
            } else {
                let d = tl + 0.6 * fs + dir.0.abs() * wmax + dir.1.abs() * fs;
                self.text(labels[k], (m.0 + dir.0 * d, m.1 + dir.1 * d), &font, h, v);
            }
        }
        if a.legend {
            self.legend(a, reg, &[])?;
        }
        Ok(())
    }
}

/// Key point kinds, in marker priority order.
#[derive(Clone, Copy, PartialEq)]
enum Kp {
    Vertex,
    Max,
    Min,
    Root,
    YInt,
    Infl,
}

enum Prim {
    Quad([Pt; 4], Rgb, (Rgb, f64)),
    Seg(Pt, Pt, Rgb, u32),
    Mark(Pt, Marker, Rgb, f64),
}

fn anchor_for(d: Pt) -> (HPos, VPos) {
    let h = if d.0 > 0.35 { HPos::Left } else if d.0 < -0.35 { HPos::Right } else { HPos::Center };
    let v = if d.1 > 0.35 { VPos::Top } else if d.1 < -0.35 { VPos::Bottom } else { VPos::Center };
    (h, v)
}

fn ipt(p: Pt) -> (i32, i32) {
    (p.0.round() as i32, p.1.round() as i32)
}
fn rgb(c: Rgb) -> RGBColor {
    RGBColor(c[0], c[1], c[2])
}
fn stroke(c: Rgb, a: f64, w: u32) -> ShapeStyle {
    ShapeStyle { color: RGBAColor(c[0], c[1], c[2], a), filled: false, stroke_width: w }
}
fn fill(c: Rgb, a: f64) -> ShapeStyle {
    ShapeStyle { color: RGBAColor(c[0], c[1], c[2], a), filled: true, stroke_width: 1 }
}
fn lerp(a: Rgb, b: Rgb, t: f64) -> Rgb {
    std::array::from_fn(|k| (a[k] as f64 + (b[k] as f64 - a[k] as f64) * t).round() as u8)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl Rect {
    fn new(a: Pt, b: Pt) -> Rect {
        Rect { x0: a.0.min(b.0), y0: a.1.min(b.1), x1: a.0.max(b.0), y1: a.1.max(b.1) }
    }
    fn w(&self) -> f64 {
        self.x1 - self.x0
    }
    fn h(&self) -> f64 {
        self.y1 - self.y0
    }
    fn round(self) -> Rect {
        Rect { x0: self.x0.round(), y0: self.y0.round(), x1: self.x1.round(), y1: self.y1.round() }
    }
    fn contains(&self, p: Pt) -> bool {
        p.0 >= self.x0 && p.0 <= self.x1 && p.1 >= self.y0 && p.1 <= self.y1
    }
    fn overlaps(&self, o: &Rect) -> bool {
        self.x0 < o.x1 && o.x0 < self.x1 && self.y0 < o.y1 && o.y0 < self.y1
    }
    fn inside(&self, o: &Rect) -> bool {
        self.x0 >= o.x0 && self.x1 <= o.x1 && self.y0 >= o.y0 && self.y1 <= o.y1
    }
    fn clip(&self, o: &Rect) -> Option<Rect> {
        let r = Rect { x0: self.x0.max(o.x0), y0: self.y0.max(o.y0), x1: self.x1.min(o.x1), y1: self.y1.min(o.y1) };
        (r.x0 < r.x1 && r.y0 < r.y1).then_some(r)
    }
}

/// Data -> pixel along one axis, linear or log10.
struct Scale {
    lo: f64,
    hi: f64,
    log: bool,
    p0: f64,
    p1: f64,
}

impl Scale {
    fn new((lo, hi): (f64, f64), log: bool, p0: f64, p1: f64) -> Scale {
        let f = |v: f64| if log { v.log10() } else { v };
        Scale { lo: f(lo), hi: f(hi), log, p0, p1 }
    }
    fn px(&self, v: f64) -> f64 {
        let v = if !self.log { v } else if v > 0.0 { v.log10() } else { f64::NAN };
        self.p0 + (v - self.lo) / (self.hi - self.lo) * (self.p1 - self.p0)
    }
}

/// Log-axis range: positive data extent padded in log space, unless set explicitly.
// ponytail: Axes::bounds() is linear; a log-aware GUI zoom would want this exposed.
fn log_range((lo, hi): (f64, f64), explicit: bool, data: Option<(f64, f64)>) -> (f64, f64) {
    if explicit && lo > 0.0 && hi > lo {
        return (lo, hi);
    }
    let Some((lo, hi)) = data else { return (1.0, 10.0) };
    if lo == hi {
        return (lo / 10.0, hi * 10.0);
    }
    let pad = 10f64.powf((hi / lo).log10() * 0.05);
    (lo / pad, hi * pad)
}

/// Positive finite extent of line/bar/quiver data along x (k=0) or y (k=1).
fn pos_extent(a: &Axes, k: usize) -> Option<(f64, f64)> {
    let vals = a.series.iter().flat_map(|s| match &s.data {
        Data::Line { x, y } | Data::Fill { x, y } | Data::Bar { x, y, .. } | Data::Quiver { x, y, .. } => if k == 0 { x.as_slice() } else { y.as_slice() },
        _ => &[],
    });
    vals.filter(|v| **v > 0.0 && v.is_finite()).fold(None, |r, &v| Some(r.map_or((v, v), |(a, b): (f64, f64)| (a.min(v), b.max(v)))))
}

/// Color range over all colormapped data of an axes.
fn clim(a: &Axes) -> (f64, f64) {
    let mut r = (f64::INFINITY, f64::NEG_INFINITY);
    for s in &a.series {
        if let Data::Surface { z, .. } | Data::Contour { z, .. } | Data::Image { z, .. } = &s.data {
            for &v in z.data.iter().filter(|v| v.is_finite()) {
                r = (r.0.min(v), r.1.max(v));
            }
        }
    }
    if r.0 < r.1 { r } else if r.0.is_finite() { (r.0 - 1.0, r.0 + 1.0) } else { (0.0, 1.0) }
}

/// ~n nice major ticks (value, label) in [lo, hi], plus minor ticks (log axes only).
fn ticks((lo, hi): (f64, f64), log: bool, n: usize) -> (Vec<(f64, String)>, Vec<f64>) {
    let (lo, hi, n) = (lo.min(hi), lo.max(hi), n.max(1)); // reversed axes (matview) tick the same values
    if log && lo > 0.0 {
        let (a, b) = (lo.log10().ceil() as i32, hi.log10().floor() as i32);
        if b > a {
            let stride = ((b - a + 1) as usize).div_ceil(n).max(1) as i32;
            let major = (a..=b).filter(|e| e.rem_euclid(stride) == 0).map(|e| (10f64.powi(e), decade(e))).collect();
            let minor = if stride > 1 { vec![] } else { (a - 1..=b).flat_map(|e| (2..10).map(move |m| m as f64 * 10f64.powi(e))).filter(|v| *v >= lo && *v <= hi).collect() };
            return (major, minor);
        }
    }
    let raw = (hi - lo) / n as f64;
    if !(raw.is_finite() && raw > 0.0) {
        return (vec![], vec![]);
    }
    let mag = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 2.5, 5.0, 10.0].iter().map(|m| m * mag).find(|s| *s >= raw * 0.999).unwrap_or(10.0 * mag);
    // pixel-centered ranges (imagesc, matview: 0.5 .. n + 0.5) tick whole cells only
    let half = |v: f64| (v - 0.5).fract() == 0.0;
    let step = if half(lo) && half(hi) { step.max(1.0) } else { step };
    let (first, last) = ((lo / step - 1e-6).ceil() as i64, (hi / step + 1e-6).floor() as i64);
    let big = lo.abs().max(hi.abs());
    let sci = !(1e-4..1e6).contains(&big);
    ((first..=last).map(|i| (i as f64 * step, fmt_tick(i as f64 * step, step, sci))).collect(), vec![])
}

/// Tick label with just enough digits for `step`.
fn fmt_tick(v: f64, step: f64, sci: bool) -> String {
    if v.abs() < step * 1e-6 {
        return "0".into();
    }
    let s = if sci {
        let digits = (v.abs().log10().floor() - step.log10().floor()).clamp(0.0, 8.0) as usize;
        format!("{v:.digits$e}")
    } else {
        let dec = (0..10).find(|&d| {
            let x = step * 10f64.powi(d);
            (x - x.round()).abs() < 1e-6 * x.max(1.0)
        });
        let prec = dec.unwrap_or(10) as usize;
        format!("{v:.prec$}")
    };
    s.replace('-', "\u{2212}")
}

/// Log-axis label for 10^e: plain up to 1000, else 1e6 (no superscripts: common fonts lack ⁰ and ⁻).
fn decade(e: i32) -> String {
    let s = if (-3..=3).contains(&e) { format!("{}", 10f64.powi(e)) } else { format!("1e{e}") };
    s.replace('-', "\u{2212}")
}

/// Auto contour levels: nice values strictly inside the data range.
pub(crate) fn auto_levels(z: &Matrix) -> Vec<f64> {
    let (lo, hi) = z.data.iter().filter(|v| v.is_finite()).fold((f64::INFINITY, f64::NEG_INFINITY), |r, &v| (r.0.min(v), r.1.max(v)));
    ticks((lo, hi), false, 10).0.into_iter().map(|t| t.0).filter(|&t| t > lo && t < hi).collect()
}

/// Liang-Barsky: the part of segment a-b inside r.
fn clip_seg(a: Pt, b: Pt, r: &Rect) -> Option<(Pt, Pt)> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (p, q) in [(-dx, a.0 - r.x0), (dx, r.x1 - a.0), (-dy, a.1 - r.y0), (dy, r.y1 - a.1)] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else if p < 0.0 {
            t0 = t0.max(q / p);
        } else {
            t1 = t1.min(q / p);
        }
    }
    (t0 <= t1).then(|| ((a.0 + t0 * dx, a.1 + t0 * dy), (a.0 + t1 * dx, a.1 + t1 * dy)))
}

/// Sutherland-Hodgman: polygon `p` clipped to `r`.
fn clip_polygon(p: &[Pt], r: &Rect) -> Vec<Pt> {
    let mut out = p.to_vec();
    // (axis, bound, keep the side above it)
    for (k, b, above) in [(0, r.x0, true), (0, r.x1, false), (1, r.y0, true), (1, r.y1, false)] {
        let v = |q: Pt| if k == 0 { q.0 } else { q.1 };
        let ins = |q: Pt| if above { v(q) >= b } else { v(q) <= b };
        let src = std::mem::take(&mut out);
        for (j, &q) in src.iter().enumerate() {
            let a = src[(j + src.len() - 1) % src.len()];
            if ins(q) != ins(a) {
                let t = (b - v(a)) / (v(q) - v(a));
                out.push((a.0 + t * (q.0 - a.0), a.1 + t * (q.1 - a.1)));
            }
            if ins(q) {
                out.push(q);
            }
        }
    }
    out
}

/// Split a polyline at non-finite points and clip it to `r`; returns the visible runs.
fn clip_polyline(pts: &[Pt], r: &Rect) -> Vec<Vec<Pt>> {
    let fin = |p: &Pt| p.0.is_finite() && p.1.is_finite();
    let mut out = vec![];
    let mut cur: Vec<Pt> = vec![];
    for w in pts.windows(2) {
        let seg = if fin(&w[0]) && fin(&w[1]) { clip_seg(w[0], w[1], r) } else { None };
        match seg {
            Some((p, q)) => {
                if cur.last() != Some(&p) {
                    if cur.len() > 1 {
                        out.push(std::mem::take(&mut cur));
                    }
                    cur = vec![p];
                }
                cur.push(q);
            }
            None if cur.len() > 1 => out.push(std::mem::take(&mut cur)),
            None => cur.clear(),
        }
    }
    if cur.len() > 1 {
        out.push(cur);
    }
    out
}

/// On-pieces of a polyline under a repeating [on, off, on, off, ...] pattern in pixels.
fn dash(pts: &[Pt], pat: &[f64]) -> Vec<Vec<Pt>> {
    let mut out = vec![];
    let Some(&first) = pts.first() else { return out };
    let (mut cur, mut k, mut left) = (vec![first], 0, pat[0]);
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = (b.0 - a.0).hypot(b.1 - a.1);
        let mut t = 0.0;
        while len - t > left {
            t += left;
            let p = (a.0 + (b.0 - a.0) * t / len, a.1 + (b.1 - a.1) * t / len);
            if k % 2 == 0 {
                cur.push(p);
                out.push(std::mem::take(&mut cur));
            } else {
                cur = vec![p];
            }
            k = (k + 1) % pat.len();
            left = pat[k];
        }
        left -= len - t;
        if k % 2 == 0 {
            cur.push(b);
        }
    }
    if k % 2 == 0 && cur.len() > 1 {
        out.push(cur);
    }
    out
}

/// Marching squares on a (possibly curvilinear) grid: segments of the `level` iso-line.
fn contour_segments(x: &Matrix, y: &Matrix, z: &Matrix, level: f64) -> Vec<[Pt; 2]> {
    let mut out = vec![];
    for i in 0..z.rows.saturating_sub(1) {
        for j in 0..z.cols.saturating_sub(1) {
            // corners clockwise; edge e runs from corner e to corner e+1
            let c = [(i, j), (i, j + 1), (i + 1, j + 1), (i + 1, j)];
            let v = c.map(|(r, q)| z.get(r, q));
            if !v.iter().all(|v| v.is_finite()) {
                continue;
            }
            let up = v.map(|v| v >= level);
            let at = |m: &Matrix, k: usize| m.get(c[k].0, c[k].1);
            let cross = |e: usize| -> Option<Pt> {
                let (a, b) = (e, (e + 1) % 4);
                (up[a] != up[b]).then(|| {
                    let t = (level - v[a]) / (v[b] - v[a]);
                    (at(x, a) + t * (at(x, b) - at(x, a)), at(y, a) + t * (at(y, b) - at(y, a)))
                })
            };
            let e: Vec<Pt> = (0..4).filter_map(cross).collect();
            match e.len() {
                2 => out.push([e[0], e[1]]),
                4 => {
                    // saddle: the cell center decides which pair of corners gets cut off
                    if (v.iter().sum::<f64>() / 4.0 >= level) == up[0] {
                        out.extend([[e[0], e[1]], [e[2], e[3]]]);
                    } else {
                        out.extend([[e[3], e[0]], [e[1], e[2]]]);
                    }
                }
                _ => {}
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plot::{Series, View};

    #[test]
    fn tick_math() {
        let (t, _) = ticks((0.0, 1.0), false, 5);
        assert_eq!(t.iter().map(|t| t.1.as_str()).collect::<Vec<_>>(), ["0", "0.2", "0.4", "0.6", "0.8", "1.0"]);
        let (t, _) = ticks((-3.0, 3.0), false, 6);
        assert_eq!(t.first().unwrap().1, "\u{2212}3");
        let (t, minor) = ticks((1.0, 1000.0), true, 5);
        assert_eq!(t.iter().map(|t| t.1.as_str()).collect::<Vec<_>>(), ["1", "10", "100", "1000"]);
        assert!(minor.contains(&20.0));
        assert_eq!(fmt_tick(2.5e6, 5e5, true), "2.5e6");
        assert_eq!(fmt_tick(0.25, 0.25, false), "0.25");
        assert_eq!(decade(-12), "1e\u{2212}12");
        assert_eq!(decade(-2), "0.01");
        assert!(ticks((0.0, f64::INFINITY), false, 5).0.is_empty());
    }

    #[test]
    fn clipping_and_dashes() {
        let r = Rect { x0: 0.0, y0: 0.0, x1: 10.0, y1: 10.0 };
        // NaN splits, out-of-box segment is clipped
        let runs = clip_polyline(&[(1.0, 1.0), (2.0, 2.0), (f64::NAN, 0.0), (3.0, 3.0), (5.0, 5.0), (20.0, 5.0)], &r);
        assert_eq!(runs, vec![vec![(1.0, 1.0), (2.0, 2.0)], vec![(3.0, 3.0), (5.0, 5.0), (10.0, 5.0)]]);
        assert!(clip_polyline(&[(20.0, 20.0), (30.0, 30.0)], &r).is_empty());
        let d = dash(&[(0.0, 0.0), (10.0, 0.0)], &[3.0, 2.0]);
        assert_eq!(d.len(), 2);
        assert_eq!(d[1], vec![(5.0, 0.0), (8.0, 0.0)]);
        // polygon clipping: the triangle's tip is cut off along x = 10
        let p = clip_polygon(&[(5.0, 2.0), (15.0, 5.0), (5.0, 8.0)], &r);
        assert_eq!(p.len(), 4);
        assert!(p.iter().all(|q| r.contains(*q)) && p.contains(&(10.0, 3.5)) && p.contains(&(10.0, 6.5)));
        assert!(clip_polygon(&[(20.0, 20.0), (30.0, 20.0), (30.0, 30.0)], &r).is_empty());
    }

    #[test]
    fn math_text() {
        let mut buf = vec![0u8; 50 * 50 * 3];
        let root = BitMapBackend::with_buffer(&mut buf, (50, 50)).into_drawing_area();
        let th = Theme::light();
        let p = Painter { root: &root, theme: &th, cmap: "viridis", fs: 14.0, k: 1.0, u: 1.0, tiny: false, clim: (0.0, 1.0) };
        let f = p.font(1.0, th.fg);
        let w = |s: &str| p.text_w(s, &f);
        // scripts are smaller, stacked ones share their x, commands measure as their symbol
        assert!(w("x^2") > w("x") && w("x^2") < w("x2"), "{} {}", w("x^2"), w("x2"));
        assert_eq!(w("x_1^2"), w("x_1"));
        assert_eq!(w("\\alpha + 1"), w("α + 1"));
    }

    fn grid(n: usize, f: fn(f64, f64) -> f64) -> (Matrix, Matrix, Matrix) {
        let v: Vec<f64> = (0..n).map(|i| -2.0 + 4.0 * i as f64 / (n - 1) as f64).collect();
        let x = Matrix::from_rows(&vec![v.clone(); n]);
        let y = x.transpose();
        let z = Matrix::new(n, n, x.data.iter().zip(&y.data).map(|(&a, &b)| f(a, b)).collect());
        (x, y, z)
    }

    #[test]
    fn marching_squares() {
        let (x, y, z) = grid(21, |a, b| a * a + b * b);
        let segs = contour_segments(&x, &y, &z, 1.0);
        assert!(segs.len() > 10);
        for p in segs.iter().flatten() {
            assert!((p.0.hypot(p.1) - 1.0).abs() < 0.03, "{p:?}");
        }
        assert!(contour_segments(&x, &y, &z, 50.0).is_empty());
        assert_eq!(cmap("gray", 0.0), [0, 0, 0]);
        assert_eq!(cmap("nope", 1.0), [253, 231, 37]);
    }

    fn figure(series: Vec<Series>) -> Figure {
        let mut f = Figure::new(1);
        let a = f.axes();
        a.hold = true;
        for s in series {
            a.add(s);
        }
        f
    }

    /// Fraction of pixels in a rect that differ from the background.
    fn ink(px: &[u8], w: u32, bg: Rgb, (x, y, cw, ch): (f64, f64, f64, f64)) -> f64 {
        let mut n = 0;
        for j in y as usize..(y + ch) as usize {
            for i in x as usize..(x + cw) as usize {
                let k = (j * w as usize + i) * 3;
                n += (px[k..k + 3] != bg[..]) as usize;
            }
        }
        n as f64 / (cw * ch)
    }

    #[test]
    fn renders_every_kind() {
        let th = Theme::dark();
        let xs: Vec<f64> = (0..200).map(|i| -4.0 + i as f64 * 0.04).collect();
        let tan: Vec<f64> = xs.iter().map(|&x| if x.cos().abs() < 0.05 { f64::NAN } else { x.tan() }).collect();
        let (x, y, z) = grid(25, |a, b| (a * a + b * b).sqrt().sin());
        let mut line = Series::new(Data::Line { x: xs.clone(), y: tan });
        line.label = "tan".into();
        let mut dashed = Series::new(Data::Line { x: xs.clone(), y: xs.iter().map(|x| x.sin()).collect() });
        (dashed.line, dashed.marker) = (LineStyle::Dashed, Marker::Circle);
        let mut log = figure(vec![Series::new(Data::Line { x: vec![1.0, 10.0, 100.0, 1000.0], y: vec![0.0, 1.0, 2.0, 3.0] })]);
        log.axes[0].logx = true;
        let mut l3 = figure(vec![Series::new(Data::Line3 { x: xs.clone(), y: xs.iter().map(|x| x.cos()).collect(), z: xs.iter().map(|x| x.sin()).collect() })]);
        l3.axes[0].view = View { yaw: 0.3, pitch: 0.5, zoom: 1.2 };
        let mut lf = figure(vec![line, dashed]);
        lf.axes[0].legend = true;
        // two polygons (NaN-split), one reaching far outside the box, plus notes
        let mut fills = figure(vec![Series::new(Data::Fill { x: vec![0.0, 1.0, 1.0, f64::NAN, 2.0, 50.0, 2.0], y: vec![0.0, 0.0, 1.0, f64::NAN, 0.0, 0.5, 1.0] })]);
        (fills.axes[0].xlim, fills.axes[0].legend) = (Some((0.0, 3.0)), true);
        fills.axes[0].series[0].label = "\\int f".into();
        fills.axes[0].notes.push(crate::plot::Note { at: [0.5, 0.5, f64::NAN], text: "a_1^2\nline 2".into(), align: 0.5 });
        let figs = [
            lf,
            log,
            figure(vec![Series::new(Data::Bar { x: vec![1.0, 2.0, 3.0], y: vec![3.0, -1.0, 2.0], width: 0.8 })]),
            l3,
            figure(vec![Series::new(Data::Surface { x: x.clone(), y: y.clone(), z: z.clone(), wire: false })]),
            figure(vec![Series::new(Data::Surface { x: x.clone(), y: y.clone(), z: z.clone(), wire: true })]),
            figure(vec![Series::new(Data::Contour { x: x.clone(), y: y.clone(), z: z.clone(), levels: vec![] })]),
            figure(vec![Series::new(Data::Image { z: z.clone(), x: (0.5, 25.5), y: (0.5, 25.5) })]),
            figure(vec![Series::new(Data::Quiver { x: vec![0.0, 1.0, 2.0], y: vec![0.0, 1.0, 0.0], u: vec![1.0, 0.5, -0.5], v: vec![0.5, 0.5, 1.0] })]),
            fills,
        ];
        for (k, f) in figs.iter().enumerate() {
            let px = render_rgb(f, &th, 400, 300).unwrap();
            assert_eq!(px.len(), 400 * 300 * 3);
            let r = ink(&px, 400, th.bg, (0.0, 0.0, 400.0, 300.0));
            assert!(r > 0.02, "figure {k}: only {r} ink");
        }
        // terminal output: one line per row
        let s = ansi(&figs[0], &th, 40, 12);
        assert_eq!(s.lines().count(), 12);
        assert!(s.contains('▀'));
    }

    #[test]
    fn subplots_fill_their_cells() {
        let th = Theme::light();
        let mut f = Figure::new(1);
        let (x, y, z) = grid(15, |a, b| a * b);
        for c in 0..4 {
            let s = match c {
                0 => Data::Line { x: vec![0.0, 1.0, 2.0], y: vec![1.0, 0.0, 1.0] },
                1 => Data::Bar { x: vec![1.0, 2.0], y: vec![1.0, 2.0], width: 0.8 },
                2 => Data::Surface { x: x.clone(), y: y.clone(), z: z.clone(), wire: false },
                _ => Data::Image { z: z.clone(), x: (0.0, 1.0), y: (0.0, 1.0) },
            };
            f.subplot(2, 2, c).add(Series::new(s));
        }
        let px = render_rgb(&f, &th, 640, 480).unwrap();
        for c in 0..4 {
            let (x, y, w, h) = f.cell_rect(c, 640.0, 480.0);
            // inner half of the cell: data, not just labels
            let r = ink(&px, 640, th.bg, (x + w / 4.0, y + h / 4.0, w / 2.0, h / 2.0));
            assert!(r > 0.02, "cell {c}: {r}");
        }
    }

    #[test]
    fn save_png_and_svg() {
        let f = figure(vec![Series::new(Data::Line { x: vec![0.0, 1.0], y: vec![0.0, 1.0] })]);
        let dir = std::env::temp_dir();
        for ext in ["png", "svg"] {
            let p = dir.join(format!("qcalcp-render-test.{ext}"));
            save(&f, &Theme::light(), &p, 320, 240).unwrap();
            assert!(std::fs::metadata(&p).unwrap().len() > 100);
            let _ = std::fs::remove_file(&p);
        }
        assert!(save(&f, &Theme::light(), Path::new("x.pdf"), 10, 10).is_err());
    }

    #[test]
    fn degenerate_inputs_dont_panic() {
        let mut figs = vec![Figure::new(1)];
        let mut f = Figure::new(2);
        f.axes(); // empty axes
        figs.push(f);
        figs.push(figure(vec![Series::new(Data::Line { x: vec![], y: vec![] }), Series::new(Data::Line { x: vec![f64::NAN; 3], y: vec![1.0, f64::INFINITY, 2.0] })]));
        let mut f = figure(vec![Series::new(Data::Bar { x: vec![1.0], y: vec![f64::NAN], width: 0.8 })]);
        f.axes[0].xlim = Some((5.0, 1.0)); // reversed, as a GUI drag might write
        figs.push(f);
        let mut f = figure(vec![Series::new(Data::Line { x: vec![-1.0, 0.0], y: vec![0.0, -2.0] })]);
        (f.axes[0].logx, f.axes[0].logy) = (true, true);
        figs.push(f);
        let (x, y, _) = grid(3, |a, _| a);
        let mut f = figure(vec![Series::new(Data::Surface { x, y, z: Matrix::filled(3, 3, f64::NAN), wire: false })]);
        f.axes[0].view = View { yaw: 0.0, pitch: std::f64::consts::FRAC_PI_2, zoom: 0.0 };
        figs.push(f);
        // key points on everything, plus curves that exercise them
        let x: Vec<f64> = (0..400).map(|i| -4.0 + i as f64 * 0.02).collect();
        figs.push(figure(vec![
            Series::new(Data::Line { x: x.clone(), y: x.iter().map(|v| v.tan()).collect() }),
            Series::new(Data::Line { x: x.clone(), y: x.iter().map(|v| v * v - 1.0).collect() }),
            Series::new(Data::Line { x: x.clone(), y: vec![f64::NAN; 400] }),
        ]));
        for a in figs.iter_mut().flat_map(|f| &mut f.axes) {
            a.keypoints = true;
        }
        for f in &figs {
            for (w, h) in [(0, 0), (1, 1), (37, 23), (640, 480)] {
                assert_eq!(render_rgb(f, &Theme::dark(), w, h).unwrap().len(), w as usize * h as usize * 3);
            }
        }
    }
}
