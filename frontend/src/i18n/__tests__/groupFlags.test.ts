// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the group flag and weekly-report phrasing — every key the shared formatter emits is in all five locales
// ABOUTME: The key scan never reads packages/, so a mistyped key there would print raw in every language without this

import { describe, expect, it } from 'vitest';
import { i18n, SUPPORTED_LANGUAGES } from '@pierre/i18n';
import {
  HEALTH_FLAG_DETAIL_KEY,
  WEEKLY_REPORT_KEY,
  freshMemberLine,
  healthFlagConcern,
  healthFlagDetail,
  oneDecimal,
  signedWhole,
  weeklyReportRecommendations,
  weeklyReportSummary,
} from '@pierre/shared-constants';
import type { FlagEvidence, GroupAggregateStats, GroupHealthFlag } from '@pierre/shared-types';
import en from '../../../../packages/i18n/src/locales/en/translation.json';
import fr from '../../../../packages/i18n/src/locales/fr/translation.json';
import es from '../../../../packages/i18n/src/locales/es/translation.json';
import de from '../../../../packages/i18n/src/locales/de/translation.json';
import pt from '../../../../packages/i18n/src/locales/pt/translation.json';

const BUNDLES: Record<string, unknown> = { en, fr, es, de, pt };

/** Every key the formatter can hand to `t()`. */
const EMITTED_KEYS = [...Object.values(HEALTH_FLAG_DETAIL_KEY), ...Object.values(WEEKLY_REPORT_KEY)];

function leaf(bundle: unknown, key: string): unknown {
  return key.split('.').reduce<unknown>((node, part) => {
    return node && typeof node === 'object' ? (node as Record<string, unknown>)[part] : undefined;
  }, bundle);
}

function placeholders(text: string): string[] {
  return [...text.matchAll(/\{\{(\w+)\}\}/g)].map((m) => m[1]).sort();
}

/** One flag of each shape the server raises, with its evidence. */
const FLAGS: GroupHealthFlag[] = [
  flag('deep_fatigue', 'critical', { kind: 'form_share', form_pct: -41.6, tsb: -38.2 }),
  flag('overreaching', 'warning', { kind: 'form_share', form_pct: -24.5, tsb: -25 }),
  flag('overreaching', 'warning', { kind: 'overtraining_risk' }),
  flag('inactive', 'warning', { kind: 'inactive_days', days: 11 }),
  flag('volume_drop', 'info', { kind: 'volume_below_group', pct_below: 35 }),
];

function flag(
  flag_type: GroupHealthFlag['flag_type'],
  severity: GroupHealthFlag['severity'],
  evidence: FlagEvidence,
): GroupHealthFlag {
  return { user_id: 'user-1', display_name: 'Marie', flag_type, severity, evidence };
}

const STATS: GroupAggregateStats = {
  total_members: 3,
  active_members: 2,
  avg_weekly_volume_km: 140.83,
  avg_ctl: 100,
  flagged_members: 1,
  weekly_trend: 'declining',
};

