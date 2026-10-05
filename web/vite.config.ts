import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { viteSingleFile } from "vite-plugin-singlefile";

// The server embeds the build as a single file (include_str!), so the bundle is
// inlined into dist/index.html.
export default defineConfig({
  plugins: [react(), viteSingleFile()],
  server: {
    proxy: {
      "/api": "http://localhost:8000",
      "/collab": { target: "ws://localhost:8000", ws: true },
      "/config": "http://localhost:8000",
      "/boats": "http://localhost:8000",
      "/boat": "http://localhost:8000",
      "/seasons": "http://localhost:8000",
      "/schedules": "http://localhost:8000",
      "/schedule": "http://localhost:8000",
      "/solve": "http://localhost:8000",
      "/status": "http://localhost:8000",
      "/swap": "http://localhost:8000",
      "/instructors": "http://localhost:8000",
      "/instructor": "http://localhost:8000",
      "/set-password": "http://localhost:8000",
      "/enrollment": "http://localhost:8000",
    },
  },
});
