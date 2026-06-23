# HTTP Fetch Tool (`agent-http`) — Design

**Date:** 2026-06-23
**Status:** Approved (pending spec review)

## Motivation

Give the agent a way to fetch documentation, read pages, and pull text/JSON off
the web — the first of the deferred *local deepeners* (#1 in
[`context/README.md`](../context/README.md)). It was deferred out of the core
because outbound HTTP widens the security surface (SSRF, context-flooding,
credential leakage) and wasn't needed for the core demo.

This is deliberately the **smallest** remaining subsystem: a single read-only
`fetch_url` tool. POST/API-calling, custom headers, a headless browser, and
response caching are explicitly out of scope (see §9).

## Core decisions

1. **A new workspace crate `agent-http`**, mirroring how `agent-mcp` bolted on.
   It depends only on `agent-tools` for the `Tool` trait. **Zero changes to the
   four core crates** (`agent-core` / `agent-model` / `agent-tools` /
   `agent-policy`) — holds the same bar that #5, #6, and #3 (mcp-client) all
   held. Wiring (config plumbing + registration) lives in the **non-core**
   `agent-runtime-config` crate and the two binaries.
2. **Network egress is gated in-tool and mapped onto the existing Read/Write
   axis** — the `agent-mcp` trick — so `ToolIntent` and `RulePolicy` need no
   network concept and stay untouched. The tool owns a `NetworkPolicy`.
3. **SSRF protection is a non-overridable hard floor**, enforced at execute time
   against the *resolved IP* (not just the hostname), and re-checked on every
   redirect — independent of the host allowlist.
4. **One GET-focused tool, `fetch_url`**, optimized for the 80% case (fetch a
   page/doc, return readable text). Simplest schema; hardest for a small local
   model to misuse.

## Architecture & seam

A new struct `FetchUrl` in `agent-http` implements the existing `Tool` trait
(`name` / `description` / `schema` / `intent` / `execute`) and is registered into
`ToolRegistry` alongside the fs/shell/git tools. Nothing above the trait changes
— `AgentLoop`, the policy engine, the approval channel, and the wire protocol
are all untouched.

```
agent-http (new crate)
  FetchUrl: Tool          # the registered tool
  NetworkPolicy           # host allow/ask decision (owned by the crate)
  ssrf::classify(ip)      # pure-function IP range classifier (hard floor)
  content::render(...)    # content-type branch -> model-ready text
  depends on: agent-tools (Tool trait), reqwest, url, ipnet, html renderer
```

### The tool: `fetch_url`

- **Schema:** `fetch_url(url: string)` — GET only. JSON Schema: one required
  string property `url`.
- **Returns** (`ToolOutput.content`): a short header line plus the rendered body,
  e.g. `GET https://example.com/ -> 200\n\n<rendered content>`. The exact
  `final_url` (after redirects) and status are included so the model sees where
  it actually landed.
- **`intent(args)`** parses the URL, rejects non-`http(s)` schemes early
  (`ToolError::InvalidArgs` — never reaches policy), and maps the `NetworkPolicy`
  decision onto the Read/Write axis (see §"Policy mapping"). `summary` =
  `GET <scheme>://<host><path>` so the approval UI is meaningful. `command` is
  `None`; `paths` is empty.
- **`display`** = `Display::Text("GET <url> -> <status> (<n> KB <kind>)")` for
  richer UI/approval rendering.
- Honors `ToolCtx.timeout` (overall + connect) and `ToolCtx.cancel`.

### Policy mapping (zero core change)

`NetworkPolicy::decide(host) -> Allow | Ask` is computed in the tool, then encoded
onto the existing axis exactly as `agent-mcp` encodes trust:

| NetworkPolicy result | `ToolIntent` shape | `RulePolicy::check` |
|---|---|---|
| host on allowlist | `access: Read`, `paths: []` | `Read` branch, `all_inside` over empty iter is vacuously true → **`Allow`** |
| host not on allowlist | `access: Write` | `Write` branch → **`Ask`** |

SSRF-blocked targets are **not** expressed through policy at all — they hard-fail
inside `execute()` (see below), because the block depends on the *resolved IP*,
which isn't known at `intent()` time and can change between calls (rebinding).

## Network policy & SSRF guard (security core)

### Host allowlist

`NetworkPolicy { allow_hosts: Vec<String> }`, owned by the crate. Default empty →
every host triggers approval. Host matching is **case-insensitive**, supporting:

- exact host match (`docs.rs`), and
- leading-dot suffix match (`.rust-lang.org` matches `doc.rust-lang.org`).

The allowlist only governs the **approval** decision (Ask → Allow). It never
relaxes the SSRF floor: an allowlisted *public* host that resolves to a *private*
IP is still blocked.

### SSRF hard floor

