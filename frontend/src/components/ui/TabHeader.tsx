// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
// ABOUTME: Shared header row for user-facing tabs (Chat, Discover, Groups)
// ABOUTME: Renders icon in gradient circle + description + optional action buttons

import React from 'react';
import { clsx } from 'clsx';

export interface TabHeaderProps {
  /** Icon element (e.g. lucide-react component) rendered inside the gradient circle */
  icon: React.ReactNode;
  /** Tailwind gradient classes for the icon circle (e.g. "boreal-hero-gradient") */
  gradient: string;
  /** Description text shown next to the icon */
  description: React.ReactNode;
  /** Optional action buttons rendered on the right side */
  actions?: React.ReactNode;
}

export function TabHeader({ icon, gradient, description, actions }: TabHeaderProps) {
  return (
    <div className="p-4 md:p-6 border-b ghost-border flex items-center justify-between gap-3 flex-shrink-0">
      <div className="flex items-center gap-3 min-w-0">
        <div
          className={clsx(
            'w-10 h-10 flex-shrink-0 flex items-center justify-center rounded-xl text-on-surface shadow-ambient bg-gradient-to-br',
            gradient
          )}
        >
          {icon}
        </div>
        <p className="text-sm text-on-surface-variant min-w-0 truncate">{description}</p>
      </div>
      {actions && (
        <div className="flex items-center gap-2 flex-shrink-0">
          {actions}
        </div>
      )}
    </div>
  );
}
