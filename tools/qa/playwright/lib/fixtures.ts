import { test as base, expect, BrowserContext, Page } from '@playwright/test';

export const randName = (p: string) => `${p}-${Math.floor(Math.random() * 1_000_000)}`;

export async function login(page: Page, loginId: string, password = 'password123') {
  await page.goto('/');
  await page.getByTestId('login-id').waitFor({ state: 'visible', timeout: 30_000 });
  await page.getByTestId('login-id').fill(loginId);
  await page.getByTestId('login-password').fill(password);
  const reg = page.getByTestId('login-toggle-register');
  if (await reg.isVisible()) await reg.click();
  await page.getByTestId('login-submit').click();
  await page.getByTestId('message-input').waitFor({ state: 'visible', timeout: 30_000 });
  await page.waitForFunction(() => !!(window as any).__kaede_debug__, null, { timeout: 20_000 });
  await page.waitForFunction(async () => {
    const dbg = (window as any).__kaede_debug__;
    if (!dbg?.channels?.list) return false;
    try { const l = await dbg.channels.list(); return Array.isArray(l) && l.length > 0; } catch { return false; }
  }, null, { timeout: 30_000 });
}

export async function getFirstVoiceChannel(page: Page): Promise<{ id: string; name: string }> {
  return await page.evaluate(async () => {
    const dbg = (window as any).__kaede_debug__;
    const list: any[] = await dbg.channels.list();
    const v = list.find((c: any) => c.kind === 'voice');
    if (!v) throw new Error('no voice channel: ' + JSON.stringify(list));
    return { id: v.id, name: v.name };
  });
}

export async function enterVoiceChannel(page: Page, chId: string, voiceName: string) {
  const byNodeId = page.locator(`[data-node-id="${chId}"]`);
  if (await byNodeId.count() > 0) {
    await byNodeId.first().waitFor({ state: 'visible', timeout: 30_000 });
    await byNodeId.first().click();
  } else {
    const byText = page.locator('aside').first().getByText(voiceName, { exact: true }).first();
    await byText.waitFor({ state: 'visible', timeout: 30_000 });
    await byText.click();
  }
  const joinBtn = page.getByTestId('media-join');
  await joinBtn.waitFor({ state: 'visible', timeout: 30_000 });
  await joinBtn.click();
  await page.getByTestId('media-connected').waitFor({ state: 'visible', timeout: 90_000 });
  await expect(page.getByTestId('media-connected')).toHaveAttribute('data-channel', chId, { timeout: 15_000 });
}

export async function dumpRoomState(page: Page) {
  return await page.evaluate(async () => {
    const r = (window as any).__kaedeRoom;
    if (!r) return { noRoom: true };
    const remote: any[] = [];
    for (const [id, p] of r.remoteParticipants) {
      remote.push({
        identity: id,
        videoPubs: Array.from((p as any).videoTrackPublications?.values?.() || []).map((pub: any) => ({
          sid: pub.trackSid, source: pub.source, subscribed: pub.isSubscribed, muted: pub.isMuted,
        })),
        audioPubs: Array.from((p as any).audioTrackPublications?.values?.() || []).map((pub: any) => ({
          sid: pub.trackSid, source: pub.source, subscribed: pub.isSubscribed,
        })),
      });
    }
    return {
      name: r.name,
      state: String(r.state),
      identity: r.localParticipant?.identity,
      localVideoPubs: Array.from((r.localParticipant as any).videoTrackPublications?.values?.() || []).map((pub: any) => ({
        sid: pub.trackSid, source: pub.source, muted: pub.isMuted,
      })),
      remoteCount: r.remoteParticipants.size,
      remote,
      e2eeEnabled: !!(r as any).e2eeManager?.isE2EEEnabled,
    };
  });
}

export async function myIdentity(page: Page): Promise<string> {
  return await page.evaluate(async () => {
    // 用页面挂载的实例，避免 HMR 下动态 import 拿到另一个模块实例（currentRoom 为 null）
    const room = (window as any).__kaedeRoom;
    return room?.localParticipant?.identity ?? '';
  });
}

