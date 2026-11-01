// KaedeNiwa Electron 主进程
// 本地 HTTP server：serve dist/web + 代理 /api/* 到用户配置的服务器
'use strict'

const { app, BrowserWindow, ipcMain, dialog, shell, clipboard, session, desktopCapturer } = require('electron')
const path = require('node:path')
const fs = require('node:fs')
const http = require('node:http')
const https = require('node:https')
const { URL } = require('node:url')

const isDev = process.argv.includes('--dev') || !app.isPackaged
const DEV_URL = 'http://127.0.0.1:5173'

// ---- 本地 server 状态 ----
let localServer = null
let localServerPort = 0
let proxyTarget = '' // 例如 'https://chat.example.com'，由用户登录时通过 IPC 设置

const MIME_MAP = {
  '.html': 'text/html; charset=utf-8',
  '.js':   'application/javascript; charset=utf-8',
  '.mjs':  'application/javascript; charset=utf-8',
  '.css':  'text/css; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.wasm': 'application/wasm',
  '.png':  'image/png', '.jpg': 'image/jpeg', '.jpeg': 'image/jpeg',
  '.svg':  'image/svg+xml', '.ico': 'image/x-icon', '.webp': 'image/webp',
  '.woff': 'font/woff', '.woff2': 'font/woff2', '.ttf': 'font/ttf',
  '.map':  'application/json', '.txt': 'text/plain; charset=utf-8',
}

const BASE_HEADERS = {
  'Cross-Origin-Opener-Policy': 'same-origin',
  'Cross-Origin-Embedder-Policy': 'require-corp',
  'Cross-Origin-Resource-Policy': 'same-origin',
}

// ---------- 配置文件 ----------
function getConfigPath() {
  return path.join(app.getPath('userData'), 'config.json')
}
function readConfig() {
  try { return JSON.parse(fs.readFileSync(getConfigPath(), 'utf8')) } catch { return {} }
}
function writeConfig(patch) {
  const cur = readConfig()
  const merged = { ...cur, ...patch }
  try {
    fs.mkdirSync(path.dirname(getConfigPath()), { recursive: true })
    fs.writeFileSync(getConfigPath(), JSON.stringify(merged, null, 2))
  } catch (e) { console.error('[config] write failed:', e) }
}

// ---------- 本地 HTTP server ----------
function startLocalServer(distDir) {
  return new Promise((resolve, reject) => {
    const server = http.createServer((req, res) => {
      try {
        if (req.url.startsWith('/api/') || req.url === '/api') {
          return proxyApi(req, res)
        }
        return serveStatic(req, res, distDir)
      } catch (e) {
        console.error('[local-server] handler error:', e)
        res.writeHead(500, BASE_HEADERS); res.end('Internal Error')
      }
    })
    server.on('error', reject)
    server.listen(0, '127.0.0.1', () => {
      localServerPort = server.address().port
      localServer = server
      console.log('[local-server] listening on 127.0.0.1:' + localServerPort)
      resolve(localServerPort)
    })
  })
}

function serveStatic(req, res, distDir) {
  const urlPath = decodeURIComponent((req.url || '/').split('?')[0])
  let filePath = path.join(distDir, urlPath === '/' ? 'index.html' : urlPath)
  const normalized = path.normalize(filePath)
  if (!normalized.startsWith(path.normalize(distDir))) {
    res.writeHead(403, BASE_HEADERS); res.end('Forbidden'); return
  }
  fs.stat(filePath, (err, stat) => {
    if (err || !stat.isFile()) {
      // SPA fallback
      filePath = path.join(distDir, 'index.html')
    }
    const ext = path.extname(filePath).toLowerCase()
    const mimeType = MIME_MAP[ext] || 'application/octet-stream'
    fs.readFile(filePath, (readErr, data) => {
      if (readErr) {
        res.writeHead(404, BASE_HEADERS); res.end('Not Found'); return
      }
      res.writeHead(200, {
        ...BASE_HEADERS,
        'Content-Type': mimeType,
        'Content-Length': data.length,
        'Cache-Control': ext === '.html' ? 'no-cache' : 'public, max-age=31536000, immutable',
      })
      res.end(data)
    })
  })
}

