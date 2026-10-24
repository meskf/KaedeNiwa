import { useEffect, useState } from 'react'
import { MediaState, subscribeMedia } from '../lib/livekit'

const INITIAL: MediaState = {
  connected: false,
  channelId: null,
  participants: [],
  screenShares: [],
  localScreenSharing: false,
  localMicEnabled: false,
  localNoiseSuppression: true,
  currentPresetId: '1080p60',
  e2eeEnabled: false,
}

export function useMediaState(): MediaState {
  const [state, setState] = useState<MediaState>(INITIAL)
  useEffect(() => {
    const unsub = subscribeMedia(setState)
    return unsub
  }, [])
  return state
}
