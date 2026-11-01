import { test, expect, _electron as electron } from '@playwright/test';
import path from 'node:path';
import os from 'node:os';
import fs from 'node:fs';

test('KaedeNiwa 客户端自动化验收', async () => {
  const installedExe = path.resolve('D:/dev2/kaedeniwa/release/desktop/win-unpacked/KaedeNiwa.exe');
  
  // 创建一个独立的临时 userData 目录，避免缓存锁冲突
  const userDataDir = path.join(os.tmpdir(), `kaede-test-${Date.now()}`);
  fs.mkdirSync(userDataDir, { recursive: true });

  const app = await electron.launch({
    executablePath: installedExe,
    args: [
      `--user-data-dir=${userDataDir}`,
      '--no-sandbox',
      '--disable-gpu'
    ],
    env: {
      ...process.env,
      PATH: `C:\\Windows\\System32;${process.env.PATH}`
    }
  });

  const window = await app.firstWindow();
  await window.waitForLoadState('domcontentloaded');

  console.log('[AutoTest] 正在等待登录界面渲染...');
  const loginIdInput = window.getByTestId('login-id');
  await expect(loginIdInput).toBeVisible({ timeout: 20000 });

  const serverAddress = await window.locator('input').first().inputValue();
  console.log('[AutoTest] 成功读取到默认服务器地址:', serverAddress);

  const screenshotPath = path.resolve('D:/dev2/kaedeniwa/release/desktop/auto-test-screenshot.png');
  await window.screenshot({ path: screenshotPath });
  console.log('[AutoTest] 已保存截图到:', screenshotPath);

  await app.close();
  
  // 清理临时目录
  fs.rmSync(userDataDir, { recursive: true, force: true });
});
