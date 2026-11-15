// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins CreateApiKey's "High Risk" permission tag to the badge tone table
// ABOUTME: A dangerous permission is tagged in the error tone, whose ink clears AA

import React from 'react';
import { render, screen } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { describe, expect, it, vi } from 'vitest';
import CreateApiKey from '../CreateApiKey';

describe('CreateApiKey', () => {
  it('tags a dangerous permission in the error tone', () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
    render(
      <QueryClientProvider client={client}>
        <CreateApiKey onBack={vi.fn()} onTokenCreated={vi.fn()} />
      </QueryClientProvider>,
    );

    const tags = screen.getAllByText('High Risk');
    expect(tags.length).toBeGreaterThan(0);
    for (const tag of tags) {
      expect(tag).toHaveClass('badge-error');
    }
  });
});
