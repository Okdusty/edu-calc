// The chart: the desktop's src/plot/render.rs for the Simple tabs' 2D axes (layout, ticks, key points,
// pins), drawn on any 2D context. SvgContext is one, for exports.
import { MINUS, analyze, equalRanges, shown as showNum, ticks, grow } from './logic.js';

const FONT = '"DejaVu Sans", "Segoe UI", "Liberation Sans", system-ui, sans-serif';
export const THEMES = {
  light: {
    bg: [255, 255, 255], fg: [30, 30, 30], grid: [220, 220, 220], fontSize: 14,
    palette: [[0, 114, 189], [217, 83, 25], [237, 177, 32], [126, 47, 142], [119, 172, 48], [77, 190, 238], [162, 20, 47]],
  },
  dark: {
    bg: [30, 30, 34], fg: [220, 220, 220], grid: [65, 65, 72], fontSize: 14,
    palette: [[77, 166, 255], [255, 120, 70], [250, 200, 60], [190, 120, 230], [140, 210, 80], [100, 220, 240], [240, 80, 110]],
  },
};

const css = (c, a = 1) => (a === 1 ? `rgb(${c[0]},${c[1]},${c[2]})` : `rgba(${c[0]},${c[1]},${c[2]},${a})`);
const lerp = (a, b, t) => a.map((v, k) => Math.round(v + (b[k] - v) * t));

/** x, y and the ranges an axes wants before axis equal: its limits, else its data with 5 % padding. */
export function axesBounds(a) {
  const ext = [[Infinity, -Infinity], [Infinity, -Infinity]];
  const eat = (k, v) => { if (Number.isFinite(v)) { ext[k][0] = Math.min(ext[k][0], v); ext[k][1] = Math.max(ext[k][1], v); } };
  if (a.origin) { eat(0, 0); eat(1, 0); }
  for (const s of a.series) {
    if (s.kind === 'bar') {
      s.x.forEach((v) => { eat(0, v - s.width / 2); eat(0, v + s.width / 2); });
      s.y.forEach((v) => eat(1, v));
      eat(1, 0);
    } else {
      s.x.forEach((v) => eat(0, v));
      s.y.forEach((v) => eat(1, v));
    }
  }
  return [a.xlim, a.ylim].map((lim, k) => {
    if (lim) return lim;
    const [lo, hi] = ext[k];
    if (lo > hi) return [0, 1];
    if (lo === hi) return [lo - 1, hi + 1];
    const p = (hi - lo) * 0.05;
    return [lo - p, hi + p];
  });
}

/** What the axes shows in plot area `rect`: its limits, else the frame's, widened like axis equal. */
export function shownRanges(a, frame, rect) {
  const xr = a.xlim ?? frame.bounds[0];
  const yr = a.ylim ?? frame.bounds[1];
  return a.equal ? equalRanges(xr, yr, rect[2] - rect[0], rect[3] - rect[1]) : [xr, yr];
}

const overlaps = (p, q) => p.x0 < q.x1 && q.x0 < p.x1 && p.y0 < q.y1 && q.y0 < p.y1;
const inside = (p, q) => p.x0 >= q.x0 && p.x1 <= q.x1 && p.y0 >= q.y0 && p.y1 <= q.y1;
const roundRect = (r) => ({ x0: Math.round(r.x0), y0: Math.round(r.y0), x1: Math.round(r.x1), y1: Math.round(r.y1) });

