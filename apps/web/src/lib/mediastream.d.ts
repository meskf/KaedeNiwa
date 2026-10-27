// W3C 草案 API：MediaStreamTrackProcessor / Generator
// TS 5.x lib.dom.d.ts 尚未收录，手动声明。
// Chrome 94+ / Edge / Safari 15.4+ 支持；Firefox 需 flag。

type AudioSampleFormat =
  | 'u8' | 's16' | 's32' | 'f32'
  | 'u8-planar' | 's16-planar' | 's32-planar' | 'f32-planar'

interface AudioDataInit {
  format: AudioSampleFormat
  sampleRate: number
  numberOfFrames: number
  numberOfChannels: number
  timestamp: number
  data: BufferSource
}

interface AudioDataCopyToOptions {
  planeIndex: number
  frameCount?: number
  format?: AudioSampleFormat
}

declare class AudioData {
  constructor(init: AudioDataInit)
  readonly format: AudioSampleFormat | null
  readonly sampleRate: number
  readonly numberOfFrames: number
  readonly numberOfChannels: number
  readonly duration: number
  readonly timestamp: number
  allocationSize(options: { planeIndex: number }): number
  copyTo(destination: BufferSource, options: AudioDataCopyToOptions): void
  close(): void
}

interface MediaStreamTrackProcessorInit {
  track: MediaStreamTrack
  maxBufferSize?: number
}

declare class MediaStreamTrackProcessor<T = AudioData | VideoFrame> {
  constructor(init: MediaStreamTrackProcessorInit)
  readonly readable: ReadableStream<T>
}

interface MediaStreamTrackGeneratorInit {
  kind: 'audio' | 'video'
}

declare class MediaStreamTrackGenerator<T = AudioData | VideoFrame> {
  constructor(init: MediaStreamTrackGeneratorInit)
  readonly kind: 'audio' | 'video'
  readonly writable: WritableStream<T>
  readonly id: string
  readonly label: string
  enabled: boolean
  readonly muted: boolean
  readonly readyState: MediaStreamTrackState
  stop(): void
}