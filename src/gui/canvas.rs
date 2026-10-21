//! Plot canvas: cached render of the current figure, touch/mouse pan, zoom and rotate, plus the
//! data cursor (hover readout, trace mode, pinned data tips) drawn as an overlay.
//! 2D pans/zooms re-sample function curves over the new view (`plot::resample`, throttled while
//! the gesture runs, once more at its end); in Octave mode the view it ends on is echoed to the
//! console and REPL history. Plot-bar tools: key points toggle, the figure's live Octave code.
use std::cell::{Cell, RefCell};
use std::f64::consts::{FRAC_PI_2, TAU};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{cairo, gdk, glib};

use super::Ui;
use crate::plot::cursor::{self, Hit, Mode};
use crate::plot::render::Frame;
use crate::plot::{Axes, Figure, Theme, View};

/// (plots version, current figure, w, h, scale bits)
type Key = (u64, usize, i32, i32, u64);

/// A press that moved further than this (px) is a drag, not a click/tap.
pub(super) const TAP_SLOP: f64 = 8.0;

pub struct Canvas {
    pub area: gtk::DrawingArea,
    /// readout mirror for the plot bar (a finger on the canvas hides the label there)
    pub status: gtk::Label,
    /// plot-bar buttons owned by the canvas (key points, figure code ...), placed by mod.rs
    pub tools: gtk::Box,
    keypts: gtk::ToggleButton,
    /// live `Figure::octave()` of the current figure
    code_pop: gtk::Popover,
    code_view: gtk::TextView,
    /// last resample during a gesture (throttle), the gesture changed the view, pending wheel settle
    resampled: Cell<Option<Instant>>,
    moved: Cell<bool>,
    wheel: Cell<u32>,
    /// the last view echoed to the console: its start offset there and the code
    echo: RefCell<Option<(i32, String)>>,
    cache: RefCell<Option<(Key, cairo::ImageSurface)>>,
    /// where each axes landed in the cached image, in widget pixels
    frames: RefCell<Vec<Frame>>,
    /// a gesture is running: render at reduced resolution
    busy: Cell<bool>,
    /// axes index the current gesture acts on
    target: Cell<Option<usize>>,
    drag: Cell<(f64, f64)>,
    /// last pinch center and scale
    pinch: Cell<(f64, f64, f64)>,
    twist: Cell<f64>,
    pointer: Cell<(f64, f64)>,
    /// data cursor (hover, tap or trace)
    hit: Cell<Option<Hit>>,
    trace: Cell<bool>,
    /// current press: farthest travel in px (INFINITY: pinch or long press, never a tap), touch input
    travel: Cell<f64>,
    touch: Cell<bool>,
    /// bumped on every press; a pending single-click pin only fires if it's unchanged
    clicks: Cell<u32>,
}

impl Canvas {
    pub fn new() -> Self {
        Canvas {
            area: gtk::DrawingArea::builder().hexpand(true).vexpand(true).focusable(true).css_classes(["plot"]).build(),
            status: gtk::Label::builder().xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["plot-status", "monospace"]).build(),
            tools: gtk::Box::builder().spacing(6).build(),
            keypts: gtk::ToggleButton::builder().icon_name("mark-location-symbolic").tooltip_text("Key points: max / min, roots, y-intercept, inflections, vertex, symmetry").build(),
            code_pop: gtk::Popover::new(),
            code_view: gtk::TextView::builder().editable(false).monospace(true).wrap_mode(gtk::WrapMode::WordChar).cursor_visible(false).top_margin(6).bottom_margin(6).left_margin(6).right_margin(6).build(),
            resampled: Cell::new(None),
            moved: Cell::new(false),
            wheel: Cell::new(0),
            echo: RefCell::new(None),
            cache: RefCell::new(None),
            frames: RefCell::new(vec![]),
            busy: Cell::new(false),
            target: Cell::new(None),
            drag: Cell::new((0.0, 0.0)),
            pinch: Cell::new((0.0, 0.0, 1.0)),
            twist: Cell::new(0.0),
            pointer: Cell::new((0.0, 0.0)),
            hit: Cell::new(None),
            trace: Cell::new(false),
            travel: Cell::new(0.0),
            touch: Cell::new(false),
            clicks: Cell::new(0),
        }
    }

    /// Drop the cached image (theme changed) and redraw.
    pub fn invalidate(&self) {
        self.cache.take();
        self.area.queue_draw();
    }
}

