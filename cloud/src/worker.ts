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
  let body: { name?: string };
  try {
    body = (await req.json()) as { name?: string };
  } catch {
    return json({ error: "invalid body" }, 400);
  }
  const { name } = body;
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
  let body: { pairing_code?: string };
  try {
    body = (await req.json()) as { pairing_code?: string };
  } catch {
    return json({ error: "invalid body" }, 400);
  }
  const { pairing_code } = body;
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

async function routeAgent(req: Request, env: Env): Promise<Response> {
  const auth = req.headers.get("Authorization") ?? "";
  const token = auth.replace(/^Bearer\s+/i, "");
  if (!token) return json({ error: "missing token" }, 401);
  const agent = await env.DB.prepare("SELECT id FROM agents WHERE token_hash = ?")
    .bind(await sha256hex(token)).first<{ id: string }>();
  if (!agent) return json({ error: "unknown agent" }, 401);
  const id = env.AGENT.idFromName(agent.id);
  const stub = env.AGENT.get(id);
  const fwd = new Request(req.url, req);
  fwd.headers.set("X-Role", "agent");
  fwd.headers.set("X-Agent-Id", agent.id);
  return stub.fetch(fwd);
}

async function routeBrowser(req: Request, env: Env): Promise<Response> {
  const url = new URL(req.url);
  const token = url.searchParams.get("token") ?? "";
  if (!token) return json({ error: "missing token" }, 401);
  const session = await env.DB.prepare(
    "SELECT id, agent_id FROM sessions WHERE token_hash = ?")
    .bind(await sha256hex(token)).first<{ id: string; agent_id: string }>();
  if (!session) return json({ error: "unknown session" }, 401);
  const stub = env.AGENT.get(env.AGENT.idFromName(session.agent_id));
  const fwd = new Request(req.url, req);
  fwd.headers.set("X-Role", "browser");
  fwd.headers.set("X-Session-Id", session.id);
  return stub.fetch(fwd);
}

export default {
  async fetch(req: Request, env: Env): Promise<Response> {
    const url = new URL(req.url);
    if (url.pathname === "/enroll" && req.method === "POST") return enroll(req, env);
    if (url.pathname === "/pair" && req.method === "POST") return pair(req, env);
    if (url.pathname === "/agent") return routeAgent(req, env);
    if (url.pathname === "/browser") return routeBrowser(req, env);
    return json({ error: "not found" }, 404);
  },
};

export { AgentSession } from "./session";
