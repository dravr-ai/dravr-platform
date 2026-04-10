// ABOUTME: Astro configuration for the Dravr marketing website
// ABOUTME: SSR via Cloudflare Pages adapter for auth middleware and API routes

import { defineConfig } from 'astro/config';
import cloudflare from '@astrojs/cloudflare';
import sitemap from '@astrojs/sitemap';
import tailwindcss from '@tailwindcss/vite';

// `site` is used for canonical URLs, sitemap entries, and OG metadata.
// Preview deploys should override this via `SITE_URL` so crawlers don't see
// production URLs on a staging host.
const site = process.env.SITE_URL ?? 'https://dravr.ai';

export default defineConfig({
  site,
  output: 'server',
  trailingSlash: 'never',
  adapter: cloudflare({
    imageService: 'compile',
    platformProxy: { enabled: true },
  }),
  integrations: [
    sitemap({
      // The gated alpha docs area is not for public crawling.
      filter: (page) => !page.includes('/docs/'),
    }),
  ],
  vite: {
    plugins: [tailwindcss()],
  },
  build: {
    assets: 'assets',
  },
});
