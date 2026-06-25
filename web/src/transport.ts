import { invoke } from "@tauri-apps/api/core";

export interface Transport {
  wsUrl: string;
  sessionId: string;
}

// Tauri v2 with withGlobalTauri=false still injects __TAURI_INTERNALS__.
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

const SESSION_KEY = "local_session_id";

function localSessionId(): string {
  let id = localStorage.getItem(SESSION_KEY);
  if (!id) {
    id = crypto.randomUUID();
    localStorage.setItem(SESSION_KEY, id);
  }
  return id;
}

export async function resolveTransport(): Promise<Transport> {
  if (isTauri()) {
    const wsUrl = await invoke<string>("get_local_ws_url");
    return { wsUrl, sessionId: localSessionId() };
  }
  return { wsUrl: "", sessionId: "" };
}
