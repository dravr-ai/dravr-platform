// ABOUTME: Extracts user-friendly error messages from API errors
// ABOUTME: Parses quota exceeded (429) responses into actionable messages

import axios from 'axios';

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

export function extractErrorMessage(err: unknown, fallback: string): string {
  if (!axios.isAxiosError(err)) {
    return err instanceof Error ? err.message : fallback;
  }

  const status = err.response?.status;
  const data = err.response?.data as QuotaErrorData | undefined;

  if (status === 429 && data?.details?.limit_type) {
    return formatQuotaMessage(data.details);
  }

  if (status === 404) {
    return 'Coach not found. It may have been removed.';
  }

  return data?.message || err.message || fallback;
}
