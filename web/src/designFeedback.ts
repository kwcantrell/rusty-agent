import type { Pin } from "./designStore";

/**
 * FROZEN CONTRACT (B-migration): this JSON shape becomes the DesignFeedback
 * tool-result payload when the first-class design channel lands. Existing field
 * names and structure must not change — the golden tests pin the exact output.
 * `url` (optional) identifies the live page a url-version's pins refer to;
 * pins on live apps are viewport-relative (iframe scroll is invisible cross-origin).
 */
export function buildFeedbackMessage(designId: string, version: number, pins: Pin[], note?: string, url?: string): string {
  const payload: Record<string, unknown> = { design_id: designId, version, pins };
  if (note !== undefined && note.trim().length > 0) payload.note = note;
  if (url !== undefined) payload.url = url;
  return `Design feedback on ${designId} (v${version}):\n\n\`\`\`design-feedback\n${JSON.stringify(payload, null, 2)}\n\`\`\``;
}
