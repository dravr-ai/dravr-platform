// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The small pieces Coach Home's phone views share — an athlete's avatar, a section label with its count, the six-step form strip, a phase band
// ABOUTME: Colour is a token everywhere; form is the server's band lit on a six-step strip, never a number and never set against another athlete

import React from 'react';
import { Text, View } from 'react-native';
import type { CoachRosterPhaseKind, FormBand } from '@pierre/shared-types';
import { FORM_BAND_LABEL_KEY } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { FORM_STRIP_STEPS, formStripIndex } from '@pierre/ui-logic';
import { avatarSlot, initialsFor } from '@pierre/chat-utils';
import { BADGE_TONE, InitialsAvatar } from '../../components/ui';

const AVATAR_SIZE = { sm: 32, md: 36, lg: 44 } as const;

/**
 * An athlete's initials on the avatar slot their user id hashes to: the colour
 * they wear in Group info and their plan thread. One who does not share wears
 * the quiet one.
 */
export function AthleteAvatar({
  userId,
  name,
  size = 'md',
  muted = false,
}: {
  userId: string;
  name: string;
  size?: keyof typeof AVATAR_SIZE;
  muted?: boolean;
}) {
  return (
    <InitialsAvatar
      initials={initialsFor(name)}
      slot={avatarSlot({ id: userId, agent_id: null, group_id: null })}
      size={AVATAR_SIZE[size]}
      muted={muted}
    />
  );
}

/**
 * The small label over a block of rows, with the block's count beside it in
 * mono when it has one. `className` carries the block's padding.
 */
export function GroupLabel({
  children,
  count,
  className = 'pb-1 pt-4',
}: {
  children: string;
  count?: number;
  className?: string;
}) {
  return (
    <View className={`flex-row items-baseline gap-2 ${className}`}>
      <Text className="text-xs font-medium text-text-secondary">{children}</Text>
      {count !== undefined && <Text className="font-mono text-xs text-text-secondary">{count}</Text>}
    </View>
  );
}

/**
 * Form on the athlete's own scale: six steps from deep fatigue to very fresh,
 * the band the server named lit. `insufficient_history` lights none. The
 * strip carries no number and sets no athlete against another.
 */
export function FormStrip({
  band,
  size = 'sm',
  muted = false,
}: {
  band: FormBand;
  size?: 'sm' | 'md';
  /** The reading is old (the data is cut): drawn quieter. */
  muted?: boolean;
}) {
  const { t } = useTranslation();
  const lit = formStripIndex(band);
  const name = t(FORM_BAND_LABEL_KEY[band]);
  const label =
    lit < 0
      ? t('coachHome.detail.formAriaNone', { band: name })
      : t('coachHome.detail.formAria', { band: name, position: lit + 1, total: FORM_STRIP_STEPS.length });
  return (
    <View
      accessible
      accessibilityRole="image"
      accessibilityLabel={label}
      testID="form-strip"
      className={`flex-row ${size === 'md' ? 'w-full gap-[3px]' : 'shrink-0 gap-0.5'} ${muted ? 'opacity-50' : ''}`}
    >
      {FORM_STRIP_STEPS.map((step, index) => (
        <View
          key={step}
          testID={index === lit ? 'form-strip-lit' : undefined}
          className={`${size === 'md' ? 'h-1.5 flex-1 rounded-sm' : 'h-2 w-1.5 rounded-sm'} ${
            index === lit ? 'bg-on-surface' : 'bg-outline-variant'
          }`}
        />
      ))}
    </View>
  );
}

/**
 * Light keeps the board's soft tints. On the dark canvas those tints sit at
 * about 1.3:1 and base and build blend into one band, so dark lifts each fill
 * to about 3:1 against the page while keeping the hues apart: a phase is told
 * by its fill alone on the season bars, which carry no label.
 */
const PHASE_FILL: Record<CoachRosterPhaseKind | 'draft', string> = {
  prep: 'bg-surface-container-high dark:bg-outline/50',
  base: 'bg-surface-container-high dark:bg-outline/50',
  build: 'bg-info/15 dark:bg-info/50',
  specialty: 'bg-primary/20 dark:bg-primary/45',
  peak: 'bg-warning/20 dark:bg-warning/50',
  taper: 'bg-warning/20 dark:bg-warning/50',
  race: 'bg-on-surface',
  transition: 'border border-dashed border-outline',
  recovery: 'border border-dashed border-outline',
  draft: 'border-[1.5px] border-dashed border-primary',
};

/**
 * A band that carries its label — the athlete screen's plan row — keeps the
 * light tint in both schemes and labels it in the ink bound to that tint. The
 * season bars' dark lift above is for a fill read on its own; under a label
 * it costs the words, which read 2.40:1 there (peak on `warning/50`, dark,
 * `surface-container-highest`). The tints are the badge's, measured with
 * their ink on every ground in both schemes (`CoachHomeParts.test.tsx`), so
 * peak and taper sit at /15: at /20 their ink reads 4.41:1 on the top tier.
 * A band tells its phase by its label here, so the light tints need not stay
 * apart in dark.
 */
const PHASE_BAND: Record<CoachRosterPhaseKind | 'draft', { fill: string; ink: string }> = {
  prep: { fill: BADGE_TONE.neutral.fill, ink: 'text-text-secondary' },
  base: { fill: BADGE_TONE.neutral.fill, ink: 'text-text-secondary' },
  build: { fill: BADGE_TONE.info.fill, ink: BADGE_TONE.info.ink },
  specialty: { fill: 'bg-primary/20', ink: 'text-on-primary-container' },
  peak: { fill: BADGE_TONE.warning.fill, ink: BADGE_TONE.warning.ink },
  taper: { fill: BADGE_TONE.warning.fill, ink: BADGE_TONE.warning.ink },
  race: { fill: 'bg-on-surface', ink: 'text-surface' },
  transition: { fill: 'border border-dashed border-outline', ink: 'text-text-secondary' },
  recovery: { fill: 'border border-dashed border-outline', ink: 'text-text-secondary' },
  draft: { fill: 'border-[1.5px] border-dashed border-primary', ink: 'text-primary' },
};

/** The classes a season bar is filled with: a tint per kind, lifted in dark, a dashed outline for the quiet ones and for a draft. */
export function phaseFill(kind: CoachRosterPhaseKind | 'draft'): string {
  return PHASE_FILL[kind];
}

/** A band that carries its label: the fill it takes in both schemes and the ink its label takes. */
export function phaseBand(kind: CoachRosterPhaseKind | 'draft'): { fill: string; ink: string } {
  return PHASE_BAND[kind];
}
