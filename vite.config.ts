import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

// Tauri sets TAURI_ENV_* while running `tauri dev` / `tauri build`.
const platform = process.env.TAURI_ENV_PLATFORM;
const debug = !!process.env.TAURI_ENV_DEBUG;

export default defineConfig({
  base: './',
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  envPrefix: ['VITE_', 'TAURI_ENV_*'],
  server: {
    port: 5173,
    strictPort: true,
    watch: {
      ignored: ['**/src-tauri/**'],
    },
  },
  build: {
    outDir: 'dist',
    // WebView2 on Windows is Chromium based; WebKitGTK and WKWebView need Safari 13 syntax.
    target: platform === 'windows' ? 'chrome105' : 'safari13',
    minify: debug ? false : 'esbuild',
    sourcemap: debug,
    // The bundle is loaded from disk by the webview, so one larger chunk is fine.
    chunkSizeWarningLimit: 1024,
  },
});
