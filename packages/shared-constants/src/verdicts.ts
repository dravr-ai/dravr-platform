// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The claim-verdict vocabulary both chat surfaces print — status, evidence and category words as corpus keys
// ABOUTME: A constants module cannot translate, so it names the keys and each client resolves them with its own t()

import type {
  ClaimEvidenceStrength,
  ClaimVerdictCategory,
  ClaimVerdictStatus,
  VerdictSummary,
} from '@pierre/shared-types';

/** The corpus key naming what the verifier concluded about a claim. */
export const VERDICT_STATUS_LABEL_KEY: Record<ClaimVerdictStatus, string> = {
  supported: 'chat.verdictStatusSupported',
  unsupported: 'chat.verdictStatusUnsupported',
  contradicted: 'chat.verdictStatusContradicted',
  rhetorical: 'chat.verdictStatusRhetorical',
  unverifiable: 'chat.verdictStatusUnverifiable',
};

/** The corpus key naming how much evidence stood behind a verdict. */
export const EVIDENCE_STRENGTH_LABEL_KEY: Record<ClaimEvidenceStrength, string> = {
  strong: 'chat.evidenceStrong',
  mixed: 'chat.evidenceMixed',
  weak: 'chat.evidenceWeak',
  none: 'chat.evidenceNone',
};

/** The corpus key naming the domain a flagged claim belongs to. */
export const VERDICT_CATEGORY_LABEL_KEY: Record<ClaimVerdictCategory, string> = {
  physiological: 'chat.verdictCategoryPhysiological',
  training_prescription: 'chat.verdictCategoryTrainingPrescription',
  nutrition: 'chat.verdictCategoryNutrition',
  recovery: 'chat.verdictCategoryRecovery',
  supplement: 'chat.verdictCategorySupplement',
  injury_rehab: 'chat.verdictCategoryInjuryRehab',
  athlete_data: 'chat.verdictCategoryAthleteData',
};

/** The corpus key for a category this client does not know yet. */
export const VERDICT_CATEGORY_OTHER_KEY = 'chat.verdictCategoryOther';

/**
 * The corpus key for a verdict's category, read off the wire.
 *
 * The server can add a category before a shipped client learns its name, so
 * a value outside `VERDICT_CATEGORY_LABEL_KEY` resolves to the generic
 * « other » word rather than the raw enum: the athlete never reads internal
 * vocabulary, and the card still says the claim was categorised.
 */
export function verdictCategoryLabelKey(category: string): string {
  return Object.prototype.hasOwnProperty.call(VERDICT_CATEGORY_LABEL_KEY, category)
    ? VERDICT_CATEGORY_LABEL_KEY[category as ClaimVerdictCategory]
    : VERDICT_CATEGORY_OTHER_KEY;
}

/** The chip line for exactly one verdict: `{{count}} verdict · {{qualifier}}`. */
export const VERDICT_CHIP_ONE_KEY = 'chat.verdictChipOne';
/** The chip line for several verdicts: `{{count}} verdicts · {{qualifier}}`. */
export const VERDICT_CHIP_N_KEY = 'chat.verdictChipN';

/** The shape of `t()` this module needs: a key and its interpolation values. */
type Translate = (key: string, values?: Record<string, string | number>) => string;

/**
 * The verdict chip's label: how many verdicts, and the worst thing about them.
 *
 * The qualifier is the worst status word — « contredite », « non appuyée » —
 * whether the surface read the rows or only has the turn's chips, so the
 * word sits on the same axis as the chip's tone. The evidence strength behind
 * a verdict is a per-claim detail and stays in the drawer and the sheet, where
 * `chat.evidenceLabel` prints it beside each card. Both words come from the
 * corpus, so the chip reads in the athlete's language on web and mobile alike.
 */
export function verdictChipLabel(
  t: Translate,
  summary: Pick<VerdictSummary, 'count' | 'worstStatus'>,
): string {
  return t(summary.count === 1 ? VERDICT_CHIP_ONE_KEY : VERDICT_CHIP_N_KEY, {
    count: summary.count,
    qualifier: t(VERDICT_STATUS_LABEL_KEY[summary.worstStatus]),
  });
}
