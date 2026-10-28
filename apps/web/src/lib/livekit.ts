import { Room, RoomEvent, RemoteParticipant, RemoteTrackPublication, RemoteTrack, Track, ConnectionState, ExternalE2EEKeyProvider, LocalTrack, VideoPreset } from 'livekit-client'
import { findPreset } from './videoPresets'
import { startRnnoiseProcessing, isRnnoiseSupported, type RnnoiseHandle } from './rnnoise'

/**
 * LiveKit Room 单例（AES-256 E2EE + simulcast + RNNoise + mic）。
 */

export type NoiseMode = 'rnnoise' | 'off' | 'unsupported'

export interface MediaState {
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
}

export interface ParticipantInfo {
  identity: string
  isLocal: boolean
  isSpeaking: boolean
  isMicrophoneEnabled: boolean
  isCameraEnabled: boolean
}

export interface ScreenShareInfo {
  participantIdentity: string
  trackSid: string
  track: RemoteTrack | LocalTrack
  element: HTMLVideoElement
  hasAudio: boolean
  /** 接收端画质偏好（仅远端共享生效；本地预览忽略） */
  quality?: 'low' | 'medium' | 'high'
}

interface ConnectOptions {
  wsUrl: string
  token: string
  channelId: string
  e2eeKeyB64: string | null
  /** 测试用：禁用 LiveKit adaptive stream 拥塞降级 */
  disableAdaptive?: boolean
}

let currentRoom: Room | null = null
let currentChannelId: string | null = null
let currentE2EEEnabled = false
let currentPresetId = '1080p60'
let currentMicEnabled = false
let currentNoiseSuppression = true
let currentNoiseMode: NoiseMode = 'off'
let micSourceStream: MediaStream | null = null
let rnnoiseHandle: RnnoiseHandle | null = null
let listeners: Set<(state: MediaState) => void> = new Set()

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
const remoteAudioElements = new Map<string, HTMLAudioElement>()

function emit() {
  const state = getState()
  for (const fn of listeners) fn(state)
}

