// ABOUTME: Coaching-persona settings tab — the cards the server renders from the live contract registry
// ABOUTME: Persona is orthogonal to the chosen coach — it shapes how every coach speaks
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import type { CoachingPersona, PersonaCard } from '@pierre/shared-types';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { personasApi, userApi } from '../services/api';
import { Section } from './ui';
import { useAuth } from '../hooks/useAuth';
import { useTranslation } from '@pierre/i18n';

/**
 * The persona picker.
 *
 * Every word on a card is the server's. This tab used to hold four
 * hand-written options — a tagline, a blurb and up to two bullets each, in
 * five locales — describing contracts it could not see, while
 * `GET /api/personas` rendered the same cards from the live contract registry
 * and no client read it. When a contract changed, the cards went on saying
 * whatever they had said before; the enforcement badge, which says whether the
 * contract is actually enforced or only logged, had nowhere to appear at all.
 */
export default function CoachingPersonaTab() {
  const { t, language } = useTranslation();
  const { user } = useAuth();
  const queryClient = useQueryClient();
  const [selected, setSelected] = useState<CoachingPersona>('casual');
  const [message, setMessage] = useState<{ type: 'success' | 'error'; text: string } | null>(null);

  useEffect(() => {
    if (user?.coaching_persona) {
      setSelected(user.coaching_persona);
    }
  }, [user?.coaching_persona]);

  const { data, isLoading, isError } = useQuery({
    queryKey: QUERY_KEYS.personas.list(language),
    queryFn: () => personasApi.list(language),
  });
  const personas: PersonaCard[] = data?.personas ?? [];

  /** The card's own brand name, for a message about a persona. */
  const nameOf = (slug: string) =>
    personas.find((persona) => persona.slug === slug)?.display_name ?? slug;

  const mutation = useMutation({
    mutationFn: (persona: CoachingPersona) => userApi.setCoachingPersona(persona),
    onSuccess: (result) => {
      setMessage({
        type: 'success',
        text: t('app.coachingStyleUpdated', { style: nameOf(result.persona) }),
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
        text: t('app.coachingStyleUpdateFailed', { style: nameOf(attempted) }),
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
    <Section title={t('app.coachingStyleLower')} description={t('app.coachingStyleIntro')}>

      {isLoading && (
        <p className="text-sm text-on-surface-variant" data-testid="persona-loading">
          {t('common.loading')}
        </p>
      )}
      {isError && (
        <p className="text-sm text-error" role="alert" data-testid="persona-error">
          {t('common.error')}
        </p>
      )}

      <div
        role="radiogroup"
        aria-label={t('app.coachingStyleLower')}
        className="grid grid-cols-1 md:grid-cols-2 gap-3"
      >
        {personas.map((persona) => {
          const isSelected = selected === persona.slug;
          return (
            <button
              key={persona.slug}
              type="button"
              role="radio"
              aria-checked={isSelected}
              data-persona={persona.slug}
              data-enforcement={persona.enforcement}
              data-testid={`persona-card-${persona.slug}`}
              onClick={() => handleSelect(persona.slug as CoachingPersona)}
              disabled={mutation.isPending}
              className={`text-left p-4 rounded-xl border transition-colors duration-150 focus:outline-none focus:ring-2 focus:ring-primary ${
                isSelected
                  ? 'border-primary bg-primary/10 ring-1 ring-primary/40'
                  : 'border-outline-variant/60 bg-surface-container-low hover:border-outline-variant'
              } ${mutation.isPending ? 'opacity-70 cursor-wait' : ''}`}
            >
              <div className="flex items-center justify-between gap-3 mb-2">
                <h3 className="text-base font-semibold text-on-surface">{persona.display_name}</h3>
                {isSelected && (
                  <span className="text-xs font-medium text-primary">
                    {t('common.active')}
                  </span>
                )}
              </div>
              <p className="text-sm text-on-surface-variant mb-3 leading-relaxed">
                {persona.summary}
              </p>
              <ul className="space-y-1.5 mb-3">
                {persona.rules.map((rule) => (
                  <li
                    key={rule.key}
                    className="text-xs text-on-surface-variant/90 flex items-start gap-2"
                  >
                    <span className="text-primary/70 mt-0.5">›</span>
                    <span>{rule.text}</span>
                  </li>
                ))}
              </ul>
              {/* Whether the contract is enforced on every reply or only
                  logged — the one thing about a persona the athlete cannot
                  infer from how it reads. */}
              <span
                className={`inline-block rounded-full px-2 py-0.5 text-xs font-medium ${
                  persona.enforcement === 'verified'
                    ? 'bg-success/15 text-on-success-container'
                    : 'bg-surface-container-high text-on-surface-variant'
                }`}
              >
                {persona.enforcement_label}
              </span>
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
    </Section>
  );
}