pub fn attach(ui: &Rc<Ui>) {
    tools(ui);
    let area = &ui.canvas.area;
    let u = ui.clone();
    area.set_draw_func(move |area, cr, w, h| draw(&u, area, cr, w, h));

    // one finger / left button: pan 2D, orbit 3D; in trace mode: move the trace cursor
    let zoom = gtk::GestureZoom::new();
    let drag = gtk::GestureDrag::new();
    let u = ui.clone();
    drag.connect_drag_begin(move |g, x, y| {
        let c = &u.canvas;
        c.target.set(hit(&u, x, y));
        c.drag.set((0.0, 0.0));
        c.travel.set(0.0);
        c.touch.set(is_touch(g));
        if c.trace.get() {
            c.area.grab_focus();
            cursor_at(&u, x, y);
        } else {
            c.busy.set(true);
        }
    });
    let (u, z) = (ui.clone(), zoom.clone());
    drag.connect_drag_update(move |g, ox, oy| {
        let c = &u.canvas;
        let (lx, ly) = c.drag.replace((ox, oy));
        c.travel.set(c.travel.get().max(ox.hypot(oy)));
        if z.is_recognized() {
            return; // two fingers down: the pinch handler pans
        }
        if c.trace.get() {
            if let Some((x, y)) = g.start_point() {
                cursor_at(&u, x + ox, y + oy);
            }
            return;
        }
        let (dx, dy) = (ox - lx, oy - ly);
        let k = u.cfg.borrow().rotate_speed;
        edit(&u, |fig, i, frame| {
            if fig.axes[i].is_3d() {
                orbit(&mut fig.axes[i].view, -dx * k, dy * k, 1.0);
            } else if let Some(f) = frame {
                pan_zoom(fig, f, (dx, dy), 1.0, (0.0, 0.0));
            }
            true
        });
    });
    let u = ui.clone();
    drag.connect_drag_end(move |_, _, _| settle(&u));

    // pinch: zoom (+ pan with the fingers' center in 2D)
    let u = ui.clone();
    zoom.connect_begin(move |g, _| {
        let (x, y) = g.bounding_box_center().unwrap_or_default();
        u.canvas.target.set(hit(&u, x, y));
        u.canvas.pinch.set((x, y, 1.0));
        u.canvas.busy.set(true);
        u.canvas.travel.set(f64::INFINITY);
    });
    let u = ui.clone();
    zoom.connect_scale_changed(move |g, s| {
        let Some((x, y)) = g.bounding_box_center() else { return };
        let (lx, ly, ls) = u.canvas.pinch.replace((x, y, s));
        let f = s / ls;
        if !(f.is_finite() && f > 0.0) {
            return;
        }
        edit(&u, |fig, i, frame| {
            if fig.axes[i].is_3d() {
                orbit(&mut fig.axes[i].view, 0.0, 0.0, f);
            } else if let Some(fr) = frame {
                pan_zoom(fig, fr, (x - lx, y - ly), 1.0 / f, (x, y));
            }
            true
        });
    });
    let u = ui.clone();
    zoom.connect_end(move |_, _| settle(&u));

    // two-finger twist: 3D yaw
    let rotate = gtk::GestureRotate::new();
    rotate.group_with(&zoom);
    let u = ui.clone();
    rotate.connect_begin(move |_, _| u.canvas.twist.set(0.0));
    let u = ui.clone();
    rotate.connect_angle_changed(move |_, _, delta| {
        let last = u.canvas.twist.replace(delta);
        edit(&u, |fig, i, _| {
            let a = &mut fig.axes[i];
            if !a.is_3d() {
                return false;
            }
            orbit(&mut a.view, delta - last, 0.0, 1.0);
            true
        });
    });

    // mouse hover: data cursor readout (touch has no hover; a tap shows it)
    let motion = gtk::EventControllerMotion::new();
    let u = ui.clone();
    motion.connect_motion(move |_, x, y| {
        u.canvas.pointer.set((x, y));
        u.canvas.touch.set(false);
        if u.cfg.borrow().hover_cursor {
            cursor_at(&u, x, y);
        }
    });
    let u = ui.clone();
    motion.connect_leave(move |_| {
        if !u.canvas.trace.get() {
            set_hit(&u, None);
        }
    });

    // wheel / touchpad scroll: zoom about the pointer
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    let u = ui.clone();
    scroll.connect_scroll(move |c, _, dy| {
        // ponytail: touchpad pixel deltas scaled by a guess, make it an [input] key if it feels off
        let dy = if c.unit() == gdk::ScrollUnit::Surface { dy / 30.0 } else { dy };
        let (x, y) = u.canvas.pointer.get();
        let f = u.cfg.borrow().zoom_step.powf(dy);
        u.canvas.target.set(hit(&u, x, y));
        edit(&u, |fig, i, frame| {
            if fig.axes[i].is_3d() {
                orbit(&mut fig.axes[i].view, 0.0, 0.0, 1.0 / f);
            } else if let Some(fr) = frame {
                pan_zoom(fig, fr, (0.0, 0.0), f, (x, y));
            }
            true
        });
        // the wheel has no gesture end: settle once it rests
        let id = u.canvas.wheel.get().wrapping_add(1);
        u.canvas.wheel.set(id);
        let u = u.clone();
        glib::timeout_add_local_once(Duration::from_millis(250), move || {
            if u.canvas.wheel.get() == id {
                settle(&u);
            }
        });
        glib::Propagation::Stop
    });

    // click / tap: pin (or unpin) after the double-click time; double-click / long-press: reset the axes
    let click = gtk::GestureClick::new();
    let u = ui.clone();
    click.connect_pressed(move |_, n, x, y| {
        u.canvas.clicks.set(u.canvas.clicks.get().wrapping_add(1));
        if n == 2 {
            reset_at(&u, x, y);
        }
    });
    let u = ui.clone();
    click.connect_released(move |_, n, x, y| {
        if n != 1 || u.canvas.travel.get() >= TAP_SLOP {
            return;
        }
        cursor_at(&u, x, y); // a tap shows the readout right away
        let (id, h) = (u.canvas.clicks.get(), u.canvas.hit.get());
        let wait = gtk::Settings::default().map_or(400, |s| s.gtk_double_click_time()).max(0) as u64;
        let u = u.clone();
        glib::timeout_add_local_once(Duration::from_millis(wait), move || {
            if u.canvas.clicks.get() == id {
                pin(&u, Some((x, y)), h);
            }
        });
    });
    let long = gtk::GestureLongPress::new();
    long.set_touch_only(true); // mouse has double-click; don't reset on a slow drag
    let u = ui.clone();
    long.connect_pressed(move |_, x, y| {
        u.canvas.travel.set(f64::INFINITY);
        reset_at(&u, x, y);
    });

    // trace keys (canvas focused): Left/Right step, Up/Down switch series, Enter pins
    let keys = gtk::EventControllerKey::new();
    let u = ui.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        if !u.canvas.trace.get() {
            return glib::Propagation::Proceed;
        }
        match key {
            gdk::Key::Left => trace_step(&u, -1, 0),
            gdk::Key::Right => trace_step(&u, 1, 0),
            gdk::Key::Up => trace_step(&u, 0, 1),
            gdk::Key::Down => trace_step(&u, 0, -1),
            gdk::Key::Return | gdk::Key::KP_Enter => pin(&u, None, u.canvas.hit.get()),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });

    area.add_controller(drag);
    area.add_controller(zoom);
    area.add_controller(rotate);
    area.add_controller(motion);
    area.add_controller(scroll);
    area.add_controller(click);
    area.add_controller(long);
    area.add_controller(keys);
}

