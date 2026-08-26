// ABOUTME: Turns a turn's own `notice` reply block into the usage banner both clients show
// ABOUTME: One wording and one counter reading, instead of a countdown scraped out of the refusal prose

import type { ReplyNotice } from '@pierre/shared-types';

/** What a quota notice puts on the banner. */
export interface QuotaBanner {
  /** How close the cap is, in the banner's own vocabulary. */
  level: 'warning' | 'burst';
  /** The sentence to show. */
  message: string;
  /** RFC3339 instant the counter resets at. */
  resetsAt: string;
}

/** The reset instant in the reader's own timezone. */
function formatResetTime(isoString: string): string {
  try {
    return new Intl.DateTimeFormat(undefined, {
      hour: 'numeric',
      minute: '2-digit',
      timeZoneName: 'short',
    }).format(new Date(isoString));
  } catch {
    return 'midnight UTC';
  }
}

/**
 * Read a turn's `notice` block into the banner it warrants.
 *
 * The block carries the counter the pre-turn quota check actually measured —
 * its value, its cap, its level and its reset instant. Reading those replaced
 * scraping `/in (\d+) seconds/` out of the refusal sentence, which only ever
 * matched English and told the athlete nothing about which cap they had hit.
 */
export function quotaNoticeBanner(notice: ReplyNotice): QuotaBanner {
  const resetTime = formatResetTime(notice.resets_at);
  const pct = notice.limit > 0 ? Math.round((notice.current / notice.limit) * 100) : 0;

  if (notice.level === 'burst') {
    return {
      level: 'burst',
      message: `You're in the burst zone for your message quota (${notice.current}/${notice.limit}). Limits reset at ${resetTime}.`,
      resetsAt: notice.resets_at,
    };
  }

  return {
    level: 'warning',
    message: `You've used ${pct}% of your message quota (${notice.current}/${notice.limit}). Limits reset at ${resetTime}.`,
    resetsAt: notice.resets_at,
  };
}
