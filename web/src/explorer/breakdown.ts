import type { ContextSnapshot } from "./types";

export interface Slice { category: string; tokens: number; pct: number }
export interface Breakdown { total: number; slices: Slice[] }

export function computeBreakdown(snap: ContextSnapshot, realTotal: number | null): Breakdown {
  const estTotal = snap.est_total;
  const total = realTotal && realTotal > estTotal ? realTotal : estTotal;
  const slices: Slice[] = snap.segments.map((s) => ({
    category: s.category, tokens: s.est_tokens, pct: 0,
  }));
  if (realTotal && realTotal > estTotal) {
    slices.push({ category: "unattributed", tokens: realTotal - estTotal, pct: 0 });
  }
  const denom = total || 1;
  for (const s of slices) s.pct = Math.round((s.tokens / denom) * 100);
  return { total, slices };
}
