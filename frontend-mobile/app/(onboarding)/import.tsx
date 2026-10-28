// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The import-your-own-data screen inside onboarding's connect step, opened from below the providers
// ABOUTME: The onboarding stack has no header, so the screen carries its own close back to the connect step

import { useRouter } from 'expo-router';
import { ImportScreen } from '../../src/screens/import/ImportScreen';

export default function OnboardingImportRoute() {
  const router = useRouter();
  return <ImportScreen onClose={() => router.back()} />;
}
