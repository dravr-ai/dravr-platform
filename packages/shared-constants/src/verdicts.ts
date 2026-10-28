// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The claim-verdict vocabulary both chat surfaces print — status, evidence and category words as corpus keys
// ABOUTME: A constants module cannot translate, so it names the keys and each client resolves them with its own t()

import { VERDICT_STATUS_ORDER } from '@pierre/shared-types';
import type {
  ClaimEvidenceStrength,
  ClaimVerdictCategory,
  ClaimVerdictStatus,
  VerdictSummary,
} from '@pierre/shared-types';
import { countKey } from '@pierre/chat-utils';
import type { CountKeys } from '@pierre/chat-utils';

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

/**
 * The corpus keys counting the verdicts of one status, singular and plural:
 * `{{count}} appuyée` / `{{count}} appuyées`. The status word agrees with
 * the count in French, Spanish and Portuguese, so each status carries both
 * forms rather than one word under a number.
 */
export const VERDICT_COUNT_LABEL_KEY: Record<ClaimVerdictStatus, CountKeys> = {
  supported: { one: 'chat.verdictCountSupportedOne', many: 'chat.verdictCountSupportedN' },
  unsupported: { one: 'chat.verdictCountUnsupportedOne', many: 'chat.verdictCountUnsupportedN' },
  contradicted: { one: 'chat.verdictCountContradictedOne', many: 'chat.verdictCountContradictedN' },
  rhetorical: { one: 'chat.verdictCountRhetoricalOne', many: 'chat.verdictCountRhetoricalN' },
  unverifiable: { one: 'chat.verdictCountUnverifiableOne', many: 'chat.verdictCountUnverifiableN' },
};

/** What sits between two status counts on the chip. */
export const VERDICT_CHIP_SEPARATOR = ' · ';

/**
 * What sits between two status counts when the rollup is spoken: a screen
 * reader names the middle dot (« point médian ») or swallows it, where a
 * comma is a pause in every catalogue language.
 */
export const VERDICT_SPOKEN_SEPARATOR = ', ';

/** The shape of `t()` this module needs: a key and its interpolation values. */
type Translate = (key: string, values?: Record<string, string | number>) => string;

/**
 * The verdict chip's label: how many claims landed on each status.
 *
 * Every status the reply carries is counted, in the rollup's own order —
 * a finding first, then the supported claims, then the abstentions — so a
 * reply whose claims held reads « 2 appuyées · 1 invérifiable » and never
 * hides its supported count behind one abstention. Zero counts are left out.
 * The evidence strength behind a verdict is a per-claim detail and stays in
 * the drawer and the sheet. Every word comes from the corpus, so the chip
 * reads in the athlete's language on web and mobile alike. `separator` is
 * the chip's middle dot unless the label is spoken
 * ([`VERDICT_SPOKEN_SEPARATOR`]).
 */
export function verdictChipLabel(
  t: Translate,
  summary: Pick<VerdictSummary, 'statusCounts'>,
  language: string | undefined,
  separator: string = VERDICT_CHIP_SEPARATOR,
): string {
  return VERDICT_STATUS_ORDER.filter((status) => summary.statusCounts[status] > 0)
    .map((status) => {
      const count = summary.statusCounts[status];
      return t(countKey(count, language, VERDICT_COUNT_LABEL_KEY[status]), { count });
    })
    .join(separator);
}
