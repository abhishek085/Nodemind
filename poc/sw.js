// Nodemind service worker — makes the app installable and offline-capable.
// Strategy: cache the app shell + the CDN JS libraries. Model weights (~1GB) are
// cached by the libraries themselves (Cache Storage), so we deliberately do NOT
// re-cache them here to avoid storing gigabytes twice.

const SHELL = "nodemind-shell-v1";
const RUNTIME = "nodemind-runtime-v1";
const SHELL_FILES = ["./", "./index.html", "./manifest.webmanifest", "./icon-192.png", "./icon-512.png"];
const CACHE_HOSTS = ["esm.run", "cdn.jsdelivr.net", "unpkg.com"];

self.addEventListener("install", (event) => {
  self.skipWaiting();
  event.waitUntil(caches.open(SHELL).then((c) => c.addAll(SHELL_FILES).catch(() => {})));
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches.keys().then((keys) =>
      Promise.all(keys.filter((k) => k !== SHELL && k !== RUNTIME).map((k) => caches.delete(k)))
    ).then(() => self.clients.claim())
  );
});

self.addEventListener("fetch", (event) => {
  const req = event.request;
  if (req.method !== "GET") return;

  const url = new URL(req.url);
  const sameOrigin = url.origin === self.location.origin;
  const cacheCdn = CACHE_HOSTS.some((h) => url.hostname === h || url.hostname.endsWith("." + h));
  if (!sameOrigin && !cacheCdn) return; // model weights etc. → straight to network / lib cache

  const bucket = sameOrigin ? SHELL : RUNTIME;
  event.respondWith(
    caches.open(bucket).then(async (cache) => {
      const cached = await cache.match(req);
      const network = fetch(req)
        .then((res) => {
          if (res && res.status === 200) cache.put(req, res.clone());
          return res;
        })
        .catch(() => cached);
      return cached || network; // offline → cached; online → cache-first, refresh in background
    })
  );
});
