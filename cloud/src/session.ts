import type { Env } from "./worker";

export class AgentSession {
  private state: DurableObjectState;
  private env: Env;
  private daemon: WebSocket | null = null;
  private browsers = new Set<WebSocket>();
  private agentId: string | null = null;
  private seq = 0;
  // Recent events buffered for fast browser-reconnect replay (per session id).
  private buffer = new Map<string, string[]>();

  constructor(state: DurableObjectState, env: Env) {
    this.state = state;
    this.env = env;
  }

  async fetch(req: Request): Promise<Response> {
    if (req.headers.get("Upgrade") !== "websocket") {
      return new Response("expected websocket", { status: 426 });
    }
    const role = req.headers.get("X-Role");
    const pair = new WebSocketPair();
    const [client, server] = [pair[0], pair[1]];
    server.accept();
    if (role === "agent") {
      this.attachDaemon(server, req.headers.get("X-Agent-Id"));
    } else {
      this.attachBrowser(server, req.headers.get("X-Session-Id") ?? "");
    }
    return new Response(null, { status: 101, webSocket: client });
  }

  private attachDaemon(ws: WebSocket, agentId: string | null) {
    this.daemon = ws;
    this.agentId = agentId;
    this.state.waitUntil(this.setPresence(true));
    this.broadcast(JSON.stringify({ v: 1, session_id: "", kind: "presence", online: true }));
    ws.addEventListener("message", (ev) => {
      const text = typeof ev.data === "string" ? ev.data : "";
      if (!text) return;
      // Fan out to browsers.
      this.broadcast(text);
      // Persist event frames to R2 + buffer for replay.
      try {
        const msg = JSON.parse(text);
        if (msg.kind === "event" && msg.session_id) {
          this.bufferEvent(msg.session_id, text);
          this.state.waitUntil(this.persist(msg.session_id, text));
        }
      } catch { /* ignore non-JSON */ }
    });
    ws.addEventListener("close", () => {
      this.daemon = null;
      this.state.waitUntil(this.setPresence(false));
      this.broadcast(JSON.stringify({ v: 1, session_id: "", kind: "presence", online: false }));
    });
  }

  private async replayFromR2(sessionId: string, ws: WebSocket) {
    const list = await this.env.LOGS.list({ prefix: `sessions/${sessionId}/` });
    const keys = list.objects.map((o) => o.key).sort();
    for (const key of keys) {
      const obj = await this.env.LOGS.get(key);
      if (obj) ws.send(await obj.text());
    }
  }

  private attachBrowser(ws: WebSocket, sessionId: string) {
    this.browsers.add(ws);
    // Replay buffered events for this session, falling back to R2 when the buffer is empty.
    const buffered = this.buffer.get(sessionId);
    if (buffered && buffered.length > 0) {
      for (const frame of buffered) ws.send(frame);
    } else {
      void this.replayFromR2(sessionId, ws);
    }
    ws.send(JSON.stringify({ v: 1, session_id: sessionId, kind: "presence",
      online: this.daemon !== null }));
    ws.addEventListener("message", (ev) => {
      const text = typeof ev.data === "string" ? ev.data : "";
      if (text && this.daemon) this.daemon.send(text);
    });
    ws.addEventListener("close", () => this.browsers.delete(ws));
  }

  private broadcast(frame: string) {
    for (const b of this.browsers) {
      try { b.send(frame); } catch { this.browsers.delete(b); }
    }
  }

  private bufferEvent(sessionId: string, frame: string) {
    const arr = this.buffer.get(sessionId) ?? [];
    arr.push(frame);
    if (arr.length > 500) arr.shift(); // bound the in-memory replay buffer
    this.buffer.set(sessionId, arr);
  }

  private async persist(sessionId: string, frame: string) {
    const key = `sessions/${sessionId}/${String(this.seq++).padStart(8, "0")}.json`;
    await this.env.LOGS.put(key, frame);
  }

  private async setPresence(online: boolean) {
    if (!this.agentId) return;
    await this.env.DB.prepare("UPDATE agents SET online = ?, last_seen = ? WHERE id = ?")
      .bind(online ? 1 : 0, Date.now(), this.agentId).run();
  }
}