/** Draw the axes on `ctx` in a w x h area; returns the frame: where the plot landed and its ranges. */
export function draw(ctx, a, theme, w, h, fontSize = theme.fontSize * grow(w, h)) {
  const fs = fontSize;
  const k = fs / 14;
  const mix = (t) => lerp(theme.bg, theme.fg, t);
  const color = (i, s) => s.color ?? theme.palette[i % theme.palette.length];
  const font = (scale) => `${fs * scale}px ${FONT}`;
  const textW = (s, f) => { ctx.font = f; return ctx.measureText(s).width; };
  const text = (s, x, y, f, c, align, baseline) => {
    ctx.font = f; ctx.fillStyle = css(c); ctx.textAlign = align; ctx.textBaseline = baseline;
    ctx.fillText(s, x, y);
  };
  const box = (r, c, alpha = 1) => { ctx.fillStyle = css(c, alpha); ctx.fillRect(r.x0, r.y0, r.x1 - r.x0, r.y1 - r.y0); };
  const hline = (x0, x1, y, c) => { const yy = Math.round(y); ctx.fillStyle = css(c); ctx.fillRect(Math.round(x0), yy, Math.round(x1) - Math.round(x0) + 1, 1); };
  const vline = (x, y0, y1, c) => { const xx = Math.round(x); ctx.fillStyle = css(c); ctx.fillRect(xx, Math.round(y0), 1, Math.round(y1) - Math.round(y0) + 1); };
  const outline = (r, c) => {
    const [x1, y1] = [r.x1 - 1, r.y1 - 1];
    hline(r.x0, x1, r.y0, c); hline(r.x0, x1, y1, c); vline(r.x0, r.y0, y1, c); vline(x1, r.y0, y1, c);
  };
  const lw = (width) => Math.max(1, width * 1.33 * k);

  ctx.fillStyle = css(theme.bg);
  ctx.fillRect(0, 0, w, h);

  let [bx, by] = axesBounds(a);
  const tickFont = font(0.88);
  const tl = Math.round(0.35 * fs);
  const layout = (rx, ry) => {
    const top = fs * 0.9;
    const bottom = h - fs * (a.xlabel ? 3.4 : 2) - tl;
    const yt = ticks(ry[0], ry[1], Math.max(Math.floor((bottom - top) / (fs * 2.6)), 2));
    const yw = yt.reduce((m, t) => Math.max(m, textW(t[1], tickFont)), 0);
    const left = fs * (a.ylabel ? 2.2 : 0.8) + yw + tl + 0.35 * fs;
    const right = w - fs * 1.4;
    let n = Math.max(Math.floor((right - left) / (fs * 3.5)), 2);
    let xt = ticks(rx[0], rx[1], n);
    while (n > 2 && xt.reduce((s, t) => s + textW(t[1], tickFont) + fs * 1.2, 0) > right - left) {
      n -= 1;
      xt = ticks(rx[0], rx[1], n);
    }
    return { plot: { x0: Math.round(left), y0: Math.round(top), x1: Math.round(right), y1: Math.round(bottom) }, xt, yt };
  };
  let lay = layout(bx, by);
  const pw = () => lay.plot.x1 - lay.plot.x0;
  const ph = () => lay.plot.y1 - lay.plot.y0;
  if (a.equal && pw() > 0 && ph() > 0) {
    [bx, by] = equalRanges(bx, by, pw(), ph());
    lay = layout(bx, by);
  }
  const { plot, xt, yt } = lay;
  const frame = { rect: [plot.x0, plot.y0, plot.x1, plot.y1], bounds: [bx, by] };
  if (pw() < 4 || ph() < 4) return frame;
  const sx = (v) => plot.x0 + ((v - bx[0]) / (bx[1] - bx[0])) * (plot.x1 - plot.x0);
  const sy = (v) => plot.y1 + ((v - by[0]) / (by[1] - by[0])) * (plot.y0 - plot.y1);

  if (a.grid) {
    xt.forEach(([v]) => vline(sx(v), plot.y0, plot.y1 - 1, theme.grid));
    yt.forEach(([v]) => hline(plot.x0, plot.x1 - 1, sy(v), theme.grid));
  }
  if (a.origin) {
    if (bx[0] <= 0 && bx[1] >= 0) vline(sx(0), plot.y0, plot.y1, theme.fg);
    if (by[0] <= 0 && by[1] >= 0) hline(plot.x0, plot.x1, sy(0), theme.fg);
  }

  ctx.save();
  ctx.beginPath();
  ctx.rect(plot.x0, plot.y0, plot.x1 - plot.x0, plot.y1 - plot.y0);
  ctx.clip();
  a.series.forEach((s, i) => {
    const c = color(i, s);
    if (s.kind === 'bar') {
      const base = sy(0);
      s.x.forEach((x, j) => {
        const [x0, x1, top] = [Math.round(sx(x - s.width / 2)), Math.round(sx(x + s.width / 2)), Math.round(sy(s.y[j]))];
        if (![x0, x1, top].every(Number.isFinite)) return;
        const r = { x0: Math.min(x0, x1), x1: Math.max(x0, x1), y0: Math.min(top, Math.round(base)), y1: Math.max(top, Math.round(base)) };
        box(r, c);
        if (r.x1 - r.x0 > 5) outline(r, theme.bg);
      });
      return;
    }
    ctx.strokeStyle = css(c);
    if (s.line !== false) {
      ctx.lineWidth = lw(s.width);
      ctx.lineJoin = 'round';
      ctx.setLineDash([]);
      ctx.beginPath();
      let pen = false;
      for (let j = 0; j < s.x.length; j++) {
        const [x, y] = [sx(s.x[j]), sy(s.y[j])];
        if (!Number.isFinite(x) || !Number.isFinite(y)) { pen = false; continue; }
        // far beyond the plot: clamped, which keeps the segment's direction close enough and the path finite
        const yy = Math.min(Math.max(y, -1e6), 1e6);
        if (pen) ctx.lineTo(x, yy); else ctx.moveTo(x, yy);
        pen = true;
      }
      ctx.stroke();
    }
    if (s.marker) {
      const r = (1.5 + 1.5 * s.width) * k;
      ctx.lineWidth = lw(s.width);
      for (let j = 0; j < s.x.length; j++) {
        const [x, y] = [sx(s.x[j]), sy(s.y[j])];
        if (!Number.isFinite(x) || !Number.isFinite(y)) continue;
        ctx.beginPath();
        ctx.arc(x, y, r, 0, Math.PI * 2);
        ctx.stroke();
      }
    }
  });
  if (a.keypoints) drawKeyPoints();
  ctx.restore();
  outline(plot, mix(0.6));
  drawPins();

  const axis = mix(0.6);
  const tickColor = mix(0.9);
  for (const [v, s] of xt) {
    const x = sx(v);
    vline(x, plot.y1, plot.y1 + tl, axis);
    text(s, Math.round(x), plot.y1 + tl + 0.3 * fs, tickFont, tickColor, 'center', 'top');
  }
  for (const [v, s] of yt) {
    const y = sy(v);
    hline(plot.x0 - tl, plot.x0, y, axis);
    text(s, plot.x0 - tl - 0.35 * fs, Math.round(y), tickFont, tickColor, 'right', 'middle');
  }
  const mid = [(plot.x0 + plot.x1) / 2, (plot.y0 + plot.y1) / 2];
  if (a.xlabel) text(a.xlabel, mid[0], h - 0.5 * fs, font(1), theme.fg, 'center', 'bottom');
  if (a.ylabel) {
    ctx.save();
    ctx.translate(1.1 * fs, mid[1]);
    ctx.rotate(-Math.PI / 2);
    text(a.ylabel, 0, 0, font(1), theme.fg, 'center', 'middle');
    ctx.restore();
  }
  return frame;

  function drawPins() {
    const f = font(0.85);
    const [pad, lh, off] = [0.45 * fs, 0.95 * fs, 0.5 * fs];
    for (const p of a.pins) {
      const q = [sx(p[0]), sy(p[1])];
      if (!(q[0] >= plot.x0 && q[0] <= plot.x1 && q[1] >= plot.y0 && q[1] <= plot.y1)) continue;
      const r = 3 * k;
      dot(q, r + 1.5 * k, theme.bg);
      dot(q, r, theme.fg);
      const lines = ['x', 'y'].map((name, j) => `${name}: ${showNum(p[j])}`);
      const bw = Math.max(...lines.map((l) => textW(l, f))) + 2 * pad;
      const bh = lh * 2 + 1.2 * pad;
      let x = q[0] + off + bw > plot.x1 ? q[0] - off - bw : q[0] + off;
      let y = q[1] - off - bh < plot.y0 ? q[1] + off : q[1] - off - bh;
      x = Math.max(Math.min(x, plot.x1 - bw), plot.x0);
      y = Math.max(Math.min(y, plot.y1 - bh), plot.y0);
      const b = roundRect({ x0: x, y0: y, x1: x + bw, y1: y + bh });
      box(b, theme.bg, 0.9);
      outline(b, mix(0.45));
      lines.forEach((l, j) => text(l, b.x0 + pad, b.y0 + 0.6 * pad + lh * (j + 0.5), f, theme.fg, 'left', 'middle'));
    }
  }

  function dot(q, r, c) {
    ctx.fillStyle = css(c);
    ctx.beginPath();
    ctx.arc(q[0], q[1], r, 0, Math.PI * 2);
    ctx.fill();
  }

  function polygon(q, shape, s) {
    ctx.beginPath();
    shape.forEach(([dx, dy], i) => { if (i) ctx.lineTo(q[0] + dx * s, q[1] + dy * s); else ctx.moveTo(q[0] + dx * s, q[1] + dy * s); });
    ctx.closePath();
  }

  function keyMarker(q, kind, c) {
    const [r, halo] = [3.5 * k, 1.5 * k];
    const width = Math.max(1, Math.round(1.5 * k));
    if (kind === 'root' || kind === 'vertex') {
      const rr = kind === 'vertex' ? 1.6 * r : r;
      dot(q, rr + halo, theme.bg);
      if (kind === 'vertex') { dot(q, rr, c); dot(q, rr * 0.45, theme.bg); return; }
      ctx.strokeStyle = css(c); ctx.lineWidth = width; ctx.setLineDash([]);
      ctx.beginPath(); ctx.arc(q[0], q[1], rr, 0, Math.PI * 2); ctx.stroke();
      return;
    }
    const shape = {
      max: [[0, -1.3], [1.15, 0.7], [-1.15, 0.7]],
      min: [[0, 1.3], [1.15, -0.7], [-1.15, -0.7]],
      inflection: [[0, -1.35], [1.1, 0], [0, 1.35], [-1.1, 0]],
      'y-int': [[-0.95, -0.95], [0.95, -0.95], [0.95, 0.95], [-0.95, 0.95]],
    }[kind];
    ctx.fillStyle = css(theme.bg);
    polygon(q, shape, r + 1.6 * halo); ctx.fill();
    if (kind === 'max' || kind === 'min') { ctx.fillStyle = css(c); polygon(q, shape, r); ctx.fill(); return; }
    ctx.strokeStyle = css(c); ctx.lineWidth = width; ctx.setLineDash([]);
    polygon(q, shape, r); ctx.stroke();
  }

  function drawKeyPoints() {
    const contains = (q) => q[0] >= plot.x0 && q[0] <= plot.x1 && q[1] >= plot.y0 && q[1] <= plot.y1;
    const order = { vertex: 0, max: 1, min: 2, root: 3, 'y-int': 4, inflection: 5 };
    const pts = [];
    const axes = [];
    const notes = [];
    a.series.forEach((s, i) => {
      if (s.kind !== 'line') return;
      const kp = analyze(s.x, s.y);
      if (!kp) return;
      const c = color(i, s);
      const low = kp.vertex !== null && kp.min.some((m) => m[0] === kp.vertex[0] && m[1] === kp.vertex[1]);
      const groups = [
        ['vertex', kp.vertex ? [kp.vertex] : [], low], ['max', kp.max, false], ['min', kp.min, true],
        ['root', kp.roots, true], ['y-int', kp.yInt ? [kp.yInt] : [], false], ['inflection', kp.inflection, false],
      ];
      for (const [kind, ps, below] of groups) {
        for (const p of ps) {
          const q = [sx(p[0]), sy(p[1])];
          // one marker per spot: the vertex beats its min, a root at the origin its intercept
          if (contains(q) && !pts.some((o) => Math.abs(o.q[0] - q[0]) < 2 * k && Math.abs(o.q[1] - q[1]) < 2 * k)) {
            pts.push({ q, kind, text: `${kind} (${showNum(p[0])}, ${showNum(p[1])})`, c, below });
          }
        }
      }
      const sym = kp.symmetry;
      if (sym.kind === 'axis' && contains([sx(sym.h), plot.y0])) axes.push([sx(sym.h), `x = ${showNum(sym.h)}`, c]);
      if (sym.kind === 'even') notes.push([`even: f(${MINUS}x) = f(x)`, c]);
      if (sym.kind === 'odd') notes.push([`odd: f(${MINUS}x) = ${MINUS}f(x)`, c]);
      if (sym.kind === 'point') notes.push([`symmetric about (${showNum(sym.h)}, ${showNum(sym.v)})`, c]);
    });
    pts.sort((p, q) => order[p.kind] - order[q.kind]);
    pts.length = Math.min(pts.length, 60);
    for (const [x, , c] of axes) {
      ctx.strokeStyle = css(c); ctx.lineWidth = 1; ctx.setLineDash([5.5, 2.9]);
      ctx.beginPath(); ctx.moveTo(x, plot.y0); ctx.lineTo(x, plot.y1); ctx.stroke();
      ctx.setLineDash([]);
    }
    for (const p of pts) keyMarker(p.q, p.kind, p.c);
    const f = font(0.8);
    const [pad, lh, gap] = [0.3 * fs, 1.05 * fs, 0.6 * fs];
    const area = { x0: plot.x0 + 1, y0: plot.y0 + 1, x1: plot.x1 - 1, y1: plot.y1 - 1 };
    const r = 6 * k;
    const taken = pts.map(({ q }) => ({ x0: q[0] - r, y0: q[1] - r, x1: q[0] + r, y1: q[1] + r }));
    let y = area.y0 + 0.3 * fs;
    for (const [note, c] of notes) {
      const nw = textW(note, f) + 2 * pad;
      const b = roundRect({ x0: area.x0 + 0.3 * fs, y0: y, x1: area.x0 + 0.3 * fs + nw, y1: y + lh });
      if (!inside(b, area)) break;
      box(b, theme.bg, 0.85);
      text(note, b.x0 + pad, (b.y0 + b.y1) / 2, f, c, 'left', 'middle');
      taken.push(b);
      y = b.y1 + 0.2 * fs;
    }
    const place = (label, cands) => {
      const lw_ = textW(label, f) + 2 * pad;
      for (const [cx, cy] of cands) {
        const b = roundRect({ x0: cx, y0: cy, x1: cx + lw_, y1: cy + lh });
        if (inside(b, area) && !taken.some((t) => overlaps(t, b))) {
          box(b, theme.bg, 0.85);
          text(label, b.x0 + pad, (b.y0 + b.y1) / 2, f, theme.fg, 'left', 'middle');
          taken.push(b);
          return true;
        }
      }
      return false;
    };
    for (const [x, label] of axes) {
      const lw_ = textW(label, f) + 2 * pad;
      const top = area.y0 + 0.3 * fs;
      place(label, [[x + 0.3 * fs, top], [x - 0.3 * fs - lw_, top], [x + 0.3 * fs, area.y1 - lh - 0.3 * fs]]);
    }
    let count = 0;
    for (const p of pts) {
      if (count === 12) break;
      const pw_ = textW(p.text, f) + 2 * pad;
      const [up, dn, md] = [p.q[1] - gap - lh, p.q[1] + gap, p.q[1] - lh / 2];
      const [l, c, rt] = [p.q[0] - gap - pw_, p.q[0] - pw_ / 2, p.q[0] + gap];
      const [first, second] = p.below ? [dn, up] : [up, dn];
      if (place(p.text, [[c, first], [rt, first], [l, first], [rt, md], [l, md], [c, second], [rt, second], [l, second]])) count++;
    }
  }
}

