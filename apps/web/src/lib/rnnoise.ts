/**
 * RNNoise 主线程降噪。
 *
 * 为什么不用 AudioWorklet：
 *   @shiguredo/rnnoise-wasm 的 emscripten 产物强制检查 `window` 或 `WorkerGlobalScope`，
 *   AudioWorkletGlobalScope 两者都没有，加载必然失败（实测：会抛
 *   "not compiled for this environment"）。所以走主线程 + MediaStreamTrackProcessor。
 *
 * 主线程处理开销：
 *   RNNoise processFrame 一次 480 samples ≈ 10ms 音频。浏览器 60fps 下每帧处理量
 *   相当于 1% 的 CPU 时间，且 AudioData 读写是异步的（ReadableStream），UI 不会卡。
 *
 * 兼容性：
 *   Chrome 94+ / Edge / Safari 15.4+ 支持 MediaStreamTrackProcessor。
 *   Firefox 需要 flag，实际不可用 → 调用方应检测 isRnnoiseSupported() 并降级为关闭降噪
 *   （注意：绝不降级为浏览器原生 noiseSuppression，避免和 RNNoise 打架）。
 */

export interface RnnoiseHandle {
  track: MediaStreamTrack
  stop: () => Promise<void>
}

export function isRnnoiseSupported(): boolean {
  if (typeof window === 'undefined') return false
  const w = window as any
  return typeof w.MediaStreamTrackProcessor === 'function'
    && typeof w.MediaStreamTrackGenerator === 'function'
    && typeof w.AudioData === 'function'
}

// 全局单例（一个页面只需要一个 Rnnoise 实例；多个 DenoiseState 共享同一 wasm 内存）
let rnnoiseInstance: any = null
let loadPromise: Promise<any> | null = null

async function loadRnnoise(): Promise<any | null> {
  if (rnnoiseInstance) return rnnoiseInstance
  if (loadPromise) return loadPromise
  loadPromise = (async () => {
    try {
      // 动态 import：4.8MB 主包只在首次开麦时加载，不阻塞首屏
      const mod: any = await import('@shiguredo/rnnoise-wasm')
      const Rnnoise = mod.Rnnoise
      if (!Rnnoise) throw new Error('Rnnoise class not exported')
      const r = await Rnnoise.load()
      rnnoiseInstance = r
      console.log('[rnnoise] loaded, frameSize =', r.frameSize)
      return r
    } catch (e) {
      console.error('[rnnoise] load failed:', e)
      return null
    }
  })()
  return loadPromise
}

