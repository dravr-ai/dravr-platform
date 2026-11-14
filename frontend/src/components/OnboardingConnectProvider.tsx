// ABOUTME: Onboarding's Tes données — connect your training: Garmin, Strava, Intervals.icu, the rest behind « Plus de services »
// ABOUTME: « Continuer sans connecter » is saved on the server, so the screen never comes back at sign-in; sign out stays quiet

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { ONBOARDING_LEAD_PROVIDERS } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { useAuth } from '../hooks/useAuth';
import { Button } from './ui';
import ProviderConnectionCards from './ProviderConnectionCards';
import OnboardingShell from './OnboardingShell';

/**
 * Tes données, the athlete's second screen (carnet#1530).
 *
 * Shown while the user has never had a real provider connected. The same
 * source of truth — `provider_connections` — drives the backend's
 * `NoProviderConnected` 403 on chat/coach/messaging, so this screen cannot
 * drift from server-side enforcement. It leads with the three services most
 * athletes use and folds the rest, and the file import, behind « Plus de
 * services ». Connecting a first provider does not leave the screen: the list
 * stays up with the new card marked connected, so a second service is one tap
 * away, and `onContinue` (offered once a provider is connected) is the one
 * filled button.
 *
 * Before that, « Continuer sans connecter » is a visible secondary: the agent
 * reads nothing without a service, which the intro says, but nobody is held
 * here. The host saves the skip on the server, so the screen never comes back
 * at sign-in; Home's "Connect your data" row is the nudge from then on. Sign
 * out sits quiet at the bottom, a session escape and never the loudest thing.
 *
 * A coach who does not train never reaches it: the coaching platforms are
 * offered on the coach-seat step (`OnboardingCoachSeat`).
 */
export default function OnboardingConnectProvider({
  onContinueWithoutProvider,
  onContinue,
}: {
  onContinueWithoutProvider?: () => void;
  /** Leave the step; defined once at least one provider is connected. */
  onContinue?: () => void;
}) {
  const { t } = useTranslation();
  const { logout } = useAuth();
  return (
    <OnboardingShell heading={t('onboarding.connect.heading')}>
      <div className="flex flex-col">
        <p className="mt-3 text-sm text-on-surface-variant text-center mx-auto max-w-md">
          {t('onboarding.connect.intro')}
        </p>

        <div className="mt-8">
          <ProviderConnectionCards leadProviders={ONBOARDING_LEAD_PROVIDERS} />
        </div>

        <div className="mx-auto mt-8 flex w-full max-w-md flex-col items-stretch gap-3">
          {onContinue ? (
            <div className="flex flex-col items-stretch gap-3" data-testid="onboarding-connect-continue">
              <p className="text-sm text-on-surface-variant text-center">
                {t('onboarding.connectAnotherOrContinue')}
              </p>
              <Button variant="primary" onClick={onContinue} className="w-full">
                {t('onboarding.continueAfterConnect')}
              </Button>
            </div>
          ) : onContinueWithoutProvider ? (
            <Button
              variant="secondary"
              onClick={onContinueWithoutProvider}
              className="w-full"
              data-testid="onboarding-connect-skip"
            >
              {t('onboarding.connect.continueWithout')}
            </Button>
          ) : null}
        </div>

        <p className="mt-6 text-xs text-on-surface-variant text-center">
          {t('onboarding.credentialsEncryptedHint')}
        </p>

        <div className="mt-6 flex justify-center">
          <button
            type="button"
            onClick={logout}
            className="touch-target rounded text-xs text-on-surface-variant underline-offset-2 hover:text-on-surface hover:underline focus-ring"
          >
            {t('common.logout')}
          </button>
        </div>
      </div>
    </OnboardingShell>
  );
}
