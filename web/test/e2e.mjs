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
const server = createServer((req, res) => {
  const path = decodeURIComponent(new URL(req.url, 'http://x').pathname);
  if (!path.startsWith('/edu-calc/')) { res.writeHead(404).end(); return; }
  let file = join(dist, path.slice('/edu-calc/'.length));
  if (path.endsWith('/')) file = join(file, 'index.html');
  try { statSync(file); } catch { res.writeHead(404).end(); return; }
  res.writeHead(200, { 'content-type': MIME[extname(file)] ?? 'application/octet-stream' }).end(readFileSync(file));
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const url = `http://127.0.0.1:${server.address().port}/edu-calc/`;

const browser = await chromium.launch();
const context = await browser.newContext({ viewport: { width: 1366, height: 860 }, deviceScaleFactor: 2, acceptDownloads: true, permissions: ['clipboard-read', 'clipboard-write'] });
const page = await context.newPage();
const problems = [];
page.on('pageerror', (e) => problems.push(`pageerror: ${e.message}`));
page.on('console', (m) => { if (m.type() === 'error') problems.push(`console: ${m.text()}`); });
page.on('dialog', (d) => d.dismiss());

let step = 'start';
const steps = [];
const check = (name) => { step = name; steps.push(name); console.log(`- ${name}`); };
const shot = (name) => page.screenshot({ path: join(shots, `${name}.png`) });
const until = (f, arg) => page.waitForFunction(f, arg, { timeout: 30000 });
const text = (sel) => page.locator(sel).first().innerText();
const key = (face) => page.locator('.keypad button', { hasText: new RegExp(`^${face.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`) }).first().click();
const keys = async (...faces) => { for (const f of faces) await key(f); };
const ready = () => until(() => window.app?.engine?.ready === true);
const previewIs = (t) => until((want) => document.querySelector('.preview').textContent === want, t);
const tab = (name) => page.locator('.tabs button', { hasText: name }).click();
const statsHas = (t) => until((want) => document.querySelector('.stats').textContent.includes(want), t);

try {
  check('loads and starts the engine');
  await page.goto(url);
  await ready();
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
  const [dl] = await Promise.all([page.waitForEvent('download'), (async () => { await page.locator('[data-do="export"]').click(); await page.locator('dialog button', { hasText: 'CSV' }).click(); })()]);
  assert.equal(dl.suggestedFilename(), 'data.csv');
  assert.equal(readFileSync(await dl.path(), 'utf8'), 'A\n9\n');
  await page.evaluate(() => { window.app.table.setGrid([['X', 'Y'], ['1', '2'], ['2', '4'], ['3', '5'], ['4', '9']], [1, 0]); window.app.table.changed(); });
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

  check('offline: the whole app, with the network gone');
  await page.evaluate(() => navigator.serviceWorker.ready);
  await until(() => navigator.serviceWorker.controller !== null || (location.reload(), false)).catch(() => {});
  await page.waitForTimeout(500);
  await until(async () => (await caches.keys()).some((k) => k.startsWith('educalc-')));
  await context.setOffline(true);
  await page.reload();
  await ready();
  assert.equal(await page.evaluate(() => navigator.onLine), false);
  await tab('Calculator');
  await page.fill('.input-row input', 'sqrt(144) + mean(X)');
  await previewIs('= 14.5');
  await key('Enter');
  await until(() => document.querySelectorAll('.history-card').length === 2);
  await tab('Graph');
  await page.locator('.graph-row input[type="text"]').first().fill('y = x³ − 3x');
  await until(() => window.app.graph.axes.series.length >= 1 && window.app.graph.axes.series[0].label.includes('x³'));
  await page.locator('.board-tools button').nth(3).click();
  await page.waitForTimeout(300);
  await shot('08-offline-graph');
  await page.locator('.header-end button', { hasText: 'Functions' }).click();
  assert.ok(await page.locator('.catalog-row').count() > 50);
  await page.keyboard.press('Escape');
  await tab('Data');
  await page.locator('.file').setInputFiles({ name: 'book.xlsx', mimeType: 'application/octet-stream', buffer: Buffer.from(book) }).catch(() => {});
  await until(() => window.app.table.g[0][0] === 'Height');
  await statsHas('n = 2');
  await page.locator('[data-do="addRow"]').click();
  await shot('09-offline-data');
  const offlineNav = await context.newPage();
  await offlineNav.goto(url);
  await offlineNav.waitForFunction(() => window.app?.engine?.ready === true, undefined, { timeout: 30000 });
  await offlineNav.close();
  await context.setOffline(false);

  check('a phone-sized screen');
  await page.setViewportSize({ width: 412, height: 800 });
  await tab('Calculator');
  await page.waitForTimeout(150);
  await shot('10-phone-calculator');
  assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1), 'no sideways scroll');
  await tab('Graph');
  await page.waitForTimeout(250);
  await shot('11-phone-graph');
  await tab('Data');
  await page.waitForTimeout(150);
  await shot('12-phone-data');
  await page.setViewportSize({ width: 1920, height: 1080 });
  await tab('Graph');
  await page.waitForTimeout(250);
  await shot('13-board-graph');

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
  server.close();
}