export async function localShareInfo(page: Page) {
  return await page.evaluate(async () => {
    // 用页面挂载的实例，避免 HMR 下动态 import 拿到另一个模块实例（currentRoom 为 null）
    const room = (window as any).__kaedeRoom;
    if (!room) return { has: false, reason: 'no-room' };
    const pubs: any[] = Array.from(room.localParticipant.videoTrackPublications.values());
    const share = pubs.find((p: any) => p.source === 'screen_share');
    if (!share) return { has: false, reason: 'no-screen-share-pub', sources: pubs.map((p: any) => p.source) };
    const mt = share.track?.mediaStreamTrack;
    // 关键：stopScreenShare 的 unpublishTrack 是 async，短暂窗口内 publication 对象
    // 还在 videoTrackPublications Map 里，但底层 mediaStreamTrack 已经是 'ended'。
    // 必须把"track 已 ended"也当作"没有共享"，否则会出现"stop 15s 后 has 仍为 true"的
    // 假阳性 —— 会被误读成"产品没 unpublish / 泄漏"。
    if (!mt || mt.readyState === 'ended') {
      return {
        has: false,
        reason: 'track-ended',
        sid: share.trackSid,
        trackState: mt?.readyState ?? null,
        isMuted: share.isMuted,
      };
    }
    const settings = mt.getSettings();
    return {
      has: true,
      sid: share.trackSid,
      isMuted: share.isMuted,
      settings,
      isScreenShareEnabled: !!room.localParticipant.isScreenShareEnabled,
    };
  });
}

export async function remoteShareInfo(page: Page, fromIdentity: string) {
  return await page.evaluate(async (id) => {
    // 用页面挂载的实例，避免 HMR 下动态 import 拿到另一个模块实例（currentRoom 为 null）
    const room = (window as any).__kaedeRoom;
    if (!room) return { has: false, reason: 'no-room' };
    const p = room.remoteParticipants.get(id);
    if (!p) return { has: false, reason: 'participant-not-found', known: Array.from(room.remoteParticipants.keys()) };
    const pubs: any[] = Array.from(p.videoTrackPublications.values());
    const share = pubs.find((pub: any) => pub.source === 'screen_share');
    if (!share) return { has: false, reason: 'no-share-pub', sources: pubs.map((pub: any) => pub.source) };
    return { has: true, sid: share.trackSid, isSubscribed: share.isSubscribed, isMuted: share.isMuted };
  }, fromIdentity);
}

// 观测层：真帧数（sender framesEncoded / receiver framesDecoded）
export async function videoFrameStats(page: Page, identity: string | null) {
  return await page.evaluate(async (id) => {
    // 用页面挂载的实例，避免 HMR 下动态 import 拿到另一个模块实例（currentRoom 为 null）
    const room = (window as any).__kaedeRoom;
    if (!room) return { ok: false, reason: 'no-room' };
    const pub = id
      ? Array.from(room.remoteParticipants.get(id)?.videoTrackPublications?.values?.() || [])
          .find((p: any) => p.source === 'screen_share')
      : Array.from(room.localParticipant.videoTrackPublications.values())
          .find((p: any) => p.source === 'screen_share');
    if (!pub || !pub.track) return { ok: false, reason: 'no-track' };
    const report = await (pub.track as any).getRTCStatsReport();
    let framesEncoded = 0, framesDecoded = 0, bytesSent = 0, bytesReceived = 0, frameWidth = 0, frameHeight = 0;
    const outboundEntries: any[] = [];
    const inboundEntries: any[] = [];
    report.forEach((s: any) => {
      // Chrome 里 outbound-rtp 的字段是 kind 和 mediaType 并存（不同版本），
      // 优先用 kind，缺省时退回 mediaType，两者都缺时按有 framesEncoded/bytesSent 推断。
      const isVideo =
        s.kind === 'video' || s.mediaType === 'video' ||
        (s.type === 'outbound-rtp' && s.framesEncoded !== undefined) ||
        (s.type === 'inbound-rtp' && s.framesDecoded !== undefined);
      if (s.type === 'outbound-rtp' && isVideo) {
        outboundEntries.push({ ssrc: s.ssrc, mid: s.mid, framesEncoded: s.framesEncoded, bytesSent: s.bytesSent });
        framesEncoded += Number(s.framesEncoded) || 0;
        bytesSent += Number(s.bytesSent) || 0;
        frameWidth = Number(s.frameWidth) || frameWidth;
        frameHeight = Number(s.frameHeight) || frameHeight;
      }
      if (s.type === 'inbound-rtp' && isVideo) {
        inboundEntries.push({ ssrc: s.ssrc, mid: s.mid, framesDecoded: s.framesDecoded, bytesReceived: s.bytesReceived });
        framesDecoded += Number(s.framesDecoded) || 0;
        bytesReceived += Number(s.bytesReceived) || 0;
        frameWidth = Number(s.frameWidth) || frameWidth;
        frameHeight = Number(s.frameHeight) || frameHeight;
      }
    });
    return { ok: true, framesEncoded, framesDecoded, bytesSent, bytesReceived, frameWidth, frameHeight, outboundEntries, inboundEntries };
  }, identity);
}

