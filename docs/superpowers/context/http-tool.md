# Context Primer — HTTP / Browser Tool

**Status:** Not started. Context primer — run `brainstorming` before implementing.
**Attaches via:** `Tool` trait, registered in `ToolRegistry`.
**Depends on:** agent core only.

## What it is

A tool that lets the agent make outbound HTTP requests (and, optionally later, drive a headless browser) to fetch documentation, call APIs, or scrape pages. Deferred from the core because it widens the security surface and isn't needed for the core demo.

## Where it fits

This is "just another tool" — it implements the existing `Tool` trait (`name`, `description`, `schema`, `intent`, `execute`) and registers in `ToolRegistry`. No core changes. The interesting part is its `intent()` declaration and how `PolicyEngine` treats network egress.

## Key responsibilities

- `http_request(method, url, headers?, body?)` → returns status, headers, body (truncated/streamed sensibly for the model).
- Declare a meaningful `ToolIntent` for network egress (target host, method) so policy can gate it.
- Respect timeout + cancellation from `ToolCtx`.
- Sane body-size limits; never dump megabytes into context.

## Proposed approach

- `reqwest` for HTTP. Stream/truncate large responses; convert HTML→text for readability when appropriate.
- Network policy: host allowlist/denylist in `PolicyEngine`; default-Ask for non-allowlisted hosts. Block link-local/loopback/metadata IPs (SSRF guard — e.g. `169.254.169.254`, `localhost`, private ranges) unless explicitly allowed.
- A headless-browser variant (Playwright/`chromiumoxide`) is a separate, larger follow-up — keep `http_request` minimal first.

## Open questions for brainstorming

- HTML→markdown conversion: in-tool, or leave raw and let the model cope?
- How aggressive should the SSRF/egress policy be by default?
- Do we need response caching to avoid re-fetching within a session?
- Is the headless browser in scope at all, or strictly HTTP?

## Definition of done (high level)

The agent can fetch a URL, the request is gated by network policy, large responses are bounded, and SSRF-class targets are blocked by default. Unit-tested against a local mock server; policy treatment tested as pure functions.
