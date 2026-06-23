import { diffLines } from "diff";

export function DiffView({ path, before, after }: { path: string; before: string; after: string }) {
  const parts = diffLines(before, after);
  return (
    <div className="rounded border border-zinc-700 bg-zinc-900 text-sm">
      <div className="border-b border-zinc-700 px-2 py-1 font-mono text-amber-400">{path}</div>
      <pre className="overflow-x-auto p-2 font-mono leading-tight">
        {parts.flatMap((part, pi) => {
          const sign = part.added ? "+" : part.removed ? "-" : " ";
          const cls = part.added ? "text-green-400" : part.removed ? "text-red-400" : "text-zinc-400";
          return part.value.replace(/\n$/, "").split("\n").map((line, li) => (
            <div key={`${pi}-${li}`} className={cls}>{sign} {line}</div>
          ));
        })}
      </pre>
    </div>
  );
}
