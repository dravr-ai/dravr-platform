// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach's morning brief in À traiter — a quiet status line under the queue's title, its settings sheet, and the athlete pane's leave-out action (carnet#1791)
// ABOUTME: Nothing draws until the brief is read, and nothing at all for a caller who coaches no one; every change saves at once and says so when refused

import { useMemo, useState } from 'react';
import { clsx } from 'clsx';
import { SlidersHorizontal, Sunrise } from 'lucide-react';
import type { CoachBrief, CoachBriefUpdate } from '@pierre/shared-types';
import { COACH_BRIEF_REASON_LABEL_KEY } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import {
  BRIEF_HOURS,
  BRIEF_REASON_KINDS,
  briefHourLabel,
  briefStatusLine,
  briefStatusWords,
  isBriefMuted,
  mutedAthletes,
  withMuted,
  withReason,
} from '@pierre/ui-logic';
import { useIsMobile } from '../../hooks/useBreakpoint';
import { useCoachBrief, useUpdateCoachBrief } from '../../hooks/useCoachBrief';
import { useCoachRoster } from '../../hooks/useCoachRoster';
import { Button, Section, Select, Switch, useErrorToast } from '../ui';
import { Sheet } from '../ui/Sheet';
import { AthleteAvatar } from './parts';

/** The hour picker's choices, 05:00 to 23:00. */
const HOUR_OPTIONS = BRIEF_HOURS.map((hour) => ({ value: String(hour), label: briefHourLabel(hour) }));

/**
 * The brief's line under À traiter's title: its name, then where it stands
 * and when it runs, at the caption size, so it reads as a note on the queue
 * and never as a row of it. The whole line opens the settings.
 */
export function CoachBriefControl({ className }: { className?: string }) {
  const { t, language } = useTranslation();
  const { brief } = useCoachBrief(true);
  const [open, setOpen] = useState(false);
  if (brief === undefined) return null;
  const line = briefStatusLine(briefStatusWords(t, language, brief));
  return (
    <>
      <button
        type="button"
        aria-haspopup="dialog"
        title={t('coachHome.brief.settings')}
        data-testid="coach-brief"
        data-state={brief.state}
        onClick={() => setOpen(true)}
        className={clsx(
          'flex w-full items-center gap-2.5 rounded-[10px] px-2 py-1.5 text-left transition-colors hover:bg-surface-container-low/60 focus-ring-visible',
          className,
        )}
      >
        <Sunrise className="h-4 w-4 shrink-0 text-on-surface-variant" aria-hidden="true" />
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="text-xs font-medium text-on-surface">{t('coachHome.brief.title')}</span>
          <span className="text-xs text-on-surface-variant" data-testid="coach-brief-status">
            {line}
          </span>
        </span>
        <SlidersHorizontal className="h-4 w-4 shrink-0 text-on-surface-variant" aria-hidden="true" />
      </button>
      {open && <CoachBriefSheet brief={brief} onClose={() => setOpen(false)} />}
    </>
  );
}

/**
 * The brief's settings: pause or resume, its hour, the reasons it checks and
 * the athletes it leaves out. Each change saves on its own, shown at once; a
 * refused one is rolled back and the sheet says so until the next change.
 */
