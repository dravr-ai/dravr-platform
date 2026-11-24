// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Phrases group health flags and the weekly report from their numbers, in the reader's language
// ABOUTME: The server sends evidence and stats only; both clients render them through these keys and their own t()

import { distanceInUnit, distanceSymbol, METRES_PER_KILOMETRE, type DistanceUnit } from '@pierre/chat-utils';
import type {
  FreshMember,
  GroupAggregateStats,
  GroupAnalyticsScope,
  GroupDigestMode,
  GroupHealthFlag,
  GroupRole,
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
  scope: 'groups.insightsScope',
  summary: 'groups.report.summaryDistance',
  fresh: 'groups.report.fresh',
  concern: 'groups.report.concern',
  reviewDeepFatigue: 'groups.report.reviewDeepFatigue',
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

/**
 * A weekly volume the server counts in kilometres, as the reader's `units`
 * count it, to one decimal in the reader's notation and without its unit:
 * `41.5` km reads `25.8` on imperial. Rounded like the server's digest
 * ([`oneDecimal`]), so the panel and the reader's own digest agree.
 */
export function weeklyVolumeFigure(language: string, km: number, units: DistanceUnit): string {
  return oneDecimal(language, distanceInUnit(km * METRES_PER_KILOMETRE, units));
}

/** A weekly volume in kilometres as the reader's `units` say it, unit included: `41.5 km`, `25.8 mi`. */
export function weeklyVolume(language: string, km: number, units: DistanceUnit): string {
  return `${weeklyVolumeFigure(language, km, units)} ${distanceSymbol(units)}`;
}

/**
 * The line saying a group's figures cover only the members who share their
 * training with the reader, and how many of the group that is; `null` when
 * they cover everyone. The server computes the stats, the weekly report and
 * the health flags over that one set, whatever the reader's role in the
 * group, so one line under the stats speaks for all three.
 */
export function analyticsScopeLine(t: Translate, scope: GroupAnalyticsScope | null): string | null {
  if (!scope || scope.visible_members >= scope.roster_members) {
    return null;
  }
  return t(WEEKLY_REPORT_KEY.scope, {
    shown: scope.visible_members,
    total: scope.roster_members,
  });
}

/**
 * The weekly report's opening sentence: who was active, and how far they
 * went, in the reader's units. `null` when no member's week is read: the
 * sentence is built around the average, and there is none to write.
 */
export function weeklyReportSummary(
  t: Translate,
  language: string,
  stats: GroupAggregateStats,
  units: DistanceUnit,
): string | null {
  if (stats.avg_weekly_volume_km === null) {
    return null;
  }
  return t(WEEKLY_REPORT_KEY.summary, {
    active: stats.active_members,
    total: stats.total_members,
    distance: weeklyVolume(language, stats.avg_weekly_volume_km, units),
  });
}

/**
 * The weekly report's recommended actions: a look at the members in deep
 * fatigue when there are any, then the group's volume trend. The trend
 * compares this week's average with the one before, so a week no member is
 * read for has no trend line.
 *
 * `flagged_members` counts the members whose form sits past -30% of their own
 * chronic load (the deep-fatigue band, Methodology › Training Load). The line
 * says so as a magnitude, never as an overtraining or injury risk: no load
 * threshold predicts either (Impellizzeri et al., 2020).
 */
export function weeklyReportRecommendations(t: Translate, stats: GroupAggregateStats): string[] {
  const lines: string[] = [];
  if (stats.flagged_members > 0) {
    lines.push(t(WEEKLY_REPORT_KEY.reviewDeepFatigue, { n: stats.flagged_members }));
  }
  if (stats.avg_weekly_volume_km !== null) {
    lines.push(t(TREND_KEY[stats.weekly_trend]));
  }
  return lines;
}

/**
 * Each weekly-digest mode's option label and the one-line hint shown beside it
 * while it is chosen, as corpus keys: the table is built at import time, where
 * `t` does not exist, and each client's render resolves them.
 */
export const DIGEST_MODES: Record<GroupDigestMode, { labelKey: string; hintKey: string }> = {
  off: { labelKey: 'groups.digestOff', hintKey: 'groups.digestOffHint' },
  chat: { labelKey: 'groups.digestChat', hintKey: 'groups.digestChatHint' },
  managers: { labelKey: 'groups.digestManagers', hintKey: 'groups.digestManagersHint' },
};

/** The weekly-digest modes in the order a settings surface offers them. */
export const DIGEST_MODE_ORDER: readonly GroupDigestMode[] = ['off', 'chat', 'managers'];

/**
 * Whether a member of `role` manages the group — its invites, its settings and
 * who is in it: the owner or an admin, as the server's routes allow. A viewer
 * with no membership (`undefined`) manages nothing.
 */
export function canManageGroup(role: GroupRole | null | undefined): boolean {
  return role === 'owner' || role === 'admin';
}
