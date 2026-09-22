// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Vitest configuration for shared-constants package tests
// ABOUTME: Runs the platform-free contracts both clients obey, such as the idle watch, in node

import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    include: ['__tests__/**/*.test.ts'],
    environment: 'node',
  },
});
