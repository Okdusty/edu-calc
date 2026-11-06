// A touch-first canvas over one axes, for the Graph and Data tabs: pan with a flick that keeps gliding,
// pinch, wheel, zoom buttons, tap for a point, double-tap to reset (src/gui/simple/board.rs).
import { adjust, num, readout } from './logic.js';
import { SvgContext, THEMES, draw, pick, pinNear, shownRanges, unproject } from './plot.js';

const TAP_SLOP = 8;
const HINT = 'Drag or flick to pan · pinch or scroll to zoom · tap a curve to read a point';
const ICONS = {
  plus: '<path d="M7 2h2v5h5v2H9v5H7V9H2V7h5z"/>',
  minus: '<path d="M2 7h12v2H2z"/>',
  reset: '<path d="M2 3h12v10H2zm2 2v6h8V5z" fill-rule="evenodd"/><path d="M7 6h2v4H7z"/>',
  pin: '<path d="M8 1a5 5 0 0 1 5 5c0 3.5-5 9-5 9S3 9.500 3 6a5 5 0 0 1 5-5zm0 3a2 2 0 1 0 0 4 2 2 0 0 0 0-4z" fill-rule="evenodd"/>',
};
const icon = (name) => `<svg viewBox="0 0 16 16" width="16" height="16" fill="currentColor" aria-hidden="true">${ICONS[name]}</svg>`;

export function emptyAxes(graph) {
  return graph
    ? { series: [], pins: [], grid: true, origin: true, equal: true, xlim: [-10, 10], ylim: [-6, 6], xlabel: 'x', ylabel: 'y', keypoints: false }
    : { series: [], pins: [], grid: true, origin: true, equal: false, xlim: null, ylim: null, xlabel: '', ylabel: '', keypoints: false };
}

export class Board {
  constructor(graph, themeName) {
    this.graph = graph;
    this.themeName = themeName;
    this.axes = emptyAxes(graph);
    this.frame = null;
    this.changed = () => {};
    this.resampled = 0;
    this.pointers = new Map();
    this.travel = 0;
    this.lastTap = 0;
    this.root = document.createElement('div');
    this.root.className = 'board';
    this.root.innerHTML = `<div class="board-area"><canvas></canvas><div class="board-tools"></div></div><div class="board-status caption dim"></div>`;
    this.canvas = this.root.querySelector('canvas');
    this.area = this.root.querySelector('.board-area');
    this.status = this.root.querySelector('.board-status');
    this.status.textContent = HINT;
    const tools = this.root.querySelector('.board-tools');
    const tool = (name, label, f) => {
      const b = document.createElement('button');
      b.type = 'button'; b.className = 'osd'; b.innerHTML = icon(name); b.title = label; b.setAttribute('aria-label', label);
      b.addEventListener('mousedown', (e) => e.preventDefault());
      b.addEventListener('click', f);
      tools.append(b);
      return b;
    };
    const center = () => (this.frame ? [(this.frame.rect[0] + this.frame.rect[2]) / 2, (this.frame.rect[1] + this.frame.rect[3]) / 2] : [0, 0]);
    tool('plus', 'Zoom in', () => { this.change([0, 0], 0.8, center()); this.resample(true); });
    tool('minus', 'Zoom out', () => { this.change([0, 0], 1.25, center()); this.resample(true); });
    tool('reset', 'Reset view (double-tap the graph)', () => this.reset());
    if (graph) {
      this.points = tool('pin', 'Key points: max / min, roots, y-intercept, vertex', () => {
        this.axes.keypoints = !this.axes.keypoints;
        this.points.setAttribute('aria-pressed', String(this.axes.keypoints));
        this.redraw();
      });
      this.points.setAttribute('aria-pressed', 'false');
    }
    new ResizeObserver(() => { this.redraw(); this.resample(false); }).observe(this.area);
    this.bind();
  }

  bind() {
    const c = this.canvas;
    const at = (e) => { const r = c.getBoundingClientRect(); return [e.clientX - r.left, e.clientY - r.top]; };
    const centerOf = () => { const v = [...this.pointers.values()]; return [(v[0][0] + v[1][0]) / 2, (v[0][1] + v[1][1]) / 2]; };
    const spread = () => { const v = [...this.pointers.values()]; return Math.hypot(v[0][0] - v[1][0], v[0][1] - v[1][1]); };
    c.addEventListener('pointerdown', (e) => {
      c.setPointerCapture(e.pointerId);
      this.stopFling();
      this.pointers.set(e.pointerId, at(e));
      if (this.pointers.size === 1) { this.travel = 0; this.start = at(e); this.track = [[performance.now(), ...at(e)]]; }
      if (this.pointers.size === 2) { this.travel = Infinity; this.pinch = { c: centerOf(), d: spread() }; }
    });
    c.addEventListener('pointermove', (e) => {
      if (!this.pointers.has(e.pointerId)) return;
      const p = at(e);
      const last = this.pointers.get(e.pointerId);
      this.pointers.set(e.pointerId, p);
      if (this.pointers.size === 2 && this.pinch) {
        const [cc, d] = [centerOf(), spread()];
        const f = d / this.pinch.d;
        if (Number.isFinite(f) && f > 0) this.change([cc[0] - this.pinch.c[0], cc[1] - this.pinch.c[1]], 1 / f, cc);
        this.pinch = { c: cc, d };
      } else if (this.pointers.size === 1) {
        this.travel = Math.max(this.travel, Math.hypot(p[0] - this.start[0], p[1] - this.start[1]));
        this.track.push([performance.now(), ...p]);
        if (this.track.length > 8) this.track.shift();
        this.change([p[0] - last[0], p[1] - last[1]], 1, [0, 0]);
      }
    });
    const up = (e) => {
      if (!this.pointers.has(e.pointerId)) return;
      const p = at(e);
      this.pointers.delete(e.pointerId);
      if (this.pointers.size) { this.pinch = null; return; }
      this.resample(true);
      if (e.type === 'pointercancel') return;
      if (this.travel < TAP_SLOP) { this.tap(p); return; }
      if (!Number.isFinite(this.travel)) return;
      const now = performance.now();
      const recent = this.track.filter((t) => now - t[0] < 120);
      if (recent.length >= 2) {
        const [a, b] = [recent[0], recent[recent.length - 1]];
        const dt = (b[0] - a[0]) / 1000;
        const v = [(b[1] - a[1]) / dt, (b[2] - a[2]) / dt];
        if (dt > 0 && Math.hypot(v[0], v[1]) > 300) this.startFling(v);
      }
    };
    c.addEventListener('pointerup', up);
    c.addEventListener('pointercancel', up);
    c.addEventListener('pointermove', (e) => { if (!this.pointers.size) this.hover = at(e); });
    c.addEventListener('wheel', (e) => {
      e.preventDefault();
      const dy = e.deltaMode === 0 ? e.deltaY / 100 : e.deltaMode === 1 ? e.deltaY / 3 : e.deltaY;
      this.stopFling();
      this.change([0, 0], 1.15 ** dy, at(e));
      clearTimeout(this.wheelTimer);
      this.wheelTimer = setTimeout(() => this.resample(true), 250);
    }, { passive: false });
  }

