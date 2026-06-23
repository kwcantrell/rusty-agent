# Cloudflare Control Plane — Best-Practices Revision Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Modernize the existing `cloud/` Cloudflare control plane to current best practices — latest wrangler/vitest-pool-workers, `wrangler.jsonc`, real secret handling, observability — and rewrite the `AgentSession` Durable Object to use the WebSocket Hibernation API, all without changing externally observable behavior.

**Architecture:** Two tasks. Task 1 is pure tooling/config modernization with the DO code left untouched, gated on the existing test suite staying green from a clean install. Task 2 rewrites `AgentSession` to extend `DurableObject<Env>`, accept sockets via `this.ctx.acceptWebSocket` with role tags + `serializeAttachment`, re-derive live sockets via `getWebSockets`, persist a durable event `seq` in DO SQLite, drop the in-memory buffer, and keep R2 as the event log.

**Tech Stack:** TypeScript on Cloudflare Workers (Durable Objects Hibernation API, DO SQLite storage, D1, R2), Wrangler 4, `@cloudflare/vitest-pool-workers` 0.16 (Miniflare).

## Global Constraints

- **Behavior-preserving on the wire:** the Rust `agent-server` daemon, the wire protocol (`{ v, session_id, id?, kind, ... }`, event `payload: { type, ... }`), and the throwaway test page semantics are UNCHANGED. Zero changes outside `cloud/`.
- **Confined to `cloud/`:** do not touch `agent/` or any Rust crate.
- **No new features:** no multi-session-per-agent, no OAuth, no RPC-ification, no R2 artifact uploads. Single-active-session-per-agent holds.
- **R2 remains the event log; D1 remains users/agents/sessions.** No DO migration tag change (same class name, same `new_sqlite_classes` tag).
- **Versions:** `wrangler ^4`, `@cloudflare/vitest-pool-workers ^0.16`, `compatibility_date ≥ 2026-04-07` (use `2026-06-01`).
- **Secret:** `BOOTSTRAP_SECRET` must NOT be a plaintext config `var`; it comes from `.dev.vars` (gitignored) for local dev and `miniflare.bindings` for tests.
- **Clean-install gate:** every task verifies with `rm -rf node_modules && npm install && npm test` (no hidden `node_modules` edits).
- Commands run from `/home/kalen/rust-agent-runtime/cloud`. Node 22 + npm available; npm registry reachable.

---

## File Structure

- `cloud/package.json` — bump devDeps (wrangler 4, vitest-pool-workers 0.16, matching vitest); remove `patch-package`/`postinstall-postinstall` + the `postinstall` script (if the WAL bug is gone).
- `cloud/wrangler.jsonc` — **new**, replaces `cloud/wrangler.toml`. Same bindings; `compatibility_date` 2026-06-01; `observability` enabled; no `BOOTSTRAP_SECRET` var.
- `cloud/wrangler.toml` — **deleted**.
- `cloud/.dev.vars` — **new** (gitignored): `BOOTSTRAP_SECRET=dev-secret-change-me`.
- `cloud/.gitignore` — add `.dev.vars`.
- `cloud/vitest.config.ts` — point at `wrangler.jsonc`; compat date 2026-06-01; `miniflare.bindings.BOOTSTRAP_SECRET` for tests.
- `cloud/patches/` — **deleted** (if WAL bug gone in 0.16).
- `cloud/RUNNING.md` — update for wrangler-4 / `.dev.vars` flow.
- `cloud/src/session.ts` — **rewritten** to the Hibernation API (Task 2).
- `cloud/src/worker.ts` — unchanged except the stale Task-8 comment on the re-export line (Task 2, cosmetic).
- `cloud/test/session.test.ts` — extended with seq-durability and presence-on-close tests (Task 2).

---

## Task 1: Tooling & config modernization (DO code unchanged)

Bump the toolchain, convert config to `wrangler.jsonc`, move the secret to `.dev.vars`, enable observability, and remove the `patch-package` hack — all while `src/session.ts` and `src/worker.ts` logic stay exactly as they are. The deliverable is the **existing** test suite passing from a clean install on the modernized toolchain.

**Files:**
- Modify: `cloud/package.json`
- Create: `cloud/wrangler.jsonc`; Delete: `cloud/wrangler.toml`
- Create: `cloud/.dev.vars`; Modify: `cloud/.gitignore`
- Modify: `cloud/vitest.config.ts`
- Delete: `cloud/patches/` (conditionally)
- Modify: `cloud/RUNNING.md`
- Test: existing `cloud/test/*.ts` (regression — no assertion changes)