// ---- data cursor ----------------------------------------------------------------------------

export function project(f, p) {
  const [x0, y0, x1, y1] = f.rect;
  const [bx, by] = f.bounds;
  return [x0 + ((p[0] - bx[0]) / (bx[1] - bx[0])) * (x1 - x0), y1 + ((p[1] - by[0]) / (by[1] - by[0])) * (y0 - y1)];
}

export function unproject(f, px, py) {
  const [x0, y0, x1, y1] = f.rect;
  const [bx, by] = f.bounds;
  return [bx[0] + ((px - x0) / (x1 - x0)) * (bx[1] - bx[0]), by[0] + ((py - y1) / (y0 - y1)) * (by[1] - by[0])];
}

/** The nearest data point within `radius` px of (px, py), or null; a bar under the pointer wins. */
export function pick(a, f, px, py, radius) {
  const [x0, y0, x1, y1] = f.rect;
  const offX = Math.max(x0 - px, px - x1, 0);
  const offY = Math.max(y0 - py, py - y1, 0);
  if (Math.hypot(offX, offY) > radius) return null;
  const [x, y] = unproject(f, px, py);
  let best = null;
  for (const s of a.series) {
    if (s.kind === 'bar') {
      for (let i = 0; i < s.x.length; i++) {
        if (Math.abs(x - s.x[i]) <= s.width / 2 && y * (y - s.y[i]) <= 0 && Number.isFinite(s.y[i])) {
          if (!best || best.d > 0) best = { d: 0, point: [s.x[i], s.y[i]] };
          break;
        }
      }
    }
    for (let i = 0; i < s.x.length; i++) {
      if (!Number.isFinite(s.x[i]) || !Number.isFinite(s.y[i])) continue;
      const q = project(f, [s.x[i], s.y[i]]);
      const d = Math.hypot(q[0] - px, q[1] - py);
      if (!best || d < best.d) best = { d, point: [s.x[i], s.y[i]] };
    }
  }
  return best && best.d <= radius ? best.point : null;
}

