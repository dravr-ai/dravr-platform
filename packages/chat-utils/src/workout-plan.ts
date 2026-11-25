// ABOUTME: The pure half of the workout-plan card — a step's compact duration, a day's fuelling fragments and its evidence words
// ABOUTME: Both clients print the plan's figures and evidence from here, so a chip or a sodium line cannot read differently on the phone

import type { PlanDay, PlanDayEvidence, PlanFueling } from '@pierre/shared-types';

import type { Translate } from './text';

/**
 * A step's duration as the compact figure the steps line carries: seconds
 * under a minute ("30 s") so a stride does not read as "0 min", whole minutes
 * as "4 min", and anything else as "1:30" so a 90 s interval is not rounded
 * up to two minutes. "min", never "m", because the same card prints distances
 * in metres.
 */
export function stepDuration(seconds: number): string {
  const total = Math.max(0, Math.round(seconds));
  if (total < 60) return `${total} s`;
  return total % 60 === 0 ? `${total / 60} min` : clockDuration(total);
}

/**
 * A duration as a clock reads it, `m:ss` (`90` → `1:30`, `600` → `10:00`): the
 * steps line's odd minutes and the day editor's step field print the same.
 */
export function clockDuration(seconds: number): string {
  const whole = Math.max(0, Math.round(seconds));
  return `${Math.floor(whole / 60)}:${String(whole % 60).padStart(2, '0')}`;
}

/**
 * The fuelling fragments for one day, in display order. Sodium is shown as
 * an estimated sweat loss and never as a required intake: the evidence does
 * not support prescribing a mg/h figure.
 */
export function fuelParts(fueling: PlanFueling, t: Translate): string[] {
  const parts = [
    t('chat.fuelCarbs', { value: fueling.carbs_g_per_h }),
    t('chat.fuelFluid', { value: fueling.fluid_ml_per_h }),
  ];
  if (fueling.sodium_mg_per_h !== undefined) {
    parts.push(t('chat.fuelSodiumLoss', { value: fueling.sodium_mg_per_h }));
  }
  return parts;
}

/**
 * `key` in the reader's words, or `fallback` when the reader's catalogue has
 * no text for it (a missing key comes back as itself).
 */
function worded(t: Translate, key: string, fallback: string): string {
  const text = t(key);
  return text === key ? fallback : text;
}

/**
 * A day's evidence chip in the reader's language. The card is saved with the
 * chip worded in the language of the turn that saved it; its kind is what
 * reads the same whoever opens it. A card saved before the kind travelled
 * prints its stored label.
 */
export function evidenceChipLabel(evidence: PlanDayEvidence, t: Translate): string {
  return evidence.kind ? worded(t, `messaging.evidence.${evidence.kind}`, evidence.label) : evidence.label;
}

/**
 * What a day's evidence chip opens on: the studies it rests on, else why none
 * covers the session, in the reader's language when the note travelled by
 * name. `undefined` when there is neither.
 */
export function evidenceChipDetail(evidence: PlanDayEvidence, t: Translate): string | undefined {
  if (evidence.basis) return evidence.basis;
  if (evidence.note_kind) {
    return worded(t, `messaging.evidence.${evidence.note_kind}`, evidence.note ?? '') || undefined;
  }
  return evidence.note;
}

/**
 * The warning on a day saved outside its family's evidence, in the reader's
 * language; `undefined` on any other day. The warning is one sentence, so a
 * card saved before the flag travelled, which carries only the worded label,
 * reads in the reader's language too.
 */
export function outsideEvidenceLabel(
  day: Pick<PlanDay, 'outside' | 'outside_evidence'>,
  t: Translate,
): string | undefined {
  if (!day.outside && day.outside_evidence === undefined) return undefined;
  return worded(t, 'messaging.evidence.outside', day.outside_evidence ?? '') || undefined;
}

/**
 * "Counts as <purpose>" for a day an edit relabelled, in the reader's
 * language when the purpose travelled; the stored line otherwise. The
 * catalogue's line is server-rendered, its placeholder positional (`{0}`).
 */
export function countsAsLine(
  day: Pick<PlanDay, 'counts_as' | 'counts_as_purpose'>,
  t: Translate,
): string | undefined {
  if (!day.counts_as_purpose) return day.counts_as;
  const purposeKey = `messaging.envelope.purpose.${day.counts_as_purpose}`;
  const purpose = t(purposeKey);
  const line = t('messaging.evidence.counts_as');
  if (purpose === purposeKey || line === 'messaging.evidence.counts_as') return day.counts_as;
  return line.split('{0}').join(purpose);
}
