// KaedeNiwa Service Worker（首版）
// 策略：
// - 壳（index.html / manifest / icon）：precache + cache-first
// - API / WS：完全 bypass（保证消息新鲜）
// - 其他静态资源（JS / WASM）：network-first（浏览器 HTTP 缓存兜底）

const CACHE = 'kaede-v1';
const SHELL = ['./', './index.html', './manifest.json', './icon.svg'];

self.addEventListener('install', (e) => {
  e.waitUntil(
    caches.open(CACHE).then((c) => c.addAll(SHELL).catch(() => {}))
  );
  self.skipWaiting();
});

self.addEventListener('activate', (e) => {
  e.waitUntil(
    caches.keys().then((keys) =>
      Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k)))
    )
  );
  self.clients.claim();
});

self.addEventListener('fetch', (e) => {
  const req = e.request;
  const url = new URL(req.url);

  // 非 GET：直连
  if (req.method !== 'GET') return;

  // 跨域（包括 LiveKit / 用户配置的 signaling server）：直连，不缓存
  if (url.origin !== self.location.origin) return;

  // API / WS：直连
  if (url.pathname.startsWith('/api/') || url.pathname === '/ws') return;

  // 壳：cache-first
  const isShell =
    url.pathname === '/' ||
    url.pathname.endsWith('/index.html') ||
    url.pathname.endsWith('/manifest.json') ||
    url.pathname.endsWith('/icon.svg');

  if (isShell) {
    e.respondWith(
      caches.match(req).then((cached) => cached || fetch(req))
    );
    return;
  }

  // 其他静态资源（JS / WASM / 字体）：network-first，失败回退缓存
  e.respondWith(
    fetch(req)
      .then((res) => {
        if (res.ok && res.type === 'basic') {
          const copy = res.clone();
          caches.open(CACHE).then((c) => c.put(req, copy)).catch(() => {});
        }
        return res;
      })
      .catch(() => caches.match(req))
  );
});