**Interfaces:**
- Consumes: nothing new.
- Produces: a `wrangler.jsonc` config and `.dev.vars`-sourced `BOOTSTRAP_SECRET` that Task 2 and `wrangler dev` rely on. No code symbols change.

- [ ] **Step 1: Bump dependencies**

Edit `cloud/package.json` to remove the `patch-package` machinery and bump versions. New file:

```json
{
  "name": "agent-control-plane",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "wrangler dev",
    "test": "vitest run",
    "db:init": "wrangler d1 execute agent-cp --local --file=./schema.sql"
  },
  "devDependencies": {
    "@cloudflare/vitest-pool-workers": "^0.16.0",
    "@cloudflare/workers-types": "^4.20250101.0",
    "typescript": "^5.6.0",
    "vitest": "^3.2.0",
    "wrangler": "^4.0.0"
  }
}
```

> `@cloudflare/vitest-pool-workers` pins `vitest` to a narrow peer range. After install, if npm reports a peer conflict on `vitest`, set `vitest` to the exact version named in the error and reinstall. Record the resolved versions in your report.

- [ ] **Step 2: Convert `wrangler.toml` → `wrangler.jsonc`**

Create `cloud/wrangler.jsonc`:

```jsonc
{
  "name": "agent-control-plane",
  "main": "src/worker.ts",
  "compatibility_date": "2026-06-01",
  "compatibility_flags": ["nodejs_compat"],
  "observability": { "enabled": true },
  "durable_objects": {
    "bindings": [
      { "name": "AGENT", "class_name": "AgentSession" }
    ]
  },
  "migrations": [
    { "tag": "v1", "new_sqlite_classes": ["AgentSession"] }
  ],
  "d1_databases": [
    { "binding": "DB", "database_name": "agent-cp", "database_id": "local" }
  ],
  "r2_buckets": [
    { "binding": "LOGS", "bucket_name": "agent-logs" }
  ]
}
```

Then delete the old config:

```bash
rm cloud/wrangler.toml
```

> Note: `BOOTSTRAP_SECRET` is intentionally absent here — it now comes from `.dev.vars` / secrets, not a config var.

- [ ] **Step 3: Move the secret to `.dev.vars` and gitignore it**

Create `cloud/.dev.vars`:

```
BOOTSTRAP_SECRET=dev-secret-change-me
```

Append to `cloud/.gitignore` (so the new file is never committed):

```
.dev.vars
```

- [ ] **Step 4: Update the Vitest config**

Replace `cloud/vitest.config.ts` with:

```ts
import { defineWorkersConfig } from "@cloudflare/vitest-pool-workers/config";

export default defineWorkersConfig({
  test: {
    poolOptions: {
      workers: {
        wrangler: { configPath: "./wrangler.jsonc" },
        miniflare: {
          compatibilityDate: "2026-06-01",
          // Tests read env.BOOTSTRAP_SECRET; provide it deterministically here
          // (the worker tests pass it back as a header, so any value works).
          bindings: { BOOTSTRAP_SECRET: "test-secret" },
        },
      },
    },
  },
});
```

- [ ] **Step 5: Remove the `patch-package` hack (conditionally)**

Delete the committed patch directory and verify the WAL bug is gone in 0.16:

```bash
rm -rf cloud/patches
```

(The `postinstall` script and `patch-package`/`postinstall-postinstall` devDeps were already removed in Step 1.)

> If — and only if — Step 6's clean install + test FAILS with the `.sqlite-wal` isolated-storage assertion, restore the `patch-package` setup (re-add the devDeps, the `postinstall` script, regenerate `cloud/patches/@cloudflare+vitest-pool-workers+<version>.patch` via `npx patch-package @cloudflare/vitest-pool-workers`) and note it. Expectation: 0.16 has fixed this and the patch is NOT needed.

- [ ] **Step 6: Clean install and run the existing suite (the gate)**

Run:

```bash
cd /home/kalen/rust-agent-runtime/cloud && rm -rf node_modules package-lock.json && npm install && npm test
```

Expected: install succeeds (resolving peer-compatible `vitest`); **all 9 existing tests pass** (`test/util.test.ts` 3, `test/worker.test.ts` 4, `test/session.test.ts` 2). The DO code is unchanged, so behavior is identical.

> If the WS-over-`fetch` test pattern in `session.test.ts` regressed due to a pool-API change (not a logic change), adjust the test harness mechanically to the 0.16 API **without changing assertions**, and note exactly what changed. Do not modify `src/session.ts` in this task.

- [ ] **Step 7: Update RUNNING.md for the wrangler-4 / `.dev.vars` flow**

