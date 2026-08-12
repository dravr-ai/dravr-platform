// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// <reference types="vitest" />
import { defineConfig, loadEnv } from 'vite'
import react from '@vitejs/plugin-react'
import { VitePWA } from 'vite-plugin-pwa'

// https://vite.dev/config/
export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), '')
  // Use 127.0.0.1 (IPv4) to avoid IPv6 conflicts with other processes on the same port
  const backendUrl = env.VITE_BACKEND_URL || 'http://127.0.0.1:8081'

  // Disable proxy during E2E tests since Playwright mocks all API routes
  const isE2EMode = process.env.E2E_TEST === 'true'

  return {
    plugins: [
      react(),
      // Add-to-Home-Screen + offline app shell. The service worker precaches
      // ONLY hashed build assets (the shell) and NEVER caches /api, /oauth,
      // /admin, /a2a, /ws, /__ — those are tenant-scoped or backend-proxied, and
      // a shared SW cache would leak one tenant's data into the next session
      // (multi-tenant isolation rule). No runtimeCaching of API responses at all.
      VitePWA({
        registerType: 'autoUpdate',
        // injectRegister 'auto' (default) wires the registration script into
        // index.html at build time, so no main.tsx import is needed.
        includeAssets: ['dravr-favicon.svg', 'apple-touch-icon.png'],
        manifest: {
          name: 'Dravr',
          short_name: 'Dravr',
          description: 'Fitness intelligence for athletes and coaches.',
          start_url: '/',
          scope: '/',
          display: 'standalone',
          orientation: 'portrait',
          background_color: '#00241a',
          theme_color: '#00241a',
          categories: ['health', 'fitness', 'sports'],
          icons: [
            { src: '/pwa-192.png', sizes: '192x192', type: 'image/png', purpose: 'any' },
            { src: '/pwa-512.png', sizes: '512x512', type: 'image/png', purpose: 'any' },
            { src: '/pwa-maskable-512.png', sizes: '512x512', type: 'image/png', purpose: 'maskable' },
          ],
        },
        workbox: {
          // Precache the app shell only. Hash-revisioned static assets — no HTML
          // API payloads ever enter the cache.
          globPatterns: ['**/*.{js,css,html,svg,png,ico,woff,woff2}'],
          // SPA deep links (#hash routes live under /) fall back to the cached
          // shell so an installed app opens offline. Backend/proxied paths are
          // denylisted so the SW never shadows a real network call to them.
          navigateFallback: '/index.html',
          // `/r/` (short-link redirects) and `/providers/` (the hosted Sciotte
          // reconnect/login + connect picker) are BACKEND routes proxied by nginx,
          // not SPA hash routes. Without them here the SW served the app shell for
          // a tapped reconnect link → user landed on the home dashboard instead of
          // the Garmin login form. Keep this list in sync with the nginx proxy
          // alternation in docker/images/frontend/nginx.conf.
          navigateFallbackDenylist: [
            /^\/api/,
            /^\/oauth/,
            /^\/admin/,
            /^\/a2a/,
            /^\/ws/,
            /^\/__/,
            /^\/r\//,
            /^\/providers\//,
            // MCP JSON-RPC endpoint + the OAuth discovery documents an MCP
            // client fetches before connecting. Both are backend routes.
            /^\/mcp/,
            /^\/\.well-known\//,
          ],
          // No runtimeCaching: tenant-scoped /api responses must always hit the
          // network. Only the immutable shell is cached, via precache above.
          cleanupOutdatedCaches: true,
        },
        // Keep the SW OUT of dev/E2E so it can't intercept the Vite proxy or
        // fight HMR. It is generated only for production builds (verify via
        // `bun run build && bun run preview`).
        devOptions: { enabled: false },
      }),
    ],
    server: isE2EMode
      ? {}
      : {
          proxy: {
            // Proxy backend OAuth endpoints but NOT /oauth-callback (frontend route)
            '/oauth': {
              target: backendUrl,
              changeOrigin: true,
              bypass: (req) => {
                // Don't proxy /oauth-callback - it's a frontend route
                if (req.url?.startsWith('/oauth-callback')) {
                  return req.url;
                }
                return undefined;
              },
            },
            '/api': {
              target: backendUrl,
              changeOrigin: true,
              ws: true,
              timeout: 1200000,
            },
            '/admin': {
              target: backendUrl,
              changeOrigin: true,
            },
            '/a2a': {
              target: backendUrl,
              changeOrigin: true,
            },
            '/ws': {
              target: backendUrl,
              ws: true,
              changeOrigin: true,
            },
            // MCP JSON-RPC endpoint and the OAuth discovery documents MCP
            // clients fetch. Present so `bun dev` matches the deployed nginx
            // routing — the absence of /mcp from BOTH layers is why nobody
            // noticed POST /mcp was unreachable in production.
            '/mcp': {
              target: backendUrl,
              changeOrigin: true,
            },
            '/.well-known': {
              target: backendUrl,
              changeOrigin: true,
            },
            // Firebase auth handler: proxy to firebaseapp.com so authDomain = localhost works
            '/__': {
              target: `https://${env.VITE_FIREBASE_PROJECT_ID || 'dravr-dev-8d4a3'}.firebaseapp.com`,
              changeOrigin: true,
              secure: true,
            },
          },
        },
    test: {
      globals: true,
      environment: 'jsdom',
      setupFiles: './src/test/setup.ts',
      include: ['src/**/*.{test,spec}.{ts,tsx}'],
      exclude: ['node_modules', 'e2e', 'dist'],
      // Vitest defaults to 5s, which assumes cheap unit tests. These are
      // component tests: every file boots its own jsdom and React Query, and
      // the assertions sit behind waitFor on async query resolution. At ~76
      // files the workers contend enough that borderline tests time out
      // non-deterministically while passing in isolation — a false red that
      // moves between files run to run. A timeout only bounds hangs; it is not
      // an assertion, so raising it loses no coverage and a genuine hang still
      // fails, 15s later.
      testTimeout: 15000,
      coverage: {
        provider: 'v8',
        reporter: ['text', 'json', 'html', 'lcov'],
        exclude: [
          'node_modules/',
          'src/test/',
          '**/*.test.{ts,tsx}',
          '**/*.config.{ts,js}',
          'dist/',
        ],
      },
      // CI-friendly configuration
      watch: false,
    },
  }
})
