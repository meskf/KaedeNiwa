/**
 * 媒体链路诊断（屏幕共享 / 语音）。
 *
 * 只做采样和日志，不参与业务逻辑。每 3 秒打印一行 [MediaStats]，用于判断
 * “观众端帧率低”到底是哪一种：
 *   - out.limit=bandwidth  → 码率预算不够（预设 bpp 太低 / 上行拥塞）
 *   - out.limit=cpu        → 编码器跑不动（分辨率/帧率档位太高，或软件编码）
 *   - out.fps 正常 & in.fps 很低 → 丢包 / TCP 回落 / 中继（看 pair 行）
 *   - in.fps 正常但画面卡 → 解码端问题
 *
 * 浏览器控制台里也可以随时执行 __kaedeMediaStats() 取最近一次快照。
 */

import type { Room, TrackPublication } from 'livekit-client'
import { Track } from 'livekit-client'

type Prev = { ts: number; frames: number; bytes: number; packets: number; lost: number }
type Snapshot = Record<string, unknown>

const prevByKey = new Map<string, Prev>()
let timer: number | undefined
let lastSnapshot: Snapshot = {}

function rate(cur: number, prev: number | undefined, dtMs: number): number {
  if (prev === undefined || dtMs <= 0) return 0
  return ((cur - prev) / dtMs) * 1000
}

function mbit(bps: number): string {
  return (bps / 1_000_000).toFixed(2)
}

/** 从一份 RTCStatsReport 里找已选中的 candidate pair，给出传输方式与 RTT。 */
function transportOf(report: RTCStatsReport): string {
  let pair: any
  let pairIdFromTransport = ''
  const candidates = new Map<string, any>()
  report.forEach((v: any) => {
    if (v.type === 'transport') pairIdFromTransport = v.selectedCandidatePairId ?? ''
    if (v.type === 'local-candidate' || v.type === 'remote-candidate') candidates.set(v.id, v)
  })
  if (pairIdFromTransport) pair = report.get(pairIdFromTransport)
  if (!pair) {
    report.forEach((v: any) => {
      if (!pair && v.type === 'candidate-pair' && v.state === 'succeeded') pair = v
    })
  }
  if (!pair) return 'pair=?'
  const local = candidates.get(pair.localCandidateId)
  const remote = candidates.get(pair.remoteCandidateId)
  const rtt = pair.currentRoundTripTime !== undefined ? `${Math.round(pair.currentRoundTripTime * 1000)}ms` : 'rtt=?'
  return `pair=${(local?.protocol ?? remote?.protocol ?? '?').toUpperCase()} ${local?.candidateType ?? '?'}->${remote?.candidateType ?? '?'} ${rtt}`
}

async function sampleOutbound(key: string, pub: TrackPublication) {
  const track = pub.track as any
  if (!track?.getRTCStatsReport) return
  const report: RTCStatsReport | undefined = await track.getRTCStatsReport()
  if (!report) return

  let video: any
  const codecs = new Map<string, any>()
  report.forEach((v: any) => {
    if (v.type === 'outbound-rtp' && v.kind === 'video') video = v
    if (v.type === 'codec') codecs.set(v.id, v)
  })
  if (!video) return

  const now = video.timestamp ?? performance.now()
  const p = prevByKey.get(key)
  const dt = p ? now - p.ts : 0
  const fps = rate(video.framesEncoded ?? 0, p?.frames, dt)
  const kbps = rate(video.bytesSent ?? 0, p?.bytes, dt) * 8
  prevByKey.set(key, {
    ts: now,
    frames: video.framesEncoded ?? 0,
    bytes: video.bytesSent ?? 0,
    packets: video.packetsSent ?? 0,
    lost: 0,
  })

  const mime = codecs.get(video.codecId)?.mimeType ?? '?'
  const limit = video.qualityLimitationReason ?? 'none'
  const target = video.targetBitrate ? `${mbit(video.targetBitrate)}Mbps` : '?'
  const retrans = (video.retransmittedPacketsSent ?? 0)
  lastSnapshot[`out:${key}`] = { fps, kbps, width: video.frameWidth, height: video.frameHeight, limit, target }
  console.log(
    `[MediaStats] out ${key} ${mime} ${video.frameWidth ?? '?'}x${video.frameHeight ?? '?'} ` +
      `fps=${fps.toFixed(1)} ${mbit(kbps)}Mbps target=${target} limit=${limit} ` +
      `nack=${video.nackCount ?? 0} retrans=${retrans} | ${transportOf(report)}`,
  )
}

async function sampleInbound(key: string, pub: TrackPublication) {
  const track = pub.track as any
  if (!track?.getRTCStatsReport) return
  const report: RTCStatsReport | undefined = await track.getRTCStatsReport()
  if (!report) return

  let video: any
  const codecs = new Map<string, any>()
  report.forEach((v: any) => {
    if (v.type === 'inbound-rtp' && v.kind === 'video') video = v
    if (v.type === 'codec') codecs.set(v.id, v)
  })
  if (!video) return

  const now = video.timestamp ?? performance.now()
  const p = prevByKey.get(key)
  const dt = p ? now - p.ts : 0
  const fps = rate(video.framesDecoded ?? 0, p?.frames, dt)
  const mbps = rate(video.bytesReceived ?? 0, p?.bytes, dt) * 8
  const lostDelta = rate(video.packetsLost ?? 0, p?.lost, dt)
  const recvDelta = rate(video.packetsReceived ?? 0, p?.packets, dt)
  prevByKey.set(key, {
    ts: now,
    frames: video.framesDecoded ?? 0,
    bytes: video.bytesReceived ?? 0,
    packets: video.packetsReceived ?? 0,
    lost: video.packetsLost ?? 0,
  })

  const lossPct = recvDelta + lostDelta > 0 ? (lostDelta / (recvDelta + lostDelta)) * 100 : 0
  const mime = codecs.get(video.codecId)?.mimeType ?? '?'
  lastSnapshot[`in:${key}`] = { fps, mbps, width: video.frameWidth, height: video.frameHeight, lossPct }
  console.log(
    `[MediaStats] in  ${key} ${mime} ${video.frameWidth ?? '?'}x${video.frameHeight ?? '?'} ` +
      `fps=${fps.toFixed(1)} ${mbit(mbps)}Mbps loss=${lossPct.toFixed(1)}% ` +
      `frozen=${video.freezeCount ?? 0} pli=${video.pliCount ?? 0} nack=${video.nackCount ?? 0} | ${transportOf(report)}`,
  )
}

async function tick(room: Room) {
  try {
    for (const pub of room.localParticipant.videoTrackPublications.values()) {
      if (pub.source === Track.Source.ScreenShare || pub.source === Track.Source.Camera) {
        await sampleOutbound(pub.trackSid, pub)
      }
    }
    for (const participant of room.remoteParticipants.values()) {
      for (const pub of participant.videoTrackPublications.values()) {
        if (pub.source === Track.Source.ScreenShare || pub.source === Track.Source.Camera) {
          await sampleInbound(`${participant.identity}/${pub.trackSid}`, pub)
        }
      }
    }
  } catch (e) {
    console.warn('[MediaStats] 采样失败:', e)
  }
}

export function startMediaStats(room: Room) {
  stopMediaStats()
  prevByKey.clear()
  lastSnapshot = {}
  timer = window.setInterval(() => void tick(room), 3000)
  ;(window as any).__kaedeMediaStats = () => lastSnapshot
}

export function stopMediaStats() {
  if (timer !== undefined) {
    window.clearInterval(timer)
    timer = undefined
  }
  prevByKey.clear()
}