In `cloud/RUNNING.md`, update the cloud-startup section so the secret comes from `.dev.vars` instead of a config var. Replace the `npx wrangler dev` lead-in with:

```markdown
## 1. Start the cloud (terminal A)
cd cloud
npm install                       # applies no patches now; clean install
echo 'BOOTSTRAP_SECRET=dev-secret-change-me' > .dev.vars   # gitignored; only if missing
npm run db:init                   # apply schema.sql to local D1
npx wrangler dev                  # Worker on http://localhost:8787 (DO/D1/R2 emulated)
```

Leave the enroll/run/test-client and verification sections unchanged except: in the enroll command, the `--bootstrap-secret` value must match `.dev.vars` (`dev-secret-change-me`). Add one line under Verify: "`.dev.vars` is gitignored — for a real deploy use `npx wrangler secret put BOOTSTRAP_SECRET`."

- [ ] **Step 8: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add cloud/package.json cloud/package-lock.json cloud/wrangler.jsonc cloud/vitest.config.ts cloud/.gitignore cloud/RUNNING.md
git rm cloud/wrangler.toml
git rm -r --cached cloud/patches 2>/dev/null || true
git add -A cloud/patches 2>/dev/null || true
git status --short   # confirm wrangler.toml deleted, patches removed, .dev.vars NOT staged
git commit -m "chore(cloud): modernize toolchain (wrangler 4, vitest-pool-workers 0.16, wrangler.jsonc, .dev.vars, observability)"
```

> Verify `git status` shows `.dev.vars` untracked (gitignored) and `cloud/patches/` removed (unless Step 5's fallback kept it).

---

## Task 2: `AgentSession` Durable Object hibernation rewrite

Rewrite the DO to the WebSocket Hibernation API (Approach 1): `extends DurableObject<Env>`, `acceptWebSocket` with role tags, `serializeAttachment` for per-connection metadata, `getWebSockets` to re-derive live sockets, a durable monotonic `seq` in DO SQLite, the in-memory buffer dropped (replay from R2). Behavior on the wire is identical; the existing tests stay green and two new tests cover the hibernation-specific guarantees.

**Files:**
- Rewrite: `cloud/src/session.ts`
- Modify: `cloud/src/worker.ts` (drop the stale Task-8 comment only)
- Modify/Test: `cloud/test/session.test.ts` (keep both existing tests; add two)

**Interfaces:**
- Consumes: `Env { DB, LOGS, AGENT, BOOTSTRAP_SECRET }` from `worker.ts`; the wire envelope shapes; the `X-Role`/`X-Session-Id`/`X-Agent-Id` headers set by the Worker's `routeAgent`/`routeBrowser` (unchanged).
- Produces: `AgentSession extends DurableObject<Env>` with hibernation handlers `webSocketMessage`/`webSocketClose`/`webSocketError`, a private `nextSeq(): number` backed by a `meta(k,v)` SQLite table, and `replayFromR2(sessionId, ws)` (unchanged behavior). No exported-symbol or wire change.

- [ ] **Step 1: Write the new failing tests first**

Append two tests to `cloud/test/session.test.ts` inside the `describe("relay", ...)` block (after the existing two). They use `runInDurableObject` to inspect DO-internal SQLite state and a close to exercise presence. Add `runInDurableObject` to the `cloudflare:test` import and `newToken`/`sha256hex` are already imported.

Add to the top import:

```ts
import { env, createExecutionContext, waitOnExecutionContext, runInDurableObject } from "cloudflare:test";
```

Append these tests:

```ts
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
    const keys = list.objects.map((o) => o.key).sort();
    expect(keys).toEqual([
      "sessions/sess-seq/00000000.json",
      "sessions/sess-seq/00000001.json",
    ]);

    // The seq survives in DO SQLite (value = next seq to hand out = 2).
    const id = env.AGENT.idFromName("agent-1");
    const stub = env.AGENT.get(id);
    const stored = await runInDurableObject(stub, async (_instance, state) => {
      const rows = state.storage.sql.exec("SELECT v FROM meta WHERE k='seq'").toArray();
      return rows.length ? Number((rows[0] as { v: number }).v) : 0;
    });
    expect(stored).toBe(2);
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
```

- [ ] **Step 2: Run the new tests to verify they fail**

Run: `cd /home/kalen/rust-agent-runtime/cloud && npm test -- session`
Expected: the two new tests FAIL against the current in-memory DO — the seq test fails because there is no `meta` SQLite table (the old `persist` uses an in-memory `this.seq`, and `runInDurableObject` finds no `meta` row), and the presence-close test may pass or fail depending on the old close path. (The existing two tests still pass.) This confirms the new tests exercise the not-yet-implemented hibernation/SQLite behavior.

- [ ] **Step 3: Rewrite `src/session.ts` to the Hibernation API**

Replace the entire contents of `cloud/src/session.ts` with:

```ts
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
      const others = this.ctx.getWebSockets("agent").filter((s) => s !== ws);
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

  /** Returns the current 0-based seq and advances the durable counter. */
  private nextSeq(): number {
    const rows = this.ctx.storage.sql
      .exec("SELECT v FROM meta WHERE k='seq'").toArray();
    const cur = rows.length ? Number((rows[0] as { v: number }).v) : 0;
    this.ctx.storage.sql.exec("INSERT OR REPLACE INTO meta (k, v) VALUES ('seq', ?)", cur + 1);
    return cur;
  }

  private async persist(sessionId: string, frame: string): Promise<void> {
    const key = `sessions/${sessionId}/${String(this.nextSeq()).padStart(8, "0")}.json`;
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
```

- [ ] **Step 4: Drop the stale comment in `worker.ts`**

In `cloud/src/worker.ts`, change the last line from:

```ts
export { AgentSession } from "./session"; // AgentSession placeholder — real DO logic added in Task 8
```

to:

```ts
export { AgentSession } from "./session";
```

- [ ] **Step 5: Run the full suite to verify green**

Run: `cd /home/kalen/rust-agent-runtime/cloud && npm test`
Expected: **all tests pass** — the two existing tests (relay + R2-replay, now exercising the hibernation path) and the two new tests (seq durability + presence-on-close). 4 tests in `session.test.ts`, 9 total across files plus the 2 new = 11.

> If the hibernation WS messages aren't delivered to `webSocketMessage` under the test pool, confirm the sockets are accepted via `this.ctx.acceptWebSocket` (not `server.accept()`), and that the test calls `.accept()` on the *client* side of the returned `webSocket`. Do not fall back to the in-memory pattern.

- [ ] **Step 6: Clean-install gate + typecheck**

Run:

```bash
cd /home/kalen/rust-agent-runtime/cloud && rm -rf node_modules && npm install && npm test && npx tsc --noEmit
```

Expected: all tests pass from a clean install; `tsc --noEmit` reports no type errors (the DO now uses `DurableObject<Env>`, `SqlStorage`, and `WebSocket` attachment APIs — all covered by `@cloudflare/workers-types`).

- [ ] **Step 7: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add cloud/src/session.ts cloud/src/worker.ts cloud/test/session.test.ts
git commit -m "feat(cloud): rewrite AgentSession to the WebSocket Hibernation API (durable seq in DO SQLite)"
```

---

## Self-Review (completed during planning)

**Spec coverage:**
- §3 tooling/config (wrangler 4, vitest-pool-workers 0.16, wrangler.jsonc, compat date, secret, observability, patch-package removal) → Task 1 (Steps 1–6).
- §3 RUNNING.md update → Task 1 Step 7.
- §4 DO hibernation rewrite (extends DurableObject, acceptWebSocket + tags, serializeAttachment/getWebSockets, durable seq in SQLite, drop in-memory buffer, R2 replay, presence via close handler, structured webSocketError logging) → Task 2 Step 3.
- §5 testing (regression-first; seq durability via runInDurableObject; reconstruct-from-attachment via the relay test now going through getWebSockets; presence via close) → Task 1 Step 6 + Task 2 Steps 1–6.
- §6 no new DO migration (same tag) → wrangler.jsonc keeps `new_sqlite_classes: ["AgentSession"]`, tag `v1`.
- §7 DoD items 1–5 → Tasks 1 and 2; behavior-preserving guarded by keeping the existing tests' assertions unchanged.

**Placeholder scan:** the only environment-resolved value is the exact `vitest` version under the pool's peer range (Task 1 Step 1) — handled with a concrete starting value (`^3.2.0`) plus a deterministic resolution procedure, not a TODO. The `patch-package` removal is conditional on a named, testable outcome (Task 1 Step 5).

**Type consistency:** `Attachment { role, sessionId, agentId }` is written by `serializeAttachment` and read by `deserializeAttachment` consistently across `fetch`/`webSocketMessage`/`webSocketClose`. `nextSeq(): number` returns a 0-based seq used by `persist` for the `00000000.json` key format, matching the existing R2 key shape the R2-replay test depends on. The `meta(k, v)` table name/columns match between the constructor, `nextSeq`, and the seq-durability test's `runInDurableObject` query. `BOOTSTRAP_SECRET` flows identically (Worker reads `env.BOOTSTRAP_SECRET`; provided by `.dev.vars` for dev and `miniflare.bindings` for tests).
