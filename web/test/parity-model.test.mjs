// The web port against what the desktop computed (web/test/parity-model.json, from
// `cargo test export_model_parity -- --ignored`): statistics, fits, key points, axes, parsing, error messages.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import * as L from '../app/logic.js';

const ref = JSON.parse(readFileSync(new URL('parity-model.json', import.meta.url)));
const eq = assert.deepEqual;
const NaNify = (v) => v.map((x) => (x === null ? NaN : x === 'inf' ? Infinity : x === '-inf' ? -Infinity : x));
const near = (a, b, what, tol = 1e-11) => {
  if (a === null || b === null || !Number.isFinite(a) || !Number.isFinite(b)) {
    assert.ok((a === null || !Number.isFinite(a)) && (b === null || !Number.isFinite(b)), `${what}: desktop ${b}, web ${a}`);
    return;
  }
  assert.ok(a === b || Math.abs(a - b) <= tol * Math.max(Math.abs(a), Math.abs(b)), `${what}: desktop ${b}, web ${a}`);
};
const nearAll = (a, b, what, tol) => { eq(a.length, b.length, `${what}: lengths`); a.forEach((v, i) => near(v, b[i], `${what}[${i}]`, tol)); };

test('numbers as shown', () => {
  for (const [v, want] of ref.num) eq(L.num(v), want, String(v));
  for (const [v, want] of ref.fmtNum) eq(L.fmtNum(v), want, String(v));
});

test('cells, names and pasted text', () => {
  const show = (v) => (v === null ? 'none' : Number.isNaN(v) ? 'nan' : v === Infinity ? 'inf' : v === -Infinity ? '-inf' : v);
  for (const [s, plain, comma] of ref.toNum) {
    eq(show(L.toNum(s, false)), plain, `${s} plain`);
    eq(show(L.toNum(s, true)), comma, `${s} comma`);
  }
  for (const [s, want] of ref.validName) eq(L.validName(s), want, s);
  for (const [s, grid, head, body] of ref.parseText) {
    const p = L.parseText(s);
    eq(p, grid, JSON.stringify(s));
    eq(L.splitHeader(p), [head, body], JSON.stringify(s));
  }
});

test('keypad templates', () => {
  for (const [t, sel, out, slot] of ref.expand) eq(L.expand(t, sel), [out, slot], `${t} on ${sel}`);
});

test('what can be graphed, and why not', () => {
  for (const [s, ok, err] of ref.expression) {
    let got = null;
    let why = null;
    try { got = L.expression(s); } catch (e) { why = e.message; }
    // the one sentence that mentions Advanced, which the web doesn't have
    const want = err?.replace('Equations, inequalities and scripts belong in Advanced.', "Equations and inequalities can't be graphed here.") ?? null;
    eq([got, why], [ok, want], s);
  }
});

test('axes', () => {
  for (const [a, b, n, values, labels] of ref.ticks) {
    const t = L.ticks(a, b, n);
    eq(t.map((p) => p[1]), labels, `ticks ${a}..${b} / ${n}`);
    nearAll(t.map((p) => p[0]), values, `ticks ${a}..${b}`);
  }
  for (const [x, y, w, h, ox, oy] of ref.equalRanges) {
    const [rx, ry] = L.equalRanges(x, y, w, h);
    nearAll(rx, ox, 'equal x'); nearAll(ry, oy, 'equal y');
  }
  for (const [r, s, f, a, o] of ref.adjust) nearAll(L.adjust(r, s === 'inf' ? Infinity : s, f, a), o, `adjust ${r} ${s} ${f} ${a}`);
});

test('plot text markup', () => {
  for (const [s, runs] of ref.mathRuns) eq(L.mathRuns(s).map((r) => [r.text, r.scale, r.shift, r.stack]), runs, s);
});

