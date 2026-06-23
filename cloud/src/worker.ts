import { sha256hex, newToken, newPairingCode } from "./util";

export interface Env {
  DB: D1Database;
  LOGS: R2Bucket;
  AGENT: DurableObjectNamespace;
  BOOTSTRAP_SECRET: string;
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

async function enroll(req: Request, env: Env): Promise<Response> {
  if (req.headers.get("X-Bootstrap-Secret") !== env.BOOTSTRAP_SECRET) {
    return json({ error: "unauthorized" }, 401);
  }
  const { name } = (await req.json()) as { name?: string };
  const agentId = crypto.randomUUID();
  const token = newToken();
  const tokenHash = await sha256hex(token);
  const pairing = newPairingCode();
  const now = Date.now();
  await env.DB.prepare(
    "INSERT INTO agents (id, name, token_hash, pairing_code, online, created_at) VALUES (?,?,?,?,0,?)"
  ).bind(agentId, name ?? "agent", tokenHash, pairing, now).run();
  return json({ agent_id: agentId, agent_token: token, pairing_code: pairing });
}

async function pair(req: Request, env: Env): Promise<Response> {
  const { pairing_code } = (await req.json()) as { pairing_code?: string };
  if (!pairing_code) return json({ error: "missing pairing_code" }, 400);
  const agent = await env.DB.prepare("SELECT id FROM agents WHERE pairing_code = ?")
    .bind(pairing_code).first<{ id: string }>();
  if (!agent) return json({ error: "invalid pairing code" }, 404);
  const sessionId = crypto.randomUUID();
  const token = newToken();
  const tokenHash = await sha256hex(token);
  await env.DB.prepare(
    "INSERT INTO sessions (id, agent_id, token_hash, status, created_at) VALUES (?,?,?,'active',?)"
  ).bind(sessionId, agent.id, tokenHash, Date.now()).run();
  return json({ session_id: sessionId, session_token: token, agent_id: agent.id });
}

export default {
  async fetch(req: Request, env: Env): Promise<Response> {
    const url = new URL(req.url);
    if (url.pathname === "/enroll" && req.method === "POST") return enroll(req, env);
    if (url.pathname === "/pair" && req.method === "POST") return pair(req, env);
    return json({ error: "not found" }, 404);
  },
};

export { AgentSession } from "./session"; // added in Task 8
