// ABOUTME: Vitest configuration for shared-types package tests
// ABOUTME: Runs the wire-shape parsers against the bodies the server sends and the ones it must never be read as

import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    include: ['__tests__/**/*.test.ts'],
    environment: 'node',
  },
});
