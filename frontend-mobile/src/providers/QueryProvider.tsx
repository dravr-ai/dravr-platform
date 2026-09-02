// ABOUTME: React Query provider with MMKV persistence and the shared focus/idle contract
// ABOUTME: Backgrounded or untouched, the app stops polling; the next touch resumes it

import React, { useMemo, useRef } from 'react';
import { AppState, type AppStateStatus, Platform, View } from 'react-native';
import { MutationCache, QueryClient, focusManager } from '@tanstack/react-query';
import { PersistQueryClientProvider } from '@tanstack/react-query-persist-client';
import Toast from 'react-native-toast-message';
import axios from 'axios';
import { IdleWatch, QUERY_FOCUS_POLICY } from '@pierre/shared-constants';
import {
  mmkvPersister,
  CACHE_TIMES,
  clearQueryCache,
} from '../utils/mmkvStorage';
import { extractErrorMessage } from '../utils/errorMessages';
import { idleAbort, registerIdleWatch, resetIdleAbort } from '../services/idleSignal';
import { useAuth } from '../contexts/AuthContext';
import { useTranslation } from '@pierre/i18n';

interface QueryProviderProps {
  children: React.ReactNode;
}

/**
 * Create a configured QueryClient with offline-first defaults
 *
 * Configuration optimized for:
 * - Stale-while-revalidate pattern (show cached data immediately, refetch in background)
 * - 7-day garbage collection for offline access
 * - Graceful degradation when offline (no error screens for stale data)
 */
function createQueryClient(
  // Module scope has no hook, so the provider — which is a component — hands
  // its `t` down. The mutation-cache error toast is the only copy in here.
  t: (key: string) => string,
): QueryClient {
  return new QueryClient({
    mutationCache: new MutationCache({
      onError: (error: Error) => {
        // 401 is handled by the axios interceptor (auto-logout) — skip
        if (axios.isAxiosError(error) && error.response?.status === 401) {
          return;
        }

        const message = extractErrorMessage(error, t('app.somethingWentWrongRetry'), t);

        Toast.show({
          type: 'error',
          text1: t('common.error'),
          text2: message,
          visibilityTime: 4000,
        });
      },
    }),
    defaultOptions: {
      queries: {
        // The focus/idle contract, shared verbatim with the web client. Spread
        // first so anything below is a deliberate, mobile-specific override
        // rather than an accidental one.
        ...QUERY_FOCUS_POLICY,

        // Retry configuration
        retry: 2,
        retryDelay: (attemptIndex) => Math.min(1000 * 2 ** attemptIndex, 30000),

        // Stale-while-revalidate: Show cached data immediately, refetch in background
        staleTime: CACHE_TIMES.DEFAULT_STALE_TIME,

        // Keep data in cache for 7 days for offline access
        gcTime: CACHE_TIMES.ACTIVITIES_GC_TIME,

        // Don't refetch on window focus in React Native
        refetchOnWindowFocus: false,

        // Don't refetch on mount if data is fresh
        refetchOnMount: true,

        // Network mode: always attempt fetch but use cached data when offline
        networkMode: 'offlineFirst',
      },
      mutations: {
        // Mutations should fail fast when offline
        networkMode: 'online',
        retry: 1,
      },
    },
  });
}

/**
 * Provider component that wraps the app with React Query + MMKV persistence
 *
 * Features:
 * - Automatic cache restoration on app start
 * - Stale-while-revalidate for instant UI updates
 * - 7-day cache for offline access
 * - Automatic cache clear on logout
 */
export function QueryProvider({ children }: QueryProviderProps) {
  const { t } = useTranslation();
  const { isAuthenticated, user } = useAuth();
  const queryClient = useMemo(() => createQueryClient(t), [t]);

  // Clear cache when user logs out
  React.useEffect(() => {
    if (!isAuthenticated && !user) {
      // User logged out, clear query cache to prevent data leakage
      queryClient.clear();
      clearQueryCache();
    }
  }, [isAuthenticated, user, queryClient]);

  // The idle watch owns `focusManager` for the whole app: backgrounding and
  // going untouched are both expressed as "not focused", so one switch governs
  // both instead of two mechanisms that have to agree.
  const watchRef = useRef<IdleWatch | null>(null);
  React.useEffect(() => {
    const watch = new IdleWatch({
      onIdle: () => {
        focusManager.setFocused(false);
        // A turn still streaming holds the connection — and the Cloud Run
        // instance behind it — open indefinitely. The athlete re-sends on
        // their way back in; `sendTurn` tells them so in as many words.
        idleAbort();
      },
      onActive: () => {
        resetIdleAbort();
        focusManager.setFocused(true);
      },
    });
    watchRef.current = watch;
    registerIdleWatch(watch);

    // Backgrounding is idleness we do not have to wait out: the athlete has
    // demonstrably looked away. Returning to the foreground counts as the
    // interaction that brought them back.
    const subscription = AppState.addEventListener('change', (status: AppStateStatus) => {
      if (Platform.OS === 'web') return;
      if (status === 'active') {
        watch.noteInteraction();
      } else {
        watch.suspend();
      }
    });

    return () => {
      registerIdleWatch(null);
      subscription.remove();
      watch.stop();
      watchRef.current = null;
      focusManager.setFocused(undefined);
    };
  }, []);

  // Every touch resets the idle deadline. Registered as a capture-phase
  // responder that always declines the gesture, so it observes the whole app's
  // interactions without taking any of them away from the real handlers.
  const noteTouch = React.useCallback(() => {
    watchRef.current?.noteInteraction();
    return false;
  }, []);

  return (
    <PersistQueryClientProvider
      client={queryClient}
      persistOptions={{
        persister: mmkvPersister,
        maxAge: CACHE_TIMES.MAX_CACHE_AGE,
        // Buster changes invalidate all cached data (useful for schema changes)
        buster: 'v1',
      }}
    >
      <View
        style={{ flex: 1 }}
        onStartShouldSetResponderCapture={noteTouch}
        onMoveShouldSetResponderCapture={noteTouch}
      >
        {children}
      </View>
    </PersistQueryClientProvider>
  );
}
