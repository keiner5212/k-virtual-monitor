import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";
import { fileURLToPath } from "node:url";

const host = process.env.TAURI_DEV_HOST;
const isDevDebug = process.env.TAURI_ENV_DEBUG === "true" || process.env.TAURI_ENV_DEBUG === "1";

export default defineConfig({
    plugins: [react(), tailwindcss()],

    resolve: {
        alias: {
            "@": path.resolve(path.dirname(fileURLToPath(import.meta.url)), "./src"),
        },
    },

    clearScreen: false,
    server: {
        port: 1420,
        strictPort: true,
        host: host || false,
        hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
        watch: {
            ignored: ["**/src-tauri/**"],
        },
    },

    envPrefix: ["VITE_", "TAURI_ENV_*"],
    build: {
        target: process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome105" : "safari13",
        // Terser, not the default oxc: the oxc minifier breaks the GSAP
        // timeline the animated title uses, and the title stays invisible.
        minify: isDevDebug ? false : "terser",
        sourcemap: isDevDebug,
    },
});
