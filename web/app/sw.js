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
  e.waitUntil(caches.open(CACHE).then((c) => Promise.all(FILES.map((f) => keep(c, f)))).then(() => self.skipWaiting()));
});

self.addEventListener('activate', (e) => {
  e.waitUntil(caches.keys()
    .then((keys) => Promise.all(keys.filter((k) => k.startsWith('educalc-') && k !== CACHE).map((k) => caches.delete(k))))
    .then(() => self.clients.claim()));
});

self.addEventListener('fetch', (e) => {
  const { request } = e;
  if (request.method !== 'GET' || new URL(request.url).origin !== self.location.origin) return;
  e.respondWith(caches.match(request, { ignoreSearch: true }).then((hit) => {
    if (hit) return hit;
    // the app's address itself, also when opened with a trailing path
    if (request.mode === 'navigate') return caches.match('./index.html').then((page) => page ?? fetch(request));
    return fetch(request);
  }));
});
