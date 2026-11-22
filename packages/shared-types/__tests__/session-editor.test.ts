// ABOUTME: Unit tests for the family editor's wire parsers — the family catalogue and a session's preview (carnet#1515)
// ABOUTME: Feeds each parser the bodies the subject-plan routes answer, then the malformed ones it must refuse rather than half-render

import { describe, it, expect } from 'vitest';
import { parseFamilyCatalogueResponse, parseSessionPreview } from '../src/session-editor';

/** `value` with `keys` left out, as a body that omits them. */
function without<T extends object>(value: T, ...keys: (keyof T)[]): Partial<T> {
  const copy: Partial<T> = { ...value };
  for (const key of keys) delete copy[key];
  return copy;
}

/** One family as `GET …/subject-plan/families` answers it: a preset whose run form asks fewer sets than its ride form. */
const family = {
  slug: 'vo2max_short',
  name: 'VO2max — short intervals',
  purpose: 'vo2max_short',
  purpose_label: 'short-interval VO2max work',
  sports: ['ride', 'run'],
  default_preset: '30_15',
  presets: [
    {
      key: '30_15',
      name: '30/15 (Rønnestad)',
      summary: 'Short on/off intervals',
      sports: ['ride', 'run'],
      params: {
        ride: { sets: { min: 2, max: 4, default: 3 }, reps: { min: 10, max: 13, default: 13 } },
        run: { sets: { min: 2, max: 3, default: 2 } },
      },
    },
  ],
};

/** A preview as `POST …/subject-plan/session-preview` answers it, with every optional part present. */
const preview = {
  family: 'vo2max_short',
  preset: '30_15',
  sport: 'ride',
  duration_min: 61,
  steps: [
    { label: 'Warm-up', duration_seconds: 900, target_zone: 'Z2' },
    { label: 'Work', duration_seconds: 30, target_zone: '100-110% MAP', repeat: 20, repeat_group: 1 },
  ],
  derived: {
    sets: 3,
    reps: 20,
    work_seconds: 30,
    rest_seconds: 15,
    ratio: 2,
    work_pct: 105,
    rest_pct: 50,
    amplitude_pct: 55,
    on_time_minutes: 30,
    total_minutes: 61,
    aerobic_max: 'MAP',
  },
  findings: [
    {
      code: 'param_out_of_range',
      text: "20 reps per set is outside this format's tested range (limit 13).",
      param: 'reps' as const,
    },
  ],
  outside_evidence: 'Outside the evidence',
  evidence: { label: 'Single-session trial', basis: 'Rønnestad 2015' },
  counts_as: 'Counts as threshold work',
  anchor: 'Anchor: FTP 280 W, your own.',
};

describe('parseFamilyCatalogueResponse', () => {
  it('reads the families with each preset and its ranges per sport', () => {
    expect(parseFamilyCatalogueResponse({ families: [family] })).toEqual({ families: [family] });
  });

  it('drops an optional part the server left out or sent as null', () => {
    const bare = { ...without(family, 'purpose_label'), presets: [{ ...family.presets[0], summary: null }] };
    const parsed = parseFamilyCatalogueResponse({ families: [bare] });
    expect(parsed?.families[0]).not.toHaveProperty('purpose_label');
    expect(parsed?.families[0]?.presets[0]).not.toHaveProperty('summary');
  });

  it('keeps only the six values and the three sports a preset is written for', () => {
    const params = {
      ride: { sets: { min: 2, max: 4, default: 3 }, contacts: { min: 1, max: 9, default: 5 }, reps: null },
      strength: { sets: { min: 3, max: 5, default: 4 } },
    };
    const parsed = parseFamilyCatalogueResponse({ families: [{ ...family, presets: [{ ...family.presets[0], params }] }] });
    expect(parsed?.families[0]?.presets[0]?.params).toEqual({ ride: { sets: { min: 2, max: 4, default: 3 } } });
  });

  it('refuses a sport, a range or a preset that breaks the contract', () => {
    const preset = family.presets[0];
    expect(parseFamilyCatalogueResponse({ families: [{ ...family, sports: ['ride', 'strength'] }] })).toBeNull();
    expect(
      parseFamilyCatalogueResponse({
        families: [{ ...family, presets: [{ ...preset, params: { ride: { sets: { min: 2, max: 4 } } } }] }],
      }),
    ).toBeNull();
    expect(
      parseFamilyCatalogueResponse({
        families: [{ ...family, presets: [{ ...preset, params: { ride: { sets: { min: 2, max: 'four', default: 3 } } } }] }],
      }),
    ).toBeNull();
    expect(parseFamilyCatalogueResponse({ families: [{ ...family, presets: [{ ...preset, summary: 7 }] }] })).toBeNull();
    expect(parseFamilyCatalogueResponse({ families: [{ ...family, default_preset: null }] })).toBeNull();
    expect(parseFamilyCatalogueResponse({ families: [{ ...family, purpose_label: 3 }] })).toBeNull();
    expect(parseFamilyCatalogueResponse({ families: {} })).toBeNull();
    expect(parseFamilyCatalogueResponse(null)).toBeNull();
  });
});

