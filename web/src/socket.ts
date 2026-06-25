import { invoke, Channel } from "@tauri-apps/api/core";
import type { Inbound, Outbound, WireEvent } from "./wire";
import type { ConnectionStatus } from "./state";

interface Handlers {
  onFrame: (f: Inbound) => void;
  onStatus: (s: ConnectionStatus) => void;
}

// A ServerEvent is the legacy WireEvent shape plus an `approval_request` case.
type ServerEvent =
  | WireEvent
  | { type: "approval_request"; id: string; summary: string; command?: string; display?: unknown };

function toInbound(ev: ServerEvent): Inbound {
  if (ev.type === "approval_request") {
    return { v: 1, session_id: "", id: ev.id, kind: "approval_request",
      summary: ev.summary, command: ev.command, display: ev.display as never };
  }
  return { v: 1, session_id: "", kind: "event", payload: ev };
}

/**
 * Tauri IPC transport behind the legacy `connect()` seam. Outbound streaming
 * arrives over one `Channel`; inbound frames are command invocations. "online"
 * now means "subscribed to the local backend" (there is no remote presence).
 */
export function connect(handlers: Handlers, _opts: Record<string, never> = {}) {
  handlers.onStatus("connecting");
  const channel = new Channel<ServerEvent>();
  channel.onmessage = (ev) => handlers.onFrame(toInbound(ev));
  invoke("subscribe", { channel })
    .then(() => {
      handlers.onStatus("open");
      handlers.onFrame({ v: 1, session_id: "", kind: "presence", online: true });
    })
    .catch(() => handlers.onStatus("error"));

  return {
    send(o: Outbound) {
      switch (o.kind) {
        case "user_input":
          invoke("send_input", { text: o.text }).catch(() => {});
          break;
        case "approval_response":
          invoke("approve", { id: o.id, decision: o.decision }).catch(() => {});
          break;
        case "settings_get":
          invoke("settings_get")
            .then((st) => handlers.onFrame(
              { v: 1, session_id: "", kind: "settings_state", ...(st as object) } as Inbound))
            .catch(() => {});
          break;
        case "settings_update":
          invoke("settings_update", { settings: o.settings })
            .then((st) => handlers.onFrame(
              { v: 1, session_id: "", kind: "settings_state", ...(st as object) } as Inbound))
            .catch((e) => handlers.onFrame(
              { v: 1, session_id: "", kind: "settings_error", message: String(e) } as Inbound));
          break;
      }
    },
    close() {
      /* IPC has no socket to close; the Channel is GC'd with the component. */
    },
  };
}
