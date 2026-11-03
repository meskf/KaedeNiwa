#!/bin/sh
# LiveKit 启动包装脚本。
#
# 背景：LiveKit 的 node_ip 不能通过环境变量（LIVEKIT_RTC_NODE_IP）覆盖，只能写在
# 配置文件里；而纯局域网部署必须把媒体候选 IP 指到服务器 LAN IP，否则客户端拿到的是
# 容器内网 IP / 公网 IP，表现为“进了语音频道但媒体连不通”。
#
# 同时，用 --config 文件时 LIVEKIT_KEYS 会被忽略（启动直接报
# "one of key-file or keys must be provided"），所以配置统一走 LIVEKIT_CONFIG。
set -eu

if [ "${LIVEKIT_NODE_IP:-}" ]; then
    case "$LIVEKIT_NODE_IP" in
        *[!0-9a-fA-F.:]*)
            echo "[livekit] invalid LIVEKIT_NODE_IP: $LIVEKIT_NODE_IP" >&2
            exit 1
            ;;
    esac
    # 指定了固定 IP 就关闭 STUN 探测（两者同时存在时 STUN 结果优先）。
    LIVEKIT_CONFIG=$(printf '%s\n' "$LIVEKIT_CONFIG" \
        | sed "s|^  node_ip:.*|  node_ip: ${LIVEKIT_NODE_IP}|" \
        | sed "s|^  use_external_ip:.*|  use_external_ip: false|")
    export LIVEKIT_CONFIG
fi

exec /livekit-server "$@"
