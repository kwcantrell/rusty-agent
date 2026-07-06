import { useEffect, useMemo, useState } from "react";
import type { Display } from "./wire";
import type { Item } from "./state";

export const MAX_VERSIONS = 20;

export interface Pin { x_pct: number; y_pct: number; comment: string }
export interface DesignVersion { display: Display; renderable: boolean }
export interface Design { id: string; title: string; versions: DesignVersion[] }

/** Variant keys ArtifactRenderer knows how to draw (mirror of its branches). */
const RENDERABLE = new Set(["Text", "Markdown", "Code", "Diff", "Terminal", "Table", "Image", "Html", "Mermaid"]);

/** The design id ("design:<name>") when this display targets the canvas, else null. */
export function displayDesignId(d: Display): string | null {
  const v = Object.values(d)[0] as { id?: string } | string;
  const id = v && typeof v === "object" ? v.id : undefined;
  return id !== undefined && id.startsWith("design:") ? id : null;
}

function displayTitle(d: Display): string | undefined {
  const v = Object.values(d)[0] as { title?: string } | string;
  return v && typeof v === "object" ? v.title : undefined;
}

function cap(d: Design): Design {
  return d.versions.length <= MAX_VERSIONS
    ? d
    : { ...d, versions: d.versions.slice(d.versions.length - MAX_VERSIONS) };
}

/** Pure derivation: every tool display with a design: id becomes a version, in order. */
export function designsFrom(items: Item[]): Design[] {
  const map = new Map<string, Design>();
  for (const it of items) {
    if (it.kind !== "tool" || !it.display) continue;
    const id = displayDesignId(it.display);
    if (!id) continue;
    const cur = map.get(id) ?? { id, title: id, versions: [] };
    cur.versions.push({
      display: it.display,
      renderable: RENDERABLE.has(Object.keys(it.display)[0]),
    });
    cur.title = displayTitle(it.display) ?? cur.title;
    map.set(id, cur);
  }
  return [...map.values()].map(cap);
}

/** Stored history (frozen at mount) followed by live-derived versions, capped.
 *  Uses multiset dedup: each stored version "consumes" one identical live version
 *  (by JSON-serialized display key), so remounting with the same items never
 *  duplicates the stack, but legitimately repeated identical renders are preserved. */
export function mergeDesigns(stored: Design[], live: Design[]): Design[] {
  const out = new Map<string, Design>(stored.map((d) => [d.id, d]));
  for (const l of live) {
    const s = out.get(l.id);
    if (!s) { out.set(l.id, l); continue; }
    // Build a multiset occurrence count of stored versions by serialized display.
    const counts = new Map<string, number>();
    for (const v of s.versions) {
      const key = JSON.stringify(v.display);
      counts.set(key, (counts.get(key) ?? 0) + 1);
    }
    // Append only live versions not already covered by a stored occurrence.
    const novel: DesignVersion[] = [];
    for (const v of l.versions) {
      const key = JSON.stringify(v.display);
      const n = counts.get(key) ?? 0;
      if (n > 0) { counts.set(key, n - 1); } else { novel.push(v); }
    }
    out.set(l.id, cap({ ...s, versions: [...s.versions, ...novel] }));
  }
  return [...out.values()];
}

interface StoredBlob { designs: Design[]; sent: Record<string, Pin[]> }
const KEY = (sid: string) => `agent.designs.${sid}`;

function loadBlob(sid: string): StoredBlob {
  try {
    const raw = localStorage.getItem(KEY(sid));
    if (!raw) return { designs: [], sent: {} };
    const v = JSON.parse(raw) as Partial<StoredBlob>;
    return { designs: Array.isArray(v.designs) ? v.designs : [], sent: v.sent ?? {} };
  } catch { return { designs: [], sent: {} }; }
}

function saveBlob(sid: string, blob: StoredBlob): void {
  try { localStorage.setItem(KEY(sid), JSON.stringify(blob)); } catch { /* in-memory only */ }
}

export interface DesignStoreApi {
  designs: Design[];
  sentPins: (designId: string, version: number) => Pin[];
  recordSent: (designId: string, version: number, pins: Pin[]) => void;
}

/**
 * DesignStore v1: stored history is FROZEN at mount (so live derivation never
 * double-counts), merged with live items, mirrored back to localStorage.
 * The B migration swaps this hook's internals for a server-backed store.
 */
export function useDesignStore(items: Item[], sessionId: string): DesignStoreApi {
  // eslint-disable-next-line react-hooks/exhaustive-deps -- blob frozen per (mount, sessionId)
  const stored = useMemo(() => loadBlob(sessionId), [sessionId]);
  const [seededFor, setSeededFor] = useState(sessionId);
  const [sent, setSent] = useState<Record<string, Pin[]>>(stored.sent);
  if (seededFor !== sessionId) {
    setSeededFor(sessionId);
    setSent(stored.sent);
  }
  const designs = useMemo(() => mergeDesigns(stored.designs, designsFrom(items)), [stored, items]);

  useEffect(() => { saveBlob(sessionId, { designs, sent }); }, [sessionId, designs, sent]);

  return {
    designs,
    sentPins: (id, version) => sent[`${id}@${version}`] ?? [],
    recordSent: (id, version, pins) =>
      setSent((s) => ({ ...s, [`${id}@${version}`]: [...(s[`${id}@${version}`] ?? []), ...pins] })),
  };
}
