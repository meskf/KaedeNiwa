const fs = require('fs')

// ============ 1. livekit.ts ============
let s = fs.readFileSync('src/lib/livekit.ts', 'utf8')

// 1.1 MediaState 扩展
const oldState = `export interface MediaState {
  connected: boolean
  channelId: string | null
  participants: ParticipantInfo[]
  screenShares: ScreenShareInfo[]
  localScreenSharing: boolean
  localMicEnabled: boolean
  localNoiseSuppression: boolean
  noiseMode: NoiseMode
  currentPresetId: string
  e2eeEnabled: boolean
}`
const newState = `export interface MediaState {
  connected: boolean
  channelId: string | null
  participants: ParticipantInfo[]
  screenShares: ScreenShareInfo[]
  localScreenSharing: boolean
  localMicEnabled: boolean
  localNoiseSuppression: boolean
  noiseMode: NoiseMode
  currentPresetId: string
  e2eeEnabled: boolean
  // audio enhancements
  deafened: boolean
  inputVolume: number
  outputVolume: number
  micDeviceId: string | null
  speakerDeviceId: string | null
  pushToTalk: boolean
  pushToTalkKey: string
  audioBitrate: number
}`
if (!s.includes(oldState)) { console.error('FAIL: MediaState'); process.exit(1) }
s = s.replace(oldState, newState)

// 1.2 模块状态变量
const oldVars = `let listeners: Set<(state: MediaState) => void> = new Set()`
const newVars = `let listeners: Set<(state: MediaState) => void> = new Set()

// audio enhancements
let currentDeafened = false
let currentInputVolume = 100
let currentOutputVolume = 100
let currentMicDeviceId: string | null = null
let currentSpeakerDeviceId: string | null = null
let currentPushToTalk = false
let currentPushToTalkKey = 'Space'
let currentAudioBitrate = 0
let bitrateTimer: ReturnType<typeof setInterval> | null = null
let lastAudioBytes = 0
let lastAudioSampleTime = 0
let pttHandler: ((e: KeyboardEvent) => void) | null = null
let pttKeyDown = false
const remoteAudioElements = new Map<string, HTMLAudioElement>()`
if (!s.includes(oldVars)) { console.error('FAIL: vars'); process.exit(1) }
s = s.replace(oldVars, newVars)

// 1.3 getState null branch
const oldNull = `      e2eeEnabled: false,
    }
  }
  const room = currentRoom`
const newNull = `      e2eeEnabled: false,
      deafened: currentDeafened,
      inputVolume: currentInputVolume,
      outputVolume: currentOutputVolume,
      micDeviceId: currentMicDeviceId,
      speakerDeviceId: currentSpeakerDeviceId,
      pushToTalk: currentPushToTalk,
      pushToTalkKey: currentPushToTalkKey,
      audioBitrate: 0,
    }
  }
  const room = currentRoom`
if (!s.includes(oldNull)) { console.error('FAIL: getState null'); process.exit(1) }
s = s.replace(oldNull, newNull)

// 1.4 getState real branch
const oldReal = `    currentPresetId,
    e2eeEnabled,
  }
}`
const newReal = `    currentPresetId,
    e2eeEnabled,
    deafened: currentDeafened,
    inputVolume: currentInputVolume,
    outputVolume: currentOutputVolume,
    micDeviceId: currentMicDeviceId,
    speakerDeviceId: currentSpeakerDeviceId,
    pushToTalk: currentPushToTalk,
    pushToTalkKey: currentPushToTalkKey,
    audioBitrate: currentAudioBitrate,
  }
}`
if (!s.includes(oldReal)) { console.error('FAIL: getState real'); process.exit(1) }
s = s.replace(oldReal, newReal)

// 1.5 TrackSubscribed audio - track element
const oldSub = `        el.play().catch(e => console.warn('[Audio] 播放失败:', e))
        console.log(\`[Audio] 已挂载并播放 \${p.identity} 的音频轨道 (\${pub.source})\`)`