function CoachBriefSheet({ brief, onClose }: { brief: CoachBrief; onClose: () => void }) {
  const { t, language } = useTranslation();
  const isMobile = useIsMobile();
  const { roster } = useCoachRoster(true);
  const { update } = useUpdateCoachBrief();
  const [failed, setFailed] = useState(false);
  const paused = brief.state === 'paused';
  const leftOut = useMemo(() => mutedAthletes(brief.muted, roster?.athletes ?? []), [brief.muted, roster]);

  const save = (change: CoachBriefUpdate) => {
    setFailed(false);
    update(change).catch(() => setFailed(true));
  };

  return (
    <Sheet
      side={isMobile ? 'bottom' : 'right'}
      title={t('coachHome.brief.settings')}
      onClose={onClose}
      data-testid="coach-brief-sheet"
    >
      <div className="flex flex-col gap-8 px-4 pb-8 pt-1">
        <div className="flex flex-col gap-3">
          <div className="flex items-center gap-3">
            <p className="m-0 min-w-0 flex-1 text-sm text-on-surface-variant" data-testid="coach-brief-sheet-status">
              {briefStatusLine(briefStatusWords(t, language, brief))}
            </p>
            <Button
              size="sm"
              variant={paused ? 'primary' : 'secondary'}
              data-testid="coach-brief-pause"
              onClick={() => save({ paused: !paused })}
            >
              {paused ? t('coachHome.brief.resume') : t('coachHome.brief.pause')}
            </Button>
          </div>
          {failed && (
            <p role="alert" className="m-0 text-sm text-error" data-testid="coach-brief-save-failed">
              {t('coachHome.brief.save_failed')}
            </p>
          )}
        </div>

        <Section title={t('coachHome.brief.hour_label')} headingLevel={3}>
          <Select
            size="sm"
            aria-label={t('coachHome.brief.hour_label')}
            data-testid="coach-brief-hour"
            value={String(brief.hour)}
            onChange={(event) => save({ hour: Number(event.target.value) })}
            options={HOUR_OPTIONS}
          />
        </Section>

        <Section title={t('coachHome.brief.reasons_label')} headingLevel={3} data-testid="coach-brief-reasons">
          <ul className="m-0 list-none p-0">
            {BRIEF_REASON_KINDS.map((kind) => {
              const label = t(COACH_BRIEF_REASON_LABEL_KEY[kind]);
              return (
                <li key={kind} className="flex min-h-12 items-center justify-between gap-4 border-b ghost-border-faint last:border-b-0">
                  <span className="text-sm text-on-surface">{label}</span>
                  <Switch
                    checked={!brief.reasons_off.includes(kind)}
                    onChange={(on) => save({ reasons_off: withReason(brief.reasons_off, kind, on) })}
                    label={label}
                    data-testid={`coach-brief-reason-${kind}`}
                  />
                </li>
              );
            })}
          </ul>
        </Section>

        <Section title={t('coachHome.brief.muted_label')} headingLevel={3} data-testid="coach-brief-muted">
          {leftOut.length === 0 ? (
            <p className="m-0 text-sm text-on-surface-variant" data-testid="coach-brief-muted-none">
              {t('coachHome.brief.muted_none')}
            </p>
          ) : (
            <ul className="m-0 list-none p-0">
              {leftOut.map((athlete) => (
                <li
                  key={athlete.user_id}
                  className="flex min-h-12 items-center gap-3 border-b ghost-border-faint last:border-b-0"
                  data-testid={`coach-brief-muted-${athlete.user_id}`}
                >
                  <AthleteAvatar userId={athlete.user_id} name={athlete.name} size="sm" />
                  <span className="min-w-0 flex-1 truncate text-sm text-on-surface">{athlete.name}</span>
                  <button
                    type="button"
                    // Every row has one: the name says which athlete it puts back.
                    aria-label={`${t('coachHome.brief.unmute')}, ${athlete.name}`}
                    data-testid={`coach-brief-unmute-${athlete.user_id}`}
                    onClick={() => save({ muted: withMuted(brief.muted, athlete.user_id, false) })}
                    className="shrink-0 rounded text-sm font-medium text-primary hover:underline focus-ring-visible"
                  >
                    {t('coachHome.brief.unmute')}
                  </button>
                </li>
              ))}
            </ul>
          )}
        </Section>
      </div>
    </Sheet>
  );
}

/**
 * The athlete pane's way to leave this athlete out of the brief, or put them
 * back, under a hairline at the pane's foot: an occasional choice, kept apart
 * from the actions that handle today's reason. It sends the whole `muted`
 * list, as the PUT replaces it.
 */
export function CoachBriefMuteAction({ athleteId }: { athleteId: string }) {
  const { t } = useTranslation();
  const { brief } = useCoachBrief(true);
  const { update } = useUpdateCoachBrief();
  const showError = useErrorToast();
  if (brief === undefined) return null;
  const muted = isBriefMuted(brief, athleteId);
  const toggle = () => {
    update({ muted: withMuted(brief.muted, athleteId, !muted) }).catch(() => showError(t('coachHome.brief.save_failed')));
  };
  return (
    <div className="border-t ghost-border pt-4">
      <Button size="sm" variant="tertiary" data-testid={muted ? 'coach-brief-unmute' : 'coach-brief-mute'} onClick={toggle}>
        <Sunrise className="mr-1.5 h-4 w-4" aria-hidden="true" />
        {muted ? t('coachHome.brief.unmute') : t('coachHome.brief.mute')}
      </Button>
    </div>
  );
}