function proxyApi(req, res) {
  if (!proxyTarget) {
    res.writeHead(503, { ...BASE_HEADERS, 'Content-Type': 'application/json' })
    res.end(JSON.stringify({ error: 'server_not_configured', message: 'Please log in first' }))
    return
  }
  let targetUrl
  try { targetUrl = new URL(req.url, proxyTarget) } catch (e) {
    res.writeHead(400, BASE_HEADERS); res.end('Bad proxy target'); return
  }
  const isHttps = targetUrl.protocol === 'https:'
  const mod = isHttps ? https : http
  const headers = { ...req.headers, host: targetUrl.host }
  delete headers['connection']
  delete headers['content-length'] // 让下游自己算
  const proxyReq = mod.request({
    hostname: targetUrl.hostname,
    port: targetUrl.port || (isHttps ? 443 : 80),
    path: targetUrl.pathname + targetUrl.search,
    method: req.method,
    headers,
  }, (proxyRes) => {
    const resHeaders = { ...BASE_HEADERS, ...proxyRes.headers }
    delete resHeaders['content-encoding'] // 让浏览器按 body 重新解码
    res.writeHead(proxyRes.statusCode || 502, resHeaders)
    proxyRes.pipe(res)
  })
  proxyReq.on('error', (err) => {
    console.error('[proxy] error:', err.message)
    res.writeHead(502, { ...BASE_HEADERS, 'Content-Type': 'application/json' })
    res.end(JSON.stringify({ error: 'proxy_error', message: err.message }))
  })
  req.pipe(proxyReq)
}

// ---------- 最大化状态 ----------
let isMaxState = false

app.commandLine.appendSwitch('enable-features', 'SharedArrayBuffer')

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
    width: 1360, height: 860, minWidth: 900, minHeight: 600,
    backgroundColor: '#1a1a1a', show: false, frame: false,
    roundedCorners: false, titleBarStyle: 'hidden', hasShadow: true,
    thickFrame: false, autoHideMenuBar: true,
    webPreferences: {
      preload: path.join(__dirname, 'preload.cjs'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false,
      webSecurity: true,
    },
  })

  win.once('ready-to-show', () => win.show())

  if (isDev) {
    win.loadURL(DEV_URL)
    win.webContents.on('before-input-event', (_e, input) => {
      if (input.type === 'keyDown' && input.key === 'F12') win.webContents.toggleDevTools()
    })
    console.log('[main] dev mode, loadURL', DEV_URL)
  } else {
    if (!localServerPort) {
      console.error('[main] local server not started, cannot load')
      return win
    }
    const url = 'http://127.0.0.1:' + localServerPort + '/'
    win.loadURL(url)
    win.webContents.on('before-input-event', (_e, input) => {
      if (input.type === 'keyDown' && input.key === 'F12') win.webContents.toggleDevTools()
    })
    console.log('[main] prod mode, loadURL', url)
  }

  win.webContents.setWindowOpenHandler(({ url }) => {
    if (/^https?:\/\//i.test(url)) shell.openExternal(url)
    return { action: 'deny' }
  })

  win.on('maximize', () => {
    if (isMaxState) return
    isMaxState = true
    win.webContents.send('window:maximize-changed', true)
  })
  win.on('unmaximize', () => {
    if (!isMaxState) return
    isMaxState = false
    win.webContents.send('window:maximize-changed', false)
  })

  win.webContents.on('before-input-event', (_e, input) => {
    if (input.type === 'keyDown' && input.control && input.shift && input.key.toLowerCase() === 'q') {
      app.quit()
    }
  })

  return win
}

