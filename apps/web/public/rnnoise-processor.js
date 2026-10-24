// RNNoise AudioWorklet Processor
// 输入：128 样本/帧（AudioWorklet 标准块大小）
// RNNoise：480 样本/帧
// 需要缓冲：累积到 480 才处理一次

import { createRNNWasmModule } from '/rnnoise/rnnoise.js'

const RNNOISE_FRAME_SIZE = 480
const AUDIO_WORKLET_FRAME_SIZE = 128

class RNNoiseProcessor extends AudioWorkletProcessor {
  constructor() {
    super()
    this.ready = false
    this.enabled = true
    this.wasm = null
    this.state = 0
    this.inBuffer = new Float32Array(RNNOISE_FRAME_SIZE)
    this.outBuffer = new Float32Array(RNNOISE_FRAME_SIZE)
    this.outQueue = [] // 已处理的帧，等待消耗
    this.inPtr = 0
    this.outPtr = 0
    this.inAccum = new Float32Array(RNNOISE_FRAME_SIZE)
    this.inAccumFill = 0
    this.outAccum = []
    this.outConsumeIdx = 0
    
    this.port.onmessage = (e) => {
      if (e.data.type === 'enable') {
        this.enabled = e.data.value
      }
    }
    
    createRNNWasmModule().then((wasm) => {
      this.wasm = wasm
      // 分配 WASM 内存
      this.inPtr = wasm._malloc(RNNOISE_FRAME_SIZE * 4)
      this.outPtr = wasm._malloc(RNNOISE_FRAME_SIZE * 4)
      this.state = wasm.rnnoise_create(0)
      this.ready = true
      this.port.postMessage({ type: 'ready' })
    }).catch((err) => {
      this.port.postMessage({ type: 'error', message: String(err) })
    })
  }

  process(inputs, outputs) {
    const input = inputs[0]
    const output = outputs[0]
    if (!input || !input[0]) {
      // 无输入，透传
      return true
    }
    const inChannel = input[0]
    const outChannel = output[0]
    
    // 未 ready 或未启用：直接透传
    if (!this.ready || !this.enabled) {
      outChannel.set(inChannel)
      return true
    }
    
    // 累积到 RNNOISE_FRAME_SIZE，逐块处理
    for (let i = 0; i < inChannel.length; i++) {
      this.inAccum[this.inAccumFill++] = inChannel[i]
      
      // 输入缓冲满了
      if (this.inAccumFill === RNNOISE_FRAME_SIZE) {
        // 复制到 WASM 内存
        this.wasm.HEAPF32.set(this.inAccum, this.inPtr / 4)
        // 调用 RNNoise
        this.wasm.rnnoise_process_frame(this.state, this.outPtr, this.inPtr)
        // 复制出来
        const outFrame = new Float32Array(RNNOISE_FRAME_SIZE)
        outFrame.set(this.wasm.HEAPF32.subarray(this.outPtr / 4, this.outPtr / 4 + RNNOISE_FRAME_SIZE))
        this.outAccum.push(outFrame)
        this.inAccumFill = 0
      }
    }
    
    // 从 outAccum 输出
    let outIdx = 0
    while (outIdx < outChannel.length && this.outAccum.length > 0) {
      const frame = this.outAccum[0]
      const remaining = frame.length - this.outConsumeIdx
      const needed = outChannel.length - outIdx
      const take = Math.min(remaining, needed)
      outChannel.set(frame.subarray(this.outConsumeIdx, this.outConsumeIdx + take), outIdx)
      outIdx += take
      this.outConsumeIdx += take
      if (this.outConsumeIdx === frame.length) {
        this.outAccum.shift()
        this.outConsumeIdx = 0
      }
    }
    // 剩余填 0
    for (; outIdx < outChannel.length; outIdx++) {
      outChannel[outIdx] = 0
    }
    
    return true
  }
}

registerProcessor('rnnoise-processor', RNNoiseProcessor)
