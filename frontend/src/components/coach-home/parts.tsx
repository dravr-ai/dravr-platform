// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The small pieces Coach Home's three views share — an athlete's avatar, a status dot, the six-step form strip, a phase band
// ABOUTME: Colour is a token everywhere; a status is an 8px dot beside a word, never a red or green verdict

import { clsx } from 'clsx';
import type { FormBand } from '@pierre/shared-types';
import { FORM_BAND_LABEL_KEY } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { FORM_STRIP_STEPS, formStripIndex } from '@pierre/ui-logic';
import { avatarSlot, initialsFor } from '@pierre/chat-utils';
import { avatarShapeClass, avatarSlotClass } from '../chat/avatarSlots';

const AVATAR_SIZE = { sm: 'h-7 w-7 text-xs', md: 'h-9 w-9 text-sm', lg: 'h-11 w-11 text-base' } as const;

/**
 * An athlete's initials on the avatar slot their user id hashes to: the colour
 * they wear in Group info and their plan thread. A non-sharer wears the quiet one.
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
    <span
      aria-hidden="true"
      className={clsx(
        'flex shrink-0 items-center justify-center font-semibold',
        avatarShapeClass('circle'),
        AVATAR_SIZE[size],
        muted
          ? 'bg-surface-container-high text-on-surface-variant'
          : avatarSlotClass(avatarSlot({ id: userId, agent_id: null, group_id: null, subject_user_id: null })),
      )}
    >
      {initialsFor(name)}
    </span>
  );
}

type DotTone = 'ok' | 'warn' | 'bad' | 'none';

const DOT_CLASS: Record<DotTone, string> = {
  ok: 'bg-success',
  warn: 'bg-warning',
  bad: 'bg-error',
  none: 'bg-outline',
};

/** The 8px dot of a status: it sits beside the word that says it. */
export function StatusDot({ tone, className }: { tone: DotTone; className?: string }) {
  return <span aria-hidden="true" className={clsx('inline-block h-2 w-2 shrink-0 rounded-full', DOT_CLASS[tone], className)} />;
}

/**
 * Form on the athlete's own scale: six steps from deep fatigue to very fresh,
 * the band the server named lit. `insufficient_history` lights none. The
 * strip carries no number and sets no athlete against another.
 */
export function FormStrip({
  band,
  size = 'md',
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
    <span
      role="img"
      aria-label={label}
      data-testid="form-strip"
      data-band={band}
      className={clsx('flex shrink-0', size === 'md' ? 'w-full gap-[3px]' : 'gap-0.5', muted && 'opacity-50')}
    >
      {FORM_STRIP_STEPS.map((step, index) => (
        <span
          key={step}
          className={clsx(
            size === 'md' ? 'h-1.5 flex-1 rounded-[3px]' : 'h-2 w-[7px] rounded-sm',
            index === lit ? 'bg-on-surface' : 'bg-outline-variant',
          )}
        />
      ))}
    </span>
  );
}