function getState(): MediaState {
  if (!currentRoom) {
    return {
      connected: false,
      channelId: null,
      participants: [],
      screenShares: [],
      localScreenSharing: false,
      localMicEnabled: false,
      localNoiseSuppression: true,
      noiseMode: 'off' as NoiseMode,
      currentPresetId,
      e2eeEnabled: false,
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
  const room = currentRoom
  const e2eeEnabled =
    (room as any).e2eeManager?.isE2EEEnabled === true ||
    (room as any).isE2EEEnabled === true ||
    currentE2EEEnabled

  return {
    connected: room.state === ConnectionState.Connected,
    channelId: currentChannelId,
    participants: getAllParticipants(room),
    screenShares: getScreenShares(),
    localScreenSharing: room.localParticipant.isScreenShareEnabled,
    localMicEnabled: currentMicEnabled,
    localNoiseSuppression: currentNoiseSuppression,
    noiseMode: currentNoiseMode,
    currentPresetId,
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
}

function getAllParticipants(room: Room): ParticipantInfo[] {
  const all: ParticipantInfo[] = []
  all.push({
    identity: room.localParticipant.identity,
    isLocal: true,
    isSpeaking: room.localParticipant.isSpeaking,
    isMicrophoneEnabled: room.localParticipant.isMicrophoneEnabled,
    isCameraEnabled: room.localParticipant.isCameraEnabled,
  })
  room.remoteParticipants.forEach((p) => {
    all.push({
      identity: p.identity,
      isLocal: false,
      isSpeaking: p.isSpeaking,
      isMicrophoneEnabled: p.isMicrophoneEnabled,
      isCameraEnabled: p.isCameraEnabled,
    })
  })
  return all
}

const screenShareCache = new Map<string, ScreenShareInfo>()
function getScreenShares(): ScreenShareInfo[] {
  return Array.from(screenShareCache.values())
}

function base64ToUint8(s: string): Uint8Array {
  let b64 = s.replace(/-/g, '+').replace(/_/g, '/')
  while (b64.length % 4 !== 0) b64 += '='
  const bin = atob(b64)
  const bytes = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i)
  return bytes
}

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const h = await crypto.subtle.digest('SHA-256', new Uint8Array(bytes))
  return Array.from(new Uint8Array(h)).map(b => b.toString(16).padStart(2, '0')).join('').slice(0, 16)
}

export async function connectMedia(opts: ConnectOptions): Promise<Room> {
  const { wsUrl, token, channelId, e2eeKeyB64 } = opts

  if (currentRoom && currentChannelId === channelId) return currentRoom
  if (currentRoom) await disconnectMedia()

  let keyProvider: ExternalE2EEKeyProvider | undefined
  let e2eeEnabled = false

  if (e2eeKeyB64) {
    try {
      const keyBytes = base64ToUint8(e2eeKeyB64)
      if (keyBytes.length !== 32) throw new Error(`E2EE key 长度错误: ${keyBytes.length}`)
      keyProvider = new ExternalE2EEKeyProvider({ keySize: 256 })
      await keyProvider.setKey(keyBytes.buffer as ArrayBuffer)
      e2eeEnabled = true
      console.log('[E2EE] AES-256 key 已注入')
      console.log('[E2EE] key hash (connectMedia):', await sha256Hex(keyBytes))
    } catch (e) {
      console.error('[E2EE] key 注入失败:', e)
    }
  }

  // 测试环境可通过 window 全局标记禁用 adaptiveStream，防止拥塞控制误降级分辨率
  const disableAdaptive =
    opts.disableAdaptive ||
    (typeof window !== 'undefined' && (window as any).__KAEDE_TEST_DISABLE_ADAPTIVE__ === true)

  const roomOptions: any = {
    adaptiveStream: !disableAdaptive,
    dynacast: !disableAdaptive,
  }
  if (keyProvider) {
    const worker = new Worker('/livekit-client.e2ee.worker.mjs', { type: 'module' })
    roomOptions.encryption = { keyProvider, worker }
  }

  const room = new Room(roomOptions)
  currentRoom = room
  currentChannelId = channelId
  currentE2EEEnabled = e2eeEnabled

  room
    .on(RoomEvent.Connected, () => emit())
    .on(RoomEvent.Disconnected, () => {
      currentRoom = null
      currentChannelId = null
      currentE2EEEnabled = false
      screenShareCache.clear()
      remoteAudioElements.clear()
      stopBitrateSampling()
      emit()
    })
    .on(RoomEvent.ParticipantConnected, () => emit())
    .on(RoomEvent.ParticipantDisconnected, (p: RemoteParticipant) => {
      screenShareCache.delete(p.identity)
      emit()
    })
    .on(RoomEvent.TrackSubscribed, (track: RemoteTrack, pub: RemoteTrackPublication, p: RemoteParticipant) => {
      // 1. 处理所有音频轨道（麦克风、屏幕共享音频等）
      if (track.kind === 'audio') {
        const el = track.attach() as HTMLAudioElement
        el.autoplay = true
        el.style.display = 'none'
        document.body.appendChild(el)
        el.play().catch(e => console.warn('[Audio] 播放失败:', e))
        remoteAudioElements.set(track.sid || p.identity, el)
        // apply deafen/volume immediately if set
        try { (track as any).setVolume?.(currentDeafened ? 0 : currentOutputVolume / 100) } catch {}
        console.log(`[Audio] 已挂载并播放 ${p.identity} 的音频轨道 (${pub.source})`)

        // 屏幕共享音频：标记对应 share 有音轨
        if (pub.source === Track.Source.ScreenShareAudio) {
          for (const [sid, info] of screenShareCache) {
            if (info.participantIdentity === p.identity) {
              screenShareCache.set(sid, { ...info, hasAudio: true })
              break
            }
          }
          emit()
        }
        return // 音频处理完就返回，不再走视频逻辑
      }

      // 2. 处理屏幕共享视频
      if (pub.source === Track.Source.ScreenShare) {
        // 接收端画质偏好：从缓存读（用户可能切换过），默认 high
        const cachedQuality = screenShareCache.get(track.sid || '')?.quality || 'high'
        try {
          // @ts-ignore - setVideoQuality 在 2.x 仍可用
          pub.setVideoQuality?.(cachedQuality)
          // 默认行为：high 档不限制分辨率；其他档位让 LiveKit 自动按层
          if (cachedQuality === 'high') {
            ;(pub as any).setVideoDimensions?.(99999, 99999)
          }
        } catch (e) {
          console.warn('[ScreenShare] setVideoQuality 失败:', e)
        }
        const el = track.attach() as HTMLVideoElement
        el.autoplay = true
        el.playsInline = true
        el.muted = true
        el.play().catch(() => {})
        screenShareCache.set(track.sid || p.identity, {
          participantIdentity: p.identity,
          trackSid: track.sid || '',
          track,
          element: el,
          hasAudio: false,
          quality: cachedQuality,  // 记录当前接收画质
        })
        emit()
      }
    })
    .on(RoomEvent.TrackUnsubscribed, (track: RemoteTrack, pub: RemoteTrackPublication, p: RemoteParticipant) => {
      if (track.kind === 'audio') {
        remoteAudioElements.delete(track.sid || p.identity)
        track.detach().forEach(el => el.remove())
        return
      }
      if (pub.source === Track.Source.ScreenShare) {
        screenShareCache.delete(track.sid || p.identity)
        emit()
      }
    })
    .on(RoomEvent.LocalTrackPublished, () => emit())
    .on(RoomEvent.LocalTrackUnpublished, () => emit())
    .on(RoomEvent.ActiveSpeakersChanged, () => emit())
    // 远端 track 发布/取消/静音/取消静音：必须 emit，否则远端麦克风/摄像头状态不会更新
    .on(RoomEvent.TrackPublished, () => emit())
    .on(RoomEvent.TrackUnpublished, () => emit())
    .on(RoomEvent.TrackMuted, () => emit())
    .on(RoomEvent.TrackUnmuted, () => emit())
    .on(RoomEvent.ParticipantMetadataChanged, () => emit())

  await room.connect(wsUrl, token)

  if (keyProvider) {
    try { await room.setE2EEEnabled(true) } catch (e) { console.error('[E2EE]', e) }
  }

  startBitrateSampling()
  emit()
  return room
}

export async function disconnectMedia() {
  if (!currentRoom) return
  // 清理 RNNoise
  await disableMicrophone()
  await currentRoom.disconnect()
  currentRoom = null
  currentChannelId = null
  currentE2EEEnabled = false
  screenShareCache.clear()
  remoteAudioElements.clear()
  stopBitrateSampling()
  emit()
}

/**
 * 开启屏幕共享（simulcast + 用户选定档位）。
 */
let isStartingShare = false
export async function startScreenShare(presetId: string): Promise<boolean> {
  if (!currentRoom) throw new Error('未加入媒体房间')
  if (isStartingShare) return false
  isStartingShare = true
  const preset = findPreset(presetId)
  currentPresetId = presetId
  
  // 自定义 constraints 精确控制分辨率
  const constraints: DisplayMediaStreamOptions = {
    video: {
      width: { ideal: preset.width, max: preset.width },
      height: { ideal: preset.height, max: preset.height },
      frameRate: { ideal: preset.fps, max: preset.fps },
    },
    audio: true, // 捕获系统/标签页音频
  }
  
  try {
    const stream = await navigator.mediaDevices.getDisplayMedia(constraints)
    const videoTrack = stream.getVideoTracks()[0]
    if (!videoTrack) throw new Error('无视频轨道')
    
    // 开启 simulcast，让接收端能切到不同分辨率层（高/中/低）
    // 注意：屏幕共享单层的话，接收端调画质会导致 SFU 强制降码率 → 画面糊。
    // 必须发 simulcast 层，SVC/Simulcast 切层才能做到真正的"分辨率"变化。
    const videoPub = await currentRoom.localParticipant.publishTrack(videoTrack, {
      source: Track.Source.ScreenShare,
      simulcast: true,
      screenShareEncoding: { maxBitrate: preset.bitrate, maxFramerate: preset.fps },
      videoSimulcastLayers: [
        new VideoPreset(3840, 2160, 15_000_000, preset.fps),
        new VideoPreset(1920, 1080, 5_000_000, Math.min(preset.fps, 30)),
        new VideoPreset(1280, 720, 1_500_000, Math.min(preset.fps, 30)),
        new VideoPreset(854, 480, 500_000, 30),
        new VideoPreset(640, 360, 200_000, 15),
      ],
      degradationPreference: 'maintain-resolution',
    })

    // 系统/标签页音频（用户勾选"共享音频"时才有）—— 独立发布
    const audioTracks = stream.getAudioTracks()
    console.log('[ScreenShare] audio track present=' + audioTracks.length + ' (constraints.audio=' + JSON.stringify(constraints.audio) + ')')
    const audioTrack = audioTracks[0]
    if (audioTrack) {
      try {
        await currentRoom.localParticipant.publishTrack(audioTrack, {
          source: Track.Source.ScreenShareAudio,
          simulcast: false,
        })
        console.log('[ScreenShare] 已发布系统音频')
        // 更新本地预览 hasAudio
        const localKey = 'LOCAL-SHARE-' + (videoTrack.id || 'local')
        const info = screenShareCache.get(localKey)
        if (info) screenShareCache.set(localKey, { ...info, hasAudio: true })
        emit()
      } catch (e) {
        console.warn('[ScreenShare] 音频发布失败:', e)
      }
    } else {
      console.log('[ScreenShare] 无系统音频轨道（用户未勾选/浏览器未提供）')
    }

    // 本地预览
    try {
      const localTrack = videoPub.track
      if (localTrack) {
        const localEl = document.createElement('video')
        localEl.autoplay = true
        localEl.muted = true
        localEl.playsInline = true
        localTrack.attach(localEl)
        localEl.play().catch(() => {})
        const sid = (localTrack as any).sid || (localTrack as any).mediaStreamTrack?.id || 'local'
        const localKey = 'LOCAL-SHARE-' + sid
        screenShareCache.set(localKey, {
          participantIdentity: currentRoom.localParticipant.identity + ' (me)',
          trackSid: localKey,
          track: localTrack,
          element: localEl,
          hasAudio: audioTracks.length > 0,
        })
        emit()
      }
    } catch (e) {
      console.warn('[ScreenShare] 本地预览注入失败:', e)
    }
    
    // 监听用户点系统停止共享
    videoTrack.onended = () => {
      stopScreenShare().catch(console.error)
    }
    
    console.log(`[ScreenShare] 已启动: ${preset.label} (${preset.bitrate/1e6} Mbps)`)
    emit()
    setTimeout(() => { isStartingShare = false }, 1000)
    return true
  } catch (e: any) {
    setTimeout(() => { isStartingShare = false }, 1000)
    console.error('屏幕共享启动失败:', e)
    // 若 getDisplayMedia 被用户取消或浏览器不支持 4K，降级重试
    if (preset.width > 1920) {
      console.warn('[ScreenShare] 4K 失败，降级到 1080p60 重试')
      return startScreenShare('1080p60')
    }
    throw e
  }
}

export async function stopScreenShare(): Promise<void> {
  if (!currentRoom) return
  // 清理本地预览
  for (const key of Array.from(screenShareCache.keys())) {
    if (key.startsWith('LOCAL-SHARE-')) {
      const info = screenShareCache.get(key)
      try { info?.track.detach().forEach(el => el.remove()) } catch {}
      screenShareCache.delete(key)
    }
  }
  for (const pub of currentRoom.localParticipant.videoTrackPublications.values()) {
    if (pub.source === Track.Source.ScreenShare) {
      await currentRoom.localParticipant.unpublishTrack(pub.track!)
    }
  }
  for (const pub of currentRoom.localParticipant.audioTrackPublications.values()) {
    if (pub.source === Track.Source.ScreenShareAudio) {
      await currentRoom.localParticipant.unpublishTrack(pub.track!)
    }
  }
  emit()
}

/**
 * 开启麦克风（可选 RNNoise 降噪）。
 */
export async function enableMicrophone(noiseSuppression: boolean): Promise<void> {
  if (!currentRoom) throw new Error('未加入媒体房间')

  const disableEcho = typeof window !== 'undefined' && (window as any).__KAEDE_TEST_DISABLE_ECHO__ === true

  // 关键：始终关闭浏览器原生 noiseSuppression。
  // 走 RNNoise 时，原生 NS 会和它打架导致音质二次失真；不走 RNNoise 时也不能用原生 NS 兜底
  // （用户明确要求"必须纯 RNNoise"）。AEC/AGC 保留——它们是不同功能，不冲突。
  micSourceStream = await navigator.mediaDevices.getUserMedia({
    audio: {
      ...(currentMicDeviceId ? { deviceId: { exact: currentMicDeviceId } } : {}),
      echoCancellation: !disableEcho,
      noiseSuppression: false,
      autoGainControl: true,
      sampleRate: 48000,
    },
  })

  let publishTrack = micSourceStream.getAudioTracks()[0]
  let mode: NoiseMode = 'off'

  if (noiseSuppression) {
    if (!isRnnoiseSupported()) {
      console.warn('[Mic] RNNoise 需要 MediaStreamTrackProcessor；当前浏览器不支持（Firefox 需 flag）。降噪已关闭，原生 NS 不会启用。')
      mode = 'unsupported'
    } else {
      const handle = await startRnnoiseProcessing(publishTrack)
      if (handle) {
        rnnoiseHandle = handle
        publishTrack = handle.track
        mode = 'rnnoise'
        console.log('[Mic] RNNoise 已启用（原生 NS 已关闭）')
      } else {
        console.warn('[Mic] RNNoise 启动失败，降噪关闭（原生 NS 不会启用）')
        mode = 'unsupported'
      }
    }
  }

  await currentRoom.localParticipant.publishTrack(publishTrack, {
    source: Track.Source.Microphone,
    simulcast: false,
    dtx: false, // 【修复】禁用 DTX，防止降噪后音质断裂
  })

  currentMicEnabled = true
  currentNoiseSuppression = noiseSuppression
  currentNoiseMode = mode
  console.log(`[Mic] 已开启，mode=${mode}`)
  emit()
}

export async function disableMicrophone(): Promise<void> {
  if (!currentRoom) return
  for (const pub of currentRoom.localParticipant.audioTrackPublications.values()) {
    if (pub.source === Track.Source.Microphone) {
      await currentRoom.localParticipant.unpublishTrack(pub.track!)
    }
  }
  if (rnnoiseHandle) {
    try { await rnnoiseHandle.stop() } catch {}
    rnnoiseHandle = null
  }
  if (micSourceStream) {
    micSourceStream.getTracks().forEach(t => t.stop())
    micSourceStream = null
  }
  currentMicEnabled = false
  currentNoiseMode = 'off'
  emit()
}

export function setNoiseSuppression(enabled: boolean) {
  currentNoiseSuppression = enabled
  emit()
}

export async function updateE2EEKey(newKeyB64: string): Promise<void> {
  if (!currentRoom) return
  const e2eeManager = (currentRoom as any).e2eeManager
  if (!e2eeManager) {
    console.warn('[E2EE] 房间未启用 E2EE，跳过 key 更新')
    return
  }
  try {
    const keyBytes = base64ToUint8(newKeyB64)
    const keyProvider = e2eeManager.keyProvider
    if (keyProvider?.setKey) {
      await keyProvider.setKey(keyBytes.buffer as ArrayBuffer)
      console.log('[E2EE] key hash (update):', await sha256Hex(keyBytes))
      console.log('[E2EE] key 已轮换')
      emit()
    }
  } catch (e) {
    console.error('[E2EE] key 轮换失败:', e)
  }
}

/**
 * 接收端设置某路屏幕共享的画质偏好。
 * - 仅远端共享生效；本地预览（LOCAL-SHARE-）直接 no-op
 * - 依赖 LiveKit simulcast：发布端 startScreenShare 已开启 simulcast
 */
export function setScreenShareQuality(sid: string, presetId: string) {
  if (!currentRoom) return
  if (sid.startsWith('LOCAL-SHARE-')) {
    console.log('[ScreenShare] 本地预览不调 setVideoQuality:', sid)
    return
  }
  const info = screenShareCache.get(sid)
  if (!info) {
    console.warn('[ScreenShare] setScreenShareQuality: sid not found', sid)
    return
  }
  let applied = false
  for (const participant of currentRoom.remoteParticipants.values()) {
    const pub = participant.getTrackPublicationBySid(sid)
    if (pub && pub.source === Track.Source.ScreenShare) {
      try {
        // 动态选择层级：LiveKit 只支持 high/medium/low
        let layer: any = 'high'
        if (presetId.includes('4k') || presetId.includes('1440p') || presetId.includes('1080p')) {
          layer = 'high'
        } else if (presetId.includes('720p')) {
          layer = 'medium'
        } else if (presetId.includes('480p') || presetId.includes('360p')) {
          layer = 'low'
        }
        
        // 同时调用 dimensions 强制让 SFU 降分辨率
        const dim = presetId.match(/(\d+)p/)
        if (dim) {
          const h = parseInt(dim[1], 10)
          const w = Math.round(h * 16 / 9)
          ;(pub as any).setVideoDimensions?.(w, h)
        }

        pub.setVideoQuality?.(layer)
        console.log(`[ScreenShare] ${sid} -> ${presetId} (layer=${layer})`)
        applied = true
      } catch (e) {
        console.warn('[ScreenShare] setVideoQuality failed:', e)
      }
      break
    }
  }
  if (!applied) {
    console.warn('[ScreenShare] setScreenShareQuality: publication not found for', sid)
  }
  
  // 记住用户选择（供下次初始订阅使用）
  screenShareCache.set(sid, { ...info, quality: presetId as any })
  emit()
}

export function subscribeMedia(fn: (state: MediaState) => void): () => void {
  listeners.add(fn)
  fn(getState())
  return () => listeners.delete(fn)
}

export function getCurrentRoom(): Room | null {
  return currentRoom
}

// 兼容旧 API
export async function toggleScreenShare(enable: boolean, presetId = '1080p60'): Promise<boolean> {
  if (enable) {
    return startScreenShare(presetId)
  } else {
    await stopScreenShare()
    return false
  }
}

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
}


