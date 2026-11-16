// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A tinted message block — an inline error, a success confirmation, a warning or an informational note
// ABOUTME: The `.alert` shape under a tone from ALERT_TONE, so its body ink clears AA on every ground in both themes

import React from 'react';
import { clsx } from 'clsx';
import { ALERT_TONE, type AlertTone } from './alertTone';

export interface AlertProps extends React.HTMLAttributes<HTMLDivElement> {
  tone: AlertTone;
  children: React.ReactNode;
}

/**
 * The block's colour comes from its tone and nothing else. `className` adds
 * layout or reshapes it (a margin, `p-4`, `flex`), and `role` is the caller's
 * call: a message that appears in answer to an action is `alert`, a standing
 * note is not.
 */
export const Alert: React.FC<AlertProps> = ({ tone, children, className, ...props }) => (
  <div className={clsx('alert', ALERT_TONE[tone].tone, className)} {...props}>
    {children}
  </div>
);
