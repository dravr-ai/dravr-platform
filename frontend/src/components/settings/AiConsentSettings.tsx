// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The privacy settings' consent-to-AI-use switches, one per connected provider whose card carries one
// ABOUTME: Withdrawing is one tap and stops AI use at once; the data stays visible and the connection stays live (carnet#726)

import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { AI_CONSENT_KEYS, aiConsentCards } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { oauthApi } from '../../services/api';
import { QUERY_KEYS } from '../../constants/queryKeys';
import { useAuth } from '../../hooks/useAuth';
import { useAiConsent } from '../../hooks/useAiConsent';
import { Switch } from '../ui';

/**
 * One switch per connected provider whose consent to AI use the athlete can
 * give or withdraw. Renders nothing while no such provider is connected.
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
  const cards = aiConsentCards(data?.providers ?? []);

  if (cards.length === 0) {
    return null;
  }

  const toggle = (provider: string, displayName: string, allow: boolean) => {
    setMessage(null);
    consent.mutate(
      { provider, allow },
      {
        onSuccess: () =>
          setMessage({
            type: 'success',
            text: t(allow ? AI_CONSENT_KEYS.allowed : AI_CONSENT_KEYS.withdrawn, { provider: displayName }),
          }),
        onError: () => setMessage({ type: 'error', text: t(AI_CONSENT_KEYS.failed) }),
      },
    );
  };

  return (
    <div className="border-b ghost-border-faint pb-5" data-testid="ai-consent-settings">
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
                onChange={(next) => toggle(card.provider, card.display_name, next)}
                label={label}
                disabled={consent.isPending}
                data-testid={`ai-consent-switch-${card.provider}`}
              />
            </li>
          );
        })}
      </ul>
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
