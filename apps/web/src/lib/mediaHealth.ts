/**
 * 屏幕共享链路健康度（诊断 + 自动降档依据）。
 *
 * 与旧版 mediaStats（只打日志）不同，这里每秒采样一次并维护一个滚动窗口，
 * 直接回答“卡顿到底是哪一层的问题”：
 *
 *   1. 传输层：selected candidate pair 是 UDP 还是 TCP / 是否 relay，
 *      有没走 TCP 回落（TCP 回落是“能用但持续掉帧、断续”的头号原因）。
 *   2. 发送端：framesEncoded 实际帧率 + qualityLimitationReason
 *      （cpu = 编码器跑不动；bandwidth = 码率被拥塞控制压死）。
 *   3. 接收端：framesDecoded 实际帧率、freezeCount / totalFreezesDuration
 *      （真冻结）、packetsLost 丢包率、jitter 抖动。
 *
 * 只在采样/记录，不改变业务。控制台可随时执行 `__kaedeMediaHealth()`。
 */

import type { Room, TrackPublication } from 'livekit-client'
import { Track } from 'livekit-client'

export type HealthReason =
  | 'tcp-fallback'
  | 'relay'
  | 'packet-loss'
  | 'high-jitter'
  | 'cpu-limited'
  | 'bandwidth-limited'
  | 'low-fps'
  | 'unstable-pacing'
  | 'freezes'

export type HealthSeverity = 'ok' | 'warn' | 'bad'

export interface EndpointHealth {
  fps: number
  nominalFps: number
  mbps: number
  width: number
  height: number
  /** 仅出站：encoder 自报的受限原因 */
  limit?: 'cpu' | 'bandwidth' | 'other' | 'none'
  /** 仅入站：丢包率 % */
  lossPct?: number
  /** 仅入站：抖动 ms */
  jitterMs?: number
  /** 仅入站：最近窗口内的冻结次数 / 累计冻结时长 */
  freezes?: number
  frozenMs?: number
  /** 仅入站：平均帧间隔 ms（理想值 1000/fps） */
  frameIntervalMs?: number
  /** 仅入站：帧间隔标准差 ms —— 这才是“帧稳定不稳定”的直接指标 */
  frameIntervalStdMs?: number
  /** 仅入站：窗口内丢弃的帧数（解码器/播放侧） */
  droppedFrames?: number
  /** 关键帧累计数（出站 keyFramesEncoded / 入站 keyFramesDecoded）；长时间不增长说明请求不到关键帧 */
  keyFrames?: number
  /**
   * 连续“有数据但无画面”的秒数。
   * 入站口径：fps≈0 且仍有码率在流入 → 解码器拿不到关键帧，画面冻结。
   * 出站口径：framesEncoded≈0 且 encoder 自报 cpu 受限 → 编码器停摆。
   */
  stalledSec?: number
  /** 仅出站：SFU 反馈的上行丢包率（remote-inbound-rtp），区分“上行丢”还是“观众侧下行丢” */
  uplinkLossPct?: number
  uplinkRttMs?: number
  /** 仅入站：浏览器没有提供 packetsReceived，丢包率无法计算（不能当 0 也不能当 100） */
  lossUnknown?: boolean
}

export interface MediaHealth {
  ts: number
  transport: 'udp' | 'direct-tcp' | 'relay' | 'unknown'
  rttMs: number
  outbound: EndpointHealth | null
  inbound: EndpointHealth | null
  reasons: HealthReason[]
  severity: HealthSeverity
  /** 最近 windowSec 秒内的统计（用于“断断续续”判定） */
  windowSec: number
  zeroFrameBuckets: number
  maxFrameGapMs: number
  sampledBuckets: number
}

interface PrevSample {
  ts: number
  frames: number
  bytes: number
  packets: number
  lost: number
  freezes: number
  frozenSeconds: number
  /** inbound-rtp 的累计帧间隔（秒）及其平方和，用于算帧间隔标准差 */
  delay?: number
  sqDelay?: number
  dropped?: number
  /** 连续“有数据无画面/编码停摆”的采样秒数 */
  stall?: number
  /** SFU 反馈的累计上行丢包数 */
  upLost?: number
}

