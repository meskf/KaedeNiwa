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
    const m: any = await import('/src/lib/livekit.ts');
    const r = m.getCurrentRoom();
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
    const m: any = await import('/src/lib/livekit.ts');
    const room = m.getCurrentRoom();
    return room?.localParticipant?.identity ?? '';
  });
}

export async function localShareInfo(page: Page) {
  return await page.evaluate(async () => {
    const m: any = await import('/src/lib/livekit.ts');
    const room = m.getCurrentRoom();
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
    const m: any = await import('/src/lib/livekit.ts');
    const room = m.getCurrentRoom();
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
    const m: any = await import('/src/lib/livekit.ts');
    const room = m.getCurrentRoom();
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
  u0: async ({ u0Ctx, u0Name }, use) => { const p = await u0Ctx.newPage(); await login(p, u0Name); await use(p); },
  u1: async ({ u1Ctx, u1Name }, use) => { const p = await u1Ctx.newPage(); await login(p, u1Name); await use(p); },
});

export { expect };
