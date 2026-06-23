import { env, createExecutionContext, waitOnExecutionContext } from "cloudflare:test";
import { describe, it, expect, beforeAll } from "vitest";
import worker from "../src/worker";

async function migrate() {
  // Apply schema.sql statements to the test D1.
  const sql = `
    CREATE TABLE IF NOT EXISTS agents (id TEXT PRIMARY KEY, name TEXT NOT NULL,
      token_hash TEXT NOT NULL UNIQUE, user_id TEXT, pairing_code TEXT NOT NULL,
      last_seen INTEGER, online INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS sessions (id TEXT PRIMARY KEY, agent_id TEXT NOT NULL,
      token_hash TEXT NOT NULL UNIQUE, status TEXT NOT NULL DEFAULT 'active',
      created_at INTEGER NOT NULL)`;
  for (const stmt of sql.split(";").map((s) => s.trim()).filter(Boolean)) {
    await env.DB.prepare(stmt).run();
  }
}

function post(path: string, body: unknown, headers: Record<string, string> = {}) {
  return new Request(`http://x${path}`, {
    method: "POST",
    headers: { "content-type": "application/json", ...headers },
    body: JSON.stringify(body),
  });
}

describe("enroll + pair", () => {
  beforeAll(migrate);

  it("rejects enroll without the bootstrap secret", async () => {
    const ctx = createExecutionContext();
    const res = await worker.fetch(post("/enroll", { name: "x" }), env, ctx);
    await waitOnExecutionContext(ctx);
    expect(res.status).toBe(401);
  });

  it("rejects enroll with a wrong bootstrap secret", async () => {
    const ctx = createExecutionContext();
    const res = await worker.fetch(
      post("/enroll", { name: "x" }, { "X-Bootstrap-Secret": "wrong-secret" }),
      env, ctx);
    await waitOnExecutionContext(ctx);
    expect(res.status).toBe(401);
  });

  it("enrolls then pairs", async () => {
    const ctx = createExecutionContext();
    const enrollRes = await worker.fetch(
      post("/enroll", { name: "x" }, { "X-Bootstrap-Secret": env.BOOTSTRAP_SECRET as string }),
      env, ctx);
    await waitOnExecutionContext(ctx);
    expect(enrollRes.status).toBe(200);
    const { pairing_code, agent_id } = await enrollRes.json<any>();
    expect(pairing_code).toMatch(/^\d{6}$/);

    const ctx2 = createExecutionContext();
    const pairRes = await worker.fetch(post("/pair", { pairing_code }), env, ctx2);
    await waitOnExecutionContext(ctx2);
    expect(pairRes.status).toBe(200);
    const paired = await pairRes.json<any>();
    expect(paired.agent_id).toBe(agent_id);
    expect(paired.session_token).toBeTruthy();
  });

  it("rejects an unknown pairing code", async () => {
    const ctx = createExecutionContext();
    const res = await worker.fetch(post("/pair", { pairing_code: "000000" }), env, ctx);
    await waitOnExecutionContext(ctx);
    expect(res.status).toBe(404);
  });
});
