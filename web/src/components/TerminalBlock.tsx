export function TerminalBlock({ command, stdout, stderr, exitCode }: { command: string; stdout: string; stderr: string; exitCode: number }) {
  return (
    <div className="rounded border border-zinc-700 bg-black text-sm">
      <div className="flex items-center justify-between border-b border-zinc-700 px-2 py-1">
        <span className="font-mono text-zinc-300">$ {command}</span>
        <span className={exitCode === 0 ? "text-green-400" : "text-red-400"}>exit {exitCode}</span>
      </div>
      <pre className="overflow-x-auto p-2 font-mono leading-tight text-zinc-200">{stdout}{stderr}</pre>
    </div>
  );
}