/** 有一次“画面冻结”事件的证据快照，供事后（不用边打游戏边操作）导出分析。 */
export interface DiagEvent {
  ts: number
  side: 'in' | 'out'
  presetId: string | null
  codec: string | null
  snapshot: MediaHealth
}

export interface DiagReport {
  version: string
  ts: number
  targetFps: number
  presetId: string | null
  codec: string | null
  ua: string
  /** 最近 30 秒每秒一条 */
  history: MediaHealth[]
  /** 最近 20 次冻结/编码停摆事件 */
  events: DiagEvent[]
}

const DIAG_VERSION = 'diag-2'
const DIAG_HISTORY_MAX = 30
const DIAG_EVENTS_MAX = 20
const DIAG_EVENTS_KEY = 'kaede.diag.events'

export const HEALTH_WINDOW_SEC = 10

const prev = new Map<string, PrevSample>()
const buckets = new Map<string, number[]>() // 每秒入站帧数
const lossBuckets = new Map<string, Array<{ lost: number; recv: number }>>() // 每秒丢包/收包，滚动窗口
const uplinkBuckets = new Map<string, Array<{ lost: number; sent: number }>>() // 每秒上行丢包/发包，滚动窗口
const diagHistory: MediaHealth[] = []
const diagEvents: DiagEvent[] = []
const diagMeta: { presetId: string | null; codec: string | null } = { presetId: null, codec: null }
let timer: number | undefined
let targetFps = 30
let last: MediaHealth = emptyHealth()
const listeners = new Set<(h: MediaHealth) => void>()
const warnedKeys = new Set<string>()

function emptyHealth(): MediaHealth {
  return {
    ts: 0,
    transport: 'unknown',
    rttMs: 0,
    outbound: null,
    inbound: null,
    reasons: [],
    severity: 'ok',
    windowSec: HEALTH_WINDOW_SEC,
    zeroFrameBuckets: 0,
    maxFrameGapMs: 0,
    sampledBuckets: 0,
  }
}

function rate(cur: number, old: number | undefined, dtMs: number): number {
  if (old === undefined || dtMs <= 0) return 0
  return ((cur - old) / dtMs) * 1000
}

/** 解析 selected candidate pair：传输方式 + relay + RTT。 */
function transportOf(report: RTCStatsReport): { transport: MediaHealth['transport']; rttMs: number } {
  let pair: any
  let pairId = ''
  const candidates = new Map<string, any>()
  report.forEach((v: any) => {
    if (v.type === 'transport') pairId = v.selectedCandidatePairId ?? ''
    if (v.type === 'local-candidate' || v.type === 'remote-candidate') candidates.set(v.id, v)
  })
  if (pairId) pair = report.get(pairId)
  if (!pair) {
    report.forEach((v: any) => {
      if (!pair && v.type === 'candidate-pair' && v.state === 'succeeded') pair = v
    })
  }
  if (!pair) return { transport: 'unknown', rttMs: 0 }
  const local = candidates.get(pair.localCandidateId)
  const remote = candidates.get(pair.remoteCandidateId)
  const proto = String(local?.protocol ?? remote?.protocol ?? '').toLowerCase()
  const relay = local?.candidateType === 'relay' || remote?.candidateType === 'relay'
  const rttMs = pair.currentRoundTripTime !== undefined ? Math.round(pair.currentRoundTripTime * 1000) : 0
  if (relay) return { transport: 'relay', rttMs }
  if (proto === 'udp') return { transport: 'udp', rttMs }
  if (proto === 'tcp') return { transport: 'direct-tcp', rttMs }
  return { transport: 'unknown', rttMs }
}

