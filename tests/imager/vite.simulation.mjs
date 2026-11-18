// Test entry point only. The normal desktop build never imports this adapter.
import { defineConfig, mergeConfig } from 'vite';
import base from '../../vite.config.ts';
import { fileURLToPath } from 'node:url';
export default defineConfig(async (env) =>
  mergeConfig(await base(env), {
    build: { outDir: 'dist-review-simulation' },
    resolve: {
      alias: [
        {
          find: /^@tauri-apps\/api\/core$/,
          replacement: fileURLToPath(new URL('./simulated-core.mjs', import.meta.url)),
        },
      ],
    },
  })
);
