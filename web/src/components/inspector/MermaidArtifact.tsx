import { useEffect, useRef, useState } from "react";

// Lazy-load mermaid so it stays out of the initial bundle.
export function MermaidArtifact({ source }: { source: string }) {
  const ref = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const mermaid = (await import("mermaid")).default;
        mermaid.initialize({ startOnLoad: false, theme: "neutral" });
        const { svg } = await mermaid.render("m" + Math.abs(hash(source)), source);
        if (!cancelled && ref.current) ref.current.innerHTML = svg;
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : "diagram error");
      }
    })();
    return () => { cancelled = true; };
  }, [source]);

  if (error) return <pre className="p-3 text-sm" style={{ color: "var(--state-error)" }}>{error}</pre>;
  return <div data-mermaid ref={ref} className="p-3" />;
}

function hash(s: string): number {
  let h = 0;
  for (let i = 0; i < s.length; i++) { h = (h << 5) - h + s.charCodeAt(i); h |= 0; }
  return h;
}
