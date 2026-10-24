/**
 * RNNoise 降噪封装。
 *
 * 用法：
 *   const { node, dispose } = await createRNNoiseNode(audioContext)
 *   sourceNode.connect(node).connect(destination)
 *   node.port.postMessage({ type: 'enable', value: true })
 */

export interface RNNoiseNode {
  node: AudioWorkletNode
  setEnabled: (enabled: boolean) => void
  dispose: () => void
}

export async function createRNNoiseNode(ctx: AudioContext): Promise<RNNoiseNode> {
  await ctx.audioWorklet.addModule('/rnnoise-processor.js')
  const node = new AudioWorkletNode(ctx, 'rnnoise-processor', {
    numberOfInputs: 1,
    numberOfOutputs: 1,
    outputChannelCount: [1],
    channelCount: 1,
    channelCountMode: 'explicit',
    channelInterpretation: 'speakers',
  })
  
  return {
    node,
    setEnabled(enabled: boolean) {
      node.port.postMessage({ type: 'enable', value: enabled })
    },
    dispose() {
      node.disconnect()
    },
  }
}
