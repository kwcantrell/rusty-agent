import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ConfigPanel } from "./ConfigPanel";
import type { RuntimeSettings } from "../../wire";

const settings: RuntimeSettings = {
  backend: "openai", base_url: "http://localhost:8080/v1", model: "m", protocol: "native",
  command_allowlist: [], command_denylist: [], temperature: 0.7, max_tokens: 1024,
  max_turns: 10, context_limit: 32768, top_p: null, top_k: null, min_p: null,
  presence_penalty: null, repeat_penalty: null, enable_thinking: true, preserve_thinking: false,
  memory: false, skills_dirs: [], active_skills: [], trace: true, trace_dir: null,
  trace_max_mb: 100, system_prompt_override: null,
};

describe("ConfigPanel", () => {
  it("shows loading before settings arrive", () => {
    render(<ConfigPanel settings={null} meta={null} error={null} disabled={false} onSave={() => {}} />);
    expect(screen.getByText(/Loading settings/)).toBeInTheDocument();
  });

  it("edits the system prompt override and saves the full settings object", () => {
    const saved: RuntimeSettings[] = [];
    render(<ConfigPanel settings={settings} meta={null} error={null} disabled={false}
      onSave={(s) => saved.push(s)} />);
    fireEvent.change(screen.getByLabelText(/Override/), { target: { value: "You are a designer." } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(saved[0].system_prompt_override).toBe("You are a designer.");
    expect(saved[0].model).toBe("m"); // full object round-trips — nothing clobbered
  });

  it("empty override saves as null", () => {
    const saved: RuntimeSettings[] = [];
    render(<ConfigPanel settings={{ ...settings, system_prompt_override: "old" }} meta={null}
      error={null} disabled={false} onSave={(s) => saved.push(s)} />);
    fireEvent.change(screen.getByLabelText(/Override/), { target: { value: "" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(saved[0].system_prompt_override).toBeNull();
  });

  it("surfaces a server rejection inline", () => {
    render(<ConfigPanel settings={settings} meta={null} error="config file changed externally — restart the daemon or re-save from the CLI"
      disabled={false} onSave={() => {}} />);
    expect(screen.getByText(/changed externally/)).toBeInTheDocument();
  });

  it("notes next-turn apply semantics", () => {
    render(<ConfigPanel settings={settings} meta={null} error={null} disabled={false} onSave={() => {}} />);
    expect(screen.getByText(/apply from the next turn/i)).toBeInTheDocument();
  });

  it("calls onLoad once on mount to fetch fresh settings", () => {
    const onLoad = vi.fn();
    const { rerender } = render(<ConfigPanel settings={null} meta={null} error={null}
      disabled={false} onSave={() => {}} onLoad={onLoad} />);
    rerender(<ConfigPanel settings={settings} meta={null} error={null}
      disabled={false} onSave={() => {}} onLoad={onLoad} />);
    expect(onLoad).toHaveBeenCalledTimes(1);
  });
});
