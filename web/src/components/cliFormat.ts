const ARG_MAX = 60;
const RESULT_MAX = 80;

function truncate(s: string, max: number): string {
  return s.length > max ? s.slice(0, max - 1) + "…" : s;
}

function firstLine(s: string): string {
  return s.trim().split("\n")[0];
}

/** First string value in a tool's args, for the `Name(arg)` header line. */
export function argSummary(args: unknown): string | null {
  if (typeof args === "string" && args.trim() !== "") return truncate(firstLine(args), ARG_MAX);
  if (args && typeof args === "object" && !Array.isArray(args)) {
    for (const v of Object.values(args)) {
      if (typeof v === "string" && v.trim() !== "") return truncate(firstLine(v), ARG_MAX);
    }
  }
  return null;
}

/** One-line ⎿ summary of a tool result's raw content. */
export function resultSummary(content: string | undefined, resultStatus: string | undefined): string {
  const failed = !!resultStatus && resultStatus !== "ok";
  const lines = (content ?? "").split("\n").filter((l) => l.trim() !== "");
  if (lines.length === 0) return failed ? "error" : "done";
  const first = truncate(lines[0].trim(), RESULT_MAX);
  return lines.length > 1 ? `${first} (+${lines.length - 1} lines)` : first;
}

/** 10-cell context gauge: ▂ filled, ░ empty. */
export function blockMeter(pct: number): string {
  const filled = Math.max(0, Math.min(10, Math.round(pct / 10)));
  return "▂".repeat(filled) + "░".repeat(10 - filled);
}
