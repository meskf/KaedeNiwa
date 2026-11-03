import { test, expect } from '../lib/fixtures';

// 这条先单独跑，验证 headless Chromium 下 getDisplayMedia 是否可用。
// 若失败，说明 playwright.l4.config.ts 的 launchOptions.args 没生效，
// 需要切 headed 或调 flags。
test('preflight: headless getDisplayMedia 能拿到真实 MediaStream', async ({ u0 }) => {
  const r = await u0.evaluate(async () => {
    try {
      const s = await navigator.mediaDevices.getDisplayMedia({ video: true, audio: false });
      const t = s.getVideoTracks()[0];
      const settings = t.getSettings();
      s.getTracks().forEach(tr => tr.stop());
      return { ok: true, settings };
    } catch (e: any) {
      return { ok: false, error: String(e), name: e?.name };
    }
  });
  expect(r.ok, 'getDisplayMedia 失败: ' + JSON.stringify(r)).toBe(true);
  expect(r.settings?.width ?? 0).toBeGreaterThan(0);
  expect(r.settings?.height ?? 0).toBeGreaterThan(0);
});
