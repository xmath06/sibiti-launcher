import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Tauri expects a fixed dev server port (matches tauri.conf.json -> devUrl).
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: 'localhost'
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    target: 'es2021'
  }
});
