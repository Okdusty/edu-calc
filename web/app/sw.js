// Offline: every file of the app goes in the cache on install, and the cache answers first. The list
// and version are filled in by web/engine/package.mjs, so any change to the app installs a new copy.
const VERSION = '__VERSION__';
const FILES = __FILES__;
const CACHE = `educalc-${VERSION}`;

// fresh from the server, not the browser's HTTP cache (no mix of two versions), and a hiccup of the
// server on one file doesn't cost the whole offline copy
async function keep(cache, file) {
  for (let attempt = 1; ; attempt++) {
    try {
      const response = await fetch(new Request(file, { cache: 'reload' }));
      if (!response.ok) throw new Error(`${file}: ${response.status}`);
      await cache.put(file, response);
      return;
    } catch (e) {
      if (attempt === 4) throw e;
      await new Promise((r) => setTimeout(r, attempt * 2000));
    }
  }
}

self.addEventListener('install', (e) => {
  // never half a copy: the version in charge stays until this one is complete
  e.waitUntil(caches.open(CACHE).then((c) => Promise.all(FILES.map((f) => keep(c, f)))).catch(async (err) => {
    await caches.delete(CACHE);
    throw err;
  }));
});

// A complete new version waits: it takes over when a page opening asks (main.js), never under a loading page.
self.addEventListener('message', (e) => {
  if (e.data === 'activate') self.skipWaiting();
});

self.addEventListener('activate', (e) => {
  e.waitUntil(caches.keys()
    .then((keys) => Promise.all(keys.filter((k) => k.startsWith('educalc-') && k !== CACHE).map((k) => caches.delete(k))))
    .then(() => self.clients.claim()));
});

self.addEventListener('fetch', (e) => {
  const { request } = e;
  if (request.method !== 'GET' || new URL(request.url).origin !== self.location.origin) return;
  const mine = { ignoreSearch: true, cacheName: CACHE };
  e.respondWith(caches.match(request, mine).then((hit) => {
    if (hit) return hit;
    // the app's address itself, also when opened with a trailing path
    if (request.mode === 'navigate') return caches.match('./index.html', mine).then((page) => page ?? fetch(request));
    return fetch(request);
  }));
});