/** Index of the pin within `radius` px of (px, py), nearest first, or -1. */
export function pinNear(a, f, px, py, radius) {
  let found = -1;
  let dist = Infinity;
  a.pins.forEach((p, i) => {
    const q = project(f, p);
    const d = Math.hypot(q[0] - px, q[1] - py);
    if (d <= radius && d < dist) { found = i; dist = d; }
  });
  return found;
}

// ---- SVG export -----------------------------------------------------------------------------

const measure = () => {
  const c = typeof OffscreenCanvas === 'function' ? new OffscreenCanvas(1, 1) : document.createElement('canvas');
  return c.getContext('2d');
};
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
const n2 = (v) => +v.toFixed(2);

/** The subset of a canvas context `draw` uses, writing SVG. */
export class SvgContext {
  constructor(w, h) {
    Object.assign(this, { w, h, out: [], stack: [], path: '', clips: 0, depth: 0, m: measure(), transform: '' });
    Object.assign(this, { font: '', fillStyle: '#000', strokeStyle: '#000', lineWidth: 1, textAlign: 'left', textBaseline: 'alphabetic', lineJoin: 'miter', dash: [] });
  }
  save() { this.stack.push({ font: this.font, fillStyle: this.fillStyle, strokeStyle: this.strokeStyle, lineWidth: this.lineWidth, textAlign: this.textAlign, textBaseline: this.textBaseline, dash: this.dash, transform: this.transform, depth: this.depth }); }
  restore() {
    const s = this.stack.pop();
    while (this.depth > s.depth) { this.out.push('</g>'); this.depth--; }
    Object.assign(this, s);
  }
  measureText(s) { this.m.font = this.font; return this.m.measureText(s); }
  setLineDash(d) { this.dash = d; }
  fillRect(x, y, w, h) { this.out.push(`<rect x="${n2(x)}" y="${n2(y)}" width="${n2(w)}" height="${n2(h)}" fill="${this.fillStyle}"/>`); }
  beginPath() { this.path = ''; }
  moveTo(x, y) { this.path += `M${n2(x)} ${n2(y)}`; }
  lineTo(x, y) { this.path += `L${n2(x)} ${n2(y)}`; }
  closePath() { this.path += 'Z'; }
  rect(x, y, w, h) { this.path += `M${n2(x)} ${n2(y)}h${n2(w)}v${n2(h)}h${n2(-w)}Z`; }
  arc(x, y, r) { this.path += `M${n2(x - r)} ${n2(y)}a${n2(r)} ${n2(r)} 0 1 0 ${n2(2 * r)} 0a${n2(r)} ${n2(r)} 0 1 0 ${n2(-2 * r)} 0Z`; }
  fill() { this.out.push(`<path d="${this.path}" fill="${this.fillStyle}"/>`); }
  stroke() {
    const dash = this.dash.length ? ` stroke-dasharray="${this.dash.join(' ')}"` : '';
    this.out.push(`<path d="${this.path}" fill="none" stroke="${this.strokeStyle}" stroke-width="${n2(this.lineWidth)}" stroke-linejoin="round"${dash}/>`);
  }
  clip() {
    const id = `c${this.clips++}`;
    this.out.push(`<clipPath id="${id}"><path d="${this.path}"/></clipPath><g clip-path="url(#${id})">`);
    this.depth++;
  }
  translate(x, y) { this.transform += `translate(${n2(x)} ${n2(y)})`; }
  rotate(rad) { this.transform += `rotate(${n2((rad * 180) / Math.PI)})`; }
  fillText(s, x, y) {
    const anchor = { center: 'middle', right: 'end' }[this.textAlign] ?? 'start';
    const base = { top: 'hanging', middle: 'central', bottom: 'text-after-edge' }[this.textBaseline] ?? 'auto';
    const size = parseFloat(this.font);
    const t = this.transform ? ` transform="${this.transform}"` : '';
    this.out.push(`<text x="${n2(x)}" y="${n2(y)}"${t} fill="${this.fillStyle}" font-size="${n2(size)}" font-family="DejaVu Sans, Segoe UI, sans-serif" text-anchor="${anchor}" dominant-baseline="${base}">${esc(s)}</text>`);
  }
  toString() { return `<svg xmlns="http://www.w3.org/2000/svg" width="${this.w}" height="${this.h}" viewBox="0 0 ${this.w} ${this.h}">${this.out.join('')}</svg>\n`; }
}
