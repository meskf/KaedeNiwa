// Android APK 全功能冒烟：通过原生 CDP 驱动 Capacitor WebView（Playwright 不能直接接入 WebView）。
// 前置：adb reverse tcp:18080 tcp:18080；adb forward tcp:9223 localabstract:webview_devtools_remote_<pid>
// 用法：node e2e-deploy/android-smoke.mjs [accountSuffix]
const base = process.env.KAEDE_CDP ?? 'http://127.0.0.1:9223';
const suffix = process.argv[2] ?? String(Date.now()).slice(-6);
const loginId = `and${suffix}`;
const password = 'android-pass-1234';
const text = `android-hello-${suffix}`;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const targets = await (await fetch(`${base}/json`)).json();
const target = targets.find((x) => x.type === 'page');
if (!target) throw new Error('no page target (WebView devtools not exposed?)');
console.log('[android-smoke] target:', target.url);

const ws = new WebSocket(target.webSocketDebuggerUrl);
let msgId = 0;
const pending = new Map();
const consoleErrors = [];
const htmlApiResponses = [];
ws.onmessage = (e) => {
  const m = JSON.parse(e.data);
  if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); return; }
  if (m.method === 'Runtime.exceptionThrown') {
    consoleErrors.push(m.params?.exceptionDetails?.exception?.description ?? 'exception');
  }
  if (m.method === 'Runtime.consoleAPICalled' && m.params?.type === 'error') {
    consoleErrors.push(m.params.args?.map((a) => a.value ?? a.description ?? '').join(' '));
  }
  if (m.method === 'Network.responseReceived') {
    const r = m.params?.response;
    // /api/* 返回 HTML 说明命中了静态站点回退（SPA fallback），JSON 解析必然失败
    if (r && r.url.includes('/api/') && (r.mimeType ?? '').includes('html')) {
      htmlApiResponses.push({ url: r.url, status: r.status, mime: r.mimeType });
    }
  }
};
const send = (method, params = {}) => new Promise((res) => {
  const i = ++msgId; pending.set(i, res); ws.send(JSON.stringify({ id: i, method, params }));
});
await new Promise((r) => { ws.onopen = r; });

async function evalJs(expression) {
  const out = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
  if (out.result?.exceptionDetails) {
    throw new Error('JS exception: ' + (out.result.exceptionDetails.exception?.description ?? out.result.exceptionDetails.text));
  }
  return out.result?.result?.value;
}

async function waitFor(desc, fn, timeoutMs = 30000, everyMs = 500) {
  const deadline = Date.now() + timeoutMs;
  let last;
  while (Date.now() < deadline) {
    try { last = await fn(); if (last) return last; } catch (e) { last = String(e); }
    await sleep(everyMs);
  }
  throw new Error(`timeout waiting for ${desc} (last=${JSON.stringify(last)})`);
}

function assert(cond, message) {
  if (!cond) throw new Error('ASSERT FAILED: ' + message);
}

// 真实触摸点击：Android WebView 里合成 mouse 事件可能被触摸设备逻辑吞掉，
// 用 Input.dispatchTouchEvent 模拟手指（与真人操作一致）。
async function touchTap(x, y) {
  await send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x, y, id: 1 }] });
  await sleep(60);
  await send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
}

// 点击带 data-testid 的元素（先滚动到可视区，再按中心点触摸）
async function clickTestId(testId) {
  const box = await evalJs(`(() => {
    const el = document.querySelector('[data-testid="${testId}"]');
    if (!el) return null;
    el.scrollIntoView({ block: 'center' });
    const r = el.getBoundingClientRect();
    return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
  })()`);
  assert(box, `element [data-testid=${testId}] not found`);
  await touchTap(box.x, box.y);
}

// React 受控输入：用原生 setter + input 事件，等价于用户键入
async function fillTestId(testId, value) {
  const ok = await evalJs(`(() => {
    const el = document.querySelector('[data-testid="${testId}"]');
    if (!el) return false;
    const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
    el.focus();
    set.call(el, ${JSON.stringify(value)});
    el.dispatchEvent(new Event('input', { bubbles: true }));
    return el.value === ${JSON.stringify(value)};
  })()`);
  assert(ok, `fill ${testId} failed`);
}

