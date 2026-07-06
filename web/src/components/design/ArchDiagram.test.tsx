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

  it("arrows sit between rows, not inside them", () => {
    render(<ArchDiagram snapshot={fixture} selected={null} onSelect={() => {}} />);
    const loopBtn = screen.getByRole("button", { name: /Agent Loop/ });
    const arrows = screen.getAllByText("↓");
    expect(arrows).toHaveLength(2);
    for (const a of arrows) expect(a.closest("div")).not.toBe(loopBtn.parentElement);
  });
});
