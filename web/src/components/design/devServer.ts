import { invoke } from "@tauri-apps/api/core";

export interface DevScriptCandidate {
  dir: string;
  script: string;
  package_manager: string;
  label: string;
}

export interface DevServerStatus {
  url: string;
  candidate: DevScriptCandidate;
}

export function detectDevScripts(): Promise<DevScriptCandidate[]> {
  return invoke<DevScriptCandidate[]>("dev_scripts_detect");
}

export function startDevServer(candidate: DevScriptCandidate): Promise<DevServerStatus> {
  return invoke<DevServerStatus>("dev_server_start", { candidate });
}

export function stopDevServer(): Promise<void> {
  return invoke("dev_server_stop");
}

export function devServerStatus(): Promise<DevServerStatus | null> {
  return invoke<DevServerStatus | null>("dev_server_status");
}
