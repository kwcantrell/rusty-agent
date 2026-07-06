import React, { useState } from "react";
import type { RuntimeSettings } from "../wire";

export interface SettingsMeta {
  workspace: string;
  apiKeySet: boolean;
  hardFloor: string[];
  discoveredSkills: { name: string; description: string }[];
}

const toLines = (xs: string[]) => xs.join("\n");
const fromLines = (s: string) => s.split("\n").map((l) => l.trim()).filter((l) => l.length > 0);

export function SettingsForm({ settings, meta, error, disabled, onSave }: {
  settings: RuntimeSettings;
  meta: SettingsMeta | null;
  error: string | null;
  disabled: boolean;
  onSave: (s: RuntimeSettings) => void;
}) {
  const [form, setForm] = useState<RuntimeSettings>(settings);
  const [allow, setAllow] = useState(toLines(settings.command_allowlist));
  const [deny, setDeny] = useState(toLines(settings.command_denylist));
  const [skillsDirs, setSkillsDirs] = useState(toLines(settings.skills_dirs));

  const set = <K extends keyof RuntimeSettings>(k: K, v: RuntimeSettings[K]) =>
    setForm((f) => ({ ...f, [k]: v }));

  const toggleSkill = (name: string) =>
    setForm((f) => ({ ...f, active_skills: f.active_skills.includes(name)
      ? f.active_skills.filter((n) => n !== name)
      : [...f.active_skills, name] }));

  const save = () => onSave({ ...form, command_allowlist: fromLines(allow),
    command_denylist: fromLines(deny), skills_dirs: fromLines(skillsDirs) });

  const num = (k: keyof RuntimeSettings) => (e: React.ChangeEvent<HTMLInputElement>) =>
    set(k, (e.target.value === "" ? null : Number(e.target.value)) as RuntimeSettings[typeof k]);
  const numVal = (v: number | null) => (v === null ? "" : v);

  const floor = meta?.hardFloor ?? [];
  const redundant = fromLines(deny).filter((d) => floor.includes(d));

  const field = "w-full rounded bg-[var(--surface-base)] px-2 py-1 text-sm text-[var(--text-strong)] border border-[var(--border)]";
  const label = "block text-xs uppercase tracking-wide text-[var(--text-muted)] mb-1";

  return (
    <>
      {error && <div className="mb-3 rounded px-2 py-1 text-sm"
        style={{ background: "var(--surface-raised)", color: "var(--state-error)", border: "1px solid var(--state-error)" }}>{error}</div>}

      <section className="mb-4 space-y-3">
        <h3 className="text-sm font-semibold text-[var(--text-strong)]">Model &amp; inference</h3>
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
        <h3 className="text-sm font-semibold text-[var(--text-strong)]">System prompt</h3>
        <div>
          <label className={label} htmlFor="system_prompt_override">Override (empty = built-in prompt)</label>
          <textarea id="system_prompt_override" rows={6} className={field}
            value={form.system_prompt_override ?? ""}
            onChange={(e) => set("system_prompt_override", e.target.value === "" ? null : e.target.value)} />
          <p className="mt-1 text-xs text-[var(--text-muted)]">
            Replaces the built-in base prompt; active skills still append on top.
          </p>
        </div>
      </section>

      <section className="mb-4 space-y-3">
        <h3 className="text-sm font-semibold text-[var(--text-strong)]">Command policy</h3>
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
          <p className="text-xs text-[var(--text-muted)]">
            Always blocked (hard floor): {meta.hardFloor.join(", ")}
          </p>
        )}
        {redundant.length > 0 && (
          <p className="text-xs text-[var(--accent-2)]">
            Redundant — already in the hard floor: {redundant.join(", ")}
          </p>
        )}
      </section>

      <section className="mb-4 space-y-3">
        <h3 className="text-sm font-semibold text-[var(--text-strong)]">Loop tuning</h3>
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
        <h3 className="text-sm font-semibold text-[var(--text-strong)]">Sampling &amp; thinking</h3>
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
        <label className="flex items-center gap-2 text-sm">
          <input id="memory" type="checkbox" checked={form.memory}
            onChange={(e) => set("memory", e.target.checked)} />
          Long-term memory (remember/recall across sessions)
        </label>
      </section>

      <section className="mb-4 space-y-3">
        <h3 className="text-sm font-semibold text-[var(--text-strong)]">Skills</h3>
        <div>
          <label className={label} htmlFor="skills_dirs">Skill directories (one per line)</label>
          <textarea id="skills_dirs" rows={3} className={field} value={skillsDirs}
            onChange={(e) => setSkillsDirs(e.target.value)} />
          <p className="mt-1 text-xs text-[var(--text-muted)]">
            Save directories, then the skills they contain appear below to activate.
          </p>
        </div>
        <div>
          <span className={label}>Active skills</span>
          {(meta?.discoveredSkills ?? []).length === 0 ? (
            <p className="text-xs text-[var(--text-muted)]">No skills found in the configured directories.</p>
          ) : (
            <ul className="space-y-1">
              {meta!.discoveredSkills.map((s) => (
                <li key={s.name}>
                  <label className="flex items-start gap-2 text-sm">
                    <input type="checkbox" className="mt-1"
                      checked={form.active_skills.includes(s.name)}
                      onChange={() => toggleSkill(s.name)} />
                    <span><span className="text-[var(--text-strong)]">{s.name}</span>
                      <span className="block text-xs text-[var(--text-muted)]">{s.description}</span></span>
                  </label>
                </li>
              ))}
            </ul>
          )}
        </div>
      </section>

      {meta && (
        <section className="mb-4 text-xs text-[var(--text-muted)]">
          <p>Workspace: <span>{meta.workspace}</span></p>
          <p>API key: <span>{meta.apiKeySet ? "set" : "not set"}</span></p>
        </section>
      )}

      <button onClick={save} disabled={disabled}
        className="w-full rounded px-3 py-2 text-sm font-medium hover:opacity-90 disabled:opacity-40"
        style={{ background: "var(--accent)", color: "var(--accent-fg)" }}>
        Save
      </button>
    </>
  );
}
