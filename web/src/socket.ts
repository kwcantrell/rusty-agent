import { parseInbound, type Inbound, type Outbound } from "./wire";
import type { ConnectionStatus } from "./state";

interface Handlers {
  onFrame: (f: Inbound) => void;
  onStatus: (s: ConnectionStatus) => void;
}
interface Opts {
  WebSocketImpl?: typeof WebSocket;
  backoffMs?: number;
}

export function connect(url: string, handlers: Handlers, opts: Opts = {}) {
  const WS = opts.WebSocketImpl ?? WebSocket;
  const baseBackoff = opts.backoffMs ?? 500;
  let ws: WebSocket;
  let closed = false;
  let backoff = baseBackoff;

  const open = () => {
    handlers.onStatus("connecting");
    ws = new WS(url);
    ws.onopen = () => { backoff = baseBackoff; handlers.onStatus("open"); };
    ws.onmessage = (e: MessageEvent) => {
      const f = parseInbound(typeof e.data === "string" ? e.data : "");
      if (f) handlers.onFrame(f);
    };
    ws.onerror = () => handlers.onStatus("error");
    ws.onclose = () => {
      handlers.onStatus("closed");
      if (closed) return;
      setTimeout(open, backoff);
      backoff = Math.min(backoff * 2, 30000);
    };
  };
  open();

  return {
    send(o: Outbound) {
      ws.send(JSON.stringify(o));
    },
    close() {
      closed = true;
      ws.close();
    },
  };
}
