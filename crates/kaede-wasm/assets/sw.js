// KaedeNiwa Service Worker
// 策略：
// - 壳（index.html / manifest / icon）：precache + cache-first
// - navigation 请求：network-first，离线回退缓存的 index
// - LiveKit SDK / E2EE worker（CDN）：precache + cache-first，保证离线语音可用
// - 同源静态资源（JS / WASM / CSS / 字体）：cache-first，后台更新
// - API / WS：完全 bypass（保证消息新鲜）

const CACHE = 'kaede-v3';
const SHELL = [
  './',
  './index.html',
  './manifest.json',
  './icon.svg',
  './boot.js',
  './livekit-client.umd.min.js',
  './livekit-client.e2ee.worker.js',
];

self.addEventListener('install', (e) => {
  e.waitUntil(
    caches.open(CACHE).then(async (c) => {
      // 逐项缓存，任一失败不影响其余资源。
      await Promise.all(
        SHELL.map((url) =>
          c.add(url).catch(() => console.warn('[sw] precache failed:', url))
        )
      );
    })
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

  if (req.method !== 'GET') return;

  // API / WS：直连
  if (url.origin === self.location.origin &&
      (url.pathname.startsWith('/api/') || url.pathname === '/ws')) {
    return;
  }

  // 跨域其他资源：直连（LiveKit SDK/worker 已改为同源，走下面的静态资源策略）
  if (url.origin !== self.location.origin) return;

  // 页面导航：network-first，离线回退 index
  if (req.mode === 'navigate') {
    e.respondWith(
      fetch(req)
        .then((res) => {
          const copy = res.clone();
          caches.open(CACHE).then((c) => c.put('./index.html', copy)).catch(() => {});
          return res;
        })
        .catch(() => caches.match('./index.html').then((r) => r || caches.match('./')))
    );
    return;
  }

  // 壳：cache-first
  const isShell =
    url.pathname.endsWith('/index.html') ||
    url.pathname.endsWith('/manifest.json') ||
    url.pathname.endsWith('/icon.svg');

  if (isShell) {
    e.respondWith(caches.match(req).then((cached) => cached || fetch(req)));
    return;
  }

  // 静态资源：cache-first + 后台刷新
  e.respondWith(
    caches.match(req).then((cached) => {
      const network = fetch(req)
        .then((res) => {
          if (res.ok && res.type === 'basic') {
            const copy = res.clone();
            caches.open(CACHE).then((c) => c.put(req, copy)).catch(() => {});
          }
          return res;
        })
        .catch(() => cached);
      return cached || network;
    })
  );
});
