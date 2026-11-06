// The core of the app in the other browser engines (Firefox, and WebKit for Safari): the engine, each
// tab, what's kept in storage, and all of it again with the network off. SITE_URL: test what's published.
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdirSync, readFileSync, statSync } from 'node:fs';
import { extname, join } from 'node:path';
import { firefox, webkit } from 'playwright';

const dist = new URL('../dist/', import.meta.url).pathname;
const shots = new URL('../shots/', import.meta.url).pathname;
mkdirSync(shots, { recursive: true });
const MIME = { '.html': 'text/html', '.js': 'text/javascript', '.mjs': 'text/javascript', '.css': 'text/css', '.wasm': 'application/wasm', '.png': 'image/png', '.webmanifest': 'application/manifest+json' };
/** The site on a port of its own; stop() is the network really going away, whatever the browser. */
async function serve() {
  const sockets = new Set();
  const server = createServer((req, res) => {
    const path = decodeURIComponent(new URL(req.url, 'http://x').pathname);
    if (!path.startsWith('/edu-calc/')) { res.writeHead(404).end(); return; }
    let file = join(dist, path.slice('/edu-calc/'.length));
    if (path.endsWith('/')) file = join(file, 'index.html');
    try { statSync(file); } catch { res.writeHead(404).end(); return; }
    res.writeHead(200, { 'content-type': MIME[extname(file)] ?? 'application/octet-stream' }).end(readFileSync(file));
  });
  server.on('connection', (c) => { sockets.add(c); c.on('close', () => sockets.delete(c)); });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  const stop = () => new Promise((r) => { server.close(r); for (const c of sockets) c.destroy(); });
  return { url: `http://localhost:${server.address().port}/edu-calc/`, stop };
}

