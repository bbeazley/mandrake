// The page's WebSockets for the game's network play.
// The game (crates/mandrake-client/src/web.rs, `net_ws`) polls these by id once a frame, as it polls a
// native socket: the page only buffers what arrives. The WebSocket carries the game's TCP stream
// unchanged (binary messages, no boundaries), to the dedicated server's proxy (mandrake-ws-proxy).
//
//   m2Net.open(address) -> id   address: ws:// or wss:// URL, or host[:port][/path]: the page's
//                               scheme (wss on https) and the proxy's path /myth2-ws
//   m2Net.state(id)             0 connecting, 1 open, 2 closed
//   m2Net.take(id)              the bytes since the last take (Uint8Array), null if none
//   m2Net.send(id, bytes)       queues a binary message
//   m2Net.close(id), m2Net.url(id), m2Net.why(id)

export const WS_PATH = '/myth2-ws';
const MAX_RECV_BYTES = 8 * 1024 * 1024;

export function wsUrl(address, loc = globalThis.location) {
  const a = String(address).trim();
  if (/^wss?:\/\//i.test(a)) return a;
  const scheme = loc && loc.protocol === 'https:' ? 'wss:' : 'ws:';
  const host = a || (loc ? loc.host : '');
  return scheme + '//' + (host.includes('/') ? host : host + WS_PATH);
}

export function installNet(target = globalThis) {
  const sockets = new Map();
  let next = 1;
  const net = {
    open(address) {
      const id = next++;
      const s = { url: wsUrl(address), chunks: [], buffered: 0, state: 0, why: '', ws: null };
      sockets.set(id, s);
      try {
        s.ws = new WebSocket(s.url);
      } catch (e) {
        s.state = 2;
        s.why = String(e && e.message ? e.message : e);
        return id;
      }
      s.ws.binaryType = 'arraybuffer';
      s.ws.onopen = () => { s.state = 1; console.log('[net] open ' + s.url); };
      s.ws.onmessage = (e) => {
        if (!(e.data instanceof ArrayBuffer)) return;
        if (s.buffered + e.data.byteLength > MAX_RECV_BYTES) {
          s.why = 'receive buffer overflow';
          s.state = 2;
          s.chunks = [];
          s.buffered = 0;
          if (s.ws && s.ws.readyState <= 1) s.ws.close(1009, 'receive buffer overflow');
          return;
        }
        s.chunks.push(new Uint8Array(e.data));
        s.buffered += e.data.byteLength;
      };
      s.ws.onerror = () => { if (!s.why) s.why = 'the WebSocket to ' + s.url + ' failed'; };
      s.ws.onclose = (e) => {
        s.state = 2;
        if (!s.why || e.code !== 1006) s.why = `closed (${e.code}${e.reason ? ' ' + e.reason : ''})`;
        console.log('[net] ' + s.url + ': ' + s.why);
      };
      return id;
    },
    state(id) { const s = sockets.get(id); return s ? s.state : 2; },
    url(id) { const s = sockets.get(id); return s ? s.url : ''; },
    why(id) { const s = sockets.get(id); return s ? s.why : 'no such socket'; },
    take(id) {
      const s = sockets.get(id);
      if (!s || s.chunks.length === 0) return null;
      const out = new Uint8Array(s.buffered);
      let at = 0;
      for (const c of s.chunks) { out.set(c, at); at += c.length; }
      s.chunks = [];
      s.buffered = 0;
      return out;
    },
    send(id, bytes) {
      const s = sockets.get(id);
      if (!s || s.state !== 1) return false;
      // A copy: the view is into the game's memory, which may move.
      s.ws.send(bytes.slice());
      return true;
    },
    close(id) {
      const s = sockets.get(id);
      if (!s) return;
      sockets.delete(id);
      if (s.ws && s.ws.readyState <= 1) s.ws.close(1000);
    },
  };
  target.m2Net = net;
  return net;
}
