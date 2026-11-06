// The published site in a real browser: every tab through its real widgets, then all of it again offline.
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { extname, join } from 'node:path';
import { chromium } from 'playwright';

const dist = new URL('../dist/', import.meta.url).pathname;
const shots = new URL('../shots/', import.meta.url).pathname;
mkdirSync(shots, { recursive: true });
const MIME = { '.html': 'text/html', '.js': 'text/javascript', '.mjs': 'text/javascript', '.css': 'text/css', '.wasm': 'application/wasm', '.png': 'image/png', '.webmanifest': 'application/manifest+json' };
// SITE_URL: test what's published instead of web/dist
// the local server can misbehave on demand: requests for the engine it answers 503 while these say so
const engineTrouble = { failNext: 0, down: false, requests: 0, newVersion: '' };
const server = process.env.SITE_URL ? null : createServer((req, res) => {
  const path = decodeURIComponent(new URL(req.url, 'http://x').pathname);
  if (path.endsWith('qalc.wasm')) {
    engineTrouble.requests++;
    if (engineTrouble.down || engineTrouble.failNext > 0) {
      engineTrouble.failNext--;
      res.writeHead(503).end('busy');
      return;
    }
  }
  if (!path.startsWith('/edu-calc/')) { res.writeHead(404).end(); return; }
  let file = join(dist, path.slice('/edu-calc/'.length));
  if (path.endsWith('/')) file = join(file, 'index.html');
  try { statSync(file); } catch { res.writeHead(404).end(); return; }
  let body = readFileSync(file);
  if (path.endsWith('/sw.js') && engineTrouble.newVersion) body = body.toString().replace(/const VERSION = '([^']*)'/, `const VERSION = '$1${engineTrouble.newVersion}'`);
  res.writeHead(200, { 'content-type': MIME[extname(file)] ?? 'application/octet-stream' }).end(body);
});
if (server) await new Promise((r) => server.listen(0, '127.0.0.1', r));
const url = process.env.SITE_URL ?? `http://127.0.0.1:${server.address().port}/edu-calc/`;

let serverStopped = false;
// the desktop's own defaults, as a file from that app would hold them
const DESKTOP_INI = (() => {
  try {
    const src = readFileSync(new URL('../../src/gui/config.rs', import.meta.url), 'utf8');
    return /pub const DEFAULT_INI: &str = "\\\n([^"]*)";/.exec(src)?.[1] ?? '';
  } catch {
    return '';
  }
})();
const browser = await chromium.launch();
const context = await browser.newContext({ viewport: { width: 1366, height: 860 }, deviceScaleFactor: 2, acceptDownloads: true, permissions: ['clipboard-read', 'clipboard-write'] });
const page = await context.newPage();
const problems = [];
page.on('pageerror', (e) => problems.push(`pageerror: ${e.message}`));
// the live server's own hiccups are for the app to survive, not failures of the app
page.on('console', (m) => { if (m.type() === 'error' && !(process.env.SITE_URL && m.text().startsWith('Failed to load resource'))) problems.push(`console: ${m.text()}`); });
page.on('dialog', (d) => d.dismiss());

let step = 'start';
const steps = [];
let pass = '';
const check = (name) => { step = `${pass}${name}`; steps.push(step); console.log(`- ${step}`); };
const shot = (name) => page.screenshot({ path: join(shots, `${pass ? 'offline-' : ''}${name}.png`) });
const until = (f, arg) => page.waitForFunction(f, arg, { timeout: 30000 });
/** A check that has to await something: waitForFunction would take its promise as already true. */
const untilAsync = async (on, f, what, ms = 60000) => {
  const end = Date.now() + ms;
  for (;;) {
    if (await on.evaluate(f)) return;
    assert.ok(Date.now() < end, `timed out: ${what}`);
    await new Promise((r) => setTimeout(r, 100));
  }
};
const copyReady = async () => {
  const r = await navigator.serviceWorker.getRegistration();
  return r?.active?.state === 'activated' && !r.installing && !r.waiting && navigator.serviceWorker.controller !== null && (await caches.keys()).length === 1;
};
const text = (sel) => page.locator(sel).first().innerText();
const key = (face) => page.locator('.keypad button', { hasText: new RegExp(`^${face.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`) }).first().click();
const keys = async (...faces) => { for (const f of faces) await key(f); };
const ready = () => until(() => window.app?.engine?.ready === true);
const previewIs = (t) => until((want) => document.querySelector('.preview').textContent === want, t);
const tab = (name) => page.locator('.tabs button', { hasText: name }).click();
const statsHas = (t) => until((want) => document.querySelector('.stats').textContent.includes(want), t);

