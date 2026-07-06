import { describe, it, expect } from "vitest";
import { buildFeedbackMessage } from "./designFeedback";

describe("buildFeedbackMessage", () => {
  it("serializes the exact frozen contract", () => {
    const msg = buildFeedbackMessage("design:landing", 3,
      [{ x_pct: 0.42, y_pct: 0.105, comment: "make the logo bigger" }],
      "overall: tighten vertical spacing");
    expect(msg).toBe(`Design feedback on design:landing (v3):

\`\`\`design-feedback
{
  "design_id": "design:landing",
  "version": 3,
  "pins": [
    {
      "x_pct": 0.42,
      "y_pct": 0.105,
      "comment": "make the logo bigger"
    }
  ],
  "note": "overall: tighten vertical spacing"
}
\`\`\``);
  });

  it("omits note when absent", () => {
    const msg = buildFeedbackMessage("design:x", 1, [{ x_pct: 0.5, y_pct: 0.5, comment: "c" }]);
    expect(msg).not.toContain('"note"');
    expect(msg).toContain('"version": 1');
  });

  it("includes the live url for url-version feedback (frozen-contract extension)", () => {
    const msg = buildFeedbackMessage("design:app", 2,
      [{ x_pct: 0.1, y_pct: 0.9, comment: "nav overlaps logo" }],
      undefined, "http://localhost:5173/settings");
    expect(msg).toBe(`Design feedback on design:app (v2):

\`\`\`design-feedback
{
  "design_id": "design:app",
  "version": 2,
  "pins": [
    {
      "x_pct": 0.1,
      "y_pct": 0.9,
      "comment": "nav overlaps logo"
    }
  ],
  "url": "http://localhost:5173/settings"
}
\`\`\``);
  });

  it("omits url when absent (existing golden unchanged)", () => {
    const msg = buildFeedbackMessage("design:x", 1, [{ x_pct: 0.5, y_pct: 0.5, comment: "c" }]);
    expect(msg).not.toContain('"url"');
  });
});
