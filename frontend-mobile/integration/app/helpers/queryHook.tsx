// ABOUTME: renderHook with a QueryClientProvider — the app's own provider, so chat hooks behave as they ship
// ABOUTME: useMessages and useConversations invalidate the conversation-list query, which needs a client in scope

import React from 'react';
import { renderHook as rtlRenderHook } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

/**
 * Render a hook the way the app mounts it: under a query client.
 *
 * A fresh client per call keeps specs independent, and retries are off so a
 * stubbed failure surfaces on the first attempt instead of after backoff.
 */
export function renderHook<T>(hook: () => T) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 }, mutations: { retry: false } },
  });
  return rtlRenderHook(hook, {
    wrapper: ({ children }: { children: React.ReactNode }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    ),
  });
}
