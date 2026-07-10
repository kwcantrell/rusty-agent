import { invoke } from "@tauri-apps/api/core";
import type { ContextSnapshot, SkillDto } from "./types";

export const getContext = () => invoke<ContextSnapshot>("context_get");
export const getSkill = (name: string) => invoke<SkillDto>("skill_get", { name });
export const saveSkill = (name: string, body: string) =>
  invoke<void>("skill_save", { name, body });
