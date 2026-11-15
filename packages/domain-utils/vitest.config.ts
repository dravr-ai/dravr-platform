// ABOUTME: Vitest configuration for domain-utils package tests
// ABOUTME: Tests formatting, OAuth detection, route sketches, the form trend, the weekly volume and the analytics id

import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    include: ['__tests__/**/*.test.ts'],
    environment: 'node',
  },
});
