const TOKEN = "agent.sessionToken";
const SID = "agent.sessionId";
const MSGS = (sid: string) => `agent.userMsgs.${sid}`;

export function saveSession(sessionId: string, token: string): void {
  localStorage.setItem(SID, sessionId);
  localStorage.setItem(TOKEN, token);
}
export function loadToken(): string | null {
  return localStorage.getItem(TOKEN);
}
export function loadSessionId(): string | null {
  return localStorage.getItem(SID);
}
export function clearSession(): void {
  localStorage.removeItem(TOKEN);
  localStorage.removeItem(SID);
}
export function loadUserMsgs(sessionId: string): string[] {
  const raw = localStorage.getItem(MSGS(sessionId));
  if (!raw) return [];
  try {
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