  tap(p) {
    const now = performance.now();
    if (now - this.lastTap < 350) { this.lastTap = 0; this.reset(); return; }
    this.lastTap = now;
    const f = this.frame;
    if (!f) return;
    const a = this.axes;
    const pin = pinNear(a, f, p[0], p[1], 24);
    if (pin >= 0) a.pins.splice(pin, 1);
    else {
      const hit = pick(a, f, p[0], p[1], 40);
      if (hit) { this.status.textContent = readout(hit); a.pins.push(hit); }
      else {
        const [x, y] = unproject(f, p[0], p[1]);
        this.status.textContent = `x = ${num(x)}    y = ${num(y)}`;
      }
    }
    this.redraw();
  }

  /** Pan by pixel delta `d`, then zoom by `factor` about pixel `at`. */
  change(d, factor, at) {
    const f = this.frame;
    if (!f) return;
    const a = this.axes;
    const [x0, y0, x1, y1] = f.rect;
    const [w, h] = [x1 - x0, y1 - y0];
    const [xr, yr] = shownRanges(a, f, f.rect);
    a.xlim = adjust(xr, -d[0] / w, factor, (at[0] - x0) / w);
    a.ylim = adjust(yr, d[1] / h, factor, 1 - (at[1] - y0) / h);
    this.redraw();
    this.resample(false);
  }

  /** Tell the owner the view changed: at once when a gesture ends, else at most every 120 ms. */
  resample(now) {
    const t = performance.now();
    if (!now && t - this.resampled < 120) return;
    this.resampled = now ? 0 : t;
    this.changed();
  }

  startFling(v0) {
    this.stopFling();
    let v = v0;
    let last = null;
    const step = (t) => {
      const dt = last === null ? 0 : (t - last) / 1000;
      last = t;
      const k = Math.exp(-5 * dt);
      const was = v;
      v = [v[0] * k, v[1] * k];
      this.change([was[0] * dt, was[1] * dt], 1, [0, 0]);
      if (Math.hypot(was[0], was[1]) < 40) { this.fling = null; this.resample(true); return; }
      this.fling = requestAnimationFrame(step);
    };
    this.fling = requestAnimationFrame(step);
  }

  stopFling() {
    if (this.fling) { cancelAnimationFrame(this.fling); this.fling = null; this.resample(true); }
  }

  reset() {
    this.stopFling();
    const a = this.axes;
    a.xlim = this.graph ? [-10, 10] : null;
    a.ylim = this.graph ? [-6, 6] : null;
    a.pins = [];
    this.redraw();
    this.resample(true);
  }

  /** The x range in view (graphs sample around it). */
  range() {
    if (this.frame) return shownRanges(this.axes, this.frame, this.frame.rect)[0];
    return this.axes.xlim ?? [-10, 10];
  }

  setTheme(name) { this.themeName = name; this.redraw(); }

  redraw() {
    if (this.queued) return;
    this.queued = true;
    requestAnimationFrame(() => { this.queued = false; this.paint(); });
  }

  paint() {
    const { clientWidth: w, clientHeight: h } = this.area;
    if (w < 4 || h < 4) return;
    const scale = window.devicePixelRatio || 1;
    const c = this.canvas;
    if (c.width !== Math.round(w * scale) || c.height !== Math.round(h * scale)) {
      c.width = Math.round(w * scale);
      c.height = Math.round(h * scale);
    }
    const ctx = c.getContext('2d');
    ctx.setTransform(scale, 0, 0, scale, 0, 0);
    this.frame = draw(ctx, this.axes, THEMES[this.themeName], w, h);
  }

  /** The chart in the light theme at 1600 x 1000: "png" -> Blob, "svg" -> Blob. */
  async exportChart(format) {
    const [w, h] = [1600, 1000];
    const theme = THEMES.light;
    if (format === 'svg') {
      const s = new SvgContext(w, h);
      draw(s, this.axes, theme, w, h, 18);
      return new Blob([s.toString()], { type: 'image/svg+xml' });
    }
    const c = document.createElement('canvas');
    c.width = w;
    c.height = h;
    draw(c.getContext('2d'), this.axes, theme, w, h, 18);
    return new Promise((resolve) => c.toBlob(resolve, 'image/png'));
  }
}
