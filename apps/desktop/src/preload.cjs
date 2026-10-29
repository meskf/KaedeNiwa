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

contextBridge.exposeInMainWorld('windowControls', {
  minimize: () => ipcRenderer.invoke('window:minimize'),
  toggleMaximize: () => ipcRenderer.invoke('window:maximize-toggle'),
  close: () => ipcRenderer.invoke('window:close'),
  isMaximized: () => ipcRenderer.invoke('window:is-maximized'),
  onMaximizeChange: (cb) => {
    const handler = (_e, isMax) => cb(isMax)
    ipcRenderer.on('window:maximize-changed', handler)
    return () => ipcRenderer.removeListener('window:maximize-changed', handler)
  },
})

console.log('[preload] electronAPI exposed')
