// ABOUTME: Unit tests for extractErrorMessage utility
// ABOUTME: Verifies quota-aware error parsing for 429 responses and other HTTP errors

import { AxiosError, AxiosHeaders } from 'axios';
import { extractErrorMessage } from '../src/utils/errorMessages';

function createAxiosError(status: number, data: unknown): AxiosError {
  const error = new AxiosError('Request failed');
  error.response = {
    status,
    statusText: status === 429 ? 'Too Many Requests' : 'Error',
    headers: {},
    config: { headers: new AxiosHeaders() },
    data,
  };
  return error;
}

describe('extractErrorMessage', () => {
  describe('quota exceeded (429)', () => {
    it('returns conversation limit message for max_active_conversations', () => {
      const err = createAxiosError(429, {
        code: 'QuotaExceeded',
        message: 'max_active_conversations quota exceeded: 2/2',
        details: {
          limit_type: 'max_active_conversations',
          current: 2,
          limit: 2,
          resets_at: '',
        },
      });

      expect(extractErrorMessage(err, 'fallback')).toBe(
        'Conversation limit reached (2/2). Delete an existing conversation to start a new one.'
      );
    });

    it('returns daily message limit message', () => {
      const err = createAxiosError(429, {
        code: 'QuotaExceeded',
        details: {
          limit_type: 'daily_messages',
          current: 50,
          limit: 50,
        },
      });

      expect(extractErrorMessage(err, 'fallback')).toBe(
        'Daily message limit reached (50/50). Resets tomorrow.'
      );
    });

    it('returns daily token limit message', () => {
      const err = createAxiosError(429, {
        code: 'QuotaExceeded',
        details: {
          limit_type: 'daily_tokens',
          current: 100000,
          limit: 100000,
        },
      });

      expect(extractErrorMessage(err, 'fallback')).toBe(
        'Daily token limit reached (100000/100000). Resets tomorrow.'
      );
    });

    it('returns weekly message limit message', () => {
      const err = createAxiosError(429, {
        code: 'QuotaExceeded',
        details: {
          limit_type: 'weekly_messages',
          current: 200,
          limit: 200,
        },
      });

      expect(extractErrorMessage(err, 'fallback')).toBe(
        'Weekly message limit reached (200/200). Resets next week.'
      );
    });

    it('returns generic quota message for unknown limit type', () => {
      const err = createAxiosError(429, {
        code: 'QuotaExceeded',
        details: {
          limit_type: 'some_future_limit',
          current: 5,
          limit: 10,
        },
      });

      expect(extractErrorMessage(err, 'fallback')).toBe(
        'Usage quota reached (5/10). Please try again later.'
      );
    });

    it('falls back to server message when 429 has no details', () => {
      const err = createAxiosError(429, {
        message: 'Rate limited',
      });

      expect(extractErrorMessage(err, 'fallback')).toBe('Rate limited');
    });
  });

  describe('404 errors', () => {
    it('returns coach not found message', () => {
      const err = createAxiosError(404, { message: 'Not found' });

      expect(extractErrorMessage(err, 'fallback')).toBe(
        'Coach not found. It may have been removed.'
      );
    });
  });

  describe('other Axios errors', () => {
    it('returns server error message when available', () => {
      const err = createAxiosError(500, {
        message: 'Internal server error',
      });

      expect(extractErrorMessage(err, 'fallback')).toBe('Internal server error');
    });

    it('returns axios error message when no response data message', () => {
      const err = createAxiosError(500, {});

      expect(extractErrorMessage(err, 'fallback')).toBe('Request failed');
    });
  });

  describe('non-Axios errors', () => {
    it('returns Error.message for standard errors', () => {
      const err = new Error('Something went wrong');

      expect(extractErrorMessage(err, 'fallback')).toBe('Something went wrong');
    });

    it('returns fallback for non-Error values', () => {
      expect(extractErrorMessage('string error', 'fallback')).toBe('fallback');
      expect(extractErrorMessage(42, 'fallback')).toBe('fallback');
      expect(extractErrorMessage(null, 'fallback')).toBe('fallback');
    });
  });
});
