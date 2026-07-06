import type { ArchitectureSnapshot } from "./architecture";

export const archFixture: ArchitectureSnapshot = {
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