const newSub = `        el.play().catch(e => console.warn('[Audio] 播放失败:', e))
        remoteAudioElements.set(track.sid || p.identity, el)
        // apply deafen/volume immediately if set
        try { (track as any).setVolume?.(currentDeafened ? 0 : currentOutputVolume / 100) } catch {}
        console.log(\`[Audio] 已挂载并播放 \${p.identity} 的音频轨道 (\${pub.source})\`)`
if (!s.includes(oldSub)) { console.error('FAIL: TrackSubscribed'); process.exit(1) }
s = s.replace(oldSub, newSub)

// 1.6 TrackUnsubscribed audio
const oldUnsub = `      if (track.kind === 'audio') {
        track.detach().forEach(el => el.remove())
        return
      }`
const newUnsub = `      if (track.kind === 'audio') {
        remoteAudioElements.delete(track.sid || p.identity)
        track.detach().forEach(el => el.remove())
        return
      }`
if (!s.includes(oldUnsub)) { console.error('FAIL: TrackUnsubscribed'); process.exit(1) }
s = s.replace(oldUnsub, newUnsub)

// 1.7 Disconnected - stop bitrate
const oldDisc = `    .on(RoomEvent.Disconnected, () => {
      currentRoom = null
      currentChannelId = null
      currentE2EEEnabled = false
      screenShareCache.clear()
      emit()
    })`
const newDisc = `    .on(RoomEvent.Disconnected, () => {
      currentRoom = null
      currentChannelId = null
      currentE2EEEnabled = false
      screenShareCache.clear()
      remoteAudioElements.clear()
      stopBitrateSampling()
      emit()
    })`
if (!s.includes(oldDisc)) { console.error('FAIL: Disconnected'); process.exit(1) }
s = s.replace(oldDisc, newDisc)

// 1.8 connectMedia start sampling
const oldConn = `  emit()
  return room
}`
const newConn = `  startBitrateSampling()
  emit()
  return room
}`
if (!s.includes(oldConn)) { console.error('FAIL: connectMedia'); process.exit(1) }
s = s.replace(oldConn, newConn)

// 1.9 disconnectMedia stop sampling
const oldDis = `  currentRoom = null
  currentChannelId = null
  currentE2EEEnabled = false
  screenShareCache.clear()
  emit()
}

/**
 * 开启屏幕共享`
const newDis = `  currentRoom = null
  currentChannelId = null
  currentE2EEEnabled = false
  screenShareCache.clear()
  remoteAudioElements.clear()
  stopBitrateSampling()
  emit()
}

/**
 * 开启屏幕共享`
if (!s.includes(oldDis)) { console.error('FAIL: disconnectMedia'); process.exit(1) }
s = s.replace(oldDis, newDis)

// 1.10 enableMicrophone - use currentMicDeviceId
const oldGetUM = `  micSourceStream = await navigator.mediaDevices.getUserMedia({
    audio: {
      echoCancellation: !disableEcho,
      noiseSuppression: false,
      autoGainControl: true,
      sampleRate: 48000,
    },
  })`
const newGetUM = `  micSourceStream = await navigator.mediaDevices.getUserMedia({
    audio: {
      ...(currentMicDeviceId ? { deviceId: { exact: currentMicDeviceId } } : {}),
      echoCancellation: !disableEcho,
      noiseSuppression: false,
      autoGainControl: true,
      sampleRate: 48000,
    },
  })`
if (!s.includes(oldGetUM)) { console.error('FAIL: getUserMedia'); process.exit(1) }
s = s.replace(oldGetUM, newGetUM)

