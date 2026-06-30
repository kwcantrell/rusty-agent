import { invoke } from "@tauri-apps/api/core";
import type { ContextSnapshot, MemoryRow, ScoredRow, SkillDto } from "./types";

export const getContext = () => invoke<ContextSnapshot>("context_get");
export const listMemories = (limit = 50, offset = 0) =>
  invoke<MemoryRow[]>("memory_list", { limit, offset });
export const updateMemory = (id: string, text?: string, tags?: string[]) =>
  invoke<MemoryRow>("memory_update", { id, text: text ?? null, tags: tags ?? null });
export const deleteMemory = (id: string) => invoke<boolean>("memory_delete", { id });
export const recallPreview = (query: string) =>
  invoke<ScoredRow[]>("memory_recall_preview", { query });
export const getSkill = (name: string) => invoke<SkillDto>("skill_get", { name });
export const saveSkill = (name: string, body: string) =>
  invoke<void>("skill_save", { name, body });
