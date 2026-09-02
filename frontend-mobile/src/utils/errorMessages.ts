// ABOUTME: Extracts user-friendly error messages from API errors
// ABOUTME: Parses quota exceeded (429) responses into actionable messages

import axios from 'axios';
import { TurnRequestError } from '@pierre/api-client';
import type { Translate } from '@pierre/chat-utils';

interface QuotaDetails {
  limit_type: string;
  current: number;
  limit: number;
  resets_at?: string;
}

interface QuotaErrorData {
  code?: string;
  message?: string;
  details?: QuotaDetails;
}

function formatQuotaMessage(details: QuotaDetails, t: Translate): string {
  const { limit_type, current, limit } = details;
  const counts = { current, limit };

  switch (limit_type) {
    case 'max_active_conversations':
      return t('errors.conversationLimitReached', counts);
    case 'daily_messages':
      return t('errors.dailyMessageLimitReached', counts);
    case 'daily_tokens':
      return t('errors.dailyTokenLimitReached', counts);
    case 'weekly_messages':
      return t('errors.weeklyMessageLimitReached', counts);
    default:
      return t('errors.usageQuotaReached', counts);
  }
}

/**
 * The two carriers a refused request can arrive in: an `AxiosError` from the
 * ordinary domain methods, and a `TurnRequestError` from `sendTurn`, which
 * reads its response body frame by frame and so cannot ride axios. Both hold
 * the same two facts, and both are formatted by the one path below.
 */
function refusal(err: unknown): { status?: number; data?: QuotaErrorData } | null {
  if (err instanceof TurnRequestError) {
    return { status: err.status, data: (err.body ?? undefined) as QuotaErrorData | undefined };
  }
  if (axios.isAxiosError(err)) {
    return { status: err.response?.status, data: err.response?.data as QuotaErrorData | undefined };
  }
  return null;
}

/**
 * `t` is the caller's translator: this module runs outside any component, and
 * the sentences it used to build were English on every screen that shows a
 * refusal (carnet#207).
 */
export function extractErrorMessage(err: unknown, fallback: string, t: Translate): string {
  const refused = refusal(err);
  if (!refused) {
    return err instanceof Error ? err.message : fallback;
  }

  const { status, data } = refused;

  if (status === 429 && data?.details?.limit_type) {
    return formatQuotaMessage(data.details, t);
  }

  if (status === 404) {
    return t('errors.coachNotFoundRemoved');
  }

  return data?.message || (err instanceof Error ? err.message : '') || fallback;
}
