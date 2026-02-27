// ABOUTME: Unit tests for QueryProvider mutation error handling
// ABOUTME: Verifies global MutationCache onError shows toast for server errors

import React from 'react';
import { render, waitFor } from '@testing-library/react-native';
import { Text } from 'react-native';
import { useMutation } from '@tanstack/react-query';
import { AxiosError, AxiosHeaders } from 'axios';
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

  it('should show error toast on 500 server error', async () => {
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
          text2: 'Internal server error',
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

  it('should show fallback message for non-axios errors', async () => {
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
          text2: 'Network failure',
        })
      );
    });
  });
});
