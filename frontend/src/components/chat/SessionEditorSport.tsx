// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The day editor's sport chips — the one-tap sports, the day's own word, and "Other…" for a sport the coach names (carnet#1727)
// ABOUTME: One pick moves both modes; the chips are built once from the opening session, so they never reorder under the cursor

import { Check } from 'lucide-react';
import { useTranslation } from '@pierre/i18n';
import { SESSION_EDITOR_KEYS, chosenSport, sportLabel } from '@pierre/ui-logic';
import { Chip, Input } from '../ui';

interface SessionEditorSportProps {
  /** The session's sport word, as the hand session holds it. */
  sport: string;
  /** The chips offered: the one-tap sports, then the day's own word when it names none of them. */
  choices: readonly string[];
  /** Whether the coach is naming a sport of their own: "Other…" is picked and its field shows. */
  otherOpen: boolean;
  /** A chip pressed: its word, the same for both modes. */
  onPick: (sport: string) => void;
  /** "Other…" pressed: the field opens empty, and what the coach types is the sport. */
  onOther: () => void;
  /** The coach typing the sport's name in the "Other…" field. */
  onName: (sport: string) => void;
}

/**
 * One chip per sport, the picked one on the tint with a check and its words
 * in semibold, then a dashed "Other…" that opens a field labelled "Name the
 * sport". The row wraps; on a phone each chip is a 44px target.
 */
export function SessionEditorSport({ sport, choices, otherOpen, onPick, onOther, onName }: SessionEditorSportProps) {
  const { t } = useTranslation();
  const picked = otherOpen ? null : chosenSport(sport, choices);
  return (
    <div className="space-y-3">
      <div
        role="group"
        aria-label={t(SESSION_EDITOR_KEYS.sport)}
        className="flex flex-wrap gap-2"
        data-testid="session-editor-sport"
      >
        {choices.map((choice) => {
          const selected = picked === choice;
          return (
            <Chip
              key={choice}
              selected={selected}
              onClick={() => onPick(choice)}
              data-testid={`session-editor-sport-${choice}`}
            >
              {selected && <Check className="h-4 w-4" aria-hidden="true" />}
              {sportLabel(t, choice)}
            </Chip>
          );
        })}
        <Chip selected={otherOpen} dashed onClick={onOther} data-testid="session-editor-sport-other">
          {otherOpen && <Check className="h-4 w-4" aria-hidden="true" />}
          {t(SESSION_EDITOR_KEYS.sportOther)}
        </Chip>
      </div>
      {otherOpen && (
        <Input
          label={t(SESSION_EDITOR_KEYS.sportOtherName)}
          value={sport}
          autoComplete="off"
          // The coach pressed "Other…" to type a name: the field takes the keyboard.
          autoFocus
          onChange={(event) => onName(event.target.value)}
          data-testid="session-editor-sport-name"
        />
      )}
    </div>
  );
}
