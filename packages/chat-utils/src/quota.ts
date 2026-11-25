// ABOUTME: The usage banner both clients show: decided from the status endpoint's counters or a turn's own `notice` block
// ABOUTME: One decision, one wording and one counter reading; the clients keep only the query and the translator

import type { LimitCheckResult, ReplyNotice, UsageStatusResponse } from '@pierre/shared-types';

import { formatCount, formatDecimal } from './number-format';
import type { TranslatableText, Translate } from './text';

/** What a quota notice puts on the banner. */
export interface QuotaBanner {
  /** How close the cap is, in the banner's own vocabulary. */
  level: 'warning' | 'burst';
  /** The sentence to show, as a catalogue key the client translates. */
  text: TranslatableText;
  /** RFC3339 instant the counter resets at. */
  resetsAt: string;
}

/** A usage counter the status endpoint reports, by the sentence that names it. */
export type UsageCounter = 'dailyMessages' | 'dailyTokens' | 'weeklyMessages';

/**
 * The banner sentence for each counter at each level: one whole sentence per
 * counter, never a sentence with the counter's name slotted in. "Limite
 * {{label}} atteinte" filled with "tes messages quotidiens" read "Limite tes
 * messages quotidiens atteinte", and German and Portuguese took the slotted
 * name in the wrong case. `messageQuota` is the counter a turn's own notice
 * names; a notice warns and never blocks, so it has no `reached` sentence.
 */
export const USAGE_SENTENCE_KEYS = {
  reached: {
    dailyMessages: 'usage.reached.dailyMessages',
    dailyTokens: 'usage.reached.dailyTokens',
    weeklyMessages: 'usage.reached.weeklyMessages',
  },
  burst: {
    dailyMessages: 'usage.burst.dailyMessages',
    dailyTokens: 'usage.burst.dailyTokens',
    weeklyMessages: 'usage.burst.weeklyMessages',
    messageQuota: 'usage.burst.messageQuota',
  },
  used: {
    dailyMessages: 'usage.used.dailyMessages',
    dailyTokens: 'usage.used.dailyTokens',
    weeklyMessages: 'usage.used.weeklyMessages',
    messageQuota: 'usage.used.messageQuota',
  },
} as const satisfies {
  reached: Record<UsageCounter, string>;
  burst: Record<UsageCounter | 'messageQuota', string>;
  used: Record<UsageCounter | 'messageQuota', string>;
};

/**
 * The reset instant in the reader's language and own timezone: `12:00 AM UTC`
 * in English, `00:00 UTC` in French.
 *
 * Every usage surface — the chat banner on both clients and both settings
 * usage cards — prints the reset instant through this one function.
 *
 * `fallback` is the caller's translated wording for an unparseable instant:
 * this module has no catalogue, so it cannot reach the wording itself.
 */
export function formatResetTime(isoString: string, fallback: string, language: string): string {
  try {
    return new Intl.DateTimeFormat(language, {
      hour: 'numeric',
      minute: '2-digit',
      timeZoneName: 'short',
    }).format(new Date(isoString));
  } catch {
    return fallback;
  }
}

/** The catalogue keys of the short-scale suffix a compacted count is written with. */
const COMPACT_COUNT_KEYS = {
  thousands: 'common.compact.thousands',
  millions: 'common.compact.millions',
} as const;

/**
 * A usage counter compacted for a quota meter, in the reader's notation:
 * `145.0K` and `2.0M` in English, `145,0 k` and `2,0 M` in French. Under a
 * thousand it is the plain grouped figure.
 *
 * The suffix comes from the catalogue rather than from `Intl.NumberFormat`'s
 * compact notation, which the phone's JavaScript engine does not carry on
 * every platform.
 */
export function formatCompactNumber(value: number, t: Translate, language: string): string {
  if (value >= 1_000_000) {
    return t(COMPACT_COUNT_KEYS.millions, { value: formatDecimal(value / 1_000_000, 1, language) });
  }
  if (value >= 1_000) {
    return t(COMPACT_COUNT_KEYS.thousands, { value: formatDecimal(value / 1_000, 1, language) });
  }
  return formatCount(value, language);
}

/**
 * Read a turn's `notice` block into the banner it warrants.
 *
 * The block carries the counter the pre-turn quota check actually measured —
 * its value, its cap, its level and its reset instant. Reading those replaced
 * extracting `/in (\d+) seconds/` out of the refusal sentence, which only ever
 * matched English and told the athlete nothing about which cap they had hit.
 */
export function quotaNoticeBanner(notice: ReplyNotice, resetFallback: string, language: string): QuotaBanner {
  const time = formatResetTime(notice.resets_at, resetFallback, language);
  const percent = notice.limit > 0 ? Math.round((notice.current / notice.limit) * 100) : 0;

  // The figures are grouped in the reader's notation — `456 792` in French —
  // because the sentence prints them as the athlete reads numbers.
  const params = {
    current: formatCount(notice.current, language),
    limit: formatCount(notice.limit, language),
    time,
  };

  if (notice.level === 'burst') {
    return {
      level: 'burst',
      text: { key: USAGE_SENTENCE_KEYS.burst.messageQuota, params },
      resetsAt: notice.resets_at,
    };
  }

  return {
    level: 'warning',
    text: { key: USAGE_SENTENCE_KEYS.used.messageQuota, params: { ...params, percent } },
    resetsAt: notice.resets_at,
  };
}