let failed = false;
for (const [name, type] of [['firefox', firefox], ['webkit', webkit]]) {
  const site = process.env.SITE_URL ? { url: process.env.SITE_URL, stop: () => {} } : await serve();
  const browser = await type.launch();
  const context = await browser.newContext({ viewport: { width: 1366, height: 860 } });
  const page = await context.newPage();
  const problems = [];
  page.on('pageerror', (e) => problems.push(e.message));
  const until = (f, arg) => page.waitForFunction(f, arg, { timeout: 60000 });
  const entry = '.input-row input';
  const key = (face) => page.locator('.keypad button', { hasText: new RegExp(`^${face}$`) }).first().click();
  const tab = (t) => page.locator('.tabs button', { hasText: t }).click();
  const ready = () => until(() => window.app?.engine?.ready === true);
  const previewIs = (t) => until((want) => document.querySelector('.preview').textContent === want, t);
  let step = '';
  const check = (s) => { step = s; console.log(`${name}: ${s}`); };
  try {
    check('starts');
    await page.goto(site.url);
    await ready();

    check('calculator');
    await page.fill(entry, '3/4 + 1/2');
    await previewIs('= 5/4  ≈ 1.25');
    await key('Enter');
    await until(() => document.querySelectorAll('.history-card').length === 1);
    await key('sin');
    assert.equal(await page.inputValue(entry), 'sin(□)');
    await key('3');
    await key('0');
    await previewIs('= 1/2  ≈ 0.5');
    await page.fill(entry, '5 km to mi');
    await previewIs('= 3.106855961 mi');
    await page.fill(entry, 'integrate(x^2, 0, 3)');
    await previewIs('= 9');
    await page.fill(entry, '');

    check('graph');
    await tab('Graph');
    const row = page.locator('.graph-row input[type="text"]').first();
    await row.fill('y = x² − 1');
    await until(() => window.app.graph.axes.series.length === 1);
    await page.waitForTimeout(200);
    assert.ok(await page.evaluate(() => {
      const c = document.querySelector('.view:not([hidden]) canvas');
      const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
      const seen = new Set();
      for (let i = 0; i < d.length; i += 4000) seen.add(`${d[i]},${d[i + 1]},${d[i + 2]}`);
      return seen.size > 2;
    }), 'the graph drew');
    const r0 = await page.evaluate(() => window.app.graph.range());
    const box = await page.locator('.view:not([hidden]) canvas').boundingBox();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.wheel(0, 300);
    await page.waitForTimeout(200);
    const r1 = await page.evaluate(() => window.app.graph.range());
    assert.ok(r1[1] - r1[0] > r0[1] - r0[0], 'wheel zooms');
    await page.locator('.board-tools button').nth(2).click();
    await page.locator('.board-tools button').nth(3).click();
    await page.waitForTimeout(200);
    await page.screenshot({ path: join(shots, `${name}-graph.png`) });
    await row.fill('sin(');
    await until(() => document.querySelector('.graph-error').textContent.includes('missing a closing'));
    await row.fill('y = x² − 1');
    await until(() => window.app.graph.axes.series.length === 1);

    check('data');
    await tab('Data');
    await page.evaluate(() => { window.app.table.setGrid([['X', 'Y'], ['1', '2'], ['2', '4'], ['3', '5'], ['4', '9']], [1, 0]); window.app.table.changed(); });
    await until(() => document.querySelector('.stats').textContent.includes('r = 0.9648'));
    const book = await page.evaluate(async () => {
      const X = await import('./vendor/xlsx.mjs');
      const wb = X.utils.book_new();
      X.utils.book_append_sheet(wb, X.utils.aoa_to_sheet([['Height', 'Mass'], [1.5, 50], [1.7, 70]]), 'People');
      return [...new Uint8Array(X.write(wb, { type: 'array', bookType: 'xlsx' }))];
    });
    await page.locator('.file').setInputFiles({ name: 'book.xlsx', mimeType: 'application/octet-stream', buffer: Buffer.from(book) });
    await until(() => window.app.table.g[0][0] === 'Height' && window.app.table.g[2][1] === '70');
    await page.evaluate(() => { window.app.table.setGrid([['X', 'Y'], ['1', '2'], ['2', '4'], ['3', '5'], ['4', '9']], [1, 0]); window.app.table.changed(); });
    await until(() => document.querySelector('.stats').textContent.includes('r = '));

    check('table in the calculator');
    await tab('Calculator');
    await until(() => JSON.stringify(window.app.sharedNames) === '["X","Y"]');
    await page.fill(entry, 'slope(X, Y)');
    await previewIs('= 11/5  ≈ 2.2');
    await page.fill(entry, '');

    check('kept after a reload');
    await page.waitForTimeout(400);
    await page.reload();
    await ready();
    assert.equal(await page.locator('.history-card').count(), 1, 'history');
    assert.equal(await page.evaluate(() => window.app.table.g[4][1]), '9', 'table');
    assert.equal(await page.locator('.graph-row input[type="text"]').first().inputValue(), 'y = x² − 1', 'functions');

    check('a broken engine is replaced');
    await page.workers().at(-1).evaluate(() => {
      const split = String.prototype.split;
      String.prototype.split = function (...a) {
        if (String(this).includes('20071')) throw new WebAssembly.RuntimeError('a test trap');
        return split.apply(this, a);
      };
    });
    await page.fill(entry, 'mean(X) + 20071');
    await until(() => document.querySelector('.preview').textContent.includes('20073.5'));
    await page.fill(entry, '');

    check('offline');
    await page.evaluate(() => navigator.serviceWorker.ready);
    // waitForFunction would take an async check's promise as already true
    for (let i = 0; ; i++) {
      const ok = await page.evaluate(async () => {
        const r = await navigator.serviceWorker.getRegistration();
        return r?.active?.state === 'activated' && !r.installing && !r.waiting && navigator.serviceWorker.controller !== null && (await caches.keys()).length === 1;
      });
      if (ok) break;
      assert.ok(i < 600, 'the offline copy');
      await page.waitForTimeout(100);
    }
    await site.stop();
    // WebKit's simulated offline fails a reload before its service worker gets to answer
    if (name !== 'webkit' || process.env.SITE_URL) await context.setOffline(true);
    await page.reload();
    await ready();
    await page.fill(entry, 'sqrt(144) + mean(X)');
    await previewIs('= 29/2  ≈ 14.5');
    await key('Enter');
    await until(() => document.querySelectorAll('.history-card').length === 2);
    await tab('Graph');
    await page.locator('.graph-row input[type="text"]').first().fill('x³');
    await until(() => window.app.graph.axes.series[0]?.label === 'x³');
    await tab('Data');
    await page.locator('.file').setInputFiles({ name: 'book.xlsx', mimeType: 'application/octet-stream', buffer: Buffer.from(book) });
    await until(() => window.app.table.g[0][0] === 'Height');
    await page.screenshot({ path: join(shots, `${name}-offline-data.png`) });

    check('offline: functions, exports, history, appearance, settings');
    await tab('Calculator');
    await page.locator('.header-end button', { hasText: 'Functions' }).click();
    await page.fill('.catalog input', 'remainder');
    await page.keyboard.press('Enter');
    assert.equal(await page.inputValue(entry), 'mod(□, □)');
    await page.fill(entry, '');
    const menu = (item) => page.locator('.header-end button[aria-label="Menu"]').click().then(() => page.locator('.menu button', { hasText: item }).click());
    await tab('Data');
    await until(() => document.querySelector('.selectors select') !== null);
    for (const [button, file] of [['PNG', 'graph.png'], ['SVG', 'graph.svg']]) {
      const [d] = await Promise.all([page.waitForEvent('download'), (async () => { await menu('Export chart'); await page.locator('dialog button', { hasText: button }).click(); })()]);
      assert.equal(d.suggestedFilename(), file);
    }
    const [csv] = await Promise.all([page.waitForEvent('download'), (async () => { await page.locator('[data-do="export"]').click(); await page.locator('dialog button', { hasText: 'CSV' }).click(); })()]);
    assert.equal(csv.suggestedFilename(), 'data.csv');
    await page.locator('[data-do="addRow"]').click();
    await page.locator('[data-do="delRow"]').click();
    await tab('Calculator');
    await page.locator('.clear-history').click();
    await page.locator('dialog button', { hasText: 'Clear History' }).click();
    await until(() => document.querySelectorAll('.history-card').length === 0);
    await menu('Dark');
    assert.equal(await page.evaluate(() => document.documentElement.dataset.theme), 'dark');
    await menu('Light');
    await menu('Settings');
    await page.fill('.settings-text', '[theme]\nplot_bg=#102030\n');
    await page.locator('dialog.settings button', { hasText: 'Save' }).click();
    await until(() => !document.querySelector('dialog.settings'));
    assert.deepEqual(await page.evaluate(() => window.app.config.plot.bg), [16, 32, 48]);
    await page.keyboard.press('F9');
    assert.ok(await page.locator('.keypad').isHidden());
    await page.keyboard.press('F9');
    assert.deepEqual(problems.filter((p) => !p.includes('a test trap')), [], 'no page errors');
    console.log(`${name}: all passed`);
  } catch (e) {
    failed = true;
    console.error(`\n${name} FAILED at "${step}":\n${e.stack}\npage errors: ${problems.join(' | ')}\n`);
    await page.screenshot({ path: join(shots, `${name}-failure.png`) }).catch(() => {});
  } finally {
    await browser.close();
    await site.stop();
  }
}
process.exitCode = failed ? 1 : 0;
