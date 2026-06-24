import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { WorkspacePane } from "./WorkspacePane";
import type { InspectorArtifact } from "../../state";

const htmlArt: InspectorArtifact = { key: "art-1", title: "page.html", display: { Html: { html: "<h1>Hi</h1>" } } as never };
const tableArt: InspectorArtifact = { key: "art-2", title: "data", display: { Table: { columns: ["a"], rows: [["1"]] } } as never };

describe("WorkspacePane", () => {
  beforeEach(() => localStorage.clear());

  it("shows the empty state with no artifacts", () => {
    render(<WorkspacePane artifacts={[]} activeKey={null} onSelect={() => {}} />);
    expect(screen.getByText("Workspace")).toBeInTheDocument();
  });

  it("renders a tab per artifact and selects on click", () => {
    const selected: string[] = [];
    render(<WorkspacePane artifacts={[htmlArt, tableArt]} activeKey="art-1" onSelect={(k) => selected.push(k)} />);
    expect(screen.getByRole("tab", { name: /page.html/ })).toHaveAttribute("aria-selected", "true");
    fireEvent.click(screen.getByRole("tab", { name: /data/ }));
    expect(selected).toContain("art-2");
  });

  it("toggles Preview/Code; Code shows source", () => {
    render(<WorkspacePane artifacts={[htmlArt]} activeKey="art-1" onSelect={() => {}} />);
    expect(screen.getByTitle("rendered html")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /^Code$/ }));
    // highlight.js splits the source across spans → assert on the container's text content
    expect(screen.getByTestId("code-view").textContent).toContain("<h1>Hi</h1>");
  });

  it("disables Code when the active artifact has no source", () => {
    render(<WorkspacePane artifacts={[tableArt]} activeKey="art-2" onSelect={() => {}} />);
    expect(screen.getByRole("button", { name: /^Code$/ })).toBeDisabled();
  });

  it("viewport selector constrains the preview width and is disabled in Code mode", () => {
    render(<WorkspacePane artifacts={[htmlArt]} activeKey="art-1" onSelect={() => {}} />);
    const frame = screen.getByTestId("preview-frame");
    expect(frame).toHaveStyle({ maxWidth: "100%" });
    fireEvent.click(screen.getByRole("button", { name: /Mobile/ }));
    expect(screen.getByTestId("preview-frame")).toHaveStyle({ maxWidth: "390px" });
    fireEvent.click(screen.getByRole("button", { name: /^Code$/ }));
    expect(screen.getByRole("button", { name: /Desktop/ })).toBeDisabled();
  });
});
