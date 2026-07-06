import type { Pin } from "./designStore";

/**
 * FROZEN CONTRACT (B-migration): this JSON shape becomes the DesignFeedback
 * tool-result payload when the first-class design channel lands. Field names
 * and structure must not change — the golden test pins the exact output.
 */
export function buildFeedbackMessage(designId: string, version: number, pins: Pin[], note?: string): string {
  const payload: Record<string, unknown> = { design_id: designId, version, pins };
  if (note !== undefined && note.trim().length > 0) payload.note = note;
  return `Design feedback on ${designId} (v${version}):\n\n\`\`\`design-feedback\n${JSON.stringify(payload, null, 2)}\n\`\`\``;
}
