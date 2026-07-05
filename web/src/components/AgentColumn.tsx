import type { AnimatedItem, PendingApproval } from "../state";
import type { Decision, RuntimeSettings, SessionStats } from "../wire";
import { SessionBanner } from "./SessionBanner";
import { MessageList } from "./MessageList";
import { BusyLine } from "./BusyLine";
import { ApprovalPrompt } from "./ApprovalPrompt";
import { ContextDashboard } from "./ContextDashboard";
import { Composer } from "./Composer";

export function AgentColumn({ items, activeArtifactKey, onSelectArtifact, projectLabel, model,
  pendingApproval, onDecide, composerDisabled, onSend, usage, settings, toolCount, artifactCount, stats,
  busy, turn, history }:
  { items: AnimatedItem[]; activeArtifactKey: string | null; onSelectArtifact: (key: string) => void;
    projectLabel: string; model?: string; pendingApproval: PendingApproval | null;
    onDecide: (d: Decision) => void; composerDisabled: boolean; onSend: (text: string) => void;
    usage: { promptTokens: number; contextLimit: number; turn: number; maxTurns: number } | null;
    settings: RuntimeSettings | null; toolCount: number; artifactCount: number;
    stats: SessionStats | null; busy: boolean; turn: number; history: () => string[] }) {
  return (
    <div className="cli flex h-full min-h-0 flex-col">
      <div className="min-h-0 flex-1 overflow-y-auto py-2">
        <SessionBanner projectLabel={projectLabel} model={model} />
        <MessageList items={items} activeArtifactKey={activeArtifactKey} onSelectArtifact={onSelectArtifact} />
        {busy && <BusyLine turn={turn} />}
      </div>
      {pendingApproval && <ApprovalPrompt approval={pendingApproval} onDecide={onDecide} />}
      <ContextDashboard usage={usage} settings={settings} toolCount={toolCount} artifactCount={artifactCount} stats={stats} />
      <Composer disabled={composerDisabled} onSend={onSend} history={history} />
    </div>
  );
}
