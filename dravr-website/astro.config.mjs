// ABOUTME: Astro configuration for the Dravr marketing website
// ABOUTME: SSR via Cloudflare Pages adapter for auth middleware and API routes

import { defineConfig } from 'astro/config';
import cloudflare from '@astrojs/cloudflare';
import tailwindcss from '@tailwindcss/vite';

export default defineConfig({
  site: 'https://dravr.ai',
  output: 'server',
  adapter: cloudflare({ imageService: 'compile' }),
  vite: {
    plugins: [tailwindcss()],
  },
  build: {
    assets: 'assets',
  },
});
