import { test, expect } from '@playwright/test';
import {
  spawnHumans,
  closeHumans,
  createNodeViaUI,
  waitForNode,
  nodeId,
  hardErrors,
} from '../lib/core';
import {
  getFirstVoiceChannel,
  enterVoiceChannel,
  leaveVoiceChannelViaUI,
  setMic,
  setDeafen,
  setNoiseSuppression,
  dumpRoomState,
  myIdentity,
} from '../lib/media';

test.describe('真人语音房：多人进出 / 麦克风 / 降噪 / 拒听 / 频道隔离', () => {
  test('三人同频道：各自能看到其他两人；一人离开后其余仍在', async ({ browser }) => {
    const humans = await spawnHumans(browser, 3, 'voice');
    try {
      const [h0, h1, h2] = humans;
      const vc = await getFirstVoiceChannel(h0.page);

      for (const h of humans) await enterVoiceChannel(h.page, vc.id, vc.name);

      const ids = await Promise.all(humans.map(h => myIdentity(h.page)));
      // 第三个客户端 join 完成时，远端 participant 缓存可能还没同步完，轮询等待
      for (let i = 0; i < humans.length; i++) {
        const expected = ids.filter((_, j) => j !== i);
        await expect
          .poll(async () => {
            const st = await dumpRoomState(humans[i].page);
            const seen = st.remote.map((r: any) => r.identity);
            return st.state === 'connected' && expected.every(x => seen.includes(x));
          }, { timeout: 25_000, intervals: [500, 1000, 2000] })
          .toBe(true);
      }
      await expect(h0.page.getByTestId('media-e2ee')).toBeVisible();

      // 真人点“离开语音”
      await leaveVoiceChannelViaUI(h1.page, vc.id);
      await expect(h1.page.getByTestId('media-connected')).toHaveCount(0, { timeout: 20_000 });

      await expect
        .poll(async () => {
          const st = await dumpRoomState(h0.page);
          return st.remote.some((r: any) => r.identity === ids[1]);
        }, { timeout: 20_000, intervals: [1000, 2000] })
        .toBe(false);

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
      expect(hardErrors(h2)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });

  test('麦克风开/关、降噪切换、拒听、离开再进入：状态与对端可见性一致', async ({ browser }) => {
    const humans = await spawnHumans(browser, 2, 'mic');
    try {
      const [h0, h1] = humans;
      const vc = await getFirstVoiceChannel(h0.page);
      await enterVoiceChannel(h0.page, vc.id, vc.name);
      await enterVoiceChannel(h1.page, vc.id, vc.name);
      const id0 = await myIdentity(h0.page);

      // 开麦
      await setMic(h0.page, true);
      await expect(h0.page.getByTestId('media-mic-toggle')).toHaveAttribute('data-mic', 'on');
      await expect
        .poll(async () => {
          const st = await dumpRoomState(h1.page);
          const remote = st.remote.find((r: any) => r.identity === id0);
          return remote?.audioPubs?.length ?? 0;
        }, { timeout: 20_000, intervals: [500, 1000] })
        .toBeGreaterThan(0);

      // 关麦：对端应看到已静音或轨道消失
      await setMic(h0.page, false);
      await expect
        .poll(async () => {
          const st = await dumpRoomState(h1.page);
          const remote = st.remote.find((r: any) => r.identity === id0);
          const pubs = remote?.audioPubs ?? [];
          return pubs.length === 0 || pubs.every((p: any) => p.muted === true);
        }, { timeout: 20_000, intervals: [500, 1000] })
        .toBe(true);

      // 再开麦并切降噪（会重建麦克风轨道，但仍应保持开启）
      await setMic(h0.page, true);
      await setNoiseSuppression(h0.page, false);
      await expect(h0.page.getByTestId('media-noise-toggle')).toHaveAttribute('data-noise', 'off');
      await expect(h0.page.getByTestId('media-mic-toggle')).toHaveAttribute('data-mic', 'on');
      await setNoiseSuppression(h0.page, true);

      // 拒听开关
      await setDeafen(h1.page, true);
      await expect(h1.page.getByTestId('media-deafen-toggle')).toHaveAttribute('data-deafened', 'on');
      await setDeafen(h1.page, false);

      // 离开 → 再进入（真人来回切换）
      await leaveVoiceChannelViaUI(h0.page, vc.id);
      await expect(h0.page.getByTestId('media-connected')).toHaveCount(0, { timeout: 20_000 });
      await enterVoiceChannel(h0.page, vc.id, vc.name);
      await expect(h0.page.getByTestId('media-connected')).toHaveAttribute('data-channel', vc.id);

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });

  test('不同语音频道互不串音：B 频道看不到 A 频道的人', async ({ browser }) => {
    const humans = await spawnHumans(browser, 2, 'iso-voice');
    try {
      const [h0, h1] = humans;
      const vcA = await getFirstVoiceChannel(h0.page);
      await enterVoiceChannel(h0.page, vcA.id, vcA.name);

      // 新建第二个语音频道（真人右键侧栏 +）
      const vcName = 'voice-b-' + Date.now().toString(36);
      await createNodeViaUI(h0.page, { name: vcName, kind: 'voice' });
      await waitForNode(h0.page, vcName, { timeout: 20_000 });
      const vcBId = await nodeId(h0.page, vcName);

      await enterVoiceChannel(h1.page, vcBId, vcName);
      const id0 = await myIdentity(h0.page);

      const [s0, s1] = await Promise.all([dumpRoomState(h0.page), dumpRoomState(h1.page)]);
      expect(s0.remote.some((r: any) => r.identity === id0)).toBe(false);
      expect(s1.remoteCount).toBe(0);

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });
});