try {
  await send('Runtime.enable');
  await send('Network.enable');

  // 1) 注册（首个注册者才是服主；这里用访客账号验证“加入已有服务器”的普通路径）
  // 登录页/注册页共用同一组输入框，按当前实际 DOM 判断，已登录则跳过。
  const hasAccountInput = await evalJs(`!!document.querySelector('[data-testid="login-id"]')`);
  if (hasAccountInput) {
    // 注册态标题：create an account / 创建账号；登录态：welcome back / 欢迎回来
    const inRegisterMode = await evalJs(`/create an account|创建账号/i.test(document.body.innerText)`);
    if (!inRegisterMode) {
      await clickTestId('login-toggle-register');
      await waitFor('register form', () => evalJs(`/create an account|创建账号/i.test(document.body.innerText)`));
    }
    await fillTestId('login-id', loginId);
    await fillTestId('login-password', password);
    assert(
      await evalJs(`document.querySelector('[data-testid="login-id"]').value === ${JSON.stringify(loginId)}
        && document.querySelector('[data-testid="login-password"]').value === ${JSON.stringify(password)}`),
      'form values not applied',
    );
    // 收起软键盘：输入框聚焦时 WebView 的 visualViewport 会缩小，
    // 直接按 layout 坐标点击会打偏（真机上也是“第一次点只是收键盘”）。
    await evalJs(`document.activeElement?.blur()`);
    await sleep(800);
    await clickTestId('login-submit');
    // 触摸兜底：若 4s 内没反应，说明这一下没打中（或 WebView 吞掉了），
    // 重试一次触摸，仍失败才退化为 JS click 并显式标记，避免“静默超时”。
    await sleep(4000);
    if (!(await evalJs(`!!JSON.parse(localStorage.getItem('kaede-auth') || '{}').state?.token`))) {
      console.log('[android-smoke] WARN tap did not submit, retry tap');
      await clickTestId('login-submit');
      await sleep(3000);
      if (!(await evalJs(`!!JSON.parse(localStorage.getItem('kaede-auth') || '{}').state?.token`))) {
        console.log('[android-smoke] WARN touch tap blocked, fallback to JS click');
        await evalJs(`document.querySelector('[data-testid="login-submit"]').click()`);
      }
    }
    // 登录完成的唯一可靠信号：auth store 里拿到 token（__kaede_debug__ 在登录页就已存在）
    const outcome = await waitFor('login outcome', async () => {
      const st = await evalJs(`(() => {
        const err = document.querySelector('.text-red-500')?.innerText ?? '';
        let token = null;
        try { token = JSON.parse(localStorage.getItem('kaede-auth') || '{}').state?.token ?? null } catch {}
        return { err, token };
      })()`);
      return st.token ? st : (st.err ? st : null);
    }, 45000);
    assert(outcome.token, 'register/login failed: ' + (outcome.err || 'no token in auth store'));
    console.log('[android-smoke] registered & logged in:', loginId);
  } else {
    console.log('[android-smoke] already logged in, skip register');
  }

  // 2) 登录成功：__kaede_debug__ 可用且频道列表非空
  const channels = await waitFor('channel list', async () => {
    const list = await evalJs(`(async () => {
      const dbg = window.__kaede_debug__;
      if (!dbg) return null;
      const l = await dbg.channels.list();
      return Array.isArray(l) && l.length ? l.map(c => ({ id: c.id, name: c.name, kind: c.kind })) : null;
    })()`);
    return list;
  }, 60000);
  console.log('[android-smoke] registered & logged in:', loginId, 'channels:', channels.length);

  // 3) 打开一个文字频道（真实点击频道树节点）
  const textCh = channels.find((c) => c.kind === 'text');
  assert(textCh, 'no text channel on server (need a channel created by op5)');
  const nodeBox = await waitFor('text channel node', async () => {
    const b = await evalJs(`(() => {
      const nodes = Array.from(document.querySelectorAll('[data-node-id]'));
      const el = nodes.find(n => n.innerText.trim().includes(${JSON.stringify(textCh.name)}));
      if (!el) return null;
      el.scrollIntoView({ block: 'center' });
      const r = el.getBoundingClientRect();
      return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
    })()`);
    return b;
  });
  await touchTap(nodeBox.x, nodeBox.y);

  // 4) 频道 MLS group 建立 + 历史同步
  const st = await waitFor('mls group', async () => {
    const s = await evalJs(`window.__kaede_debug__.channelState(${JSON.stringify(textCh.id)})`);
    return s && s.has_group ? s : null;
  }, 60000);
  console.log('[android-smoke] channel group ready:', textCh.name, 'epoch=', st.epoch ?? st.epoch_num ?? '?');

  // 5) 通过 UI 输入框发消息（Input.insertText + Enter，走 Lexical 真实输入链路）
  await waitFor('composer', () => evalJs(`!!document.querySelector('[data-testid="message-input"]')`));
  const composerBox = await evalJs(`(() => {
    const el = document.querySelector('[data-testid="message-input"]');
    el.focus();
    const r = el.getBoundingClientRect();
    return { x: r.x + 20, y: r.y + Math.min(20, r.height / 2) };
  })()`);
  await touchTap(composerBox.x, composerBox.y);
  await send('Input.insertText', { text });
  await sleep(300);
  for (const type of ['rawKeyDown', 'keyUp']) {
    await send('Input.dispatchKeyEvent', { type, key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 13 });
  }

  // 6) 消息上屏（且只出现一次），历史回读也能拿到
  const rendered = await waitFor('rendered message', async () => {
    return await evalJs(`document.querySelectorAll('[data-testid="message-row"]').length
      ? Array.from(document.querySelectorAll('[data-testid="message-row"]')).filter(r => r.innerText.includes(${JSON.stringify(text)})).length
      : 0`);
  }, 30000);
  assert(rendered === 1, `message rendered ${rendered} times, expected 1`);

  const history = await waitFor('history echo', async () => {
    // loadHistory 直接返回消息数组（历史上有的版本返回 { messages: [...] }，两种都兼容）
    const rows = await evalJs(`(async () => {
      const r = await window.__kaede_debug__.loadHistory(${JSON.stringify(textCh.id)}, 20);
      return Array.isArray(r) ? r : (r && Array.isArray(r.messages) ? r.messages : []);
    })()`);
    return Array.isArray(rows) && rows.some((m) => JSON.stringify(m).includes(text));
  }, 30000);
  assert(history, 'history does not contain sent message');

  assert(
    consoleErrors.length === 0,
    'unexpected console errors: ' + consoleErrors.slice(0, 5).join(' | ')
      + (htmlApiResponses.length ? ' ; html api responses: ' + JSON.stringify(htmlApiResponses.slice(0, 5)) : ''),
  );
  console.log('[android-smoke] PASS', JSON.stringify({ loginId, channel: textCh.name, text, rendered, history }));
} finally {
  ws.close();
}
