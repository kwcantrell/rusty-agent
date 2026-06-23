import { DurableObject } from "cloudflare:workers";
import type { Env } from "./worker";

type Attachment = { role: "agent" | "browser"; sessionId: string; agentId: string };

/**
 * Per-agent rendezvous Durable Object. Uses the WebSocket Hibernation API so the
 * object can evict from memory while connections stay open. All per-connection
 * state lives in socket attachments; the only durable field is a monotonic event
 * `seq` in DO SQLite (so R2 keys stay collision-free across hibernation). R2 is
 * the event log; D1 holds presence.
 */
export class AgentSession extends DurableObject<Env> {
  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    this.ctx.storage.sql.exec(
      "CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v INTEGER NOT NULL)");
  }

  async fetch(req: Request): Promise<Response> {
    if (req.headers.get("Upgrade") !== "websocket") {
      return new Response("expected websocket", { status: 426 });
    }
    const role: "agent" | "browser" =
      req.headers.get("X-Role") === "agent" ? "agent" : "browser";
    const sessionId = req.headers.get("X-Session-Id") ?? "";
    const agentId = req.headers.get("X-Agent-Id") ?? "";

    const pair = new WebSocketPair();
    const [client, server] = [pair[0], pair[1]];
    this.ctx.acceptWebSocket(server, [role]);
    server.serializeAttachment({ role, sessionId, agentId } satisfies Attachment);

    if (role === "agent") {
      this.ctx.waitUntil(this.setPresence(agentId, true));
      this.broadcast(JSON.stringify({ v: 1, session_id: "", kind: "presence", online: true }));
    } else {
      this.ctx.waitUntil(this.replayFromR2(sessionId, server));
      server.send(JSON.stringify({
        v: 1, session_id: sessionId, kind: "presence",
        online: this.ctx.getWebSockets("agent").length > 0 }));
    }
    return new Response(null, { status: 101, webSocket: client });
  }

  async webSocketMessage(ws: WebSocket, message: string | ArrayBuffer): Promise<void> {
    const text = typeof message === "string" ? message : "";
    if (!text) return;
    const att = ws.deserializeAttachment() as Attachment | null;
    if (att?.role === "agent") {
      // Fan out to browsers.
      for (const b of this.ctx.getWebSockets("browser")) {
        try { b.send(text); } catch { /* socket gone */ }
      }
      // Persist event frames to R2 with a durable monotonic seq.
      try {
        const msg = JSON.parse(text);
        if (msg.kind === "event" && msg.session_id) {
          this.ctx.waitUntil(this.persist(msg.session_id, text));
        }
      } catch { /* ignore non-JSON */ }
    } else {
      // Browser -> daemon.
      const daemon = this.ctx.getWebSockets("agent")[0];
      if (daemon) { try { daemon.send(text); } catch { /* socket gone */ } }
    }
  }

  async webSocketClose(ws: WebSocket): Promise<void> {
    const att = ws.deserializeAttachment() as Attachment | null;
    if (att?.role === "agent") {
      // Only count other OPEN agent sockets; exclude the closing socket and any already-closed ones.
      const OPEN = 1;
      const others = this.ctx.getWebSockets("agent").filter((s) => s !== ws && s.readyState === OPEN);
      if (others.length === 0) {
        this.ctx.waitUntil(this.setPresence(att.agentId, false));
        this.broadcast(JSON.stringify({ v: 1, session_id: "", kind: "presence", online: false }));
      }
    }
    // No manual ws.close(): web_socket_auto_reply_to_close is on (compat >= 2026-04-07).
  }

  async webSocketError(_ws: WebSocket, error: unknown): Promise<void> {
    console.error(JSON.stringify({
      level: "error", at: "AgentSession.webSocketError",
      message: error instanceof Error ? error.message : String(error) }));
  }

  private broadcast(frame: string): void {
    for (const b of this.ctx.getWebSockets("browser")) {
      try { b.send(frame); } catch { /* socket gone */ }
    }
  }

  /** Returns the current 0-based seq for a session and advances the durable counter. */
  private nextSeq(sessionId: string): number {
    const k = `seq:${sessionId}`;
    const rows = this.ctx.storage.sql
      .exec("SELECT v FROM meta WHERE k=?", k).toArray();
    const cur = rows.length ? Number((rows[0] as { v: number }).v) : 0;
    this.ctx.storage.sql.exec("INSERT OR REPLACE INTO meta (k, v) VALUES (?, ?)", k, cur + 1);
    return cur;
  }

  private async persist(sessionId: string, frame: string): Promise<void> {
    const key = `sessions/${sessionId}/${String(this.nextSeq(sessionId)).padStart(8, "0")}.json`;
    await this.env.LOGS.put(key, frame);
  }

  private async replayFromR2(sessionId: string, ws: WebSocket): Promise<void> {
    let cursor: string | undefined;
    do {
      const list = await this.env.LOGS.list({ prefix: `sessions/${sessionId}/`, cursor });
      const keys = list.objects.map((o) => o.key).sort();
      for (const key of keys) {
        const obj = await this.env.LOGS.get(key);
        if (obj) ws.send(await obj.text());
      }
      cursor = list.truncated ? list.cursor : undefined;
    } while (cursor);
  }

  private async setPresence(agentId: string, online: boolean): Promise<void> {
    if (!agentId) return;
    await this.env.DB.prepare("UPDATE agents SET online = ?, last_seen = ? WHERE id = ?")
      .bind(online ? 1 : 0, Date.now(), agentId).run();
  }
}
