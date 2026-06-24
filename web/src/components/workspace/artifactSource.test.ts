import { describe, it, expect } from "vitest";
import { artifactSource } from "./artifactSource";
import type { Display } from "../../wire";

describe("artifactSource", () => {
  it("extracts html source", () => {
    const d = { Html: { html: "<h1>hi</h1>" } } as unknown as Display;
    expect(artifactSource(d)).toEqual({ source: "<h1>hi</h1>", lang: "html" });
  });
  it("extracts mermaid source", () => {
    const d = { Mermaid: { source: "graph TD; A-->B" } } as unknown as Display;
    expect(artifactSource(d)).toEqual({ source: "graph TD; A-->B", lang: "mermaid" });
  });
  it("extracts code source with its language", () => {
    const d = { Code: { filename: "a.rs", lang: "rust", text: "fn main() {}" } } as unknown as Display;
    expect(artifactSource(d)).toEqual({ source: "fn main() {}", lang: "rust" });
  });
  it("extracts plain text and markdown", () => {
    expect(artifactSource({ Text: "hello" } as unknown as Display)).toEqual({ source: "hello", lang: "text" });
    expect(artifactSource({ Markdown: { text: "# h" } } as unknown as Display)).toEqual({ source: "# h", lang: "markdown" });
  });
  it("returns null for non-source displays", () => {
    expect(artifactSource({ Diff: { path: "a", before: "x", after: "y" } } as unknown as Display)).toBeNull();
    expect(artifactSource({ Terminal: { command: "ls", stdout: "", stderr: "", exit_code: 0 } } as unknown as Display)).toBeNull();
    expect(artifactSource({ Image: { mime: "image/png", data: "..." } } as unknown as Display)).toBeNull();
    expect(artifactSource({ Table: { columns: [], rows: [] } } as unknown as Display)).toBeNull();
  });
});
