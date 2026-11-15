// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: What a provider connect on the phone says and opens next — an export's route, a refused connect's line on its row
// ABOUTME: Pure helpers shared by Connections, onboarding's connect step and the sign-in window; the card logic is @pierre/ui-logic's

import type { ApiErrorTranslate } from '@pierre/ui-logic';
import { IMPORT_ROUTE, ONBOARDING_IMPORT_ROUTE } from '../navigation/routes';

/** Where the import of `source` opens: inside onboarding's connect step, or over the app. */
export function importRouteFor(source: string, onboarding: boolean) {
  return { pathname: onboarding ? ONBOARDING_IMPORT_ROUTE : IMPORT_ROUTE, params: { source } } as const;
}

/** A row's connect that did not go through. */
export interface RefusedConnect {
  card: string;
}

/** The line a refused row shows, its tap starting the connect again: it did not connect. */
export function refusedConnectLine(t: ApiErrorTranslate, platform: string): string {
  return t('providers.connectRefused', { provider: platform });
}
