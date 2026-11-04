import { test, expect } from '@playwright/test';
import {
  spawnHumans,
  closeHumans,
  createNodeViaUI,
  waitForNode,
  clickChannel,
  waitChannelGroup,
  waitMainGroupReady,
  nodeId,
  sendMessageHuman,
  messageRowByText,
  humanPause,
  hardErrors,
} from '../lib/core';
import {
  enterVoiceChannel,
  installSyntheticDisplay,
  startShareViaUI,
  stopShareViaUI,
  waitRemoteShareWithFrames,
  videoFrameStats,
  setMic,
  dumpRoomState,
  myIdentity,
} from '../lib/media';

test.describe('真人混合长链路：建频道 → 聊天 → 语音 → 共享 → 刷新后继续', () => {
  test('三个人的真实一晚：消息、表情、麦克风、共享、刷新恢复都在同一条链路上', async ({ browser }) => {
    test.setTimeout(360_000);
    const humans = await spawnHumans(browser, 3, 'mix');
    try {
      const [a, b, c] = humans;
      const tag = Date.now().toString(36);
      const folder = `party-${tag}`;
      const text = `chat-${tag}`;
      const voice = `voice-${tag}`;

      // A 建分类 + 文字 + 语音
      await createNodeViaUI(a.page, { name: folder, kind: 'folder' });
      await waitForNode(a.page, folder);
      await waitForNode(b.page, folder, { timeout: 35_000 });
      await waitForNode(c.page, folder, { timeout: 35_000 });

      await createNodeViaUI(a.page, { name: text, kind: 'text', parent: folder });
      await createNodeViaUI(a.page, { name: voice, kind: 'voice', parent: folder });
      for (const h of [b, c]) {
        await waitForNode(h.page, text, { timeout: 35_000 });
        await waitForNode(h.page, voice, { timeout: 35_000 });
      }

      // 聊天 + 表情。
      // 注意：第三个成员点进频道后，authority 需要异步把新成员加进该频道的 MLS group。
      // 不等 group 就绪就发/发反应，会让新成员处于“能收不能发”的半接入态（反应发送会失败）。
      const textId = await nodeId(a.page, text);
      for (const h of humans) {
        await clickChannel(h.page, text);
        await waitChannelGroup(h.page, textId);
      }
      // 双向就绪屏障：B、C 各发一条，确认三个人都能发也能收。
      const probeB = `journey-probe-b-${tag}`;
      const probeC = `journey-probe-c-${tag}`;
      await sendMessageHuman(b.page, probeB);
      await sendMessageHuman(c.page, probeC);
      for (const h of humans) {
        await expect(messageRowByText(h.page, probeB)).toBeVisible({ timeout: 30_000 });
        await expect(messageRowByText(h.page, probeC)).toBeVisible({ timeout: 30_000 });
      }

      const hello = `journey-hello-${tag}`;
      await sendMessageHuman(a.page, hello);
      for (const h of [b, c]) {
        await expect(messageRowByText(h.page, hello)).toBeVisible({ timeout: 30_000 });
      }
      const idle = `journey-reply-${tag}`;
      await sendMessageHuman(b.page, idle);
      await expect(messageRowByText(a.page, idle)).toBeVisible({ timeout: 30_000 });

      // 公开频道的 reaction 走主群 MLS 广播，先等主群就绪（否则发送失败会被静默回滚）。
      for (const h of humans) await waitMainGroupReady(h.page, 45_000);

      await messageRowByText(c.page, hello).click({ button: 'right' });
      await expect(c.page.getByTestId('message-context-menu')).toBeVisible({ timeout: 10_000 });
      await c.page.getByTestId('quick-react-👍').click();
      // 先确认本端乐观生效（发送失败会被回滚），再看对端同步。
      await expect(c.page.getByTestId('reaction-pill-👍')).toBeVisible({ timeout: 20_000 });
      await expect(a.page.getByTestId('reaction-pill-👍')).toBeVisible({ timeout: 20_000 });

      // 语音房：A 上麦，三人同房
      const voiceId = await a.page.evaluate(n => {
        const st = (window as any).__kaede_debug__.store.getState();
        return st.nodes.find((x: any) => x.name === n)?.id as string | undefined;
      }, voice);
      if (!voiceId) throw new Error('mixed: voice channel id not found');
      for (const h of [a, b, c]) await enterVoiceChannel(h.page, voiceId, voice);
      await setMic(a.page, true);

      const idA = await myIdentity(a.page);
      await expect
        .poll(async () => {
          const st = await dumpRoomState(b.page);
          return (st.remote.find((r: any) => r.identity === idA)?.audioPubs?.length ?? 0) > 0;
        }, { timeout: 20_000, intervals: [500, 1000] })
        .toBe(true);

      // 共享：A 发，B/C 看；B 降低接收档位，画面不断
      const share = `journey-share-${tag}`;
      await clickChannel(a.page, text); // 真人回到聊天视图才发现共享按钮
      await installSyntheticDisplay(a.page, { width: 1920, height: 1080, fps: 60, motion: 'high' });
      await startShareViaUI(a.page, '1080p30');
      for (const viewer of [b, c]) await waitRemoteShareWithFrames(viewer.page, idA, 30_000);

      const sid = (await b.page.evaluate(id => {
        const p = (window as any).__kaedeRoom.remoteParticipants.get(id);
        const pub = Array.from(p.videoTrackPublications.values()).find((x: any) => x.source === 'screen_share');
        return pub?.trackSid ?? null;
      }, idA)) as string | null;
      if (sid) await b.page.getByTestId(`screen-share-quality-${sid}`).selectOption('720p30');
      await humanPause(b.page, 2000, 3000);
      const f1 = await videoFrameStats(b.page, idA);
      await b.page.waitForTimeout(2500);
      const f2 = await videoFrameStats(b.page, idA);
      expect(f2.framesDecoded ?? 0).toBeGreaterThan(f1.framesDecoded ?? 0);

      await stopShareViaUI(a.page);
      for (const viewer of [b, c]) {
        await expect(viewer.page.getByTestId('screenshare-panel')).toHaveCount(0, { timeout: 20_000 });
      }
      expect(share).toBeTruthy();

      // A 刷新页面，重新打开文字频道继续聊
      await a.page.reload();
      await a.page.getByTestId('message-input').waitFor({ state: 'visible', timeout: 45_000 });
      await a.page.waitForFunction(() => !!(window as any).__kaede_debug__, null, { timeout: 25_000 });
      await clickChannel(a.page, text);
      const after = `after-reload-${tag}`;
      await sendMessageHuman(a.page, after);
      await clickChannel(b.page, text);
      await expect(messageRowByText(b.page, after)).toBeVisible({ timeout: 30_000 });

      // A 用搜索找回刚才的消息
      await a.page.getByTestId('search-trigger').click();
      await expect(a.page.getByTestId('search-input')).toBeVisible({ timeout: 10_000 });
      await a.page.getByTestId('search-input').fill(after);
      await expect(a.page.getByTestId('search-result-0')).toBeVisible({ timeout: 15_000 });

      // 刷新后的 host 仍是 host：这时才登录的第 4 人必须也能被拉进主群。
      // （旧实现 isMlsHost 用“成员数 == 1”推断，第 3 人起就再没人拉人；
      //   刷新后 is_host 标记还要能从本地存储恢复，否则同样拉不到新成员。）
      const d = (await spawnHumans(browser, 1, 'mixd'))[0];
      try {
        await waitMainGroupReady(d.page, 60_000);
      } finally {
        await closeHumans([d]);
      }

      for (const h of humans) expect(hardErrors(h)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });
});
