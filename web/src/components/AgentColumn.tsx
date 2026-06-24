import type { AnimatedItem, PendingApproval } from "../state";
import type { Decision, RuntimeSettings } from "../wire";
import { AgentHeader } from "./AgentHeader";
import { MessageList } from "./MessageList";
import { ApprovalPrompt } from "./ApprovalPrompt";
import { ContextDashboard } from "./ContextDashboard";
import { Composer } from "./Composer";

export function AgentColumn({ items, activeArtifactKey, onSelectArtifact, projectLabel, model,
  pendingApproval, onDecide, composerDisabled, onSend, usage, settings, toolCount, artifactCount }:
  { items: AnimatedItem[]; activeArtifactKey: string | null; onSelectArtifact: (key: string) => void;
    projectLabel: string; model?: string; pendingApproval: PendingApproval | null;
    onDecide: (d: Decision) => void; composerDisabled: boolean; onSend: (text: string) => void;
    usage: { promptTokens: number; contextLimit: number; turn: number; maxTurns: number } | null;
    settings: RuntimeSettings | null; toolCount: number; artifactCount: number }) {
  return (
    <div className="flex h-full min-h-0 flex-col" style={{ background: "var(--surface-base)" }}>
      <AgentHeader projectLabel={projectLabel} model={model} />
      <div className="min-h-0 flex-1 overflow-y-auto py-2">
        <MessageList items={items} activeArtifactKey={activeArtifactKey} onSelectArtifact={onSelectArtifact} />
      </div>
      {pendingApproval && <ApprovalPrompt approval={pendingApproval} onDecide={onDecide} />}
      <ContextDashboard usage={usage} settings={settings} toolCount={toolCount} artifactCount={artifactCount} />
      <Composer disabled={composerDisabled} onSend={onSend} />
    </div>
  );
}
