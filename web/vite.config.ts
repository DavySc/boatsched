import { defineConfig } from "vite";
import { viteSingleFile } from "vite-plugin-singlefile";

// The server embeds the build as a single file (include_str!), so the bundle is
// inlined into dist/index.html. React is aliased to Preact's compat layer: same
// API, a fraction of the bytes on a phone's first load.
export default defineConfig({
  plugins: [viteSingleFile()],
  resolve: {
    alias: {
      "react-dom/client": "preact/compat/client",
      "react-dom": "preact/compat",
      "react/jsx-runtime": "preact/jsx-runtime",
      "react/jsx-dev-runtime": "preact/jsx-dev-runtime",
      react: "preact/compat",
    },
  },
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
