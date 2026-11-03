import { Page } from '@playwright/test';

export type ErrorCollector = {
  /** 全部错误（稳态 spec 用，保持严格） */
  get: () => string[];
  /** 去掉混沌测试里不可避免的网络层噪音（混沌 spec 用） */
  getFiltered: () => string[];
  /** 被 getFiltered 剔除的噪音（仅诊断，不算失败） */
  getNoise: () => string[];
  clear: () => void;
  events: () => Array<{ kind: string; detail: any }>;
};

const IGNORE_404 = [/\/api\/avatars\/.+/, /\/favicon\.ico/, /\.well-known\//, /\.map$/];
// join 接口的 400/401/409 是协议内正常信号：
//   409 = solo first-member，客户端据此自建 group
//   401 = 私密频道密码错误（PasswordPrompt 会显示）
//   400 = join 参数/限流
// 这些都会在 UI 层被处理，不应算作“未预期错误”。
const JOIN_HTTP_NOISE = /\/api\/channels\/[^/]+\/join$/;
const TAKEOVER_HTTP_NOISE = /\/api\/channels\/[^/]+\/takeover$/;
// 非成员读取私密频道成员列表会被 403 拒绝，这是正确的权限语义；
// 点击私密频道时 UI 会先探测一次，不应算测试失败。
const MEMBERS_HTTP_NOISE = /\/api\/channels\/[^/]+\/members$/;
// 权限不足的 403 是 UI 明确展示给用户的结果（“需要管理员权限”），
// 权限测试里属于预期，不是未捕获错误。
const PERM_HTTP_NOISE = [/^\/api\/server$/, /^\/api\/members\/[^/]+$/];
function shouldIgnore(url: string, status: number) {
  if (status !== 404) return false;
  return IGNORE_404.some(re => re.test(url));
}
function isExpectedHttpNoise(url: string, status: number) {
  if (status === 403) {
    if (MEMBERS_HTTP_NOISE.test(url)) return true;
    try {
      const path = new URL(url).pathname;
      if (PERM_HTTP_NOISE.some(re => re.test(path))) return true;
    } catch { /* relative URL */ }
  }
  if (TAKEOVER_HTTP_NOISE.test(url) && status === 409) return true;
  return JOIN_HTTP_NOISE.test(url) && [400, 401, 409].includes(status);
}

// 混沌测试里必然出现的浏览器/网络层噪音。这些不是产品 bug：
//   - Failed to load resource: net::ERR_*  （浏览器标准）
//   - send failed: Send error: ...        （wasm 客户端 catch 后打 console.error，附 wasm stack）
//   - http: error sending request for url （reqwest 内部错误）
// 稳态 spec 仍用 get() 保持严格；混沌 spec 用 getFiltered() 剥离这些。
const EXPECTED_NET_NOISE: RegExp[] = [
  /^Failed to load resource: net::ERR_/,
  /^send failed: Send error:/,
  /^transport error: http: error sending request/,
  /^transport error: .*Failed to fetch/,
  /^http: error sending request for url/,
  /^TypeError: Failed to fetch/,
  // 长时间断网时客户端重试/节点的预期日志（连接恢复后自动消失）
  /^\[initChannel\] join failed/,
  /^\[initChannel\] giving up/,
  /^\[initChannel\] retry/,
  /^load nodes err:/,
  /^WebSocket connection to .*failed: Error in connection establishment: net::/,
];

function isExpectedNoise(line: string): boolean {
  // line 形如 "[u0] console.error: <text>"；剥离前缀后再匹配
  const text = line.replace(/^\[\w+\] console\.error:\s*/, '');
  return EXPECTED_NET_NOISE.some(re => re.test(text));
}

function formatConsole(raw: string, args: any[]): string {
  if (!/%[sdifoOc]/.test(raw)) return raw;
  // console.error(format, ...subs) 时 args[0]=format 本身
  const subs = (args.length > 1 && typeof args[0] === 'string' && args[0].includes('%')) ? args.slice(1) : args;
  if (subs.length === 0) return raw;
  let i = 0;
  return raw.replace(/%[sdifoOc]/g, () => {
    const v = subs[i++];
    if (v === undefined) return '%?';
    if (typeof v === 'object') { try { return JSON.stringify(v); } catch { return String(v); } }
    return String(v);
  });
}

export function attachErrorCollector(page: Page, tag = ''): ErrorCollector {
  const errors: string[] = [];
  const events: Array<{ kind: string; detail: any }> = [];

  page.on('pageerror', e => {
    errors.push(`[${tag}] pageerror: ${e.message}`);
    events.push({ kind: 'pageerror', detail: { message: e.message } });
  });

  page.on('response', (res) => {
    const s = res.status();
    if (s < 400) return;
    const url = res.url();
    if (shouldIgnore(url, s)) return;
    const req = res.request();
    if (isExpectedHttpNoise(url, s)) {
      events.push({ kind: 'http-noise', detail: { status: s, method: req.method(), url, resourceType: req.resourceType() } });
      return;
    }
    errors.push(`[${tag}] http ${s} ${req.method()} ${url}`);
    events.push({ kind: 'http', detail: { status: s, method: req.method(), url, resourceType: req.resourceType() } });
  });

  page.on('requestfailed', req => {
    const u = req.url();
    if (u.startsWith('data:') || u.includes('favicon') || u.endsWith('.map')) return;
    const failure = req.failure()?.errorText ?? 'unknown';
    if (failure.includes('net::ERR_ABORTED')) return;
    // CDP emulateNetwork / ctx.setOffline 会触发以下错误，属于测试基础设施，不算产品错误
    if (failure.includes('net::ERR_INTERNET_DISCONNECTED')) return;
    if (failure.includes('net::ERR_NAME_NOT_RESOLVED')) return;
    if (failure.includes('net::ERR_NETWORK_CHANGED')) return;
    if (failure.includes('net::ERR_CONNECTION_RESET')) return;
    errors.push(`[${tag}] reqfailed ${req.method()} ${u} -> ${failure}`);
    events.push({ kind: 'reqfail', detail: { method: req.method(), url: u, failure } });
  });

  page.on('console', async (m) => {
    if (m.type() !== 'error') return;
    const raw = m.text();
    if (/^Failed to load resource: the server responded with a status of \d+/.test(raw)) return;
    let args: any[] = [];
    try { args = await Promise.all(m.args().map(a => a.jsonValue().catch(() => null))); } catch {}
    const msg = formatConsole(raw, args);
    errors.push(`[${tag}] console.error: ${msg}`);
    events.push({ kind: 'console', detail: { text: msg, rawText: raw, args } });
  });

  return {
    get: () => errors.slice(),
    getFiltered: () => errors.filter(e => !isExpectedNoise(e)),
    getNoise: () => errors.filter(e => isExpectedNoise(e)),
    clear: () => { errors.length = 0; events.length = 0; },
    events: () => events.slice(),
  };
}
