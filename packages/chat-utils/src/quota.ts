// ABOUTME: Turns a turn's own `notice` reply block into the usage banner both clients show, and prints its figures
// ABOUTME: One wording and one counter reading, instead of a countdown scraped out of the refusal prose

import type { ReplyNotice } from '@pierre/shared-types';

import type { TranslatableText } from './text';

/** What a quota notice puts on the banner. */
export interface QuotaBanner {
  /** How close the cap is, in the banner's own vocabulary. */
  level: 'warning' | 'burst';
  /** The sentence to show, as a catalogue key the client translates. */
  text: TranslatableText;
  /** RFC3339 instant the counter resets at. */
  resetsAt: string;
}

/**
 * The reset instant in the reader's own timezone: `12:00 AM UTC`, `00:00 UTC`.
 *
 * Every usage surface — the chat banner on both clients and both settings
 * usage cards — prints the reset instant through this one function.
 *
 * `fallback` is the caller's translated wording for an unparseable instant:
 * this module has no locale, so it cannot reach the catalogue itself.
 */
export function formatResetTime(isoString: string, fallback: string): string {
  try {
    return new Intl.DateTimeFormat(undefined, {
      hour: 'numeric',
      minute: '2-digit',
      timeZoneName: 'short',
    }).format(new Date(isoString));
  } catch {
    return fallback;
  }
}

/**
 * A usage counter compacted for a quota meter: `145.0K`, `2.0M`, and the
 * plain locale-grouped figure under a thousand.
 */
export function formatCompactNumber(value: number): string {
  if (value >= 1_000_000) {
    return `${(value / 1_000_000).toFixed(1)}M`;
  }
  if (value >= 1_000) {
    return `${(value / 1_000).toFixed(1)}K`;
  }
  return value.toLocaleString();
}

/**
 * Read a turn's `notice` block into the banner it warrants.
 *
 * The block carries the counter the pre-turn quota check actually measured —
 * its value, its cap, its level and its reset instant. Reading those replaced
 * scraping `/in (\d+) seconds/` out of the refusal sentence, which only ever
 * matched English and told the athlete nothing about which cap they had hit.
 */
export function quotaNoticeBanner(notice: ReplyNotice, resetFallback: string): QuotaBanner {
  const time = formatResetTime(notice.resets_at, resetFallback);
  const percent = notice.limit > 0 ? Math.round((notice.current / notice.limit) * 100) : 0;

  // `label` is itself a catalogue key: the client translates it and passes it
  // back in, so the sentence and the thing it names agree on one language.
  const params = {
    label: 'usage.messageQuota',
    current: notice.current,
    limit: notice.limit,
    time,
  };

  if (notice.level === 'burst') {
    return {
      level: 'burst',
      text: { key: 'usage.burstZone', params },
      resetsAt: notice.resets_at,
    };
  }

  return {
    level: 'warning',
    text: { key: 'usage.percentUsed', params: { ...params, percent } },
    resetsAt: notice.resets_at,
  };
}
