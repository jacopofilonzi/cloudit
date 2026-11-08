/// <reference types="vitest/config" />
import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vite';

// `tauri dev` loads this dev server (`devUrl` in src-tauri/tauri.conf.json); the UI
// talks to the Rust backend only through Tauri commands, so there is no API proxy.
export default defineConfig({
  plugins: [
    tailwindcss(),
    sveltekit({
      compilerOptions: {
        // Runes everywhere (our code and the workspace UI package); untouched for
        // third-party libraries in node_modules.
        runes: ({ filename }) =>
          filename.split(/[/\\]/).includes('node_modules') ? undefined : true
      },
      // SPA inside the Tauri window: no SSR, unknown paths get `index.html`.
      adapter: adapter({ fallback: 'index.html' })
    })
  ],
  server: { port: 5174, strictPort: true },
  clearScreen: false,
  test: {
    environment: 'node',
    include: ['src/**/*.test.ts'],
    expect: { requireAssertions: true }
  }
});
