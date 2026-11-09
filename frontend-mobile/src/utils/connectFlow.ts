// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: What a provider connect on the phone says and opens next — an export's route, a refused connect's line on its row
// ABOUTME: Pure helpers shared by Connections, onboarding's connect step and the sign-in window; the card logic is @pierre/ui-logic's

import { SIGN_IN_OAUTH_REQUIRED } from '@pierre/shared-constants';
import type { ApiErrorTranslate } from '@pierre/ui-logic';
import { IMPORT_ROUTE, ONBOARDING_IMPORT_ROUTE } from '../navigation/routes';

/** Where the import of `source` opens: inside onboarding's connect step, or over the app. */
export function importRouteFor(source: string, onboarding: boolean) {
  return { pathname: onboarding ? ONBOARDING_IMPORT_ROUTE : IMPORT_ROUTE, params: { source } } as const;
}

/**
 * The sentence a sign-in refused with `oauth_required` says: the provider now
 * connects through its official authorization. The key is server-rendered,
 * so its positional `{0}` is filled here with the platform's name, as the
 * web's sign-in window does.
 */
export function signInOauthRequiredLine(t: ApiErrorTranslate, platform: string): string {
  return t('hosted.common.signInOauthRequired').split('{0}').join(platform);
}

/** A row's connect that did not go through, and why when the server named it (`details.reason`). */
export interface RefusedConnect {
  card: string;
  reason?: string;
}

/**
 * The line a refused row shows, its tap starting the connect again: a sign-in
 * refused because the provider now connects through its authorization says
 * so; any other refusal says it did not connect.
 */
export function refusedConnectLine(t: ApiErrorTranslate, refusal: RefusedConnect, platform: string): string {
  return refusal.reason === SIGN_IN_OAUTH_REQUIRED
    ? signInOauthRequiredLine(t, platform)
    : t('providers.connectRefused', { provider: platform });
}
