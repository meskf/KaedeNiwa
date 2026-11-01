// KaedeNiwa Electron preload
'use strict'

const { contextBridge, ipcRenderer } = require('electron')

contextBridge.exposeInMainWorld('electronAPI', {
  platform: 'electron',
  appVersion: () => ipcRenderer.invoke('electron:app-version'),
  openExternal: (url) => ipcRenderer.invoke('electron:open-external', url),
  copyText: (text) => ipcRenderer.invoke('electron:copy-text', text),
  readText: () => ipcRenderer.invoke('electron:read-text'),
  saveFile: (opts) => ipcRenderer.invoke('electron:save-file', opts),
  getServerUrl: () => ipcRenderer.invoke('electron:get-server-url'),
  setServerUrl: (url) => ipcRenderer.invoke('electron:set-server-url', url),
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
