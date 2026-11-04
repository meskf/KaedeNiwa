import { Page } from '@playwright/test';

export type ErrorCollector = {
  /** 全部错误（稳态用例用，严格） */
  get: () => string[];
  /** 去掉混沌/异常注入里不可避免的网络层噪音 */
  getFiltered: () => string[];
  /** 被 getFiltered 剔除的噪音（仅诊断） */
  getNoise: () => string[];
  clear: () => void;
  events: () => Array<{ kind: string; detail: any }>;
};

const IGNORE_404 = [/\/api\/avatars\/.+/, /\/favicon\.ico/, /\.well-known\//, /\.map$/];

// join / takeover / members 的 4xx 是协议内正常信号，UI 会处理。
const JOIN_HTTP_NOISE = /\/api\/channels\/[^/]+\/join$/;
const TAKEOVER_HTTP_NOISE = /\/api\/channels\/[^/]+\/takeover$/;
const MEMBERS_HTTP_NOISE = /\/api\/channels\/[^/]+\/members$/;
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
    } catch {
      /* relative URL */
    }
  }
  if (TAKEOVER_HTTP_NOISE.test(url) && status === 409) return true;
  return JOIN_HTTP_NOISE.test(url) && [400, 401, 409].includes(status);
}

// 异常注入（断网/浏览器停止共享/取消拾取器）必然出现的浏览器层噪音。
const EXPECTED_NET_NOISE: RegExp[] = [
  /^Failed to load resource: net::ERR_/,
  /^send failed: Send error:/,
  /^transport error: http: error sending request/,
  /^transport error: .*Failed to fetch/,
  /^http: error sending request for url/,
  /^TypeError: Failed to fetch/,
  /^\[initChannel\] join failed/,
  /^\[initChannel\] giving up/,
  /^\[initChannel\] retry/,
  /^load nodes err:/,
  /^WebSocket connection to .*failed: Error in connection establishment: net::/,
  // 用户取消共享是产品预期分支，MediaBar 会把错误渲染给用户
  /NotAllowedError/,
  /user cancelled/,
  /OverconstrainedError/,
  /No screen capture/,
];

function isExpectedNoise(line: string): boolean {
  const text = line.replace(/^\[\w+\] console\.error:\s*/, '');
  return EXPECTED_NET_NOISE.some(re => re.test(text));
}

function formatConsole(raw: string, args: any[]): string {
  if (!/%[sdifoOc]/.test(raw)) return raw;
  const subs = args.length > 1 && typeof args[0] === 'string' && args[0].includes('%') ? args.slice(1) : args;
  if (subs.length === 0) return raw;
  let i = 0;
  return raw.replace(/%[sdifoOc]/g, () => {
    const v = subs[i++];
    if (v === undefined) return '%?';
    if (typeof v === 'object') {
      try {
        return JSON.stringify(v);
      } catch {
        return String(v);
      }
    }
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

  page.on('response', res => {
    const status = res.status();
    if (status < 400) return;
    const url = res.url();
    if (shouldIgnore(url, status)) return;
    const req = res.request();
    if (isExpectedHttpNoise(url, status)) {
      events.push({ kind: 'http-noise', detail: { status, method: req.method(), url } });
      return;
    }
    errors.push(`[${tag}] http ${status} ${req.method()} ${url}`);
    events.push({ kind: 'http', detail: { status, method: req.method(), url } });
  });

  page.on('requestfailed', req => {
    const url = req.url();
    if (url.startsWith('data:') || url.includes('favicon') || url.endsWith('.map')) return;
    const failure = req.failure()?.errorText ?? 'unknown';
    if (failure.includes('net::ERR_ABORTED')) return;
    if (failure.includes('net::ERR_INTERNET_DISCONNECTED')) return;
    if (failure.includes('net::ERR_NAME_NOT_RESOLVED')) return;
    if (failure.includes('net::ERR_NETWORK_CHANGED')) return;
    if (failure.includes('net::ERR_CONNECTION_RESET')) return;
    errors.push(`[${tag}] reqfailed ${req.method()} ${url} -> ${failure}`);
    events.push({ kind: 'reqfail', detail: { method: req.method(), url, failure } });
  });

  page.on('console', async m => {
    if (m.type() !== 'error') return;
    const raw = m.text();
    if (/^Failed to load resource: the server responded with a status of \d+/.test(raw)) return;
    let args: any[] = [];
    try {
      args = await Promise.all(m.args().map(a => a.jsonValue().catch(() => null)));
    } catch {
      /* 参数不可序列化时忽略 */
    }
    const msg = formatConsole(raw, args);
    errors.push(`[${tag}] console.error: ${msg}`);
    events.push({ kind: 'console', detail: { text: msg, rawText: raw, args } });
  });

  return {
    get: () => errors.slice(),
    getFiltered: () => errors.filter(e => !isExpectedNoise(e)),
    getNoise: () => errors.filter(e => isExpectedNoise(e)),
    clear: () => {
      errors.length = 0;
      events.length = 0;
    },
    events: () => events.slice(),
  };
}
