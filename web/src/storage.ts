import type { Theme } from "./theme";

const TOKEN = "agent.sessionToken";
const SID = "agent.sessionId";
const MSGS = (sid: string) => `agent.userMsgs.${sid}`;
const THEME_KEY = "agent.theme";

export function loadTheme(): Theme | null {
  try {
    const v = localStorage.getItem(THEME_KEY);
    return v === "light" || v === "dark" ? v : null;
  } catch { return null; }
}

export function saveTheme(t: Theme): void {
  try { localStorage.setItem(THEME_KEY, t); } catch { /* ignore */ }
}

export function saveSession(sessionId: string, token: string): void {
  localStorage.setItem(SID, sessionId);
  localStorage.setItem(TOKEN, token);
}
export function loadToken(): string | null {
  try { return localStorage.getItem(TOKEN); } catch { return null; }
}
export function loadSessionId(): string | null {
  try { return localStorage.getItem(SID); } catch { return null; }
}
export function clearSession(): void {
  localStorage.removeItem(TOKEN);
  localStorage.removeItem(SID);
}
export function loadUserMsgs(sessionId: string): string[] {
  try {
    const raw = localStorage.getItem(MSGS(sessionId));
    if (!raw) return [];
    const v = JSON.parse(raw);
    return Array.isArray(v) ? (v as string[]) : [];
  } catch {
    return [];
  }
}
export function appendUserMsg(sessionId: string, text: string): void {
  const arr = loadUserMsgs(sessionId);
  arr.push(text);
  localStorage.setItem(MSGS(sessionId), JSON.stringify(arr));
}

export type WorkspaceView = {
  mode: "preview" | "code";
  viewport: "desktop" | "tablet" | "mobile";
};
const WORKSPACE_VIEW = "agent.workspaceView";
const DEFAULT_VIEW: WorkspaceView = { mode: "preview", viewport: "desktop" };

export function loadWorkspaceView(): WorkspaceView {
  try {
    const raw = localStorage.getItem(WORKSPACE_VIEW);
    if (!raw) return { ...DEFAULT_VIEW };
    const v = JSON.parse(raw) as Partial<WorkspaceView>;
    const mode = v.mode === "code" ? "code" : "preview";
    const viewport = v.viewport === "tablet" || v.viewport === "mobile" ? v.viewport : "desktop";
    return { mode, viewport };
  } catch {
    return { ...DEFAULT_VIEW };
  }
}
export function saveWorkspaceView(v: WorkspaceView): void {
  try { localStorage.setItem(WORKSPACE_VIEW, JSON.stringify(v)); } catch { /* ignore */ }
}

const DASH_EXPANDED = "agent.contextDashExpanded";

export function loadDashExpanded(): boolean {
  try { return localStorage.getItem(DASH_EXPANDED) === "1"; } catch { return false; }
}
export function saveDashExpanded(v: boolean): void {
  try { localStorage.setItem(DASH_EXPANDED, v ? "1" : "0"); } catch { /* ignore */ }
}

const RIGHT_TAB = "rightTab";
export type RightTab = "workspace" | "context" | "design";
export function loadRightTab(): RightTab {
  try {
    const v = localStorage.getItem(RIGHT_TAB);
    return v === "context" || v === "design" ? v : "workspace";
  } catch { return "workspace"; }
}
export function saveRightTab(t: RightTab): void {
  try { localStorage.setItem(RIGHT_TAB, t); } catch { /* ignore */ }
}
