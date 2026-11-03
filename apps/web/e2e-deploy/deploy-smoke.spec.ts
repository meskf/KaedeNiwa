import { test, expect } from '@playwright/test';
import { createHmac } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import {
  spawnUsers, closeUsers, sendMessage, setupIsolatedChannel,
  createNodeViaUI, clickChannel, waitForNode,
} from '../e2e-multi/lib/fixtures';
import { attachErrorCollector } from '../e2e-multi/lib/observe';

test.describe.configure({ mode: 'serial' });

const tag = () => Date.now().toString(36).slice(-6);
const TINY_PNG = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==',
  'base64',
);

const LIVEKIT_HTTP = process.env.KAEDE_LIVEKIT_HTTP_URL ?? 'http://127.0.0.1:7880';
const LK_KEY = process.env.LIVEKIT_API_KEY ?? 'devkey';
const LK_SECRET = process.env.LIVEKIT_API_SECRET ?? '12345678901234567890123456789012';

function b64url(v: Buffer | string) {
  return Buffer.from(v).toString('base64url');
}

/** LiveKit 管理 JWT（只用于部署验收查询房间人数，不参与产品逻辑）。 */
function livekitAdminJwt() {
  const now = Math.floor(Date.now() / 1000);
  const header = b64url(JSON.stringify({ alg: 'HS256', typ: 'JWT' }));
  const payload = b64url(JSON.stringify({
    iss: LK_KEY, sub: LK_KEY, nbf: now - 10, exp: now + 300,
    video: { roomList: true },
  }));
  const data = `${header}.${payload}`;
  const sig = createHmac('sha256', LK_SECRET).update(data).digest('base64url');
  return `${data}.${sig}`;
}

