export interface ContextSegment { category: string; est_tokens: number; items: string[]; count: number }
export interface ContextSnapshot { turn: number; model_limit: number; est_total: number; segments: ContextSegment[] }
export interface SkillDto { name: string; description: string; body: string; files: string[] }
