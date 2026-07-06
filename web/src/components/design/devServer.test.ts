import { describe, it, expect, vi, beforeEach } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import { detectDevScripts, startDevServer, stopDevServer } from "./devServer";

describe("devServer wrappers", () => {
  beforeEach(() => invoke.mockReset());

  it("detect calls the dev_scripts_detect command", async () => {
    invoke.mockResolvedValueOnce([{ dir: "/w/web", script: "dev", package_manager: "pnpm", label: "web — dev" }]);
    const got = await detectDevScripts();
    expect(invoke).toHaveBeenCalledWith("dev_scripts_detect");
    expect(got[0].script).toBe("dev");
  });

  it("start passes the candidate as an argument", async () => {
    const cand = { dir: "/w/web", script: "dev", package_manager: "pnpm", label: "web — dev" };
    invoke.mockResolvedValueOnce({ url: "http://localhost:5173/", candidate: cand });
    const got = await startDevServer(cand);
    expect(invoke).toHaveBeenCalledWith("dev_server_start", { candidate: cand });
    expect(got.url).toBe("http://localhost:5173/");
  });

  it("stop calls dev_server_stop", async () => {
    invoke.mockResolvedValueOnce(undefined);
    await stopDevServer();
    expect(invoke).toHaveBeenCalledWith("dev_server_stop");
  });
});
