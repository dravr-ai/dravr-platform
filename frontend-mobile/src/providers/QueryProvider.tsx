// ABOUTME: React Query provider with MMKV persistence and the shared focus/idle contract
// ABOUTME: Backgrounded stops polling at once; untouched also drops the open turn; coming back resumes both

import React, { useMemo, useRef } from 'react';
import { AppState, type AppStateStatus, Platform, View } from 'react-native';
import { MutationCache, QueryClient, focusManager } from '@tanstack/react-query';
import { PersistQueryClientProvider } from '@tanstack/react-query-persist-client';
import Toast from 'react-native-toast-message';
import {
  IdleWatch,
  QUERY_FOCUS_POLICY,
  idleAbort,
  registerIdleWatch,
  resetIdleAbort,
} from '@pierre/shared-constants';
import {
  classifyApiError,
  describeApiError,
  type ApiErrorTranslate,
} from '@pierre/ui-logic';
import { droveReauthentication } from '@pierre/api-client';
import {
  mmkvPersister,
  CACHE_TIMES,
  clearQueryCache,
} from '../utils/mmkvStorage';
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
  // a REF to its `t` down, read at toast time. A ref rather than the function
  // itself because this client is built once and must never be rebuilt: i18next
  // hands back a new `t` identity whenever the language resolves, and a client
  // memoised on that identity is a new client, a new cache, and every
  // `setQueryData` written into the old one silently lost (carnet#215).
  t: React.MutableRefObject<ApiErrorTranslate>,
): QueryClient {
  return new QueryClient({
    mutationCache: new MutationCache({
      onError: (error: Error) => {
        // Two ways a refusal earns silence here, and each catches what the
        // other cannot. A refusal the transport already recovered is on its way
        // to the login form, and only the transport can say so — a 403 carrying
        // RFC 6750's `insufficient_scope` recovers exactly as a 401 does, and
        // nothing in the error's status distinguishes it from a role refusal.
        // And any unauthenticated refusal means the session is gone, whether or
        // not it reached this app through that interceptor.
        if (droveReauthentication(error) || classifyApiError(error).kind === 'unauthorized') {
          return;
        }

        Toast.show({
          type: 'error',
          text1: t.current('common.error'),
          // The one classifier both clients read. It names the failure from the
          // status and the transport state, so a 403 says what the server
          // refused instead of "Request failed with status code 403", and a
          // 5xx's internals never reach the toast.
          text2: describeApiError(error, {
            t: t.current,
            fallbackKey: 'app.somethingWentWrongRetry',
          }),
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

        // Retry configuration. An authorization refusal is exempt: the server
        // has answered, and asking the same question twice more only spends two
        // round trips plus three seconds of backoff before the screen is
        // allowed to say what happened.
        retry: (failureCount: number, error: Error) => {
          const { kind } = classifyApiError(error);
          if (kind === 'unauthorized' || kind === 'forbidden') {
            return false;
          }
          return failureCount < 2;
        },
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
  // The translator is read through a ref so the client is built ONCE. Memoising
  // on `t` rebuilt it every time i18next handed back a new function identity,
  // which threw the whole cache away mid-session: the onboarding profile-type
  // choice wrote `true` into the outgoing client and the redirect never saw it,
  // leaving a permanent spinner on the first screen of first-run (carnet#215).
  const translate = React.useRef<ApiErrorTranslate>(t);
  translate.current = t;
  const queryClient = useMemo(() => createQueryClient(translate), []);

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
        // instance behind it — open indefinitely. The send path re-reads the
        // conversation when the athlete returns, so a reply the server went on
        // to write is shown rather than lost.
        idleAbort();
      },
      // Backgrounded: nothing on screen is being read, so the polls stop now.
      // The stream stays open until the idle deadline.
      onSuspend: () => {
        focusManager.setFocused(false);
      },
      onActive: () => {
        resetIdleAbort();
        focusManager.setFocused(true);
      },
    });
    watchRef.current = watch;
    registerIdleWatch(watch);

    // Backgrounding stops the polls at once, but an open turn waits out the
    // same deadline an untouched screen does: an athlete who left for the
    // Strava app to authorize is back in a minute, and the reply the server is
    // still writing must be there when they are. Returning to the foreground
    // is the interaction that ends the absence.
    const subscription = AppState.addEventListener('change', (status: AppStateStatus) => {
      if (Platform.OS === 'web') return;
      if (status === 'active') {
        watch.resume();
      } else {
        watch.suspend();
      }
    });
    // An app launched into the background — a notification action, a
    // background fetch — reports no change until it is opened, so the watch
    // is told where it starts. Only an explicit `background`: a state the
    // platform has not resolved yet is not evidence that nobody is here.
    if (Platform.OS !== 'web' && AppState.currentState === 'background') watch.suspend();

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
        // Changing the buster drops every persisted query. It moves whenever a
        // cached response changes shape, so a new bundle never renders a
        // response its components no longer read: v2 is the group health flag
        // `evidence` and the weekly report's `fresh_members`.
        buster: 'v2',
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
