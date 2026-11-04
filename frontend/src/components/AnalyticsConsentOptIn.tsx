// ABOUTME: The sign-up forms' usage-analytics opt-in: its own unticked box, a line on what it records, the policy link
// ABOUTME: Shared by the register form and the invitation form, so both ask the same question the same way (carnet#1366)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { ANALYTICS_POLICY_URL } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { Checkbox } from './ui';

/**
 * The usage-analytics opt-in on a form that creates an account.
 *
 * Law 25 s. 14 and the CAI's guideline 2023-1 want the consent manifest, free
 * and specific: so it is a box of its own, starting unticked, bundled with
 * nothing (there is no terms box to fold it into), and the form submits the
 * same whether it is ticked or not.
 */
export default function AnalyticsConsentOptIn({
  checked,
  onChange,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="space-y-1.5">
      <Checkbox
        id="analyticsConsent"
        name="analyticsConsent"
        data-testid="analytics-consent-opt-in"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
        label={t('auth.analyticsConsentLabel')}
        description={t('auth.analyticsConsentHint')}
      />
      <a
        href={ANALYTICS_POLICY_URL}
        target="_blank"
        rel="noopener noreferrer"
        className="ml-7 inline-block text-xs text-primary hover:text-primary-fixed-dim font-medium underline-offset-2 hover:underline"
      >
        {t('auth.analyticsConsentPolicyLink')}
      </a>
    </div>
  );
}
