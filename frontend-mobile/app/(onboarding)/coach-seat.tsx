// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Mobile onboarding step for a coach — get the coach seat: connect TrainingPeaks or Intervals.icu, or ask the team
// ABOUTME: Route gated by RootLayoutNav; the connect screen in its seat mode, so the connect flows are the athlete's own

import React from 'react';
import { OnboardingConnectScreen } from '../../src/screens/onboarding/OnboardingConnectScreen';

export default function CoachSeatRoute() {
  return <OnboardingConnectScreen mode="seat" />;
}