describe('group flag and weekly-report phrasing', () => {
  it('names only keys every locale carries, with the placeholders English declares', () => {
    for (const key of EMITTED_KEYS) {
      const english = leaf(en, key);
      expect(typeof english, key).toBe('string');
      for (const language of SUPPORTED_LANGUAGES) {
        const text = leaf(BUNDLES[language], key);
        expect(typeof text, `${language}: ${key}`).toBe('string');
        expect(placeholders(text as string), `${language}: ${key}`).toEqual(placeholders(english as string));
      }
    }
  });

  it('phrases every flag shape in every locale, carrying its number and no raw key', () => {
    for (const language of SUPPORTED_LANGUAGES) {
      const t = i18n.getFixedT(language);
      for (const f of FLAGS) {
        const detail = healthFlagDetail(t, f);
        expect(detail, `${language}: ${f.evidence.kind}`).not.toMatch(/groups\.|\{\{/);
        expect(detail.length).toBeGreaterThan(0);
        expect(healthFlagConcern(t, f)).toContain('Marie');
        expect(healthFlagConcern(t, f)).toContain(detail);
      }
      expect(healthFlagDetail(t, FLAGS[0])).toContain('-42');
      expect(healthFlagDetail(t, FLAGS[0])).toContain('-38');
      expect(healthFlagDetail(t, FLAGS[3])).toContain('11');
      expect(healthFlagDetail(t, FLAGS[4])).toContain('35');
    }
  });

  it('tells the deepest fatigue band from the deep end of a block by flag type', () => {
    const t = i18n.getFixedT('en');
    expect(healthFlagDetail(t, FLAGS[0])).toBe(
      'Form at -42% of chronic load (TSB -38), deepest fatigue band',
    );
    expect(healthFlagDetail(t, FLAGS[1])).toBe(
      // -24.5 is a tie: it goes to the even neighbour, as the chat digest prints it.
      'Form at -24% of chronic load (TSB -25), deep end of the productive zone',
    );
    expect(healthFlagDetail(t, FLAGS[2])).toBe('High overtraining risk, plan some recovery');
  });

  it('renders a French flag and the French report in French', () => {
    const t = i18n.getFixedT('fr');
    expect(healthFlagDetail(t, FLAGS[0])).toBe(
      'Forme à -42 % de sa charge chronique (TSB -38), fatigue profonde',
    );
    expect(healthFlagDetail(t, FLAGS[3])).toBe('Aucune activité depuis 11 jours');
    expect(healthFlagConcern(t, FLAGS[4])).toBe('Marie : Volume hebdo 35 % sous la moyenne du groupe');
    expect(
      freshMemberLine(t, { user_id: 'user-2', display_name: 'Phil', form_pct: 12.4, tsb: 8.6 }),
    ).toBe('Phil : forme fraîche (+12 % de sa charge chronique, TSB +9)');
    expect(weeklyReportSummary(t, 'fr', STATS)).toBe(
      '2/3 membres actifs cette semaine, 140,8 km en moyenne par membre.',
    );
    expect(weeklyReportRecommendations(t, STATS)).toEqual([
      'Membres à risque de surentraînement élevé : 1. Pense à adapter leur récupération.',
      'Volume du groupe en baisse par rapport à la semaine dernière — prends des nouvelles des membres moins actifs.',
    ]);
  });

  it('still says which way a steady week went, with nobody at risk', () => {
    const t = i18n.getFixedT('en');
    expect(weeklyReportRecommendations(t, { ...STATS, flagged_members: 0, weekly_trend: 'stable' })).toEqual([
      'Group volume is steady compared with last week.',
    ]);
  });

  it('writes decimals in the reader notation and signs whole numbers like the server digest', () => {
    expect(oneDecimal('en', 140.83)).toBe('140.8');
    expect(oneDecimal('fr', 140.83)).toBe('140,8');
    expect(oneDecimal('de', 1234.56)).toBe('1234,6');
    // The server's `{:.1}` rounds the exact binary value, ties to even.
    expect(oneDecimal('en', 38.25)).toBe('38.2');
    expect(oneDecimal('en', 140.85)).toBe('140.8');
    expect(oneDecimal('en', 12.35)).toBe('12.3');
    expect(oneDecimal('en', 2.35)).toBe('2.4');
    expect(oneDecimal('en', 0.05)).toBe('0.1');
    expect(oneDecimal('fr', 99.95)).toBe('100,0');
    expect(oneDecimal('en', 0)).toBe('0.0');
    expect(signedWhole(12.4)).toBe('+12');
    expect(signedWhole(-40)).toBe('-40');
    // Ties go to the even neighbour, as Rust's `{:+.0}` prints them.
    expect(signedWhole(-2.5)).toBe('-2');
    expect(signedWhole(2.5)).toBe('+2');
    expect(signedWhole(12.5)).toBe('+12');
    expect(signedWhole(13.5)).toBe('+14');
    expect(signedWhole(-3.5)).toBe('-4');
    expect(signedWhole(-0.4)).toBe('+0');
  });
});