fn is_touch(g: &impl IsA<gtk::EventController>) -> bool {
    g.current_event_device().is_some_and(|d| d.source() == gdk::InputSource::Touchscreen)
}

/// Apply `f` to every axes of the current figure (reset view, clear pins).
pub fn for_all_axes(ui: &Ui, f: impl Fn(&mut Axes)) {
    if let Ok(mut ip) = ui.interp.try_borrow_mut() {
        let p = &mut ip.plots;
        if let Some(fig) = p.figures.get_mut(p.current) {
            fig.axes.iter_mut().for_each(f);
            p.touch();
        }
    }
    set_hit(ui, ui.canvas.hit.get()); // Δ to the last pin may have changed
}

fn reset_at(ui: &Ui, x: f64, y: f64) {
    ui.canvas.target.set(hit(ui, x, y));
    edit(ui, |fig, i, _| {
        reset(&mut fig.axes[i]);
        true
    });
}

pub fn reset(a: &mut Axes) {
    (a.xlim, a.ylim, a.zlim) = (None, None, None);
    a.view = View::default();
}

/// Axes index under widget point (x, y) in the current figure.
fn hit(ui: &Ui, x: f64, y: f64) -> Option<usize> {
    let ip = ui.interp.try_borrow().ok()?;
    let fig = ip.plots.figures.get(ip.plots.current)?;
    fig.axes_at(x, y, ui.canvas.area.width() as f64, ui.canvas.area.height() as f64)
}

/// Run `f(figure, axes index, its frame)` on the gesture target; if it reports a change, bump the
/// version and redraw, re-sampling curves at most every 100 ms.
fn edit(ui: &Ui, f: impl FnOnce(&mut Figure, usize, Option<&Frame>) -> bool) {
    let c = &ui.canvas;
    let Some(i) = c.target.get() else { return };
    let Ok(mut ip) = ui.interp.try_borrow_mut() else { return };
    let changed = {
        let frames = c.frames.borrow();
        let p = &mut ip.plots;
        let Some(fig) = p.figures.get_mut(p.current) else { return };
        i < fig.axes.len() && f(fig, i, frames.iter().find(|f| f.axes == i))
    };
    if !changed {
        return;
    }
    c.moved.set(true);
    if c.resampled.get().is_none_or(|t| t.elapsed() >= Duration::from_millis(100)) {
        c.resampled.set(Some(Instant::now()));
        crate::plot::resample(&mut ip);
    }
    ip.plots.touch();
    c.area.queue_draw();
}

/// A gesture ended: full-resolution redraw with a last resample; if the view moved, the code
/// popover (when open) refreshed and the view echoed to the Octave console.
fn settle(ui: &Rc<Ui>) {
    let c = &ui.canvas;
    c.busy.set(false);
    if let Ok(mut ip) = ui.interp.try_borrow_mut() {
        crate::plot::resample(&mut ip);
    }
    c.resampled.set(None);
    c.area.queue_draw();
    if !c.moved.replace(false) {
        return;
    }
    if c.code_pop.is_visible() {
        refresh_code(ui);
    }
    echo_view(ui);
}