// 精确失败注入（唯一允许的失败注入：用于测产品自己的降级/取消分支）
export async function installGdm4kReject(page: Page) {
  await page.addInitScript(() => {
    const orig = navigator.mediaDevices.getDisplayMedia.bind(navigator.mediaDevices);
    (navigator.mediaDevices as any).getDisplayMedia = async (c: any) => {
      const w = c?.video?.width?.ideal ?? c?.video?.width;
      if (typeof w === 'number' && w > 1920) {
        throw new DOMException('4k unsupported by fake source', 'OverconstrainedError');
      }
      return orig(c);
    };
  });
}

export async function installGdmNotAllowed(page: Page) {
  await page.addInitScript(() => {
    (navigator.mediaDevices as any).getDisplayMedia = async () => {
      throw new DOMException('user cancelled', 'NotAllowedError');
    };
  });
}

// 观测：保存最近一次 getDisplayMedia 返回的 stream，用于模拟"浏览器原生停止共享"
export async function installStreamObserver(page: Page) {
  await page.addInitScript(() => {
    const orig = navigator.mediaDevices.getDisplayMedia.bind(navigator.mediaDevices);
    (navigator.mediaDevices as any).getDisplayMedia = async (c: any) => {
      const s = await orig(c);
      (window as any).__kaede_last_display_stream__ = s;
      return s;
    };
  });
}

export async function stopDisplayStreamViaBrowser(page: Page) {
  await page.evaluate(() => {
    const s = (window as any).__kaede_last_display_stream__;
    if (!s) throw new Error('no display stream captured');
    s.getVideoTracks().forEach((t: MediaStreamTrack) => t.stop());
  });
}

/**
 * 用可控的合成帧源替换 getDisplayMedia。
 *
 * 为什么需要它：headless Chromium 里的真实/伪造桌面抓取是空画面，Chrome 对
 * 静态画面会跳过编码，framesEncoded 会是 0~2，根本测不出“帧率”和“断续”。
 * 这里用 canvas.captureStream 提供**每帧都在变化**的真实视频源：
 *   - motion='high'：模拟游戏/视频这类高动态画面，编码器持续满负荷；
 *   - motion='static'：模拟读文档，画面基本不动（用于验证自适应**不误触发**）。
 *
 * 必须在点击“开始共享”之前调用（startScreenShare 会在点击时读取该方法）。
 */
export async function installSyntheticDisplay(
  page: Page,
  opts: { width?: number; height?: number; fps?: number; motion?: 'high' | 'static' } = {},
) {
  await page.evaluate((o) => {
    const width = o.width ?? 1920;
    const height = o.height ?? 1080;
    const targetFps = o.fps ?? 60;
    const motion = o.motion ?? 'high';
    (window as any).__kaede_synthetic_frames__ = 0;
    (navigator.mediaDevices as any).getDisplayMedia = async () => {
      const canvas = document.createElement('canvas');
      canvas.width = width;
      canvas.height = height;
      const ctx = canvas.getContext('2d')!;
      let frame = 0;
      const draw = (now: number) => {
        frame++;
        const t = now / 1000;
        if (motion === 'high') {
          // 大面积渐变色 + 随机色块位移：保证每帧差异，编码器无法跳帧
          const g = ctx.createLinearGradient(0, 0, width, height);
          g.addColorStop(0, `hsl(${Math.floor((t * 120) % 360)},90%,45%)`);
          g.addColorStop(1, `hsl(${Math.floor((t * 120 + 120) % 360)},90%,35%)`);
          ctx.fillStyle = g;
          ctx.fillRect(0, 0, width, height);
          for (let i = 0; i < 60; i++) {
            const x = (i * 97 + frame * 37) % width;
            const y = (i * 131 + frame * 53) % height;
            ctx.fillStyle = `hsl(${(i * 9 + frame * 7) % 360},100%,60%)`;
            ctx.fillRect(x, y, Math.max(48, width / 20), Math.max(48, width / 20));
          }
        } else {
          // 静态：只画一次，后续仅极小的时钟变化（模拟文档页）
          if (frame === 1) {
            ctx.fillStyle = '#f5f5f5';
            ctx.fillRect(0, 0, width, height);
            ctx.fillStyle = '#222';
            ctx.font = `${Math.round(height / 20)}px monospace`;
            ctx.fillText('static document', 80, height / 2);
          }
          ctx.fillStyle = '#f5f5f5';
          ctx.fillRect(0, 0, 60, 60);
          ctx.fillStyle = '#333';
          ctx.fillRect(frame % 60, 10, 4, 4);
        }
        (window as any).__kaede_synthetic_frames__ = frame;
      };
      if (motion === 'static') {
        // 静态文档：约 2fps 低频重绘（模拟翻页/光标），让 framesEncoded 天然很低。
        // 之前用 rAF 每帧重绘，静态源也变成 60fps“高动态源”，导致降档用例误判。
        draw(0);
        setInterval(() => draw(performance.now()), 500);
      } else {
        const loop = (now: number) => {
          draw(now);
          requestAnimationFrame(loop);
        };
        requestAnimationFrame(loop);
      }
      const stream = (canvas as any).captureStream(targetFps) as MediaStream;
      (window as any).__kaede_synthetic_stream__ = stream;
      return stream;
    };
  }, opts);
}

