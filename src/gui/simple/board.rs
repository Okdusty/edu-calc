//! A canvas over an explicit Figure, shared by the classroom graph and data views. Renders run on a
//! worker thread; until one lands, the last image is drawn shifted and scaled to the current view,
//! so pans, pinches, flings and resizes stay smooth whatever a render costs.
use super::super::{Ui, canvas};
use crate::plot::{Figure, Theme, cursor, render};
use gtk::prelude::*;
use gtk::{cairo, gdk, gio, glib};
use gtk4 as gtk;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

/// A figure copy without curve sources, for the render thread.
struct Detached(Figure);
// SAFETY: a Source::Func holds the interpreter's Rc values; `new` drops every source, so the copy is plain data.
unsafe impl Send for Detached {}
impl Detached {
    fn new(fig: &Figure) -> Self {
        let mut fig = fig.clone();
        fig.axes.iter_mut().flat_map(|a| &mut a.series).for_each(|s| s.source = None);
        Self(fig)
    }
}

/// Text and line scale for a w x h board: 1 up to about 1000 x 600 px, growing with the board
/// (a smartboard's graph is read from across the room) up to 1.6.
fn grow(w: i32, h: i32) -> f64 {
    (w as f64 / 1000.0).min(h as f64 / 600.0).clamp(1.0, 1.6)
}

/// The plot area of `f` (rendered at `old` widget size) once the widget is `new`: the margins
/// (tick labels) keep their size, the plot area takes the change.
fn resized_rect(f: &render::Frame, old: (i32, i32), new: (i32, i32)) -> [f64; 4] {
    let [x0, y0, x1, y1] = f.rect;
    [x0, y0, x1 + (new.0 - old.0) as f64, y1 + (new.1 - old.1) as f64]
}

/// widget size, scale factor, dark theme, figure version
type Key = (i32, i32, i32, bool, u64);
pub struct Board {
    pub root: gtk::Box,
    pub area: gtk::DrawingArea,
    ui: Weak<Ui>,
    pub figure: RefCell<Figure>,
    /// the last finished render and where its axes landed
    shot: RefCell<Option<(Key, cairo::ImageSurface)>>,
    frames: RefCell<Vec<render::Frame>>,
    /// bumped on every figure change; the key last sent to a render, one render at a time
    version: Cell<u64>,
    wanted: Cell<Option<Key>>,
    rendering: Cell<bool>,
    status: gtk::Label,
    changed: RefCell<Option<Box<dyn Fn()>>>,
    resampled: Cell<Option<Instant>>,
    graph: bool,
    drag: Cell<(f64, f64)>,
    /// farthest travel of the current press in px; INFINITY once a second finger lands
    travel: Cell<f64>,
    pinch: Cell<(f64, f64, f64)>,
    pointer: Cell<(f64, f64)>,
    wheel: Cell<u32>,
    fling: RefCell<Option<gtk::TickCallbackId>>,
}

