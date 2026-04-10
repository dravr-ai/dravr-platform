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
      // The gated alpha docs area is not for public crawling. Matches both
      // the bare index URLs (/docs, /fr/docs) and nested guide URLs, since
      // `trailingSlash: 'never'` means the canonical index URLs have no
      // trailing slash and wouldn't be caught by a `/docs/` substring check.
      filter: (page) => !/\/(fr\/)?docs(\/|$)/.test(page),
    }),
  ],
  vite: {
    plugins: [tailwindcss()],
  },
  build: {
    assets: 'assets',
  },
});