export interface QualityBucket {
  /** 该秒内实际帧数（发送端 framesEncoded / 接收端 framesDecoded 的增量） */
  fps: number;
  lostDelta: number;
  recvDelta: number;
  freezeDelta: number;
  frozenMsDelta: number;
  limit?: string;
}

/**
 * 在页面内连续采样 N 秒，按 1s 桶返回真实帧率 / 丢包 / 冻结。
 *
 * 放在页面内循环，避免 Playwright 往返延迟把“秒”拉长导致 fps 计算失真。
 */
export async function sampleStreamQuality(
  page: Page,
  identity: string | null,
  seconds: number,
): Promise<QualityBucket[]> {
  return await page.evaluate(async ({ id, secs }) => {
    // 用页面挂载的实例，避免 HMR 下 import 拿到另一个模块实例导致 currentRoom 为 null
    const room = (window as any).__kaedeRoom;
    if (!room) return [];
    const getPub = () => {
      if (id) {
        const p = room.remoteParticipants.get(id);
        return Array.from(p?.videoTrackPublications?.values?.() || []).find((x: any) => x.source === 'screen_share');
      }
      return Array.from(room.localParticipant.videoTrackPublications.values()).find((x: any) => x.source === 'screen_share');
    };
    const out: any[] = [];
    let prev: any = null;
    const start = performance.now();
    while (performance.now() - start < secs * 1000) {
      await new Promise((r) => setTimeout(r, 1000));
      const pub: any = getPub();
      if (!pub?.track) continue;
      const report: RTCStatsReport = await pub.track.getRTCStatsReport();
      let s: any;
      report.forEach((v: any) => {
        const type = id ? 'inbound-rtp' : 'outbound-rtp';
        if (v.type === type && (v.kind === 'video' || v.mediaType === 'video')) s = v;
      });
      if (!s) continue;
      const cur = {
        t: performance.now(),
        frames: (id ? s.framesDecoded : s.framesEncoded) ?? 0,
        lost: s.packetsLost ?? 0,
        recv: s.packetsReceived ?? 0,
        freezes: s.freezeCount ?? 0,
        frozen: s.totalFreezesDuration ?? 0,
        limit: s.qualityLimitationReason,
      };
      if (prev) {
        const dt = (cur.t - prev.t) / 1000;
        out.push({
          fps: dt > 0 ? (cur.frames - prev.frames) / dt : 0,
          lostDelta: cur.lost - prev.lost,
          recvDelta: cur.recv - prev.recv,
          freezeDelta: Math.max(0, cur.freezes - prev.freezes),
          frozenMsDelta: Math.max(0, (cur.frozen - prev.frozen) * 1000),
          limit: cur.limit,
        });
      }
      prev = cur;
    }
    return out;
  }, { id: identity, secs: seconds });
}

export interface FramePacing {
  available: boolean;
  /** 采样窗口内解码帧数 */
  frames: number;
  /** 平均帧间隔 ms（1080p30 理想值 ≈33.3） */
  meanMs: number;
  /** 帧间隔标准差 ms —— 判定“帧稳不稳”的核心指标 */
  stdMs: number;
  /** 窗口内丢帧数 */
  dropped: number;
}

/**
 * 采样接收端的**帧间隔稳定性**。
 *
 * 平均帧率达标 ≠ 看着流畅：帧间隔忽长忽短就是肉眼看到的“一卡一卡”。
 * 这里用 inbound-rtp 的 totalInterFrameDelay / totalSquaredInterFrameDelay
 * 计算窗口内帧间隔的均值与标准差。这两个字段缺失时 available=false。
 */
