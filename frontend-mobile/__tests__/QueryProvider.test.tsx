// ABOUTME: Unit tests for QueryProvider mutation error handling and the refusal retry policy
// ABOUTME: Verifies the global MutationCache toast wording and that an authorization refusal is never retried

import React from 'react';
import { render, waitFor } from '@testing-library/react-native';
import { Text } from 'react-native';
import { useMutation, useQuery } from '@tanstack/react-query';
import { AxiosError, AxiosHeaders } from 'axios';
import {
  createAxiosClient,
  createMobileAdapter,
  type AsyncStorageLike,
} from '@pierre/api-client';
import Toast from 'react-native-toast-message';

// Must mock AuthContext before importing QueryProvider
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({ isAuthenticated: true, user: { id: '1' } }),
}));

// Mock mmkvStorage to avoid MMKV native module dependency
jest.mock('../src/utils/mmkvStorage', () => ({
  mmkvPersister: {
    persistClient: jest.fn(),
    restoreClient: jest.fn().mockResolvedValue(undefined),
    removeClient: jest.fn(),
  },
  CACHE_TIMES: {
    DEFAULT_STALE_TIME: 60000,
    ACTIVITIES_GC_TIME: 604800000,
    MAX_CACHE_AGE: 604800000,
  },
  clearQueryCache: jest.fn(),
}));

import { QueryProvider } from '../src/providers/QueryProvider';

function MutationTrigger({ error }: { error: Error }) {
  const mutation = useMutation({
    mutationFn: async () => {
      throw error;
    },
    retry: false,
  });

  React.useEffect(() => {
    mutation.mutate();
  }, []);

  return <Text>{mutation.status}</Text>;
}

/**
 * A query that always fails, counting its attempts.
 *
 * `retryDelay` is overridden to zero so the test measures the retry PREDICATE
 * rather than sitting through the provider's exponential backoff; the `retry`
 * option itself is inherited, which is what is under test.
 */
function QueryTrigger({ error, attempt }: { error: Error; attempt: () => void }) {
  const query = useQuery({
    queryKey: ['refusal', error.message],
    queryFn: async () => {
      attempt();
      throw error;
    },
    retryDelay: 0,
    gcTime: 0,
  });

  return <Text>{query.status}</Text>;
}

/** Storage the adapter can write to without a native module behind it. */
function memoryAsyncStorage(): AsyncStorageLike {
  const store = new Map<string, string>();
  return {
    getItem: async (key) => store.get(key) ?? null,
    setItem: async (key, value) => {
      store.set(key, value);
    },
    removeItem: async (key) => {
      store.delete(key);
    },
    multiRemove: async (keys) => {
      keys.forEach((key) => store.delete(key));
    },
  };
}

function createAxiosError(status: number, data?: Record<string, unknown>): AxiosError {
  const headers = new AxiosHeaders();
  const error = new AxiosError(
    `Request failed with status code ${status}`,
    String(status),
    undefined,
    undefined,
    {
      data: data || {},
      status,
      statusText: String(status),
      headers,
      config: { headers },
    }
  );
  return error;
}

