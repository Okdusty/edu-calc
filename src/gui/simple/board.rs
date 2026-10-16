//! A canvas over an explicit Figure, shared by the classroom graph and data views.
use super::super::canvas;
use crate::plot::{Figure, Theme, cursor, render};
use gtk::prelude::*;
use gtk::{cairo, gio, glib};
use gtk4 as gtk;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

type ImageCache = Option<((i32, i32, i32), cairo::ImageSurface)>;
pub struct Board {
    pub root: gtk::Box,
    pub tools: gtk::Box,
    pub area: gtk::DrawingArea,
    pub figure: RefCell<Figure>,
    frames: RefCell<Vec<render::Frame>>,
    cache: RefCell<ImageCache>,
    status: gtk::Label,
    changed: RefCell<Option<Box<dyn Fn()>>>,
    graph: bool,
    drag: Cell<(f64, f64)>,
    dragged: Cell<bool>,
    zoom: Cell<f64>,
}

impl Board {
    pub fn new(graph: bool) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        root.set_hexpand(true);
        root.set_vexpand(true);
        let tools = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let plus = gtk::Button::with_label("Zoom +");
        let minus = gtk::Button::with_label("Zoom −");
        let reset = gtk::Button::with_label("Reset View");
        for b in [&plus, &minus, &reset] {
            b.set_focus_on_click(false);
            tools.append(b);
        }
        let points = gtk::ToggleButton::with_label("Points");
        points.set_visible(graph);
        points.set_focus_on_click(false);
        tools.append(&points);
        let area = gtk::DrawingArea::builder().hexpand(true).vexpand(true).height_request(140).focusable(true).build();
        let status = gtk::Label::builder().label("Drag to pan · Pinch to zoom · Tap a curve to read a point").xalign(0.0).wrap(true).build();
        status.add_css_class("simple-hint");
        root.append(&tools);
        root.append(&area);
        root.append(&status);
        let fig = if graph {
            super::model::graph_figure()
        } else {
            let mut f = Figure::new(1);
            f.axes().grid = true;
            f
        };
        let b = Rc::new(Self {
            root,
            tools,
            area,
            figure: RefCell::new(fig),
            frames: RefCell::new(vec![]),
            cache: RefCell::new(None),
            status,
            changed: RefCell::new(None),
            graph,
            drag: Cell::new((0.0, 0.0)),
            dragged: Cell::new(false),
            zoom: Cell::new(1.0),
        });
        let weak = Rc::downgrade(&b);
        b.area.set_draw_func(move |area, cr, w, h| {
            let Some(b) = weak.upgrade() else { return };
            if w < 4 || h < 4 {
                return;
            }
            let key = (w, h, area.scale_factor());
            if b.cache.borrow().as_ref().map(|(k, _)| *k) != Some(key) {
                let mut theme = Theme::light();
                theme.font_size = 18;
                match canvas::surface(&b.figure.borrow(), &theme, w, h, key.2 as f64) {
                    Ok((surface, frames)) => {
                        *b.frames.borrow_mut() = frames;
                        *b.cache.borrow_mut() = Some((key, surface));
                    }
                    Err(e) => {
                        b.status.set_text(&format!("Could not draw graph: {e}"));
                        return;
                    }
                }
            }
            if let Some((_, image)) = &*b.cache.borrow() {
                let _ = cr.set_source_surface(image, 0.0, 0.0);
                let _ = cr.paint();
            }
        });
        let weak = Rc::downgrade(&b);
        b.area.connect_resize(move |_, _, _| {
            let weak = weak.clone();
            glib::idle_add_local_once(move || {
                if let Some(b) = weak.upgrade() {
                    if let Some(f) = &*b.changed.borrow() {
                        f();
                    }
                }
            });
        });
        for (button, factor) in [(plus, 0.8), (minus, 1.25)] {
            let weak = Rc::downgrade(&b);
            button.connect_clicked(move |_| {
                if let Some(b) = weak.upgrade() {
                    b.change((0.0, 0.0), factor);
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
        let drag = gtk::GestureDrag::new();
        let weak = Rc::downgrade(&b);
        drag.connect_drag_begin(move |_, _, _| {
            if let Some(b) = weak.upgrade() {
                b.drag.set((0.0, 0.0));
                b.dragged.set(false);
            }
        });
        let weak = Rc::downgrade(&b);
        drag.connect_drag_update(move |_, x, y| {
            if let Some(b) = weak.upgrade() {
                let prev = b.drag.replace((x, y));
                if x.abs() + y.abs() > 5.0 {
                    b.dragged.set(true);
                }
                b.change((x - prev.0, y - prev.1), 1.0);
            }
        });
        b.area.add_controller(drag);
        let zoom = gtk::GestureZoom::new();
        let weak = Rc::downgrade(&b);
        zoom.connect_begin(move |_, _| {
            if let Some(b) = weak.upgrade() {
                b.zoom.set(1.0);
                b.dragged.set(true);
            }
        });
        let weak = Rc::downgrade(&b);
        zoom.connect_scale_changed(move |_, scale| {
            if let Some(b) = weak.upgrade() {
                let old = b.zoom.replace(scale);
                b.change((0.0, 0.0), old / scale);
            }
        });
        b.area.add_controller(zoom);
        let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let weak = Rc::downgrade(&b);
        wheel.connect_scroll(move |_, _, dy| {
            if let Some(b) = weak.upgrade() {
                b.change((0.0, 0.0), 1.15f64.powf(dy));
            }
            glib::Propagation::Stop
        });
        b.area.add_controller(wheel);
        let tap = gtk::GestureClick::new();
        let weak = Rc::downgrade(&b);
        tap.connect_released(move |_, n, x, y| {
            if let Some(b) = weak.upgrade() {
                if b.dragged.get() {
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
            }
        });
        b.area.add_controller(tap);
        b
    }

    pub fn connect_changed(&self, f: impl Fn() + 'static) {
        *self.changed.borrow_mut() = Some(Box::new(f));
    }
    pub fn redraw(&self) {
        self.cache.borrow_mut().take();
        self.area.queue_draw();
    }
    pub fn range(&self) -> (f64, f64) {
        self.frames.borrow().first().map(|f| f.bounds[0]).unwrap_or((-10.0, 10.0))
    }
    fn change(&self, delta: (f64, f64), factor: f64) {
        if let Some(f) = self.frames.borrow().first() {
            let center = ((f.rect[0] + f.rect[2]) / 2.0, (f.rect[1] + f.rect[3]) / 2.0);
            canvas::pan_zoom(&mut self.figure.borrow_mut(), f, delta, factor, center);
        }
        self.redraw();
        if let Some(f) = &*self.changed.borrow() {
            f();
        }
    }
    pub fn reset(&self) {
        let mut fig = self.figure.borrow_mut();
        let a = fig.axes();
        a.xlim = self.graph.then_some((-10.0, 10.0));
        a.ylim = self.graph.then_some((-6.0, 6.0));
        a.pins.clear();
        drop(fig);
        self.frames.borrow_mut().clear();
        self.redraw();
        if let Some(f) = &*self.changed.borrow() {
            f();
        }
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
