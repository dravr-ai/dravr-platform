// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React from 'react';
import { clsx } from 'clsx';
import { BADGE_TONE, type BadgeTone } from './badgeTone';

/** The tones by name, plus the admin alias `destructive`, which names the error tone. */
type BadgeVariant = BadgeTone | 'destructive';

/** An action name drawn in an existing tone rather than one of its own. */
const VARIANT_TONE: Record<BadgeVariant, BadgeTone> = {
  neutral: 'neutral',
  primary: 'primary',
  success: 'success',
  warning: 'warning',
  error: 'error',
  info: 'info',
  accent: 'accent',
  activity: 'activity',
  nutrition: 'nutrition',
  recovery: 'recovery',
  mobility: 'mobility',
  secondary: 'secondary',
  destructive: 'error',
};

interface BadgeProps {
  variant: BadgeVariant;
  children: React.ReactNode;
  className?: string;
  'data-testid'?: string;
}

export const Badge: React.FC<BadgeProps> = ({ variant, children, className, 'data-testid': testId }) => (
  <span className={clsx('badge', BADGE_TONE[VARIANT_TONE[variant]].tone, className)} data-testid={testId}>
    {children}
  </span>
);
