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

/**
 * A translator that returns the key and appends its params, so an assertion
 * can name the key the module chose without pinning any locale's wording.
 */
const translate = (key: string, params?: Record<string, string | number>): string =>
  params === undefined ? key : `${key} ${JSON.stringify(params)}`;

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

      expect(extractErrorMessage(err, 'fallback', translate)).toBe(
        'errors.conversationLimitReached {"current":2,"limit":2}'
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

      expect(extractErrorMessage(err, 'fallback', translate)).toBe(
        'errors.dailyMessageLimitReached {"current":50,"limit":50}'
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

      expect(extractErrorMessage(err, 'fallback', translate)).toBe(
        'errors.dailyTokenLimitReached {"current":100000,"limit":100000}'
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

      expect(extractErrorMessage(err, 'fallback', translate)).toBe(
        'errors.weeklyMessageLimitReached {"current":200,"limit":200}'
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

      expect(extractErrorMessage(err, 'fallback', translate)).toBe(
        'errors.usageQuotaReached {"current":5,"limit":10}'
      );
    });

    it('falls back to server message when 429 has no details', () => {
      const err = createAxiosError(429, {
        message: 'Rate limited',
      });

      expect(extractErrorMessage(err, 'fallback', translate)).toBe('Rate limited');
    });
  });

  describe('404 errors', () => {
    it('returns coach not found message', () => {
      const err = createAxiosError(404, { message: 'Not found' });

      expect(extractErrorMessage(err, 'fallback', translate)).toBe(
        'errors.coachNotFoundRemoved'
      );
    });
  });

  describe('other Axios errors', () => {
    it('returns server error message when available', () => {
      const err = createAxiosError(500, {
        message: 'Internal server error',
      });

      expect(extractErrorMessage(err, 'fallback', translate)).toBe('Internal server error');
    });

    it('returns axios error message when no response data message', () => {
      const err = createAxiosError(500, {});

      expect(extractErrorMessage(err, 'fallback', translate)).toBe('Request failed');
    });
  });

  describe('non-Axios errors', () => {
    it('returns Error.message for standard errors', () => {
      const err = new Error('Something went wrong');

      expect(extractErrorMessage(err, 'fallback', translate)).toBe('Something went wrong');
    });

    it('returns fallback for non-Error values', () => {
      expect(extractErrorMessage('string error', 'fallback', translate)).toBe('fallback');
      expect(extractErrorMessage(42, 'fallback', translate)).toBe('fallback');
      expect(extractErrorMessage(null, 'fallback', translate)).toBe('fallback');
    });
  });
});
