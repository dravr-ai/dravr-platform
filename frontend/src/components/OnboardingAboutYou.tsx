// ABOUTME: Onboarding step — three questions about the athlete, before the provider gate
// ABOUTME: Feeds build_coach_proposal, which already reads these and falls back to sport-mix without them

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { userApi } from '../services/api';
import { Button, Input, Textarea } from './ui';
import OnboardingShell from './OnboardingShell';
import { useTranslation } from '@pierre/i18n';
import { ONBOARDING_SPORTS as SPORTS, SPORT_LABEL_KEY } from '@pierre/shared-constants';


/**
 * Who the athlete is, in three questions.
 *
 * This runs *before* the provider gate on purpose. The coach proposal already
 * reads a North Star and covered pillars and explicitly falls back to sport-mix
 * when they're absent — and on a first-run connection there is no sport-mix
 * either, because the provider hasn't synced. Three answers we can always get
 * beat a data pull we don't control, and they're the only proposal input
 * guaranteed to exist when it runs.
 *
 * Every field is optional and the step is skippable: a partial answer is worth
 * strictly more than none, and a wall of required fields ahead of any value is
 * what people abandon.
 */
export default function OnboardingAboutYou({
  userDisplayName,
  onComplete,
}: {
  userDisplayName?: string | null;
  onComplete: (status?: 'complete' | 'skipped') => void;
}) {
  const { t } = useTranslation();
  const [sport, setSport] = useState('');
  const [goal, setGoal] = useState('');
  const [northStar, setNorthStar] = useState('');
  const [saving, setSaving] = useState(false);

  const hasAnything = Boolean(sport.trim() || goal.trim() || northStar.trim());

  const handleContinue = async () => {
    if (saving) return;
    if (!hasAnything) {
      onComplete('skipped');
      return;
    }
    setSaving(true);
    try {
      await userApi.saveAboutYou({
        primary_sport: sport.trim() || undefined,
        goal: goal.trim() || undefined,
        north_star: northStar.trim() || undefined,
      });
    } catch {
      // Non-fatal, deliberately: the answers are a head start, not a gate. A
      // failed write must not trap someone on the step — the pillar walk asks
      // the same things conversationally later.
    }
    onComplete('complete');
  };

  return (
    <OnboardingShell
      heading={userDisplayName ? t('app.obTellMeGreeting', { name: userDisplayName }) : t('onboarding.tellMeAboutTraining')}
    >
      <p className="mt-3 text-sm text-on-surface-variant font-label text-center">
        {t('onboarding.aboutYouHint')}
      </p>

      <div className="mt-8 space-y-6">
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
                  className={`rounded-full border px-4 py-1.5 text-sm transition-colors ${
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

      <div className="mt-8 space-y-3">
        <Button
          variant="primary"
          onClick={() => void handleContinue()}
          disabled={saving}
          className="w-full"
        >
          {saving ? t('onboarding.saving') : t('onboarding.continueButton')}
        </Button>
        <button
          type="button"
          onClick={() => onComplete('skipped')}
          disabled={saving}
          className="w-full text-sm font-medium text-on-surface-variant hover:text-on-surface underline-offset-2 hover:underline transition-colors"
        >
          {t('onboarding.skipForNow')}
        </button>
      </div>
    </OnboardingShell>
  );
}
