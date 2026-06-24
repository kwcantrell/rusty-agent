// Agent HTML is rendered in a fully sandboxed iframe (empty sandbox = no scripts,
// no same-origin) so it cannot touch the app, cookies, or storage.
export function HtmlArtifact({ html }: { html: string }) {
  return (
    <iframe
      title="rendered html"
      sandbox=""
      srcDoc={html}
      className="h-full w-full"
      style={{ border: "none", minHeight: "240px", background: "var(--surface-overlay)" }}
    />
  );
}