async function sampleOutbound(key: string, pub: TrackPublication): Promise<{ ep: EndpointHealth; transport: MediaHealth['transport']; rttMs: number } | null> {
  const track = pub.track as any
  if (!track?.getRTCStatsReport) return null
  const report: RTCStatsReport | undefined = await track.getRTCStatsReport()
  if (!report) return null

  let video: any
  report.forEach((v: any) => {
    if (v.type === 'outbound-rtp' && v.kind === 'video') video = v
  })
  if (!video) return null

  const now = (video.timestamp as number) ?? performance.now()
  const p = prev.get(key)
  const dt = p ? now - p.ts : 0
  const fps = Math.max(0, rate(video.framesEncoded ?? 0, p?.frames, dt))
  const mbps = Math.max(0, (rate(video.bytesSent ?? 0, p?.bytes, dt) * 8) / 1e6)
  const keyFrames = video.keyFramesEncoded ?? 0
  const rawLimit = String(video.qualityLimitationReason ?? 'none')
  const limit: EndpointHealth['limit'] =
    rawLimit === 'cpu' ? 'cpu' : rawLimit === 'bandwidth' ? 'bandwidth' : rawLimit === 'none' ? 'none' : 'other'
  // 只有 encoder 自报 cpu 受限且完全不出帧时才算“编码器停摆”；
  // 静态文档本来就不出帧（Chrome 只在画面变化时编码），不能算停摆。
  const stalledSec = limit === 'cpu' && fps < 0.5 ? (p?.stall ?? 0) + 1 : 0
  prev.set(key, {
    ts: now,
    frames: video.framesEncoded ?? 0,
    bytes: video.bytesSent ?? 0,
    packets: video.packetsSent ?? 0,
    lost: 0,
    freezes: 0,
    frozenSeconds: 0,
    stall: stalledSec,
    upLost: 0,
  })

  const t = transportOf(report)
  // SFU 回传的 remote-inbound-rtp：只有发布端才有。
  // 观众端的 inbound 丢包可能是“观众网络”，而上行丢包只能在这里看到。
  let upLost = 0
  let upRtt = 0
  report.forEach(s => {
    if (s.type !== 'remote-inbound-rtp' || s.kind !== 'video') return
    upLost = Math.max(upLost, Number(s.packetsLost ?? 0))
    if (typeof s.roundTripTime === 'number') upRtt = s.roundTripTime * 1000
  })
  // 上行丢包率同样按 10 秒窗口算：用累计值会被早期的一次丢包永久拉高，
  // 导致自动降码率一路降到地板。
  const sentDelta = Math.max(0, rate(video.packetsSent ?? 0, p?.packets, dt))
  const upLostDelta = Math.max(0, rate(upLost, p?.upLost, dt))
  const ub = uplinkBuckets.get(key) ?? []
  if (dt > 0) ub.push({ lost: upLostDelta, sent: sentDelta })
  while (ub.length > HEALTH_WINDOW_SEC) ub.shift()
  uplinkBuckets.set(key, ub)
  const sumUpLost = ub.reduce((n, b) => n + b.lost, 0)
  const sumUpSent = ub.reduce((n, b) => n + b.sent, 0)
  const uplinkLossPct = sumUpSent > 0 ? Math.min(100, (sumUpLost / sumUpSent) * 100) : 0
  prev.set(key, { ...prev.get(key)!, upLost })

  return {
    ep: {
      fps,
      nominalFps: targetFps,
      mbps,
      width: video.frameWidth ?? 0,
      height: video.frameHeight ?? 0,
      limit,
      keyFrames,
      stalledSec,
      uplinkLossPct,
      uplinkRttMs: upRtt,
    },
    transport: t.transport,
    rttMs: t.rttMs,
  }
}

