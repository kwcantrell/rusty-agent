import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// In dev, the browser hits Vite (:5173); proxy the Worker's API/WS routes to wrangler dev (:8787)
// so the app is same-origin from the browser's perspective (no CORS) and WebSockets work.
const target = "http://127.0.0.1:8787";
export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    proxy: {
      "/enroll": { target, changeOrigin: true },
      "/pair": { target, changeOrigin: true },
      "/agent": { target, ws: true, changeOrigin: true },
      "/browser": { target, ws: true, changeOrigin: true },
    },
  },
});
