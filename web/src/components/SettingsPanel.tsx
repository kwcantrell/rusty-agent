import React, { useState } from "react";
import type { RuntimeSettings } from "../wire";

interface Meta { workspace: string; apiKeySet: boolean; hardFloor: string[] }

interface Props {
  settings: RuntimeSettings;
  meta: Meta | null;
  error: string | null;
  disabled: boolean;
  onSave: (s: RuntimeSettings) => void;
  onClose: () => void;
}

const toLines = (xs: string[]) => xs.join("\n");
const fromLines = (s: string) => s.split("\n").map((l) => l.trim()).filter((l) => l.length > 0);

export function SettingsPanel({ settings, meta, error, disabled, onSave, onClose }: Props) {
  const [form, setForm] = useState<RuntimeSettings>(settings);
  const [allow, setAllow] = useState(toLines(settings.command_allowlist));
  const [deny, setDeny] = useState(toLines(settings.command_denylist));

  const set = <K extends keyof RuntimeSettings>(k: K, v: RuntimeSettings[K]) =>
    setForm((f) => ({ ...f, [k]: v }));

  const save = () => onSave({ ...form, command_allowlist: fromLines(allow), command_denylist: fromLines(deny) });

  const num = (k: keyof RuntimeSettings) => (e: React.ChangeEvent<HTMLInputElement>) =>
    set(k, (e.target.value === "" ? null : Number(e.target.value)) as RuntimeSettings[typeof k]);
  const numVal = (v: number | null) => (v === null ? "" : v);

  const floor = meta?.hardFloor ?? [];
  const redundant = fromLines(deny).filter((d) => floor.includes(d));

  const field = "w-full rounded bg-zinc-800 px-2 py-1 text-sm text-zinc-100";
  const label = "block text-xs uppercase tracking-wide text-zinc-400 mb-1";

  return (
    <div className="absolute inset-0 z-10 flex justify-end bg-black/50">
      <div className="h-full w-96 overflow-y-auto bg-zinc-900 p-4 text-zinc-200 shadow-xl">
        <div className="mb-4 flex items-center justify-between">
          <h2 className="text-lg font-semibold">Settings</h2>
          <button onClick={onClose} className="text-zinc-400 hover:text-zinc-200">close</button>
        </div>

        {error && <div className="mb-3 rounded bg-red-900/50 px-2 py-1 text-sm text-red-200">{error}</div>}

        <section className="mb-4 space-y-3">
          <h3 className="text-sm font-semibold text-zinc-300">Model &amp; inference</h3>
          <div>
            <label className={label} htmlFor="backend">Backend</label>
            <select id="backend" className={field} value={form.backend}
              onChange={(e) => set("backend", e.target.value)}>
              <option value="openai">openai</option>
              <option value="claude-cli">claude-cli</option>
            </select>
          </div>
          <div>
            <label className={label} htmlFor="base_url">Base URL</label>
            <input id="base_url" className={field} value={form.base_url}
              onChange={(e) => set("base_url", e.target.value)} />
          </div>
          <div>
            <label className={label} htmlFor="model">Model</label>
            <input id="model" className={field} value={form.model}
              onChange={(e) => set("model", e.target.value)} />
          </div>
          <div>
            <label className={label} htmlFor="protocol">Protocol</label>
            <select id="protocol" className={field} value={form.protocol}
              onChange={(e) => set("protocol", e.target.value)}>
              <option value="native">native</option>
              <option value="prompted">prompted</option>
            </select>
          </div>
        </section>

        <section className="mb-4 space-y-3">
          <h3 className="text-sm font-semibold text-zinc-300">Command policy</h3>
          <div>
            <label className={label} htmlFor="allowlist">Allowlist (one per line)</label>
            <textarea id="allowlist" rows={4} className={field} value={allow}
              onChange={(e) => setAllow(e.target.value)} />
          </div>
          <div>
            <label className={label} htmlFor="denylist">Denylist (one per line)</label>
            <textarea id="denylist" rows={3} className={field} value={deny}
              onChange={(e) => setDeny(e.target.value)} />
          </div>
          {meta && (
            <p className="text-xs text-zinc-500">
              Always blocked (hard floor): {meta.hardFloor.join(", ")}
            </p>
          )}
          {redundant.length > 0 && (
            <p className="text-xs text-amber-400/80">
              Redundant — already in the hard floor: {redundant.join(", ")}
            </p>
          )}
        </section>

        <section className="mb-4 space-y-3">
          <h3 className="text-sm font-semibold text-zinc-300">Loop tuning</h3>
          <div>
            <label className={label} htmlFor="temperature">Temperature</label>
            <input id="temperature" type="number" step="0.1" className={field} value={form.temperature}
              onChange={(e) => set("temperature", Number(e.target.value))} />
          </div>
          <div>
            <label className={label} htmlFor="max_tokens">Max tokens</label>
            <input id="max_tokens" type="number" className={field} value={form.max_tokens}
              onChange={(e) => set("max_tokens", Number(e.target.value))} />
          </div>
          <div>
            <label className={label} htmlFor="max_turns">Max turns</label>
            <input id="max_turns" type="number" className={field} value={form.max_turns}
              onChange={(e) => set("max_turns", Number(e.target.value))} />
          </div>
          <div>
            <label className={label} htmlFor="context_limit">Context limit</label>
            <input id="context_limit" type="number" className={field} value={form.context_limit}
              onChange={(e) => set("context_limit", Number(e.target.value))} />
          </div>
        </section>

        <section className="mb-4 space-y-3">
          <h3 className="text-sm font-semibold text-zinc-300">Sampling &amp; thinking</h3>
          <div>
            <label className={label} htmlFor="top_p">Top-p</label>
            <input id="top_p" type="number" step="0.05" className={field}
              value={numVal(form.top_p)} onChange={num("top_p")} />
          </div>
          <div>
            <label className={label} htmlFor="top_k">Top-k</label>
            <input id="top_k" type="number" className={field}
              value={numVal(form.top_k)} onChange={num("top_k")} />
          </div>
          <div>
            <label className={label} htmlFor="min_p">Min-p</label>
            <input id="min_p" type="number" step="0.01" className={field}
              value={numVal(form.min_p)} onChange={num("min_p")} />
          </div>
          <div>
            <label className={label} htmlFor="presence_penalty">Presence penalty</label>
            <input id="presence_penalty" type="number" step="0.1" className={field}
              value={numVal(form.presence_penalty)} onChange={num("presence_penalty")} />
          </div>
          <div>
            <label className={label} htmlFor="repeat_penalty">Repeat penalty</label>
            <input id="repeat_penalty" type="number" step="0.05" className={field}
              value={numVal(form.repeat_penalty)} onChange={num("repeat_penalty")} />
          </div>
          <label className="flex items-center gap-2 text-sm">
            <input id="enable_thinking" type="checkbox" checked={form.enable_thinking}
              onChange={(e) => set("enable_thinking", e.target.checked)} />
            Enable thinking
          </label>
          <label className="flex items-center gap-2 text-sm">
            <input id="preserve_thinking" type="checkbox" checked={form.preserve_thinking}
              onChange={(e) => set("preserve_thinking", e.target.checked)} />
            Preserve thinking in history
          </label>
        </section>

        {meta && (
          <section className="mb-4 text-xs text-zinc-500">
            <p>Workspace: <span>{meta.workspace}</span></p>
            <p>API key: <span>{meta.apiKeySet ? "set" : "not set"}</span></p>
          </section>
        )}

        <button onClick={save} disabled={disabled}
          className="w-full rounded bg-blue-600 px-3 py-2 text-sm font-medium text-white hover:bg-blue-500 disabled:opacity-40">
          Save
        </button>
      </div>
    </div>
  );
}
