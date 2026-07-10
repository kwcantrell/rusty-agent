import { invoke, Channel } from "@tauri-apps/api/core";
import type { Inbound, Outbound, WireEvent } from "./wire";
import type { ConnectionStatus } from "./state";

interface Handlers {
  onFrame: (f: Inbound) => void;
  onStatus: (s: ConnectionStatus) => void;
}

// A ServerEvent is the legacy WireEvent shape plus the events lifted into
// their own frame kinds (`approval_request`, `parked_runs`, `approval_resolved`,
// `resumed`).
type ServerEvent =
  | WireEvent
  | { type: "approval_request"; id: string; summary: string; command?: string; display?: unknown;
      origin?: import("./wire").ApprovalOrigin }
  | { type: "parked_runs"; runs: import("./wire").ParkedRun[] }
  | { type: "approval_resolved"; id: string }
  | { type: "resumed"; session_id: string };

function toInbound(ev: ServerEvent): Inbound {
  if (ev.type === "approval_request") {
    return { v: 1, session_id: "", id: ev.id, kind: "approval_request",
      summary: ev.summary, command: ev.command, display: ev.display as never, origin: ev.origin };
  }
  if (ev.type === "parked_runs") {
    return { v: 1, session_id: "", kind: "parked_runs", runs: ev.runs };
  }
  if (ev.type === "approval_resolved") {
    return { v: 1, session_id: "", kind: "approval_resolved", id: ev.id };
  }
  if (ev.type === "resumed") {
    return { v: 1, session_id: "", kind: "resumed", resumed_session_id: ev.session_id };
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
