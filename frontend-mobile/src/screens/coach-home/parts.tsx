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
import { InitialsAvatar } from '../../components/ui';

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

const PHASE_FILL: Record<CoachRosterPhaseKind | 'draft', string> = {
  prep: 'bg-surface-container-high',
  base: 'bg-surface-container-high',
  build: 'bg-info/15',
  specialty: 'bg-primary/20',
  peak: 'bg-warning/20',
  taper: 'bg-warning/20',
  race: 'bg-on-surface',
  transition: 'border border-dashed border-outline',
  recovery: 'border border-dashed border-outline',
  draft: 'border-[1.5px] border-dashed border-primary',
};

const PHASE_INK: Record<CoachRosterPhaseKind | 'draft', string> = {
  prep: 'text-text-secondary',
  base: 'text-text-secondary',
  build: 'text-on-info-container',
  specialty: 'text-on-primary-container',
  peak: 'text-on-warning-container',
  taper: 'text-on-warning-container',
  race: 'text-surface',
  transition: 'text-text-secondary',
  recovery: 'text-text-secondary',
  draft: 'text-primary',
};

/** The classes a phase band is filled with: a tint per kind, a dashed outline for the quiet ones and for a draft. */
export function phaseFill(kind: CoachRosterPhaseKind | 'draft'): string {
  return PHASE_FILL[kind];
}

/** The ink a label on that band takes. */
export function phaseInk(kind: CoachRosterPhaseKind | 'draft'): string {
  return PHASE_INK[kind];
}
