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
  /** 建议码率（bps），根据 kaede-protocol 的 0.032 bpp 经验系数 */
  bitrate: number
}

// 根据 0.032 bpp × 1.0（Standard 档）计算，与 kaede-protocol 一致
function calcBitrate(width: number, height: number, fps: number): number {
  return Math.round(width * height * fps * 0.032)
}

export const VIDEO_PRESETS: VideoPreset[] = [
  { id: '4k60',   label: '3840x2160 60fps',      width: 3840, height: 2160, fps: 60, bitrate: calcBitrate(3840, 2160, 60) },
  { id: '4k30',   label: '3840x2160 30fps',      width: 3840, height: 2160, fps: 30, bitrate: calcBitrate(3840, 2160, 30) },
  { id: '1440p60',label: '2560x1440 60fps',   width: 2560, height: 1440, fps: 60, bitrate: calcBitrate(2560, 1440, 60) },
  { id: '1080p60',label: '1920x1080 60fps',   width: 1920, height: 1080, fps: 60, bitrate: calcBitrate(1920, 1080, 60) },
  { id: '1080p30',label: '1920x1080 30fps',   width: 1920, height: 1080, fps: 30, bitrate: calcBitrate(1920, 1080, 30) },
  { id: '720p30', label: '1280x720 30fps',    width: 1280, height: 720,  fps: 30, bitrate: calcBitrate(1280, 720, 30) },
  { id: '480p30', label: '854x480 30fps', width: 854, height: 480, fps: 30, bitrate: calcBitrate(854, 480, 30) },
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
