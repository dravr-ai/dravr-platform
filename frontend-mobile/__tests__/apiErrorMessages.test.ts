// ABOUTME: The sentences the phone shows for a refused request, read off the real English catalogue
// ABOUTME: Succeeds the deleted mobile extractErrorMessage; covers both refusal carriers and every quota limit

import { AxiosError, AxiosHeaders } from 'axios';
import { TurnRequestError } from '@pierre/api-client';
import { describeApiError } from '@pierre/ui-logic';
import { i18n } from '@pierre/i18n';

/**
 * The app's own translator, not a stand-in.
 *
 * A key that ships on web but never reaches the phone's bundle resolves to the
 * key itself, which typecheck and a fake-translator test both pass — so the
 * sentences are asserted verbatim against the catalogue jest.setup.js pins to
 * English.
 */
const t = (key: string, params?: Record<string, string | number>): string =>
  i18n.t(key, params ?? {});

/** The wording a screen asks for when nothing more specific is available. */
const FALLBACK = 'app.somethingWentWrongRetry';

function axiosRefusal(status: number, data: unknown): AxiosError {
  const headers = new AxiosHeaders();
  return new AxiosError(
    `Request failed with status code ${status}`,
    String(status),
    undefined,
    undefined,
    { status, statusText: String(status), headers, data, config: { headers } },
  );
}

describe('what a refused mobile request says', () => {
  describe('a role refusal (403)', () => {
    it('shows the server sentence naming what was refused', () => {
      const err = axiosRefusal(403, {
        code: 'PermissionDenied',
        message: "Only the conversation's owner can delete it",
      });

      expect(describeApiError(err, { t, fallbackKey: FALLBACK })).toBe(
        "Only the conversation's owner can delete it",
      );
    });

    // With nothing from the server, the screen's own wording is the most
    // specific thing anyone has — which is why the fallback key is per call
    // site rather than one sentence for the whole kind.
    it("shows the screen's own wording when the server named nothing", () => {
      const err = axiosRefusal(403, {});

      expect(describeApiError(err, { t, fallbackKey: FALLBACK })).toBe(
        'Something went wrong. Please try again.',
      );
      expect(describeApiError(err, { t, fallbackKey: 'app.failedRemoveParticipant' })).toBe(
        'Failed to remove participant',
      );
    });
  });

  describe('a usage limit (429)', () => {
    it('names the conversation cap with its counts', () => {
      const err = axiosRefusal(429, {
        code: 'QuotaExceeded',
        message: 'max_active_conversations quota exceeded: 2/2',
        details: { limit_type: 'max_active_conversations', current: 2, limit: 2, resets_at: '' },
      });

      expect(describeApiError(err, { t, fallbackKey: FALLBACK })).toBe(
        'Conversation limit reached (2/2). Delete an existing conversation to start a new one.',
      );
    });

    it('names the daily message cap with its counts', () => {
      const err = axiosRefusal(429, {
        code: 'QuotaExceeded',
        details: { limit_type: 'daily_messages', current: 50, limit: 50 },
      });

      expect(describeApiError(err, { t, fallbackKey: FALLBACK })).toBe(
        'Daily message limit reached (50/50). Resets tomorrow.',
      );
    });

    it('names the daily token cap with its counts', () => {
      const err = axiosRefusal(429, {
        code: 'QuotaExceeded',
        details: { limit_type: 'daily_tokens', current: 100000, limit: 100000 },
      });

      expect(describeApiError(err, { t, fallbackKey: FALLBACK })).toBe(
        'Daily token limit reached (100000/100000). Resets tomorrow.',
      );
    });

    it('names the weekly message cap with its counts', () => {
      const err = axiosRefusal(429, {
        code: 'QuotaExceeded',
        details: { limit_type: 'weekly_messages', current: 200, limit: 200 },
      });

      expect(describeApiError(err, { t, fallbackKey: FALLBACK })).toBe(
        'Weekly message limit reached (200/200). Resets next week.',
      );
    });

    it('reads a limit type it has no sentence for as a generic quota', () => {
      const err = axiosRefusal(429, {
        code: 'QuotaExceeded',
        details: { limit_type: 'some_future_limit', current: 5, limit: 10 },
      });

      expect(describeApiError(err, { t, fallbackKey: FALLBACK })).toBe(
        'Usage quota reached (5/10). Please try again later.',
      );
    });

    it('shows the screen fallback when the server counted nothing', () => {
      const err = axiosRefusal(429, { message: 'Rate limited' });

      expect(describeApiError(err, { t, fallbackKey: FALLBACK })).toBe(
        'Something went wrong. Please try again.',
      );
    });

    // A turn is the one request that cannot ride axios — its body is read frame
    // by frame — so a quota refusal on the send path arrives as a
    // TurnRequestError. It wears the same `response` shape, and the same
    // sentence has to come out of it.
    it('names the cap when the refusal arrived on the turn transport', () => {
      const err = new TurnRequestError('Turn refused', 429, {
        code: 'QuotaExceeded',
        details: { limit_type: 'daily_messages', current: 50, limit: 50 },
      });

      expect(describeApiError(err, { t, fallbackKey: FALLBACK })).toBe(
        'Daily message limit reached (50/50). Resets tomorrow.',
      );
    });
  });

  describe('a missing row (404)', () => {
    // The deleted mobile helper answered every 404 with the coach sentence, so
    // a missing group or conversation claimed a coach had been removed. The
    // wording is the screen's to choose now.
    it('shows the server sentence', () => {
      const err = axiosRefusal(404, { message: 'Conversation not found' });

      expect(describeApiError(err, { t, fallbackKey: FALLBACK })).toBe('Conversation not found');
    });

    it("shows the screen's own wording when the server named nothing", () => {
      const err = axiosRefusal(404, {});

      expect(describeApiError(err, { t, fallbackKey: FALLBACK })).toBe(
        'Something went wrong. Please try again.',
      );
    });

    it('shows the agent sentence only for a screen that asks for it', () => {
      const err = axiosRefusal(404, {});

      expect(describeApiError(err, { t, fallbackKey: 'app.coachNotFound' })).toBe('Agent not found');
    });
  });

  describe('a server fault (5xx)', () => {
    it('never repeats the server internals back to the athlete', () => {
      const withDetail = axiosRefusal(500, { message: 'Internal server error' });
      const withoutDetail = axiosRefusal(500, {});

      expect(describeApiError(withDetail, { t, fallbackKey: FALLBACK })).toBe(
        'Server error. Try again a bit later.',
      );
      expect(describeApiError(withoutDetail, { t, fallbackKey: FALLBACK })).toBe(
        'Server error. Try again a bit later.',
      );
    });
  });

  describe('a request that never landed', () => {
    it('reads a thrown Error as a dead transport, not as its own message', () => {
      expect(describeApiError(new Error('Something went wrong'), { t, fallbackKey: FALLBACK })).toBe(
        'Network error. Check your connection.',
      );
    });

    it('reads it as offline when the device says so', () => {
      expect(
        describeApiError(new Error('Something went wrong'), {
          online: false,
          t,
          fallbackKey: FALLBACK,
        }),
      ).toBe("You're offline. Check your connection and try again.");
    });

    it('still answers with a sentence when the thrown value is not an error', () => {
      expect(describeApiError(42, { t, fallbackKey: FALLBACK })).toBe(
        'Network error. Check your connection.',
      );
      expect(describeApiError(null, { t, fallbackKey: FALLBACK })).toBe(
        'Network error. Check your connection.',
      );
    });
  });
});
