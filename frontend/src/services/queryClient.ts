// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The app's one QueryClient — the focus/idle contract, and where a refused call goes
// ABOUTME: Built at module scope, so nothing a component re-renders can ever replace the cache

import { MutationCache, QueryCache, QueryClient } from '@tanstack/react-query';
import { QUERY_FOCUS_POLICY } from '@pierre/shared-constants';
import { classifyApiError } from '@pierre/ui-logic';

/**
 * Announces a failed call to the athlete.
 *
 * Installed by `useApiRefusalSurface`, which mounts inside the toast provider.
 */
export type RefusalReporter = (error: unknown) => void;

/**
 * The reporter, held by reference rather than passed in at construction.
 *
 * Saying it out loud is the point: a sentence needs the toast context and `t`,
 * and both live BELOW this client in the tree, because `ToastProvider` mounts
 * inside `QueryClientProvider`. Handing the reporter in would mean building the
 * client from a component, and a client built from a component is rebuilt when
 * its inputs change identity — i18next returns a new `t` every time the
 * language resolves, and `ToastProvider.addToast` is a new function every
 * render. Mobile learned what that costs: a new client is a new cache, and
 * every `setQueryData` written into the outgoing one is silently lost, which
 * stranded first-run onboarding on a permanent spinner (carnet#215). So the
 * client is built once, here, and the surface writes itself into this box.
 */
export const refusalReporter: { current: RefusalReporter | null } = { current: null };

function report(error: unknown): void {
  refusalReporter.current?.(error);
}

export const queryClient = new QueryClient({
  // Both caches, not just mutations: a refused admin tab is a GET, so the most
  // common authorization refusal in the app never passes through a mutation.
  queryCache: new QueryCache({ onError: report }),
  mutationCache: new MutationCache({ onError: report }),
  defaultOptions: {
    queries: {
      // The focus/idle contract, stated rather than inherited. `useIdleWatch`
      // drives it: an untouched tab stops polling instead of renewing a Cloud
      // Run instance forever.
      ...QUERY_FOCUS_POLICY,
      // An authorization refusal is the server's answer, not a hiccup — asking
      // twice more buys nothing and delays the explanation, which is how an
      // out-of-role admin tab came to sit there re-requesting its own 403 while
      // showing filter chrome it could never fill.
      retry: (failureCount: number, error: Error) => {
        const { kind } = classifyApiError(error);
        if (kind === 'unauthorized' || kind === 'forbidden') {
          return false;
        }
        return failureCount < 3;
      },
    },
  },
});
