// KaedeNiwa Electron 主进程
// 目标（本轮）：dev 模式加载 http://127.0.0.1:5173，提供最小原生 API 桥。
// 生产模式（file:// 加载 dist）与 CORS 绕过留待下一轮。
'use strict'

const { app, BrowserWindow, ipcMain, dialog, shell, clipboard, session, desktopCapturer } = require('electron')
const path = require('node:path')
const fs = require('node:fs')

const isDev = process.argv.includes('--dev') || !app.isPackaged
const DEV_URL = 'http://127.0.0.1:5173'
const PROD_HTML = path.join(__dirname, '..', '..', 'web', 'dist', 'index.html')

// 维护最大化状态（Windows 上 w.isMaximized() 在 maximize/unmaximize 后
// 短时间返回旧值，unmaximize 有时不触发事件——这是 Electron 已知怪癖。
// 用事件维护的布尔量代替同步读取，比 w.isMaximized() 可靠。）
let isMaxState = false

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
    frame: false,
    roundedCorners: false,
    titleBarStyle: 'hidden',
    hasShadow: true,
    thickFrame: false,
    autoHideMenuBar: true,
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

  // 最大化/还原状态变化 -> 通知 renderer（更新按钮图标）
  win.on('maximize', () => {
    if (isMaxState) return
    isMaxState = true
    win.webContents.send('window:maximize-changed', true)
    console.log('[main] maximize event -> isMaxState=true')
  })
  win.on('unmaximize', () => {
    if (!isMaxState) return
    isMaxState = false
    win.webContents.send('window:maximize-changed', false)
    console.log('[main] unmaximize event -> isMaxState=false')
  })

  // 应急快捷键 Ctrl+Shift+Q 强制退出（无边框窗口的自保险）
  win.webContents.on('before-input-event', (_e, input) => {
    if (input.type === 'keyDown' && input.control && input.shift && input.key.toLowerCase() === 'q') {
      console.log('[main] emergency quit Ctrl+Shift+Q')
      app.quit()
    }
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

  // ---------- Window controls (custom titlebar) ----------
  ipcMain.handle('window:minimize', (e) => {
    const w = BrowserWindow.fromWebContents(e.sender)
    if (!w) return
    w.minimize()
    console.log('[main] window:minimize')
  })
  ipcMain.handle('window:maximize-toggle', (e) => {
    const w = BrowserWindow.fromWebContents(e.sender)
    if (!w) return
    const before = isMaxState
    // 乐观更新：先翻转状态并通知 renderer（Windows unmaximize 异步且事件可能不触发）
    isMaxState = !before
    if (before) w.unmaximize()
    else w.maximize()
    w.webContents.send('window:maximize-changed', isMaxState)
    console.log('[main] window:maximize-toggle before=' + before + ' -> ' + (before ? 'unmaximize' : 'maximize') + ', optimistic isMax=' + isMaxState)
    // 300ms 后校验：若真实状态不符，纠正并再次通知
    setTimeout(() => {
      if (w.isDestroyed()) return
      const real = w.isMaximized()
      if (real !== isMaxState) {
        isMaxState = real
        w.webContents.send('window:maximize-changed', real)
        console.log('[main] state corrected -> isMax=' + real)
      }
    }, 300)
  })
  ipcMain.handle('window:close', (e) => {
    const w = BrowserWindow.fromWebContents(e.sender)
    if (!w) return
    console.log('[main] window:close')
    w.close()
  })
  ipcMain.handle('window:is-maximized', () => isMaxState)
}

// ---------- app lifecycle ----------
app.whenReady().then(() => {
  // 允许 getDisplayMedia (屏幕共享)
  // 默认直接给第一个屏幕，生产环境可以扩展成自己写一个多屏幕选择器
  session.defaultSession.setDisplayMediaRequestHandler((request, callback) => {
    desktopCapturer.getSources({ types: ['screen'] }).then((sources) => {
      if (sources.length > 0) {
        // audio: 'loopback' 允许共享系统声音 (Windows/macOS 支持)
        callback({ video: sources[0], audio: 'loopback' })
      } else {
        callback({})
      }
    }).catch((err) => {
      console.error('[main] desktopCapturer error:', err)
      callback({})
    })
  }, { useSystemPicker: true })

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