try {
  // every feature, through its real widgets; run online, then again with the network gone
  const features = async () => {
  assert.equal(await page.title(), 'EduCalc');
  assert.equal(await page.evaluate(() => document.documentElement.dataset.theme), 'light', 'light by default');
  assert.equal(await page.locator('text=Advanced').count(), 0, 'no Advanced mode on the web');
  await shot('01-start');

  check('calculator: keys, live preview, history');
  await keys('a/b');
  assert.equal(await page.inputValue('.input-row input'), '□/□');
  await keys('3');
  assert.equal(await page.inputValue('.input-row input'), '3/□');
  await page.fill('.input-row input', '3/4 + 1/2');
  await previewIs('= 5/4  ≈ 1.25');
  await key('Enter');
  await until(() => document.querySelectorAll('.history-card').length === 1);
  assert.equal(await page.inputValue('.input-row input'), '');
  assert.match(await text('.history-card'), /5\/4/);
  await keys('sin');
  assert.equal(await page.inputValue('.input-row input'), 'sin(□)');
  await keys('3', '0');
  assert.equal(await page.inputValue('.input-row input'), 'sin(30)');
  await previewIs('= 1/2  ≈ 0.5');
  await page.keyboard.press('Enter');
  await until(() => document.querySelectorAll('.history-card').length === 2);
  assert.ok(await page.evaluate(() => window.app.lastAnswer === '1/2'));

  check('calculator: errors, slots, reuse, Decimal, DEG/RAD');
  await page.fill('.input-row input', 'sin(');
  await until(() => document.querySelector('.message').textContent !== '');
  await page.fill('.input-row input', '');
  await keys('aᵇ');
  assert.equal(await page.inputValue('.input-row input'), '□^□');
  await key('Enter');
  await until(() => document.querySelector('.message').textContent === 'Fill each □ before calculating.');
  await page.fill('.input-row input', '');
  await page.locator('.history-card').first().click();
  assert.equal(await page.inputValue('.input-row input'), '3/4 + 1/2');
  await page.locator('.input-row button', { hasText: 'Decimal' }).click();
  await previewIs('= 1.25');
  await page.locator('.input-row button', { hasText: 'Decimal' }).click();
  await page.fill('.input-row input', 'sin(pi/2)');
  await page.locator('.header-end button', { hasText: 'DEG' }).click();
  await previewIs('= 1');
  await page.locator('.header-end button', { hasText: 'RAD' }).click();
  await page.fill('.input-row input', '5 km to mi');
  await previewIs('= 3.106855961 mi');
  await page.fill('.input-row input', 'integrate(x^2, 0, 3)');
  await previewIs('= 9');
  await page.fill('.input-row input', 'x² + 1');
  await until(() => !document.querySelector('.input-row button:last-child').disabled);
  await page.locator('.input-row button', { hasText: 'Graph' }).click();

  check('graph: Graph button, rows, curves, errors');
  await until(() => window.app.graph.axes.series.length === 1);
  // like the desktop: the first, empty row stays and the function gets a new one
  const first = page.locator('.graph-row input[type="text"]').first();
  assert.equal(await page.locator('.graph-row input[type="text"]').nth(1).inputValue(), 'x² + 1');
  await first.fill('y = 2x + 1');
  await until(() => window.app.graph.axes.series.length === 2);
  await page.locator('.graph-tools button', { hasText: 'Add function' }).click();
  assert.equal(await page.locator('.graph-row').count(), 3);
  await page.locator('.graph-delete').nth(2).click();
  assert.equal(await page.locator('.graph-row').count(), 2);
  for (const [bad, want] of [['sin(', '“sin(” is missing a closing “)”.'], ['2x+', '“+” at the end'], ['foo(x)', '“foo”'], ['x+t', '“t”'], ['2,5x', 'For decimals']]) {
    await first.fill(bad);
    await until((w) => document.querySelector('.graph-error').textContent.includes(w), want);
  }
  await first.fill('sqrt(-x² - 1)');
  await until(() => document.querySelector('.graph-error').textContent === 'No real values in this view.');
  await first.fill('1/x');
  await until(() => document.querySelector('.graph-error').hidden);
  await until(() => window.app.graph.axes.series.length === 2);
  await page.waitForTimeout(200);
  await shot('02-graph');
  const pixels = () => page.evaluate(() => {
    const c = document.querySelector('.view:not([hidden]) canvas');
    const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
    const seen = new Set();
    for (let i = 0; i < d.length; i += 4000) seen.add(`${d[i]},${d[i + 1]},${d[i + 2]}`);
    return seen.size;
  });
  assert.ok(await pixels() > 2, 'the graph drew something');
  await page.locator('.graph-check input').first().uncheck();
  await until(() => window.app.graph.axes.series.length === 1);
  await page.locator('.graph-check input').first().check();
  await until(() => window.app.graph.axes.series.length === 2);

  check('graph: pan, wheel, tap, key points, reset, axis equal');
  const box = await page.locator('.view:not([hidden]) canvas').boundingBox();
  const mid = [box.x + box.width / 2, box.y + box.height / 2];
  const before = await page.evaluate(() => window.app.graph.range());
  await page.mouse.move(mid[0], mid[1]);
  await page.mouse.down();
  await page.mouse.move(mid[0] - 100, mid[1], { steps: 5 });
  await page.waitForTimeout(150);
  await page.mouse.up();
  const after = await page.evaluate(() => window.app.graph.range());
  assert.ok(after[0] > before[0], 'dragging left moves the view right');
  await page.mouse.wheel(0, 300);
  await page.waitForTimeout(100);
  const zoomed = await page.evaluate(() => window.app.graph.range());
  assert.ok(zoomed[1] - zoomed[0] > after[1] - after[0], 'wheel zooms out');
  const equal = await page.evaluate(() => { const b = window.app.graph; const f = b.frame; const [x, y] = [f.bounds[0], f.bounds[1]]; return ((f.rect[2] - f.rect[0]) / (x[1] - x[0])) / ((f.rect[3] - f.rect[1]) / (y[1] - y[0])); });
  assert.ok(Math.abs(equal - 1) < 0.03, `one scale on both axes: ${equal}`);
  await page.locator('.board-tools button').nth(2).click();
  await page.waitForTimeout(100);
  const curve = await page.evaluate(() => { const b = window.app.graph; const f = b.frame; const y = 2 * 2 + 1; const [x0, y0, x1, y1] = f.rect; return [x0 + ((2 - f.bounds[0][0]) / (f.bounds[0][1] - f.bounds[0][0])) * (x1 - x0), y1 + ((y - f.bounds[1][0]) / (f.bounds[1][1] - f.bounds[1][0])) * (y0 - y1)]; });
  await page.mouse.click(box.x + curve[0], box.y + curve[1]);
  await until(() => window.app.graph.axes.pins.length === 1);
  assert.match(await text('.view:not([hidden]) .board-status'), /x = 2(\.\d+)?, y = /);
  await page.waitForTimeout(400);
  await page.locator('.board-tools button').nth(3).click();
  await page.waitForTimeout(300);
  await shot('03-graph-keypoints');
  await page.locator('.board-tools button').nth(3).click();
  await page.locator('.board-tools button').nth(2).click();
  await until(() => window.app.graph.axes.pins.length === 0);

  check('graph: equations scale with the list, and long ones fit');
  const fontPx = () => first.evaluate((e) => parseFloat(getComputedStyle(e).fontSize));
  const px0 = await fontPx();
  await page.locator('.graph-tools button', { hasText: 'A+' }).click();
  assert.ok(await fontPx() > px0, 'A+ makes equations bigger');
  const targetH = await first.evaluate((e) => e.getBoundingClientRect().height);
  assert.ok(targetH >= 44, `touch target ${targetH}`);
  await first.fill('y = sin(x) + cos(2x) − tan(x / 3) + 1/x + 2x² + 7');
  assert.ok(await fontPx() < (await page.evaluate(() => window.app.equationPx)), 'a long equation shrinks');
  await page.locator('.graph-tools button', { hasText: 'A−' }).click();
  await first.fill('1/x');
  await page.setViewportSize({ width: 1800, height: 1000 });
  await page.waitForTimeout(300);
  assert.ok(await fontPx() > px0, 'a wider list reads bigger');
  await page.setViewportSize({ width: 1366, height: 860 });

  check('keypad: keys line up in rows and widths at every size');
  const aligned = () => page.evaluate(() => {
    const rect = (e) => e.getBoundingClientRect();
    const side = [...document.querySelectorAll('.side-keys button')];
    const fn = [...document.querySelectorAll('.key-grid.functions button')];
    const nb = [...document.querySelectorAll('.key-grid.numbers button')];
    const out = [];
    for (let r = 0; r < 4; r++) out.push(Math.abs(rect(side[r]).top - rect(fn[r * 5]).top), Math.abs(rect(nb[r * 6]).top - rect(fn[r * 5]).top), Math.abs(rect(side[r]).height - rect(fn[r * 5]).height));
    out.push(Math.abs(rect(fn[0]).width - rect(nb[0]).width));
    return Math.max(...out);
  });
  for (const [w, hh] of [[1366, 860], [1920, 1080], [900, 900]]) {
    await page.setViewportSize({ width: w, height: hh });
    await page.waitForTimeout(100);
    const worst = await aligned();
    assert.ok(worst < 1.5, `${w}x${hh}: off by ${worst}px`);
  }
  await page.setViewportSize({ width: 1366, height: 860 });
  for (const p of ['Stats', 'Trig', 'f(x)', 'Basic']) await page.locator('.side-keys button', { hasText: p }).click();

  check('functions catalog: search and insert');
  await tab('Calculator');
  await page.fill('.input-row input', '');
  await page.locator('.header-end button', { hasText: 'Functions' }).click();
  await page.fill('.catalog input', 'remainder');
  assert.equal(await page.locator('.catalog-row:visible').count(), 1);
  await page.keyboard.press('Enter');
  assert.equal(await page.inputValue('.input-row input'), 'mod(□, □)');
  await page.fill('.input-row input', '');

  check('data: typing, keypad, charts, statistics');
  await tab('Data');
  const cell = (r, c) => page.locator(`.table-scroll tr[data-row="${r}"] input[data-col="${c}"]`);
  await cell(1, 0).click();
  await keys('1', 'Enter', '2', 'Enter', '3');
  assert.equal(await cell(3, 0).inputValue(), '3');
  await statsHas('3 rows skipped'); // the y column, Y, is still empty
  const [chartSel, xSel, ySel] = [0, 1, 2].map((i) => page.locator('.selectors select').nth(i));
  await xSel.selectOption({ label: 'Row number' });
  await ySel.selectOption({ label: 'X' });
  await statsHas('mean = 2');
  await statsHas('sd = 1');
  await statsHas('Best-fit line: y = 1x + 0');
  await chartSel.selectOption({ label: 'Histogram' });
  await statsHas('mean = 2');
  assert.ok(!(await page.locator('.stats').innerText()).includes('Best-fit'), 'no line through a histogram');
  await chartSel.selectOption({ label: 'Scatter' });
  await xSel.selectOption({ label: 'X' });
  await ySel.selectOption({ label: 'Y' });
  await page.evaluate(() => { window.app.table.setGrid([['X', 'Y'], ['1', '2.1'], ['2', '3.9'], ['3', '6.2'], ['4', '7.8'], ['5', '10.1'], ['6', '12.2'], ['', '']], [1, 0]); window.app.table.changed(); });
  await statsHas('Best-fit line: y = 2.02x');
  await statsHas('r = 0.9991');
  await statsHas('r² = 0.9982');
  for (const [i, want] of [[2, 'Quadratic fit'], [3, 'Exponential fit'], [4, 'Power fit'], [5, 'Logarithmic fit'], [0, 'Pearson r = '], [1, 'Best-fit line']]) {
    await page.locator('.selectors select').nth(3).selectOption({ index: i });
    await statsHas(want);
  }
  for (const c of [1, 2, 3, 0]) { await page.locator('.selectors select').first().selectOption({ index: c }); await page.waitForTimeout(50); }
  await page.locator('.more-stats summary').click();
  for (const label of ['Sum', 'Q1', 'IQR', 'SD, population', 'Standard error', 'Spearman ρ', 'Covariance, sample', 'Slope m']) assert.ok(await page.locator('.more-grid .label', { hasText: label }).count() >= 1, label);
  await page.waitForTimeout(200);
  await shot('04-data-statistics');
  await page.locator('.more-stats summary').click();

  check('data: row and column buttons, Ans, names');
  const size = () => page.evaluate(() => [window.app.table.g.length, window.app.table.cols]);
  await cell(1, 0).click();
  const [rows0, cols0] = await size();
  await page.locator('[data-do="addRow"]').click();
  assert.deepEqual(await size(), [rows0 + 1, cols0]);
  await page.locator('[data-do="delRow"]').click();
  assert.deepEqual(await size(), [rows0, cols0]);
  await page.locator('[data-do="addCol"]').click();
  assert.deepEqual(await size(), [rows0, cols0 + 1]);
  await page.locator('[data-do="delCol"]').click();
  assert.deepEqual(await size(), [rows0, cols0]);
  assert.equal(await cell(1, 0).inputValue(), '1', 'the data column stayed');

  check('calculator and table share numbers');
  await tab('Calculator');
  await until(() => JSON.stringify(window.app.sharedNames) === '["X","Y"]');
  await page.locator('.table-bar button', { hasText: 'Correlation r' }).click();
  assert.equal(await page.inputValue('.input-row input'), 'pearson(X, Y)');
  await page.waitForTimeout(400);
  await shot('05-calculator-table');
  await key('Enter');
  await until(() => window.app.lastExpr === 'pearson(X, Y)');
  const r = await page.evaluate(() => Number(window.app.lastNumber));
  assert.ok(Math.abs(r - 0.9991049325) < 1e-8, `r ${r}`);
  for (const [f, want] of [['mean(Y)', '7.05'], ['slope(X, Y)', '2.02'], ['intercept(X, Y)', '-0.02']]) {
    await page.fill('.input-row input', f);
    await key('Enter');
    await until((e) => window.app.lastExpr === e, f);
    assert.ok(Math.abs(await page.evaluate(() => Number(window.app.lastNumber)) - Number(want)) < 1e-9, f);
  }
  await tab('Data');
  await cell(7, 0).click();
  await key('Ans');
  assert.equal(await cell(7, 0).inputValue(), '-0.02');
  await page.keyboard.press('Control+a');
  await page.keyboard.press('Delete');
  await cell(0, 1).fill('m');
  await tab('Calculator');
  await until(() => document.querySelector('.table-note').textContent.includes('Not available here: m'));
  await tab('Data');
  await cell(0, 1).fill('Y');
  await cell(2, 1).fill('');
  await tab('Calculator');
  await until(() => document.querySelector('.table-note').textContent.includes('no longer line up'));
  await tab('Data');
  await cell(2, 1).fill('3.9');

  check('data: paste, import csv and xlsx, export');
  await cell(1, 0).click();
  await page.evaluate(() => {
    const e = new Event('paste', { bubbles: true });
    e.clipboardData = { getData: () => '10\t20\n30\t40\n' };
    document.querySelector('.table-scroll input').dispatchEvent(e);
  });
  await until(() => window.app.table.g[2][1] === '40');
  const [fileChooser] = await Promise.all([page.waitForEvent('filechooser', { timeout: 15000 }).catch(() => null), (async () => { await page.locator('[data-do="import"]').click(); const b = page.locator('dialog button', { hasText: 'Discard' }); if (await b.count()) await b.click(); })()]);
  assert.ok(fileChooser, 'import opens a file chooser');
  await fileChooser.setFiles({ name: 'weather.csv', mimeType: 'text/csv', buffer: Buffer.from('Title line\nday;temp\n1;20,5\n2;22\n') });
  await until(() => window.app.table.g[0][1] === 'temp' && window.app.table.g[1][1] === '20.5');
  assert.match(await text('.table-status'), /2 rows × 2 columns · weather\.csv/);
  const book = await page.evaluate(async () => {
    const X = await import('./vendor/xlsx.mjs');
    const wb = X.utils.book_new();
    X.utils.book_append_sheet(wb, X.utils.aoa_to_sheet([['Height', 'Mass'], [1.5, 50], [1.7, 70]]), 'People');
    X.utils.book_append_sheet(wb, X.utils.aoa_to_sheet([['A'], [9]]), 'Other');
    return [...new Uint8Array(X.write(wb, { type: 'array', bookType: 'xlsx' }))];
  });
  await page.locator('.file').setInputFiles({ name: 'book.xlsx', mimeType: 'application/octet-stream', buffer: Buffer.from(book) });
  await until(() => window.app.table.g[0][0] === 'Height' && window.app.table.g[2][1] === '70');
  assert.ok(await page.locator('.sheets').isVisible(), 'a workbook offers its sheets');
  await page.locator('.sheets').selectOption('Other');
  await until(() => window.app.table.g[0][0] === 'A' && window.app.table.g[1][0] === '9');
  // the desktop's own test files, cell for cell (web/test/parity-model.json)
  const model = JSON.parse(readFileSync(new URL('parity-model.json', import.meta.url)));
  for (const { file, sheets } of model.books) {
    if (await page.evaluate(() => window.app.table.dirty)) await page.evaluate(() => { window.app.table.dirty = false; });
    await page.locator('.file').setInputFiles(new URL(`../../${file}`, import.meta.url).pathname);
    for (const [name, want] of sheets) {
      if (name) await page.locator('.sheets').selectOption(name);
      // a header row names the columns, as on the desktop
      const firstData = want.findIndex((r) => r.some((c) => /^-?[\d.]+(e-?\d+)?$/i.test(c)));
      await until(([names, rows]) => JSON.stringify(window.app.table.g[0].slice(0, names.length)) === JSON.stringify(names) && window.app.table.g.length >= rows, [want[firstData - 1] ?? [], want.length - Math.max(firstData, 0) + 1]);
      const got = await page.evaluate(() => window.app.table.g);
      want.slice(Math.max(firstData, 0)).forEach((row, i) => {
        assert.deepEqual(got[i + 1].slice(0, row.length), row, `${file} ${name} row ${i}`);
      });
    }
  }
  await page.evaluate(() => { window.app.table.setGrid([['A'], ['9']], [1, 0]); window.app.table.changed(); window.app.table.dirty = false; });
  const [dl] = await Promise.all([page.waitForEvent('download'), (async () => { await page.locator('[data-do="export"]').click(); await page.locator('dialog button', { hasText: 'CSV' }).click(); })()]);
  assert.equal(dl.suggestedFilename(), 'data.csv');
  assert.equal(readFileSync(await dl.path(), 'utf8'), 'A\n9\n');
  await page.evaluate(() => { window.app.table.setGrid([['X', 'Y'], ['1', '2'], ['2', '4'], ['3', '5'], ['4', '9']], [1, 0]); window.app.table.changed(); });
  await until(() => document.querySelectorAll('.selectors select')[2].options.length === 2);
  await ySel.selectOption({ label: 'Y' }); // a one-column sheet had moved it to the only column
  await statsHas('r = 0.9648');

  check('chart export: PNG and SVG');
  for (const [button, name] of [['PNG', 'graph.png'], ['SVG', 'graph.svg']]) {
    const [d] = await Promise.all([page.waitForEvent('download'), (async () => { await page.locator('.header-end button[aria-label="Menu"]').click(); await page.locator('.menu button', { hasText: 'Export chart' }).click(); await page.locator('dialog button', { hasText: button }).click(); })()]);
    assert.equal(d.suggestedFilename(), name);
    const data = readFileSync(await d.path());
    if (button === 'PNG') assert.equal(data.subarray(1, 4).toString(), 'PNG');
    else assert.match(data.toString(), /^<svg .*<\/svg>\s*$/s);
    assert.ok(data.length > 1000);
  }

  check('plot labels: math markup, as on the desktop');
  await cell(0, 1).fill('v_0');
  await page.waitForTimeout(350);
  const [markup] = await Promise.all([page.waitForEvent('download'), (async () => { await page.locator('.header-end button[aria-label="Menu"]').click(); await page.locator('.menu button', { hasText: 'Export chart' }).click(); await page.locator('dialog button', { hasText: 'SVG' }).click(); })()]);
  const svgText = readFileSync(await markup.path()).toString();
  assert.match(svgText, />v<\/text>/, 'the base');
  assert.match(svgText, />0<\/text>/, 'the subscript, its own smaller run');
  await cell(0, 1).fill('Y');

  check('history stays pinned to the newest answer as space changes');
  await tab('Calculator');
  await page.evaluate(() => { for (let i = 0; i < 40; i++) window.app.historyCard({ expr: `${i}`, input: `${i}`, result: `${i}`, approx: '' }); });
  await page.waitForTimeout(150);
  const atBottom = () => page.evaluate(() => { const s = document.querySelector('.history-scroll'); return Math.abs(s.scrollHeight - s.clientHeight - s.scrollTop) < 2 && s.scrollHeight > s.clientHeight; });
  assert.ok(await atBottom());
  for (const hh of [320, 150, 240]) {
    await page.evaluate((v) => window.app.setKeypadHeight(v), hh);
    await page.waitForTimeout(120);
    assert.ok(await atBottom(), `keypad ${hh}: newest answer in view`);
  }
  await page.setViewportSize({ width: 1000, height: 600 });
  await page.waitForTimeout(120);
  assert.ok(await atBottom(), 'smaller window: newest answer in view');
  await page.setViewportSize({ width: 1366, height: 860 });
  await page.waitForTimeout(120);
  assert.ok(await atBottom(), 'bigger window: newest answer in view');
  await tab('Graph');
  await tab('Calculator');
  await page.waitForTimeout(120);
  assert.ok(await atBottom(), 'back from another tab: newest answer in view');
  const hist = await page.locator('.history-scroll').boundingBox();
  await page.mouse.move(hist.x + hist.width / 2, hist.y + 50);
  await page.mouse.wheel(0, -600);
  await page.waitForTimeout(150);
  assert.ok(!(await atBottom()), 'scrolled up by hand');
  const kept = await page.evaluate(() => document.querySelector('.history-scroll').scrollTop);
  await page.evaluate(() => window.app.setKeypadHeight(200));
  await page.waitForTimeout(120);
  assert.ok(Math.abs(await page.evaluate(() => document.querySelector('.history-scroll').scrollTop) - kept) < 2, 'and it stays there');
  await page.fill('.input-row input', '1 + 1');
  await key('Enter');
  await until(() => window.app.lastExpr === '1 + 1');
  await page.waitForTimeout(120);
  assert.ok(await atBottom(), 'a new answer brings it back');

  check('clear history, in the page and in storage');
  await page.locator('.clear-history').click();
  await shot('06-clear-history');
  await page.locator('dialog button', { hasText: 'Cancel' }).click();
  assert.ok(await page.locator('.history-card').count() > 40 - 1);
  await page.locator('.clear-history').click();
  await page.locator('dialog button', { hasText: 'Clear History' }).click();
  await until(() => document.querySelectorAll('.history-card').length === 0);
  assert.equal(await page.evaluate(() => localStorage.getItem('educalc.history')), null);
  assert.ok(await page.locator('.clear-history').isHidden());

  check('appearance: dark, system, light');
  const menuItem = async (name) => { await page.locator('.header-end button[aria-label="Menu"]').click(); await page.locator('.menu button', { hasText: name }).click(); };
  await menuItem('Dark');
  assert.equal(await page.evaluate(() => document.documentElement.dataset.theme), 'dark');
  await tab('Graph');
  await page.waitForTimeout(200);
  await shot('07-graph-dark');
  await page.emulateMedia({ colorScheme: 'dark' });
  await menuItem('System Default');
  assert.equal(await page.evaluate(() => document.documentElement.dataset.theme), 'dark');
  await page.emulateMedia({ colorScheme: 'light' });
  await until(() => document.documentElement.dataset.theme === 'light');
  await menuItem('Dark');
  await page.keyboard.press('Control+Shift+D');
  assert.equal(await page.evaluate(() => document.documentElement.dataset.theme), 'light');

  check('settings: qcalcp.ini and style.css, also a desktop file as it is');
  const openSettings = async () => { await page.locator('.header-end button[aria-label="Menu"]').click(); await page.locator('.menu button', { hasText: 'Settings' }).click(); };
  const save = () => page.locator('dialog.settings button', { hasText: 'Save' }).click();
  await tab('Graph');
  await openSettings();
  assert.match(await page.locator('.settings-text').inputValue(), /\[theme\]/);
  await page.fill('.settings-text', '[theme]\nplot_bg=#102030\nplot_fg=white\nfont_size=20\npalette=#ff0000;#00ff00\n[input]\nzoom_step=2\n[keys]\nkeypad=F8\n');
  await shot('15-settings');
  await save();
  await until(() => !document.querySelector('dialog.settings'));
  const corner = () => page.evaluate(() => {
    const c = document.querySelector('.view:not([hidden]) canvas');
    return [...c.getContext('2d').getImageData(1, 1, 1, 1).data].slice(0, 3);
  });
  await page.waitForTimeout(100);
  assert.deepEqual(await corner(), [16, 32, 48], 'plot colors from the ini');
  assert.equal(await page.evaluate(() => window.app.graph.wheelStep), 2, 'zoom step from the ini');
  await page.keyboard.press('F8');
  assert.ok(await page.locator('.keypad').isHidden(), 'keys from the ini');
  await page.keyboard.press('F8');
  await page.keyboard.press('F9');
  assert.ok(await page.locator('.keypad').isVisible(), 'the old key is free');
  await page.reload();
  await ready();
  await page.waitForTimeout(200);
  assert.deepEqual(await corner(), [16, 32, 48], 'settings survive a reload');
  await openSettings();
  await page.fill('.settings-text', '[theme]\nplot_fg=nope\n[keys]\nmine=<Alt>x\n');
  await save();
  await until(() => document.querySelector('.settings-problems').textContent.includes('plot_fg=nope'));
  assert.match(await text('.settings-problems'), /mine: no such action/);
  await page.locator('dialog.settings button', { hasText: 'Defaults' }).click();
  await save();
  await until(() => !document.querySelector('dialog.settings'));
  await openSettings();
  await page.locator('dialog.settings button[role="tab"]', { hasText: 'style.css' }).click();
  await page.fill('.settings-text', '.simple-entry { color: rgb(1, 2, 3); }');
  await save();
  await tab('Calculator');
  await until(() => getComputedStyle(document.querySelector('.simple-entry')).color === 'rgb(1, 2, 3)');
  if (DESKTOP_INI) {
    await openSettings();
    await page.fill('.settings-text', DESKTOP_INI);
    await save();
    await until(() => !document.querySelector('dialog.settings'));
  }
  await openSettings();
  await page.locator('dialog.settings button', { hasText: 'Defaults' }).click();
  await page.locator('dialog.settings button[role="tab"]', { hasText: 'style.css' }).click();
  await page.locator('dialog.settings button', { hasText: 'Defaults' }).click();
  await save();
  await until(() => !document.querySelector('dialog.settings') && getComputedStyle(document.querySelector('.simple-entry')).color !== 'rgb(1, 2, 3)');
  assert.equal(await page.evaluate(() => window.app.graph.wheelStep), 1.15);

  check('keypad: F9, height, and keeping it');
  await page.keyboard.press('F9');
  assert.ok(await page.locator('.keypad').isHidden());
  await page.keyboard.press('F9');
  assert.ok(await page.locator('.keypad').isVisible());

  check('everything comes back after a reload');
  await tab('Calculator');
  await page.fill('.input-row input', '6 × 7');
  await key('Enter');
  await until(() => document.querySelectorAll('.history-card').length === 1);
  await tab('Data');
  await page.waitForTimeout(300);
  await page.reload();
  await ready();
  assert.equal(await page.evaluate(() => window.app.tab), 'Data', 'same tab');
  assert.equal(await cell(4, 1).inputValue(), '9', 'the table');
  assert.equal(await page.locator('.history-card').count(), 1, 'the history');
  assert.equal(await page.locator('.graph-row input[type="text"]').first().inputValue(), '1/x', 'the functions');

  check('a phone-sized screen');
  const phoneLayout = (name) => page.evaluate(() => {
    const visible = (e) => e.offsetParent !== null && getComputedStyle(e).visibility !== 'hidden';
    const out = [];
    if (document.documentElement.scrollWidth > window.innerWidth + 1) out.push('sideways scroll');
    const tabs = document.querySelector('.tabs').getBoundingClientRect();
    const end = document.querySelector('.header-end').getBoundingClientRect();
    if (tabs.right > end.left + 1) out.push(`tabs (to ${tabs.right}) run into the tools (from ${end.left})`);
    for (const b of document.querySelectorAll('.headerbar button, .keypad button')) {
      if (visible(b) && b.scrollWidth > b.clientWidth + 1) out.push(`clipped: ${b.textContent}`);
      if (visible(b) && b.closest('.keypad') && (b.scrollHeight > b.clientHeight + 1 || b.getBoundingClientRect().height < 26)) out.push(`too short (${b.getBoundingClientRect().height}): ${b.textContent}`);
      if (visible(b) && b.getBoundingClientRect().height > 60 && b.closest('.headerbar')) out.push(`wrapped: ${b.textContent}`);
    }
    return out;
  }).then((problems2) => assert.deepEqual(problems2, [], name));
  for (const width of [412, 360]) {
    await page.setViewportSize({ width, height: 800 });
    await tab('Calculator');
    await page.waitForTimeout(150);
    if (width === 412) await shot('10-phone-calculator');
    await phoneLayout(`${width}: calculator`);
    await tab('Graph');
    await page.waitForTimeout(250);
    if (width === 412) await shot('11-phone-graph');
    await phoneLayout(`${width}: graph`);
    await tab('Data');
    await page.waitForTimeout(150);
    if (width === 412) await shot('12-phone-data');
    await phoneLayout(`${width}: data`);
    // the chart keeps a usable size and nothing overlaps: everything is below the chart, reached by scrolling
    const chart = await page.locator('.view:not([hidden]) .board-area').boundingBox();
    assert.ok(chart.height >= 200, `${width}: chart ${chart.height} tall`);
    const [status, stats] = [await page.locator('.view:not([hidden]) .board-status').boundingBox(), await page.locator('.stats').boundingBox()];
    assert.ok(stats.y >= status.y + status.height - 1, `${width}: statistics overlap the chart's status line`);
  }
  await page.setViewportSize({ width: 412, height: 800 });
  await tab('Calculator');
  await page.locator('.side-keys button', { hasText: 'Stats' }).click();
  await page.waitForTimeout(100);
  await shot('14-phone-stats-keys');
  await phoneLayout('phone: stats keys');
  await page.locator('.side-keys button', { hasText: 'Basic' }).click();
  await page.setViewportSize({ width: 1920, height: 1080 });
  await tab('Graph');
  await page.waitForTimeout(250);
  await shot('13-board-graph');

  await page.setViewportSize({ width: 1366, height: 860 });
  };

  check('loads and starts the engine');
  await page.goto(url);
  await ready();
  await features();

  if (!process.env.SITE_URL) {
    check('the server hiccups: the engine and the offline copy still arrive');
    const flaky = await browser.newContext({ viewport: { width: 1366, height: 860 } });
    const f = await flaky.newPage();
    Object.assign(engineTrouble, { failNext: 3, requests: 0 });
    await f.goto(url);
    await f.waitForFunction(() => window.app?.engine?.ready === true, undefined, { timeout: 60000 });
    await f.fill('.input-row input', '2 + 2');
    await f.waitForFunction(() => document.querySelector('.preview').textContent === '= 4', undefined, { timeout: 30000 });
    assert.ok(engineTrouble.requests > 3, `tried again after the failures (${engineTrouble.requests} requests)`);
    await f.evaluate(() => navigator.serviceWorker.ready);
    await untilAsync(f, async () => {
      const names = await caches.keys();
      return names.length === 1 && (await (await caches.open(names[0])).keys()).some((r) => r.url.endsWith('engine/qalc.wasm'));
    }, 'the engine in the offline copy after the hiccup');
    await flaky.close();

    check('the server stays down: the app says so, and recovers by itself');
    const gone = await browser.newContext({ viewport: { width: 1366, height: 860 } });
    const g = await gone.newPage();
    engineTrouble.down = true;
    await g.goto(url);
    await g.waitForFunction(() => document.querySelector('.message').textContent.includes('could not load'), undefined, { timeout: 60000 });
    assert.match(await g.locator('.message').first().innerText(), /503/);
    await g.screenshot({ path: join(shots, '15-server-down.png') });
    engineTrouble.down = false;
    await g.waitForFunction(() => window.app?.engine?.ready === true && document.querySelector('.message').textContent === '', undefined, { timeout: 60000 });
    await g.fill('.input-row input', '2 + 2');
    await g.waitForFunction(() => document.querySelector('.preview').textContent === '= 4', undefined, { timeout: 30000 });
    await gone.close();
  }

  check('touch: tap, flick, pinch and double-tap with real fingers');
  const touchContext = await browser.newContext({ viewport: { width: 1366, height: 860 }, hasTouch: true });
  const t = await touchContext.newPage();
  t.on('pageerror', (e) => problems.push(`touch pageerror: ${e.message}`));
  await t.goto(url);
  await t.waitForFunction(() => window.app?.engine?.ready === true);
  // the offline copy, then fingers on it with the network gone
  await t.evaluate(() => navigator.serviceWorker.ready);
  await t.reload();
  await untilAsync(t, copyReady, 'the offline copy for touch');
  await touchContext.setOffline(true);
  await t.reload();
  await t.waitForFunction(() => window.app?.engine?.ready === true);
  assert.equal(await t.inputValue('.input-row input'), '');
  assert.equal(await t.locator('.input-row input').evaluate((e) => e.inputMode), 'none', 'the keypad replaces the on-screen keyboard');
  await t.locator('.keypad button', { hasText: /^7$/ }).tap();
  await t.locator('.keypad button', { hasText: /^x²$/ }).tap();
  assert.equal(await t.inputValue('.input-row input'), '7□²', 'keys type at the cursor');
  await t.fill('.input-row input', '');
  await t.locator('.tabs button', { hasText: 'Graph' }).tap();
  await t.locator('.graph-row input[type="text"]').first().fill('x²');
  await t.waitForFunction(() => window.app.graph.axes.series.length >= 1);
  const cdp = await touchContext.newCDPSession(t);
  const fingers = (type, points) => cdp.send('Input.dispatchTouchEvent', { type, touchPoints: points.map(([x, y], id) => ({ x, y, id })) });
  const area = await t.locator('.view:not([hidden]) canvas').boundingBox();
  const [cx, cy] = [area.x + area.width / 2, area.y + area.height / 2];
  const range = () => t.evaluate(() => window.app.graph.range());
  await t.waitForTimeout(100);
  const r0 = await range();
  await fingers('touchStart', [[cx - 40, cy], [cx + 40, cy]]);
  for (let i = 1; i <= 8; i++) { await fingers('touchMove', [[cx - 40 - i * 15, cy], [cx + 40 + i * 15, cy]]); await t.waitForTimeout(16); }
  await fingers('touchEnd', []);
  await t.waitForTimeout(100);
  const r1 = await range();
  assert.ok(r1[1] - r1[0] < (r0[1] - r0[0]) * 0.6, `pinch out zooms in: ${r0} -> ${r1}`);
  assert.ok(Math.abs((r1[0] + r1[1]) / 2 - (r0[0] + r0[1]) / 2) < (r0[1] - r0[0]) * 0.1, 'about the fingers');
  await fingers('touchStart', [[cx, cy]]);
  for (let i = 1; i <= 6; i++) { await fingers('touchMove', [[cx - i * 40, cy]]); await t.waitForTimeout(16); }
  await fingers('touchEnd', []);
  const r2 = await range();
  await t.waitForTimeout(250);
  const r3 = await range();
  assert.ok(r3[0] > r2[0], `a flick keeps gliding: ${r2} -> ${r3}`);
  await t.waitForTimeout(1500);
  await t.locator('.view:not([hidden]) canvas').tap({ position: { x: area.width / 2, y: area.height / 2 } });
  await t.waitForTimeout(60);
  await t.locator('.view:not([hidden]) canvas').tap({ position: { x: area.width / 2, y: area.height / 2 } });
  await t.waitForFunction(() => { const r = window.app.graph.axes.xlim; return r[0] === -10 && r[1] === 10; });
  const handle = await t.locator('.view:not([hidden]) .pane-handle').boundingBox();
  const before2 = await t.locator('.graph-side').evaluate((e) => e.clientWidth);
  await fingers('touchStart', [[handle.x + 5, handle.y + 200]]);
  for (let i = 1; i <= 5; i++) { await fingers('touchMove', [[handle.x + 5 + i * 30, handle.y + 200]]); await t.waitForTimeout(16); }
  await fingers('touchEnd', []);
  await t.waitForTimeout(150);
  const after2 = await t.locator('.graph-side').evaluate((e) => e.clientWidth);
  assert.ok(after2 > before2 + 100, `dragging the divider widens the list: ${before2} -> ${after2}`);
  const eq2 = await t.locator('.graph-row input[type="text"]').first().evaluate((e) => parseFloat(getComputedStyle(e).fontSize));
  await t.setViewportSize({ width: 1800, height: 900 });
  await t.waitForTimeout(150);
  const widened = await t.locator('.graph-side').evaluate((e) => e.clientWidth);
  assert.ok(widened > after2, 'the panes keep their proportions in a bigger window');
  assert.ok(await t.locator('.graph-row input[type="text"]').first().evaluate((e) => parseFloat(getComputedStyle(e).fontSize)) > eq2, 'and the equations grow with them');
  const grip = await t.locator('.keypad-grip').boundingBox();
  const kh = await t.locator('.keypad').evaluate((e) => e.offsetHeight);
  await fingers('touchStart', [[grip.x + grip.width / 2, grip.y + 5]]);
  for (let i = 1; i <= 5; i++) { await fingers('touchMove', [[grip.x + grip.width / 2, grip.y + 5 - i * 15]]); await t.waitForTimeout(16); }
  await fingers('touchEnd', []);
  await t.waitForTimeout(100);
  assert.ok(await t.locator('.keypad').evaluate((e) => e.offsetHeight) > kh + 40, 'dragging the grip resizes the keypad');
  await touchContext.close();

  if (!process.env.SITE_URL) {
    check('an update replaces the offline copy');
    await page.reload();
    await untilAsync(page, copyReady, 'the offline copy before the update');
    const before = await page.evaluate(() => caches.keys());
    engineTrouble.newVersion = '-next';
    await page.evaluate(async () => (await navigator.serviceWorker.getRegistration()).update());
    await untilAsync(page, async () => { const k = await caches.keys(); return k.length === 1 && k[0].endsWith('-next'); }, 'the new offline copy, the old one gone');
    assert.notDeepEqual(await page.evaluate(() => caches.keys()), before);
    await untilAsync(page, copyReady, 'the new version in charge');
    await page.reload();
    await ready();
    await tab('Calculator');
    await page.fill('.input-row input', '2 + 3');
    await previewIs('= 5');
    await page.fill('.input-row input', '');
  }

  check('offline: the network really goes away');
  await page.evaluate(() => navigator.serviceWorker.ready);
  await page.reload();
  await untilAsync(page, copyReady, 'the offline copy');
  // locally the server itself goes away; GitHub's can't be stopped, so there the browser is told it's offline
  if (server) {
    server.closeAllConnections();
    await new Promise((r) => server.close(r));
    serverStopped = true;
    // really gone, seen from outside the browser; only the offline copy can answer now
    await assert.rejects(fetch(url), undefined, 'the server is still up');
    assert.equal(await page.evaluate(() => fetch('keys.js').then((r) => r.status)), 200, 'the offline copy answers');
  } else {
    await context.setOffline(true);
    assert.equal(await page.evaluate(() => navigator.onLine), false);
  }
  // from a clean slate: nothing left over from the online pass, only the offline copy
  await page.evaluate(() => localStorage.clear());
  await page.setViewportSize({ width: 1366, height: 860 });
  await page.reload();
  await ready();
  const offlineTab = await context.newPage();
  await offlineTab.goto(url);
  await offlineTab.waitForFunction(() => window.app?.engine?.ready === true, undefined, { timeout: 30000 });
  await offlineTab.close();
  pass = 'offline: ';
  await features();

  assert.deepEqual(problems, [], 'no page errors');
  console.log(`\nAll ${steps.length} checks passed.`);
} catch (e) {
  await page.screenshot({ path: join(shots, 'failure.png') }).catch(() => {});
  console.error(`\nFAILED at "${step}":\n${e.stack}`);
  console.error('page problems:', problems);
  writeFileSync(join(shots, 'failure.txt'), `${step}\n${e.stack}\n${problems.join('\n')}\n`);
  process.exitCode = 1;
} finally {
  await browser.close();
  if (!serverStopped) server?.close();
}
