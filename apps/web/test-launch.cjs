// 统一的浏览器启动函数。
// - 本地 Windows 开发：headless: false + msedge（有 UI，方便调试）
// - CI Linux（KAEDE_HEADLESS=1）：headless: true + Playwright 自带 chromium
const { chromium } = require('playwright');

async function launch(extraOptions = {}) {
  if (process.env.KAEDE_HEADLESS === '1') {
    return chromium.launch({ headless: true, ...extraOptions });
  }
  return chromium.launch({ headless: false, channel: 'msedge', ...extraOptions });
}

module.exports = { launch };
