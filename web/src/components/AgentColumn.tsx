import type { AnimatedItem, PendingApproval } from "../state";
import type { Decision } from "../wire";
import { AgentHeader } from "./AgentHeader";
import { MessageList } from "./MessageList";
import { ApprovalPrompt } from "./ApprovalPrompt";
import { Composer } from "./Composer";

export function AgentColumn({ items, activeArtifactKey, onSelectArtifact, projectLabel, model,
  pendingApproval, onDecide, composerDisabled, onSend }:
  { items: AnimatedItem[]; activeArtifactKey: string | null; onSelectArtifact: (key: string) => void;
    projectLabel: string; model?: string; pendingApproval: PendingApproval | null;
    onDecide: (d: Decision) => void; composerDisabled: boolean; onSend: (text: string) => void }) {
  return (
    <div className="flex h-full min-h-0 flex-col" style={{ background: "var(--surface-base)" }}>
      <AgentHeader projectLabel={projectLabel} model={model} />
      <div className="min-h-0 flex-1 overflow-y-auto py-2">
        <MessageList items={items} activeArtifactKey={activeArtifactKey} onSelectArtifact={onSelectArtifact} />
      </div>
      {pendingApproval && <ApprovalPrompt approval={pendingApproval} onDecide={onDecide} />}
      <Composer disabled={composerDisabled} onSend={onSend} />
    </div>
  );
}
