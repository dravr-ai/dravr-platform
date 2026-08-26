// ABOUTME: The claim-verdict row and the one severity rollup web and mobile both read
// ABOUTME: Lifted out of MessageItem.tsx so a chip, a drawer and an admin table cannot disagree

import type { ReplyVerdictChip } from './turn.js';

/** What the verifier concluded about one claim. */
export type ClaimVerdictStatus =
  | 'supported'
  | 'unsupported'
  | 'contradicted'
  | 'rhetorical'
  | 'unverifiable';

/** How much evidence stood behind the conclusion. */
export type ClaimEvidenceStrength = 'strong' | 'mixed' | 'weak' | 'none';

/** The domain a flagged claim belongs to. */
export type ClaimVerdictCategory =
  | 'physiological'
  | 'training_prescription'
  | 'nutrition'
  | 'recovery'
  | 'supplement'
  | 'injury_rehab';

/** Which verifier layer produced the verdict. */
export type ClaimVerdictLayer =
  | 'rhetoric'
  | 'deterministic'
  | 'personalized'
  | 'evidence'
  | 'consistency'
  | 'judge';

/** Feedback tone a surface paints a verdict in. */
export type VerdictTone = 'success' | 'warning' | 'error' | 'info' | 'secondary';

/**
 * One `claim_verdicts` row, as both the chat read and the admin read carry it.
 *
 * One database row had two client-side shapes — a chat one and an admin one —
 * that differed only in which columns each read happened to select. They are
 * one type here, and the columns only the admin read returns are the optional
 * ones.
 */
export interface ClaimVerdict {
  /** Verdict id. */
  id: string;
  /** Conversation the claim was made in, when it came from chat. */
  conversation_id: string | null;
  /** Message the claim was made in, when it came from chat. */
  message_id: string | null;
  /** Coach whose reply carried the claim. */
  coach_id: string | null;
  /** The claim's own sentence, verbatim. */
  claim_text: string;
  /** The domain the claim belongs to. */
  category: ClaimVerdictCategory;
  /** What the verifier concluded. */
  status: ClaimVerdictStatus;
  /** How much evidence stood behind the conclusion. */
  evidence_strength: ClaimEvidenceStrength;
  /** Verifier confidence, 0–1. */
  confidence: number;
  /** Which verifier layer produced the verdict. */
  layer_fired: ClaimVerdictLayer;
  /** What the detector found, when it explained itself. */
  explanation: string | null;
  /** Comma-separated evidence references. */
  evidence_refs: string | null;
  /** RFC3339 instant the verdict was written. */
  created_at: string;
  /** Owning tenant. Returned by the admin read only. */
  tenant_id?: string;
  /** Athlete the claim was made to. Returned by the admin read only. */
  user_id?: string;
}

/**
 * The severity of one verdict, which is all a chip needs.
 *
 * A `verdicts` reply block carries a status alone; the conversation's verdict
 * read carries a strength beside it. Both reduce to this.
 */
export interface VerdictSeverity {
  /** What the verifier concluded. */
  status: ClaimVerdictStatus;
  /** Absent on a claim that arrived as a reply-block chip. */
  evidence_strength?: ClaimEvidenceStrength;
}

/** The worst verdict attached to one reply, and how many there were. */
export interface VerdictSummary {
  /** The most severe status across the set. */
  worstStatus: ClaimVerdictStatus;
  /** The weakest evidence across the set, or `null` when none carried any. */
  worstStrength: ClaimEvidenceStrength | null;
  /** How many verdicts the summary covers. */
  count: number;
  /** The tone [`worstStatus`](VerdictSummary.worstStatus) paints in. */
  tone: VerdictTone;
}

/** Severity order for a status — higher wins the rollup. */
const STATUS_PRIORITY: Record<ClaimVerdictStatus, number> = {
  contradicted: 4,
  unsupported: 3,
  unverifiable: 2,
  rhetorical: 1,
  supported: 0,
};

/** Severity order for an evidence strength — higher (weaker) wins the rollup. */
const STRENGTH_PRIORITY: Record<ClaimEvidenceStrength, number> = {
  none: 4,
  weak: 3,
  mixed: 2,
  strong: 1,
};

/** The tone each status paints in, on every surface. */
export const VERDICT_STATUS_TONE: Record<ClaimVerdictStatus, VerdictTone> = {
  contradicted: 'error',
  unsupported: 'warning',
  unverifiable: 'secondary',
  rhetorical: 'info',
  supported: 'success',
};

/**
 * Decode a reply-block chip into a severity.
 *
 * The block's single `contradicted` flag is the server's own two-state
 * verdict: `true` means the claim violated a deterministic bound, `false`
 * means it merely had no supporting evidence. Reading it anywhere else would
 * be a second copy of that rule.
 */
export function verdictChipSeverity(chip: ReplyVerdictChip): VerdictSeverity {
  return { status: chip.contradicted ? 'contradicted' : 'unsupported' };
}

/**
 * Combine the verdict rows a surface read with the chips its turn carried.
 *
 * The two describe the same claims: the rows come from the conversation's
 * verdict read and carry an evidence strength, the chips ride the turn's own
 * `verdicts` block and carry a status alone. A claim present in both is
 * counted once, from the row, because the row says more about it.
 */
export function mergeVerdictSeverities(
  rows: readonly ClaimVerdict[],
  chips: readonly ReplyVerdictChip[],
): VerdictSeverity[] {
  const claimed = new Set(rows.map((row) => row.claim_text));
  const merged: VerdictSeverity[] = rows.map((row) => ({
    status: row.status,
    evidence_strength: row.evidence_strength,
  }));
  for (const chip of chips) {
    if (!claimed.has(chip.claim)) merged.push(verdictChipSeverity(chip));
  }
  return merged;
}

/**
 * Roll a set of verdicts up into the one chip a reply shows.
 *
 * Returns `null` for an empty set, which is the "draw no chip" answer.
 */
export function summarizeVerdicts(
  verdicts: readonly VerdictSeverity[],
): VerdictSummary | null {
  if (verdicts.length === 0) return null;

  let worstStatus: ClaimVerdictStatus = 'supported';
  let worstStrength: ClaimEvidenceStrength | null = null;
  for (const verdict of verdicts) {
    if (STATUS_PRIORITY[verdict.status] > STATUS_PRIORITY[worstStatus]) {
      worstStatus = verdict.status;
    }
    const strength = verdict.evidence_strength;
    if (
      strength !== undefined &&
      (worstStrength === null ||
        STRENGTH_PRIORITY[strength] > STRENGTH_PRIORITY[worstStrength])
    ) {
      worstStrength = strength;
    }
  }

  return {
    worstStatus,
    worstStrength,
    count: verdicts.length,
    tone: VERDICT_STATUS_TONE[worstStatus],
  };
}

/**
 * The chip's label: how many verdicts, and the worst thing about them.
 *
 * The qualifier is the weakest evidence where a surface read the rows, and the
 * worst status where it only has the turn's chips — so a chip never has to
 * invent a strength nobody sent it.
 */
export function verdictSummaryLabel(summary: VerdictSummary): string {
  const noun = summary.count === 1 ? 'verdict' : 'verdicts';
  return `${summary.count} ${noun} · ${summary.worstStrength ?? summary.worstStatus}`;
}
