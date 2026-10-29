// ABOUTME: Vitest configuration for api-client package tests
// ABOUTME: Runs the domain methods over a stubbed fetch and adapter in Node, no client tree

import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    include: ['__tests__/**/*.test.ts'],
    environment: 'node',
  },
});
