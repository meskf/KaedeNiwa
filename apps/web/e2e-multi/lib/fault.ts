// apps/web/e2e-multi/lib/fault.ts
// P1-b：CDP 层面的网络故障注入。只在 Chromium 生效（Playwright 默认 Chromium，OK）。
//
// 两种粒度：
//   - 网络层：emulateNetwork / blockUrls —— CDP Network domain，影响所有流量
//   - 应用层：pauseWs —— 拦截 WebSocket.prototype.send，精确丢包/延迟
//
// 注意：pauseWs 会同时影响 vite HMR 的 WebSocket。测试里不 reload 页面，无影响。
//       但不要用 dropRate=1 + reload 的组合，会让 HMR 失效后 reload 卡住。

import { BrowserContext, Page, CDPSession } from '@playwright/test';

const sessions = new WeakMap<Page, CDPSession>();

async function cdp(page: Page): Promise<CDPSession> {
  let s = sessions.get(page);
  if (!s) {
    s = await page.context().newCDPSession(page);
    await s.send('Network.enable');
    sessions.set(page, s);
  }
  return s;
}

export type NetworkProfile = {
  latency?: number;
  downloadThroughput?: number; // bytes/s, -1 = unlimited
  uploadThroughput?: number;   // bytes/s, -1 = unlimited
  offline?: boolean;
};

// 常用预设
export const NetProfiles = {
  offline: { offline: true, latency: 0, downloadThroughput: 0, uploadThroughput: 0 } as NetworkProfile,
  slow3g: { latency: 200, downloadThroughput: 50_000, uploadThroughput: 25_000 } as NetworkProfile,
  fast3g: { latency: 100, downloadThroughput: 200_000, uploadThroughput: 100_000 } as NetworkProfile,
  lossy10: { latency: 80, downloadThroughput: 500_000, uploadThroughput: 250_000 } as NetworkProfile,
  clear: { offline: false, latency: 0, downloadThroughput: -1, uploadThroughput: -1 } as NetworkProfile,
};

export async function emulateNetwork(
  ctx: BrowserContext,
  profile: NetworkProfile,
  pages?: Page[],
) {
  const targets = pages ?? ctx.pages();
  for (const p of targets) {
    const s = await cdp(p);
    await s.send('Network.emulateNetworkConditions', {
      offline: !!profile.offline,
      latency: profile.latency ?? 0,
      downloadThroughput: profile.downloadThroughput ?? -1,
      uploadThroughput: profile.uploadThroughput ?? -1,
    });
  }
}

// 阻断 URL 模式：['/ws']、['/api/messages'] 等。传空数组恢复。
// 注意：注释里不要出现 星号紧跟斜杠 的组合，JSDoc 会提前闭合。
export async function blockUrls(ctx: BrowserContext, patterns: string[], pages?: Page[]) {
  const targets = pages ?? ctx.pages();
  for (const p of targets) {
    const s = await cdp(p);
    await s.send('Network.setBlockedURLs', { urls: patterns });
  }
}

/**
 * 应用层 WS 故障注入。
 * - dropRate: 0-1，send 被调用的概率丢弃
 * - sendDelayMs: 每次 send 的延迟（在 drop 之后，二者可叠加）
 *
 * 幂等：重复调用只更新配置，不重复包装。
 */
export async function pauseWs(
  page: Page,
  opts: { dropRate?: number; sendDelayMs?: number } = {},
) {
  const dropRate = opts.dropRate ?? 0;
  const sendDelayMs = opts.sendDelayMs ?? 0;
  await page.evaluate(({ dropRate, sendDelayMs }) => {
    const w = window as any;
    if (!w.__fault_ws_installed__) {
      w.__fault_ws_installed__ = true;
      w.__fault_ws_dropped__ = 0;
      w.__fault_ws_delayed__ = 0;
      const proto = (WebSocket as any).prototype;
      const origSend = proto.send;
      proto.send = function (this: any, data: any) {
        const cfg = w.__fault_ws_cfg__;
        if (cfg.dropRate > 0 && Math.random() < cfg.dropRate) {
          w.__fault_ws_dropped__++;
          return;
        }
        if (cfg.sendDelayMs > 0) {
          w.__fault_ws_delayed__++;
          setTimeout(() => origSend.call(this, data), cfg.sendDelayMs);
          return;
        }
        return origSend.call(this, data);
      };
    }
    w.__fault_ws_cfg__ = { dropRate, sendDelayMs };
  }, { dropRate, sendDelayMs });
}

export async function stopPauseWs(page: Page) {
  await page.evaluate(() => {
    const w = window as any;
    if (w.__fault_ws_installed__) {
      w.__fault_ws_cfg__ = { dropRate: 0, sendDelayMs: 0 };
    }
  });
}

export async function getWsFaultStats(page: Page) {
  return page.evaluate(() => {
    const w = window as any;
    return {
      installed: !!w.__fault_ws_installed__,
      dropped: w.__fault_ws_dropped__ ?? 0,
      delayed: w.__fault_ws_delayed__ ?? 0,
    };
  });
}

/** 统计一条消息在 DOM 上出现的次数。P1-a 里反复用。 */
export async function countMsg(page: Page, text: string): Promise<number> {
  return page.getByText(text, { exact: false }).count();
}
