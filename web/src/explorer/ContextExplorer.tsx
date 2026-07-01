import { useEffect, useState } from "react";
import type { ContextSnapshot } from "./types";
import { getContext } from "./api";
import { computeBreakdown } from "./breakdown";
import { MemorySection } from "./MemorySection";
import { SkillSection } from "./SkillSection";

const COLORS: Record<string, string> = {
  system: "var(--accent)", goal: "var(--ctx-goal)", memory: "var(--ctx-memory)",
  summary: "var(--ctx-summary)", messages: "var(--text-muted)", unattributed: "var(--state-error)",
};

export function ContextExplorer(
  { realTotal, refreshKey, skills, lastQuery }: {
    realTotal: number | null;
    refreshKey: number;
    skills: { name: string; description: string }[];
    lastQuery: string | null;
  },
) {
  const [snap, setSnap] = useState<ContextSnapshot | null>(null);
  const [open, setOpen] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    getContext().then((s) => { if (active) setSnap(s); }).catch(() => {});
    return () => { active = false; };
  }, [refreshKey]);

  if (!snap) {
    return <div className="p-3 text-xs" style={{ color: "var(--text-muted)" }}>No context yet.</div>;
  }
  const b = computeBreakdown(snap, realTotal);
  const openSeg = open !== null ? (snap.segments.find((s) => s.category === open) ?? null) : null;

  return (
    <div className="flex h-full flex-col overflow-y-auto" style={{ background: "var(--surface-overlay)" }}>
      <div className="px-3 pt-3">
        <div className="font-mono text-xs" style={{ color: "var(--text-strong)" }}>
          {b.total} / {snap.model_limit} tokens
        </div>
        <div className="mt-2 flex h-3 w-full overflow-hidden rounded-full"
          style={{ background: "var(--surface-base)" }}>
          {b.slices.map((s) => (
            <div key={s.category} title={`${s.category}: ${s.tokens} (${s.pct}%)`}
              style={{ width: `${s.pct}%`, background: COLORS[s.category] ?? "var(--text-muted)" }} />
          ))}
        </div>
        <div className="mt-2 flex flex-wrap gap-2 text-xs" style={{ color: "var(--text-muted)" }}>
          {b.slices.map((s) => (
            <button key={s.category} onClick={() => setOpen(open === s.category ? null : s.category)}
              className="flex items-center gap-1">
              <span className="inline-block h-2 w-2 rounded-full"
                style={{ background: COLORS[s.category] ?? "var(--text-muted)" }} />
              {s.category} {s.tokens}
            </button>
          ))}
        </div>
        {open !== null && (
          <div className="mt-2 rounded p-2 text-xs"
            style={{ background: "var(--surface-base)", border: "1px solid var(--border)" }}>
            {openSeg ? (
              <>
                <div style={{ color: "var(--text-muted)" }}>
                  {openSeg.category} — {openSeg.count} item{openSeg.count !== 1 ? "s" : ""}
                </div>
                {openSeg.items.length === 0
                  ? <div style={{ color: "var(--text-muted)" }}>— none —</div>
                  : openSeg.items.map((item, i) => (
                      <div key={i} style={{ color: "var(--text-strong)" }}>· {item}</div>
                    ))
                }
              </>
            ) : (
              <div style={{ color: "var(--text-muted)" }}>
                Gap between server total and estimated sum
                ({b.slices.find((s) => s.category === "unattributed")?.tokens ?? 0} tokens unaccounted)
              </div>
            )}
          </div>
        )}
      </div>

      <div className="mt-3 border-t" style={{ borderColor: "var(--border)" }}>
        <MemorySection
          recalled={snap.segments.find((x) => x.category === "memory")?.items ?? []}
          lastQuery={lastQuery}
        />
        <SkillSection skills={skills} />
      </div>
    </div>
  );
}