export async function startRnnoiseProcessing(inputTrack: MediaStreamTrack): Promise<RnnoiseHandle | null> {
  if (!isRnnoiseSupported()) {
    console.warn('[rnnoise] MediaStreamTrackProcessor/Generator not available')
    return null
  }
  const rn = await loadRnnoise()
  if (!rn) return null

  const w = window as any
  const ProcessorCtor = w.MediaStreamTrackProcessor
  const GeneratorCtor = w.MediaStreamTrackGenerator
  const AudioDataCtor = w.AudioData

  const frameSize: number = rn.frameSize
  if (!Number.isFinite(frameSize) || frameSize <= 0) {
    console.error('[rnnoise] invalid frameSize:', frameSize)
    return null
  }

  const processor = new ProcessorCtor({ track: inputTrack })
  const generator = new GeneratorCtor({ kind: 'audio' })
  const state = rn.createDenoiseState()

  const reader = processor.readable.getReader()
  const writer = generator.writable.getWriter()
  console.log('[rnnoise] reader created, input track readyState=' + inputTrack.readyState + ' enabled=' + inputTrack.enabled)

  // 每通道独立的环形缓冲（麦克风基本单声道，多通道留作保险）
  // 输入累积到 frameSize 才处理一次；输出累积到块大小后写出
  const inputAccum = new Map<number, { buf: Float32Array; fill: number }>()
  const outputQueue = new Map<number, { buf: Float32Array[]; read: number }>()

  function getInput(ch: number) {
    let s = inputAccum.get(ch)
    if (!s) { s = { buf: new Float32Array(frameSize), fill: 0 }; inputAccum.set(ch, s) }
    return s
  }
  function getOutput(ch: number) {
    let q = outputQueue.get(ch)
    if (!q) { q = { buf: [], read: 0 }; outputQueue.set(ch, q) }
    return q
  }

  let stopped = false
  let processedFrames = 0
  let firstFrameLogged = false
  let firstReadLogged = false
  let firstResolveLogged = false

  const run = async () => {
    try {
      console.log('[rnnoise] run() started, awaiting first read')
      while (!stopped) {
        if (!firstReadLogged) {
          firstReadLogged = true
          console.log('[rnnoise] awaiting first read')
        }
        const { value, done } = await reader.read()
        if (!firstResolveLogged) {
          firstResolveLogged = true
          console.log('[rnnoise] first reader.read() resolved: done=' + done + ' hasValue=' + !!value + (value ? ' format=' + value.format + ' sampleRate=' + value.sampleRate : ''))
        }
        if (done || !value) {
          console.log('[rnnoise] reader done or no value, exiting loop')
          break
        }
        const audioData: any = value
        try {
          const sampleRate: number = audioData.sampleRate
          const frames: number = audioData.numberOfFrames
          const channels: number = audioData.numberOfChannels
          const format: string = audioData.format || ''

          // RNNoise 必须 48kHz；format 由 copyTo 自动转换，不检查 format
          if (sampleRate !== 48000) {
            if (!stopped) await writer.write(audioData)
            continue
          }

          const outData = new Float32Array(channels * frames)

          for (let ch = 0; ch < channels; ch++) {
            const inPlane = new Float32Array(frames)
            audioData.copyTo(inPlane, { planeIndex: ch, format: 'f32-planar' })

            if (ch !== 0) {
              // RNNoise 是单声道；非主通道原样透传
              outData.set(inPlane, ch * frames)
              continue
            }

            const inp = getInput(ch)
            const outq = getOutput(ch)

            for (let i = 0; i < frames; i++) {
              inp.buf[inp.fill++] = inPlane[i]
              if (inp.fill === frameSize) {
                // 计算输入 RMS（float 范围 [-1, 1]，仅用于诊断日志）
                let inRms = 0
                for (let k = 0; k < frameSize; k++) inRms += inp.buf[k] * inp.buf[k]
                inRms = Math.sqrt(inRms / frameSize)

                // 关键修复：RNNoise 假设输入是 int16-range float（±32768），不是归一化 float（±1）。
                // 见 @shiguredo/rnnoise-wasm 的 DenoiseState.processFrame 文档：
                //   "RNNoise は入力音声フレームが 16ビットPCM であると仮定している"
                // 传归一化值会让 RNNoise 看到极安静的信号，输出被错误衰减。
                const buf = new Float32Array(frameSize)
                for (let k = 0; k < frameSize; k++) buf[k] = inp.buf[k] * 32768

                // processFrame：单参数（就地修改 buf），返回 VAD 概率（0~1）
                let vad = 0
                let rnnoiseOk = false
                try {
                  vad = state.processFrame(buf)
                  rnnoiseOk = true
                } catch (err: any) {
                  console.warn('[rnnoise] processFrame failed:', err)
                }

                // 转回 float [-1, 1] + clamp + NaN 兜底
                const procOut = new Float32Array(frameSize)
                let outRms = 0
                for (let k = 0; k < frameSize; k++) {
                  let v = rnnoiseOk ? buf[k] / 32768 : inp.buf[k]
                  if (!Number.isFinite(v)) v = 0
                  else if (v > 1) v = 1
                  else if (v < -1) v = -1
                  procOut[k] = v
                  outRms += v * v
                }
                outRms = Math.sqrt(outRms / frameSize)

                // 窄兜底：仅在 RNNoise 完全失败（输出全零）且输入有明显信号时透传
                if (inRms > 0.01 && outRms === 0) {
                  for (let k = 0; k < frameSize; k++) procOut[k] = inp.buf[k]
                  outRms = inRms
                }

                outq.buf.push(procOut)
                inp.fill = 0
                processedFrames++

                if (!firstFrameLogged) {
                  firstFrameLogged = true
                  console.log('[rnnoise] first frame: format=' + format + ' sampleRate=' + sampleRate + ' frames=' + frames + ' channels=' + channels + ' inRms=' + inRms.toFixed(4) + ' outRms=' + outRms.toFixed(4) + ' vad=' + (typeof vad !== 'undefined' ? vad.toFixed(2) : '?') + ' ratio=' + (inRms > 0 ? (outRms / inRms).toFixed(2) : 'inf'))
                }
                if (processedFrames % 50 === 0) {
                  console.log('[rnnoise] processed=' + processedFrames + ' inRms=' + inRms.toFixed(4) + ' outRms=' + outRms.toFixed(4) + ' vad=' + (typeof vad !== 'undefined' ? vad.toFixed(2) : '?') + ' ratio=' + (inRms > 0 ? (outRms / inRms).toFixed(2) : 'inf'))
                }
              }
            }

            // 从输出队列取 frames 个样本
            const base = ch * frames
            let written = 0
            while (written < frames && outq.buf.length > 0) {
              const head = outq.buf[0]
              const avail = head.length - outq.read
              const take = Math.min(avail, frames - written)
              outData.set(head.subarray(outq.read, outq.read + take), base + written)
              written += take
              outq.read += take
              if (outq.read === head.length) {
                outq.buf.shift()
                outq.read = 0
              }
            }
            // 【修复】缓冲不足部分使用原始输入数据兜底，防止前期静音或断音
          for (let j = written; j < frames; j++) {
            outData[base + j] = inPlane[j];
          }
          }

          if (stopped) { audioData.close(); break }

          const out = new AudioDataCtor({
            format: 'f32-planar',
            sampleRate,
            numberOfFrames: frames,
            numberOfChannels: channels,
            timestamp: audioData.timestamp,
            data: outData,
          })
          audioData.close()
          await writer.write(out)
        } catch (e) {
          console.error('[rnnoise] frame error (passthrough):', e)
          // 关键：出错时透传原始帧，绝不静音
          try {
            if (!stopped) {
              try { await writer.write(audioData) }
              catch { try { audioData.close() } catch {} }
            } else {
              audioData.close()
            }
          } catch {}
        }
      }
    } catch (e: any) {
      if (!stopped) console.error('[rnnoise] pipeline:', e)
    }
  }

  run().catch((e: any) => console.error('[rnnoise] run() threw:', e))

  // reader timeout 诊断：如果 3 秒内没读到第一帧，打印状态
  setTimeout(() => {
    if (!firstResolveLogged && !stopped) {
      console.warn('[rnnoise] reader timeout warn: 3s 内无帧到达！input track 状态: readyState=' + inputTrack.readyState + ' enabled=' + inputTrack.enabled + ' muted=' + inputTrack.muted)
    }
  }, 3000)

  return {
    track: generator as unknown as MediaStreamTrack,
    stop: async () => {
      stopped = true
      try { await reader.cancel() } catch {}
      try { await writer.close() } catch {}
      try { state.destroy() } catch {}
      console.log('[rnnoise] stopped, processed', processedFrames, 'frames')
    },
  }
}