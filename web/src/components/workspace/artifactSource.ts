import type { Display } from "../../wire";

/** The raw source + highlight language for the Code tab, or null when an
 *  artifact has no meaningful source (Diff/Terminal/Table/Image → Code disabled). */
export function artifactSource(d: Display): { source: string; lang: string } | null {
  if ("Html" in d) return { source: d.Html.html, lang: "html" };
  if ("Mermaid" in d) return { source: d.Mermaid.source, lang: "mermaid" };
  if ("Code" in d) return { source: d.Code.text, lang: d.Code.lang };
  if ("Text" in d) return { source: d.Text, lang: "text" };
  if ("Markdown" in d) return { source: d.Markdown.text, lang: "markdown" };
  return null;
}
