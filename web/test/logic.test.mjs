// The Rust test suite's cases and expected values, against the JavaScript port.
import test from 'node:test';
import assert from 'node:assert/strict';
import * as L from '../app/logic.js';

const eq = assert.deepEqual;
const close = (a, b, tol = 1e-9) => assert.ok(Math.abs(a - b) <= tol, `${a} vs ${b}`);
const grid = (rows) => rows.map((r) => [...r]);
const err = (s) => { try { L.expression(s); } catch (e) { return e.message; } assert.fail(`no error for ${s}`); };

test('num', () => {
  eq(L.num(1.23456), '1.235'); eq(L.num(0.944), '0.944'); eq(L.num(-2.5), '-2.5');
  eq(L.num(1234567), '1.235e6'); eq(L.num(0.00001234), '1.234e-5'); eq(L.num(100), '100');
  eq(L.num(0), '0'); eq(L.num(-0), '0'); eq(L.num(NaN), 'NaN');
});

test('expressions', () => {
  eq(L.expression(' y = 2x + 1 '), '2x + 1'); eq(L.expression('x²'), 'x²');
  for (const s of ['x=2', 'x²+y²=1', 'y=x<2', 'y=', 'sin(□)', 'y=x; quit']) assert.throws(() => L.expression(s), undefined, s);
  for (const s of ['-x + 1', 'x**2', '2^-x', 'log(x, 2)', 'sin(cos(x))', 'x! / 2', 'e^(-x²)', '(x + 1)(x − 1)', '3.5x', '√(x)']) eq(L.expression(s), s);
});

