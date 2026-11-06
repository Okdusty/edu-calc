// The web engine against the desktop's: web/test/parity.json holds what the desktop build answered
// (`cargo test export_parity -- --ignored`), and every answer here must match it character by character.
import { readFileSync } from 'node:fs';
import createQalc from '../app/engine/qalc.mjs';
import { Engine, DEFAULT_OPTIONS } from '../app/engine-core.js';
import * as XLSX from '../app/vendor/xlsx.mjs';
import { parseText, readBook, sheetRows, trimCells } from '../app/logic.js';

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

// whole graph rows through the engine, errors in a student's words
const model = JSON.parse(readFileSync(new URL('parity-model.json', import.meta.url)));
for (const [text, points, error] of model.graphRows) {
  let got;
  try {
    got = [text, engine.graphRow(text, settings.deg, -5, 5)[0].length, null];
  } catch (e) {
    got = [text, null, e.message];
  }
  if (JSON.stringify(got) !== JSON.stringify([text, points, error])) differences.push(['graph row', [text, points, error], got]);
}

// the same workbooks and file, read as the Data tab reads them
const root = new URL('../../', import.meta.url);
for (const { file, sheets } of model.books) {
  const bytes = readFileSync(new URL(file, root));
  const wb = file.endsWith('.csv') ? null : readBook(XLSX, new Uint8Array(bytes));
  if (wb && JSON.stringify(wb.SheetNames) !== JSON.stringify(sheets.map((s) => s[0]))) differences.push(['sheet names', sheets.map((s) => s[0]), wb.SheetNames]);
  for (const [name, want] of sheets) {
    const got = wb ? sheetRows(XLSX, wb.Sheets[name]) : trimCells(parseText(bytes.toString('utf8')));
    if (JSON.stringify(got) !== JSON.stringify(want)) differences.push([`${file} ${name}`, want, got]);
  }
}

for (const [kind, want, got] of differences.slice(0, 40)) {
  console.log(`${kind} differs:\n  desktop: ${JSON.stringify(want).slice(0, 400)}\n  web:     ${JSON.stringify(got).slice(0, 400)}`);
}
console.log(`${reference.answers.length} answers, ${reference.curves.length} curves, ${model.graphRows.length} graph rows and ${model.books.length} files compared: ${differences.length} differ`);
process.exitCode = differences.length ? 1 : 0;
