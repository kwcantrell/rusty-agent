/**
 * True only for http(s) URLs whose host is the local machine.
 *
 * Guard-parity note: this JS guard is the authoritative one — it uses the WHATWG
 * `URL` parser (the same engine the browser uses to actually connect), which
 * strips userinfo and normalises the host before we read `hostname`.  The Rust
 * `validate_local_url` (render.rs) is a coarser literal-string matcher that acts
 * as a first line of defence on the agent side; it rejects obvious non-local
 * targets (including userinfo) but cannot replicate full WHATWG normalisation.
 * Both guards must pass — fail closed.
 */
export function isLocalUrl(raw: string): boolean {
  try {
    const u = new URL(raw);
    const local = ["localhost", "127.0.0.1", "[::1]", "::1"];
    return (u.protocol === "http:" || u.protocol === "https:") && local.includes(u.hostname);
  } catch { return false; }
}

/** An https-served page cannot embed an http iframe (browser mixed-content block). */
export function isMixedContent(raw: string, pageProtocol: string = window.location.protocol): boolean {
  try { return pageProtocol === "https:" && new URL(raw).protocol === "http:"; } catch { return true; }
}
