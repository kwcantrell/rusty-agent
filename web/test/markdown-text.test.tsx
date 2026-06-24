import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { MarkdownText } from "../src/components/MarkdownText";

describe("MarkdownText", () => {
  it("renders plain text as-is", () => {
    render(<MarkdownText text="hello world" />);
    expect(screen.getByText("hello world")).toBeInTheDocument();
  });

  it("renders headings", () => {
    // NOTE: must use a JS expression (curly braces) so \n is a real newline;
    // JSX attribute strings treat \n as a literal backslash-n, breaking markdown parsing.
    render(<MarkdownText text={"# Heading 1\n## Heading 2"} />);
    expect(screen.getByText("Heading 1")).toBeInTheDocument();
    expect(screen.getByText("Heading 2")).toBeInTheDocument();
  });

  it("renders bold and italic", () => {
    render(<MarkdownText text="**bold** and *italic*" />);
    expect(screen.getByText("bold")).toBeInTheDocument();
    expect(screen.getByText("italic")).toBeInTheDocument();
  });

  it("renders inline code", () => {
    render(<MarkdownText text="Use `code` here" />);
    expect(screen.getByText(/Use/)).toBeInTheDocument();
    expect(screen.getByText(/code/)).toBeInTheDocument();
  });

  it("renders code blocks", () => {
    // NOTE: must use a JS expression so \n is a real newline (fenced code block requires it).
    // rehype-highlight tokenizes code into <span> nodes, so we can't use getByText for the
    // exact string. Instead verify the pre's full textContent and that the code element
    // carries a highlight.js class. (Changed from the brief's getByText assertion for both reasons.)
    const { container } = render(<MarkdownText text={"```\nconst x = 1;\n```"} />);
    const pre = container.querySelector("pre");
    expect(pre?.textContent).toContain("const x = 1;");
    const code = pre?.querySelector("code");
    // rehype-highlight adds "hljs" class (and optionally language-* class)
    expect(code?.className).toMatch(/hljs|language-/);
  });

  it("renders links", () => {
    render(<MarkdownText text="[link](https://example.com)" />);
    const link = screen.getByText("link");
    expect(link).toBeInTheDocument();
    expect(link).toHaveAttribute("href", "https://example.com");
  });

  it("renders lists", () => {
    // NOTE: must use a JS expression so \n is a real newline (list items require it).
    render(<MarkdownText text={"- item 1\n- item 2"} />);
    expect(screen.getByText("item 1")).toBeInTheDocument();
    expect(screen.getByText("item 2")).toBeInTheDocument();
  });
});
