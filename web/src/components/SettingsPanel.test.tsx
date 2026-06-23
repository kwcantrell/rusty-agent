import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { SettingsPanel } from "./SettingsPanel";
import type { RuntimeSettings } from "../wire";

const base: RuntimeSettings = {
  backend: "openai", base_url: "http://x", model: "m", protocol: "native",
  command_allowlist: [], command_denylist: [], temperature: 0.2, max_tokens: 2048,
  max_turns: 25, context_limit: 8192,
  top_p: null, top_k: null, min_p: null, presence_penalty: null, repeat_penalty: null,
  enable_thinking: true, preserve_thinking: false,
};

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
});
