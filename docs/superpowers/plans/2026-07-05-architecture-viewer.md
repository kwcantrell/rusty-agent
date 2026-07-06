# Architecture Viewer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an Architecture sub-section to the Design tab: a live, read-only self-portrait of the running agent (model, tools, policy, sandbox, context, loop, prompt) fetched via a new `architecture_get` Tauri command and rendered as a fixed-layout block diagram with drill-down.

**Architecture:** Per spec `docs/superpowers/specs/2026-07-05-architecture-viewer-design.md` (Approach A). The daemon assembles an `ArchitectureSnapshot` from state it already holds (`RuntimeState` config + live loop's `tool_schemas()`/`sandbox_descriptor()`, `Session.recall_budget`); one read-only Tauri command carries it over IPC; the web side renders a CSS-grid block diagram with an SVG arrow layer and a detail panel — all pure components behind a single `architecture.ts` fetch seam.

**Tech Stack:** Rust (agent-core, agent-server, src-tauri), React 19 + vitest (web/).

## Global Constraints

- Two Cargo workspaces: `agent/` and `src-tauri/`; `source ~/.cargo/env` first; run cargo from the right directory.
- Conventional commits: `type(scope): summary`.
- Read-only feature: NO mutation paths, NO socket wire-protocol changes (`ServerEvent`/`Outbound` untouched) — this adds one Tauri IPC command only.
- Tauri-gated like Config: the Architecture sub-tab must not render at all outside Tauri.
- Redaction rules (safe to screenshot): `base_url` → scheme+host only; the system prompt TEXT never crosses IPC — only `est_tokens`, `override_active`, `override_chars`.
- No localStorage caching of snapshots; fetch on sub-tab entry + manual refresh.
- Block ids are exactly: `"model" | "loop" | "tools" | "policy" | "sandbox" | "context" | "prompt"` (7 blocks).
- Tool `kind` values are exactly: `"builtin" | "mcp" | "memory" | "skills" | "context"`; unclassifiable tools land in `"builtin"`, never dropped.
- Final gate: `bash scripts/ci.sh` from repo root.

---

### Task 1: Snapshot types + base-url redaction + loop accessor (Rust)

**Files:**
- Modify: `agent/crates/agent-server/src/wire.rs` (append after `SettingsState`/`DiscoveredSkill` block)
- Modify: `agent/crates/agent-core/src/loop_.rs` (one accessor next to `sandbox_descriptor()`, ~line 189)
- Test: both files' existing `#[cfg(test)]` modules

**Interfaces:**
- Consumes: `agent_tools::ToolSchema { name: String, description: String, parameters: serde_json::Value }`; `AgentLoop.tools: Arc<ToolRegistry>` (private field; `ToolRegistry::schemas() -> Vec<ToolSchema>` exists).
- Produces (used by Tasks 2–3):

```rust
// wire.rs
pub struct ArchitectureSnapshot { pub model: ModelInfo, pub tools: Vec<ToolEntry>,
  pub policy: PolicyInfo, pub sandbox: SandboxInfo, pub context: ContextInfo,
  #[serde(rename = "loop")] pub loop_info: LoopInfo, pub prompt: PromptInfo }
pub struct ModelInfo { pub backend: String, pub base_url_host: String, pub model: String,
  pub protocol: String, pub temperature: f32, pub top_p: Option<f32>, pub top_k: Option<u32>,
  pub enable_thinking: bool, pub preserve_thinking: bool }
pub struct ToolEntry { pub name: String, pub summary: String, pub kind: String }
pub struct PolicyInfo { pub allowlist: Vec<String>, pub denylist: Vec<String>,
  pub hard_floor: Vec<String>, pub http_allow_hosts: Vec<String> }
pub struct SandboxInfo { pub mode: String, pub mechanism: String, pub image: Option<String>,
  pub network: bool, pub degraded: Option<String> }
pub struct ContextInfo { pub context_limit: usize, pub max_tool_result_bytes: usize,
  pub memory_enabled: bool, pub recall_budget: usize, pub compaction_model: Option<String> }
pub struct LoopInfo { pub max_turns: usize, pub max_parallel_tools: usize,
  pub subagents_enabled: bool, pub subagent_max_depth: usize, pub subagent_model: Option<String> }
pub struct PromptInfo { pub est_tokens: usize, pub override_active: bool,
  pub override_chars: Option<usize> }
pub fn redact_base_url(url: &str) -> String;
// loop_.rs
impl AgentLoop { pub fn tool_schemas(&self) -> Vec<agent_tools::ToolSchema> }
```

- [ ] **Step 1: Write failing tests**

In `agent/crates/agent-server/src/wire.rs` tests module:

```rust
#[test]
fn redact_base_url_keeps_scheme_and_host_only() {
    assert_eq!(redact_base_url("http://localhost:8080/v1"), "http://localhost:8080");
    assert_eq!(redact_base_url("https://user:pw@api.example.com/v1?key=s3cret"),
        "https://api.example.com");
    assert_eq!(redact_base_url("localhost:8080"), "localhost:8080");
    assert_eq!(redact_base_url(""), "");
}

#[test]
fn architecture_snapshot_serializes_loop_under_the_loop_key() {
    let snap = ArchitectureSnapshot {
        model: ModelInfo { backend: "openai".into(), base_url_host: "http://x".into(),
            model: "m".into(), protocol: "native".into(), temperature: 0.6,
            top_p: None, top_k: None, enable_thinking: true, preserve_thinking: false },
        tools: vec![ToolEntry { name: "render".into(), summary: "Render an artifact".into(),
            kind: "builtin".into() }],
        policy: PolicyInfo { allowlist: vec![], denylist: vec![], hard_floor: vec![],
            http_allow_hosts: vec![] },
        sandbox: SandboxInfo { mode: "auto".into(), mechanism: "docker".into(),
            image: Some("img".into()), network: false, degraded: None },
        context: ContextInfo { context_limit: 32768, max_tool_result_bytes: 1,
            memory_enabled: false, recall_budget: 0, compaction_model: None },
        loop_info: LoopInfo { max_turns: 40, max_parallel_tools: 4,
            subagents_enabled: false, subagent_max_depth: 1, subagent_model: None },
        prompt: PromptInfo { est_tokens: 97, override_active: false, override_chars: None },
    };
    let j = serde_json::to_value(&snap).unwrap();
    assert!(j.get("loop").is_some(), "loop_info must serialize as \"loop\": {j}");
    assert_eq!(j["tools"][0]["kind"], "builtin");
    let back: ArchitectureSnapshot = serde_json::from_value(j).unwrap();
    assert_eq!(back.loop_info.max_turns, 40);
}
```

In `agent/crates/agent-core/src/loop_.rs` tests module (reuse the module's existing loop fixture — read neighboring tests such as the ones constructing `AgentLoop::new` with the testkit; adapt the fixture helper name, assertions are binding):

```rust
#[test]
fn tool_schemas_exposes_registered_tools() {
    let l = /* module's existing minimal-loop fixture */;
    let names: Vec<String> = l.tool_schemas().into_iter().map(|s| s.name).collect();
    assert!(!names.is_empty(), "fixture loop registers at least one tool");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run (in `agent/`): `source ~/.cargo/env && cargo test -p agent-server redact_base_url && cargo test -p agent-core tool_schemas_exposes`
Expected: compile FAIL (types/functions missing) — the expected failure mode.

- [ ] **Step 3: Implement**

In `wire.rs`, add after the `DiscoveredSkill` block (all structs `#[derive(Debug, Clone, Serialize, Deserialize)]`, fields exactly as in Interfaces above), plus:

```rust
/// Scheme+host(+port) only — no path, query, or userinfo. The snapshot must be
/// safe to screenshot/share, and base_url may carry credentials or key params.
pub fn redact_base_url(url: &str) -> String {
    let (scheme, rest) = match url.split_once("://") {
        Some((s, r)) => (Some(s), r),
        None => (None, url),
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or(host);
    match scheme {
        Some(s) => format!("{s}://{host}"),
        None => host.to_string(),
    }
}
```

In `loop_.rs`, next to `sandbox_descriptor()`:

```rust
/// The registered tool schemas (read-only; the architecture viewer's tool list).
pub fn tool_schemas(&self) -> Vec<agent_tools::ToolSchema> {
    self.tools.schemas()
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run (in `agent/`): `cargo test -p agent-server -p agent-core` → PASS (full crates, not just new tests).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-server/src/wire.rs agent/crates/agent-core/src/loop_.rs
git commit -m "feat(server): ArchitectureSnapshot types, base-url redaction, tool_schemas accessor"
```

---

### Task 2: Snapshot assembly — `RuntimeState::architecture()` + `Session::architecture()`

**Files:**
- Modify: `agent/crates/agent-server/src/runtime.rs`
- Modify: `agent/crates/agent-server/src/session.rs` (next to `settings_get`, ~line 146)
- Test: `runtime.rs` tests module (reuse its existing `RuntimeState` fixture, as the external-edit-guard tests do)

**Interfaces:**
- Consumes: Task 1's types; `RuntimeState` fields `config`, `mcp_tools: Arc<[Arc<dyn Tool>]>`, `memory_tools: Arc<[Arc<dyn Tool>]>`; `current_loop()`, `current_system_prompt()`; `cfg.effective_denylist()`; `agent_core::estimate_tokens(&str) -> usize`; `HARD_FLOOR_DENYLIST`; `Session.recall_budget` (match its actual field type when passing).
- Produces: `RuntimeState::architecture(&self, recall_budget: usize) -> ArchitectureSnapshot`; `Session::architecture(&self) -> ArchitectureSnapshot` (used by Task 3).

- [ ] **Step 1: Write failing tests** (in `runtime.rs` tests, adapting the module's `make()`-style fixture; if the fixture doesn't inject MCP/memory tools, extend a copy of it with one stub `Tool` in each arc — the crate's tests already define stub tools; reuse them)

```rust
#[test]
fn architecture_lists_registered_tools_with_provenance() {
    let (rs, _dir) = /* fixture with one injected mcp tool named "mcp_x" and one memory tool "remember" */;
    let snap = rs.architecture(512);
    let names: Vec<&str> = snap.tools.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"mcp_x"));
    let kind_of = |n: &str| snap.tools.iter().find(|t| t.name == n).unwrap().kind.clone();
    assert_eq!(kind_of("mcp_x"), "mcp");
    assert_eq!(kind_of("remember"), "memory");
    // context tools registered by the loop are classified "context"
    assert!(snap.tools.iter().any(|t| t.kind == "context"),
        "context_recall/context_compact must be classified context");
}

#[test]
fn architecture_policy_carries_hard_floor_and_redacted_url() {
    let (rs, _dir) = /* fixture */;
    let snap = rs.architecture(0);
    for f in agent_runtime_config::HARD_FLOOR_DENYLIST {
        assert!(snap.policy.hard_floor.contains(&f.to_string()));
        assert!(snap.policy.denylist.contains(&f.to_string()), "effective denylist includes floor");
    }
    assert!(!snap.model.base_url_host.contains("/v1"), "path must be redacted: {}", snap.model.base_url_host);
}

#[test]
fn architecture_prompt_flags_track_override() {
    let (rs, _dir) = /* fixture */;
    assert!(!rs.architecture(0).prompt.override_active);
    let mut cfg = rs.settings_state().settings;
    cfg.system_prompt_override = Some("OVERRIDE".into());
    rs.apply(cfg).unwrap();
    let p = rs.architecture(0).prompt;
    assert!(p.override_active);
    assert_eq!(p.override_chars, Some(8));
    assert!(p.est_tokens > 0);
}

#[test]
fn architecture_reflects_sandbox_and_recall_budget() {
    let (rs, _dir) = /* fixture */;
    let snap = rs.architecture(1234);
    assert_eq!(snap.context.recall_budget, 1234);
    assert!(!snap.sandbox.mechanism.is_empty());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run (in `agent/`): `cargo test -p agent-server architecture_` → compile FAIL (method missing).

- [ ] **Step 3: Implement**

In `runtime.rs` (imports: `crate::wire::{redact_base_url, ArchitectureSnapshot, ContextInfo, LoopInfo, ModelInfo, PolicyInfo, PromptInfo, SandboxInfo, ToolEntry}`, `std::collections::HashSet`):

```rust
/// Read-only self-portrait of the LIVE loop (post-apply). Assembles from state
/// this struct already holds; never mutates, never exposes the prompt text.
pub fn architecture(&self, recall_budget: usize) -> ArchitectureSnapshot {
    const CONTEXT_TOOLS: [&str; 2] = ["context_recall", "context_compact"];
    const SKILL_TOOLS: [&str; 4] = ["list_skills", "use_skill", "create_skill", "read_skill_file"];
    let cfg = self.config.lock().unwrap().clone();
    let loop_ = self.current_loop();
    let mcp: HashSet<String> = self.mcp_tools.iter().map(|t| t.name().to_string()).collect();
    let mem: HashSet<String> = self.memory_tools.iter().map(|t| t.name().to_string()).collect();
    let tools = loop_
        .tool_schemas()
        .into_iter()
        .map(|s| {
            let kind = if mcp.contains(&s.name) { "mcp" }
                else if mem.contains(&s.name) { "memory" }
                else if CONTEXT_TOOLS.contains(&s.name.as_str()) { "context" }
                else if SKILL_TOOLS.contains(&s.name.as_str()) { "skills" }
                else { "builtin" };
            ToolEntry {
                summary: s.description.split('.').next().unwrap_or("").trim().to_string(),
                name: s.name,
                kind: kind.to_string(),
            }
        })
        .collect();
    let d = loop_.sandbox_descriptor();
    let prompt = self.current_system_prompt();
    ArchitectureSnapshot {
        model: ModelInfo {
            backend: cfg.backend.clone(),
            base_url_host: redact_base_url(&cfg.base_url),
            model: cfg.model.clone(),
            protocol: cfg.protocol.clone(),
            temperature: cfg.temperature,
            top_p: cfg.top_p,
            top_k: cfg.top_k,
            enable_thinking: cfg.enable_thinking,
            preserve_thinking: cfg.preserve_thinking,
        },
        tools,
        policy: PolicyInfo {
            allowlist: cfg.command_allowlist.clone(),
            denylist: cfg.effective_denylist(),
            hard_floor: HARD_FLOOR_DENYLIST.iter().map(|s| s.to_string()).collect(),
            http_allow_hosts: cfg.http_allow_hosts.clone(),
        },
        sandbox: SandboxInfo {
            mode: d.mode.to_string(),
            mechanism: d.mechanism.to_string(),
            image: d.image,
            network: d.network,
            degraded: d.degraded,
        },
        context: ContextInfo {
            context_limit: cfg.context_limit,
            max_tool_result_bytes: cfg.max_tool_result_bytes,
            memory_enabled: cfg.memory,
            recall_budget,
            compaction_model: cfg.compaction_model.as_ref().and_then(|m| m.model.clone()),
        },
        loop_info: LoopInfo {
            max_turns: cfg.max_turns,
            max_parallel_tools: cfg.max_parallel_tools,
            subagents_enabled: cfg.subagents,
            subagent_max_depth: cfg.subagent_max_depth,
            subagent_model: cfg.subagent_model.as_ref().and_then(|m| m.model.clone()),
        },
        prompt: PromptInfo {
            est_tokens: agent_core::estimate_tokens(&prompt),
            override_active: cfg.system_prompt_override.is_some(),
            override_chars: cfg.system_prompt_override.as_ref().map(|s| s.chars().count()),
        },
    }
}
```

(If `SandboxDescriptor.mode` lacks `Display`, format via its existing string conversion — check how `settings_state`/`sandbox_degraded_from` stringify it and match.)

In `session.rs`, next to `settings_get`:

```rust
/// Read-only architecture self-portrait (the `architecture_get` command).
pub fn architecture(&self) -> ArchitectureSnapshot {
    self.runtime.architecture(self.recall_budget)
}
```

(Adapt if `recall_budget`'s field type isn't `usize` — convert at the call site, keep the snapshot field `usize`.)

- [ ] **Step 4: Run tests to verify they pass**

Run (in `agent/`): `cargo test -p agent-server` → PASS (all).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-server/src/runtime.rs agent/crates/agent-server/src/session.rs
git commit -m "feat(server): assemble live ArchitectureSnapshot from runtime state"
```

---

### Task 3: `architecture_get` Tauri command

**Files:**
- Modify: `src-tauri/src/lib.rs` (command next to `settings_get` ~line 60; register in the `all_handlers!` macro list ~line 158)
- Test: `src-tauri/src/lib.rs` tests (mirror `settings_get_returns_state_over_ipc`, ~line 242)

**Interfaces:**
- Consumes: `Session::architecture()` (Task 2); `ArchitectureSnapshot` (Task 1 — import from `agent_server` alongside the existing `SettingsState` import).
- Produces: IPC command `architecture_get` returning the snapshot JSON (consumed by Task 4's `invoke("architecture_get")`).

- [ ] **Step 1: Write the failing test** (in the `src-tauri` workspace — mirror the existing IPC test's mock-app setup; assertions binding, harness names adapted)

```rust
#[test]
fn architecture_get_returns_snapshot_over_ipc() {
    // same mock-app harness as settings_get_returns_state_over_ipc, cmd: "architecture_get"
    // assert the result is Ok and its JSON has the seven block keys:
    // model, tools, policy, sandbox, context, loop, prompt
}
```

Write it concretely by copying the neighboring test's harness lines and swapping the command name + assertions:

```rust
let res = /* harness invoke */("architecture_get", /* no args */);
assert!(res.is_ok(), "architecture_get should resolve: {res:?}");
let v: serde_json::Value = /* harness deserialize */;
for key in ["model", "tools", "policy", "sandbox", "context", "loop", "prompt"] {
    assert!(v.get(key).is_some(), "missing block {key}: {v}");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run (in `src-tauri/`): `source ~/.cargo/env && cargo test architecture_get` → FAIL (unknown command).

- [ ] **Step 3: Implement**

```rust
#[tauri::command]
fn architecture_get(state: tauri::State<'_, AppState>) -> ArchitectureSnapshot {
    session(&state).architecture()
}
```

Add `architecture_get,` to the `all_handlers!` macro list (after `settings_get,`), and `ArchitectureSnapshot` to the existing `agent_server` imports.

- [ ] **Step 4: Run tests to verify they pass**

Run (in `src-tauri/`): `cargo test` → PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(desktop): architecture_get IPC command"
```

---

### Task 4: `architecture.ts` + `ArchDiagram` (web)

**Files:**
- Create: `web/src/components/design/architecture.ts`
- Create: `web/src/components/design/ArchDiagram.tsx`
- Test: `web/src/components/design/ArchDiagram.test.tsx`

**Interfaces:**
- Consumes: nothing from web yet (`invoke` via dynamic `import("@tauri-apps/api/core")`, matching App.tsx's pattern).
- Produces (used by Tasks 5–6):

```ts
// architecture.ts
export type BlockId = "model" | "loop" | "tools" | "policy" | "sandbox" | "context" | "prompt";
export interface ToolEntry { name: string; summary: string; kind: "builtin" | "mcp" | "memory" | "skills" | "context" }
export interface ArchitectureSnapshot {
  model: { backend: string; base_url_host: string; model: string; protocol: string;
    temperature: number; top_p: number | null; top_k: number | null;
    enable_thinking: boolean; preserve_thinking: boolean };
  tools: ToolEntry[];
  policy: { allowlist: string[]; denylist: string[]; hard_floor: string[]; http_allow_hosts: string[] };
  sandbox: { mode: string; mechanism: string; image: string | null; network: boolean; degraded: string | null };
  context: { context_limit: number; max_tool_result_bytes: number; memory_enabled: boolean;
    recall_budget: number; compaction_model: string | null };
  loop: { max_turns: number; max_parallel_tools: number; subagents_enabled: boolean;
    subagent_max_depth: number; subagent_model: string | null };
  prompt: { est_tokens: number; override_active: boolean; override_chars: number | null };
}
export async function fetchArchitecture(): Promise<ArchitectureSnapshot>;
// ArchDiagram.tsx
export function ArchDiagram(props: { snapshot: ArchitectureSnapshot; selected: BlockId | null;
  onSelect: (b: BlockId) => void }): JSX.Element;
```

- [ ] **Step 1: Write failing tests**

`web/src/components/design/ArchDiagram.test.tsx`:

```tsx
import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ArchDiagram } from "./ArchDiagram";
import type { ArchitectureSnapshot } from "./architecture";

export const fixture: ArchitectureSnapshot = {
  model: { backend: "openai", base_url_host: "http://localhost:8080", model: "qwen3.6",
    protocol: "native", temperature: 0.6, top_p: 0.95, top_k: 20,
    enable_thinking: true, preserve_thinking: true },
  tools: [
    { name: "render", summary: "Render an artifact", kind: "builtin" },
    { name: "remember", summary: "Store a memory", kind: "memory" },
    { name: "context_recall", summary: "Recall an offloaded result", kind: "context" },
  ],
  policy: { allowlist: ["ls"], denylist: ["rm -rf /"], hard_floor: ["rm -rf /"], http_allow_hosts: [] },
  sandbox: { mode: "auto", mechanism: "docker", image: "agent-sandbox-dev:latest",
    network: false, degraded: null },
  context: { context_limit: 262144, max_tool_result_bytes: 65536, memory_enabled: true,
    recall_budget: 512, compaction_model: null },
  loop: { max_turns: 40, max_parallel_tools: 4, subagents_enabled: true,
    subagent_max_depth: 1, subagent_model: null },
  prompt: { est_tokens: 97, override_active: false, override_chars: null },
};

describe("ArchDiagram", () => {
  it("renders all seven blocks", () => {
    render(<ArchDiagram snapshot={fixture} selected={null} onSelect={() => {}} />);
    for (const label of ["Model", "Agent Loop", "Tools", "Policy", "Sandbox", "Context", "Prompt"]) {
      expect(screen.getByRole("button", { name: new RegExp(label) })).toBeInTheDocument();
    }
  });

  it("shows dynamic badges", () => {
    render(<ArchDiagram snapshot={fixture} selected={null} onSelect={() => {}} />);
    expect(screen.getByText("3 tools")).toBeInTheDocument();
    expect(screen.getByText("memory on")).toBeInTheDocument();
    expect(screen.queryByText("degraded")).not.toBeInTheDocument();
    expect(screen.queryByText("override")).not.toBeInTheDocument();
  });

  it("shows degraded and override badges when present", () => {
    const s = { ...fixture,
      sandbox: { ...fixture.sandbox, degraded: "no docker daemon" },
      prompt: { est_tokens: 20, override_active: true, override_chars: 40 } };
    render(<ArchDiagram snapshot={s} selected={null} onSelect={() => {}} />);
    expect(screen.getByText("degraded")).toBeInTheDocument();
    expect(screen.getByText("override")).toBeInTheDocument();
  });

  it("fires onSelect with the block id and marks selection", () => {
    const picked: string[] = [];
    render(<ArchDiagram snapshot={fixture} selected="tools" onSelect={(b) => picked.push(b)} />);
    fireEvent.click(screen.getByRole("button", { name: /Policy/ }));
    expect(picked).toEqual(["policy"]);
    expect(screen.getByRole("button", { name: /Tools/ })).toHaveAttribute("aria-pressed", "true");
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run (in `web/`): `npm test -- --run ArchDiagram` → FAIL (modules missing).

- [ ] **Step 3: Implement `architecture.ts`**

```ts
export type BlockId = "model" | "loop" | "tools" | "policy" | "sandbox" | "context" | "prompt";

export interface ToolEntry {
  name: string; summary: string;
  kind: "builtin" | "mcp" | "memory" | "skills" | "context";
}

export interface ArchitectureSnapshot {
  model: { backend: string; base_url_host: string; model: string; protocol: string;
    temperature: number; top_p: number | null; top_k: number | null;
    enable_thinking: boolean; preserve_thinking: boolean };
  tools: ToolEntry[];
  policy: { allowlist: string[]; denylist: string[]; hard_floor: string[]; http_allow_hosts: string[] };
  sandbox: { mode: string; mechanism: string; image: string | null; network: boolean; degraded: string | null };
  context: { context_limit: number; max_tool_result_bytes: number; memory_enabled: boolean;
    recall_budget: number; compaction_model: string | null };
  loop: { max_turns: number; max_parallel_tools: number; subagents_enabled: boolean;
    subagent_max_depth: number; subagent_model: string | null };
  prompt: { est_tokens: number; override_active: boolean; override_chars: number | null };
}

/** One read-only IPC fetch; throws on invoke failure (pane shows retry). */
export async function fetchArchitecture(): Promise<ArchitectureSnapshot> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<ArchitectureSnapshot>("architecture_get");
}
```

- [ ] **Step 4: Implement `ArchDiagram.tsx`**

```tsx
import type { ArchitectureSnapshot, BlockId } from "./architecture";

interface BlockDef { id: BlockId; label: string; sub: (s: ArchitectureSnapshot) => string;
  badge?: (s: ArchitectureSnapshot) => string | null }

const BLOCKS: BlockDef[] = [
  { id: "prompt", label: "Prompt", sub: (s) => `~${s.prompt.est_tokens} tok`,
    badge: (s) => (s.prompt.override_active ? "override" : null) },
  { id: "model", label: "Model", sub: (s) => `${s.model.model} (${s.model.backend})` },
  { id: "context", label: "Context", sub: (s) => `${Math.round(s.context.context_limit / 1024)}k window`,
    badge: (s) => (s.context.memory_enabled ? "memory on" : "memory off") },
  { id: "loop", label: "Agent Loop", sub: (s) => `${s.loop.max_turns} turns max`,
    badge: (s) => (s.loop.subagents_enabled ? "subagents" : null) },
  { id: "tools", label: "Tools", sub: (s) => s.tools.map((t) => t.name).slice(0, 3).join(", ") + "…",
    badge: (s) => `${s.tools.length} tools` },
  { id: "policy", label: "Policy", sub: (s) => `${s.policy.allowlist.length} allowed / ${s.policy.denylist.length} denied` },
  { id: "sandbox", label: "Sandbox", sub: (s) => s.sandbox.image ?? s.sandbox.mechanism,
    badge: (s) => (s.sandbox.degraded ? "degraded" : null) },
];

/** Row layout: [prompt model context] feed [loop]; [tools policy sandbox] execute below. */
const ROWS: BlockId[][] = [["prompt", "model", "context"], ["loop"], ["tools", "policy", "sandbox"]];

export function ArchDiagram({ snapshot, selected, onSelect }: {
  snapshot: ArchitectureSnapshot; selected: BlockId | null; onSelect: (b: BlockId) => void;
}) {
  const byId = Object.fromEntries(BLOCKS.map((b) => [b.id, b])) as Record<BlockId, BlockDef>;
  return (
    <div className="relative flex flex-col gap-2 p-3" data-testid="arch-diagram">
      {ROWS.map((row, ri) => (
        <div key={ri} className="flex justify-center gap-2">
          {ri > 0 && <ArrowRow />}
          {row.map((id) => {
            const b = byId[id];
            const on = selected === id;
            const badge = b.badge?.(snapshot);
            return (
              <button key={id} aria-pressed={on} onClick={() => onSelect(id)}
                className="min-w-0 flex-1 rounded-lg px-3 py-2 text-left"
                style={{ maxWidth: "14rem", border: `1px solid ${on ? "var(--accent)" : "var(--border)"}`,
                  background: on ? "var(--surface-raised)" : "var(--surface-overlay)" }}>
                <span className="block text-xs font-semibold" style={{ color: "var(--text-strong)" }}>
                  {b.label}
                </span>
                <span className="block truncate text-[11px]" style={{ color: "var(--text-muted)" }}>
                  {b.sub(snapshot)}
                </span>
                {badge && (
                  <span className="mt-1 inline-block rounded-full px-1.5 text-[10px]"
                    style={{ background: badge === "degraded" ? "var(--state-error)" : "var(--surface-base)",
                      color: badge === "degraded" ? "var(--accent-fg)" : "var(--text-muted)",
                      border: "1px solid var(--border)" }}>
                    {badge}
                  </span>
                )}
              </button>
            );
          })}
        </div>
      ))}
    </div>
  );
}

/** Downward arrow between rows (pure decoration). */
function ArrowRow() {
  return <span aria-hidden className="self-center text-sm" style={{ color: "var(--text-muted)" }}>↓</span>;
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run (in `web/`): `npm test -- --run ArchDiagram` → PASS. Also `npm run typecheck`.

- [ ] **Step 6: Commit**

```bash
git add web/src/components/design/architecture.ts web/src/components/design/ArchDiagram.tsx web/src/components/design/ArchDiagram.test.tsx
git commit -m "feat(web): architecture snapshot types and block diagram"
```

---

### Task 5: `ArchDetail` (drill-down panel)

**Files:**
- Create: `web/src/components/design/ArchDetail.tsx`
- Test: `web/src/components/design/ArchDetail.test.tsx`

**Interfaces:**
- Consumes: `ArchitectureSnapshot`, `BlockId`, `ToolEntry` from `./architecture` (Task 4); reuse Task 4's exported test `fixture` by importing from `./ArchDiagram.test` — NO: test files must not import each other; duplicate the fixture into this test file verbatim instead (or extract to `web/src/components/design/archFixture.ts` shared test helper — do the latter: move the fixture into `archFixture.ts` in THIS task and update ArchDiagram.test.tsx to import it).
- Produces: `export function ArchDetail(props: { snapshot: ArchitectureSnapshot; block: BlockId }): JSX.Element`.

- [ ] **Step 1: Extract shared fixture**

Create `web/src/components/design/archFixture.ts` containing `export const archFixture: ArchitectureSnapshot = { … }` (the exact object from Task 4's test), and change `ArchDiagram.test.tsx` to `import { archFixture as fixture } from "./archFixture";` (delete its inline copy). Run `npm test -- --run ArchDiagram` → still PASS.

- [ ] **Step 2: Write failing tests**

`web/src/components/design/ArchDetail.test.tsx`:

```tsx
import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { ArchDetail } from "./ArchDetail";
import { archFixture as fixture } from "./archFixture";

describe("ArchDetail", () => {
  it("tools block renders a table grouped with kind labels", () => {
    render(<ArchDetail snapshot={fixture} block="tools" />);
    expect(screen.getByText("render")).toBeInTheDocument();
    expect(screen.getByText("Render an artifact")).toBeInTheDocument();
    expect(screen.getByText("memory")).toBeInTheDocument(); // kind chip for `remember`
  });

  it("policy block marks hard-floor entries", () => {
    render(<ArchDetail snapshot={fixture} block="policy" />);
    expect(screen.getByText("rm -rf /")).toBeInTheDocument();
    expect(screen.getByText("hard floor")).toBeInTheDocument();
    expect(screen.getByText("ls")).toBeInTheDocument();
  });

  it("model block lists backend, host, and sampling", () => {
    render(<ArchDetail snapshot={fixture} block="model" />);
    expect(screen.getByText("http://localhost:8080")).toBeInTheDocument();
    expect(screen.getByText(/0.6/)).toBeInTheDocument();
  });

  it("sandbox block shows degraded reason when present", () => {
    const s = { ...fixture, sandbox: { ...fixture.sandbox, degraded: "no docker daemon" } };
    render(<ArchDetail snapshot={s} block="sandbox" />);
    expect(screen.getByText("no docker daemon")).toBeInTheDocument();
  });

  it("prompt block never renders prompt text, only stats", () => {
    render(<ArchDetail snapshot={fixture} block="prompt" />);
    expect(screen.getByText(/97/)).toBeInTheDocument();
    expect(screen.getByText(/built-in/)).toBeInTheDocument(); // override inactive wording
  });
});
```

- [ ] **Step 3: Run tests to verify they fail**

Run (in `web/`): `npm test -- --run ArchDetail` → FAIL (module missing).

- [ ] **Step 4: Implement `ArchDetail.tsx`**

```tsx
import type { ArchitectureSnapshot, BlockId, ToolEntry } from "./architecture";

const dt = "text-[11px] uppercase tracking-wide";
const dd = "mb-2 text-sm";

function Row({ k, v }: { k: string; v: string }) {
  return (
    <div>
      <dt className={dt} style={{ color: "var(--text-muted)" }}>{k}</dt>
      <dd className={dd} style={{ color: "var(--text-strong)" }}>{v}</dd>
    </div>
  );
}

function ToolsTable({ tools }: { tools: ToolEntry[] }) {
  return (
    <table className="w-full text-sm" style={{ color: "var(--text)" }}>
      <tbody>
        {tools.map((t) => (
          <tr key={t.name} style={{ borderBottom: "1px solid var(--border)" }}>
            <td className="py-1 pr-2 font-mono text-xs" style={{ color: "var(--text-strong)" }}>{t.name}</td>
            <td className="py-1 pr-2 text-xs">{t.summary}</td>
            <td className="py-1">
              <span className="rounded-full px-1.5 text-[10px]"
                style={{ background: "var(--surface-base)", color: "var(--text-muted)",
                  border: "1px solid var(--border)" }}>{t.kind}</span>
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function PolicyLists({ p }: { p: ArchitectureSnapshot["policy"] }) {
  return (
    <dl>
      <dt className={dt} style={{ color: "var(--text-muted)" }}>Allowlist</dt>
      <dd className={dd}>{p.allowlist.length === 0 ? "—" : p.allowlist.join(", ")}</dd>
      <dt className={dt} style={{ color: "var(--text-muted)" }}>Denylist (effective)</dt>
      <dd className={dd}>
        <ul>
          {p.denylist.map((d) => (
            <li key={d} className="flex items-center gap-2">
              <span className="font-mono text-xs">{d}</span>
              {p.hard_floor.includes(d) && (
                <span className="rounded-full px-1.5 text-[10px]"
                  style={{ background: "var(--surface-base)", color: "var(--state-error)",
                    border: "1px solid var(--state-error)" }}>hard floor</span>
              )}
            </li>
          ))}
        </ul>
      </dd>
      <dt className={dt} style={{ color: "var(--text-muted)" }}>HTTP allow hosts</dt>
      <dd className={dd}>{p.http_allow_hosts.length === 0 ? "— (all fetches need approval)" : p.http_allow_hosts.join(", ")}</dd>
    </dl>
  );
}

export function ArchDetail({ snapshot, block }: { snapshot: ArchitectureSnapshot; block: BlockId }) {
  const s = snapshot;
  return (
    <div className="min-h-0 overflow-y-auto p-3" data-testid="arch-detail">
      {block === "model" && (
        <dl>
          <Row k="Backend" v={`${s.model.backend} (${s.model.protocol})`} />
          <Row k="Endpoint" v={s.model.base_url_host} />
          <Row k="Model" v={s.model.model} />
          <Row k="Sampling" v={`temp ${s.model.temperature}, top_p ${s.model.top_p ?? "—"}, top_k ${s.model.top_k ?? "—"}`} />
          <Row k="Thinking" v={`${s.model.enable_thinking ? "on" : "off"}${s.model.preserve_thinking ? ", preserved in history" : ""}`} />
        </dl>
      )}
      {block === "tools" && <ToolsTable tools={s.tools} />}
      {block === "policy" && <PolicyLists p={s.policy} />}
      {block === "sandbox" && (
        <dl>
          <Row k="Mode" v={s.sandbox.mode} />
          <Row k="Mechanism" v={s.sandbox.mechanism} />
          <Row k="Image" v={s.sandbox.image ?? "—"} />
          <Row k="Network" v={s.sandbox.network ? "enabled" : "disabled"} />
          {s.sandbox.degraded && <Row k="Degraded" v={s.sandbox.degraded} />}
        </dl>
      )}
      {block === "context" && (
        <dl>
          <Row k="Context limit" v={`${s.context.context_limit.toLocaleString()} tokens`} />
          <Row k="Max tool result" v={`${s.context.max_tool_result_bytes.toLocaleString()} bytes`} />
          <Row k="Memory" v={s.context.memory_enabled ? `on (recall budget ${s.context.recall_budget})` : "off"} />
          <Row k="Compaction model" v={s.context.compaction_model ?? "— (primary model)"} />
        </dl>
      )}
      {block === "loop" && (
        <dl>
          <Row k="Max turns" v={String(s.loop.max_turns)} />
          <Row k="Parallel tools" v={String(s.loop.max_parallel_tools)} />
          <Row k="Subagents" v={s.loop.subagents_enabled
            ? `on (depth ${s.loop.subagent_max_depth}${s.loop.subagent_model ? `, model ${s.loop.subagent_model}` : ""})`
            : "off"} />
        </dl>
      )}
      {block === "prompt" && (
        <dl>
          <Row k="Composed size" v={`~${s.prompt.est_tokens} tokens`} />
          <Row k="Base prompt" v={s.prompt.override_active
            ? `override active (${s.prompt.override_chars} chars)` : "built-in"} />
        </dl>
      )}
    </div>
  );
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run (in `web/`): `npm test -- --run ArchDetail ArchDiagram` → PASS.

- [ ] **Step 6: Commit**

```bash
git add web/src/components/design/ArchDetail.tsx web/src/components/design/ArchDetail.test.tsx web/src/components/design/archFixture.ts web/src/components/design/ArchDiagram.test.tsx
git commit -m "feat(web): architecture drill-down detail panel"
```

---

### Task 6: `ArchitecturePane` + DesignPane sub-nav

**Files:**
- Create: `web/src/components/design/ArchitecturePane.tsx`
- Modify: `web/src/components/design/DesignPane.tsx` (section union + third sub-tab, Tauri-gated)
- Test: `web/src/components/design/ArchitecturePane.test.tsx`; extend `web/src/components/design/DesignPane.test.tsx`

**Interfaces:**
- Consumes: `fetchArchitecture`, `ArchDiagram`, `ArchDetail`, `archFixture` (tests), DesignPane's existing `tauri`/`sub()` helpers.
- Produces: `export function ArchitecturePane(): JSX.Element` (no props — self-contained behind `architecture.ts`).

- [ ] **Step 1: Write failing tests**

`web/src/components/design/ArchitecturePane.test.tsx`:

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { archFixture } from "./archFixture";

const fetchMock = vi.hoisted(() => ({ fn: vi.fn() }));
vi.mock("./architecture", async (importOriginal) => ({
  ...(await importOriginal<object>()),
  fetchArchitecture: fetchMock.fn,
}));

import { ArchitecturePane } from "./ArchitecturePane";

describe("ArchitecturePane", () => {
  beforeEach(() => fetchMock.fn.mockReset());

  it("shows loading then the diagram with the loop block pre-selected", async () => {
    fetchMock.fn.mockResolvedValue(archFixture);
    render(<ArchitecturePane />);
    expect(screen.getByText(/Loading architecture/)).toBeInTheDocument();
    await waitFor(() => expect(screen.getByTestId("arch-diagram")).toBeInTheDocument());
    expect(screen.getByRole("button", { name: /Agent Loop/ })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByTestId("arch-detail")).toBeInTheDocument();
  });

  it("drills down on block click", async () => {
    fetchMock.fn.mockResolvedValue(archFixture);
    render(<ArchitecturePane />);
    await waitFor(() => screen.getByTestId("arch-diagram"));
    fireEvent.click(screen.getByRole("button", { name: /Tools/ }));
    expect(screen.getByText("Render an artifact")).toBeInTheDocument();
  });

  it("error shows retry which refetches", async () => {
    fetchMock.fn.mockRejectedValueOnce(new Error("daemon gone"));
    fetchMock.fn.mockResolvedValueOnce(archFixture);
    render(<ArchitecturePane />);
    await waitFor(() => expect(screen.getByText(/daemon gone/)).toBeInTheDocument());
    fireEvent.click(screen.getByRole("button", { name: /Retry/ }));
    await waitFor(() => expect(screen.getByTestId("arch-diagram")).toBeInTheDocument());
    expect(fetchMock.fn).toHaveBeenCalledTimes(2);
  });

  it("refresh button refetches", async () => {
    fetchMock.fn.mockResolvedValue(archFixture);
    render(<ArchitecturePane />);
    await waitFor(() => screen.getByTestId("arch-diagram"));
    fireEvent.click(screen.getByRole("button", { name: /Refresh/ }));
    expect(fetchMock.fn).toHaveBeenCalledTimes(2);
  });
});
```

Extend `DesignPane.test.tsx` with two cases (reuse its existing `base` props + tauri mock):

```tsx
it("shows the Architecture sub-tab under Tauri and renders the pane", () => {
  render(<DesignPane {...base} items={[]} />);
  fireEvent.click(screen.getByRole("tab", { name: "Architecture" }));
  expect(screen.getByText(/Loading architecture/)).toBeInTheDocument();
});

it("hides the Architecture sub-tab outside Tauri", () => {
  tauriMock.value = false;
  render(<DesignPane {...base} items={[]} />);
  expect(screen.queryByRole("tab", { name: "Architecture" })).not.toBeInTheDocument();
});
```

(The first case hits the real `fetchArchitecture`, whose dynamic `import("@tauri-apps/api/core")` rejects in jsdom → the pane will settle into the error state; asserting the initial "Loading architecture" text keeps the test synchronous and mock-free.)

- [ ] **Step 2: Run tests to verify they fail**

Run (in `web/`): `npm test -- --run ArchitecturePane DesignPane` → FAIL (module missing / no Architecture tab).

- [ ] **Step 3: Implement `ArchitecturePane.tsx`**

```tsx
import { useCallback, useEffect, useState } from "react";
import { fetchArchitecture, type ArchitectureSnapshot, type BlockId } from "./architecture";
import { ArchDiagram } from "./ArchDiagram";
import { ArchDetail } from "./ArchDetail";

/** Self-contained: fetches on mount; staleness is the enemy, so no caching. */
export function ArchitecturePane() {
  const [snapshot, setSnapshot] = useState<ArchitectureSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<BlockId>("loop");

  const load = useCallback(() => {
    setError(null);
    setSnapshot(null);
    fetchArchitecture().then(setSnapshot).catch((e) => setError(String(e)));
  }, []);
  useEffect(() => { load(); }, [load]);

  if (error) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-2 p-6 text-sm"
        style={{ color: "var(--text-muted)" }}>
        <p>Could not read the runtime architecture: {error}</p>
        <button onClick={load} className="rounded px-3 py-1 text-xs"
          style={{ background: "var(--accent)", color: "var(--accent-fg)" }}>Retry</button>
      </div>
    );
  }
  if (!snapshot) {
    return <p className="p-4 text-sm" style={{ color: "var(--text-muted)" }}>Loading architecture…</p>;
  }
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center justify-end px-3 pt-2">
        <button onClick={load} className="rounded px-2 py-0.5 text-xs"
          style={{ color: "var(--text-muted)", border: "1px solid var(--border)" }}>Refresh</button>
      </div>
      <ArchDiagram snapshot={snapshot} selected={selected} onSelect={setSelected} />
      <div className="min-h-0 flex-1" style={{ borderTop: "1px solid var(--border)" }}>
        <ArchDetail snapshot={snapshot} block={selected} />
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Wire into `DesignPane.tsx`**

- Section state union becomes `"canvas" | "config" | "architecture"`.
- Add after the Config button (inside the same `{tauri && …}` gating — add a second gated button):

```tsx
{tauri && (
  <button role="tab" aria-selected={section === "architecture"}
    onClick={() => setSection("architecture")}
    className="rounded-t-lg px-3 py-1 text-xs" style={sub(section === "architecture")}>Architecture</button>
)}
```

- Content branch: render `<ArchitecturePane />` when `section === "architecture" && tauri`, keeping the existing config branch:

```tsx
{section === "config" && tauri ? (
  <ConfigPanel … unchanged … />
) : section === "architecture" && tauri ? (
  <ArchitecturePane />
) : !active ? ( … existing empty state/canvas … )}
```

- [ ] **Step 5: Run tests to verify they pass**

Run (in `web/`): `npm test -- --run` (FULL suite) and `npm run typecheck` → all PASS.

- [ ] **Step 6: Commit**

```bash
git add web/src/components/design/ArchitecturePane.tsx web/src/components/design/ArchitecturePane.test.tsx web/src/components/design/DesignPane.tsx web/src/components/design/DesignPane.test.tsx
git commit -m "feat(web): Architecture sub-section wired into the Design tab"
```

---

### Task 7: Full gate + manual verification

**Files:** none (verification only; fix regressions if any).

- [ ] **Step 1: Run the CI gate**

From repo root: `bash scripts/ci.sh` → fmt + clippy + full `agent/` tests + web typecheck/vitest all green. Also `cd src-tauri && cargo test` (second workspace, not covered by ci.sh's cargo run if it only covers `agent/` — check the script; run it explicitly regardless).

- [ ] **Step 2: Manual e2e (desktop app)**

Launch `npm run desktop:dev`, then:
1. Design tab → **Architecture** sub-tab appears (third), loads a diagram with 7 blocks.
2. Tools block badge shows the real registered count; click it → the tool table lists `render`, `bash`-family, context tools with kind chips.
3. Policy detail marks the hard-floor entries; Sandbox detail shows `agent-sandbox-dev:latest` and no degraded badge.
4. Open Config, set a system-prompt override, Save; back to Architecture → Refresh → Prompt block shows the "override" badge.
5. Confirm the prompt text itself appears nowhere in the pane.

- [ ] **Step 3: Done**

Use `superpowers:finishing-a-development-branch`.

---

## Deviations from the spec

1. **Offload/compaction posture (Context block):** The spec anticipated a separate offload-posture knob. In the implementation, offloading is always-on with no distinct posture field; the Context block instead exposes `max_tool_result_bytes` (the offload threshold) and `compaction_model` (null = primary model). There is no additional posture value to show.

2. **Stream idle timeout (Loop block):** Initially omitted from the snapshot. Restored post-review as `stream_idle_timeout_secs` — a build-time constant (`DEFAULT_STREAM_IDLE_TIMEOUT`), not a config knob — because a self-portrait of the running agent should reflect actual runtime values, not only config-settable ones.

---

## Self-review (performed at plan-writing time)

- **Spec coverage:** snapshot blocks (T1 types, T2 assembly incl. recall_budget via Session), redaction (T1 golden), tool provenance partitioning + best-effort builtin (T2), degraded propagation (T2 test + T4 badge + T5 detail), command + registration + IPC smoke (T3), diagram + badges + selection (T4), drill-down incl. hard-floor marks and no-prompt-text (T5), fetch-on-entry/refresh/error-retry/no-caching (T6 pane), Tauri gating (T6 DesignPane tests), manual freshness check (T7). Out-of-scope items (telemetry, editing, Worker) have no tasks — correct. See "Deviations from the spec" section above for two items where the implementation differs from the original spec: offload posture representation and stream idle timeout.
- **Type consistency:** `ArchitectureSnapshot` field names identical across Rust serde (snake_case, `loop` rename) and TS mirror; `BlockId` values match ROWS/BLOCKS and ArchDetail branches; `fetchArchitecture` name consistent in mock and pane.
- **Placeholders:** Task 1's loop fixture and Task 3's IPC harness say "adapt to the module's existing fixture/harness" with binding assertions spelled out — codebase-reading instructions, not gaps. No TBDs.
