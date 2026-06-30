export interface ContextSegment { category: string; est_tokens: number; items: string[]; count: number }
export interface ContextSnapshot { turn: number; model_limit: number; est_total: number; segments: ContextSegment[] }
export interface MemoryRow { id: string; text: string; tags: string[]; scope_kind: string; updated_at: number }
export interface ScoredRow { id: string; text: string; score: number; scope_kind: string }
export interface SkillDto { name: string; description: string; body: string; files: string[] }