describe('QueryProvider MutationCache', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('should show the generic server sentence on a 500, never the server internals', async () => {
    const error = createAxiosError(500, { message: 'Internal server error' });

    render(
      <QueryProvider>
        <MutationTrigger error={error} />
      </QueryProvider>
    );

    await waitFor(() => {
      expect(Toast.show).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'error',
          text1: 'Error',
          text2: 'Server error. Try again a bit later.',
        })
      );
    });
  });

  it('should show quota message on 429 with details', async () => {
    const error = createAxiosError(429, {
      code: 'QUOTA_EXCEEDED',
      message: 'Rate limit exceeded',
      details: {
        limit_type: 'daily_messages',
        current: 50,
        limit: 50,
      },
    });

    render(
      <QueryProvider>
        <MutationTrigger error={error} />
      </QueryProvider>
    );

    await waitFor(() => {
      expect(Toast.show).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'error',
          text2: 'Daily message limit reached (50/50). Resets tomorrow.',
        })
      );
    });
  });

  it('should show what a role 403 refused, not the axios status line', async () => {
    const error = createAxiosError(403, {
      code: 'PermissionDenied',
      message: "Only the conversation's owner can delete it",
    });

    render(
      <QueryProvider>
        <MutationTrigger error={error} />
      </QueryProvider>
    );

    await waitFor(() => {
      expect(Toast.show).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'error',
          text2: "Only the conversation's owner can delete it",
        })
      );
    });
  });

  it('should not show toast on 401 (handled by axios interceptor)', async () => {
    const error = createAxiosError(401);

    const { getByText } = render(
      <QueryProvider>
        <MutationTrigger error={error} />
      </QueryProvider>
    );

    // Wait for mutation to settle (status changes to 'error')
    await waitFor(() => {
      expect(getByText('error')).toBeTruthy();
    });

    expect(Toast.show).not.toHaveBeenCalled();
  });

  it('should not show toast on a 403 the transport is already recovering', async () => {
    // The 403 nothing but the challenge can identify. Produced by driving the
    // REAL shared interceptor rather than hand-built, because what earns the
    // silence is the mark the interceptor leaves — a fabricated error would
    // prove only that the assertion was written to pass.
    const adapter = createMobileAdapter({ asyncStorage: memoryAsyncStorage() });
    const client = createAxiosClient(adapter);
    client.defaults.adapter = (config) =>
      Promise.reject(
        new AxiosError(
          'Request failed with status code 403',
          AxiosError.ERR_BAD_REQUEST,
          config,
          null,
          {
            status: 403,
            statusText: '',
            data: { code: 'InsufficientScope', message: 'Scope fitness:write required' },
            headers: AxiosHeaders.from({
              'www-authenticate':
                'Bearer resource_metadata="https://x/.well-known/oauth-protected-resource", ' +
                'error="insufficient_scope", scope="fitness:write"',
            }),
            config,
          }
        )
      );
    const error = await client
      .post('/api/activities', {})
      .then(() => null)
      .catch((err: Error) => err);

    const { getByText } = render(
      <QueryProvider>
        <MutationTrigger error={error as Error} />
      </QueryProvider>
    );

    await waitFor(() => {
      expect(getByText('error')).toBeTruthy();
    });

    // The athlete is being sent to sign in; a permission message about a scope
    // they no longer hold would be both wrong and unactionable.
    expect(Toast.show).not.toHaveBeenCalled();
  });

  it('should show the transport sentence for an error that never reached a server', async () => {
    const error = new Error('Network failure');

    render(
      <QueryProvider>
        <MutationTrigger error={error} />
      </QueryProvider>
    );

    await waitFor(() => {
      expect(Toast.show).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'error',
          text2: 'Network error. Check your connection.',
        })
      );
    });
  });
});

describe('QueryProvider query retry policy', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('asks once and gives up on a 403', async () => {
    const attempt = jest.fn();

    const { getByText } = render(
      <QueryProvider>
        <QueryTrigger error={createAxiosError(403, { code: 'PermissionDenied' })} attempt={attempt} />
      </QueryProvider>
    );

    await waitFor(() => {
      expect(getByText('error')).toBeTruthy();
    });

    expect(attempt).toHaveBeenCalledTimes(1);
  });

  it('asks once and gives up on a 401', async () => {
    const attempt = jest.fn();

    const { getByText } = render(
      <QueryProvider>
        <QueryTrigger error={createAxiosError(401)} attempt={attempt} />
      </QueryProvider>
    );

    await waitFor(() => {
      expect(getByText('error')).toBeTruthy();
    });

    expect(attempt).toHaveBeenCalledTimes(1);
  });

  it('still retries twice on a 500', async () => {
    const attempt = jest.fn();

    const { getByText } = render(
      <QueryProvider>
        <QueryTrigger error={createAxiosError(500)} attempt={attempt} />
      </QueryProvider>
    );

    await waitFor(() => {
      expect(getByText('error')).toBeTruthy();
    });

    expect(attempt).toHaveBeenCalledTimes(3);
  });
});
