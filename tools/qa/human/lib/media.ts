import { Page, expect } from '@playwright/test';

// ---------------------------------------------------------------------------
// 语音房：真人点侧栏频道 → 点“加入语音”
// ---------------------------------------------------------------------------

export async function getFirstVoiceChannel(page: Page): Promise<{ id: string; name: string }> {
  return await page.evaluate(async () => {
    const dbg = (window as any).__kaede_debug__;
    const list: any[] = await dbg.channels.list();
    const v = list.find((c: any) => c.kind === 'voice');
    if (!v) throw new Error('no voice channel: ' + JSON.stringify(list));
    return { id: v.id, name: v.name };
  });
}

export async function connectedChannel(page: Page): Promise<string | null> {
  const el = page.getByTestId('media-connected');
  if (!(await el.isVisible().catch(() => false))) return null;
  return await el.getAttribute('data-channel');
}

/**
 * 进入语音频道。若已在该频道则直接返回（真人不会重复点“加入”）。
 */
export async function enterVoiceChannel(page: Page, chId: string, voiceName: string) {
  if ((await connectedChannel(page)) === chId) return;

  const byNodeId = page.locator(`[data-node-id="${chId}"]`);
  if ((await byNodeId.count()) > 0) {
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
  await expect(page.getByTestId('media-connected')).toHaveAttribute('data-channel', chId, { timeout: 30_000 });
}

/** 真人点“离开语音”按钮，等待 UI 回到未连接状态。 */
export async function leaveVoiceChannelViaUI(page: Page, chId?: string) {
  if (!(await page.getByTestId('media-connected').isVisible().catch(() => false))) return;
  if (chId) {
    const leave = page.getByTestId('media-leave');
    if (!(await leave.isVisible().catch(() => false))) {
      await page.locator(`[data-node-id="${chId}"]`).first().click().catch(() => undefined);
    }
  }
  const leave = page.getByTestId('media-leave');
  if (await leave.isVisible().catch(() => false)) {
    await leave.click();
  }
  await page.getByTestId('media-join').waitFor({ state: 'visible', timeout: 30_000 }).catch(() => undefined);
  await expect(page.getByTestId('media-connected')).toHaveCount(0, { timeout: 30_000 });
}

export async function setMic(page: Page, on: boolean) {
  const btn = page.getByTestId('media-mic-toggle');
  const cur = await btn.getAttribute('data-mic');
  if (cur === (on ? 'on' : 'off')) return;
  await btn.click();
  await expect(btn).toHaveAttribute('data-mic', on ? 'on' : 'off', { timeout: 15_000 });
}

export async function setDeafen(page: Page, on: boolean) {
  const btn = page.getByTestId('media-deafen-toggle');
  const cur = await btn.getAttribute('data-deafened');
  if (cur === (on ? 'on' : 'off')) return;
  await btn.click();
  await expect(btn).toHaveAttribute('data-deafened', on ? 'on' : 'off', { timeout: 10_000 });
}

export async function setNoiseSuppression(page: Page, on: boolean) {
  const btn = page.getByTestId('media-noise-toggle');
  const cur = await btn.getAttribute('data-noise');
  if (cur === (on ? 'on' : 'off')) return;
  await btn.click();
  await expect(btn).toHaveAttribute('data-noise', on ? 'on' : 'off', { timeout: 15_000 });
}

// ---------------------------------------------------------------------------
// 屏幕共享：真实按钮 + 合成帧源（保证每帧都在变化，可测帧率/断续）
// ---------------------------------------------------------------------------

export async function installSyntheticDisplay(
  page: Page,
  opts: { width?: number; height?: number; fps?: number; motion?: 'high' | 'static'; maxWidth?: number } = {},
) {
  await page.evaluate(o => {
    const width = o.width ?? 1920;
    const height = o.height ?? 1080;
    const targetFps = o.fps ?? 60;
    const motion = o.motion ?? 'high';
    (window as any).__kaede_synthetic_frames__ = 0;
    (navigator.mediaDevices as any).getDisplayMedia = async (constraints: any) => {
      const asked = Number(constraints?.video?.width?.ideal ?? constraints?.video?.width ?? 0);
      if (o.maxWidth && asked > o.maxWidth) {
        throw new DOMException('requested width exceeds synthetic source', 'OverconstrainedError');
      }
      const canvas = document.createElement('canvas');
      canvas.width = width;
      canvas.height = height;
      const ctx = canvas.getContext('2d')!;
      let frame = 0;
      const draw = (now: number) => {
        frame++;
        const t = now / 1000;
        if (motion === 'high') {
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

/** 浏览器拒绝 4K 请求（只对 width>1920 抛 OverconstrainedError）。须在导航前调用。 */
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

/** 用户点了“取消共享”。须在导航前调用。 */
export async function installGdmNotAllowed(page: Page) {
  await page.addInitScript(() => {
    (navigator.mediaDevices as any).getDisplayMedia = async () => {
      throw new DOMException('user cancelled', 'NotAllowedError');
    };
  });
}

/**
 * 模拟用户点浏览器原生的“停止共享”：
 * 真实 Chrome 会先给底层 display track 派发 ended 事件（track.stop() 本身不派发），
 * 这里显式补上，才能覆盖产品的原生停止收尾逻辑。
 */
export async function stopDisplayStreamViaBrowser(page: Page) {
  await page.evaluate(() => {
    const s = (window as any).__kaede_synthetic_stream__;
    if (!s) throw new Error('no synthetic display stream captured');
    (s as MediaStream).getVideoTracks().forEach((t: MediaStreamTrack) => {
      t.dispatchEvent(new Event('ended'));
      t.stop();
    });
  });
}

export async function selectSharePreset(page: Page, presetId: string) {
  await page.getByTestId('media-quality').selectOption(presetId);
}

/** 真人点共享按钮，等待 UI 与实际发布都就绪。 */
export async function startShareViaUI(page: Page, presetId?: string) {
  if (presetId) await selectSharePreset(page, presetId);
  const toggle = page.getByTestId('media-share-toggle');
  await toggle.click();
  await expect(toggle).toHaveAttribute('data-sharing', 'on', { timeout: 35_000 });
  await expect(page.getByTestId('screenshare-panel')).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId('screen-share-video').first()).toBeVisible({ timeout: 20_000 });
}

export async function stopShareViaUI(page: Page) {
  const toggle = page.getByTestId('media-share-toggle');
  if ((await toggle.getAttribute('data-sharing')) === 'off') return;
  await toggle.click();
  await expect(toggle).toHaveAttribute('data-sharing', 'off', { timeout: 25_000 });
  // 可能还有别人在共享，所以只断言“本端不再发布”，不要求整个面板消失
  await expect
    .poll(async () => (await localShareInfo(page)).has, { timeout: 25_000, intervals: [500, 1000] })
    .toBe(false);
}

export function sharePanel(page: Page) {
  return page.getByTestId('screenshare-panel');
}

export async function sharePanelCount(page: Page): Promise<number> {
  return await page.getByTestId('screenshare-panel').count();
}

/** 等待接收端真正出现共享并开始出帧。 */
export async function waitRemoteShareWithFrames(page: Page, senderIdentity: string, timeoutMs = 30_000) {
  await expect(page.getByTestId('screenshare-panel')).toBeVisible({ timeout: timeoutMs });
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    const stats = await videoFrameStats(page, senderIdentity);
    if (stats.ok && (stats.framesDecoded ?? 0) > 0) return;
    await page.waitForTimeout(700);
  }
  const stats = await videoFrameStats(page, senderIdentity);
  throw new Error(`remote share has no decoded frames within ${timeoutMs}ms: ${JSON.stringify(stats)}`);
}

// ---------------------------------------------------------------------------
// 观测：只读页面内 LiveKit / 媒体健康对象
// ---------------------------------------------------------------------------

export async function myIdentity(page: Page): Promise<string> {
  return await page.evaluate(() => (window as any).__kaedeRoom?.localParticipant?.identity ?? '');
}

export async function dumpRoomState(page: Page) {
  return await page.evaluate(() => {
    const r = (window as any).__kaedeRoom;
    if (!r) return { noRoom: true } as any;
    const remote: any[] = [];
    for (const [id, p] of r.remoteParticipants) {
      remote.push({
        identity: id,
        videoPubs: Array.from((p as any).videoTrackPublications?.values?.() || []).map((pub: any) => ({
          sid: pub.trackSid,
          source: pub.source,
          subscribed: pub.isSubscribed,
          muted: pub.isMuted,
        })),
        audioPubs: Array.from((p as any).audioTrackPublications?.values?.() || []).map((pub: any) => ({
          sid: pub.trackSid,
          source: pub.source,
          subscribed: pub.isSubscribed,
          muted: pub.isMuted,
        })),
      });
    }
    return {
      name: r.name,
      state: String(r.state),
      identity: r.localParticipant?.identity,
      localVideoPubs: Array.from((r.localParticipant as any).videoTrackPublications?.values?.() || []).map(
        (pub: any) => ({ sid: pub.trackSid, source: pub.source, muted: pub.isMuted }),
      ),
      remoteCount: r.remoteParticipants.size,
      remote,
      e2eeEnabled: !!(r as any).e2eeManager?.isE2EEEnabled,
    };
  });
}

export async function localShareInfo(page: Page) {
  return await page.evaluate(() => {
    const room = (window as any).__kaedeRoom;
    if (!room) return { has: false, reason: 'no-room' } as any;
    const pubs: any[] = Array.from(room.localParticipant.videoTrackPublications.values());
    const share = pubs.find((p: any) => p.source === 'screen_share');
    if (!share) return { has: false, reason: 'no-screen-share-pub', sources: pubs.map((p: any) => p.source) } as any;
    const mt = share.track?.mediaStreamTrack;
    if (!mt || mt.readyState === 'ended') {
      return { has: false, reason: 'track-ended', sid: share.trackSid, trackState: mt?.readyState ?? null } as any;
    }
    return {
      has: true,
      sid: share.trackSid,
      isMuted: share.isMuted,
      settings: mt.getSettings(),
      isScreenShareEnabled: !!room.localParticipant.isScreenShareEnabled,
    } as any;
  });
}

export async function remoteShareInfo(page: Page, fromIdentity: string) {
  return await page.evaluate((id: string) => {
    const room = (window as any).__kaedeRoom;
    if (!room) return { has: false, reason: 'no-room' } as any;
    const p = room.remoteParticipants.get(id);
    if (!p) return { has: false, reason: 'participant-not-found', known: Array.from(room.remoteParticipants.keys()) } as any;
    const pubs: any[] = Array.from(p.videoTrackPublications.values());
    const share = pubs.find((pub: any) => pub.source === 'screen_share');
    if (!share) return { has: false, reason: 'no-share-pub', sources: pubs.map((pub: any) => pub.source) } as any;
    return { has: true, sid: share.trackSid, isSubscribed: share.isSubscribed, isMuted: share.isMuted } as any;
  }, fromIdentity);
}

export async function videoFrameStats(page: Page, identity: string | null) {
  return await page.evaluate(async (id: string | null) => {
    const room = (window as any).__kaedeRoom;
    if (!room) return { ok: false, reason: 'no-room' } as any;
    const pub = id
      ? Array.from(room.remoteParticipants.get(id)?.videoTrackPublications?.values?.() || []).find(
          (p: any) => p.source === 'screen_share',
        )
      : Array.from(room.localParticipant.videoTrackPublications.values()).find(
          (p: any) => p.source === 'screen_share',
        );
    if (!pub || !pub.track) return { ok: false, reason: 'no-track' } as any;
    const report = await (pub.track as any).getRTCStatsReport();
    let framesEncoded = 0;
    let framesDecoded = 0;
    let bytesSent = 0;
    let bytesReceived = 0;
    let frameWidth = 0;
    let frameHeight = 0;
    report.forEach((s: any) => {
      const isVideo =
        s.kind === 'video' ||
        s.mediaType === 'video' ||
        (s.type === 'outbound-rtp' && s.framesEncoded !== undefined) ||
        (s.type === 'inbound-rtp' && s.framesDecoded !== undefined);
      if (s.type === 'outbound-rtp' && isVideo) {
        framesEncoded += Number(s.framesEncoded) || 0;
        bytesSent += Number(s.bytesSent) || 0;
        frameWidth = Number(s.frameWidth) || frameWidth;
        frameHeight = Number(s.frameHeight) || frameHeight;
      }
      if (s.type === 'inbound-rtp' && isVideo) {
        framesDecoded += Number(s.framesDecoded) || 0;
        bytesReceived += Number(s.bytesReceived) || 0;
        frameWidth = Number(s.frameWidth) || frameWidth;
        frameHeight = Number(s.frameHeight) || frameHeight;
      }
    });
    return { ok: true, framesEncoded, framesDecoded, bytesSent, bytesReceived, frameWidth, frameHeight } as any;
  }, identity);
}

export interface QualityBucket {
  fps: number;
  lostDelta: number;
  recvDelta: number;
  freezeDelta: number;
  frozenMsDelta: number;
  limit?: string;
}

/** 页面内连续采样 N 秒，按 1s 桶返回真实帧率 / 丢包 / 冻结。 */
export async function sampleStreamQuality(
  page: Page,
  identity: string | null,
  seconds: number,
): Promise<QualityBucket[]> {
  return await page.evaluate(
    async ({ id, secs }: { id: string | null; secs: number }) => {
      const room = (window as any).__kaedeRoom;
      if (!room) return [];
      const getPub = () => {
        if (id) {
          const p = room.remoteParticipants.get(id);
          return Array.from(p?.videoTrackPublications?.values?.() || []).find(
            (x: any) => x.source === 'screen_share',
          );
        }
        return Array.from(room.localParticipant.videoTrackPublications.values()).find(
          (x: any) => x.source === 'screen_share',
        );
      };
      const out: any[] = [];
      let prev: any = null;
      const start = performance.now();
      while (performance.now() - start < secs * 1000) {
        await new Promise(r => setTimeout(r, 1000));
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
    },
    { id: identity, secs: seconds },
  );
}

export interface FramePacing {
  available: boolean;
  frames: number;
  meanMs: number;
  stdMs: number;
  dropped: number;
}

/** 采样接收端帧间隔稳定性（平均帧率达标 ≠ 看着流畅）。 */
export async function sampleFramePacing(page: Page, identity: string, seconds: number): Promise<FramePacing> {
  return await page.evaluate(
    async ({ id, secs }: { id: string; secs: number }) => {
      const room = (window as any).__kaedeRoom;
      const empty = { available: false, frames: 0, meanMs: 0, stdMs: 0, dropped: 0 };
      if (!room) return empty;
      const snap = async () => {
        const p = room.remoteParticipants.get(id);
        const pub = Array.from(p?.videoTrackPublications?.values?.() || []).find(
          (x: any) => x.source === 'screen_share',
        );
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
      let a: any = null;
      const t0 = performance.now();
      while (performance.now() - t0 < 15_000) {
        a = await snap();
        if (a && a.frames > 0) break;
        await new Promise(r => setTimeout(r, 500));
      }
      if (!a) return empty;
      await new Promise(r => setTimeout(r, secs * 1000));
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
    },
    { id: identity, secs: seconds },
  );
}

export function summarize(buckets: QualityBucket[]) {
  const n = buckets.length;
  const avgFps = n ? buckets.reduce((a, b) => a + b.fps, 0) / n : 0;
  const recv = buckets.reduce((a, b) => a + b.recvDelta, 0);
  const lost = buckets.reduce((a, b) => a + b.lostDelta, 0);
  const lossPct = recv + lost > 0 ? (lost / (recv + lost)) * 100 : 0;
  const freezes = buckets.reduce((a, b) => a + b.freezeDelta, 0);
  const frozenMs = buckets.reduce((a, b) => a + b.frozenMsDelta, 0);
  const starved = buckets.filter(b => b.fps < avgFps / 3 && avgFps > 0).length;
  const limits = Array.from(new Set(buckets.map(b => b.limit).filter(Boolean)));
  return { n, avgFps, lossPct, freezes, frozenMs, starved, limits };
}

export async function getDiag(page: Page) {
  return await page.evaluate(() => (window as any).__kaedeDiag?.() ?? null);
}

export async function getHealth(page: Page) {
  return await page.evaluate(() => (window as any).__kaedeMediaHealth?.() ?? null);
}

export async function getMediaState(page: Page) {
  return await page.evaluate(() => (window as any).__kaedeMediaState?.() ?? null);
}