test('syntax mistakes are explained', () => {
  assert.match(err('2x +'), /“\+” at the end/);
  assert.match(err('* x'), /“\*” at the start/);
  assert.match(err('x +* 2'), /“\+\*” is two operators/);
  assert.match(err('(^x)'), /right after “\(”/);
  assert.match(err('sin(x +)'), /right before “\)”/);
  eq(err('sin(x'), '“sin(x” is missing a closing “)”.');
  eq(err('sin(cos(x'), '“sin(cos(x” is missing 2 closing “)”.');
  assert.match(err('x + 1)'), /without a “\(”/);
  assert.match(err('sin()'), /“sin\(\)” has nothing inside/);
  assert.match(err('()'), /for example \(x \+ 1\)/);
  assert.match(err('2,5x'), /For decimals use a point/);
  assert.match(err('log(x,)'), /missing next to a comma/);
  assert.match(err('1.2.3x'), /“1\.2\.3”/);
  assert.match(err('x..2'), /“\.\.2” has more than one decimal point/);
});

test('calculator errors are explained', () => {
  const fns = new Set(['sin', 'cos', 'log']);
  const sig = (f) => (f === 'log' ? 'log(x, base)' : null);
  const ex = (t, e) => L.explain(t, e, fns, sig);
  assert.match(ex('abc', L.UNITS_ERROR), /“abc”/);
  assert.match(ex('2foo(x)', L.UNITS_ERROR), /“foo”/);
  eq(ex('sinx + 30°', L.UNITS_ERROR), L.UNITS_ERROR);
  assert.match(ex('x+', 'Misplaced operator(s) "+" ignored'), /^“\+” is in a place/);
  assert.match(ex('#', '"#" is not a valid variable/function/unit.'), /“#”/);
  assert.match(ex('log()', 'You need at least 1 argument (Value) in function log().'), /^log needs more values/);
  assert.match(ex('x', 'plot sampling timed out'), /too long/);
  eq(ex('x', 'something else'), 'something else');
});

test('keypad templates', () => {
  eq(L.expand('sin(□)', ''), ['sin(□)', 4]);
  eq(L.expand('7', '□'), ['7', null]);
  eq(L.expand('sin(□)', 'x+1'), ['sin(x+1)', null]);
  eq(L.expand('□²', 'x+1'), ['(x+1)²', null]);
  eq(L.expand('√(□)', '2'), ['√(2)', null]);
  eq(L.expand('1/□', '(a+b)'), ['1/(a+b)', null]);
  eq(L.expand('□^□', 'x'), ['x^□', 2]);
  eq(L.expand('□^□', '2x+1'), ['(2x+1)^□', 7]);
  eq(L.expand('integrate(□, □, □, x)', 'x²'), ['integrate(x², □, □, x)', 14]);
  eq(L.expand('sin(□)', '□'), ['sin(□)', 4]);
  eq(L.expand('abs(□)', '□+1'), ['abs(□+1)', 4]);
  eq(L.expand(' to ', ''), [' to ', null]);
  const t = 'integrate(□, □, □, x)';
  eq(L.slots(t), [10, 13, 16]);
  eq(L.nextSlot(t, 0, 0, false), 10);
  eq(L.nextSlot(t, 10, 11, false), 13);
  eq(L.nextSlot(t, 16, 17, false), 10);
  eq(L.nextSlot(t, 10, 11, true), 16);
  eq(L.nextSlot('none', 0, 0, false), null);
});

test('statistics', () => {
  const s = L.summary([1, 2, null, 3, Infinity]);
  eq([s.count, s.skipped, s.mean, s.median, s.std], [3, 2, 2, 2, 1]);
  eq(L.summary([2]).std, null);
  eq(L.summary([null]).mean, null);
  eq(L.summary([null]).sum, null);
  const m = L.summary([1, 2, 2, 3, 4, 7, 9, 12]);
  eq(m.modes, [2]); eq(m.sum, 40);
  close(m.q1, 2); // calculator's quartile: linear interpolation
  close(m.q3, 7.5);
  eq(L.summary([1, 2]).modes, []);
  eq(L.summary([1, 1, 2, 2]).modes, [1, 2]);
  close(L.summary([1, 2, 3, 4]).q1, 1.75);
  close(L.summary([1, 2, 3, 4, 5, 6, 7]).q1, 2.5);
});

test('regression', () => {
  const g = grid([['X', 'Y'], ['1', '1,5'], ['2', 'oops'], ['3', '5,5'], ['NaN', '7'], ['', '']]);
  const reg = (x, y, rows = g) => L.leastSquares(L.pairs(rows, x, y));
  eq(reg(0, 1), [2, -0.5]);
  eq(reg(null, 1, g.slice(0, 4)), [2, -0.5]);
  eq(reg(1, 0), [0.5, 0.25]);
  eq(reg(0, 1, grid([['X', 'Y'], ['1', '3'], ['2', '3']])), [0, 3]);
  assert.throws(() => reg(0, 1, grid([['X', 'Y'], ['1', '3'], ['1', '4']])), /differ/);
  assert.throws(() => reg(0, 1, grid([['X', 'Y'], ['1', '3'], ['2', 'inf']])));
  eq(reg(0, 1, grid([['X', 'Y'], ['1e200', '2e200'], ['2e200', '4e200']])), [2, 0]);
});

test('correlation', () => {
  eq(L.pearson([[1, 2], [2, 4], [3, 6]]), 1);
  eq(L.pearson([[1, 3], [2, 2], [3, 1]]), -1);
  eq(L.pearson([[1, 2], [1, 3]]), null);
  eq(L.pearson([[1, 2], [2, 2]]), null);
  eq(L.pearson([[1, 2]]), null);
  eq(L.pearson([]), null);
  eq(L.pearson([[1e200, 2e200], [2e200, 4e200], [3e200, 6e200]]), 1);
  close(L.pearson([[1, 2], [2, 4], [3, 5], [4, 9]]), 0.9647638212);
  eq(L.spearman([[1, 1], [2, 8], [3, 27], [4, 64]]), 1);
  eq(L.ranks([10, 20, 20, 5]), [2, 3.5, 3.5, 1]);
  eq(L.covariance([[1, 2], [2, 4], [3, 5], [4, 9]]), 11 / 4);
});

test('fits recover their curves', () => {
  const on = (f) => [1, 2, 3, 4, 5, 6].map((x) => [x, f(x)]);
  for (const [kind, curve, want] of [
    [1, on((x) => 2 * x - 0.5), 'y = 2x − 0.5'],
    [2, on((x) => 0.5 * x * x - 3 * x + 1), 'y = 0.5x² − 3x + 1'],
    [3, on((x) => 1.5 * Math.exp(0.5 * x)), 'y = 1.5·e^(0.5x)'],
    [4, on((x) => 3 * x ** 2), 'y = 3·x^2'],
    [5, on((x) => 2 + 4 * Math.log(x)), 'y = 2 + 4·ln(x)'],
  ]) {
    const f = L.fit(curve, kind);
    eq(L.fitEquation(f), want);
    close(f.r2, 1);
    for (const [x, y] of curve) close(L.fitAt(f, x), y, 1e-6);
  }
  assert.ok(L.fit([[1, 1], [2, 5], [3, 1], [4, 5]], 1).r2 < 0.3);
  eq(L.fit([[1, 2], [2, 2]], 1).r2, null);
});

test('fits say why not', () => {
  const m = (p, k) => { try { L.fit(p, k); } catch (e) { return e.message; } return ''; };
  const two = [[1, 1], [2, 2]];
  assert.match(m(two.slice(0, 1), 1), /at least 2/);
  assert.match(m(two, 2), /at least 3/);
  assert.match(m([[1, 1], [1, 2], [2, 3]], 2), /3 different/);
  assert.match(m([[1, 1], [1, 2]], 1), /must differ/);
  assert.match(m([[1, -1], [2, 2]], 3), /every y/);
  assert.match(m([[0, 1], [2, 2]], 4), /every x/);
  assert.match(m([[-1, 1], [2, 2]], 5), /every x/);
});

test('table columns for the calculator', () => {
  let t = L.tableColumns(grid([['X', 'Y'], ['1', '2,5'], ['2', '4'], ['', '']]));
  eq(t.named, [['X', [1, 2]], ['Y', [2.5, 4]]]);
  assert.ok(!t.misaligned && !t.unnamed.length);
  t = L.tableColumns(grid([['X', 'Y'], ['1', ''], ['2', '']]));
  eq(t.named.length, 1); assert.ok(!t.misaligned);
  assert.ok(L.tableColumns(grid([['X', 'Y'], ['1', '2'], ['', '3'], ['4', '']])).misaligned);
  t = L.tableColumns(grid([['Height (cm)', '', 'A', 'A'], ['1', '2', '3', '4']]));
  eq(t.unnamed, ['Height (cm)', 'Column 2']);
  eq(t.named, [['A', [3]]]);
  eq(L.tableColumns([]), { named: [], unnamed: [], misaligned: false });
});

test('answers become cells', () => {
  eq(L.answerNumber('5/4', '1.25'), '1.25');
  eq(L.answerNumber('−3', ''), '-3');
  eq(L.toNum(L.answerNumber('−1/2', '−0.5'), true), -0.5);
});

test('charts and histograms', () => {
  const g = grid([['X', 'Y'], ['1', '1,5'], ['2', 'oops'], ['3', '3'], ['', '']]);
  const c = L.dataChart(g, 1, 0, 1);
  eq([c.stats.count, c.skipped], [2, 1]);
  eq(c.series.y[0], 1.5); assert.ok(Number.isNaN(c.series.y[1])); eq(c.series.y[2], 3);
  for (const chart of [0, 2, 3]) {
    const d = L.dataChart(g, chart, null, 1);
    eq(d.skipped, 1);
    if (chart === 3) eq(d.series.y.reduce((a, b) => a + b, 0), 2);
  }
});

test('cells and files', () => {
  eq(L.toNum(' 1,5 ', true), 1.5); eq(L.toNum('1,5', false), null); eq(L.toNum('-2e3', false), -2000);
  eq(L.toNum('Inf', false), Infinity); assert.ok(Number.isNaN(L.toNum('NaN', false))); eq(L.toNum('', true), null); eq(L.toNum('abc', true), null);
  eq(L.parseText('a,b\n1,2\n'), [['a', 'b'], ['1', '2']]);
  eq(L.parseText('1\t2,5\n3\t4\n'), [['1', '2.5'], ['3', '4']]);
  eq(L.parseText('1;2,5\n3;4'), [['1', '2.5'], ['3', '4']]);
  eq(L.parseText('1 2\n3 4\n'), [['1', '2'], ['3', '4']]);
  eq(L.parseText('"a,b",2\n"say ""hi""",3\n'), [['a,b', '2'], ['say "hi"', '3']]);
  eq(L.parseText('﻿x,y\n1,2'), [['x', 'y'], ['1', '2']]);
  eq(L.splitHeader([['title'], ['a', 'b'], ['1', '2']]), [[['title'], ['a', 'b']], [['1', '2']]]);
  eq(L.writeCells([['a,b', 'c'], ['1', '2']], ','), '"a,b",c\n1,2\n');
  eq(L.cells([['X', 'Y', ''], ['1,5', '', ''], ['', '', '']]), [['X'], [['1.5']]]);
  eq(L.validName('Height (cm)'), 'Height_cm_'); eq(L.validName('X'), 'X'); eq(L.validName('2nd'), 'x2nd');
});

test('history', () => {
  const h = [{ expr: '1+1', input: '1 + 1', result: '2', approx: '' }, { expr: 'a\tb', input: 'c', result: 'd', approx: '0.5' }];
  const back = L.parseHistory(L.historyText(h));
  eq(back[0], h[0]); eq(back[1].expr, 'a b');
  eq(L.parseHistory('garbage\n\n\tx\ty\n2+2\t2 + 2\t4\n'), [{ expr: '2+2', input: '2 + 2', result: '4', approx: '' }]);
  eq(L.parseHistory('1\t2\t3\n'.repeat(600)).length, 500);
});

test('axes', () => {
  eq(L.ticks(-10, 10, 10).map((t) => t[1]).slice(0, 3), ['−10', '−8', '−6']);
  eq(L.ticks(0, 1, 4).map((t) => t[1]), ['0', '0.25', '0.50', '0.75', '1.00']);
  const [x, y] = L.equalRanges([-10, 10], [-6, 6], 500, 600);
  eq(x, [-10, 10]); close(y[1], 12);
  const [x2, y2] = L.equalRanges([-10, 10], [-6, 6], 1200, 300);
  close(x2[1], 24); eq(y2, [-6, 6]);
  eq(L.adjust([0, 10], 0.1, 1, 0), [1, 11]);
  eq(L.adjust([0, 10], 0, 0.5, 0.5), [2.5, 7.5]);
  eq(L.adjust([0, 10], 0, 2, 0), [0, 20]);
  eq(L.adjust([0, 10], Infinity, 1, 0), [0, 10]);
  eq(L.adjust([0, 10], 0, 0, 0.5), [0, 10]);
  eq(L.grow(800, 500), 1); eq(L.grow(1600, 1200), 1.6);
  assert.ok(L.graphTextPx(600, 300, 1) < L.graphTextPx(600, 900, 1));
  eq(L.graphTextPx(5000, 5000, 3), 56); eq(L.graphTextPx(10, 10, 0.6), 15);
  eq(L.fitScale(100, 200), 1); eq(L.fitScale(200, 100), 0.6); eq(L.fitScale(150, 120), 0.8); eq(L.fitScale(100, 0), 1);
});

// src/plot/analysis.rs's tests
const linspace = (a, b, n) => Array.from({ length: n }, (_, i) => a + ((b - a) * i) / Math.max(n - 1, 1));
const of = (f, lo, hi, n) => { const x = linspace(lo, hi, n); return L.analyze(x, x.map(f)); };
const near = (a, b, tol) => a.length === b.length && a.every((p, i) => Math.abs(p[0] - b[i][0]) <= tol && Math.abs(p[1] - b[i][1]) <= tol);

test('key points: parabola', () => {
  const k = of((x) => (x - 1) * (x - 3), -4, 8, 601);
  eq(k.vertex, [2, -1]); eq(k.min, [[2, -1]]); eq(k.max.length, 0);
  eq(k.roots, [[1, 0], [3, 0]]); eq(k.yInt, [0, 3]); eq(k.inflection.length, 0);
  eq(k.symmetry, { kind: 'axis', h: 2 });
  eq(of((x) => x * x, -3, 3, 300).symmetry.kind, 'even');
});

test('key points: sine and cubic', () => {
  const PI = Math.PI;
  let k = of(Math.sin, -6, 6, 1200); // over two periods
  const [a, b] = [(3 * PI) / 2, PI / 2];
  assert.ok(near(k.max, [[-a, 1], [b, 1]], 1e-5) && near(k.min, [[-b, -1], [a, -1]], 1e-5), JSON.stringify(k));
  assert.ok(near(k.roots, [[-PI, 0], [0, 0], [PI, 0]], 1e-6), JSON.stringify(k.roots));
  assert.ok(near(k.inflection, [[-PI, 0], [0, 0], [PI, 0]], 1e-3), JSON.stringify(k.inflection));
  eq(k.vertex, null); eq(k.symmetry.kind, 'odd');
  eq(of(Math.cos, -4, 6, 500).symmetry.kind, 'even');
  k = of((x) => x ** 3 - 3 * x, -3, 3, 601);
  assert.ok(near(k.max, [[-1, 2]], 1e-9) && near(k.min, [[1, -2]], 1e-9), JSON.stringify(k));
  assert.ok(near(k.roots, [[-Math.sqrt(3), 0], [0, 0], [Math.sqrt(3), 0]], 1e-6), JSON.stringify(k.roots));
  eq(k.inflection, [[0, 0]]); eq(k.symmetry.kind, 'odd'); eq(k.yInt, [0, 0]);
  k = of((x) => (x - 1) ** 3 + 2, -4, 6, 501);
  eq(k.symmetry, { kind: 'point', h: 1, v: 2 });
  eq(of((x) => Math.cos(x - 1) + 0.3 * (x - 1) ** 2, -3, 6, 900).symmetry, { kind: 'axis', h: 1 });
});

test('curves break at poles', () => {
  // NaN beside the pole, as the engine samples them: the jump around it still breaks the line
  const xs = linspace(-20, 20, 2001);
  for (const [f, breaks] of [[(x) => (x === 0 ? NaN : 1 / x), 2], [(x) => (x <= 0 ? NaN : Math.log(x)), 1], [(x) => (x * x === 1 ? NaN : 1 / (x * x - 1)), 4]]) {
    const [bx, by] = L.breakJumps(xs, xs.map(f));
    eq(bx.filter(Number.isNaN).length, breaks, String(f));
    eq(bx.length, by.length);
  }
  const huge = Array.from({ length: 300000 }, (_, i) => [String(i), String(i % 7)]);
  eq(L.dataChart([['a', 'b'], ...huge], 3, null, 1).stats.count, 300000, 'a very large table is fine');
});

test('key points: refuses what it should', () => {
  eq(L.analyze([1, 2, 3], [1, 2, 1]), null);
  eq(L.analyze([0, 1, 2, 1, 0, 3], [0, 1, 0, 1, 0, 1]), null);
  eq(L.analyze(new Array(9).fill(NaN), new Array(9).fill(1)), null);
  eq(L.analyze(linspace(0, 1, 9), new Array(4).fill(1)), null);
  const [x, y] = L.breakJumps(linspace(-1, 1, 11), linspace(-1, 1, 11).map((v) => (v === 0 ? NaN : 1 / v)));
  assert.ok(x.length >= 11 && y.length === x.length);
});