export async function sampleFramePacing(page: Page, identity: string, seconds: number): Promise<FramePacing> {
  return await page.evaluate(async ({ id, secs }) => {
    const room = (window as any).__kaedeRoom;
    const empty = { available: false, frames: 0, meanMs: 0, stdMs: 0, dropped: 0 };
    if (!room) return empty;
    const snap = async () => {
      const p = room.remoteParticipants.get(id);
      const pub = Array.from(p?.videoTrackPublications?.values?.() || [])
        .find((x: any) => x.source === 'screen_share');
      if (!pub?.track) return null;
      const report: RTCStatsReport = await pub.track.getRTCStatsReport();
      let v: any;
      report.forEach((x: any) => {
        if (x.type === 'inbound-rtp' && (x.kind === 'video' || x.mediaType === 'video')) v = x;
      });
      if (!v) return null;
      return {
        frames: v.framesDecoded ?? 0,
        delay: v.totalInterFrameDelay,
        sqDelay: v.totalSquaredInterFrameDelay,
        dropped: v.framesDropped ?? 0,
      };
    };
    // 等首帧/订阅就绪（最多 15s）
    let a: any = null;
    const t0 = performance.now();
    while (performance.now() - t0 < 15_000) {
      a = await snap();
      if (a && a.frames > 0) break;
      await new Promise((r) => setTimeout(r, 500));
    }
    if (!a) return empty;
    await new Promise((r) => setTimeout(r, secs * 1000));
    const b = await snap();
    if (!b) return empty;
    const n = b.frames - a.frames;
    const usable = typeof a.delay === 'number' && typeof a.sqDelay === 'number' && n > 0;
    const mean = usable ? (b.delay - a.delay) / n : 0;
    const variance = usable ? Math.max(0, (b.sqDelay - a.sqDelay) / n - mean * mean) : 0;
    return {
      available: usable,
      frames: n,
      meanMs: mean * 1000,
      stdMs: Math.sqrt(variance) * 1000,
      dropped: Math.max(0, b.dropped - a.dropped),
    };
  }, { id: identity, secs: seconds });
}

export function summarize(buckets: QualityBucket[]) {
  const n = buckets.length;
  const avgFps = n ? buckets.reduce((a, b) => a + b.fps, 0) / n : 0;
  const recv = buckets.reduce((a, b) => a + b.recvDelta, 0);
  const lost = buckets.reduce((a, b) => a + b.lostDelta, 0);
  const lossPct = recv + lost > 0 ? (lost / (recv + lost)) * 100 : 0;
  const freezes = buckets.reduce((a, b) => a + b.freezeDelta, 0);
  const frozenMs = buckets.reduce((a, b) => a + b.frozenMsDelta, 0);
  // “断续”：某秒帧数低于不丢帧时的 1/3
  const starved = buckets.filter((b) => b.fps < avgFps / 3 && avgFps > 0).length;
  const limits = Array.from(new Set(buckets.map((b) => b.limit).filter(Boolean)));
  return { n, avgFps, lossPct, freezes, frozenMs, starved, limits };
}

type Fixtures = {
  u0: Page; u1: Page;
  u0Name: string; u1Name: string;
  u0Ctx: BrowserContext; u1Ctx: BrowserContext;
};

export const test = base.extend<Fixtures>({
  u0Name: async ({}, use) => use(randName('u0')),
  u1Name: async ({}, use) => use(randName('u1')),
  u0Ctx: async ({ browser }, use) => {
    const ctx = await browser.newContext({ ignoreHTTPSErrors: true, permissions: ['camera', 'microphone'] });
    await use(ctx);
    await ctx.close();
  },
  u1Ctx: async ({ browser }, use) => {
    const ctx = await browser.newContext({ ignoreHTTPSErrors: true, permissions: ['camera', 'microphone'] });
    await use(ctx);
    await ctx.close();
  },
  // teardown 里显式 Leave：避免 abrupt close 触发 LiveKit departure 宽限，
  // 幽灵 participant/共享面板残留污染下一个用例
  u0: async ({ u0Ctx, u0Name }, use) => { const p = await u0Ctx.newPage(); await login(p, u0Name); await use(p); try { await p.evaluate(async () => { await (window as any).__kaedeRoom?.disconnect(); }); } catch { /* 页面已关闭则忽略 */ } },
  u1: async ({ u1Ctx, u1Name }, use) => { const p = await u1Ctx.newPage(); await login(p, u1Name); await use(p); try { await p.evaluate(async () => { await (window as any).__kaedeRoom?.disconnect(); }); } catch { /* 页面已关闭则忽略 */ } },
});

export { expect };
