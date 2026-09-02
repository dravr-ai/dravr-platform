// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The claim-verdict vocabulary both chat surfaces print — status and evidence words as corpus keys
// ABOUTME: A constants module cannot translate, so it names the keys and each client resolves them with its own t()

import type { ClaimEvidenceStrength, ClaimVerdictStatus, VerdictSummary } from '@pierre/shared-types';

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

/** The chip line for exactly one verdict: `{{count}} verdict · {{qualifier}}`. */
export const VERDICT_CHIP_ONE_KEY = 'chat.verdictChipOne';
/** The chip line for several verdicts: `{{count}} verdicts · {{qualifier}}`. */
export const VERDICT_CHIP_N_KEY = 'chat.verdictChipN';

/** The shape of `t()` this module needs: a key and its interpolation values. */
type Translate = (key: string, values?: Record<string, string | number>) => string;

/**
 * The verdict chip's label: how many verdicts, and the worst thing about them.
 *
 * The qualifier is the weakest evidence where a surface read the rows, and
 * the worst status where it only has the turn's chips — so a chip never has
 * to invent a strength nobody sent it. Both words come from the corpus, so
 * the chip reads in the athlete's language on web and mobile alike.
 */
export function verdictChipLabel(
  t: Translate,
  summary: Pick<VerdictSummary, 'count' | 'worstStatus' | 'worstStrength'>,
): string {
  const qualifier = t(
    summary.worstStrength === null
      ? VERDICT_STATUS_LABEL_KEY[summary.worstStatus]
      : EVIDENCE_STRENGTH_LABEL_KEY[summary.worstStrength],
  );
  return t(summary.count === 1 ? VERDICT_CHIP_ONE_KEY : VERDICT_CHIP_N_KEY, {
    count: summary.count,
    qualifier,
  });
}
