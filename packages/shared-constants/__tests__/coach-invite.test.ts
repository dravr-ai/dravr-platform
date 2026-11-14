// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves which bar segments a coach-seat invitation settles, the line that names them, and that the line resolves in every catalogue
// ABOUTME: Only segments the bar shows done, only when the server says an invitation settled them; never for a coach who answered alone

import { describe, expect, it } from 'vitest';
import de from '../../i18n/src/locales/de/translation.json';
import en from '../../i18n/src/locales/en/translation.json';
import es from '../../i18n/src/locales/es/translation.json';
import fr from '../../i18n/src/locales/fr/translation.json';
import pt from '../../i18n/src/locales/pt/translation.json';
import { SETTLED_BY_INVITE_KEY, settledByInviteLine, settledByInviteSegments } from '../src/coach-invite';
import type { OnboardingProgressItem } from '../src/onboarding';

const BAR: OnboardingProgressItem[] = [
  { id: 'profile_type', labelKey: 'onboarding.stepProfile', status: 'done' },
  { id: 'coach_seat', labelKey: 'humanCoach.stepSeat', status: 'done' },
  { id: 'coach_group', labelKey: 'onboarding.stepGroup', status: 'current' },
  { id: 'coach_invite', labelKey: 'onboarding.stepInvite', status: 'upcoming' },
];

/** A translator over one catalogue, interpolating `{{name}}` as i18next does. */
function translator(catalogue: Record<string, unknown>) {
  return (key: string, params: Record<string, unknown> = {}): string => {
    const value = key.split('.').reduce<unknown>((node, part) => (node as Record<string, unknown>)?.[part], catalogue);
    if (typeof value !== 'string') return key;
    return value.replace(/\{\{(\w+)\}\}/g, (_, name: string) => String(params[name] ?? ''));
  };
}

describe('the coach-seat invitation on the progress bar', () => {
  it('settles the role question and the seat, nothing else', () => {
    const allDone = BAR.map((s) => ({ ...s, status: 'done' as const }));
    expect(settledByInviteSegments(allDone, true).map((s) => s.id)).toEqual(['profile_type', 'coach_seat']);
  });

  it('names the done segments it settled when the server says an invitation settled them', () => {
    expect(settledByInviteSegments(BAR, true).map((s) => s.id)).toEqual(['profile_type', 'coach_seat']);
  });

  it('names none for a coach who answered for themselves', () => {
    expect(settledByInviteSegments(BAR, false)).toEqual([]);
    expect(settledByInviteLine(translator(en), BAR, false, 'en')).toBeNull();
  });

  it('never calls a segment still ahead settled', () => {
    const ahead = BAR.map((s) => (s.id === 'coach_seat' ? { ...s, status: 'upcoming' as const } : s));
    expect(settledByInviteSegments(ahead, true).map((s) => s.id)).toEqual(['profile_type']);
  });

  it('says the line in the reader language, the labels joined as a list', () => {
    expect(settledByInviteLine(translator(fr), BAR, true, 'fr')).toBe('Profil et Place de coach réglés par ton invitation');
    expect(settledByInviteLine(translator(en), BAR, true, 'en')).toBe('Profile and Coach seat settled by your invitation');
  });

  it('resolves in all five catalogues with its steps placeholder', () => {
    for (const catalogue of [fr, en, es, de, pt]) {
      const value = translator(catalogue)(SETTLED_BY_INVITE_KEY, { steps: 'X' });
      expect(value).not.toBe(SETTLED_BY_INVITE_KEY);
      expect(value).toContain('X');
    }
  });
});
