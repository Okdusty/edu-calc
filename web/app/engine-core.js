// The calculator engine (libqalculate and src/qalc/shim.cpp in WebAssembly) with the same calls as
// the desktop's src/qalc/mod.rs. Used by the Web Worker, and directly by the Node smoke test.
import { UNITS_ERROR, breakJumps, explain, expression } from './logic.js';

const TIMEOUT_MS = 5000;
// the desktop's 300, for a slower engine
const PREVIEW_MS = 1000;
const PURE = 2;
export { DEFAULT_OPTIONS } from './engine-core-options.js';

const list = (s) => s.split('\x1e').filter((x) => x !== '');
const literal = (values) => `[${values.map((x) => x.toExponential().toUpperCase().replace('E+', 'E')).join(', ')}]`;
const linspace = (a, b, n) => Array.from({ length: n }, (_, i) => a + ((b - a) * i) / (Math.max(n, 2) - 1));

/** The engine itself broke, not a calculation (a trap, a stack overflow): its memory can't be trusted. */
export const broken = (e) => e.constructor !== Error;

export class Engine {
  constructor(module) {
    this.m = module;
    this.catalogItems = null;
    this.shared = new Set();
  }

  /** Take a string the shim allocated. */
  take(ptr) {
    if (!ptr) return null;
    const s = this.m.UTF8ToString(ptr);
    this.m._qs_free(ptr);
    return s;
  }

  applyOptions(o) {
    const ix = (v, all) => Math.max(all.indexOf(v), 0);
    this.m._qs_options(ix(o.angle, ['rad', 'deg', 'gra']), ix(o.approx, ['try', 'exact', 'approx']), o.precision,
      ix(o.fraction, ['auto', 'decimal', 'fraction', 'mixed']), ix(o.notation, ['auto', 'normal', 'sci', 'eng']), o.base, o.inputBase, o.twos ? 1 : 0, o.bits);
  }

  rawCalc(expr, ms, mode) {
    const r = this.take(this.m.ccall('qs_calc', 'number', ['string', 'number', 'number'], [expr, ms, mode]));
    if (r === null) throw new Error('qalculate: out of memory');
    const f = r.split('\x1f');
    if (f[0] !== '') throw new Error(list(f[0]).join('; '));
    if (f.length !== 12) throw new Error('qalculate: malformed reply');
    return { input: f[1], result: f[2], approx: f[3], freeVars: list(f[7]), messages: list(f[8]), units: f[11] === '1' };
  }

  /** Plain answer of an expression; never changes ans, variables or files. */
  classroomEval(expr, options) {
    this.applyOptions(options);
    const a = this.rawCalc(expr, TIMEOUT_MS, PURE);
    return { input: a.input, result: a.result, approx: a.approx, freeVars: a.freeVars };
  }

  sample(expr, xs) {
    const m = this.m;
    const n = xs.length;
    const xp = m._malloc(n * 8);
    const out = m._malloc(n * 8);
    try {
      m.HEAPF64.set(xs, xp / 8);
      m.HEAPF64.fill(NaN, out / 8, out / 8 + n);
      const err = this.take(m.ccall('qs_sample', 'number', ['string', 'string', 'string', 'number', 'number', 'number', 'number', 'number'], [expr, 'x', '', xp, xp, n, out, TIMEOUT_MS]));
      if (err !== null) throw new Error(err);
      // memory may have grown during the call: read through the current view
      return Array.from(m.HEAPF64.subarray(out / 8, out / 8 + n));
    } finally {
      m._free(xp);
      m._free(out);
    }
  }

  /** Explicit y = f(x), 2001 samples of [lo, hi] (the graph asks for twice the view). */
  classroomCurve(expr, options, lo, hi) {
    this.applyOptions(options);
    const a = this.rawCalc(expr, PREVIEW_MS, PURE);
    const ignored = a.messages.find((m) => m.includes('ignored'));
    if (ignored) throw new Error(ignored);
    const others = a.freeVars.filter((v) => v !== 'x').map((v) => `“${v}”`);
    if (others.length) throw new Error(`Only x can vary in a graph, but this also uses ${others.join(', ')}. Replace it with a number, for example y = 2x + 1.`);
    const xs = linspace(lo, hi, 2001);
    const ys = this.sample(expr, xs);
    // a unit left over makes no sample a number (sin(x + 30°) keeps rad inside sin, and is fine)
    if (a.units && ys.every(Number.isNaN)) throw new Error(UNITS_ERROR);
    return breakJumps(xs, ys);
  }

  /** The curve for a graph row's text, or the reason in a student's words. */
  graphRow(text, options, lo, hi) {
    const expr = expression(text);
    try {
      return this.classroomCurve(expr, options, lo, hi);
    } catch (e) {
      if (broken(e)) throw e;
      throw new Error(explain(expr, e.message, this.functionNames(), (f) => this.catalog().find((i) => i.kind === 'f' && i.name === f)?.signature));
    }
  }

  /** The table's columns as calculator row vectors; returns the names taken (built-in names are refused). */
  shareColumns(cols, gone) {
    for (const name of gone) this.m.ccall('qs_unset_var', null, ['string'], [name]);
    const taken = [];
    for (const [name, values] of cols) {
      if (!values.length || values.length > 10000) { this.m.ccall('qs_unset_var', null, ['string'], [name]); continue; }
      const err = this.take(this.m.ccall('qs_set_var', 'number', ['string', 'string'], [name, literal(values)]));
      if (err === null) taken.push(name);
    }
    return taken;
  }

  catalog() {
    if (!this.catalogItems) {
      const s = this.take(this.m._qs_catalog()) ?? '';
      this.catalogItems = list(s).map((r) => r.split('\x1f')).filter((f) => f.length === 5).map(([kind, name, insert, signature, desc]) => ({ kind, name, insert, signature, desc }));
    }
    return this.catalogItems;
  }

  functionNames() {
    if (!this.names) this.names = new Set(this.catalog().filter((i) => i.kind === 'f').map((i) => i.name));
    return this.names;
  }
}
