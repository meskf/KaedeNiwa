// 页面启动脚本（独立文件，配合 CSP script-src 'self'）。
//
// LiveKit E2EE worker 与页面同源创建；保持 async 接口让 Rust 侧继续 await。
window.__kaedeMakeE2EEWorker = async function (url) {
  return new Worker(url);
};

if (
  'serviceWorker' in navigator &&
  (location.protocol === 'https:' ||
    location.hostname === 'localhost' ||
    location.hostname === '127.0.0.1')
) {
  window.addEventListener('load', () => {
    navigator.serviceWorker
      .register('./sw.js')
      .catch((e) => console.warn('SW register failed:', e));
  });
}
