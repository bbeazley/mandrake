#!/bin/bash
set -u
conf() { if [ -f "/config/$1" ]; then echo "/config/$1"; else echo "/etc/myth2/$1"; fi; }
SERVER_CONF=$(conf server.toml)
PROXY_CONF=$(conf proxy.toml)
if [ "${M2_SITE_DATA:-0}" = 1 ] && ! [ -f /config/proxy.toml ]; then
  PROXY_CONF=/etc/myth2/proxy-site.toml
  echo "myth2-entrypoint: M2_SITE_DATA=1: /data/myth2/tags and /data/plugins are served to browsers"
fi
if ! [ -d /data/myth2/tags ]; then
  echo "myth2-entrypoint: no /data/myth2/tags: mount the Myth II folder there (MYTH2_DATA)" >&2
  exit 2
fi
if ! [ -w /films ]; then
  echo "myth2-entrypoint: /films is not writable by uid $(id -u) (mkdir it on the host, owned by that uid)" >&2
  exit 2
fi
LOGS=/logs
if ! [ -w "$LOGS" ]; then
  echo "myth2-entrypoint: $LOGS is not writable by uid $(id -u); logging to stdout only" >&2
  LOGS=""
fi
log() { if [ -n "$LOGS" ]; then tee -a "$LOGS/$1.log"; else cat; fi; }
echo "myth2-entrypoint: mandrake-server $SERVER_CONF ${M2_SERVER_ARGS:-}; mandrake-ws-proxy $PROXY_CONF"
TFL_CONF=""
if [ -d /data/tfl/tags ]; then
  TFL_CONF=$(conf tfl-server.toml)
  echo "myth2-entrypoint: TFL: mandrake-server $TFL_CONF ${M2_TFL_SERVER_ARGS:-}"
else
  echo "myth2-entrypoint: no /data/tfl/tags: no TFL instance (mount TFL's folder there, TFL_DATA)"
fi

if [ -n "$TFL_CONF" ] && ! [ -f /config/proxy.toml ]; then
  {
    cat "$PROXY_CONF"
    if [ "${M2_SITE_TFL:-0}" = 1 ]; then
      echo 'tfl_dir = "/data/tfl"'
      echo "myth2-entrypoint: M2_SITE_TFL=1: /data/tfl/tags is served to browsers" >&2
    fi
    printf '[targets]\ntfl = "127.0.0.1:3454"\n'
  } >/tmp/proxy.toml
  PROXY_CONF=/tmp/proxy.toml
  echo "myth2-entrypoint: TFL: the proxy's /myth2-ws/tfl reaches the TFL instance"
fi

/usr/local/bin/mandrake-server "$SERVER_CONF" ${M2_SERVER_ARGS:-} > >(log server) 2>&1 &
SERVER=$!
/usr/local/bin/mandrake-ws-proxy "$PROXY_CONF" > >(log proxy) 2>&1 &
PROXY=$!
PIDS="$SERVER $PROXY"
TFL=""
if [ -n "$TFL_CONF" ]; then
  /usr/local/bin/mandrake-server "$TFL_CONF" ${M2_TFL_SERVER_ARGS:-} > >(log tfl-server) 2>&1 &
  TFL=$!
  PIDS="$PIDS $TFL"
fi

stop() {
  kill -TERM $PIDS 2>/dev/null
  wait $PIDS 2>/dev/null
}
trap 'echo "myth2-entrypoint: stopping"; stop; exit 0' TERM INT QUIT

wait -n $PIDS
RC=$?
if ! kill -0 "$SERVER" 2>/dev/null; then WHO=mandrake-server
elif [ -n "$TFL" ] && ! kill -0 "$TFL" 2>/dev/null; then WHO="mandrake-server (TFL)"
else WHO=mandrake-ws-proxy; fi
echo "myth2-entrypoint: $WHO exited ($RC); stopping the container"
stop
exit "$RC"
