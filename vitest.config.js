import { defineConfig } from "vitest/config";

// Frontend test config. The UI is vanilla ES-module JS with no build step, so
// Vitest just needs a DOM: jsdom gives the unit tests (pure helpers) and the
// end-to-end test (drives `init()` against the real index.html markup with a
// mocked Tauri IPC bridge) a `window`/`document` to run against.
export default defineConfig({
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.js"],
    globals: true,
  },
});
