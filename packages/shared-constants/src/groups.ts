// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Phrases group health flags and the weekly report from their numbers, in the reader's language
// ABOUTME: The server sends evidence and stats only; both clients render them through these keys and their own t()

import type {
  FreshMember,
  GroupAggregateStats,
  GroupHealthFlag,
  GroupTrend,
} from '@pierre/shared-types';

/** The corpus key for each phrasing of a health flag's evidence. */
export const HEALTH_FLAG_DETAIL_KEY = {
  deepFatigue: 'groups.flagDetail.deepFatigue',
  heavyBlock: 'groups.flagDetail.heavyBlock',
  overtrainingRisk: 'groups.flagDetail.overtrainingRisk',
  inactive: 'groups.flagDetail.inactive',
  volumeBelowGroup: 'groups.flagDetail.volumeBelowGroup',
} as const;

/** The corpus key for each line of the weekly report. */
export const WEEKLY_REPORT_KEY = {
  summary: 'groups.report.summary',
  fresh: 'groups.report.fresh',
  concern: 'groups.report.concern',
  reviewFlagged: 'groups.report.reviewFlagged',
  trendDeclining: 'groups.report.trendDeclining',
  trendImproving: 'groups.report.trendImproving',
  trendStable: 'groups.report.trendStable',
} as const;

/** The line each volume trend reads as, so the report always says which way the group went. */
const TREND_KEY: Record<GroupTrend, string> = {
  improving: WEEKLY_REPORT_KEY.trendImproving,
  declining: WEEKLY_REPORT_KEY.trendDeclining,
  stable: WEEKLY_REPORT_KEY.trendStable,
};

/** The shape of `t()` this module needs: a key and its interpolation values. */
type Translate = (key: string, values?: Record<string, string | number>) => string;

/**
 * A number rounded to a whole value with its sign always shown: `+12`, `-40`.
 *
 * Ties round to the even neighbour (`12.5` → `+12`, `-2.5` → `-2`), which is
 * how the server's digest formats the same form share, so the panel and the
 * chat digest print the same figure. A value that rounds to zero reads `+0`.
 */
export function signedWhole(value: number): string {
  const magnitude = Math.abs(roundHalfEven(value));
  return `${value < 0 && magnitude !== 0 ? '-' : '+'}${magnitude}`;
}

/** `value` rounded to a whole number, ties to the even neighbour. */
function roundHalfEven(value: number): number {
  const floor = Math.floor(value);
  const fraction = value - floor;
  if (fraction > 0.5) {
    return floor + 1;
  }
  if (fraction < 0.5) {
    return floor;
  }
  return floor % 2 === 0 ? floor : floor + 1;
}

/**
 * A number with one decimal in the reader's notation — `41.5` in English,
 * `41,5` in French — without thousands grouping, rounded the way the server's
 * digest rounds it so the panel and the chat digest print the same figure.
 *
 * The server formats with Rust's `{:.1}`, which rounds the double's exact
 * binary value and sends a true tie to the even tenth: `38.25` → `38.2`,
 * while `140.85` (stored just below the tie) → `140.8`. `Intl.NumberFormat`
 * rounds the shortest decimal form half away from zero and would print
 * `38.3` and `140.9`, so the tenth is chosen here ([`nearestTenthHalfEven`])
 * and `Intl.NumberFormat` only writes it. It is the one Intl service both the
 * browser and the phone's Hermes runtime carry in full.
 */
export function oneDecimal(language: string, value: number): string {
  const tenths = nearestTenthHalfEven(value);
  return new Intl.NumberFormat(language, {
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
    useGrouping: false,
  }).format(tenths === 0 ? 0 : tenths / 10);
}

/**
 * `value` in whole tenths, rounded from its exact binary value with ties to
 * the even tenth — the rounding of Rust's `{:.1}`.
 *
 * `toFixed(100)` spells out a double's exact value for every magnitude a
 * distance or a volume takes, so the digits after the first decimal decide
 * the rounding without any floating-point step.
 */
function nearestTenthHalfEven(value: number): number {
  const [whole, fraction] = Math.abs(value).toFixed(100).split('.');
  const truncated = Number(whole) * 10 + Number(fraction[0]);
  const rest = fraction.slice(1);
  const pastHalf = rest[0] > '5' || (rest[0] === '5' && /[1-9]/.test(rest.slice(1)));
  const tie = rest[0] === '5' && !pastHalf;
  const rounded = pastHalf || (tie && truncated % 2 === 1) ? truncated + 1 : truncated;
  return value < 0 ? -rounded : rounded;
}

/**
 * The sentence describing why a member was flagged, from the flag's evidence.
 *
 * Form in a fatigue band reads as the deepest band or as the deep end of the
 * productive zone according to the flag type, the same split the server makes
 * when it raises the flag.
 */
export function healthFlagDetail(t: Translate, flag: Pick<GroupHealthFlag, 'flag_type' | 'evidence'>): string {
  const { evidence } = flag;
  switch (evidence.kind) {
    case 'form_share':
      return t(
        flag.flag_type === 'deep_fatigue'
          ? HEALTH_FLAG_DETAIL_KEY.deepFatigue
          : HEALTH_FLAG_DETAIL_KEY.heavyBlock,
        { pct: signedWhole(evidence.form_pct), tsb: signedWhole(evidence.tsb) },
      );
    case 'overtraining_risk':
      return t(HEALTH_FLAG_DETAIL_KEY.overtrainingRisk);
    case 'inactive_days':
      return t(HEALTH_FLAG_DETAIL_KEY.inactive, { days: evidence.days });
    case 'volume_below_group':
      return t(HEALTH_FLAG_DETAIL_KEY.volumeBelowGroup, { pct: evidence.pct_below });
  }
}

/** One concern line of the weekly report: the flagged member and why. */
export function healthFlagConcern(t: Translate, flag: GroupHealthFlag): string {
  return t(WEEKLY_REPORT_KEY.concern, {
    name: flag.display_name,
    detail: healthFlagDetail(t, flag),
  });
}

/** One highlight line of the weekly report: a member in fresh form. */
export function freshMemberLine(t: Translate, member: FreshMember): string {
  return t(WEEKLY_REPORT_KEY.fresh, {
    name: member.display_name,
    pct: signedWhole(member.form_pct),
    tsb: signedWhole(member.tsb),
  });
}

/** The weekly report's opening sentence: who was active, and how far they went. */
export function weeklyReportSummary(t: Translate, language: string, stats: GroupAggregateStats): string {
  return t(WEEKLY_REPORT_KEY.summary, {
    active: stats.active_members,
    total: stats.total_members,
    km: oneDecimal(language, stats.avg_weekly_volume_km),
  });
}

/**
 * The weekly report's recommended actions: a review of the members at high
 * overtraining risk when there are any, then the group's volume trend.
 */
export function weeklyReportRecommendations(t: Translate, stats: GroupAggregateStats): string[] {
  const lines: string[] = [];
  if (stats.flagged_members > 0) {
    lines.push(t(WEEKLY_REPORT_KEY.reviewFlagged, { n: stats.flagged_members }));
  }
  lines.push(t(TREND_KEY[stats.weekly_trend]));
  return lines;
}
