/** True only for http(s) URLs whose host is the local machine. Mirrors the Rust
 *  tool-side guard (render.rs validate_local_url); both must hold — fail closed. */
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
