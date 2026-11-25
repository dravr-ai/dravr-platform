// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Connected apps' "Always allowed" group — each app the athlete lets renew a preset grant without the consent screen, with Stop
// ABOUTME: Stopping a policy leaves the grants it renewed; the app asks again once its access ends (carnet#1544)

import { useState } from 'react';
import { useTranslation } from '@pierre/i18n';
import { describeApiError } from '@pierre/ui-logic';
import { formatDate } from '@pierre/chat-utils';
import type { OAuthStandingPolicy } from '@pierre/shared-types';
import { Alert, Button, ConfirmDialog, EmptyState, Section, useErrorToast } from '../ui';
import { useRemoveStandingPolicy, useStandingPolicies } from '../../hooks/useGrantHistory';
import { connectedAppPresetLabel } from './connectedAppPreset';

/** The app's registered name, else the id it was allowed under. */
function policyAppName(policy: OAuthStandingPolicy): string {
  return policy.client_name || policy.client_id;
}

/** The apps the athlete always allows, each with a Stop that asks first. */
export default function StandingPoliciesSection() {
  const { t, language } = useTranslation();
  const { data: policies, isLoading, error } = useStandingPolicies();
  const remove = useRemoveStandingPolicy();
  const showErrorToast = useErrorToast();
  const [toStop, setToStop] = useState<OAuthStandingPolicy | null>(null);

  return (
    <Section
      title={t('tokens.standingPolicies')}
      description={t('tokens.standingPoliciesHint')}
      data-testid="standing-policies"
    >
      {isLoading ? (
        <div className="flex justify-center py-6">
          <div className="pierre-spinner w-6 h-6"></div>
        </div>
      ) : error ? (
        <Alert tone="error" className="border">
          {describeApiError(error, { t, fallbackKey: 'tokens.standingPoliciesLoadFailed' })}
        </Alert>
      ) : policies && policies.length > 0 ? (
        <div>
          {policies.map((policy) => (
            <div
              key={policy.id}
              className="flex items-start justify-between gap-3 border-t ghost-border-faint py-3 first:border-t-0"
              data-testid={`standing-policy-${policy.id}`}
            >
              <div className="min-w-0">
                <p className="font-medium text-on-surface break-all">{policyAppName(policy)}</p>
                {policy.client_host && (
                  <p className="text-sm text-on-surface-variant break-all">{policy.client_host}</p>
                )}
                <p className="text-sm text-on-surface-variant break-words">
                  {connectedAppPresetLabel(policy.preset, t) ?? policy.preset}
                </p>
                <p className="text-xs text-outline mt-1">
                  {t('tokens.standingPolicySince', { date: formatDate(policy.created_at, language) })}
                </p>
              </div>
              <Button
                variant="danger-outline"
                size="sm"
                onClick={() => setToStop(policy)}
                disabled={remove.isPending}
                aria-label={t('tokens.standingPolicyStopLabel', { app: policyAppName(policy) })}
                className="flex-shrink-0"
                data-testid={`stop-standing-policy-${policy.id}`}
              >
                {t('tokens.standingPolicyStop')}
              </Button>
            </div>
          ))}
        </div>
      ) : (
        <EmptyState>{t('tokens.standingPoliciesEmpty')}</EmptyState>
      )}
      <ConfirmDialog
        isOpen={toStop !== null}
        onClose={() => setToStop(null)}
        onConfirm={() =>
          toStop &&
          remove.mutate(toStop.id, {
            onSuccess: () => setToStop(null),
            onError: (err: unknown) => {
              setToStop(null);
              showErrorToast(
                t('tokens.standingPolicyStopFailed'),
                describeApiError(err, { t, fallbackKey: 'errors.unknown' }),
              );
            },
          })
        }
        title={t('tokens.standingPolicyStopQ')}
        message={t('tokens.standingPolicyStopConfirm', { app: toStop ? policyAppName(toStop) : '' })}
        confirmLabel={t('tokens.standingPolicyStop')}
        variant="danger"
        isLoading={remove.isPending}
      />
    </Section>
  );
}
