// Only used by vitest: compiles `.svelte` / `.svelte.ts` (runes) for the unit tests.
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  plugins: [svelte({ configFile: false, compilerOptions: { runes: true } })],
  test: {
    environment: 'node',
    include: ['src/**/*.{test,spec}.{js,ts}'],
    expect: { requireAssertions: true }
  }
});