/// Octave mode: the 2D view a gesture left as one dim console line, `% view: xlim(...); ylim(...);`
/// (replacing the previous one while nothing came after it, so zooming never spams), and as the
/// newest REPL history entry so Up recalls it.
fn echo_view(ui: &Ui) {
    if ui.cfg.borrow().calc_mode {
        return;
    }
    let line = {
        let Ok(ip) = ui.interp.try_borrow() else { return };
        let Some(fig) = ip.plots.figures.get(ip.plots.current) else { return };
        match ui.canvas.target.get() {
            Some(i) if fig.axes.get(i).is_some_and(|a| !a.is_3d()) => fig.view_code(i),
            _ => return,
        }
    };
    if line.is_empty() {
        return;
    }
    let b = ui.console.buffer();
    let prev = ui.canvas.echo.take();
    if let Some((start, old)) = &prev {
        let (mut s, mut e) = (b.iter_at_offset(*start), b.end_iter());
        if b.text(&s, &e, false) == format!("% view: {old}\n") {
            b.delete(&mut s, &mut e);
        }
    }
    if b.tag_table().lookup("dim").is_none() {
        b.create_tag(Some("dim"), &[("foreground", &"#8a8a8a")]);
    }
    let mut end = b.end_iter();
    let start = end.offset();
    b.insert_with_tags_by_name(&mut end, &format!("% view: {line}\n"), &["dim"]);
    b.place_cursor(&b.end_iter());
    ui.console.scroll_mark_onscreen(&b.get_insert());
    let (h, pos) = &mut *ui.history.borrow_mut();
    if prev.as_ref().is_some_and(|(_, old)| h.last() == Some(old)) {
        h.pop();
    }
    h.push(line.clone());
    *pos = h.len();
    ui.canvas.echo.replace(Some((start, line)));
}

// ---- plot-bar tools ----

/// Key points toggle (all axes of the current figure) and the figure-code popover (Copy, To Octave).
fn tools(ui: &Rc<Ui>) {
    let c = &ui.canvas;
    let u = ui.clone();
    c.keypts.connect_clicked(move |b| {
        let on = b.is_active();
        for_all_axes(&u, |a| a.keypoints = on);
    });
    let copy = gtk::Button::builder().label("Copy").tooltip_text("Copy the code").build();
    let send = gtk::Button::builder().label("To Octave").tooltip_text("Put the code into the REPL (or the script, if it was used last)").build();
    let u = ui.clone();
    copy.connect_clicked(move |b| b.clipboard().set_text(&code_text(&u)));
    let u = ui.clone();
    send.connect_clicked(move |_| {
        let code = code_text(&u);
        u.canvas.code_pop.popdown();
        super::send_to_octave(&u, &code);
    });
    let u = ui.clone();
    c.code_pop.connect_show(move |_| refresh_code(&u));
    let row = gtk::Box::builder().spacing(6).halign(gtk::Align::End).build();
    row.append(&copy);
    row.append(&send);
    let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
    col.append(&gtk::ScrolledWindow::builder().child(&c.code_view).min_content_width(420).min_content_height(40).max_content_height(360).propagate_natural_height(true).build());
    col.append(&row);
    c.code_pop.set_child(Some(&col));
    let code = gtk::MenuButton::builder().icon_name("text-x-generic-symbolic").tooltip_text("Octave code of this figure, following pans and zooms").popover(&c.code_pop).build();
    c.tools.append(&c.keypts);
    c.tools.append(&code);
}

/// Current figure's `octave()` into the popover.
fn refresh_code(ui: &Ui) {
    let code = ui.interp.try_borrow().ok().and_then(|ip| Some(ip.plots.figures.get(ip.plots.current)?.octave())).unwrap_or_default();
    let text = if code.is_empty() { "% no Octave code recorded for this figure".to_string() } else { code };
    ui.canvas.code_view.buffer().set_text(&text);
}

fn code_text(ui: &Ui) -> String {
    let b = ui.canvas.code_view.buffer();
    b.text(&b.start_iter(), &b.end_iter(), false).to_string()
}

// ---- data cursor ----

/// Show `h` as the data cursor: overlay, plot-bar readout.
fn set_hit(ui: &Ui, h: Option<Hit>) {
    ui.canvas.hit.set(h);
    ui.canvas.status.set_text(&readout(ui).join("   "));
    ui.canvas.area.queue_draw();
}

/// Cursor readout lines, with Δ to the last pin of its axes.
fn readout(ui: &Ui) -> Vec<String> {
    let Some(h) = ui.canvas.hit.get() else { return vec![] };
    let ip = ui.interp.try_borrow();
    let pins = ip.as_ref().ok().and_then(|ip| Some(ip.plots.figures.get(ip.plots.current)?.axes.get(h.axes)?.pins.clone()));
    let (a, b) = cursor::readout(h.point, &pins.unwrap_or_default());
    [a].into_iter().chain(b).collect()
}

/// Point the data cursor at widget (x, y): nearest point, or along the traced series in trace mode.
fn cursor_at(ui: &Ui, x: f64, y: f64) {
    let c = &ui.canvas;
    let trace = c.trace.get();
    let h = {
        let Ok(ip) = ui.interp.try_borrow() else { return };
        let Some(fig) = ip.plots.figures.get(ip.plots.current) else { return };
        let mode = if trace { Mode::Trace { series: c.hit.get().map_or(0, |h| h.series) } } else { Mode::Nearest { radius: ui.cfg.borrow().pin_radius } };
        cursor::pick(fig, &c.frames.borrow(), x, y, mode)
    };
    if h.is_some() || !trace {
        set_hit(ui, h);
    }
}

