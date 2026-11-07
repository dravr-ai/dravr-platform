// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The import-your-own-data screen inside onboarding's connect step, opened from below the providers or a card's export link
// ABOUTME: `?source=<offered source>` opens straight on that source's import; the screen carries its own close back to the connect step

import { useLocalSearchParams, useRouter } from 'expo-router';
import { ImportScreen } from '../../src/screens/import/ImportScreen';

export default function OnboardingImportRoute() {
  const router = useRouter();
  const { source } = useLocalSearchParams<{ source?: string }>();
  return <ImportScreen initialSource={source} onClose={() => router.back()} />;
}
