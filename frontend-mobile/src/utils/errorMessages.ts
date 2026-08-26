// ABOUTME: Extracts user-friendly error messages from API errors
// ABOUTME: Parses quota exceeded (429) responses into actionable messages

import axios from 'axios';
import { TurnRequestError } from '@pierre/api-client';

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

function formatQuotaMessage(details: QuotaDetails): string {
  const { limit_type, current, limit } = details;

  switch (limit_type) {
    case 'max_active_conversations':
      return `Conversation limit reached (${current}/${limit}). Delete an existing conversation to start a new one.`;
    case 'daily_messages':
      return `Daily message limit reached (${current}/${limit}). Resets tomorrow.`;
    case 'daily_tokens':
      return `Daily token limit reached (${current}/${limit}). Resets tomorrow.`;
    case 'weekly_messages':
      return `Weekly message limit reached (${current}/${limit}). Resets next week.`;
    default:
      return `Usage quota reached (${current}/${limit}). Please try again later.`;
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

export function extractErrorMessage(err: unknown, fallback: string): string {
  const refused = refusal(err);
  if (!refused) {
    return err instanceof Error ? err.message : fallback;
  }

  const { status, data } = refused;

  if (status === 429 && data?.details?.limit_type) {
    return formatQuotaMessage(data.details);
  }

  if (status === 404) {
    return 'Coach not found. It may have been removed.';
  }

  return data?.message || (err instanceof Error ? err.message : '') || fallback;
}
