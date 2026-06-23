import type { Decision } from "../wire";
import type { PendingApproval } from "../state";

export function ApprovalPrompt({ approval, onDecide }: { approval: PendingApproval; onDecide: (d: Decision) => void }) {
  return (
    <div className="mx-4 my-2 rounded border border-amber-700 bg-amber-950 p-3 text-sm">
      <div className="mb-2 text-amber-200">Allow: {approval.summary}</div>
      {approval.command && <pre className="mb-2 overflow-x-auto font-mono text-amber-300">{approval.command}</pre>}
      <div className="flex gap-2">
        <button onClick={() => onDecide("approve")} className="rounded bg-green-700 px-3 py-1 text-white hover:bg-green-600">Approve</button>
        <button onClick={() => onDecide("approve_always")} className="rounded bg-green-900 px-3 py-1 text-green-200 hover:bg-green-800">Approve always</button>
        <button onClick={() => onDecide("deny")} className="rounded bg-red-800 px-3 py-1 text-white hover:bg-red-700">Deny</button>
      </div>
    </div>
  );
}
