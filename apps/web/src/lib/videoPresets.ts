/**
 * 视频画质档位（与 kaede-protocol/video.rs 的 VideoProfile 对齐）。
 *
 * 选定的档位会传给 getDisplayMedia + LiveKit simulcast。
 * 4K60 = 3840×2160 @ 60fps，需要用户显示器 + 硬件编码器都支持，否则浏览器自动降级。
 */

export interface VideoPreset {
  id: string
  label: string
  width: number
  height: number
  fps: number
  /** 建议码率（bps），屏幕共享按 0.08 bpp 估算（见下方 calcScreenShareBitrate） */
  bitrate: number
}

// 屏幕共享码率估算。
//
// 0.032 bpp 是“视频通话”的低运动量经验值。屏幕共享在滚动、播放视频、拖窗口时
// 运动量远高于通话，用同样的 bpp 会把编码器的码率预算压死；Chrome 的实时编码器
// 在预算不够时优先丢帧 → 观众端只剩 5~10fps 且帧间隔抖动。
// 这里改用 0.08 bpp（对标 LiveKit 自带 ScreenSharePresets：1080p30 ≈ 5 Mbps），
// 4K 上限 25 Mbps，避免把上行带宽一次性打满（实际码率仍由拥塞控制决定）。
const MIN_SCREEN_BITRATE = 300_000
const MAX_SCREEN_BITRATE = 25_000_000

function calcScreenShareBitrate(width: number, height: number, fps: number): number {
  const raw = Math.round(width * height * fps * 0.08)
  return Math.min(MAX_SCREEN_BITRATE, Math.max(MIN_SCREEN_BITRATE, raw))
}

export const VIDEO_PRESETS: VideoPreset[] = [
  { id: '4k60',   label: '3840x2160 60fps',      width: 3840, height: 2160, fps: 60, bitrate: calcScreenShareBitrate(3840, 2160, 60) },
  { id: '4k30',   label: '3840x2160 30fps',      width: 3840, height: 2160, fps: 30, bitrate: calcScreenShareBitrate(3840, 2160, 30) },
  { id: '1440p60',label: '2560x1440 60fps',   width: 2560, height: 1440, fps: 60, bitrate: calcScreenShareBitrate(2560, 1440, 60) },
  { id: '1080p60',label: '1920x1080 60fps',   width: 1920, height: 1080, fps: 60, bitrate: calcScreenShareBitrate(1920, 1080, 60) },
  { id: '1080p30',label: '1920x1080 30fps',   width: 1920, height: 1080, fps: 30, bitrate: calcScreenShareBitrate(1920, 1080, 30) },
  // 高动态（游戏 / 高速滚动）推荐档：60fps 保证节奏，720p 把单帧编码耗时压下来。
  // CS2 这类快节奏画面在 30fps 下天然发飘，即使编码器不丢帧也“看着卡”。
  { id: '720p60', label: '1280x720 60fps',    width: 1280, height: 720,  fps: 60, bitrate: calcScreenShareBitrate(1280, 720, 60) },
  { id: '720p30', label: '1280x720 30fps',    width: 1280, height: 720,  fps: 30, bitrate: calcScreenShareBitrate(1280, 720, 30) },
  { id: '480p30', label: '854x480 30fps', width: 854, height: 480, fps: 30, bitrate: calcScreenShareBitrate(854, 480, 30) },
]

export const DEFAULT_PRESET_ID = '1080p60'

export function findPreset(id: string): VideoPreset {
  return VIDEO_PRESETS.find(p => p.id === id) || VIDEO_PRESETS[3]
}

/**
 * Simulcast 三档：高/中/低。
 * - 高：用户选择的分辨率
 * - 中：min(原尺寸, 720p)
 * - 低：360p（弱网兜底）
 */
export function buildSimulcastLayers(preset: VideoPreset) {
  return [
    { rid: 'h', width: preset.width, height: preset.height, bitrate: preset.bitrate, fps: preset.fps },
    {
      rid: 'm',
      width: Math.min(preset.width, 1280),
      height: Math.min(preset.height, 720),
      bitrate: 800_000,
      fps: Math.min(preset.fps, 30),
    },
    { rid: 'l', width: 640, height: 360, bitrate: 200_000, fps: 15 },
  ]
}