function registerIpc() {
  ipcMain.handle('electron:app-version', () => app.getVersion())
  ipcMain.handle('electron:open-external', async (_e, url) => {
    if (typeof url !== 'string' || !/^https?:\/\//i.test(url)) throw new Error('only http(s) urls')
    await shell.openExternal(url)
  })
  ipcMain.handle('electron:copy-text', (_e, text) => {
    if (typeof text !== 'string') throw new Error('copyText expects string')
    clipboard.writeText(text)
  })
  ipcMain.handle('electron:read-text', () => clipboard.readText())
  ipcMain.handle('electron:save-file', async (_e, opts) => {
    const { name, data, mime } = opts || {}
    if (typeof name !== 'string') throw new Error('saveFile: name required')
    const { canceled, filePath } = await dialog.showSaveDialog({
      defaultPath: name,
      filters: mime && mime !== 'application/octet-stream'
        ? [{ name: 'file', extensions: [mime.split('/').pop() || '*'] }] : undefined,
    })
    if (canceled || !filePath) return { canceled: true }
    const buf = Buffer.from(data)
    fs.writeFileSync(filePath, buf)
    return { canceled: false, path: filePath, bytes: buf.length }
  })

  // ---- 服务器地址配置 ----
  ipcMain.handle('electron:get-server-url', () => proxyTarget || readConfig().serverUrl || '')
  ipcMain.handle('electron:set-server-url', (_e, url) => {
    if (typeof url !== 'string') throw new Error('setServerUrl expects string')
    // 规范化：去掉尾部斜杠
    proxyTarget = url.replace(/\/+$/, '')
    writeConfig({ serverUrl: proxyTarget })
    console.log('[main] proxy target set to', proxyTarget)
    return { ok: true, serverUrl: proxyTarget }
  })

  // ---- 窗口控制 ----
  ipcMain.handle('window:minimize', (e) => {
    const w = BrowserWindow.fromWebContents(e.sender); if (w) w.minimize()
  })
  ipcMain.handle('window:maximize-toggle', (e) => {
    const w = BrowserWindow.fromWebContents(e.sender); if (!w) return
    const before = isMaxState
    isMaxState = !before
    if (before) w.unmaximize(); else w.maximize()
    w.webContents.send('window:maximize-changed', isMaxState)
    setTimeout(() => {
      if (w.isDestroyed()) return
      const real = w.isMaximized()
      if (real !== isMaxState) {
        isMaxState = real
        w.webContents.send('window:maximize-changed', real)
      }
    }, 300)
  })
  ipcMain.handle('window:close', (e) => {
    const w = BrowserWindow.fromWebContents(e.sender); if (w) w.close()
  })
  ipcMain.handle('window:is-maximized', () => isMaxState)
}

app.whenReady().then(async () => {
  // 共享屏幕 handler
  session.defaultSession.setDisplayMediaRequestHandler((request, callback) => {
    desktopCapturer.getSources({ types: ['screen'] }).then((sources) => {
      if (sources.length > 0) callback({ video: sources[0], audio: 'loopback' })
      else callback({})
    }).catch((err) => { console.error('[main] desktopCapturer error:', err); callback({}) })
  }, { useSystemPicker: true })

  // dev 模式下用 webRequest 加 COOP/COEP；prod 模式下由本地 server 加
  if (isDev) {
    session.defaultSession.webRequest.onHeadersReceived((details, callback) => {
      callback({
        responseHeaders: {
          ...details.responseHeaders,
          'Cross-Origin-Opener-Policy': ['same-origin'],
          'Cross-Origin-Embedder-Policy': ['require-corp'],
        },
      })
    })
  }

  registerIpc()

  // 读取上次保存的 serverUrl
  const cfg = readConfig()
  proxyTarget = (cfg.serverUrl || '').replace(/\/+$/, '')
  console.log('[main] loaded proxy target:', proxyTarget || '(none)')

  // prod 模式启动本地 HTTP server
  if (!isDev) {
    const distDir = app.isPackaged
      ? path.join(process.resourcesPath, 'web')
      : path.join(__dirname, '..', '..', 'web', 'dist')
    if (!fs.existsSync(path.join(distDir, 'index.html'))) {
      console.error('[main] dist index.html not found at', distDir)
    }
    try {
      await startLocalServer(distDir)
    } catch (e) {
      console.error('[main] failed to start local server:', e)
    }
  }

  createWindow()

  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow()
  })
})

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') app.quit()
})

app.on('before-quit', () => {
  if (localServer) { try { localServer.close() } catch {} }
})

process.on('uncaughtException', (err) => console.error('[main] uncaughtException:', err))
process.on('unhandledRejection', (reason) => console.error('[main] unhandledRejection:', reason))
