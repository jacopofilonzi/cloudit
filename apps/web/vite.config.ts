/// <reference types="vitest/config" />
import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vite';

// The cloudit server (`make dev-server`) the dev server proxies API calls to.
const apiTarget = 'http://localhost:8080';

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
      // SPA: no SSR, every unknown path is served `index.html` (by the Rust server
      // in production, see `WEB_DIR`).
      adapter: adapter({ fallback: 'index.html' })
    })
  ],
  server: {
    proxy: {
      '/api': apiTarget,
      '/health': apiTarget,
      '/ready': apiTarget
    }
  },
  test: {
    // Unit tests of the plain TS modules in `src/lib` (no DOM).
    environment: 'node',
    include: ['src/**/*.test.ts'],
    expect: { requireAssertions: true }
  }
});
