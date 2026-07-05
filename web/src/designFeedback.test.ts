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
});
