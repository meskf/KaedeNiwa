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
  noiseMode: 'off',
  currentPresetId: '1080p60',
  e2eeEnabled: false,
  deafened: false,
  inputVolume: 100,
  outputVolume: 100,
  micDeviceId: null,
  speakerDeviceId: null,
  pushToTalk: false,
  pushToTalkKey: 'Space',
  audioBitrate: 0,
}

export function useMediaState(): MediaState {
  const [state, setState] = useState<MediaState>(INITIAL)
  useEffect(() => {
    const unsub = subscribeMedia(setState)
    return unsub
  }, [])
  return state
}
