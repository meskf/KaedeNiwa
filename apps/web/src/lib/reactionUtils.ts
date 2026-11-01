// ReactionPayload 解码（ReactionPayload 的字段定义见 crates/kaede-client/src/payload.rs）
// 公开频道 → AppMessageReceived.payload（number[] 或 string）
// 私密频道 → ChannelReaction.plaintext（number[] 或 string）
export interface DecodedReaction {
  c: string
  u: string
  target_id: number
  target_ts: number
  emoji: string
  action: 'add' | 'remove'
}

export function tryDecodeReactionPayload(raw: any): DecodedReaction | null {
  if (raw == null) return null

  let text: string | null = null
  if (typeof raw === 'string') {
    text = raw
  } else if (Array.isArray(raw)) {
    try { text = new TextDecoder().decode(new Uint8Array(raw)) } catch { return null }
  } else if (raw instanceof Uint8Array) {
    try { text = new TextDecoder().decode(raw) } catch { return null }
  } else if (typeof raw === 'object') {
    const o: any = raw
    if (typeof o.target_ts === 'number' && typeof o.emoji === 'string'
        && (o.action === 'add' || o.action === 'remove')) {
      return { c: o.c || '', u: o.u || '', target_id: typeof o.target_id === 'number' ? o.target_id : 0, target_ts: o.target_ts, emoji: o.emoji, action: o.action }
    }
    return null
  }

  if (!text || !text.startsWith('{')) return null
  try {
    const o = JSON.parse(text)
    if (typeof o.target_ts !== 'number') return null
    if (typeof o.emoji !== 'string' || !o.emoji) return null
    if (o.action !== 'add' && o.action !== 'remove') return null
    return { c: o.c || '', u: o.u || '', target_id: typeof o.target_id === 'number' ? o.target_id : 0, target_ts: o.target_ts, emoji: o.emoji, action: o.action }
  } catch {
    return null
  }
}
