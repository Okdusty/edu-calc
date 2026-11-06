// The WebAssembly engine against the desktop's expected answers (src/qalc/mod.rs tests).
import assert from 'node:assert/strict';
import createQalc from '../app/engine/qalc.mjs';
import { Engine, DEFAULT_OPTIONS } from '../app/engine-core.js';
import { UNITS_ERROR } from '../app/logic.js';

const started = Date.now();
const e = new Engine(await createQalc());
console.log(`loaded in ${Date.now() - started} ms`);
const o = DEFAULT_OPTIONS;
const deg = { ...o, angle: 'deg' };
const dec = { ...deg, fraction: 'decimal' };
const calc = (s, op = o) => e.classroomEval(s, op);
const eq = assert.deepEqual;

eq(calc('sin(30)', deg).result, '1/2');
eq(calc('sin(pi/2)').result, '1');
eq(calc('3/4 + 1/2', deg).result, '5/4');
eq(calc('3/4 + 1/2', deg).approx, '1.25');
eq(calc('3/4 + 1/2', dec).result, '1.25');
assert.throws(() => calc('sin(', deg));
eq(calc('5 km to mi').result, '3.106855961 mi');
eq(calc('integrate(x^2, 0, 3)').result, '9');
eq(calc('binomial(5, 2)').result, '10');
eq(calc('2^100').result, '1267650600228229401496703205376');
eq(calc('mean([2, 4, 9])').result, '5');
eq(calc('quartile([1, 2, 3, 4], 1)').result, '7/4');
eq(calc('x + 1').freeVars, ['x']);

// the graph
let [xs, ys] = e.classroomCurve('x²', deg, -2, 2);
eq(xs.length, ys.length);
assert.ok(Math.abs(ys[0] - 4) < 1e-9);
[, ys] = e.classroomCurve('sin(x)', deg, 0, 90);
assert.ok(Math.abs(ys[ys.length - 1] - 1) < 1e-9);
[, ys] = e.classroomCurve('1/x', deg, -1, 1);
assert.ok(ys.some(Number.isNaN), 'broken at the pole');
const message = (f) => { try { f(); } catch (err) { return err.message; } assert.fail('no error'); };
assert.match(message(() => e.classroomCurve('x+y', deg, -1, 1)), /“y”/);
eq(message(() => e.classroomCurve('abc', deg, -1, 1)), UNITS_ERROR);
assert.match(message(() => e.classroomCurve('2x+', deg, -1, 1)), /ignored/);
e.classroomCurve('sin(x + 30°)', deg, -1, 1);
assert.ok(e.classroomCurve('3', deg, -1, 1)[1].every((y) => y === 3));
assert.match(message(() => e.graphRow('foo(x)', deg, -1, 1)), /“foo”/);
assert.match(message(() => e.graphRow('sin(x', deg, -1, 1)), /missing a closing/);
let t = Date.now();
e.graphRow('y = sin(x) + x²', deg, -30, 30);
console.log(`2001 samples of sin(x) + x²: ${Date.now() - t} ms`);

// the table in the calculator
const cols = [['X', [1, 2, 3, 4]], ['Y', [2, 4, 5, 9]], ['m', [1]]];
eq(e.shareColumns(cols, []), ['X', 'Y']);
eq(calc('mean(X)').result, '5/2');
eq(calc('pearson(X, Y)').approx, '0.9647638212');
eq(calc('slope(X, Y)').result, '11/5');
eq(calc('intercept(X, Y)').result, '−1/2');
eq(e.shareColumns(cols.slice(0, 1), ['Y', 'm']), ['X']);
assert.throws(() => calc('mean(Y)'));
e.shareColumns([], ['X']);
assert.throws(() => calc('mean(X)'));

// a runaway calculation ends, and the engine still works
t = Date.now();
try { calc('factorial(10^8)'); } catch { /* timed out */ }
console.log(`runaway ended in ${Date.now() - t} ms`);
assert.ok(Date.now() - t < 30000);
eq(calc('1 + 1').result, '2');

assert.ok(e.catalog().length > 1000, `catalog ${e.catalog().length}`);
assert.ok(e.functionNames().has('sin') && e.functionNames().has('slope'));
console.log('engine ok');
