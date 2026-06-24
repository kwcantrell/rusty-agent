import { describe, it, expect } from "vitest";
import { parseInbound } from "../src/wire";

describe("Display variants over the wire", () => {
  it("parses a tool_result carrying a Markdown artifact", () => {
    const raw = JSON.stringify({
      v: 1, session_id: "s", kind: "event",
      payload: { type: "tool_result", name: "render", content: "rendered markdown",
        display: { Markdown: { text: "# Hi", title: "Plan" } } },
    });
    const msg = parseInbound(raw);
    expect(msg?.kind).toBe("event");
    if (msg?.kind === "event" && msg.payload.type === "tool_result") {
      expect(msg.payload.display).toEqual({ Markdown: { text: "# Hi", title: "Plan" } });
    } else {
      throw new Error("expected a tool_result event");
    }
  });
});
