// ABOUTME: React Query provider with MMKV persistence for offline caching
// ABOUTME: Implements stale-while-revalidate pattern with 7-day activity cache

import React, { useMemo } from 'react';
import { MutationCache, QueryClient } from '@tanstack/react-query';
import { PersistQueryClientProvider } from '@tanstack/react-query-persist-client';
import Toast from 'react-native-toast-message';
import axios from 'axios';
import {
  mmkvPersister,
  CACHE_TIMES,
  clearQueryCache,
} from '../utils/mmkvStorage';
import { extractErrorMessage } from '../utils/errorMessages';
import { useAuth } from '../contexts/AuthContext';

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
function createQueryClient(): QueryClient {
  return new QueryClient({
    mutationCache: new MutationCache({
      onError: (error: Error) => {
        // 401 is handled by the axios interceptor (auto-logout) — skip
        if (axios.isAxiosError(error) && error.response?.status === 401) {
          return;
        }

        const message = extractErrorMessage(error, 'Something went wrong. Please try again.');

        Toast.show({
          type: 'error',
          text1: 'Error',
          text2: message,
          visibilityTime: 4000,
        });
      },
    }),
    defaultOptions: {
      queries: {
        // Retry configuration
        retry: 2,
        retryDelay: (attemptIndex) => Math.min(1000 * 2 ** attemptIndex, 30000),

        // Stale-while-revalidate: Show cached data immediately, refetch in background
        staleTime: CACHE_TIMES.DEFAULT_STALE_TIME,

        // Keep data in cache for 7 days for offline access
        gcTime: CACHE_TIMES.ACTIVITIES_GC_TIME,

        // Don't refetch on window focus in React Native
        refetchOnWindowFocus: false,

        // Refetch when network reconnects (useful for offline-first)
        refetchOnReconnect: true,

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
  const { isAuthenticated, user } = useAuth();
  const queryClient = useMemo(() => createQueryClient(), []);

  // Clear cache when user logs out
  React.useEffect(() => {
    if (!isAuthenticated && !user) {
      // User logged out, clear query cache to prevent data leakage
      queryClient.clear();
      clearQueryCache();
    }
  }, [isAuthenticated, user, queryClient]);

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
      {children}
    </PersistQueryClientProvider>
  );
}