describe('parseSessionPreview', () => {
  it('reads a preview with its derived values, findings, chip, relabel and anchor', () => {
    expect(parseSessionPreview(preview)).toEqual(preview);
  });

  it('reads a preview the kernel derived nothing for and the card marks nothing on', () => {
    const quiet = {
      family: preview.family,
      preset: preview.preset,
      sport: 'run',
      duration_min: 40,
      steps: preview.steps,
      derived: null,
      findings: [],
      evidence: null,
    };
    expect(parseSessionPreview(quiet)).toEqual({
      family: preview.family,
      preset: preview.preset,
      sport: 'run',
      duration_min: 40,
      steps: preview.steps,
      findings: [],
    });
  });

  it("keeps the field a finding names, and drops one this client does not show", () => {
    const named = parseSessionPreview(preview);
    expect(named?.findings[0]?.param).toBe('reps');
    const unknown = parseSessionPreview({
      ...preview,
      findings: [{ code: 'param_out_of_range', text: 'Twelve contacts is past the tested range.', param: 'contacts' }],
    });
    expect(unknown?.findings).toEqual([{ code: 'param_out_of_range', text: 'Twelve contacts is past the tested range.' }]);
    expect(parseSessionPreview({ ...preview, findings: [{ code: 'x', text: 'y', param: 3 }] })).toBeNull();
  });

  it('keeps a derived figure the kernel left out absent, never zero', () => {
    const parsed = parseSessionPreview({ ...preview, derived: without(preview.derived, 'ratio', 'work_pct') });
    expect(parsed?.derived).not.toHaveProperty('ratio');
    expect(parsed?.derived).not.toHaveProperty('work_pct');
    expect(parsed?.derived?.rest_pct).toBe(50);
  });

  it('refuses a preview that breaks the contract rather than half-render it', () => {
    expect(parseSessionPreview({ ...preview, sport: 'strength' })).toBeNull();
    expect(parseSessionPreview({ ...preview, duration_min: '61' })).toBeNull();
    expect(parseSessionPreview({ ...preview, steps: [{ label: 'Work', duration_seconds: 30 }] })).toBeNull();
    expect(parseSessionPreview({ ...preview, findings: [{ code: 'param_out_of_range', text: null }] })).toBeNull();
    expect(parseSessionPreview({ ...preview, derived: { ...preview.derived, ratio: '2' } })).toBeNull();
    expect(parseSessionPreview({ ...preview, derived: { ...preview.derived, aerobic_max: undefined } })).toBeNull();
    expect(parseSessionPreview({ ...preview, evidence: { basis: 'Rønnestad 2015' } })).toBeNull();
    expect(parseSessionPreview({ ...preview, outside_evidence: true })).toBeNull();
    expect(parseSessionPreview({ ...preview, anchor: 280 })).toBeNull();
    expect(parseSessionPreview(null)).toBeNull();
  });
});
