import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { ActivityRail } from "../src/components/ActivityRail";
import { Inspector } from "../src/components/inspector/Inspector";
import { artifactsFrom, type Item } from "../src/state";

// Shell wiring is integration-level; this test asserts the pieces compose:
// artifactsFrom feeds Inspector, and ActivityRail + Inspector render side by side.
describe("workbench shell pieces compose", () => {
  it("derives artifacts and feeds them to the Inspector", () => {
    const items: Item[] = [
      { kind: "tool", name: "render", args: {}, status: "done",
        display: { Markdown: { text: "# Hello inspector", title: "Doc" } } },
    ];
    const arts = artifactsFrom(items);
    render(
      <div style={{ display: "flex" }}>
        <ActivityRail items={items} sessionLabel="s" collapsed={false} onToggleCollapse={() => {}} />
        <Inspector artifacts={arts} activeKey={arts[0].key} onSelect={() => {}} onClose={() => {}} />
      </div>
    );
    expect(screen.getByRole("tab", { name: "Doc" })).toBeInTheDocument();
    expect(screen.getByText("Hello inspector")).toBeInTheDocument();
  });
});
