// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The privacy settings' consent-to-AI-use switches: one per connected provider whose card carries one, and both sensitive tiers
// ABOUTME: Withdrawing is one tap and stops AI use at once; the data stays visible and the connection stays live (carnet#726, J2)

import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import {
  AI_CONSENT_KEYS,
  SENSITIVE_CONSENT_KEYS,
  SENSITIVE_T2_CONSENT,
  SENSITIVE_T3_CONSENT,
  aiConsentCards,
} from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { oauthApi } from '../../services/api';
import { QUERY_KEYS } from '../../constants/queryKeys';
import { useAuth } from '../../hooks/useAuth';
import { useAiConsent, useAiConsentState } from '../../hooks/useAiConsent';
import { Switch } from '../ui';

/**
 * One switch per connected provider whose consent to AI use the athlete can
 * give or withdraw, shown while one is connected, and the Sensitive health
 * records switch, shown to every account: those records stay off limits to
 * the AI agent and the human coach until the athlete allows them here.
 */
export default function AiConsentSettings() {
  const { t } = useTranslation();
  const { isAuthenticated } = useAuth();
  const [message, setMessage] = useState<{ type: 'success' | 'error'; text: string } | null>(null);
  const { data } = useQuery({
    queryKey: QUERY_KEYS.user.providerConnections(),
    queryFn: () => oauthApi.getProvidersStatus(),
    enabled: isAuthenticated,
  });
  const consent = useAiConsent();
  const sensitive = useAiConsentState(SENSITIVE_T2_CONSENT, isAuthenticated);
  const intimate = useAiConsentState(SENSITIVE_T3_CONSENT, isAuthenticated);
  const cards = aiConsentCards(data?.providers ?? []);

  const toggle = (provider: string, allow: boolean, success: string) => {
    setMessage(null);
    consent.mutate(
      { provider, allow },
      {
        onSuccess: () => setMessage({ type: 'success', text: success }),
        onError: () => setMessage({ type: 'error', text: t(AI_CONSENT_KEYS.failed) }),
      },
    );
  };

  const classes = [
    { key: SENSITIVE_T2_CONSENT, label: t(SENSITIVE_CONSENT_KEYS.t2Label), state: sensitive.data },
    {
      key: SENSITIVE_T3_CONSENT,
      label: t(SENSITIVE_CONSENT_KEYS.t3Label),
      hint: t(SENSITIVE_CONSENT_KEYS.t3Hint),
      state: intimate.data,
    },
  ];
  return (
    <div className="border-b ghost-border-faint pb-5 space-y-5" data-testid="ai-consent-settings">
      {cards.length > 0 && (
        <div>
          <h3 className="text-sm font-medium text-on-surface mb-2">{t(AI_CONSENT_KEYS.title)}</h3>
          <p className="text-sm text-on-surface-variant leading-relaxed mb-4">{t(AI_CONSENT_KEYS.blurb)}</p>
          <ul className="space-y-3">
            {cards.map((card) => {
              const allowed = card.ai_consent === true;
              const label = t(AI_CONSENT_KEYS.label, { provider: card.display_name });
              return (
                <li key={card.provider} className="flex items-center justify-between gap-4">
                  <span className="text-sm text-on-surface">{label}</span>
                  <Switch
                    checked={allowed}
                    onChange={(next) =>
                      toggle(
                        card.provider,
                        next,
                        t(next ? AI_CONSENT_KEYS.allowed : AI_CONSENT_KEYS.withdrawn, {
                          provider: card.display_name,
                        }),
                      )
                    }
                    label={label}
                    disabled={consent.isPending}
                    data-testid={`ai-consent-switch-${card.provider}`}
                  />
                </li>
              );
            })}
          </ul>
        </div>
      )}
      <div data-testid="ai-consent-sensitive">
        <h3 className="text-sm font-medium text-on-surface mb-2">{t(SENSITIVE_CONSENT_KEYS.title)}</h3>
        <p className="text-sm text-on-surface-variant leading-relaxed mb-4">{t(SENSITIVE_CONSENT_KEYS.blurb)}</p>
        <ul className="space-y-3">
          {classes.map(({ key, label, hint, state }) => (
            <li key={key} className="flex items-center justify-between gap-4">
              <div>
                <span className="text-sm text-on-surface">{label}</span>
                {hint && (
                  <p className="text-xs text-on-surface-variant leading-relaxed mt-1" data-testid={`ai-consent-hint-${key}`}>
                    {hint}
                  </p>
                )}
              </div>
              <Switch
                checked={state === true}
                onChange={(next) =>
                  toggle(key, next, t(next ? SENSITIVE_CONSENT_KEYS.allowed : SENSITIVE_CONSENT_KEYS.withdrawn))
                }
                label={label}
                disabled={consent.isPending || state === undefined}
                data-testid={`ai-consent-switch-${key}`}
              />
            </li>
          ))}
        </ul>
      </div>
      {message && (
        <p
          role="status"
          className={`mt-3 text-sm ${message.type === 'success' ? 'text-on-surface-variant' : 'text-error'}`}
        >
          {message.text}
        </p>
      )}
    </div>
  );
}
