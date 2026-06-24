import "highlight.js/styles/github-dark.css";
import { useState } from "react";
import ReactMarkdown from "react-markdown";
import rehypeHighlight from "rehype-highlight";
import remarkGfm from "remark-gfm";

interface Props {
  text: string;
}

export function MarkdownText({ text }: Props) {
  const [copiedIndex, setCopiedIndex] = useState<number | null>(null);

  const handleCopy = (code: string, index: number) => {
    navigator.clipboard.writeText(code).catch(() => {});
    setCopiedIndex(index);
    setTimeout(() => setCopiedIndex(null), 1500);
  };

  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      rehypePlugins={[[rehypeHighlight, { detect: true }]]}
      components={{
        // Override pre to add copy button on hover
        pre({ node, children, ...props }) {
          // Extract text content from the code child for the copy button
          const codeText =
            (node?.children as Array<{ children?: Array<{ value?: string }> }> | undefined)
              ?.flatMap((child) => child.children ?? [])
              .map((leaf) => leaf.value ?? "")
              .join("") ?? "";
          const index = node?.position?.start.offset ?? -1;
          return (
            <div className="relative group rounded">
              <pre className="overflow-x-auto p-2 font-mono text-sm leading-tight" {...props}>
                {children}
              </pre>
              <button
                className="absolute right-2 top-2 rounded bg-[var(--surface-raised)] px-2 py-0.5 text-xs text-[var(--text)] opacity-0 transition-opacity group-hover:opacity-100 hover:opacity-80 border border-[var(--border)]"
                onClick={() => handleCopy(codeText, index)}
              >
                {copiedIndex === index ? "Copied!" : "Copy"}
              </button>
            </div>
          );
        },
        // Inline code styling; block code carries a language-* class from rehype-highlight
        code({ className, children, ...props }) {
          const isBlock = className?.includes("language-") || className?.includes("hljs");
          if (isBlock) {
            return (
              <code className={className} {...props}>
                {children}
              </code>
            );
          }
          return (
            <code className="rounded bg-[var(--surface-raised)] px-1 font-mono text-sm text-[var(--text)]" {...props}>
              {children}
            </code>
          );
        },
        // Headings
        h1({ children }) {
          return <h1 className="mb-1 mt-2 text-xl font-semibold text-[var(--text-strong)]">{children}</h1>;
        },
        h2({ children }) {
          return <h2 className="mb-1 mt-2 text-lg font-semibold text-[var(--text-strong)]">{children}</h2>;
        },
        h3({ children }) {
          return <h3 className="mb-1 mt-2 text-base font-semibold text-[var(--text-strong)]">{children}</h3>;
        },
        // Links
        a({ children, href, ...props }) {
          return (
            <a className="text-[var(--accent)] underline" href={href} target="_blank" rel="noopener noreferrer" {...props}>
              {children}
            </a>
          );
        },
        // Lists
        ul({ children }) {
          return <ul className="my-2 ml-4 list-disc space-y-1 text-[var(--text-strong)]">{children}</ul>;
        },
        ol({ children }) {
          return <ol className="my-2 ml-4 list-decimal space-y-1 text-[var(--text-strong)]">{children}</ol>;
        },
        li({ children }) {
          return <li className="text-[var(--text-strong)]">{children}</li>;
        },
        // Paragraphs
        p({ children }) {
          return <p className="my-1 text-[var(--text-strong)]">{children}</p>;
        },
      }}
    >
      {text}
    </ReactMarkdown>
  );
}
