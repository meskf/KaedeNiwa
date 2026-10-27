export interface ExtractedMessage {
  text: string
  attachments: any[]
}

export function extractMessage(raw: any): ExtractedMessage {
  let text = ''
  let attachments: any[] = []

  if (Array.isArray(raw)) {
    text = new TextDecoder().decode(new Uint8Array(raw))
  } else if (typeof raw === 'string') {
    text = raw
  } else if (raw && typeof raw === 'object') {
    return {
      text: typeof raw.t === 'string' ? raw.t : (raw.text || ''),
      attachments: raw.a || raw.attachments || []
    }
  } else {
    return { text: '', attachments: [] }
  }

  // 如果是 JSON 字符串，尝试解析出 t 和 a
  if (text.startsWith('{') && text.endsWith('}')) {
    try {
      const parsed = JSON.parse(text)
      if (parsed && typeof parsed.t === 'string') {
        return {
          text: parsed.t,
          attachments: parsed.a || []
        }
      }
    } catch {
      // ignore
    }
  }
  return { text, attachments }
}