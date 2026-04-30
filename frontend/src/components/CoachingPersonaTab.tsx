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

interface PersonaOption {
  id: CoachingPersona;
  name: string;
  tagline: string;
  description: string;
  bullets: string[];
}

const PERSONA_OPTIONS: PersonaOption[] = [
  {
    id: 'casual',
    name: 'Casual',
    tagline: 'Friendly, encouraging, no jargon',
    description:
      "Talks to you like a knowledgeable friend texting a quick reply. Round numbers, plain language, short replies.",
    bullets: [
      'Prose, not bullet lists. No structured blocks.',
      'No framework citations or technical acronyms.',
      'Only urgent (P0) push notifications — weekly digest otherwise.',
    ],
  },
  {
    id: 'enthusiast',
    name: 'Enthusiast',
    tagline: 'Prose plus the numbers that matter',
    description:
      "Mostly prose, with a small data block when it actually changes the recommendation. Citations only when you ask why.",
    bullets: [
      'Headline answer first, key datum second. Replies under ~250 words.',
      'Acronyms (CTL, ACWR) glossed on first use, then used freely.',
      'P0 + P1 unsolicited push (acute injury risk, near-overreached).',
    ],
  },
  {
    id: 'power_athlete',
    name: 'Power-athlete',
    tagline: 'Section 11 discipline — line-by-line, framework-cited',
    description:
      "Deterministic, auditable output. Per-activity reports with exact numbers. Framework citations on every numeric claim.",
    bullets: [
      'Line-by-line activity blocks with framework labels (Coggan, Banister, Foster).',
      'Full P0–P3 readiness ladder verbatim. 10-point validation checklist.',
      'Full P0/P1/P2 unsolicited push. Pre-workout briefing on request.',
    ],
  },
  {
    id: 'coach',
    name: 'Coach',
    tagline: 'Inherits Power-athlete + roster framing for managing other athletes',
    description:
      "Same Power-athlete discipline, plus per-athlete tenant-scoped reports and roster-wide summaries.",
    bullets: [
      'Athlete-scoped queries — never cross-aggregate the coach and their athletes.',
      'Roster-wide blocks (e.g. ACWR > 1.3 across the squad).',
      'Full P0/P1/P2 push for every athlete; P3 only for the coach themselves.',
    ],
  },
];

export default function CoachingPersonaTab() {
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
      setMessage({ type: 'success', text: `Coaching style updated to ${data.persona}.` });
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
        text: `Failed to update coaching style to ${attempted}.`,
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
      <h2 className="text-lg font-semibold text-on-surface mb-2">Coaching style</h2>
      <p className="text-sm text-on-surface-variant mb-6">
        Choose how detailed and structured you want every coach&rsquo;s replies to be. This is
        independent of which coach you talk to — the same coach speaks differently to a Casual user
        than to a Power-athlete.
      </p>

      <div
        role="radiogroup"
        aria-label="Coaching style"
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
              className={`text-left p-4 rounded-xl border transition-colors duration-150 focus:outline-none focus:ring-2 focus:ring-pierre-violet ${
                isSelected
                  ? 'border-pierre-violet bg-pierre-violet/10 ring-1 ring-pierre-violet/40'
                  : 'border-zinc-700/60 bg-surface-container-low hover:border-zinc-500'
              } ${mutation.isPending ? 'opacity-70 cursor-wait' : ''}`}
            >
              <div className="flex items-center justify-between gap-3 mb-2">
                <h3 className="text-base font-semibold text-on-surface">{option.name}</h3>
                {isSelected && (
                  <span className="text-xs font-medium text-pierre-violet uppercase tracking-wide">
                    Active
                  </span>
                )}
              </div>
              <p className="text-sm text-pierre-violet/90 mb-2">{option.tagline}</p>
              <p className="text-sm text-on-surface-variant mb-3 leading-relaxed">
                {option.description}
              </p>
              <ul className="space-y-1.5">
                {option.bullets.map((bullet) => (
                  <li
                    key={bullet}
                    className="text-xs text-on-surface-variant/90 flex items-start gap-2"
                  >
                    <span className="text-pierre-violet/70 mt-0.5">›</span>
                    <span>{bullet}</span>
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
              ? 'bg-emerald-500/10 text-emerald-400 border border-emerald-500/20'
              : 'bg-red-500/10 text-red-400 border border-red-500/20'
          }`}
        >
          {message.text}
        </div>
      )}
    </Card>
  );
}
