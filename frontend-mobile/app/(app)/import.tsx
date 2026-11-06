// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The import-your-own-data modal over the app, opened from the bottom of Connections and the Strava card
// ABOUTME: `?source=<offered source>` (`strava_archive`, `whoop_archive`, ...) opens straight on that source's import

import { useLocalSearchParams } from 'expo-router';
import { ImportScreen } from '../../src/screens/import/ImportScreen';

export default function ImportRoute() {
  const { source } = useLocalSearchParams<{ source?: string }>();
  return <ImportScreen initialSource={source} />;
}