fn trace_step(ui: &Ui, di: isize, ds: isize) {
    let h = {
        let Ok(ip) = ui.interp.try_borrow() else { return };
        let Some(fig) = ip.plots.figures.get(ip.plots.current) else { return };
        cursor::step(fig, &ui.canvas.frames.borrow(), ui.canvas.hit.get(), di, ds)
    };
    set_hit(ui, h);
}

pub fn set_trace(ui: &Ui, on: bool) {
    ui.canvas.trace.set(on);
    if on {
        ui.canvas.area.grab_focus();
        trace_step(ui, 0, 0); // snap the cursor onto its series, or start one
    } else {
        set_hit(ui, None);
    }
}

/// Pin `h`; with a tap position, a pin within reach of it is removed instead.
fn pin(ui: &Ui, tap: Option<(f64, f64)>, h: Option<Hit>) {
    let r = ui.cfg.borrow().pin_radius;
    {
        let Ok(mut ip) = ui.interp.try_borrow_mut() else { return };
        let p = &mut ip.plots;
        let Some(fig) = p.figures.get_mut(p.current) else { return };
        let near = tap.and_then(|(x, y)| cursor::pin_near(fig, &ui.canvas.frames.borrow(), x, y, r));
        match (near, h) {
            (Some((a, k)), _) => {
                fig.axes[a].pins.remove(k);
            }
            (None, Some(h)) if h.axes < fig.axes.len() => fig.axes[h.axes].pins.push(h.point),
            _ => return,
        }
        p.touch();
    }
    set_hit(ui, ui.canvas.hit.get());
}

fn draw(ui: &Ui, area: &gtk::DrawingArea, cr: &cairo::Context, w: i32, h: i32) {
    let cfg = ui.cfg.borrow();
    let theme = cfg.theme();
    let Ok(ip) = ui.interp.try_borrow() else { return };
    let c = &ui.canvas;
    let Some(fig) = ip.plots.figures.get(ip.plots.current) else {
        return hint(cr, theme, w, h, "plot(sin(0:0.1:10))");
    };
    let scale = if c.busy.get() { 0.5 } else { area.scale_factor().max(1) as f64 };
    let key = (ip.plots.version, ip.plots.current, w, h, scale.to_bits());
    let mut cache = c.cache.borrow_mut();
    if cache.as_ref().map(|(k, _)| *k) != Some(key) {
        if cache.as_ref().is_some_and(|(k, _)| k.1 != key.1) {
            // another figure: the cursor belonged to the old one
            c.hit.set(None);
            c.status.set_text("");
        }
        c.keypts.set_active(fig.axes.iter().any(|a| a.keypoints));
        match surface(fig, theme, w, h, scale) {
            Ok((s, frames)) => {
                *cache = Some((key, s));
                *c.frames.borrow_mut() = frames;
            }
            Err(e) => {
                *cache = None;
                c.frames.borrow_mut().clear();
                return hint(cr, theme, w, h, &format!("render: {e}"));
            }
        }
    }
    if let Some((_, s)) = cache.as_ref() {
        let _ = cr.set_source_surface(s, 0.0, 0.0);
        let _ = cr.paint();
    }
    let frames = c.frames.borrow();
    if let Some((h0, f)) = c.hit.get().and_then(|h0| Some((h0, frames.iter().find(|f| f.axes == h0.axes)?))) {
        // from the data point, so the cursor stays on it while zooming
        let p = cursor::project(f, h0.point);
        overlay(cr, theme, f, p, &readout(ui), c.touch.get(), (w as f64, h as f64));
    }
}

/// Render at `scale` x the widget size into a cairo surface drawn back at widget size, plus the
/// axes frames in widget pixels.
pub(super) fn surface(fig: &Figure, theme: &Theme, w: i32, h: i32, scale: f64) -> Result<(cairo::ImageSurface, Vec<Frame>), String> {
    pixels(fig, theme, w, h, scale)?.surface()
}

/// The thread-safe half of `surface`: ARGB32 rows and the frames, cairo-free so a render thread can make them.
pub(super) struct Pixels {
    data: Vec<u8>,
    size: (i32, i32, i32, i32),
    stride: i32,
    frames: Vec<Frame>,
}

pub(super) fn pixels(fig: &Figure, theme: &Theme, w: i32, h: i32, scale: f64) -> Result<Pixels, String> {
    let (pw, ph) = (((w as f64 * scale).round() as i32).max(1), ((h as f64 * scale).round() as i32).max(1));
    // the renderer sizes text (and with it lines and markers) in buffer pixels: on a 2x display a
    // 14 px font would come out 7 px, so it gets the device scale
    let theme = Theme { font_size: (theme.font_size as f64 * scale).round() as u32, ..theme.clone() };
    let (rgb, frames) = crate::plot::render::render_frames(fig, &theme, pw as u32, ph as u32)?;
    let stride = cairo::Format::ARgb32.stride_for_width(pw as u32).map_err(|e| e.to_string())?;
    let data = argb(&rgb, pw as usize, ph as usize, stride as usize).ok_or("renderer returned a short buffer")?;
    let k = pw as f64 / w.max(1) as f64;
    Ok(Pixels { data, size: (w.max(1), h.max(1), pw, ph), stride, frames: frames.into_iter().map(|f| f.scaled(k)).collect() })
}

