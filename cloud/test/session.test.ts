import { env, createExecutionContext, waitOnExecutionContext, runInDurableObject } from "cloudflare:test";
import { describe, it, expect, beforeAll } from "vitest";
import worker from "../src/worker";
import { AgentSession } from "../src/session";
import { sha256hex, newToken } from "../src/util";

async function seed() {
  const sql = `
    CREATE TABLE IF NOT EXISTS agents (id TEXT PRIMARY KEY, name TEXT NOT NULL,
      token_hash TEXT NOT NULL UNIQUE, user_id TEXT, pairing_code TEXT NOT NULL,
      last_seen INTEGER, online INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS sessions (id TEXT PRIMARY KEY, agent_id TEXT NOT NULL,
      token_hash TEXT NOT NULL UNIQUE, status TEXT NOT NULL DEFAULT 'active',
      created_at INTEGER NOT NULL);`;
  for (const s of sql.split(";").map((x) => x.trim()).filter(Boolean)) {
    await env.DB.prepare(s).run();
  }
  const agentTok = newToken();
  const sessTok = newToken();
  const r2OnlyTok = newToken();
  await env.DB.prepare(
    "INSERT INTO agents (id,name,token_hash,pairing_code,online,created_at) VALUES (?,?,?,?,0,?)")
    .bind("agent-1", "a", await sha256hex(agentTok), "111111", Date.now()).run();
  await env.DB.prepare(
    "INSERT INTO sessions (id,agent_id,token_hash,status,created_at) VALUES (?,?,?,'active',?)")
    .bind("sess-1", "agent-1", await sha256hex(sessTok), Date.now()).run();
  await env.DB.prepare(
    "INSERT INTO sessions (id,agent_id,token_hash,status,created_at) VALUES (?,?,?,'active',?)")
    .bind("sess-r2-only", "agent-1", await sha256hex(r2OnlyTok), Date.now()).run();
  return { agentTok, sessTok, r2OnlyTok };
}

function wsReq(path: string, headers: Record<string, string> = {}) {
  return new Request(`http://x${path}`, { headers: { Upgrade: "websocket", ...headers } });
}

