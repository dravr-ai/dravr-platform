// ABOUTME: Home's « Ton objectif et ton pourquoi » sheet body — the sport, the goal and the why, once onboarding's About you
// ABOUTME: Saved through the same about-you endpoint; the host ticks the row once the answers are stored

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useTranslation } from '@pierre/i18n';
import { ONBOARDING_SPORTS as SPORTS, SPORT_LABEL_KEY } from '@pierre/shared-constants';
import { Button, Input, Textarea } from '../ui';

/** What the sheet hands its host to store: only the fields the athlete filled. */
export interface GoalWhyAnswers {
  primary_sport?: string;
  goal?: string;
  north_star?: string;
}

/**
 * Who the athlete is, in three questions — onboarding's About you, moved to
 * a « Pour bien démarrer » row on Home (carnet#1530) so the wizard reaches
 * the team in four screens. Every field is optional, as it was: a partial
 * answer is worth strictly more than none. The why keeps its hint, word for
 * word: it is what the agent comes back to when training gets hard.
 *
 * Save waits for the host's write; a refusal keeps the answers on screen with
 * an error. Closing the sheet saves nothing and ticks nothing.
 */
export function GoalWhyForm({ onSave }: { onSave: (answers: GoalWhyAnswers) => Promise<void> }) {
  const { t } = useTranslation();
  const [sport, setSport] = useState('');
  const [goal, setGoal] = useState('');
  const [northStar, setNorthStar] = useState('');
  const [saving, setSaving] = useState(false);
  const [failed, setFailed] = useState(false);

  const hasAnything = Boolean(sport.trim() || goal.trim() || northStar.trim());

  const handleSave = async () => {
    if (saving || !hasAnything) return;
    setSaving(true);
    setFailed(false);
    try {
      await onSave({
        primary_sport: sport.trim() || undefined,
        goal: goal.trim() || undefined,
        north_star: northStar.trim() || undefined,
      });
    } catch {
      setFailed(true);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div>
      <p className="text-sm text-on-surface-variant">{t('onboarding.aboutYouHint')}</p>

      <div className="mt-6 space-y-6">
        <div>
          <span className="block text-sm font-medium text-on-surface">{t('onboarding.primarySportLabel')}</span>
          <div className="mt-3 flex flex-wrap gap-2">
            {SPORTS.map((option) => {
              const selected = sport === option;
              return (
                <button
                  key={option}
                  type="button"
                  aria-pressed={selected}
                  onClick={() => setSport(selected ? '' : option)}
                  className={`touch-target rounded-full border px-4 py-1.5 text-sm transition-colors focus-ring ${
                    selected
                      ? 'border-primary bg-primary text-on-primary'
                      : 'border-outline-variant bg-surface-container-low text-on-surface hover:border-primary'
                  }`}
                >
                  {t(SPORT_LABEL_KEY[option])}
                </button>
              );
            })}
          </div>
          <div className="mt-3">
            <Input
              id="about-sport-other"
              type="text"
              label={t('onboarding.otherSportLabel')}
              placeholder={t('onboarding.otherSportPlaceholder')}
              value={SPORTS.includes(sport as (typeof SPORTS)[number]) ? '' : sport}
              onChange={(e) => setSport(e.target.value)}
            />
          </div>
        </div>

        <div>
          <Input
            id="about-goal"
            type="text"
            label={t('onboarding.goalLabel')}
            placeholder={t('onboarding.goalPlaceholder')}
            value={goal}
            onChange={(e) => setGoal(e.target.value)}
          />
        </div>

        <div>
          <Textarea
            id="about-north-star"
            rows={3}
            label={t('onboarding.northStarLabel')}
            placeholder={t('onboarding.northStarPlaceholder')}
            helpText={t('app.obWhyHint')}
            value={northStar}
            onChange={(e) => setNorthStar(e.target.value)}
          />
        </div>
      </div>

      {failed ? (
        <p role="alert" className="mt-4 text-sm text-error">
          {t('home.firstSteps.saveFailed')}
        </p>
      ) : null}

      <div className="mt-8">
        <Button
          variant="primary"
          onClick={() => void handleSave()}
          disabled={saving || !hasAnything}
          className="w-full"
          data-testid="goal-why-save"
        >
          {saving ? t('onboarding.saving') : t('common.save')}
        </Button>
      </div>
    </div>
  );
}
