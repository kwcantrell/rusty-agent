/** Raw stats as served by the backend fixture (public/stats.json). */
export interface RawStats {
  dau: number;
  p95_ms: number;
  uptime_pct: number;
}

/** View model rendered on the /stats page. */
export interface StatsView {
  dailyActive: number;
  latencyP95: string;
  uptime: string;
}

/** TODO: map raw stats to the view per the requirements given in this session. */
export function formatStats(raw: RawStats): StatsView {
  void raw;
  return { dailyActive: 0, latencyP95: "", uptime: "" };
}