async function sampleInbound(key: string, pub: TrackPublication): Promise<{ ep: EndpointHealth; transport: MediaHealth['transport']; rttMs: number } | null> {
  const track = pub.track as any
  if (!track?.getRTCStatsReport) return null
  const report: RTCStatsReport | undefined = await track.getRTCStatsReport()
  if (!report) return null

  let video: any
  report.forEach((v: any) => {
    if (v.type === 'inbound-rtp' && v.kind === 'video') video = v
  })
  if (!video) return null

  const now = (video.timestamp as number) ?? performance.now()
  const p = prev.get(key)
  const dt = p ? now - p.ts : 0
  const fps = Math.max(0, rate(video.framesDecoded ?? 0, p?.frames, dt))
  const mbps = Math.max(0, (rate(video.bytesReceived ?? 0, p?.bytes, dt) * 8) / 1e6)
  const lostDelta = Math.max(0, rate(video.packetsLost ?? 0, p?.lost, dt))
  // 部分浏览器/版本的 inbound-rtp 不提供 packetsReceived。缺了它分母就只剩
  // lostDelta，任何一次丢包都会算出 100%，这正是“丢包率极高、却和分辨率无关”的来源。
  const recvKnown = typeof video.packetsReceived === 'number'
  const recvDelta = recvKnown ? Math.max(0, rate(video.packetsReceived, p?.packets, dt)) : 0
  const freezeDelta = Math.max(0, rate(video.freezeCount ?? 0, p?.freezes, dt))
  const frozenMsDelta = Math.max(0, rate(video.totalFreezesDuration ?? 0, p?.frozenSeconds, dt) * 1000)
  const keyFrames = video.keyFramesDecoded ?? 0

  // 帧间隔稳定性：Chrome 的 inbound-rtp 提供 totalInterFrameDelay（秒）与
  // totalSquaredInterFrameDelay，二者之差除以帧数就是帧间隔方差 → 标准差。
  // 平均帧率达标但帧间隔忽长忽短，就是肉眼看到的“一卡一卡/不跟手”。
  const framesDelta = Math.max(0, (video.framesDecoded ?? 0) - (p?.frames ?? video.framesDecoded ?? 0))
  let frameIntervalMs = 0
  let frameIntervalStdMs = 0
  if (p && p.delay !== undefined && p.sqDelay !== undefined && framesDelta > 0) {
    const mean = ((video.totalInterFrameDelay ?? 0) - p.delay) / framesDelta
    const meanSq = ((video.totalSquaredInterFrameDelay ?? 0) - p.sqDelay) / framesDelta
    if (mean > 0) {
      frameIntervalMs = mean * 1000
      frameIntervalStdMs = Math.sqrt(Math.max(0, meanSq - mean * mean)) * 1000
    }
  }
  const droppedFrames = Math.max(0, (video.framesDropped ?? 0) - (p?.dropped ?? video.framesDropped ?? 0))

  // “有码率在流入、但一帧都解不出来” → 关键帧丢失，画面冻结。
  // 静态文档时码率≈0，不会误判成冻结。
  const stalledSec = fps < 0.5 && mbps > 0.2 ? (p?.stall ?? 0) + 1 : 0

  prev.set(key, {
    ts: now,
    frames: video.framesDecoded ?? 0,
    bytes: video.bytesReceived ?? 0,
    packets: video.packetsReceived ?? 0,
    lost: video.packetsLost ?? 0,
    freezes: video.freezeCount ?? 0,
    frozenSeconds: video.totalFreezesDuration ?? 0,
    delay: video.totalInterFrameDelay,
    sqDelay: video.totalSquaredInterFrameDelay,
    dropped: video.framesDropped ?? 0,
    stall: stalledSec,
  })

  // 丢包率按 10 秒窗口累计：单秒突发不应被显示成“极高丢包”。
  const lb = lossBuckets.get(key) ?? []
  if (dt > 0) lb.push({ lost: lostDelta, recv: recvDelta })
  while (lb.length > HEALTH_WINDOW_SEC) lb.shift()
  lossBuckets.set(key, lb)
  const sumLost = lb.reduce((n, b) => n + b.lost, 0)
  const sumRecv = lb.reduce((n, b) => n + b.recv, 0)
  const lossPct = recvKnown && sumLost + sumRecv > 0 ? (sumLost / (sumLost + sumRecv)) * 100 : 0
  const jitterMs = typeof video.jitter === 'number' ? video.jitter * 1000 : 0

  // 每秒入站帧数，用于检测“断断续续”（某些秒完全无帧 / 帧数骤降）
  const list = buckets.get(key) ?? []
  if (dt > 0) list.push(fps * (dt / 1000))
  while (list.length > HEALTH_WINDOW_SEC) list.shift()
  buckets.set(key, list)

  const t = transportOf(report)
  return {
    ep: {
      fps,
      nominalFps: targetFps,
      mbps,
      width: video.frameWidth ?? 0,
      height: video.frameHeight ?? 0,
      lossPct,
      jitterMs,
      freezes: freezeDelta,
      frozenMs: frozenMsDelta,
      frameIntervalMs,
      frameIntervalStdMs,
      droppedFrames,
      keyFrames,
      stalledSec,
      lossUnknown: !recvKnown,
    },
    transport: t.transport,
    rttMs: t.rttMs,
  }
}