/** How loudly the usage banner speaks: silent, approaching a cap, inside the burst allowance, or at the cap. */
export type WarningLevel = 'none' | 'warning' | 'burst' | 'blocked';

/** What the chat surface does about usage: the banner it shows and whether sending is closed. */
export interface UsageWarningState {
  /** The most severe level across the counters read. */
  level: WarningLevel;
  /** Whether sending a message is closed. */
  sendDisabled: boolean;
  /** The banner sentence, as a catalogue key plus its params; `null` when there is nothing to say. */
  text: TranslatableText | null;
  /** RFC3339 instant the triggering counter resets at; empty when silent. */
  resetsAt: string;
  /** The counter that set the level; `null` when silent or when a turn's notice set it. */
  triggerCounter: LimitCheckResult | null;
}

/** The level one counter alone would set. */
function getCounterLevel(counter: LimitCheckResult): WarningLevel {
  if (!counter.allowed) return 'blocked';
  if (counter.burst_zone) return 'burst';
  if (counter.warning) return 'warning';
  return 'none';
}

/** Severity order: blocked > burst > warning > none. */
const LEVEL_PRIORITY: Record<WarningLevel, number> = {
  none: 0,
  warning: 1,
  burst: 2,
  blocked: 3,
};

/** The state with nothing to show. */
const NO_USAGE_WARNING: UsageWarningState = {
  level: 'none',
  sendDisabled: false,
  text: null,
  resetsAt: '',
  triggerCounter: null,
};

/**
 * Decide the banner from the status endpoint's answer: the worst of the daily
 * message, daily token and weekly message counters speaks, in its own whole
 * sentence, so no locale inflects a slotted counter name.
 *
 * `resetFallback` is the caller's translated wording for an unparseable reset
 * instant, and `language` the one the figures and the instant are spelled in.
 * The sentence comes back as a catalogue key: hardcoded English templates once
 * rendered verbatim under French chrome.
 */
export function computeWarningState(
  data: UsageStatusResponse | undefined,
  resetFallback: string,
  language: string,
): UsageWarningState {
  if (!data) return NO_USAGE_WARNING;

  const counters: Array<{ counter: LimitCheckResult; name: UsageCounter }> = [
    { counter: data.daily.messages, name: 'dailyMessages' },
    { counter: data.daily.tokens, name: 'dailyTokens' },
    { counter: data.weekly.messages, name: 'weeklyMessages' },
  ];

  let worstLevel: WarningLevel = 'none';
  let worstCounter: LimitCheckResult | null = null;
  let worstName: UsageCounter = 'dailyMessages';

  for (const { counter, name } of counters) {
    const level = getCounterLevel(counter);
    if (LEVEL_PRIORITY[level] > LEVEL_PRIORITY[worstLevel]) {
      worstLevel = level;
      worstCounter = counter;
      worstName = name;
    }
  }

  if (!worstCounter || worstLevel === 'none') return NO_USAGE_WARNING;

  // Grouped in the athlete's notation: the raw `456792/500000` read as one
  // unbroken string of digits under any chrome.
  const params = {
    current: formatCount(worstCounter.current, language),
    limit: formatCount(worstCounter.limit, language),
    time: formatResetTime(worstCounter.resets_at, resetFallback, language),
  };
  const percent = worstCounter.limit > 0
    ? Math.round((worstCounter.current / worstCounter.limit) * 100)
    : 0;

  let text: TranslatableText;
  switch (worstLevel) {
    case 'blocked':
      text = { key: USAGE_SENTENCE_KEYS.reached[worstName], params };
      break;
    case 'burst':
      text = { key: USAGE_SENTENCE_KEYS.burst[worstName], params };
      break;
    case 'warning':
      text = { key: USAGE_SENTENCE_KEYS.used[worstName], params: { ...params, percent } };
      break;
  }

  return {
    level: worstLevel,
    sendDisabled: worstLevel === 'blocked',
    text,
    resetsAt: worstCounter.resets_at,
    triggerCounter: worstCounter,
  };
}

/**
 * Decide the banner from a turn's own `notice` reply block, in the wording
 * {@link quotaNoticeBanner} gives it. A notice never blocks sending: the turn
 * it rode already succeeded.
 */
export function warningStateFromNotice(
  notice: ReplyNotice,
  resetFallback: string,
  language: string,
): UsageWarningState {
  const banner = quotaNoticeBanner(notice, resetFallback, language);
  return {
    level: banner.level,
    sendDisabled: false,
    text: banner.text,
    resetsAt: banner.resetsAt,
    triggerCounter: null,
  };
}
