import path from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig, type Plugin } from "vitest/config";
import react from "@vitejs/plugin-react";
import { createGhostApi } from "./server/api.ts";

// Die lokale API (server/api.ts) hängt am Dev- und am Preview-Server.
// Sie ruft ../ghostctl auf und ist nur für den Betrieb auf dem eigenen Rechner
// gedacht. Deshalb lauschen beide Server nur auf localhost.
const APP_DIR = path.dirname(fileURLToPath(import.meta.url));
const PROJECT_DIR = path.resolve(APP_DIR, "..");

function ghostApi(): Plugin {
  const handler = createGhostApi(PROJECT_DIR);
  return {
    name: "ghost-api",
    configureServer(server) {
      server.middlewares.use(handler);
      handler.startAboTimer();
    },
    configurePreviewServer(server) {
      server.middlewares.use(handler);
      handler.startAboTimer();
    },
  };
}

export default defineConfig({
  base: "./",
  plugins: [react(), ghostApi()],
  // zweite Seite: Browser-Wallet-Probe (nur lokal, docs/wallet-probe.md)
  build: {
    rolldownOptions: {
      input: { main: path.join(APP_DIR, "index.html"), probe: path.join(APP_DIR, "wallet-probe.html") },
    },
  },
  server: { host: "localhost", cors: false },
  preview: { host: "localhost", cors: false },
  test: {
    environment: "node",
    include: ["src/**/*.test.ts", "server/**/*.test.ts"],
  },
});