// 1.11 append new exported functions at end
const tail = `export async function toggleScreenShare(enable: boolean, presetId = '1080p60'): Promise<boolean> {
  if (enable) {
    return startScreenShare(presetId)
  } else {
    await stopScreenShare()
    return false
  }
}`
const newTail = tail + `

// ===================================================================
// Audio enhancements (deafen / volume / devices / PTT / bitrate)
// ===================================================================

function applyOutputVolume() {
  if (!currentRoom) return
  const vol = currentDeafened ? 0 : currentOutputVolume / 100
  currentRoom.remoteParticipants.forEach(p => {
    p.audioTrackPublications.forEach(pub => {
      const tr: any = pub.track
      if (tr && typeof tr.setVolume === 'function') {
        try { tr.setVolume(vol) } catch {}
      }
    })
  })
  remoteAudioElements.forEach(el => {
    try { el.volume = vol } catch {}
  })
}

export function setDeafen(deafened: boolean) {
  currentDeafened = deafened
  applyOutputVolume()
  emit()
}

export async function setInputVolume(vol: number) {
  currentInputVolume = Math.max(0, Math.min(100, vol))
  if (micSourceStream) {
    micSourceStream.getAudioTracks().forEach(t => {
      try { (t as any).applyConstraints?.({ volume: currentInputVolume / 100 }) } catch {}
    })
  }
  emit()
}

export function setOutputVolume(vol: number) {
  currentOutputVolume = Math.max(0, Math.min(100, vol))
  applyOutputVolume()
  emit()
}

export async function listAudioDevices(): Promise<{ inputs: MediaDeviceInfo[]; outputs: MediaDeviceInfo[] }> {
  try {
    const devices = await navigator.mediaDevices.enumerateDevices()
    return {
      inputs: devices.filter(d => d.kind === 'audioinput'),
      outputs: devices.filter(d => d.kind === 'audiooutput'),
    }
  } catch {
    return { inputs: [], outputs: [] }
  }
}

export async function setMicDevice(deviceId: string) {
  currentMicDeviceId = deviceId
  if (currentMicEnabled) {
    try {
      await disableMicrophone()
      await enableMicrophone(currentNoiseSuppression)
    } catch (e) { console.error('[Mic] 切换设备失败', e) }
  }
  emit()
}

export function setSpeakerDevice(deviceId: string) {
  currentSpeakerDeviceId = deviceId
  remoteAudioElements.forEach(el => {
    try { (el as any).setSinkId?.(deviceId) } catch {}
  })
  emit()
}

export async function setPushToTalk(enabled: boolean, key: string = 'Space') {
  currentPushToTalk = enabled
  currentPushToTalkKey = key
  if (pttHandler) {
    window.removeEventListener('keydown', pttHandler as any)
    window.removeEventListener('keyup', pttHandler as any)
    pttHandler = null
  }
  pttKeyDown = false
  if (enabled) {
    if (currentMicEnabled) {
      try { await disableMicrophone() } catch {}
    }
    pttHandler = async (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || (t as any).isContentEditable)) return
      if (e.code !== key) return
      if (e.type === 'keydown' && !pttKeyDown) {
        pttKeyDown = true
        if (!currentMicEnabled && currentRoom) {
          try { await enableMicrophone(currentNoiseSuppression) } catch (err) { console.warn('[PTT]', err) }
        }
      } else if (e.type === 'keyup' && pttKeyDown) {
        pttKeyDown = false
        if (currentMicEnabled) {
          try { await disableMicrophone() } catch {}
        }
      }
    }
    window.addEventListener('keydown', pttHandler as any)
    window.addEventListener('keyup', pttHandler as any)
  }
  emit()
}

function startBitrateSampling() {
  stopBitrateSampling()
  lastAudioBytes = 0
  lastAudioSampleTime = 0
  bitrateTimer = setInterval(async () => {
    if (!currentRoom) return
    try {
      const pubs = currentRoom.localParticipant.audioTrackPublications
      let track: any = null
      for (const pub of pubs.values()) {
        if (pub.source === Track.Source.Microphone && pub.track) { track = pub.track; break }
      }
      if (!track || !track.getRTCStatsReport) { currentAudioBitrate = 0; emit(); return }
      const stats = await track.getRTCStatsReport()
      if (!stats) { currentAudioBitrate = 0; emit(); return }
      let bytes = 0
      stats.forEach((s: any) => {
        if (s.type === 'outbound-rtp' && (s.kind === 'audio' || s.mediaType === 'audio')) {
          if (typeof s.bytesSent === 'number') bytes = s.bytesSent
        }
      })
      const now = Date.now()
      if (lastAudioBytes > 0 && lastAudioSampleTime > 0) {
        const bps = ((bytes - lastAudioBytes) * 8 * 1000) / (now - lastAudioSampleTime)
        currentAudioBitrate = Math.max(0, Math.round(bps / 1000))
      }
      lastAudioBytes = bytes
      lastAudioSampleTime = now
      emit()
    } catch {}
  }, 2000)
}

function stopBitrateSampling() {
  if (bitrateTimer) { clearInterval(bitrateTimer); bitrateTimer = null }
  currentAudioBitrate = 0
  lastAudioBytes = 0
  lastAudioSampleTime = 0
}`
if (!s.includes(tail)) { console.error('FAIL: tail'); process.exit(1) }
s = s.replace(tail, newTail)

