import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { SettingsPanel } from "./SettingsPanel";
import type { RuntimeSettings } from "../wire";

const base: RuntimeSettings = {
  backend: "openai", base_url: "http://x", model: "m", protocol: "native",
  command_allowlist: [], command_denylist: [], temperature: 0.2, max_tokens: 2048,
  max_turns: 25, context_limit: 8192,
  top_p: null, top_k: null, min_p: null, presence_penalty: null, repeat_penalty: null,
  enable_thinking: true, preserve_thinking: false, memory: true,
  skills_dirs: [], active_skills: [],
  trace: false, trace_dir: null, trace_max_mb: 64,
  system_prompt_override: null,
};

describe("SettingsPanel skills", () => {
  const meta = { workspace: "/w", apiKeySet: false, hardFloor: [],
    discoveredSkills: [{ name: "greeter", description: "says hi" }] };

  it("checks an active skill and saves it in active_skills", () => {
    const onSave = vi.fn();
    render(<SettingsPanel settings={base} meta={meta} error={null} disabled={false}
      onSave={onSave} onClose={() => {}} />);
    fireEvent.click(screen.getByLabelText(/greeter/));
    fireEvent.click(screen.getByText("Save"));
    expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ active_skills: ["greeter"] }));
  });

  it("round-trips edited skill directories", () => {
    const onSave = vi.fn();
    render(<SettingsPanel settings={base} meta={meta} error={null} disabled={false}
      onSave={onSave} onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("Skill directories (one per line)"),
      { target: { value: "/a\n/b" } });
    fireEvent.click(screen.getByText("Save"));
    expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ skills_dirs: ["/a", "/b"] }));
  });
});

describe("SettingsPanel sampling inputs", () => {
  it("maps empty top_p to null and a typed value to a number on save", () => {
    const onSave = vi.fn();
    render(<SettingsPanel settings={base} meta={null} error={null} disabled={false}
      onSave={onSave} onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("Top-p"), { target: { value: "0.9" } });
    fireEvent.click(screen.getByText("Save"));
    expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ top_p: 0.9, top_k: null }));
  });

  it("toggles enable_thinking", () => {
    const onSave = vi.fn();
    render(<SettingsPanel settings={base} meta={null} error={null} disabled={false}
      onSave={onSave} onClose={() => {}} />);
    fireEvent.click(screen.getByLabelText("Enable thinking"));
    fireEvent.click(screen.getByText("Save"));
    expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ enable_thinking: false }));
  });

  it("toggles memory off and saves it", () => {
    const onSave = vi.fn();
    render(<SettingsPanel settings={base} meta={null} error={null} disabled={false}
      onSave={onSave} onClose={() => {}} />);
    fireEvent.click(screen.getByLabelText("Long-term memory — project memory files (memories/project/)"));
    fireEvent.click(screen.getByText("Save"));
    expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ memory: false }));
  });
});
