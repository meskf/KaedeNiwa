import { Room, RoomEvent, RemoteParticipant, RemoteTrackPublication, RemoteTrack, Track, ConnectionState, ExternalE2EEKeyProvider, LocalTrack } from 'livekit-client'
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
        screenShareCache.set(track.sid || p.identity, {
          participantIdentity: p.identity,
          trackSid: track.sid || '',
          track,
          element: el,
          hasAudio: false,
        })
        emit()
      }
    })
    .on(RoomEvent.TrackUnsubscribed, (track: RemoteTrack, pub: RemoteTrackPublication, p: RemoteParticipant) => {
      if (track.kind === 'audio') {
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
    
    // 用 publishTrack 手动控制 simulcast
    const videoPub = await currentRoom.localParticipant.publishTrack(videoTrack, {
      source: Track.Source.ScreenShare,
      simulcast: false,
      screenShareEncoding: { maxBitrate: preset.bitrate, maxFramerate: preset.fps },
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