function computeWindow(key: string): { zeroFrameBuckets: number; maxFrameGapMs: number; sampledBuckets: number; peakFps: number } {
  const list = buckets.get(key) ?? []
  // 只统计进入稳定期的桶（前 2 秒通常含首帧/关键帧，不算断续）
  const stable = list.slice(2)
  if (stable.length === 0) return { zeroFrameBuckets: 0, maxFrameGapMs: 0, sampledBuckets: 0, peakFps: 0 }
  // 以窗口内峰值作为“目标帧率”基准：接收端并不知道发布方选的档位，
  // 但峰值能反映这一路真实能达到的帧率（30fps 档峰值≈30，60fps 档峰值≈60）。
  const peak = Math.max(1, ...stable)
  let zero = 0
  let maxGap = 0
  for (const frames of stable) {
    if (frames < peak * 0.25) {
      zero++
      maxGap = Math.max(maxGap, ((peak - frames) / peak) * 1000)
    }
  }
  return { zeroFrameBuckets: zero, maxFrameGapMs: Math.round(maxGap), sampledBuckets: stable.length, peakFps: peak }
}

function deriveReasons(out: EndpointHealth | null, inbound: EndpointHealth | null, transport: MediaHealth['transport'], win: ReturnType<typeof computeWindow>): HealthReason[] {
  const r: HealthReason[] = []
  if (transport === 'direct-tcp') r.push('tcp-fallback')
  if (transport === 'relay') r.push('relay')
  if (out?.limit === 'cpu') r.push('cpu-limited')
  if (out?.limit === 'bandwidth') r.push('bandwidth-limited')
  if (inbound) {
    if ((inbound.lossPct ?? 0) > 3) r.push('packet-loss')
    if ((inbound.jitterMs ?? 0) > 40) r.push('high-jitter')
    const ref = Math.max(inbound.nominalFps, win.peakFps)
    if (ref >= 20 && (inbound.fps ?? 0) < ref * 0.7) r.push('low-fps')
    // 帧稳定：平均帧率够了，但帧间隔忽长忽短（标准差大）同样是“卡/不跟手”。
    // 30fps 的理想间隔是 33ms，标准差超过 18ms 或超过间隔的 45% 就判为不稳定。
    const expectedMs = 1000 / Math.max(1, ref)
    const stdMs = inbound.frameIntervalStdMs ?? 0
    if ((inbound.fps ?? 0) >= 15 && stdMs > Math.max(18, expectedMs * 0.45)) r.push('unstable-pacing')
    if ((inbound.droppedFrames ?? 0) > 2) r.push('unstable-pacing')
    if ((inbound.freezes ?? 0) > 0 || win.zeroFrameBuckets > 0) r.push('freezes')
  }
  return r
}

function severityOf(reasons: HealthReason[]): HealthSeverity {
  if (reasons.includes('tcp-fallback') || reasons.includes('packet-loss') || reasons.includes('freezes')) return 'bad'
  if (reasons.length > 0) return 'warn'
  return 'ok'
}

