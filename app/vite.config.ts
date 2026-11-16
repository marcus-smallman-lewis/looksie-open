import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import fs from "node:fs";
import path from "node:path";

// In a plain browser (no Tauri window) the UI reads saved answers from app/fixtures instead of calling the backend.
// This only exists in `npm run dev`; nothing from fixtures/ is ever bundled into the app.
function devFixtures(): Plugin {
  return {
    name: "looksie-dev-fixtures",
    apply: "serve",
    configureServer(server) {
      server.middlewares.use("/fixtures", (req, res) => {
        const file = path.join(__dirname, "fixtures", path.basename((req.url ?? "").split("?")[0]));
        if (!fs.existsSync(file)) {
          res.statusCode = 404;
          res.end("{}");
          return;
        }
        res.setHeader("Content-Type", "application/json");
        res.end(fs.readFileSync(file));
      });
    },
  };
}

export default defineConfig({
  plugins: [react(), devFixtures()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: { target: "es2022", outDir: "dist" },
});
