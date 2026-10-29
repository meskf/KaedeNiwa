// KaedeNiwa Electron preload
// 通过 contextBridge 暴露最小原生 API。apps/web/src/platform/index.ts
// 会在下一轮检测 window.electronAPI 并切换到 electron.ts 实现。
'use strict'

const { contextBridge, ipcRenderer } = require('electron')

contextBridge.exposeInMainWorld('electronAPI', {
  platform: 'electron',
  appVersion: () => ipcRenderer.invoke('electron:app-version'),
  openExternal: (url) => ipcRenderer.invoke('electron:open-external', url),
  copyText: (text) => ipcRenderer.invoke('electron:copy-text', text),
  readText: () => ipcRenderer.invoke('electron:read-text'),
  saveFile: (opts) => ipcRenderer.invoke('electron:save-file', opts),
})

console.log('[preload] electronAPI exposed')