test('Data tab: charts, statistics, correlation, fits', () => {
  for (const [gi, d] of ref.data.entries()) {
    eq(L.tableColumns(d.grid).named, d.table.named, `grid ${gi} columns`);
    eq(L.tableColumns(d.grid).unnamed, d.table.unnamed, `grid ${gi} unnamed`);
    eq(L.tableColumns(d.grid).misaligned, d.table.misaligned, `grid ${gi} misaligned`);
    for (const v of d.views) {
      const at = `grid ${gi} x=${v.x} y=${v.y}`;
      const p = L.pairs(d.grid, v.x, v.y);
      eq(p, v.pairs, `${at} pairs`);
      near(L.pearson(p), v.pearson, `${at} pearson`);
      near(L.spearman(p), v.spearman, `${at} spearman`);
      near(L.covariance(p), v.cov, `${at} cov`);
      let ls;
      try { ls = L.leastSquares(p); } catch (e) { ls = e.message; }
      if (typeof v.leastSquares === 'string') eq(ls, v.leastSquares, `${at} line`);
      else nearAll(ls, v.leastSquares, `${at} line`);
      v.fits.forEach((want, i) => {
        const kind = i + 1;
        let f;
        try { f = L.fit(p, kind); } catch (e) { eq(e.message, want.error, `${at} fit ${kind}`); return; }
        assert.equal(want.error, undefined, `${at} fit ${kind}: desktop refused with ${want.error}`);
        eq(L.fitEquation(f), want.equation, `${at} fit ${kind}`);
        near(f.r2, want.r2, `${at} fit ${kind} r²`, 1e-9);
        nearAll(p.map((q) => L.fitAt(f, q[0])), want.at, `${at} fit ${kind} curve`, 1e-9);
      });
      v.charts.forEach((want, c) => {
        const got = L.dataChart(d.grid, c, v.x, v.y);
        const what = `${at} chart ${c}`;
        eq([got.xlabel, got.ylabel, got.skipped], [want.xlabel, want.ylabel, want.skipped], what);
        eq(got.series.kind, want.series.kind, what);
        nearAll(Array.from(got.series.x, (x) => (Number.isFinite(x) ? x : null)), want.series.x, `${what} x`);
        nearAll(Array.from(got.series.y, (x) => (Number.isFinite(x) ? x : null)), want.series.y, `${what} y`);
        if (want.series.kind === 'bar') near(got.series.width, want.series.width, `${what} width`);
        else eq([got.series.line, got.series.marker], [want.series.line, want.series.marker], what);
        const s = got.stats;
        eq([s.count, s.skipped], [want.count, want.statSkipped], what);
        for (const k of ['mean', 'median', 'min', 'max', 'std', 'sum', 'q1', 'q3', 'popStd']) near(s[k], want[k], `${what} ${k}`);
        eq(s.modes, want.modes, `${what} modes`);
      });
    }
  }
});

test('key points of real engine samples', () => {
  const near9 = (a, b, what) => { eq(a.length, b.length, `${what}: ${JSON.stringify(a)} vs desktop ${JSON.stringify(b)}`); a.forEach((p, i) => { near(p[0], b[i][0], `${what} x`, 1e-9); near(p[1], b[i][1], `${what} y`, 1e-9); }); };
  for (const c of ref.keyPoints) {
    const [x, y] = [NaNify(c.x), NaNify(c.y)];
    const [bx, by] = L.breakJumps(x, y);
    eq([bx.filter(Number.isNaN).length, by.length], [c.breaks, c.points], `${c.expr} breaks`);
    const k = L.analyze(x, y);
    if (c.keys === null) { eq(k, null, c.expr); continue; }
    assert.ok(k, `${c.expr}: the desktop found key points`);
    for (const name of ['max', 'min', 'roots', 'inflection']) near9(k[name], c.keys[name], `${c.expr} ${name}`);
    near9(k.yInt ? [k.yInt] : [], c.keys.yInt ? [c.keys.yInt] : [], `${c.expr} y-int`);
    near9(k.vertex ? [k.vertex] : [], c.keys.vertex ? [c.keys.vertex] : [], `${c.expr} vertex`);
    const sym = k.symmetry;
    const text = sym.kind === 'axis' ? `x = ${L.num(sym.h)}` : sym.kind === 'point' ? `point (${L.num(sym.h)}, ${L.num(sym.v)})` : sym.kind;
    eq(text, c.keys.symmetry, `${c.expr} symmetry`);
  }
});