impl Board {
    pub fn new(ui: &Rc<Ui>, graph: bool) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_hexpand(true);
        root.set_vexpand(true);
        // zoom and key-point buttons float over the plot's corner instead of taking a toolbar row
        let tools = gtk::Box::builder().css_classes(["linked"]).halign(gtk::Align::End).valign(gtk::Align::Start).margin_top(8).margin_end(8).build();
        let icon = |name: &str, tip: &str| gtk::Button::builder().icon_name(name).tooltip_text(tip).css_classes(["osd"]).focus_on_click(false).build();
        let plus = icon("zoom-in-symbolic", "Zoom in");
        let minus = icon("zoom-out-symbolic", "Zoom out");
        let reset = icon("zoom-original-symbolic", "Reset view (double-tap the graph)");
        for b in [&plus, &minus, &reset] {
            tools.append(b);
        }
        let points = gtk::ToggleButton::builder()
            .icon_name("mark-location-symbolic")
            .tooltip_text("Key points: max / min, roots, y-intercept, vertex")
            .css_classes(["osd"])
            .visible(graph)
            .focus_on_click(false)
            .build();
        tools.append(&points);
        let area = gtk::DrawingArea::builder().hexpand(true).vexpand(true).height_request(140).focusable(true).build();
        let overlay = gtk::Overlay::builder().child(&area).build();
        overlay.add_overlay(&tools);
        let status = gtk::Label::builder()
            .label("Drag or flick to pan · pinch or scroll to zoom · tap a curve to read a point")
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .margin_start(8)
            .margin_end(8)
            .css_classes(["dim-label", "caption"])
            .build();
        root.append(&overlay);
        root.append(&status);
        let fig = if graph {
            super::model::graph_figure()
        } else {
            let mut f = Figure::new(1);
            f.axes().grid = true;
            f.axes().origin_axes = true;
            f
        };
        let b = Rc::new(Self {
            root,
            area,
            ui: Rc::downgrade(ui),
            figure: RefCell::new(fig),
            shot: RefCell::new(None),
            frames: RefCell::new(vec![]),
            version: Cell::new(0),
            wanted: Cell::new(None),
            rendering: Cell::new(false),
            status,
            changed: RefCell::new(None),
            resampled: Cell::new(None),
            graph,
            drag: Cell::new((0.0, 0.0)),
            travel: Cell::new(0.0),
            pinch: Cell::new((0.0, 0.0, 1.0)),
            pointer: Cell::new((0.0, 0.0)),
            wheel: Cell::new(0),
            fling: RefCell::new(None),
        });
        let weak = Rc::downgrade(&b);
        b.area.set_draw_func(move |area, cr, w, h| {
            let (Some(b), Some(ui)) = (weak.upgrade(), weak.upgrade().and_then(|b| b.ui.upgrade())) else { return };
            if w < 4 || h < 4 {
                return;
            }
            // the Advanced plot theme, so the graph follows dark / light and the ini's colors
            let theme = ui.cfg.borrow().theme().clone();
            let key = (w, h, area.scale_factor(), ui.cfg.borrow().dark, b.version.get());
            if b.wanted.get() != Some(key) && !b.rendering.get() {
                b.render(key, theme.clone());
            }
            b.paint(cr, w, h, &theme);
        });
        let weak = Rc::downgrade(&b);
        b.area.connect_resize(move |_, _, _| {
            if let Some(b) = weak.upgrade() {
                b.resample(false);
            }
        });
        for (button, factor) in [(plus, 0.8), (minus, 1.25)] {
            let weak = Rc::downgrade(&b);
            button.connect_clicked(move |_| {
                if let Some(b) = weak.upgrade() {
                    let center = b.frames.borrow().first().map(|f| ((f.rect[0] + f.rect[2]) / 2.0, (f.rect[1] + f.rect[3]) / 2.0));
                    b.change((0.0, 0.0), factor, center.unwrap_or_default());
                    b.resample(true);
                }
            });
        }
        let weak = Rc::downgrade(&b);
        reset.connect_clicked(move |_| {
            if let Some(b) = weak.upgrade() {
                b.reset();
            }
        });
        let weak = Rc::downgrade(&b);
        points.connect_toggled(move |t| {
            if let Some(b) = weak.upgrade() {
                b.figure.borrow_mut().axes().keypoints = t.is_active();
                b.redraw();
            }
        });

