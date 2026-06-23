import { defineConfig } from "vitest/config";
import { cloudflareTest } from "@cloudflare/vitest-pool-workers";

export default defineConfig({
  plugins: [
    cloudflareTest({
      wrangler: { configPath: "./wrangler.jsonc" },
      miniflare: {
        compatibilityDate: "2026-06-01",
        // Tests read env.BOOTSTRAP_SECRET and send it as the enroll auth header,
        // so the suite is self-consistent for any fixed value.
        bindings: { BOOTSTRAP_SECRET: "test-secret" },
      },
    }),
  ],
});