1. Disable `reqwest`'s automatic redirect following; handle redirects manually
   (max **5** hops).
2. For each hop: **resolve DNS ourselves**, validate **every** resolved A/AAAA
   against the blocked-range table, then **pin the connection to a validated IP**
   (via reqwest's IP-resolution override) while preserving the original Host
   header / TLS SNI. This defeats DNS-rebinding (TOCTOU between check and connect)
   and is re-run on every redirect target.
3. If any resolved IP is in a blocked range → `ToolError::Denied` with a short
   reason; the request is never made.

**Blocked ranges** (`ssrf::classify`, implemented with explicit CIDR checks via
`ipnet`):

- IPv4: `0.0.0.0/8`, `10.0.0.0/8`, `100.64.0.0/10` (CGNAT), `127.0.0.0/8`
  (loopback), `169.254.0.0/16` (link-local incl. cloud metadata
  `169.254.169.254`), `172.16.0.0/12`, `192.0.0.0/24`, `192.0.2.0/24`,
  `192.168.0.0/16`, `198.18.0.0/15`, `198.51.100.0/24`, `203.0.113.0/24`,
  `224.0.0.0/4` (multicast), `240.0.0.0/4` (reserved), `255.255.255.255`.
- IPv6: `::` (unspecified), `::1` (loopback), `fc00::/7` (ULA), `fe80::/10`
  (link-local). IPv4-mapped (`::ffff:0:0/96`) is unwrapped and re-checked against
  the IPv4 table.

### Other egress hygiene

- Only `http`/`https` schemes accepted; anything else → `InvalidArgs`.
- No credentials or `Authorization` headers are ever sent; a fixed `User-Agent`
  is set.
- The SSRF floor is IP-range based, so it blocks `localhost:<anyport>` (e.g. the
  local model server) regardless of port.

## Content handling

Branch on the response `Content-Type`:

| Content-Type | Handling |
|---|---|
| `text/html` | strip `script` / `style` / `nav` / `header` / `footer`, render the remainder to readable markdown |
| `application/json` | pass through raw (the model parses JSON well) |
| `text/*` (non-HTML) | pass through raw |
| binary / unknown | refuse: `non-text content (<N> bytes, <type>)` |

The HTML→markdown renderer crate is finalized in the plan (leading candidates:
`html2text` for robust text extraction, or `htmd` for HTML→markdown); the
pre-strip of non-content tags is done first regardless of renderer.

## Bounding

Tuned for a 32k-context local model — one fetch must not dominate the window:

- **Download cap:** abort the response stream beyond **2 MiB** of body bytes.
- **Return cap:** truncate the rendered result to **~8 KB**, appending
  `[truncated: showing 8K of <M>]`.
- **Redirects:** max 5, each re-resolved + re-SSRF-checked.
- **Timeout/cancel:** overall + connect timeouts derived from `ToolCtx.timeout`;
  `ToolCtx.cancel` aborts in-flight requests.

## Configuration & wiring

Mirrors the existing `command_allowlist` plumbing in `agent-runtime-config`:

- `RuntimeConfig` gains `http_allow_hosts: Vec<String>` (default `[]`), settable
  from the config file.
- CLI (`agent-cli`, `agent-server`) gains a repeatable `--allow-host <host>` flag.
- `build_registry(...)` is extended to take the http config and register
  `fetch_url`, so both binaries get the tool uniformly (the static fs/shell/git
  tools register exactly as today).

## Testing (Definition of Done)

- **Pure-function SSRF classifier:** a table of IPs (loopback, metadata, ULA,
  IPv4-mapped, public) → blocked/allowed. This is the "policy treatment tested as
  pure functions" DoD item.
- **Policy mapping:** `NetworkPolicy` host → `Access` → real `RulePolicy::check`
  → `Decision` (allowlisted = `Allow`, unknown = `Ask`), in the style of the
  existing `agent-mcp` tests.
- **Content branch + bounding:** HTML→markdown strip, JSON/text passthrough,
  binary refusal, 2 MiB download abort, ~8 KB truncation marker; scheme
  rejection.
- **Live mock-server integration** (`wiremock` or a tiny hyper server): fetch →
  convert + bound; redirect chain re-validation; SSRF block on a `127.0.0.1`
  target; binary refusal.
- `cargo test --workspace` green; `cargo clippy --all-targets -- -D warnings`
  clean.

## Out of scope (deferred)

Primer-deferred and intentionally left out to keep this the smallest slice:

- In-session **response caching**.
- **Headless browser** (Playwright/`chromiumoxide`) — a separate, larger
  follow-up.
- **POST / custom headers / general `http_request`** — `fetch_url` is GET-only.
- **Overriding the SSRF floor** for an explicitly-allowed private host — noted as
  a possible future follow-up; the floor is non-overridable in this slice.
