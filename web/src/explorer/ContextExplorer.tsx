import { useEffect, useState } from "react";
import type { ContextSnapshot } from "./types";
import { getContext } from "./api";
import { computeBreakdown } from "./breakdown";
import { MemorySection } from "./MemorySection";
import { SkillSection } from "./SkillSection";

const COLORS: Record<string, string> = {
  system: "var(--accent)", goal: "#a78bfa", memory: "#34d399",
  summary: "#fbbf24", messages: "var(--text-muted)", unattributed: "var(--state-error)",
};

export function ContextExplorer(
  { realTotal, refreshKey }: { realTotal: number | null; refreshKey: number },
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
      </div>

      <div className="mt-3 border-t" style={{ borderColor: "var(--border)" }}>
        <MemorySection
          recalled={snap.segments.find((x) => x.category === "memory")?.items ?? []}
        />
        <SkillSection />
      </div>
    </div>
  );
}
