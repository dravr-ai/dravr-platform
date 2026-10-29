// ABOUTME: First-run onboarding screen that forces a provider connection before the user reaches the dashboard
// ABOUTME: Sits in the shared OnboardingShell like every other step, the welcome as its heading

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useAuth } from '../hooks/useAuth';
import { Button } from './ui';
import ProviderConnectionCards from './ProviderConnectionCards';
import ConnectPreview from './ConnectPreview';

// Dravr boreal-palette logo. Same dot-and-line motif as Login and
// PendingApproval; gradient IDs are suffixed with `-onboarding` so the
// embedded `<defs>` don't collide when multiple instances of the logo render
// in the same document tree.
import OnboardingShell from './OnboardingShell';
import { useTranslation } from '@pierre/i18n';
import { COACH_PLATFORM_PROVIDERS } from '../onboarding/steps';

/**
 * Hard gate shown right after first login when the user has zero connected
 * providers. The same source of truth — `provider_connections` — drives the
 * backend's `NoProviderConnected` 403 on chat/coach/messaging, so this screen
 * cannot drift from server-side enforcement. Connecting a first provider
 * does not leave the screen: the list stays up with the new card marked
 * connected, so a second service is one tap away, and `onContinue` (offered
 * once a provider is connected) moves on to the next step.
 *
 * Skip is intentionally absent from the provider list — the agent has
 * nothing to reason about without provider data. Sign Out is offered as a
 * session escape (same convention as `PendingApproval`) so the user is never
 * trapped.
 *
 * A coach who does not train (`coachOnly`) is offered only the coaching
 * platforms, with no preview of athlete coaching: nothing reads their own
 * training, and skipping moves them on to their group.
 */
export default function OnboardingConnectProvider({
  userDisplayName,
  onContinueWithoutProvider,
  onContinue,
  coachOnly = false,
}: {
  userDisplayName?: string | null;
  onContinueWithoutProvider?: () => void;
  /** Leave the step; defined once at least one provider is connected. */
  onContinue?: () => void;
  coachOnly?: boolean;
}) {
  const { t } = useTranslation();
  const { logout } = useAuth();
  // OnboardingShell's `pt-24` reserves room for the OnboardingProgress bar so
  // this taller provider-list card clears it; `my-auto` still centers it when
  // the viewport has room.
  return (
    <>
      <OnboardingShell
        heading={userDisplayName ? t('onboarding.welcomeNamed', { name: userDisplayName }) : t('onboarding.welcomeToDravr')}
      >
        <div className="flex flex-col">
          <p className="mt-3 text-sm text-on-surface-variant text-center mx-auto max-w-md">
            {coachOnly ? t('onboarding.connectCoachPlatformIntro') : t('onboarding.connectProviderIntro')}
          </p>

            <div className="mt-8">
              <ProviderConnectionCards
                onlyProviders={coachOnly ? COACH_PLATFORM_PROVIDERS : undefined}
              />
            </div>

            {onContinue && (
              <div className="mt-6 flex flex-col items-center gap-3" data-testid="onboarding-connect-continue">
                <p className="text-sm text-on-surface-variant text-center">
                  {t('onboarding.connectAnotherOrContinue')}
                </p>
                <Button variant="primary" onClick={onContinue}>
                  {t('onboarding.continueAfterConnect')}
                </Button>
              </div>
            )}

            {!coachOnly && <ConnectPreview />}

            <p className="mt-6 text-xs text-on-surface-variant text-center">
              {t('onboarding.credentialsEncryptedHint')}
            </p>

            {onContinueWithoutProvider && !onContinue && (
              <div className="mt-6 text-center">
                <button
                  type="button"
                  onClick={onContinueWithoutProvider}
                  className="text-sm font-medium text-on-surface-variant hover:text-on-surface underline-offset-2 hover:underline transition-colors"
                >
                  {t('onboarding.continueWithoutProvider')}
                </button>
                <p className="mt-1 text-xs text-on-surface-variant">
                  {coachOnly ? t('onboarding.connectCoachPlatformLaterHint') : t('onboarding.connectAnytimeHint')}
                </p>
              </div>
            )}

            <div className="mt-8 flex justify-center">
              <Button variant="tertiary" onClick={logout}>
                {t('common.logout')}
              </Button>
            </div>
        </div>
      </OnboardingShell>
    </>
  );
}
