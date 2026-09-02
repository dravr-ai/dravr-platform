// ABOUTME: Coaching-persona settings tab — pick output format / cadence preference
// ABOUTME: Persona is orthogonal to the chosen coach — it shapes how every coach speaks
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect, useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import type { CoachingPersona } from '@pierre/shared-types';
import { userApi } from '../services/api';
import { Card } from './ui';
import { useAuth } from '../hooks/useAuth';
import { useTranslation } from '@pierre/i18n';
import { PERSONA_NAME } from '@pierre/shared-constants';

interface PersonaOption {
  id: CoachingPersona;
  /** The persona's own label. Stored on the account and quoted back inside the
   *  coach's system prompt, so it is deliberately not translated. */
  name: string;
  taglineKey: string;
  descriptionKey: string;
  bulletKeys: string[];
}

const PERSONA_OPTIONS: PersonaOption[] = [
  {
    id: 'casual',
    name: PERSONA_NAME.casual,
    taglineKey: 'app.styleCasualTag',
    descriptionKey: 'app.styleCasualBlurb',
    bulletKeys: [
      'app.styleCasualBullet1',
      'app.styleCasualBullet2',
    ],
  },
  {
    id: 'enthusiast',
    name: PERSONA_NAME.enthusiast,
    taglineKey: 'app.styleEnthusiastTag',
    descriptionKey: 'app.styleEnthusiastBlurb',
    bulletKeys: [
      'app.styleEnthusiastBullet1',
      'app.styleEnthusiastBullet2',
    ],
  },
  {
    id: 'power_athlete',
    name: PERSONA_NAME.power_athlete,
    taglineKey: 'app.stylePowerTagWeb',
    descriptionKey: 'app.stylePowerBlurb',
    bulletKeys: [
      'app.stylePowerBullet1',
      'app.stylePowerBullet2Web',
    ],
  },
  {
    id: 'coach',
    name: PERSONA_NAME.coach,
    taglineKey: 'app.styleCoachTagWeb',
    descriptionKey: 'app.styleCoachBlurbWeb',
    bulletKeys: [],
  },
];

export default function CoachingPersonaTab() {
  const { t } = useTranslation();
  const { user } = useAuth();
  const queryClient = useQueryClient();
  const [selected, setSelected] = useState<CoachingPersona>('casual');
  const [message, setMessage] = useState<{ type: 'success' | 'error'; text: string } | null>(null);

  useEffect(() => {
    if (user?.coaching_persona) {
      setSelected(user.coaching_persona);
    }
  }, [user?.coaching_persona]);

  const mutation = useMutation({
    mutationFn: (persona: CoachingPersona) => userApi.setCoachingPersona(persona),
    onSuccess: (data) => {
      setMessage({
        type: 'success',
        text: t('app.coachingStyleUpdated', { style: PERSONA_NAME[data.persona] ?? data.persona }),
      });
      void queryClient.invalidateQueries({ queryKey: ['user'] });
      setTimeout(() => setMessage(null), 3000);
    },
    onError: (_, attempted) => {
      // Roll back the optimistic selection so the UI reflects backend state.
      if (user?.coaching_persona) {
        setSelected(user.coaching_persona);
      }
      setMessage({
        type: 'error',
        text: t('app.coachingStyleUpdateFailed', { style: PERSONA_NAME[attempted] ?? attempted }),
      });
      setTimeout(() => setMessage(null), 3000);
    },
  });

  const handleSelect = (persona: CoachingPersona) => {
    if (persona === selected || mutation.isPending) {
      return;
    }
    setSelected(persona);
    mutation.mutate(persona);
  };

  return (
    <Card variant="dark">
      <h2 className="text-lg font-semibold text-on-surface mb-2">{t('app.coachingStyleLower')}</h2>
      <p className="text-sm text-on-surface-variant mb-6">{t('app.coachingStyleIntro')}</p>

      <div
        role="radiogroup"
        aria-label={t('app.coachingStyleLower')}
        className="grid grid-cols-1 md:grid-cols-2 gap-3"
      >
        {PERSONA_OPTIONS.map((option) => {
          const isSelected = selected === option.id;
          return (
            <button
              key={option.id}
              type="button"
              role="radio"
              aria-checked={isSelected}
              data-persona={option.id}
              data-testid={`persona-card-${option.id}`}
              onClick={() => handleSelect(option.id)}
              disabled={mutation.isPending}
              className={`text-left p-4 rounded-xl border transition-colors duration-150 focus:outline-none focus:ring-2 focus:ring-primary ${
                isSelected
                  ? 'border-primary bg-primary/10 ring-1 ring-primary/40'
                  : 'border-outline-variant/60 bg-surface-container-low hover:border-outline-variant'
              } ${mutation.isPending ? 'opacity-70 cursor-wait' : ''}`}
            >
              <div className="flex items-center justify-between gap-3 mb-2">
                <h3 className="text-base font-semibold text-on-surface">{option.name}</h3>
                {isSelected && (
                  <span className="text-xs font-medium text-primary uppercase tracking-wide">
                    {t('common.active')}
                  </span>
                )}
              </div>
              <p className="text-sm text-primary/90 mb-2">{t(option.taglineKey)}</p>
              <p className="text-sm text-on-surface-variant mb-3 leading-relaxed">
                {t(option.descriptionKey)}
              </p>
              <ul className="space-y-1.5">
                {option.bulletKeys.map((bulletKey) => (
                  <li
                    key={bulletKey}
                    className="text-xs text-on-surface-variant/90 flex items-start gap-2"
                  >
                    <span className="text-primary/70 mt-0.5">›</span>
                    <span>{t(bulletKey)}</span>
                  </li>
                ))}
              </ul>
            </button>
          );
        })}
      </div>

      {message && (
        <div
          role="status"
          data-testid="persona-status"
          className={`mt-4 p-3 rounded-lg text-sm ${
            message.type === 'success'
              ? 'bg-success/10 text-on-success-container border border-success/20'
              : 'bg-error/10 text-error border border-error/20'
          }`}
        >
          {message.text}
        </div>
      )}
    </Card>
  );
}