describe("relay", () => {
  let toks: { agentTok: string; sessTok: string; r2OnlyTok: string };
  beforeAll(async () => { toks = await seed(); });

  it("relays a daemon event to a connected browser and flips presence", async () => {
    const ctx = createExecutionContext();
    // Daemon connects.
    const agentRes = await worker.fetch(
      wsReq("/agent", { Authorization: `Bearer ${toks.agentTok}` }), env, ctx);
    expect(agentRes.status).toBe(101);
    const daemonWs = agentRes.webSocket!;
    daemonWs.accept();

    // Browser connects.
    const browserRes = await worker.fetch(
      wsReq(`/browser?token=${toks.sessTok}`), env, ctx);
    expect(browserRes.status).toBe(101);
    const browserWs = browserRes.webSocket!;
    const received: string[] = [];
    browserWs.addEventListener("message", (e) => received.push(e.data as string));
    browserWs.accept();

    // Daemon emits an event; expect the browser to receive it.
    daemonWs.send(JSON.stringify({
      v: 1, session_id: "sess-1", kind: "event",
      payload: { type: "token", text: "hi" },
    }));

    await new Promise((r) => setTimeout(r, 50));
    await waitOnExecutionContext(ctx);

    expect(received.some((m) => m.includes("\"token\"") && m.includes("hi"))).toBe(true);
    const row = await env.DB.prepare("SELECT online FROM agents WHERE id='agent-1'")
      .first<{ online: number }>();
    expect(row?.online).toBe(1);
    // Clean up open sockets so subsequent tests start with a clean presence state.
    daemonWs.close(1000, "done");
    browserWs.close(1000, "done");
    await new Promise((r) => setTimeout(r, 30));
  });

  it("replays the event log from R2 to a freshly attached browser", async () => {
    // Pre-seed R2 under a session that has NO live buffer entry, so the DO must read R2.
    await env.LOGS.put("sessions/sess-r2-only/00000000.json", JSON.stringify({
      v: 1, session_id: "sess-r2-only", kind: "event", payload: { type: "token", text: "one" } }));
    await env.LOGS.put("sessions/sess-r2-only/00000001.json", JSON.stringify({
      v: 1, session_id: "sess-r2-only", kind: "event", payload: { type: "token", text: "two" } }));

    // sess-r2-only has no buffer entry, so attachBrowser falls through to replayFromR2.
    const ctx = createExecutionContext();
    const browserRes = await worker.fetch(
      wsReq(`/browser?token=${toks.r2OnlyTok}`), env, ctx);
    const browserWs = browserRes.webSocket!;
    const received: string[] = [];
    browserWs.addEventListener("message", (e) => received.push(e.data as string));
    browserWs.accept();

    await new Promise((r) => setTimeout(r, 80));
    await waitOnExecutionContext(ctx);

    const joined = received.join("\n");
    expect(joined).toContain("one");
    expect(joined).toContain("two");
  });

  it("persists a durable monotonic seq in DO SQLite and writes R2 keys in order", async () => {
    const ctx = createExecutionContext();
    const agentRes = await worker.fetch(
      wsReq("/agent", { Authorization: `Bearer ${toks.agentTok}` }), env, ctx);
    const daemonWs = agentRes.webSocket!;
    daemonWs.accept();

    for (const text of ["alpha", "beta"]) {
      daemonWs.send(JSON.stringify({
        v: 1, session_id: "sess-seq", kind: "event", payload: { type: "token", text } }));
    }
    await new Promise((r) => setTimeout(r, 80));
    await waitOnExecutionContext(ctx);

    // R2 keys are zero-padded and monotonic for this session.
    const list = await env.LOGS.list({ prefix: "sessions/sess-seq/" });
    const keys = list.objects.map((o: R2Object) => o.key).sort();
    expect(keys).toEqual([
      "sessions/sess-seq/00000000.json",
      "sessions/sess-seq/00000001.json",
    ]);

    // The seq survives in DO SQLite (value = next seq to hand out = 2).
    const id = env.AGENT.idFromName("agent-1");
    const stub = env.AGENT.get(id);
    const stored = await runInDurableObject(stub, async (_instance: AgentSession, state: DurableObjectState) => {
      const rows = state.storage.sql.exec("SELECT v FROM meta WHERE k='seq:sess-seq'").toArray();
      return rows.length ? Number((rows[0] as { v: number }).v) : 0;
    });
    expect(stored).toBe(2);
    // Clean up: close the daemon socket so the next test sees a fresh presence state.
    daemonWs.close(1000, "done");
    await new Promise((r) => setTimeout(r, 30));
  });

  it("flips presence offline when the daemon socket closes", async () => {
    const ctx = createExecutionContext();
    const agentRes = await worker.fetch(
      wsReq("/agent", { Authorization: `Bearer ${toks.agentTok}` }), env, ctx);
    const daemonWs = agentRes.webSocket!;
    daemonWs.accept();

    const browserRes = await worker.fetch(
      wsReq(`/browser?token=${toks.sessTok}`), env, ctx);
    const browserWs = browserRes.webSocket!;
    const received: string[] = [];
    browserWs.addEventListener("message", (e) => received.push(e.data as string));
    browserWs.accept();
    await new Promise((r) => setTimeout(r, 30));

    // Daemon goes away.
    daemonWs.close(1000, "bye");
    await new Promise((r) => setTimeout(r, 50));
    await waitOnExecutionContext(ctx);

    const row = await env.DB.prepare("SELECT online FROM agents WHERE id='agent-1'")
      .first<{ online: number }>();
    expect(row?.online).toBe(0);
    expect(received.some((m) => m.includes("\"presence\"") && m.includes("false"))).toBe(true);
  });
});