/** 直接查询 LiveKit，验证两个客户端真的在同一个房间（不依赖前端插桩）。 */
async function listRooms(): Promise<Array<{ name: string; num_participants: number; num_publishers: number }>> {
  const res = await fetch(`${LIVEKIT_HTTP}/twirp/livekit.RoomService/ListRooms`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${livekitAdminJwt()}` },
    body: '{}',
  });
  if (!res.ok) throw new Error(`ListRooms ${res.status}: ${await res.text()}`);
  const json = await res.json() as { rooms?: any[] };
  return (json.rooms ?? []).map(r => ({
    name: String(r.name ?? ''),
    num_participants: Number(r.num_participants ?? 0),
    num_publishers: Number(r.num_publishers ?? 0),
  }));
}

async function channelIdByName(page: any, name: string): Promise<string> {
  const id = await page.evaluate((n: string) => {
    const st = (window as any).__kaede_debug__.store.getState();
    return st.nodes.find((x: any) => x.name === n)?.id as string | undefined;
  }, name);
  if (!id) throw new Error(`node id not found: ${name}`);
  return id;
}

test('部署冒烟：注册/登录/建频道/文本/附件/语音 E2EE', async ({ browser }) => {
  const users = await spawnUsers(browser, 2, 'dep');
  const cols = users.map((u, i) => attachErrorCollector(u.page, `u${i}`));
  try {
    const [a, b] = users;

    // 1) 注册 + 登录 + 默认频道可见（spawnUsers 内部已断言 __kaede_debug__ 就绪）
    for (const u of users) {
      const chans = await u.page.evaluate(async () => (window as any).__kaede_debug__.channels.list());
      expect(Array.isArray(chans) && chans.length > 0, `${u.name} 应看到频道列表`).toBe(true);
    }

    // 2) 双人独立文字频道 + MLS group 就绪 + 收发同步
    const { id: chId, name: chName } = await setupIsolatedChannel(users, 'dep');
    expect(chId.length).toBeGreaterThan(0);

    const t = tag();
    const fromA = `A_${t}`;
    const fromB = `B_${t}`;
    await sendMessage(a.page, fromA);
    await sendMessage(b.page, fromB);

    await expect(b.page.getByText(fromA, { exact: false }).first()).toBeVisible({ timeout: 30_000 });
    await expect(a.page.getByText(fromB, { exact: false }).first()).toBeVisible({ timeout: 30_000 });
    await expect(a.page.getByText(fromA, { exact: false }).first()).toBeVisible();
    await expect(b.page.getByText(fromB, { exact: false }).first()).toBeVisible();

    // 重复行检查：同一条消息只应渲染一次
    for (const [page, text] of [[b.page, fromA], [a.page, fromB]] as const) {
      const count = await page.locator('[data-testid="message-row"]').filter({ hasText: text }).count();
      expect(count, `${text} 重复渲染`).toBe(1);
    }

    // 3) 附件：明文文件 + 图片，对端可见、可下载、内容一致
    const fileName = `note-${t}.txt`;
    const fileBody = `hello-deploy-${t}`;
    const chooserP = a.page.waitForEvent('filechooser');
    await a.page.getByTestId('attachment-picker').click();
    const chooser = await chooserP;
    await chooser.setFiles([{ name: fileName, mimeType: 'text/plain', buffer: Buffer.from(fileBody, 'utf8') }]);
    const fileMsg = `file-${t}`;
    await sendMessage(a.page, fileMsg);
    await expect(b.page.getByText(fileName, { exact: false }).first()).toBeVisible({ timeout: 30_000 });
    const dlP = b.page.waitForEvent('download', { timeout: 30_000 });
    await b.page.getByText(fileName, { exact: false }).first().click();
    const dl = await dlP;
    expect(dl.suggestedFilename()).toBe(fileName);
    const dlPath = await dl.path();
    expect(dlPath, 'download path').toBeTruthy();
    expect((await readFile(dlPath as string)).toString('utf8')).toBe(fileBody);

    const imgName = `pic-${t}.png`;
    const imgChooserP = a.page.waitForEvent('filechooser');
    await a.page.getByTestId('attachment-picker').click();
    const imgChooser = await imgChooserP;
    await imgChooser.setFiles([{ name: imgName, mimeType: 'image/png', buffer: TINY_PNG }]);
    const imgMsg = `img-${t}`;
    await sendMessage(a.page, imgMsg);
    const row = b.page.locator('[data-testid="message-row"]').filter({ hasText: imgMsg }).first();
    await expect(row).toBeVisible({ timeout: 30_000 });
    await expect(row.locator(`img[alt="${imgName}"]`)).toBeVisible({ timeout: 20_000 });

    // 4) 语音：双人进同一 voice 频道，两端 E2EE = true，LiveKit 房间里确有 2 人
    const vName = `voice-${t}`;
    await createNodeViaUI(a.page, { name: vName, kind: 'voice' });
    await waitForNode(a.page, vName);
    await waitForNode(b.page, vName, { timeout: 30_000 });
    const vId = await channelIdByName(a.page, vName);

    await clickChannel(a.page, vName);
    await a.page.getByTestId('media-join').click();
    await expect(a.page.getByTestId('media-connected')).toHaveAttribute('data-channel', vId, { timeout: 120_000 });
    await expect(a.page.getByTestId('media-e2ee')).toHaveAttribute('data-e2ee', 'true', { timeout: 30_000 });

    await clickChannel(b.page, vName);
    await b.page.getByTestId('media-join').click();
    await expect(b.page.getByTestId('media-connected')).toHaveAttribute('data-channel', vId, { timeout: 120_000 });
    await expect(b.page.getByTestId('media-e2ee')).toHaveAttribute('data-e2ee', 'true', { timeout: 30_000 });

    await expect.poll(async () => {
      try {
        const rooms = await listRooms();
        const hit = rooms.find(r => r.name.includes(vId) || r.name.includes(vName));
        return hit?.num_participants ?? 0;
      } catch { return -1; }
    }, { timeout: 60_000, intervals: [2000] }).toBeGreaterThanOrEqual(2);

    // 5) 全程无未预期前端错误（HTTP/console/pageerror）
    const errs = cols.flatMap(c => c.getFiltered());
    if (errs.length) console.log('unexpected errors:\n' + errs.slice(0, 20).join('\n'));
    expect(errs, `未预期错误 ${errs.length} 条`).toEqual([]);
    console.log(`[deploy] OK channel=${chName} (${chId}) voice=${vName} (${vId})`);
  } finally {
    await closeUsers(users);
  }
});
