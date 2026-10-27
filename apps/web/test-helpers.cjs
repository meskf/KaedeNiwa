const { chromium } = require('playwright');
const net = require('net');

async function checkPort(port) {
  const tryConnect = (host) => new Promise((resolve) => {
    const socket = net.createConnection(port, host);
    socket.setTimeout(1000);
    socket.on('connect', () => { socket.destroy(); resolve(true); });
    socket.on('timeout', () => { socket.destroy(); resolve(false); });
    socket.on('error', () => resolve(false));
  });
  if (await tryConnect('127.0.0.1')) return true;
  return await tryConnect('::1');
}

async function checkPrereqs() {
  const checks = await Promise.all([
    { name: 'Vite (5173)', ok: await checkPort(5173) },
    { name: 'Kaede Server (18080)', ok: await checkPort(18080) },
    { name: 'LiveKit (7880)', ok: await checkPort(7880) },
  ]);
  for (const c of checks) console.log(`   ${c.ok ? '✅' : '❌'} ${c.name}`);
  if (!checks.every(c => c.ok)) {
    console.log('\n❌ 缺少前置服务。请先执行 .\\start-all.ps1 -SkipTest');
    process.exit(1);
  }
}

async function login(page, name) {
  await page.goto('http://localhost:5173');
  await page.waitForSelector('[data-testid="login-id"]', { timeout: 15000 });
  await page.fill('[data-testid="login-id"]', name);
  await page.fill('[data-testid="login-password"]', 'password123');
  await page.click('[data-testid="login-toggle-register"]');
  await page.click('[data-testid="login-submit"]');
  await page.waitForSelector('input[placeholder^="Message #"]', { timeout: 15000 });
}

async function typeMessage(page, text) {
  await page.click('[data-testid="message-input"]');
  await page.keyboard.type(text);
  await page.keyboard.press('Enter');
}

function randName(prefix) {
  return prefix + '-' + Math.floor(Math.random() * 100000);
}

// 注入动态尺寸的假屏幕共享（支持 480p / 1080p / 4K）
async function injectFakeDisplayMedia(page) {
  await page.addInitScript(() => {
    if (!navigator.mediaDevices) navigator.mediaDevices = {};
    navigator.mediaDevices.getDisplayMedia = async (constraints) => {
      const v = constraints?.video || {};
      const w = v.width?.ideal || 1280;
      const h = v.height?.ideal || 720;
      const fps = v.frameRate?.ideal || 30;
      const canvas = document.createElement('canvas');
      canvas.width = w;
      canvas.height = h;
      const ctx = canvas.getContext('2d');
      let frame = 0;
      const interval = 1000 / fps;
      setInterval(() => {
        ctx.fillStyle = `hsl(${(frame * 7) % 360}, 70%, 50%)`;
        ctx.fillRect(0, 0, w, h);
        ctx.fillStyle = '#fff';
        ctx.font = `bold ${Math.floor(h / 10)}px sans-serif`;
        ctx.fillText(`MOCK ${w}x${h}@${fps}`, 40, h / 2);
        ctx.font = `${Math.floor(h / 20)}px monospace`;
        ctx.fillText('Frame ' + frame, 40, h / 2 + h / 6);
        frame++;
      }, interval);
      return canvas.captureStream(fps);
    };
  });
}

async function setDisableAdaptive(page) {
  await page.addInitScript(() => {
    window.__KAEDE_TEST_DISABLE_ADAPTIVE__ = true;
  });
}

module.exports = { chromium, checkPort, checkPrereqs, login, randName, injectFakeDisplayMedia, setDisableAdaptive, typeMessage };
