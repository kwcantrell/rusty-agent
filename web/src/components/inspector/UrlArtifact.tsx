import { isLocalUrl, isMixedContent } from "./urlGuard";

function Notice({ text }: { text: string }) {
  return (
    <div className="flex h-full items-center justify-center p-6 text-center text-sm"
      style={{ color: "var(--text-muted)", minHeight: "240px" }}>
      <p>{text}</p>
    </div>
  );
}

// Live preview of the user's own dev server. Unlike agent-authored HTML (fully
// sandboxed), a real app needs scripts and its own origin — acceptable only
// because non-localhost targets never reach the iframe.
export function UrlArtifact({ url }: { url: string }) {
  if (!isLocalUrl(url)) {
    return <Notice text={`Blocked: only localhost URLs render here (got ${url}).`} />;
  }
  if (isMixedContent(url)) {
    return <Notice text={"This page is served over HTTPS, so the browser blocks embedding an "
      + "http:// localhost app. Use the desktop app (or a locally served UI) for live preview."} />;
  }
  return (
    <iframe title="live preview" src={url} sandbox="allow-scripts allow-same-origin"
      className="h-full w-full"
      style={{ border: "none", minHeight: "240px", background: "var(--surface-overlay)" }} />
  );
}
