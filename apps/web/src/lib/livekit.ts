import {
  Room,
  RoomEvent,
  RemoteParticipant,
  RemoteTrackPublication,
  RemoteTrack,
  Track,

  ConnectionState,
  ExternalE2EEKeyProvider,

} from 'livekit-client'
import { findPreset } from './videoPresets'

/**
 * LiveKit Room 单例（AES-256 E2EE + simulcast + RNNoise + mic）。
 */

export interface MediaState {
  connected: boolean
  channelId: string | null
  participants: ParticipantInfo[]
  screenShares: ScreenShareInfo[]
  localScreenSharing: boolean
  localMicEnabled: boolean
  localNoiseSuppression: boolean
  currentPresetId: string
  e2eeEnabled: boolean
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
  element: HTMLVideoElement
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
let micSourceStream: MediaStream | null = null
let listeners: Set<(state: MediaState) => void> = new Set()

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
      currentPresetId,
      e2eeEnabled: false,
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
    currentPresetId,
    e2eeEnabled,
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
      emit()
    })
    .on(RoomEvent.ParticipantConnected, () => emit())
    .on(RoomEvent.ParticipantDisconnected, (p: RemoteParticipant) => {
      screenShareCache.delete(p.identity)
      emit()
    })
    .on(RoomEvent.TrackSubscribed, (track: RemoteTrack, pub: RemoteTrackPublication, p: RemoteParticipant) => {
      if (pub.source === Track.Source.ScreenShare) {
        // 强制订阅最高档，避免默认订阅 mid/low 层导致分辨率减半
        try {
          // @ts-ignore - setVideoQuality 在 2.x 仍可用
          pub.setVideoQuality?.('high')
          // 新 API（如果存在）
          ;(pub as any).setVideoDimensions?.(99999, 99999)
        } catch (e) {
          console.warn('[ScreenShare] setVideoQuality 失败:', e)
        }
        const el = track.attach() as HTMLVideoElement
        el.autoplay = true
        el.playsInline = true
        el.muted = true
        el.play().catch(() => {})
        screenShareCache.set(p.identity, {
          participantIdentity: p.identity,
          trackSid: track.sid || '',
          element: el,
        })
        emit()
      }
    })
    .on(RoomEvent.TrackUnsubscribed, (_t: RemoteTrack, pub: RemoteTrackPublication, p: RemoteParticipant) => {
      if (pub.source === Track.Source.ScreenShare) {
        screenShareCache.delete(p.identity)
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
  emit()
}

/**
 * 开启屏幕共享（simulcast + 用户选定档位）。
 */
export async function startScreenShare(presetId: string): Promise<boolean> {
  if (!currentRoom) throw new Error('未加入媒体房间')
  const preset = findPreset(presetId)
  currentPresetId = presetId
  
  // 自定义 constraints 精确控制分辨率
  const constraints: DisplayMediaStreamOptions = {
    video: {
      width: { ideal: preset.width, max: preset.width },
      height: { ideal: preset.height, max: preset.height },
      frameRate: { ideal: preset.fps, max: preset.fps },
    },
    audio: false,
  }
  
  try {
    const stream = await navigator.mediaDevices.getDisplayMedia(constraints)
    const videoTrack = stream.getVideoTracks()[0]
    if (!videoTrack) throw new Error('无视频轨道')
    
    // 用 publishTrack 手动控制 simulcast
    await currentRoom.localParticipant.publishTrack(videoTrack, {
      source: Track.Source.ScreenShare,
      simulcast: false,
      screenShareEncoding: { maxBitrate: preset.bitrate, maxFramerate: preset.fps },
      degradationPreference: 'maintain-resolution',
    })
    
    // 监听用户点系统停止共享
    videoTrack.onended = () => {
      stopScreenShare().catch(console.error)
    }
    
    console.log(`[ScreenShare] 已启动: ${preset.label} (${preset.bitrate/1e6} Mbps)`)
    emit()
    return true
  } catch (e: any) {
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
  for (const pub of currentRoom.localParticipant.videoTrackPublications.values()) {
    if (pub.source === Track.Source.ScreenShare) {
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

  // 麦克风：浏览器原生降噪（RNNoise AudioWorklet 集成放下轮）
  micSourceStream = await navigator.mediaDevices.getUserMedia({
    audio: {
      echoCancellation: true,
      noiseSuppression: noiseSuppression,
      autoGainControl: true,
      sampleRate: 48000,
    },
  })

  const track = micSourceStream.getAudioTracks()[0]
  await currentRoom.localParticipant.publishTrack(track, {
    source: Track.Source.Microphone,
    simulcast: false,
  })

  currentMicEnabled = true
  currentNoiseSuppression = noiseSuppression
  console.log(`[Mic] 已开启，noiseSuppression=${noiseSuppression}`)
  emit()
}

export async function disableMicrophone(): Promise<void> {
  if (!currentRoom) return
  for (const pub of currentRoom.localParticipant.audioTrackPublications.values()) {
    if (pub.source === Track.Source.Microphone) {
      await currentRoom.localParticipant.unpublishTrack(pub.track!)
    }
  }
  if (micSourceStream) {
    micSourceStream.getTracks().forEach(t => t.stop())
    micSourceStream = null
  }
  currentMicEnabled = false
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
      console.log('[E2EE] key 已轮换')
      emit()
    }
  } catch (e) {
    console.error('[E2EE] key 轮换失败:', e)
  }
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


