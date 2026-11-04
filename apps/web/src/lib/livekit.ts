import { setLogLevel as setLiveKitLogLevel, Room, RoomEvent, RemoteParticipant, RemoteTrackPublication, RemoteTrack, Track, ConnectionState, ExternalE2EEKeyProvider, LocalTrack, VideoQuality } from 'livekit-client'

type QualityStr = 'low' | 'medium' | 'high'
setLiveKitLogLevel('debug')
function toVideoQuality(q: QualityStr): VideoQuality {
  return q === 'high' ? VideoQuality.HIGH : q === 'low' ? VideoQuality.LOW : VideoQuality.MEDIUM
}
import { findPreset, type VideoPreset } from './videoPresets'
import {
  startMediaHealth, stopMediaHealth, subscribeMediaHealth, setMediaHealthTargetFps,
  setMediaHealthContext,
  type MediaHealth, type HealthReason,
} from './mediaHealth'
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
  /** 链路健康度（UDP/TCP、丢包、实际接收帧率）；无媒体会话时为 null */
  link: LinkInfo | null
  /** 自动降档原因（如“编码器 CPU 受限，已降分辨率、帧率保持不变”）；未降档为 null */
  autoQualityNote: string | null
}

export interface LinkInfo {
  severity: 'ok' | 'warn' | 'bad'
  transport: MediaHealth['transport']
  rttMs: number
  inFps: number
  lossPct: number
  /** 接收端帧间隔标准差（ms）：越大越“不跟手” */
  pacingStdMs: number
  /** 接收端连续“有码率但无画面”的秒数（关键帧丢失/卡死） */
  stalledSec: number
  reasons: HealthReason[]
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
  presetId?: string
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

// 链路健康度 / 自适应降档状态
let currentLink: LinkInfo | null = null
let currentAutoQualityNote: string | null = null
let unsubHealth: (() => void) | null = null
let activeShareTrack: MediaStreamTrack | null = null
/** 底层 RTCRtpSender：用于上行丢包时动态下调 maxBitrate（LiveKit 发布参数只在发布时生效） */
let activeShareSender: RTCRtpSender | null = null
let activePreset: VideoPreset | null = null
let cpuStreak = 0

function emit() {
  const state = getState()
  for (const fn of listeners) fn(state)
}

// ---- 屏幕共享自适应降档 ----
//
// 触发条件：encoder 自报 qualityLimitationReason === 'cpu'，或编码器连续
// 2 秒完全不出帧（stalledSec >= 2）。
//
// 为什么不用“帧率不达标”：静态文档共享时 Chrome 本来就只在画面变化时出帧，
// framesEncoded 很低但观看完全正常（用户实测“读文档还好”）。而游戏/滚动这类
// 高动态画面会把编码器压满，Chrome 报 cpu → 实际丢帧、画面断续。
//
// 稳帧优先：前两档只降分辨率、不动帧率；后两档才允许帧率减半。
// 帧率一旦被砍（30→22→15），帧间隔反而更不匀，肉眼就是“一卡一卡”。
// 但编码器完全停摆（framesEncoded≈0）时，继续保帧率只会一帧都发不出，
// 所以最后两档允许 60→30 的回落。
const ADAPT_STEPS: Array<{ scale: number; fpsRatio: number }> = [
  { scale: 0.85, fpsRatio: 1 },
  { scale: 0.7, fpsRatio: 1 },
  { scale: 0.55, fpsRatio: 0.5 },
  { scale: 0.45, fpsRatio: 0.5 },
]
let adaptiveStep = -1

function computeAdaptiveTarget() {
  const preset = activePreset
  if (!preset) return null
  const step = ADAPT_STEPS[Math.max(0, adaptiveStep)]
  // 两档之内保持档位原帧率：宁可糊一点，也要节奏稳
  const fps = Math.max(15, Math.round(preset.fps * step.fpsRatio))
  const scale = step.scale
  const w = Math.max(640, Math.round((preset.width * scale) / 2) * 2)
  const h = Math.max(360, Math.round((preset.height * scale) / 2) * 2)
  return { fps, w, h }
}

async function applyAdaptiveDowngrade() {
  if (!activeShareTrack || !activePreset) return
  if (adaptiveStep >= ADAPT_STEPS.length - 1) return
  adaptiveStep++
  const target = computeAdaptiveTarget()
  if (!target) return
  try {
    await activeShareTrack.applyConstraints({
      width: { max: target.w },
      height: { max: target.h },
      frameRate: { ideal: target.fps, max: target.fps },
    })
    setMediaHealthTargetFps(target.fps)
    currentAutoQualityNote =
      `检测到编码器压力过高，已自动降到 ${target.w}x${target.h}（帧率保持 ${target.fps}fps，稳帧优先）`
    console.warn('[ScreenShare] ' + currentAutoQualityNote)
  } catch (e) {
    console.warn('[ScreenShare] 自适应降档失败:', e)
  }
  emit()
}

function onHealth(h: MediaHealth) {
  currentLink = {
    severity: h.severity,
    transport: h.transport,
    rttMs: h.rttMs,
    inFps: h.inbound?.fps ?? 0,
    lossPct: h.inbound?.lossPct ?? 0,
    pacingStdMs: h.inbound?.frameIntervalStdMs ?? 0,
    stalledSec: h.inbound?.stalledSec ?? 0,
    reasons: h.reasons,
  }
  if (activeShareTrack && activePreset) {
    // 上行丢包（SFU 反馈）：这才是“PC → 服务器”这一段，观众换成 4G 也照样卡。
    if ((h.outbound?.uplinkLossPct ?? 0) >= 4) void reduceSentBitrate(h.outbound?.uplinkLossPct ?? 0)

    // 两条触发线：encoder 自报 cpu 受限；或编码器连续 2 秒完全不出帧。
    // 后者比“cpu”更快，能在画面已经冻住时立即降档。
    const stalled = (h.outbound?.stalledSec ?? 0) >= 2
    const cpu = h.outbound?.limit === 'cpu'
    cpuStreak = stalled || cpu ? cpuStreak + 1 : 0
    const need = stalled ? 2 : 4
    if (cpuStreak >= need) {
      cpuStreak = 0
      if (adaptiveStep >= ADAPT_STEPS.length - 1) {
        currentAutoQualityNote =
          '编码器持续无输出：游戏可能占满 GPU/CPU。可尝试 ?codec=vp8 强制软件编码，或把游戏改为无边框窗口并限制帧率'
        emit()
      } else {
        void applyAdaptiveDowngrade()
      }
    }
  } else {
    cpuStreak = 0
    // 作为观众：连续 2 秒“有码率但解不出画面”= 关键帧丢失，重新订阅触发关键帧请求。
    if ((h.inbound?.stalledSec ?? 0) >= 2) void resubscribeRemoteShare()
  }
  emit()
}

let lastResubAt = 0
let lastBitrateCutAt = 0

/**
 * 上行丢包时只降编码码率上限，不动分辨率/帧率。
 * LiveKit 的 screenShareEncoding 只在发布时生效，这里直接改底层 sender 参数。
 */
async function reduceSentBitrate(lossPct: number) {
  if (!activeShareTrack) return
  const now = Date.now()
  if (now - lastBitrateCutAt < 10_000) return
  try {
    const sender = activeShareSender
    if (!sender) return
    const params = sender.getParameters()
    const enc = params.encodings?.[0]
    if (!enc || typeof enc.maxBitrate !== 'number') return
    const next = Math.max(400_000, Math.round(enc.maxBitrate * 0.65))
    if (next >= enc.maxBitrate) return
    lastBitrateCutAt = now
    enc.maxBitrate = next
    await sender.setParameters(params)
    currentAutoQualityNote = `上行丢包 ${lossPct.toFixed(1)}%，已自动降码率至 ${(next / 1e6).toFixed(1)} Mbps`
    console.warn(`[ScreenShare] ${currentAutoQualityNote}`)
    emit()
  } catch (e) {
    console.warn('[ScreenShare] 降低上行码率失败:', e)
  }
}
async function resubscribeRemoteShare() {
  if (!currentRoom) return
  const now = Date.now()
  // 重订阅会让画面闪一下，20 秒内最多做一次，避免抖动循环
  if (now - lastResubAt < 20_000) return
  for (const p of currentRoom.remoteParticipants.values()) {
    for (const pub of p.videoTrackPublications.values()) {
      if (pub.source !== Track.Source.ScreenShare) continue
      lastResubAt = now
      try {
        console.warn('[ScreenShare] 接收端连续 2 秒有码率无画面，重新订阅以请求关键帧')
        await pub.setSubscribed(false)
        setTimeout(() => {
          try { pub.setSubscribed(true) } catch (e) { console.warn('[ScreenShare] 重订阅恢复失败:', e) }
        }, 300)
        currentAutoQualityNote = '画面连续无帧，已重新订阅请求关键帧'
      } catch (e) {
        console.warn('[ScreenShare] 重新订阅失败:', e)
      }
      return
    }
  }
}

function resetAdaptive() {
  activeShareTrack = null
  activeShareSender = null
  activePreset = null
  adaptiveStep = -1
  cpuStreak = 0
  lastBitrateCutAt = 0
  currentAutoQualityNote = null
  setMediaHealthContext({ presetId: null })
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
      link: null,
      autoQualityNote: null,
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
    link: currentLink,
    autoQualityNote: currentAutoQualityNote,
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
      stopMediaHealth()
      unsubHealth?.()
      unsubHealth = null
      currentLink = null
      resetAdaptive()
      ;(window as any).__kaedeRoom = null
      emit()
    })
    .on(RoomEvent.ParticipantConnected, () => emit())
    .on(RoomEvent.ParticipantDisconnected, (p: RemoteParticipant) => {
      screenShareCache.delete(p.identity)
      emit()
    })
    .on(RoomEvent.ConnectionStateChanged, (state) => {
      console.log('[LiveKit Debug] 连接状态改变:', state)
      if (typeof window !== 'undefined' && window.innerWidth < 800) {
        const div = document.createElement('div')
        div.style.cssText = 'position:fixed;bottom:80px;left:10px;right:10px;background:#3b82f6;color:white;padding:10px;z-index:9999;border-radius:8px;font-size:14px;'
        div.innerText = '状态: ' + state
        document.body.appendChild(div)
        setTimeout(() => div.remove(), 5000)
      }
    })
    .on(RoomEvent.Disconnected, (reason) => {
      console.log('[LiveKit Debug] 已断开:', reason)
      if (typeof window !== 'undefined' && window.innerWidth < 800) {
        const div = document.createElement('div')
        div.style.cssText = 'position:fixed;bottom:80px;left:10px;right:10px;background:#ef4444;color:white;padding:10px;z-index:9999;border-radius:8px;font-size:14px;'
        div.innerText = '断开: ' + reason
        document.body.appendChild(div)
        setTimeout(() => div.remove(), 5000)
      }
    })
    .on(RoomEvent.TrackSubscriptionFailed, (_sid, _p, reason) => {
      console.log('[LiveKit Debug] 订阅失败:', reason)
      if (typeof window !== 'undefined' && window.innerWidth < 800) {
        const div = document.createElement('div')
        div.style.cssText = 'position:fixed;bottom:80px;left:10px;right:10px;background:#ef4444;color:white;padding:10px;z-index:9999;border-radius:8px;font-size:14px;'
        div.innerText = '订阅失败: ' + reason
        document.body.appendChild(div)
        setTimeout(() => div.remove(), 8000)
      }
    })
    .on(RoomEvent.TrackSubscribed, (track: RemoteTrack, pub: RemoteTrackPublication, p: RemoteParticipant) => {
      // 【调试】超级详细的弹窗
      const debugMsg = `收到轨道: ${track.kind} (${pub.source}) from ${p.identity} | Subscribed: ${pub.isSubscribed}`
      console.log('[LiveKit Debug]', debugMsg)
      if (typeof window !== 'undefined' && window.innerWidth < 800) {
        const div = document.createElement('div')
        // 根据不同的源给不同颜色
        const color = pub.source === 'screen_share' ? '#eab308' : '#22c55e'
        div.style.cssText = `position:fixed;top:80px;left:10px;right:10px;background:${color};color:white;padding:10px;z-index:9999;border-radius:8px;font-size:14px;font-weight:bold;box-shadow:0 4px 6px rgba(0,0,0,0.3);`
        div.innerText = debugMsg
        document.body.appendChild(div)
        setTimeout(() => div.remove(), 8000)
      }
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
        // 清理旧的同参与者轨道，防止 React key 冲突
        for (const [sid, info] of screenShareCache) {
          if (info.participantIdentity === p.identity) {
            screenShareCache.delete(sid)
          }
        }
        // 接收端画质偏好：从缓存读（用户可能切换过），默认 high
        const cachedQuality = screenShareCache.get(track.sid || '')?.quality || 'high'
        try {
          // @ts-ignore - setVideoQuality 在 2.x 仍可用
          // TrackSubscribed 时序修复：track attach 后立即调 setVideoQuality 可能无效（SFU 未就绪）
          // 首帧未到位时 setVideoQuality 可能被忽略；立即调 + 1.5s 后再调一次（test-20 的两次调用实证有效）
          const applyQ = () => { try { pub.setVideoQuality?.(toVideoQuality(cachedQuality as QualityStr)) } catch {} }
          applyQ()
          setTimeout(applyQ, 1500)
          // 默认行为：high 档不限制分辨率；其他档位让 LiveKit 自动按层
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
    .on(RoomEvent.TrackPublished, (pub: RemoteTrackPublication, _p: RemoteParticipant) => {
      // 暴力强制订阅所有远端轨道（防止因属性识别延迟漏掉视频）
      pub.setSubscribed(true)
      emit()
    })
    .on(RoomEvent.TrackUnpublished, () => emit())
    .on(RoomEvent.TrackMuted, () => emit())
    .on(RoomEvent.TrackUnmuted, () => emit())
    .on(RoomEvent.ParticipantMetadataChanged, () => emit())

  await room.connect(wsUrl, token)

  if (keyProvider) {
    try { await room.setE2EEEnabled(true) } catch (e) { console.error('[E2EE]', e) }
  }

  startBitrateSampling()
  startMediaHealth(room)
  // 暴露给 QA/线上诊断：避免测试里 import 模块拿到另一个实例（HMR 下会失真）
  ;(window as any).__kaedeRoom = room
  ;(window as any).__kaedeMediaState = () => getState()
  unsubHealth?.()
  unsubHealth = subscribeMediaHealth(onHealth)
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
  stopMediaHealth()
  unsubHealth?.()
  unsubHealth = null
  currentLink = null
  resetAdaptive()
  ;(window as any).__kaedeRoom = null
  emit()
}

/**
 * 开启屏幕共享（simulcast + 用户选定档位）。
 */
/**
 * 屏幕共享首选编码器。
 *
 * LiveKit 默认 VP8（纯软件编码）。游戏/高动态共享时编码器会把 CPU 吃满，
 * 表现就是“读文档没事、一打游戏就掉帧断续”。Windows 上 H.264 可走硬件编码
 * （Media Foundation / QuickSync / NVENC 的 H.264 引擎），明显降低 CPU 占用。
 * 浏览器没有 H.264 编码能力时退回 VP8。
 */
function preferredScreenShareCodec(): 'h264' | 'vp8' {
  // 现场二分用：?codec=vp8 或 localStorage.kaede.codec=vp8 强制软件编码。
  // 若 VP8 下高动态画面不再冻结，说明问题在 H.264 硬件编码器（游戏抢占 GPU）。
  try {
    const fromQuery = new URLSearchParams(window.location.search).get('codec')
    const forced = fromQuery ?? localStorage.getItem('kaede.codec')
    if (forced === 'vp8' || forced === 'h264') {
      console.log('[ScreenShare] codec forced by user: ' + forced)
      return forced
    }
  } catch {
    /* 非浏览器环境忽略 */
  }
  try {
    const caps = RTCRtpSender.getCapabilities('video')
    const hasH264 = !!caps?.codecs?.some((c) => /video\/h264/i.test(c.mimeType))
    return hasH264 ? 'h264' : 'vp8'
  } catch {
    return 'vp8'
  }
}

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

    // contentHint / degradationPreference 决定“码率不够时牺牲谁”：
    // - motion + maintain-framerate：优先保帧率（滚动、播放视频时画面跟手）
    // - detail + maintain-resolution：优先保清晰度（文字/代码）
    // 观众端目前报“帧率只有 5fps”，30/60fps 档位统一按流畅优先；15fps 及以下
    // 本身帧率预算充足，保持清晰优先。
    const smoothFirst = preset.fps >= 30
    videoTrack.contentHint = smoothFirst ? 'motion' : 'detail'
    console.log('[ScreenShare] capture settings:', JSON.stringify(videoTrack.getSettings()))

    // 游戏/高动态场景：编码器压力是主要瓶颈。H.264 在 Windows 上可走硬件编码
    // （Media Foundation），显著降低 CPU 占用；浏览器不支持时回退 VP8。
    const codec = preferredScreenShareCodec()
    console.log('[ScreenShare] preferred codec=' + codec)

    // 记录本端共享状态，供健康采样器判定“降档是否有效”
    resetAdaptive()
    activePreset = preset
    activeShareTrack = videoTrack
    setMediaHealthTargetFps(preset.fps)
    setMediaHealthContext({ presetId, codec })
    lastResubAt = 0
    
    // 开启 simulcast，让接收端能切到不同分辨率层（高/中/低）
    // 注意：屏幕共享单层的话，接收端调画质会导致 SFU 强制降码率 → 画面糊。
    // 必须发 simulcast 层，SVC/Simulcast 切层才能做到真正的"分辨率"变化。
    // LiveKit JS 2.23 在 screen_share video 上传 `simulcast: true` + 非空
    // `screenShareSimulcastLayers` 时，会把整条 video track 静默从 publisher offer
    // 中丢弃（audio 正常）。复现：Playwright 双端 screen-share-smoke，SFU 侧只会看到
    // SCREEN_SHARE_AUDIO 的 published 日志，video 从未发出，接收端 remoteParticipants
    // 的 videoTrackPublications 一直是空。改为单层编码后 video 正常到达接收端（已用
    // screen-share-diag-nosimulcast.spec.ts 验证）。
    //
    // 功能取舍：
    //   - 失去 SFU 层切换能力：接收端 setScreenShareQuality 只能走本地
    //     setVideoDimensions，无法真正让 SFU 降码率/降分辨率。
    //   - 恢复前提：要么 livekit-client 升级修复此 bug，要么在 SDP 层重新验证
    //     simulcast 组合。恢复时把本块改回 `simulcast: true` + `screenShareSimulcastLayers`。
    const videoPub = await currentRoom.localParticipant.publishTrack(videoTrack, {
      source: Track.Source.ScreenShare,
      simulcast: false,
      videoCodec: codec,
      screenShareEncoding: { maxBitrate: preset.bitrate, maxFramerate: preset.fps },
      degradationPreference: smoothFirst ? 'maintain-framerate' : 'maintain-resolution',
    })
    activeShareSender = (videoPub.track as LocalTrack | undefined)?.sender ?? null

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
    
    // 监听用户点系统/浏览器原生的“停止共享”：
    // LiveKit 的 LocalVideoTrack 没有 onended，必须挂在底层 MediaStreamTrack 上，
    // 否则浏览器原生停止后 UI 会一直显示“共享中”。
    videoTrack.onended = () => {
      stopScreenShare().catch(console.error)
    }
    
    console.log(`[ScreenShare] 已启动: ${preset.label} (${preset.bitrate/1e6} Mbps)`)
    emit()
    setTimeout(() => { isStartingShare = false }, 1000)
    return true
  } catch (e: any) {
    console.error('屏幕共享启动失败:', e)
    // 【修复 bug】降级重试必须在 isStartingShare 复位之后才能成功。
    // 原实现：setTimeout(1000) 是异步，下面 return startScreenShare('1080p60') 是
    // 同步立即递归 —— 此时 isStartingShare 还是 true，递归第一行直接 return false，
    // 用户选 4K 时屏幕共享完全不可用（MediaBar 的 handleToggleShare 不看返回值，
    // 仍然 setSharing(true)，造成"UI 显示共享中，实际零流"的静默失败）。
    // 由 tools/qa/playwright/specs/screen-share-4k-cancel.spec.ts 捕获。
    if (preset.width > 1920) {
      console.warn('[ScreenShare] 4K 失败，降级到 1080p60 重试')
      isStartingShare = false
      return startScreenShare('1080p60')
    }
    setTimeout(() => { isStartingShare = false }, 1000)
    throw e
  }
}