impl Pixels {
    pub fn surface(self) -> Result<(cairo::ImageSurface, Vec<Frame>), String> {
        let (w, h, pw, ph) = self.size;
        let s = cairo::ImageSurface::create_for_data(self.data, cairo::Format::ARgb32, pw, ph, self.stride).map_err(|e| e.to_string())?;
        s.set_device_scale(pw as f64 / w as f64, ph as f64 / h as f64);
        Ok((s, self.frames))
    }
}

/// Data cursor on top of the plot: dashed crosshair (2D), ring marker, readout box next to it
/// (above it for a finger), all inside the w x h canvas.
fn overlay(cr: &cairo::Context, t: &Theme, f: &Frame, p: (f64, f64), lines: &[String], above: bool, (w, h): (f64, f64)) {
    let rgb = |c: [u8; 3]| c.map(|v| v as f64 / 255.0);
    let ([fr, fg, fb], [br, bg, bb]) = (rgb(t.fg), rgb(t.bg));
    if !(p.0.is_finite() && p.1.is_finite()) {
        return;
    }
    if f.cube.is_none() {
        let [x0, y0, x1, y1] = f.rect;
        let (x, y) = (p.0.round() + 0.5, p.1.round() + 0.5);
        cr.set_source_rgba(fr, fg, fb, 0.4);
        cr.set_line_width(1.0);
        cr.set_dash(&[4.0, 3.0], 0.0);
        cr.move_to(x, y0);
        cr.line_to(x, y1);
        cr.move_to(x0, y);
        cr.line_to(x1, y);
        let _ = cr.stroke();
        cr.set_dash(&[], 0.0);
    }
    cr.new_sub_path();
    cr.arc(p.0, p.1, 6.0, 0.0, TAU);
    cr.set_source_rgb(br, bg, bb);
    cr.set_line_width(4.5);
    let _ = cr.stroke_preserve();
    cr.set_source_rgb(fr, fg, fb);
    cr.set_line_width(2.0);
    let _ = cr.stroke();
    if lines.is_empty() {
        return;
    }
    cr.select_font_face("sans-serif", cairo::FontSlant::Normal, cairo::FontWeight::Normal);
    cr.set_font_size(13.0);
    let (pad, lh) = (6.0, 17.0);
    let tw = lines.iter().filter_map(|l| cr.text_extents(l).ok()).map(|e| e.x_advance()).fold(0.0, f64::max);
    let (bw, bh) = (tw + 2.0 * pad, lh * lines.len() as f64 + pad);
    // mouse: up and right of the point; finger: centered well above it. Flipped/clamped to fit.
    let (gap, mut x) = if above { (56.0, p.0 - bw / 2.0) } else { (14.0, p.0 + 14.0) };
    if x + bw > w {
        x = if above { w - bw } else { p.0 - 14.0 - bw };
    }
    let y = if p.1 - gap - bh >= 0.0 { p.1 - gap - bh } else { p.1 + gap.min(24.0) };
    let (x, y) = (x.min(w - bw - 2.0).max(2.0).round(), y.min(h - bh - 2.0).max(2.0).round());
    cr.rectangle(x + 0.5, y + 0.5, bw, bh);
    cr.set_source_rgba(br, bg, bb, 0.92);
    let _ = cr.fill_preserve();
    cr.set_source_rgba(fr, fg, fb, 0.45);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
    cr.set_source_rgb(fr, fg, fb);
    for (i, l) in lines.iter().enumerate() {
        cr.move_to(x + pad, y + pad / 2.0 + lh * (i as f64 + 1.0) - 4.0);
        let _ = cr.show_text(l);
    }
}

