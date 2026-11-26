#!/bin/sh
# Starts ocw-server on a Fly.io machine (spec 11.5; README "Hosting the
# server on Fly.io"). Runs as root only to prepare the volume, then as the
# user `ocw`.
#
# Fly delivers UDP only to the address `fly-global-services` and its replies
# must leave from there, so the game (QUIC), WebTransport and the voice
# server's media port bind that address; the public address riders use is
# OCW_PUBLIC_HOST (default: the IPv4 of APP.fly.dev, which is the app's
# dedicated IPv4 once it is allocated).
#
# The volume (/data):
#   worlds/   the world packages with their manifests and routes (uploaded,
#             README); the server rides OCW_WORLD and may switch to the
#             others next to it
#   tls/      the server's own TLS identity (QUIC certificate, WebTransport
#             certificate; made on the first start, kept across restarts)
#   home/     the server user's data dir (voice server logs, trusted keys)
#   web-info.json  the WebTransport URL and certificate hash for pages
#
# Pages from the internet (the public preview on GitHub Pages) fetch the
# same, current, from https://APP.fly.dev/join.json (spec 11.5 *Join info*):
# Fly's proxy ends TLS on 443 and forwards to the join endpoint on 8080,
# which answers /join.json and passes everything else (the voice server's
# WebSocket) on to LiveKit's signalling port.
set -eu

world=${OCW_WORLD:-/data/worlds/pilot/liechtenstein.ocwpkg}

mkdir -p /data/worlds /data/tls /data/home
chown ocw:ocw /data /data/tls /data/home
chmod 0700 /data/tls

# UDP on Fly: the address of fly-global-services (IPv4).
udp_ip=$(getent ahostsv4 fly-global-services | awk 'NR == 1 { print $1 }')
if [ -z "$udp_ip" ]; then
  echo "fly-global-services does not resolve: not on Fly? Using 0.0.0.0" >&2
  udp_ip=0.0.0.0
fi

# The public IPv4 riders reach (the voice server announces it for media,
# the grants and the join link name it).
public=${OCW_PUBLIC_HOST:-}
if [ -z "$public" ] && [ -n "${FLY_APP_NAME:-}" ]; then
  public=$(getent ahostsv4 "$FLY_APP_NAME.fly.dev" | awk 'NR == 1 { print $1 }')
fi
if [ -z "$public" ]; then
  echo "no public IPv4: allocate one (fly ips allocate-v4) or set OCW_PUBLIC_HOST" >&2
  exit 1
fi
echo "public address $public, UDP on $udp_ip" >&2

# Voice for riders from the internet over TLS (spec 11.5): Fly's proxy ends
# TLS on 443 for APP.fly.dev (a Fly certificate) and forwards to LiveKit's
# signalling port, so pages served over https:// can connect (wss://).
# OCW_VOICE_PUBLIC_URL="" keeps ws://IPV4:7880 for everyone.
voice_url=${OCW_VOICE_PUBLIC_URL-${FLY_APP_NAME:+wss://$FLY_APP_NAME.fly.dev}}

# Wait for the world package (a new volume is empty until it is uploaded:
# the machine keeps running, so `fly ssh sftp` can reach it).
while [ ! -f "$world" ]; do
  echo "waiting for the world package $world (README: Hosting the server on Fly.io)" >&2
  sleep 30
done

# shellcheck disable=SC2086  # OCW_SERVER_ARGS is a list of options
exec setpriv --reuid=ocw --regid=ocw --clear-groups --reset-env -- \
  env HOME=/data/home PATH=/usr/local/bin:/usr/bin:/bin RUST_LOG="${RUST_LOG:-info}" NO_COLOR=1 \
  /usr/local/bin/ocw-server \
    --listen "$udp_ip:47800" \
    --webtransport "$udp_ip:47801" \
    --tls-dir /data/tls \
    --web-info-file /data/web-info.json \
    --join-http 0.0.0.0:8080 \
    --world "$world" \
    --voice-public-host "$public" \
    --voice-ports 7880,7881,7882 \
    --no-gso --no-mtu-discovery \
    ${voice_url:+--voice-public-url "$voice_url"} \
    ${OCW_SERVER_ARGS:-}