export async function stopScreenShare(): Promise<void> {
  if (!currentRoom) return
  resetAdaptive()
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
  let appliedLayer: 'low' | 'medium' | 'high' = 'high'
  for (const participant of currentRoom.remoteParticipants.values()) {
    const pub = participant.getTrackPublicationBySid(sid)
    if (pub && pub.source === Track.Source.ScreenShare) {
      try {
        // 动态选择层级：LiveKit 只支持 high/medium/low
        let layer: 'low' | 'medium' | 'high' = 'high'
        if (presetId.includes('4k') || presetId.includes('1440p') || presetId.includes('1080p')) {
          layer = 'high'
        } else if (presetId.includes('720p')) {
          layer = 'medium'
        } else if (presetId.includes('480p') || presetId.includes('360p')) {
          layer = 'low'
        }
        
        // 同时调用 dimensions 强制让 SFU 降分辨率
        let dimApplied = false
        const dim = presetId.match(/(\d+)p/)
        if (dim) {
          const h = parseInt(dim[1], 10)
          const w = h === 480 ? 854 : Math.round(h * 16 / 9)
          try { (pub as any).setVideoDimensions?.({ width: w, height: h }); dimApplied = true } catch (e) { console.warn('[ScreenShare] setVideoDimensions failed:', e) }
        }

        if (!dimApplied) pub.setVideoQuality?.(toVideoQuality(layer as QualityStr))
        console.log(`[ScreenShare] ${sid} -> ${presetId} (layer=${layer})`)
        appliedLayer = layer
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
  screenShareCache.set(sid, { ...info, quality: appliedLayer, presetId })
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

/** 供 QA/诊断读取当前媒体状态（不含 DOM）。 */
export function getMediaState(): MediaState {
  return getState()
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



