import { defineConfig } from "vitest/config";
import { cloudflareTest } from "@cloudflare/vitest-pool-workers";

export default defineConfig({
  plugins: [
    cloudflareTest({
      wrangler: { configPath: "./wrangler.jsonc" },
      miniflare: {
        compatibilityDate: "2026-06-01",
        // Tests read env.BOOTSTRAP_SECRET; provide it deterministically here
        // (the worker tests pass it back as a header, so any value works).
        bindings: { BOOTSTRAP_SECRET: "test-secret" },
      },
    }),
  ],
});
