import { useEffect, useRef } from 'react';
import { getClient, scheduleReconnect, recordLoginArgs, allowReconnect } from '../wasm/client';
import { useAuthStore } from '../stores/auth';
import { useChatStore } from '../stores/chat';

export function useKaedeEvents(currentChannel: string, mediaConnected: boolean, mediaChannelId: string | null) {
  const currentChannelRef = useRef(currentChannel);
  const mediaConnectedRef = useRef(mediaConnected);
  const mediaChannelIdRef = useRef(mediaChannelId);

  // 始终保持 Ref 最新，防止闭包捕获旧状态
  useEffect(() => { currentChannelRef.current = currentChannel; }, [currentChannel]);
  useEffect(() => { mediaConnectedRef.current = mediaConnected; }, [mediaConnected]);
  useEffect(() => { mediaChannelIdRef.current = mediaChannelId; }, [mediaChannelId]);

  useEffect(() => {
    const st = useAuthStore.getState();
    if (st.serverUrl && st.wsUrl && st.loginId && st.token) {
      recordLoginArgs({ serverUrl: st.serverUrl, wsUrl: st.wsUrl, loginId: st.loginId, token: st.token });
      allowReconnect();
    }

    const client = getClient();
    if (!client) return;

    client.setOnEvent((rawEv: any) => {
      let ev = rawEv;
      if (typeof rawEv === 'string') {
        try { ev = JSON.parse(rawEv); } catch (e) { return; }
      }

      if (ev.Disconnected) {
        scheduleReconnect();
        return;
      }

      // 调试事件
      const eventDiv = document.createElement('div');
      eventDiv.setAttribute('data-testid', 'raw-event');
      eventDiv.innerText = 'EVENT: ' + JSON.stringify(ev);
      eventDiv.style.display = 'none';
      document.body.appendChild(eventDiv);

      const store = useChatStore.getState();
      const activeChannel = currentChannelRef.current;

      if (ev.Joined) {
        const peers = ev.Joined.peers || [];
        if (peers.length === 0) {
          try {
            const created = client.ensure_channel(activeChannel);
            if (created) store.setIsAuthority(true);
          } catch (e: any) { console.error('ensure_channel failed:', e); }
        }
      }

      if (ev.ChannelMemberJoined) {
        const { channel_id, requester } = ev.ChannelMemberJoined;
        if (channel_id !== activeChannel || !store.isAuthority) return;
        client.add_member_to_channel(activeChannel, requester).then(() => {
          store.setPeerStatus(`Added ${requester}`);
        }).catch((e: any) => store.setPeerStatus(`Add failed: ${e}`));
      }

      if (ev.PeerJoined) {
        const uid = ev.PeerJoined.uid;
        if (!store.isAuthority) return;
        store.setPeerStatus(`Adding ${uid}...`);
        client.add_member_to_channel(activeChannel, uid).then(() => {
          store.setPeerStatus(`Added ${uid}`);
        }).catch((e: any) => store.setPeerStatus(`Add failed: ${e}`));
      }

      if (ev.ChannelSync) {
        const channelId = ev.ChannelSync.channel_id;
        const payload = new Uint8Array(ev.ChannelSync.payload);
        client.process_channel_sync(channelId, payload).then(() => {
          store.setPeerStatus(`Synced`);
        }).catch((e: any) => store.setPeerStatus(`Sync failed: ${e}`));
      }

      if (ev.MlsEpochChanged && mediaConnectedRef.current && mediaChannelIdRef.current === activeChannel) {
        (async () => {
          try {
            const c = getClient();
            if (!c) return;
            const tokenJson = await c.begin_media_session(activeChannel);
            const tokenData = typeof tokenJson === 'string' ? JSON.parse(tokenJson) : tokenJson;
            if (tokenData.e2ee_seed) {
              const { updateE2EEKey } = await import('../lib/livekit');
              await updateE2EEKey(tokenData.e2ee_seed);
            }
          } catch (e) { console.error('E2EE key rotate failed:', e); }
        })();
      }

      if (ev.ChannelMessage) {
        if (ev.ChannelMessage.channel_id !== activeChannel) return;
        try {
          const plaintext = ev.ChannelMessage.plaintext;
          let text = '';
          if (Array.isArray(plaintext)) {
            text = new TextDecoder().decode(new Uint8Array(plaintext));
          } else if (typeof plaintext === 'string') {
            text = plaintext;
          }
          store.addMessage(activeChannel, {
            sender: ev.ChannelMessage.from || 'Unknown',
            plaintext: text,
            ts: ev.ChannelMessage.ts || Date.now()
          });
        } catch (err) { console.error('Parse message failed:', err); }
        return;
      }

      if (ev.AppMessageReceived) {
        if (ev.AppMessageReceived.channel_id !== activeChannel) return;
        try {
          const text = new TextDecoder().decode(new Uint8Array(ev.AppMessageReceived.payload));
          store.addMessage(activeChannel, {
            sender: ev.AppMessageReceived.from || 'Unknown',
            plaintext: text,
            ts: Date.now()
          });
        } catch (err) { console.error('Parse message failed:', err); }
      }
    });
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []); // 空依赖数组：只绑定一次，避免内存泄漏
}