function warnOnce(h: MediaHealth) {
  const tips: Partial<Record<HealthReason, string>> = {
    'tcp-fallback': '媒体走 TCP 回落（UDP 7882 不可达）：会周期性丢帧/断续，请在服务器安全组放行 UDP 7882，或启用 TURN。',
    relay: '媒体经 TURN 中继：延迟更高，优先确保 UDP 直连可达。',
    'packet-loss': '接收端丢包率偏高：链路抖动/拥塞，检查网络质量。',
    'high-jitter': '接收端抖动偏高：画面会出现断续。',
    'cpu-limited': '发送端编码器 CPU 受限：已触发自动降档。',
    'bandwidth-limited': '发送端码率被带宽压制：画面降质/掉帧。',
    'low-fps': '接收端实际帧率低于目标值：网络正常时通常是发布端抓屏/编码跟不上（游戏独占全屏尤其明显，建议改为无边框窗口）。',
    freezes: '接收端出现冻结/空帧，画面会卡顿：网络指标正常时优先排查发布端抓屏（独占全屏 → 改无边框窗口）与编码器占用。',
    'unstable-pacing': '接收端帧间隔不均匀（平均帧率可能正常，但节奏忽快忽慢）：优先排查发布端抓屏/编码抖动，其次检查丢包与抖动。',
  }
  for (const reason of h.reasons) {
    const key = `warn:${reason}`
    if (warnedKeys.has(key)) continue
    warnedKeys.add(key)
    console.warn(`[MediaHealth] ${tips[reason] ?? reason}`, h)
  }
}

function recordStall(side: 'in' | 'out', h: MediaHealth) {
  const ep = side === 'in' ? h.inbound : h.outbound
  // 只在“刚满 2 秒”那一刻记录一次，避免每秒刷屏/刷爆存储
  if (!ep?.stalledSec || ep.stalledSec !== 2) return
  diagEvents.push({ ts: Date.now(), side, presetId: diagMeta.presetId, codec: diagMeta.codec, snapshot: h })
  while (diagEvents.length > DIAG_EVENTS_MAX) diagEvents.shift()
  try {
    localStorage.setItem(DIAG_EVENTS_KEY, JSON.stringify(diagEvents))
  } catch {
    /* 隐私模式/配额不足时忽略 */
  }
  console.warn(
    `[Diag] ${side === 'in' ? '接收端连续 2 秒有数据但无画面（疑似关键帧丢失）' : '编码器连续 2 秒无输出（疑似 CPU/GPU 被打满）'}`,
    h,
  )
}

