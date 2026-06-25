export interface Transport {
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

/// The local conversation id — a client-side localStorage history key only; it
/// never crosses to the backend (IPC has no session correlation).
export async function resolveTransport(): Promise<Transport> {
  return { sessionId: localSessionId() };
}
