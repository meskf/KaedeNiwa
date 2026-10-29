// KaedeNiwa Electron 主进程
// 目标（本轮）：dev 模式加载 http://127.0.0.1:5173，提供最小原生 API 桥。
// 生产模式（file:// 加载 dist）与 CORS 绕过留待下一轮。
'use strict'

const { app, BrowserWindow, ipcMain, dialog, shell, clipboard, session } = require('electron')
const path = require('node:path')
const fs = require('node:fs')

const isDev = process.argv.includes('--dev') || !app.isPackaged
const DEV_URL = 'http://127.0.0.1:5173'
const PROD_HTML = path.join(__dirname, '..', '..', 'web', 'dist', 'index.html')

// WebRTC 需要 SharedArrayBuffer（HANDOFF 第九部分）
// 通过 COOP/COEP 头启用（见 createWindow 里的 onHeadersReceived）
app.commandLine.appendSwitch('enable-features', 'SharedArrayBuffer')

// 单实例锁（避免重复开窗口）
const gotLock = app.requestSingleInstanceLock()
if (!gotLock) {
  app.quit()
} else {
  app.on('second-instance', () => {
    const wins = BrowserWindow.getAllWindows()
    if (wins.length > 0) {
      const win = wins[0]
      if (win.isMinimized()) win.restore()
      win.focus()
    }
  })
}

function createWindow() {
  const win = new BrowserWindow({
    width: 1360,
    height: 860,
    minWidth: 900,
    minHeight: 600,
    backgroundColor: '#1a1a1a',
    show: false,
    webPreferences: {
      preload: path.join(__dirname, 'preload.cjs'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false, // preload 需要 require('electron')
      webSecurity: true,
    },
  })

  // SharedArrayBuffer 需要跨域隔离。给所有响应加 COOP/COEP。
  // 注意：生产模式 file:// 加载时这两个头不生效，需要自定义协议或
  // session.protocol.handle —— 下一轮处理。
  session.defaultSession.webRequest.onHeadersReceived((details, callback) => {
    callback({
      responseHeaders: {
        ...details.responseHeaders,
        'Cross-Origin-Opener-Policy': ['same-origin'],
        'Cross-Origin-Embedder-Policy': ['require-corp'],
      },
    })
  })

  win.once('ready-to-show', () => {
    win.show()
  })

  if (isDev) {
    win.loadURL(DEV_URL)
    // 不自动开 DevTools（避免每次弹独立窗口）。F12 或 Ctrl+Shift+I 手动开。
    win.webContents.on('before-input-event', (_e, input) => {
      if (input.type === 'keyDown' && input.key === 'F12') {
        win.webContents.toggleDevTools()
      }
    })
    console.log('[main] loading dev URL:', DEV_URL)
  } else {
    console.log('[main] loading prod HTML:', PROD_HTML)
    if (!fs.existsSync(PROD_HTML)) {
      console.error('[main] prod HTML not found. Run `npm run build` in apps/web first.')
    }
    win.loadFile(PROD_HTML)
  }

  // 外部链接走系统浏览器
  win.webContents.setWindowOpenHandler(({ url }) => {
    if (/^https?:\/\//i.test(url)) shell.openExternal(url)
    return { action: 'deny' }
  })

  return win
}

// ---------- IPC handlers ----------
function registerIpc() {
  ipcMain.handle('electron:app-version', () => app.getVersion())

  ipcMain.handle('electron:open-external', async (_evt, url) => {
    if (typeof url !== 'string' || !/^https?:\/\//i.test(url)) {
      throw new Error('electron:openExternal only accepts http(s) URLs')
    }
    await shell.openExternal(url)
  })

  ipcMain.handle('electron:copy-text', (_evt, text) => {
    if (typeof text !== 'string') throw new Error('copyText expects string')
    clipboard.writeText(text)
  })

  ipcMain.handle('electron:read-text', () => clipboard.readText())

  ipcMain.handle('electron:save-file', async (_evt, opts) => {
    const { name, data, mime } = opts || {}
    if (typeof name !== 'string') throw new Error('saveFile: name required')
    const { canceled, filePath } = await dialog.showSaveDialog({
      defaultPath: name,
      filters: mime && mime !== 'application/octet-stream'
        ? [{ name: 'file', extensions: [mime.split('/').pop() || '*'] }]
        : undefined,
    })
    if (canceled || !filePath) return { canceled: true }
    const buf = Buffer.from(data)
    fs.writeFileSync(filePath, buf)
    return { canceled: false, path: filePath, bytes: buf.length }
  })
}

// ---------- app lifecycle ----------
app.whenReady().then(() => {
  registerIpc()
  createWindow()

  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow()
  })
})

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') app.quit()
})

process.on('uncaughtException', (err) => {
  console.error('[main] uncaughtException:', err)
})

process.on('unhandledRejection', (reason) => {
  console.error('[main] unhandledRejection:', reason)
})
