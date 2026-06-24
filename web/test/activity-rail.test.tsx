import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { ActivityRail } from "../src/components/ActivityRail";
import type { Item } from "../src/state";

const items: Item[] = [
  { kind: "user", text: "hi" },
  { kind: "tool", name: "edit_file", args: {}, status: "done" },
  { kind: "tool", name: "execute_command", args: {}, status: "running" },
];

describe("ActivityRail", () => {
  it("lists tool activity with the session label", () => {
    render(<ActivityRail items={items} sessionLabel="auth-refactor"
      collapsed={false} onToggleCollapse={() => {}} />);
    expect(screen.getByText("auth-refactor")).toBeInTheDocument();
    expect(screen.getByText("edit_file")).toBeInTheDocument();
    expect(screen.getByText("execute_command")).toBeInTheDocument();
  });
  it("hides labels when collapsed", () => {
    render(<ActivityRail items={items} sessionLabel="auth-refactor"
      collapsed={true} onToggleCollapse={() => {}} />);
    expect(screen.queryByText("edit_file")).not.toBeInTheDocument();
  });
});
