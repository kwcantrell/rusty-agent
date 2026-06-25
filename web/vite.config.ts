import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Pure-desktop: the webview connects directly to the Tauri local bridge
// (ws://127.0.0.1:<port>/agent), so no dev-server proxy is needed.
export default defineConfig({
  plugins: [react(), tailwindcss()],
});
