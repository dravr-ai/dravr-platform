// ABOUTME: Unit tests for the workout-plan card's figures and evidence words — step durations, fuelling, the chip in the reader's language
// ABOUTME: Red if a 90 s interval reads "2 min", sodium turns into a prescribed intake, or a chip saved in English reads English to a French reader

import { describe, expect, it } from 'vitest';
import {
  clockDuration,
  countsAsLine,
  evidenceChipDetail,
  evidenceChipLabel,
  fuelParts,
  outsideEvidenceLabel,
  stepDuration,
} from '../src/workout-plan';

/** A translator that shows which key and params it was handed. */
const t = (key: string, params?: Record<string, string | number>) =>
  `${key}(${JSON.stringify(params ?? {})})`;

describe('clockDuration', () => {
  it('reads a duration as a clock, seconds padded, a negative as nothing', () => {
    expect(clockDuration(90)).toBe('1:30');
    expect(clockDuration(600)).toBe('10:00');
    expect(clockDuration(5)).toBe('0:05');
    expect(clockDuration(89.6)).toBe('1:30');
    expect(clockDuration(-3)).toBe('0:00');
  });
});

describe('stepDuration', () => {
  it('prints seconds under a minute so a stride never reads "0 min"', () => {
    expect(stepDuration(20)).toBe('20 s');
    expect(stepDuration(59)).toBe('59 s');
  });

  it('prints whole minutes as "min", never "m", which reads as metres', () => {
    expect(stepDuration(60)).toBe('1 min');
    expect(stepDuration(240)).toBe('4 min');
    expect(stepDuration(1200)).toBe('20 min');
  });

  it('keeps the seconds of a part minute instead of rounding them away', () => {
    expect(stepDuration(90)).toBe('1:30');
    expect(stepDuration(450)).toBe('7:30');
    expect(stepDuration(65)).toBe('1:05');
  });

  it('rounds a fractional second before choosing the form', () => {
    expect(stepDuration(59.6)).toBe('1 min');
    expect(stepDuration(29.4)).toBe('29 s');
  });
});

describe('fuelParts', () => {
  it('lists carbohydrate then fluid', () => {
    expect(fuelParts({ carbs_g_per_h: 60, fluid_ml_per_h: 500 }, t)).toEqual([
      'chat.fuelCarbs({"value":60})',
      'chat.fuelFluid({"value":500})',
    ]);
  });

  it('adds sodium as an estimated loss when the plan carries it', () => {
    expect(
      fuelParts({ carbs_g_per_h: 90, fluid_ml_per_h: 750, sodium_mg_per_h: 800 }, t),
    ).toEqual([
      'chat.fuelCarbs({"value":90})',
      'chat.fuelFluid({"value":750})',
      'chat.fuelSodiumLoss({"value":800})',
    ]);
  });
});

/** A French reader's catalogue: what a card saved in English reads as for them. */
const FRENCH: Record<string, string> = {
  'messaging.evidence.convention': "Convention d'entraînement",
  'messaging.evidence.no_basis_convention': 'Aucun essai ne couvre cette séance.',
  'messaging.evidence.outside': 'Hors des données probantes',
  'messaging.evidence.counts_as': 'Compte comme {0}',
  'messaging.envelope.purpose.threshold': 'une séance au seuil',
};
const french = (key: string) => FRENCH[key] ?? key;

describe('evidenceChipLabel and evidenceChipDetail', () => {
  it('word a chip saved in another language by its kind, in the reader\'s language', () => {
    const evidence = {
      kind: 'convention' as const,
      label: 'Coaching convention',
      note: 'No trial covers this session.',
      note_kind: 'no_basis_convention' as const,
    };
    expect(evidenceChipLabel(evidence, french)).toBe("Convention d'entraînement");
    expect(evidenceChipDetail(evidence, french)).toBe('Aucun essai ne couvre cette séance.');
  });

  it('print the stored words of a card saved before the kind travelled', () => {
    const evidence = { label: 'Coaching convention', note: 'No trial covers this session.' };
    expect(evidenceChipLabel(evidence, french)).toBe('Coaching convention');
    expect(evidenceChipDetail(evidence, french)).toBe('No trial covers this session.');
  });

  it('open on the cited studies before any note', () => {
    const evidence = { kind: 'transposed' as const, label: 'Transposed', basis: 'Rønnestad 2015 [RCT]' };
    expect(evidenceChipDetail(evidence, french)).toBe('Rønnestad 2015 [RCT]');
  });

  it('fall back to the stored label when the reader\'s catalogue lacks the kind', () => {
    expect(evidenceChipLabel({ kind: 'acute_only', label: 'Short-term data only' }, french)).toBe(
      'Short-term data only',
    );
  });
});

describe('outsideEvidenceLabel', () => {
  it('words the warning in the reader\'s language, from the flag or from a stored label alone', () => {
    expect(outsideEvidenceLabel({ outside: true, outside_evidence: 'Outside the evidence' }, french)).toBe(
      'Hors des données probantes',
    );
    expect(outsideEvidenceLabel({ outside_evidence: 'Outside the evidence' }, french)).toBe(
      'Hors des données probantes',
    );
  });

  it('says nothing on a day inside the evidence', () => {
    expect(outsideEvidenceLabel({}, french)).toBeUndefined();
  });
});

describe('countsAsLine', () => {
  it('words a relabel by its purpose in the reader\'s language', () => {
    expect(
      countsAsLine({ counts_as: 'Counts as threshold work', counts_as_purpose: 'threshold' }, french),
    ).toBe('Compte comme une séance au seuil');
  });

  it('prints the stored line when the purpose did not travel or has no words here', () => {
    expect(countsAsLine({ counts_as: 'Counts as threshold work' }, french)).toBe('Counts as threshold work');
    expect(countsAsLine({ counts_as: 'Counts as hills', counts_as_purpose: 'hills' }, french)).toBe(
      'Counts as hills',
    );
  });
});
