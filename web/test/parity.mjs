// The web engine against the desktop's: web/test/parity.json holds what the desktop build answered
// (`cargo test export_parity -- --ignored`), and every answer here must match it character by character.
import { readFileSync } from 'node:fs';
import createQalc from '../app/engine/qalc.mjs';
import { Engine, DEFAULT_OPTIONS } from '../app/engine-core.js';

const reference = JSON.parse(readFileSync(new URL('parity.json', import.meta.url)));
const settings = {
  rad: DEFAULT_OPTIONS,
  deg: { ...DEFAULT_OPTIONS, angle: 'deg' },
  decimal: { ...DEFAULT_OPTIONS, angle: 'deg', fraction: 'decimal' },
};
const engine = new Engine(await createQalc());
const differences = [];

for (const want of reference.answers) {
  const { expr, set } = want;
  let got;
  try {
    const a = engine.classroomEval(expr, settings[set]);
    got = { expr, set, input: a.input, result: a.result, approx: a.approx, freeVars: a.freeVars };
  } catch (e) {
    got = { expr, set, error: e.message };
  }
  if (JSON.stringify(got) !== JSON.stringify(want)) differences.push(['answer', want, got]);
}

const close = (a, b) => (a === null || b === null ? a === b : a === b || Math.abs(a - b) <= 1e-12 * Math.max(Math.abs(a), Math.abs(b)));
for (const want of reference.curves) {
  const { expr, set } = want;
  let got;
  try {
    const [x, y] = engine.classroomCurve(expr, settings[set], -20, 20);
    const ys = y.filter((_, i) => i % 100 === 0).map((v) => (Number.isFinite(v) ? v : null));
    got = { expr, set, points: y.length, xs: x.filter(Number.isNaN).length, ys };
  } catch (e) {
    got = { expr, set, error: e.message };
  }
  const same = want.error !== undefined
    ? got.error === want.error
    : got.points === want.points && got.xs === want.xs && got.ys.length === want.ys.length && got.ys.every((v, i) => close(v, want.ys[i]));
  if (!same) differences.push(['curve', want, got]);
}

for (const [kind, want, got] of differences.slice(0, 40)) {
  console.log(`${kind} differs:\n  desktop: ${JSON.stringify(want).slice(0, 400)}\n  web:     ${JSON.stringify(got).slice(0, 400)}`);
}
console.log(`${reference.answers.length} answers and ${reference.curves.length} curves compared: ${differences.length} differ`);
process.exitCode = differences.length ? 1 : 0;
