// The network going away at any moment during an update: the app must still open, with one complete copy.
// Each round: a browser with the offline copy, a new version on the server, and the server stopped some
// milliseconds into the update. Then reload.
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFileSync, statSync } from 'node:fs';
import { extname, join } from 'node:path';
import { chromium, firefox, webkit } from 'playwright';

const dist = new URL('../dist/', import.meta.url).pathname;
const MIME = { '.html': 'text/html', '.js': 'text/javascript', '.mjs': 'text/javascript', '.css': 'text/css', '.wasm': 'application/wasm', '.png': 'image/png', '.webmanifest': 'application/manifest+json' };

function serve() {
  const state = { version: '' };
  const server = createServer((req, res) => {
    const path = decodeURIComponent(new URL(req.url, 'http://x').pathname);
    let file = join(dist, path.slice('/edu-calc/'.length));
    if (path.endsWith('/')) file = join(file, 'index.html');
    try { statSync(file); } catch { res.writeHead(404).end(); return; }
    let body = readFileSync(file);
    if (path.endsWith('/sw.js') && state.version) body = body.toString().replace(/const VERSION = '([^']*)'/, `const VERSION = '$1${state.version}'`);
    res.writeHead(200, { 'content-type': MIME[extname(file)] ?? 'application/octet-stream' }).end(body);
  });
  return new Promise((resolve) => server.listen(0, '127.0.0.1', () => resolve({
    state,
    url: `http://localhost:${server.address().port}/edu-calc/`,
    stop: () => new Promise((r) => { server.closeAllConnections(); server.close(r); }),
  })));
}

const poll = async (page, f, what, ms = 60000) => {
  const end = Date.now() + ms;
  for (;;) {
    if (await page.evaluate(f)) return;
    assert.ok(Date.now() < end, `timed out: ${what}`);
    await new Promise((r) => setTimeout(r, 100));
  }
};
const settled = async () => {
  const r = await navigator.serviceWorker.getRegistration();
  return r?.active?.state === 'activated' && !r.installing && !r.waiting && navigator.serviceWorker.controller !== null;
};

let failed = 0;
for (const [name, type, rounds] of [['chromium', chromium, [0, 20, 50, 100, 150, 200, 300, 400, 600, 800, 1200, 2000, 3000, 5000]], ['firefox', firefox, [0, 100, 400, 1500]], ['webkit', webkit, [0, 100, 400, 1500]]]) {
  const browser = await type.launch();
  for (const delay of rounds) {
    const site = await serve();
    const context = await browser.newContext();
    const page = await context.newPage();
    const start = Date.now();
    let state = '';
    try {
      await page.goto(site.url);
      await page.waitForFunction(() => window.app?.engine?.ready === true, undefined, { timeout: 60000 });
      await poll(page, settled, 'the first offline copy');
      await page.reload();
      await poll(page, settled, 'the page under the service worker');
      site.state.version = '-next';
      await page.evaluate(() => { navigator.serviceWorker.getRegistration().then((r) => r.update()); });
      await new Promise((r) => setTimeout(r, delay));
      await site.stop();
      state = await page.evaluate(async () => {
        const r = await navigator.serviceWorker.getRegistration();
        return JSON.stringify({ active: r?.active?.state, waiting: r?.waiting?.state, installing: r?.installing?.state, caches: await caches.keys() });
      });
      const t = Date.now();
      await page.reload({ timeout: 25000 });
      await page.waitForFunction(() => window.app?.engine?.ready === true, undefined, { timeout: 25000 });
      await page.fill('.input-row input', '6 × 7');
      await page.waitForFunction(() => document.querySelector('.preview').textContent === '= 42', undefined, { timeout: 15000 });
      console.log(`${name} +${delay} ms: opened in ${Date.now() - t} ms  ${state}`);
    } catch (e) {
      failed++;
      console.log(`${name} +${delay} ms: FAILED after ${Date.now() - start} ms  ${state}\n  ${e.message.split('\n')[0]}`);
    } finally {
      await context.close();
      await site.stop().catch(() => {});
    }
  }
  await browser.close();
}
console.log(failed ? `${failed} rounds failed` : 'every round opened');
process.exitCode = failed ? 1 : 0;
