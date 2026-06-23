import { render, screen, fireEvent } from "@testing-library/react";
import { SettingsPanel } from "../src/components/SettingsPanel";
import type { RuntimeSettings } from "../src/wire";

const settings: RuntimeSettings = {
  backend: "openai", base_url: "http://localhost:8080", model: "qwen", protocol: "native",
  command_allowlist: ["ls", "git"], command_denylist: ["foo"], temperature: 0.2,
  max_tokens: 2048, max_turns: 25, context_limit: 8192,
};
const meta = { workspace: "/home/me/proj", apiKeySet: true, hardFloor: ["sudo", "rm -rf /"] };

test("renders fields and read-only metadata", () => {
  render(<SettingsPanel settings={settings} meta={meta} error={null} disabled={false}
    onSave={() => {}} onClose={() => {}} />);
  expect(screen.getByLabelText(/model/i)).toHaveValue("qwen");
  expect(screen.getByText("/home/me/proj")).toBeInTheDocument();
  expect(screen.getByText(/sudo/)).toBeInTheDocument();
  expect(screen.getByText(/api key/i)).toBeInTheDocument();
});

test("editing the model and saving emits the updated settings", () => {
  const onSave = vi.fn();
  render(<SettingsPanel settings={settings} meta={meta} error={null} disabled={false}
    onSave={onSave} onClose={() => {}} />);
  fireEvent.change(screen.getByLabelText(/model/i), { target: { value: "new-model" } });
  fireEvent.click(screen.getByRole("button", { name: /save/i }));
  expect(onSave).toHaveBeenCalledTimes(1);
  expect(onSave.mock.calls[0][0].model).toBe("new-model");
});

test("textareas convert newline lists back to arrays on save", () => {
  const onSave = vi.fn();
  render(<SettingsPanel settings={settings} meta={meta} error={null} disabled={false}
    onSave={onSave} onClose={() => {}} />);
  fireEvent.change(screen.getByLabelText(/allowlist/i), { target: { value: "ls\ncat\n" } });
  fireEvent.click(screen.getByRole("button", { name: /save/i }));
  expect(onSave.mock.calls[0][0].command_allowlist).toEqual(["ls", "cat"]);
});

test("shows an error message", () => {
  render(<SettingsPanel settings={settings} meta={meta} error="bad base_url" disabled={false}
    onSave={() => {}} onClose={() => {}} />);
  expect(screen.getByText("bad base_url")).toBeInTheDocument();
});

test("flags denylist entries that are redundant with the hard floor", () => {
  const overlapping: RuntimeSettings = { ...settings, command_denylist: ["sudo", "mine"] };
  render(<SettingsPanel settings={overlapping} meta={meta} error={null} disabled={false}
    onSave={() => {}} onClose={() => {}} />);
  // "sudo" is in meta.hardFloor → redundant; "mine" is not → no warning about it.
  const note = screen.getByText(/already in the hard floor/i);
  expect(note).toHaveTextContent("sudo");
  expect(note).not.toHaveTextContent("mine");
});

test("shows no redundancy note when the denylist and hard floor are disjoint", () => {
  render(<SettingsPanel settings={settings} meta={meta} error={null} disabled={false}
    onSave={() => {}} onClose={() => {}} />);
  expect(screen.queryByText(/already in the hard floor/i)).not.toBeInTheDocument();
});

test("save is disabled when offline", () => {
  render(<SettingsPanel settings={settings} meta={meta} error={null} disabled={true}
    onSave={() => {}} onClose={() => {}} />);
  expect(screen.getByRole("button", { name: /save/i })).toBeDisabled();
});