async function tick(room: Room) {
  try {
    let transport: MediaHealth['transport'] = 'unknown'
    let rttMs = 0
    let outbound: EndpointHealth | null = null
    let inbound: EndpointHealth | null = null
    let windowStats: ReturnType<typeof computeWindow> = { zeroFrameBuckets: 0, maxFrameGapMs: 0, sampledBuckets: 0, peakFps: 0 }

    for (const pub of room.localParticipant.videoTrackPublications.values()) {
      if (pub.source !== Track.Source.ScreenShare && pub.source !== Track.Source.Camera) continue
      const s = await sampleOutbound(pub.trackSid, pub)
      if (s) {
        outbound = s.ep
        transport = s.transport
        rttMs = s.rttMs
      }
      break // 只关心本端共享的那一路
    }

    for (const participant of room.remoteParticipants.values()) {
      for (const pub of participant.videoTrackPublications.values()) {
        if (pub.source !== Track.Source.ScreenShare && pub.source !== Track.Source.Camera) continue
        const key = `${participant.identity}/${pub.trackSid}`
        const s = await sampleInbound(key, pub)
        if (s) {
          inbound = s.ep
          transport = transport === 'unknown' ? s.transport : transport
          rttMs = rttMs || s.rttMs
          windowStats = computeWindow(key)
          inbound.nominalFps = Math.max(inbound.nominalFps, windowStats.peakFps)
        }
        break // 只看第一路共享
      }
    }

    const reasons = deriveReasons(outbound, inbound, transport, windowStats)
    last = {
      ts: Date.now(),
      transport,
      rttMs,
      outbound,
      inbound,
      reasons,
      severity: severityOf(reasons),
      windowSec: HEALTH_WINDOW_SEC,
      ...windowStats,
    }

    const outTxt = outbound
      ? `out ${outbound.width}x${outbound.height} fps=${outbound.fps.toFixed(1)}/${outbound.nominalFps} ${outbound.mbps.toFixed(2)}Mbps up=${(outbound.uplinkLossPct ?? 0).toFixed(1)}% kf=${outbound.keyFrames ?? 0} limit=${outbound.limit} stall=${outbound.stalledSec ?? 0}s`
      : 'out -'
    const inTxt = inbound
      ? `in  ${inbound.width}x${inbound.height} fps=${inbound.fps.toFixed(1)} ${inbound.mbps.toFixed(2)}Mbps loss=${(inbound.lossPct ?? 0).toFixed(1)}%${inbound.lossUnknown ? '(unknown)' : ''} jitter=${(inbound.jitterMs ?? 0).toFixed(0)}ms gap=${(inbound.frameIntervalStdMs ?? 0).toFixed(0)}ms kf=${inbound.keyFrames ?? 0} dropped=${inbound.droppedFrames ?? 0} freeze=${(inbound.freezes ?? 0).toFixed(1)}/${(inbound.frozenMs ?? 0).toFixed(0)}ms stall=${inbound.stalledSec ?? 0}s`
      : 'in  -'
    console.log(
      `[MediaHealth] ${transport} rtt=${rttMs}ms | ${outTxt} | ${inTxt} | gaps=${windowStats.zeroFrameBuckets}/${windowStats.sampledBuckets}` +
        (reasons.length ? ` | ${reasons.join(',')}` : ''),
    )
    diagHistory.push(last)
    while (diagHistory.length > DIAG_HISTORY_MAX) diagHistory.shift()
    recordStall('out', last)
    recordStall('in', last)
    warnOnce(last)
    for (const fn of listeners) fn(last)
  } catch (e) {
    console.warn('[MediaHealth] 采样失败:', e)
  }
}

export function startMediaHealth(room: Room) {
  stopMediaHealth()
  prev.clear()
  buckets.clear()
  lossBuckets.clear()
  uplinkBuckets.clear()
  warnedKeys.clear()
  diagHistory.length = 0
  last = emptyHealth()
  timer = window.setInterval(() => void tick(room), 1000)
  ;(window as any).__kaedeMediaHealth = () => last
  ;(window as any).__kaedeDiag = () => getDiag()
  // 兼容旧诊断入口
  ;(window as any).__kaedeMediaStats = () => last
}

export function stopMediaHealth() {
  if (timer !== undefined) {
    window.clearInterval(timer)
    timer = undefined
  }
  prev.clear()
  buckets.clear()
  lossBuckets.clear()
  uplinkBuckets.clear()
}

/** 记录当前共享档位/编码器，冻结事件里会带上，便于事后定位。 */
export function setMediaHealthContext(ctx: { presetId?: string | null; codec?: string | null }) {
  if (ctx.presetId !== undefined) diagMeta.presetId = ctx.presetId
  if (ctx.codec !== undefined) diagMeta.codec = ctx.codec
}

/** 事后诊断：最近 30 秒逐秒数据 + 冻结事件（控制台 `copy(JSON.stringify(__kaedeDiag()))`）。 */
export function getDiag(): DiagReport {
  return {
    version: DIAG_VERSION,
    ts: Date.now(),
    targetFps,
    presetId: diagMeta.presetId,
    codec: diagMeta.codec,
    ua: typeof navigator !== 'undefined' ? navigator.userAgent : '',
    history: diagHistory.slice(),
    events: diagEvents.slice(),
  }
}

/** 发布端把当前档位目标帧率告诉健康采样器，用于判定 “帧率是否达标”。 */
export function setMediaHealthTargetFps(fps: number) {
  targetFps = Math.max(1, fps)
}

export function getMediaHealth(): MediaHealth {
  return last
}

export function subscribeMediaHealth(fn: (h: MediaHealth) => void): () => void {
  listeners.add(fn)
  fn(last)
  return () => listeners.delete(fn)
}
