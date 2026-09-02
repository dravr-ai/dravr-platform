// ABOUTME: Shared full-screen onboarding chrome (gradient accent bar + centered Card) used across onboarding steps
// ABOUTME: Extracted from the byte-identical Shell previously duplicated in OnboardingProfileType and OnboardingCoachProposal

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import type { ReactNode } from 'react';
import { Card } from './ui';

/** Shared full-screen onboarding chrome (gradient accent + card). */
export default function OnboardingShell({
  heading,
  children,
}: {
  heading?: string;
  children: ReactNode;
}) {
  // `pt-20` reserves a top safe-area for the fixed OnboardingProgress bar;
  // `flex-col` + `my-auto` centers short cards but lets tall cards (e.g. the
  // provider list) top-align and scroll below the bar rather than under it.
  return (
    <div
      data-testid="onboarding-shell"
      className="min-h-dvh flex flex-col items-center px-4 sm:px-6 lg:px-8 py-12 pt-20 bg-surface-container-low"
    >
      <div className="max-w-2xl w-full my-auto">
        <Card className="overflow-hidden">
          <div className="h-1 w-full boreal-hero-gradient" />
          <div className="px-8 py-10">
            {heading ? (
              <h1 className="font-display font-semibold text-2xl text-on-surface text-center">
                {heading}
              </h1>
            ) : null}
            {children}
          </div>
        </Card>
      </div>
    </div>
  );
}
