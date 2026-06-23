import type { Item } from "../state";
import { DiffView } from "./DiffView";
import { TerminalBlock } from "./TerminalBlock";

type ToolItem = Extract<Item, { kind: "tool" }>;

export function ToolCall({ item }: { item: ToolItem }) {
  const statusIcon = item.status === "running" ? "…" : "✓";
  const d = item.display;
  return (
    <div className="my-2">
      <div className="font-mono text-cyan-400">⚙ {item.name} <span className="text-zinc-500">{statusIcon}</span></div>
      {d && "Diff" in d && <DiffView path={d.Diff.path} before={d.Diff.before} after={d.Diff.after} />}
      {d && "Terminal" in d && (
        <TerminalBlock command={d.Terminal.command} stdout={d.Terminal.stdout} stderr={d.Terminal.stderr} exitCode={d.Terminal.exit_code} />
      )}
      {d && "Text" in d && <pre className="whitespace-pre-wrap p-2 font-mono text-sm text-zinc-300">{d.Text}</pre>}
      {!d && item.content && <pre className="whitespace-pre-wrap p-2 font-mono text-sm text-zinc-400">{item.content}</pre>}
    </div>
  );
}