        // one finger / mouse: pan, and a flick keeps it gliding; two fingers: zoom about them and pan with them
        let zoom = gtk::GestureZoom::new();
        let drag = gtk::GestureDrag::new();
        let weak = Rc::downgrade(&b);
        drag.connect_drag_begin(move |_, _, _| {
            if let Some(b) = weak.upgrade() {
                b.stop_fling();
                b.drag.set((0.0, 0.0));
                b.travel.set(0.0);
            }
        });
        let (weak, z) = (Rc::downgrade(&b), zoom.clone());
        drag.connect_drag_update(move |_, x, y| {
            let Some(b) = weak.upgrade() else { return };
            let (lx, ly) = b.drag.replace((x, y));
            b.travel.set(b.travel.get().max(x.hypot(y)));
            if !z.is_recognized() {
                b.change((x - lx, y - ly), 1.0, (0.0, 0.0));
            }
        });
        let weak = Rc::downgrade(&b);
        drag.connect_drag_end(move |_, _, _| {
            if let Some(b) = weak.upgrade() {
                b.resample(true);
            }
        });
        b.area.add_controller(drag);
        let swipe = gtk::GestureSwipe::new();
        let weak = Rc::downgrade(&b);
        swipe.connect_swipe(move |_, vx, vy| {
            if let Some(b) = weak.upgrade()
                && b.travel.get().is_finite()
                && vx.hypot(vy) > 300.0
            {
                b.start_fling((vx, vy));
            }
        });
        b.area.add_controller(swipe);
        let weak = Rc::downgrade(&b);
        zoom.connect_begin(move |g, _| {
            if let Some(b) = weak.upgrade() {
                let (x, y) = g.bounding_box_center().unwrap_or_default();
                b.stop_fling();
                b.pinch.set((x, y, 1.0));
                b.travel.set(f64::INFINITY);
            }
        });
        let weak = Rc::downgrade(&b);
        zoom.connect_scale_changed(move |g, s| {
            let (Some(b), Some((x, y))) = (weak.upgrade(), g.bounding_box_center()) else { return };
            let (lx, ly, ls) = b.pinch.replace((x, y, s));
            let f = s / ls;
            if f.is_finite() && f > 0.0 {
                b.change((x - lx, y - ly), 1.0 / f, (x, y));
            }
        });
        let weak = Rc::downgrade(&b);
        zoom.connect_end(move |_, _| {
            if let Some(b) = weak.upgrade() {
                b.resample(true);
            }
        });
        b.area.add_controller(zoom);
        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(&b);
        motion.connect_motion(move |_, x, y| {
            if let Some(b) = weak.upgrade() {
                b.pointer.set((x, y));
            }
        });
        b.area.add_controller(motion);
        // wheel / touchpad: zoom about the pointer, settled once it rests
        let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let weak = Rc::downgrade(&b);
        wheel.connect_scroll(move |c, _, dy| {
            let Some(b) = weak.upgrade() else { return glib::Propagation::Stop };
            let dy = if c.unit() == gdk::ScrollUnit::Surface { dy / 30.0 } else { dy };
            let step = b.ui.upgrade().map_or(1.15, |ui| ui.cfg.borrow().zoom_step);
            b.stop_fling();
            b.change((0.0, 0.0), step.powf(dy), b.pointer.get());
            let id = b.wheel.get().wrapping_add(1);
            b.wheel.set(id);
            let weak = Rc::downgrade(&b);
            glib::timeout_add_local_once(Duration::from_millis(250), move || {
                if let Some(b) = weak.upgrade().filter(|b| b.wheel.get() == id) {
                    b.resample(true);
                }
            });
            glib::Propagation::Stop
        });
        b.area.add_controller(wheel);
        let tap = gtk::GestureClick::new();
        let weak = Rc::downgrade(&b);
        tap.connect_released(move |_, n, x, y| {
            let Some(b) = weak.upgrade() else { return };
            if b.travel.get() >= canvas::TAP_SLOP {
                return;
            }
            if n == 2 {
                b.reset();
                return;
            }
            let frames = b.frames.borrow();
            let mut fig = b.figure.borrow_mut();
            if let Some((a, p)) = cursor::pin_near(&fig, &frames, x, y, 24.0) {
                fig.axes[a].pins.remove(p);
            } else if let Some(hit) = cursor::pick(&fig, &frames, x, y, cursor::Mode::Nearest { radius: 40.0 }) {
                b.status.set_text(&cursor::readout(hit.point, &[]).0);
                fig.axes[hit.axes].pins.push(hit.point);
            } else if let Some(f) = frames.first() {
                let (x, y) = cursor::unproject(f, (x, y));
                b.status.set_text(&format!("x = {}    y = {}", cursor::num(x), cursor::num(y)));
            }
            drop(fig);
            drop(frames);
            b.redraw();
        });
        b.area.add_controller(tap);
        b
    }

    /// Render `key` on a worker thread; when it lands, draw again (which renders again if the view moved on).
    fn render(self: &Rc<Self>, key: Key, mut theme: Theme) {
        theme.font_size = (theme.font_size as f64 * grow(key.0, key.1)).round() as u32;
        self.wanted.set(Some(key));
        self.rendering.set(true);
        let fig = Detached::new(&self.figure.borrow());
        let job = gio::spawn_blocking(move || {
            let fig = fig; // the whole wrapper moves, not just its field
            canvas::pixels(&fig.0, &theme, key.0, key.1, key.2 as f64)
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = job.await;
            let Some(b) = weak.upgrade() else { return };
            b.rendering.set(false);
            match result.map_err(|_| "renderer crashed".to_string()).and_then(|p| p?.surface()) {
                Ok((image, frames)) => {
                    *b.shot.borrow_mut() = Some((key, image));
                    *b.frames.borrow_mut() = frames;
                }
                Err(e) => b.status.set_text(&format!("Could not draw graph: {e}")),
            }
            b.area.queue_draw();
        });
    }

    /// The last image with its plot area moved and scaled to what the figure shows now, in the plot
    /// area the widget's current size will get. Axis-equal graphs scale evenly, so a resize or a
    /// gesture never squashes the curves or the text.
    fn paint(&self, cr: &cairo::Context, w: i32, h: i32, theme: &Theme) {
        let shot = self.shot.borrow();
        let Some(((sw, sh, ..), image)) = &*shot else { return };
        let fig = self.figure.borrow();
        let frames = self.frames.borrow();
        let (Some(f), Some(a)) = (frames.first(), fig.axes.first()) else {
            // nothing to line up: stretch the whole image
            let _ = cr.save();
            cr.scale(w as f64 / *sw as f64, h as f64 / *sh as f64);
            let _ = cr.set_source_surface(image, 0.0, 0.0);
            let _ = cr.paint();
            let _ = cr.restore();
            return;
        };
        let [x0, y0, x1, y1] = f.rect;
        let rect = resized_rect(f, (*sw, *sh), (w, h));
        let ((ox, oy), (nx, ny)) = ((f.bounds[0], f.bounds[1]), canvas::shown(a, f, rect));
        let resized = (w, h) != (*sw, *sh);
        let bg = theme.bg.map(|v| v as f64 / 255.0);
        let _ = cr.save();
        if resized {
            cr.set_source_rgb(bg[0], bg[1], bg[2]);
            let _ = cr.paint();
        }
        let _ = cr.set_source_surface(image, 0.0, 0.0);
        let _ = cr.paint();
        let (ow, oh, nw, nh) = (x1 - x0, y1 - y0, rect[2] - rect[0], rect[3] - rect[1]);
        // ponytail: linear axes only, the classroom charts have no log scales
        if (resized || (nx, ny) != (ox, oy)) && nx.1 != nx.0 && ny.1 != ny.0 && nw > 1.0 && nh > 1.0 {
            // old plot pixel -> data -> new plot pixel
            let (sx, sy) = ((ox.1 - ox.0) / ow * nw / (nx.1 - nx.0), (oy.1 - oy.0) / oh * nh / (ny.1 - ny.0));
            cr.rectangle(rect[0], rect[1], nw, nh);
            cr.clip();
            cr.set_source_rgb(bg[0], bg[1], bg[2]);
            let _ = cr.paint();
            cr.translate(rect[0] + (ox.0 - nx.0) / (nx.1 - nx.0) * nw, rect[1] + (ny.1 - oy.1) / (ny.1 - ny.0) * nh);
            cr.scale(sx, sy);
            cr.translate(-x0, -y0);
            let _ = cr.set_source_surface(image, 0.0, 0.0);
            cr.source().set_filter(cairo::Filter::Bilinear);
            let _ = cr.paint();
        }
        let _ = cr.restore();
    }

    pub fn connect_changed(&self, f: impl Fn() + 'static) {
        *self.changed.borrow_mut() = Some(Box::new(f));
    }
    pub fn redraw(&self) {
        self.version.set(self.version.get() + 1);
        self.area.queue_draw();
    }
    /// The x range in view (graphs sample around it).
    pub fn range(&self) -> (f64, f64) {
        let fig = self.figure.borrow();
        let (a, f) = (fig.axes.first(), self.frames.borrow().first().cloned());
        let size = self.shot.borrow().as_ref().map(|((w, h, ..), _)| (*w, *h));
        match (a, f, size) {
            // at the widget's size now: a resize lands before its render
            (Some(a), Some(f), Some(old)) => canvas::shown(a, &f, resized_rect(&f, old, (self.area.width(), self.area.height()))).0,
            (a, ..) => a.and_then(|a| a.xlim).unwrap_or((-10.0, 10.0)),
        }
    }
    /// Pan by pixel delta `d`, then zoom by `factor` about pixel `at`.
    fn change(&self, d: (f64, f64), factor: f64, at: (f64, f64)) {
        if let Some(f) = self.frames.borrow().first() {
            canvas::pan_zoom(&mut self.figure.borrow_mut(), f, d, factor, at);
        }
        self.redraw();
        self.resample(false);
    }
    /// Tell the owner the view changed: at once when a gesture ends, else at most every 120 ms.
    fn resample(&self, now: bool) {
        if !now && self.resampled.get().is_some_and(|t| t.elapsed() < Duration::from_millis(120)) {
            return;
        }
        self.resampled.set((!now).then(Instant::now));
        if let Some(f) = &*self.changed.borrow() {
            f();
        }
    }
    fn start_fling(self: &Rc<Self>, v: (f64, f64)) {
        self.stop_fling();
        let (weak, v, last) = (Rc::downgrade(self), Cell::new(v), Cell::new(None::<i64>));
        let id = self.area.add_tick_callback(move |_, clock| {
            let Some(b) = weak.upgrade() else { return glib::ControlFlow::Break };
            let t = clock.frame_time();
            let dt = last.replace(Some(t)).map_or(0.0, |l| (t - l) as f64 / 1e6);
            // ponytail: friction 5/s (a flick glides v/5 px), make it an [input] key if boards want more
            let (vx, vy) = v.get();
            let k = (-5.0 * dt).exp();
            v.set((vx * k, vy * k));
            b.change((vx * dt, vy * dt), 1.0, (0.0, 0.0));
            if vx.hypot(vy) < 40.0 {
                b.fling.take();
                b.resample(true);
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
        *self.fling.borrow_mut() = Some(id);
    }
    fn stop_fling(&self) {
        if let Some(id) = self.fling.take() {
            id.remove();
            self.resample(true);
        }
    }
    pub fn reset(&self) {
        self.stop_fling();
        let mut fig = self.figure.borrow_mut();
        let a = fig.axes();
        a.xlim = self.graph.then_some((-10.0, 10.0));
        a.ylim = self.graph.then_some((-6.0, 6.0));
        a.pins.clear();
        drop(fig);
        self.redraw();
        self.resample(true);
    }
    pub fn export(self: &Rc<Self>, win: &gtk::ApplicationWindow) {
        let d = gtk::FileDialog::builder().title("Export chart (.png or .svg)").initial_name("graph.png").modal(true).build();
        let weak = Rc::downgrade(self);
        d.save(Some(win), None::<&gio::Cancellable>, move |result| {
            let (Some(b), Some(path)) = (weak.upgrade(), result.ok().and_then(|f| f.path())) else { return };
            let mut theme = Theme::light();
            theme.font_size = 18;
            let result = render::save(&b.figure.borrow(), &theme, &path, 1600, 1000);
            b.status.set_text(&match result {
                Ok(()) => format!("Exported {}", path.display()),
                Err(e) => format!("Export failed: {e}"),
            });
        });
    }
}