/// RGB8 rows -> cairo ARGB32 (native-endian 0xAARRGGBB words) with `stride` bytes per row.
fn argb(rgb: &[u8], w: usize, h: usize, stride: usize) -> Option<Vec<u8>> {
    if rgb.len() < w * h * 3 {
        return None;
    }
    let mut out = vec![0u8; stride * h];
    for (row, src) in out.chunks_exact_mut(stride).zip(rgb.chunks_exact(w * 3)) {
        for (px, p) in row.chunks_exact_mut(4).zip(src.chunks_exact(3)) {
            let v = 0xff00_0000 | (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32;
            px.copy_from_slice(&v.to_ne_bytes());
        }
    }
    Some(out)
}

/// Background plus one centered line of text (empty session hint, render errors).
fn hint(cr: &cairo::Context, t: &Theme, w: i32, h: i32, text: &str) {
    let rgb = |c: [u8; 3]| c.map(|v| v as f64 / 255.0);
    let [r, g, b] = rgb(t.bg);
    cr.set_source_rgb(r, g, b);
    let _ = cr.paint();
    let [r, g, b] = rgb(t.fg);
    cr.set_source_rgba(r, g, b, 0.55);
    cr.select_font_face("monospace", cairo::FontSlant::Normal, cairo::FontWeight::Normal);
    cr.set_font_size(16.0);
    if let Ok(e) = cr.text_extents(text) {
        cr.move_to(((w as f64 - e.width()) / 2.0 - e.x_bearing()).max(4.0), h as f64 / 2.0);
        let _ = cr.show_text(text);
    }
}

/// Pan by pixel delta `d`, then zoom by `factor` (< 1 zooms in) about pixel `p`: 2D axes of frame `f`.
/// Pixels map through the frame's plot area, so the content tracks the finger exactly.
pub fn pan_zoom(fig: &mut Figure, f: &Frame, d: (f64, f64), factor: f64, p: (f64, f64)) {
    let Some(a) = fig.axes.get_mut(f.axes) else { return };
    let [x0, y0, x1, y1] = f.rect;
    let (w, h) = (x1 - x0, y1 - y0);
    let (xr, yr) = shown(a, f, f.rect);
    a.xlim = Some(adjust(xr, -d.0 / w, factor, (p.0 - x0) / w, a.logx));
    a.ylim = Some(adjust(yr, d.1 / h, factor, 1.0 - (p.1 - y0) / h, a.logy));
}

/// The x and y ranges `a` shows in plot area `rect` (px): its limits, else the frame's (limits win:
/// several drag updates can land before the next render), widened like the renderer's axis equal.
pub fn shown(a: &Axes, f: &Frame, rect: [f64; 4]) -> ((f64, f64), (f64, f64)) {
    let (xr, yr) = (a.xlim.unwrap_or(f.bounds[0]), a.ylim.unwrap_or(f.bounds[1]));
    if a.equal && !(a.logx || a.logy) && !a.is_3d() {
        crate::plot::render::equal_ranges(xr, yr, rect[2] - rect[0], rect[3] - rect[1])
    } else {
        (xr, yr)
    }
}

/// Shift range `r` by `shift` spans, then scale it by `factor` about `anchor` (fraction of the span
/// from the low end). Log axes work in log10 space. Non-finite or collapsed results keep `r`.
pub fn adjust(r: (f64, f64), shift: f64, factor: f64, anchor: f64, log: bool) -> (f64, f64) {
    let log = log && r.0 > 0.0 && r.1 > 0.0;
    let (lo, hi) = if log { (r.0.log10(), r.1.log10()) } else { r };
    let span = hi - lo;
    let c = lo + span * (shift + anchor);
    let (a, b) = (c - span * anchor * factor, c + span * (1.0 - anchor) * factor);
    let out = if log { (10f64.powf(a), 10f64.powf(b)) } else { (a, b) };
    // abs: a reversed range (imagesc, matview: y runs down) zooms like any other
    let ok = out.0.is_finite() && out.1.is_finite() && (out.1 - out.0).abs() > out.0.abs().max(out.1.abs()) * 1e-12;
    if ok { out } else { r }
}

/// 3D camera: add yaw/pitch (radians, pitch clamped to ±90°), multiply zoom.
pub fn orbit(v: &mut View, yaw: f64, pitch: f64, zoom: f64) {
    v.yaw += yaw;
    v.pitch = (v.pitch + pitch).clamp(-FRAC_PI_2, FRAC_PI_2);
    let z = v.zoom * zoom;
    if z.is_finite() && z > 0.0 {
        v.zoom = z.clamp(0.05, 50.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9
    }

    #[test]
    fn range_math() {
        assert!(close(adjust((0., 10.), 0.1, 1.0, 0.0, false), (1., 11.)));
        assert!(close(adjust((0., 10.), 0.0, 0.5, 0.5, false), (2.5, 7.5)));
        assert!(close(adjust((0., 10.), 0.0, 2.0, 0.0, false), (0., 20.)));
        assert!(close(adjust((1., 100.), 0.0, 0.5, 0.5, true), (10f64.powf(0.5), 10f64.powf(1.5))));
        assert!(close(adjust((-1., 100.), 0.0, 0.5, 0.0, true), (-1., 49.5))); // log with <= 0: linear
        assert_eq!(adjust((0., 10.), f64::INFINITY, 1.0, 0.0, false), (0., 10.));
        assert_eq!(adjust((0., 10.), 0.0, 0.0, 0.5, false), (0., 10.));
    }

    #[test]
    fn pan_and_zoom_axes() {
        use crate::plot::cursor::project;
        use crate::plot::{Data, Series};
        let mut f = Figure::new(1);
        let a = f.subplot(1, 2, 1); // right half of the canvas
        a.add(Series::new(Data::Line { x: vec![0., 10.], y: vec![0., 10.] }));
        (a.xlim, a.ylim) = (Some((0., 10.)), Some((0., 10.)));
        let (_, frames) = crate::plot::render::render_frames(&f, &Theme::light(), 800, 400).unwrap();
        let fr = frames[0].clone().scaled(2.0); // as the GUI keeps them on a 2x screen
        let [x0, y0, x1, y1] = fr.rect;
        assert!(x0 > 200.0 && x1 < 400.0 && y1 < 200.0);
        // the point under the finger stays under it
        let before = project(&fr, [3., 4., 0.]);
        pan_zoom(&mut f, &fr, ((x1 - x0) / 10.0, (y1 - y0) / 10.0), 1.0, (0., 0.));
        assert!(close(f.axes[0].xlim.unwrap(), (-1., 9.)));
        assert!(close(f.axes[0].ylim.unwrap(), (1., 11.)));
        let moved = Frame { bounds: [f.axes[0].xlim.unwrap(), f.axes[0].ylim.unwrap(), (0., 1.)], ..fr.clone() };
        let after = project(&moved, [3., 4., 0.]);
        assert!((after.0 - before.0 - (x1 - x0) / 10.0).abs() < 1e-9 && (after.1 - before.1 - (y1 - y0) / 10.0).abs() < 1e-9);
        // zoom in 2x about the plot area's center keeps the center
        pan_zoom(&mut f, &fr, (0., 0.), 0.5, ((x0 + x1) / 2.0, (y0 + y1) / 2.0));
        assert!(close(f.axes[0].xlim.unwrap(), (1.5, 6.5)));
        assert!(close(f.axes[0].ylim.unwrap(), (3.5, 8.5)));
    }

    #[test]
    fn shown_matches_axis_equal_render() {
        // the Simple graph's axes; any window shape widens one of its limits
        let mut fig = Figure::new(1);
        let a = fig.axes();
        (a.equal, a.origin_axes, a.grid) = (true, true, true);
        (a.xlim, a.ylim) = (Some((-10., 10.)), Some((-6., 6.)));
        for (w, h, scale) in [(520, 640, 2.0), (1085, 610, 1.0), (300, 900, 1.0), (1600, 300, 2.0)] {
            let p = pixels(&fig, &Theme::dark(), w, h, scale).unwrap();
            let f = &p.frames[0];
            let (x, y) = shown(&fig.axes[0], f, f.rect);
            let [(bx0, bx1), (by0, by1)] = [f.bounds[0], f.bounds[1]];
            // the renderer's second layout may move the plot area by a tick label's width
            let near = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs().max((a.1 - b.1).abs()) < 0.03 * (b.1 - b.0);
            assert!(near(x, (bx0, bx1)) && near(y, (by0, by1)), "{w}x{h}: shown {x:?} {y:?}, drawn {:?}", &f.bounds[..2]);
            // and the drawn ranges keep one scale on both axes
            let [x0, y0, x1, y1] = f.rect;
            let (kx, ky) = ((x1 - x0) / (bx1 - bx0), (y1 - y0) / (by1 - by0));
            assert!((kx / ky - 1.0).abs() < 0.03, "{w}x{h}: {kx} vs {ky} px per unit");
        }
    }

    /// Visual QA without a window: `cargo test overlay_sample -- --ignored`, then look at samples/.
    #[test]
    #[ignore]
    fn overlay_sample() {
        use crate::plot::{Data, Series};
        let mut f = Figure::new(1);
        let xs: Vec<f64> = (0..200).map(|i| i as f64 * 0.05).collect();
        let a = f.axes();
        a.add(Series::new(Data::Line { x: xs.clone(), y: xs.iter().map(|x| (-x / 4.0).exp() * (2.0 * x).sin()).collect() }));
        a.grid = true;
        a.pins.push([0.8, (-0.2f64).exp() * 1.6f64.sin(), f64::NAN]);
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("samples");
        std::fs::create_dir_all(&dir).unwrap();
        for (th, name, above) in [(Theme::dark(), "overlay-dark.png", false), (Theme::light(), "overlay-touch-light.png", true)] {
            let (plot, frames) = surface(&f, &th, 800, 600, 1.0).unwrap();
            let fig = &f;
            let h = cursor::pick(fig, &frames, 380.0, 0.0, Mode::Trace { series: 0 }).unwrap();
            let (a, b) = cursor::readout(h.point, &fig.axes[0].pins);
            let mut out = cairo::ImageSurface::create(cairo::Format::ARgb32, 800, 600).unwrap();
            let cr = cairo::Context::new(&out).unwrap();
            cr.set_source_surface(&plot, 0.0, 0.0).unwrap();
            cr.paint().unwrap();
            overlay(&cr, &th, &frames[0], cursor::project(&frames[0], h.point), &[a, b.unwrap()], above, (800.0, 600.0));
            drop(cr);
            let stride = out.stride() as usize;
            let argb = out.data().unwrap();
            let rgb: Vec<u8> = (0..600 * 800).flat_map(|k| u32::from_ne_bytes(argb[k / 800 * stride + k % 800 * 4..][..4].try_into().unwrap()).to_be_bytes()[1..].to_vec()).collect();
            use plotters::prelude::*;
            let path = dir.join(name);
            let root = BitMapBackend::new(&path, (800, 600)).into_drawing_area();
            root.draw(&BitMapElement::with_owned_buffer((0, 0), (800, 600), rgb).unwrap()).unwrap();
            root.present().unwrap();
        }
    }

    #[test]
    fn camera_and_pixels() {
        let mut v = View::default();
        orbit(&mut v, 0.5, 10.0, 2.0);
        assert_eq!(v.pitch, FRAC_PI_2);
        assert_eq!(v.zoom, 2.0);
        orbit(&mut v, 0.0, 0.0, f64::NAN);
        assert_eq!(v.zoom, 2.0);
        let out = argb(&[1, 2, 3, 4, 5, 6], 2, 1, 12).unwrap();
        assert_eq!(out.len(), 12);
        assert_eq!(u32::from_ne_bytes(out[4..8].try_into().unwrap()), 0xff04_0506);
        assert!(argb(&[1, 2], 2, 1, 8).is_none());
    }
}
