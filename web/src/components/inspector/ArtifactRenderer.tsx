import type { Display } from "../../wire";
import { DiffView } from "../DiffView";
import { TerminalBlock } from "../TerminalBlock";
import { MarkdownText } from "../MarkdownText";
import { HtmlArtifact } from "./HtmlArtifact";
import { MermaidArtifact } from "./MermaidArtifact";
import { UrlArtifact } from "./UrlArtifact";
import { isLocalUrl } from "./urlGuard";

export function ArtifactRenderer({ display }: { display: Display }) {
  if ("Text" in display) {
    return <pre className="whitespace-pre-wrap p-3 font-mono text-sm" style={{ color: "var(--text)" }}>{display.Text}</pre>;
  }
  if ("Markdown" in display) {
    return <div className="p-3"><MarkdownText text={display.Markdown.text} /></div>;
  }
  if ("Code" in display) {
    const { filename, lang, text } = display.Code;
    return (
      <div className="m-3 rounded" style={{ border: "1px solid var(--border)" }}>
        <div className="px-2 py-1 font-mono text-xs"
          style={{ background: "var(--surface-raised)", color: "var(--text-muted)", borderBottom: "1px solid var(--border)" }}>
          {filename ?? lang}
        </div>
        <MarkdownText text={"```" + lang + "\n" + text + "\n```"} />
      </div>
    );
  }
  if ("Diff" in display) {
    return <div className="p-3"><DiffView path={display.Diff.path} before={display.Diff.before} after={display.Diff.after} /></div>;
  }
  if ("Terminal" in display) {
    const t = display.Terminal;
    return <div className="p-3"><TerminalBlock command={t.command} stdout={t.stdout} stderr={t.stderr} exitCode={t.exit_code} /></div>;
  }
  if ("Table" in display) {
    const { columns, rows } = display.Table;
    return (
      <div className="p-3 overflow-x-auto">
        <table className="w-full text-sm" style={{ color: "var(--text)" }}>
          <thead>
            <tr>{columns.map((c, i) => (
              <th key={i} className="px-2 py-1 text-left font-semibold"
                style={{ color: "var(--text-strong)", borderBottom: "1px solid var(--border)" }}>{c}</th>
            ))}</tr>
          </thead>
          <tbody>
            {rows.map((r, ri) => (
              <tr key={ri}>{r.map((cell, ci) => (
                <td key={ci} className="px-2 py-1" style={{ borderBottom: "1px solid var(--border)" }}>{cell}</td>
              ))}</tr>
            ))}
          </tbody>
        </table>
      </div>
    );
  }
  if ("Image" in display) {
    const { mime, data } = display.Image;
    // Agent-controlled http(s) srcs are an outbound-fetch channel (tracking
    // pixel / exfil beacon) on the browser path, which ships no CSP — allow
    // only data: URIs and the same localhost set UrlArtifact accepts.
    if (data.startsWith("http") && !isLocalUrl(data)) {
      return (
        <div className="p-3 text-sm" style={{ color: "var(--text-muted)" }}>
          Blocked remote image URL — only data: and localhost image sources render here.
        </div>
      );
    }
    const src = data.startsWith("http") || data.startsWith("data:") ? data : `data:${mime};base64,${data}`;
    return <div className="p-3"><img src={src} alt="rendered artifact" className="max-w-full rounded" /></div>;
  }
  if ("Html" in display) {
    return <HtmlArtifact html={display.Html.html} />;
  }
  if ("Mermaid" in display) {
    return <MermaidArtifact source={display.Mermaid.source} />;
  }
  if ("Url" in display) {
    return <UrlArtifact url={display.Url.url} />;
  }
  return null;
}
