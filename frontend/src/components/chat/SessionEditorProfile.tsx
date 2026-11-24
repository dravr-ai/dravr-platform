// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The hand session's profile — one bar per step and per repetition of a set, as wide as it lasts and as tall as its target (carnet#1727)
// ABOUTME: A first-round bar is a button that opens its step; a set's later rounds are drawn quieter, under a bracket that reads × N

import { clsx } from 'clsx';
import { useTranslation } from '@pierre/i18n';
import type { PlanStep } from '@pierre/shared-types';
import {
  PROFILE_EFFORT_LEVEL,
  SESSION_EDITOR_KEYS,
  formatStepDuration,
  profileBlocks,
  stepsTotalSeconds,
  type ProfileBar,
} from '@pierre/ui-logic';

interface SessionEditorProfileProps {
  steps: readonly PlanStep[];
  /** The step whose editor is open, outlined on the profile. */
  openIndex: number | null;
  /** A bar pressed: the step it draws opens. */
  onOpen: (index: number) => void;
}

/**
 * The session drawn left to right. Widths follow each bar's seconds through
 * flex-grow, heights its target's level (12px at the floor, the row's full
 * 64px on a phone and 88px from `lg`). Colour is never the only channel: each
 * first-round bar is named by its step, its time and its target.
 */
export function SessionEditorProfile({ steps, openIndex, onOpen }: SessionEditorProfileProps) {
  const { t } = useTranslation();
  const blocks = profileBlocks(steps);
  const totalMinutes = Math.ceil(stepsTotalSeconds(steps) / 60);

  const bar = (item: ProfileBar, key: string) => {
    const step = steps[item.index] as PlanStep;
    const className = clsx(
      'min-w-0.5 basis-0 rounded-t-[3px]',
      item.level >= PROFILE_EFFORT_LEVEL ? 'bg-primary' : 'bg-outline-variant',
    );
    // 12px at the floor, the row's height at a level of 1.
    const style = { flexGrow: item.seconds, height: `calc(12px + (100% - 12px) * ${item.level.toFixed(3)})` };
    if (item.repeated) {
      return <span key={key} aria-hidden="true" className={clsx(className, 'opacity-[.55]')} style={style} />;
    }
    return (
      <button
        key={key}
        type="button"
        aria-label={t(SESSION_EDITOR_KEYS.profileStep, {
          number: item.index + 1,
          label: step.label,
          duration: formatStepDuration(step.duration_seconds),
          target: step.target_zone,
        })}
        aria-pressed={openIndex === item.index}
        onClick={() => onOpen(item.index)}
        className={clsx(
          className,
          'transition-opacity duration-base hover:opacity-80 focus-ring-visible',
          openIndex === item.index && 'outline outline-2 outline-offset-2 outline-on-surface',
        )}
        style={style}
        data-testid={`session-editor-profile-step-${item.index + 1}`}
      />
    );
  };

  return (
    <div data-testid="session-editor-profile">
      <div
        role="group"
        aria-label={t(SESSION_EDITOR_KEYS.profile)}
        className="flex items-end gap-0.5 border-b border-outline"
      >
        {blocks.map((block) => {
          if (block.kind === 'step') {
            return (
              <div
                key={`step-${block.bar.index}`}
                className="flex h-16 items-end lg:h-[88px]"
                style={{ flexGrow: block.bar.seconds, flexBasis: 0 }}
              >
                {bar(block.bar, 'bar')}
              </div>
            );
          }
          const seconds = block.bars.reduce((sum, item) => sum + item.seconds, 0);
          return (
            <div
              key={`series-${block.start}`}
              className="flex min-w-0 flex-col"
              style={{ flexGrow: seconds, flexBasis: 0 }}
            >
              {/* The set's bracket: one line over every round, and its count; the list's stepper says it aloud. */}
              <span aria-hidden="true" className="mb-1 border-t-2 border-on-surface pt-0.5 font-mono text-xs text-on-surface">
                × {block.repeat}
              </span>
              <div className="flex h-16 items-end gap-0.5 lg:h-[88px]">
                {block.bars.map((item, at) => bar(item, `bar-${at}`))}
              </div>
            </div>
          );
        })}
      </div>
      <div aria-hidden="true" className="mt-1 flex justify-between font-mono text-xs text-outline">
        <span>0</span>
        <span>{t(SESSION_EDITOR_KEYS.minutes, { count: String(totalMinutes) })}</span>
      </div>
    </div>
  );
}