fs.writeFileSync('src/lib/livekit.ts', s)
console.log('PATCHED livekit.ts')

// ============ 2. useLiveKitRoom.ts INITIAL ============
let h = fs.readFileSync('src/hooks/useLiveKitRoom.ts', 'utf8')
const oldInit = `const INITIAL: MediaState = {
  connected: false,
  channelId: null,
  participants: [],
  screenShares: [],
  localScreenSharing: false,
  localMicEnabled: false,
  localNoiseSuppression: true,
  noiseMode: 'off',
  currentPresetId: '1080p60',
  e2eeEnabled: false,
}`
const newInit = `const INITIAL: MediaState = {
  connected: false,
  channelId: null,
  participants: [],
  screenShares: [],
  localScreenSharing: false,
  localMicEnabled: false,
  localNoiseSuppression: true,
  noiseMode: 'off',
  currentPresetId: '1080p60',
  e2eeEnabled: false,
  deafened: false,
  inputVolume: 100,
  outputVolume: 100,
  micDeviceId: null,
  speakerDeviceId: null,
  pushToTalk: false,
  pushToTalkKey: 'Space',
  audioBitrate: 0,
}`
if (!h.includes(oldInit)) { console.error('FAIL: INITIAL'); process.exit(1) }
h = h.replace(oldInit, newInit)
fs.writeFileSync('src/hooks/useLiveKitRoom.ts', h)
console.log('PATCHED useLiveKitRoom.ts')

// ============ 3. i18n keys ============
let en = fs.readFileSync('src/locales/en.ts', 'utf8')
const enAnchor = `  common: { save: 'Save', cancel: 'Cancel', close: 'Close', loading: 'Loading...' },`
const enAdd = `  audio: {
    title: 'Audio Settings',
    deafen: 'Deafen', undeafen: 'Undeafen',
    inputDevice: 'Input device', outputDevice: 'Output device', defaultDevice: 'Default',
    inputVolume: 'Input volume', outputVolume: 'Output volume',
    pushToTalk: 'Push to talk', pushToTalkKey: 'Key',
    unsupportedSpeaker: 'Speaker selection not supported on this browser',
  },
  menu: {
    sendMessage: 'Send message',
    viewProfile: 'View profile',
    addFriend: 'Add friend',
    userVolume: 'User volume',
    muteUser: 'Mute user',
  },
${enAnchor}`
if (!en.includes(enAnchor)) { console.error('FAIL: en anchor'); process.exit(1) }
en = en.replace(enAnchor, enAdd)
fs.writeFileSync('src/locales/en.ts', en)
console.log('PATCHED en.ts')

let zh = fs.readFileSync('src/locales/zh.ts', 'utf8')
const zhAnchor = `  common: { save: '保存', cancel: '取消', close: '关闭', loading: '加载中...' },`
const zhAdd = `  audio: {
    title: '音频设置',
    deafen: '拒听', undeafen: '取消拒听',
    inputDevice: '输入设备', outputDevice: '输出设备', defaultDevice: '默认设备',
    inputVolume: '输入音量', outputVolume: '输出音量',
    pushToTalk: '按键说话', pushToTalkKey: '按键',
    unsupportedSpeaker: '当前浏览器不支持扬声器选择',
  },
  menu: {
    sendMessage: '发送消息',
    viewProfile: '查看资料',
    addFriend: '添加好友',
    userVolume: '用户音量',
    muteUser: '静音该用户',
  },
${zhAnchor}`
if (!zh.includes(zhAnchor)) { console.error('FAIL: zh anchor'); process.exit(1) }
zh = zh.replace(zhAnchor, zhAdd)
fs.writeFileSync('src/locales/zh.ts', zh)
console.log('PATCHED zh.ts')

console.log('DONE Round 1')
