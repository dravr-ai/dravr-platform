// ABOUTME: The admin triage panel one verdict card carries — the knob to adjust, and the disposition control that writes support's call
// ABOUTME: Operator chrome (English by decision); reached only from ClaimVerdictsTab, never from the chat surface's drawer
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import type { ClaimVerdict, DispositionReason, VerdictDisposition } from '@pierre/shared-types';
import { DISPOSITION_REASONS, VERDICT_DISPOSITIONS } from '@pierre/shared-types';
import { formatDateTime } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';
import type { VerdictKnob } from '../services/api/admin';
import { Button, Select, Textarea } from './ui';

/** What an admin records about one verdict. */
export interface DispositionInput {
  disposition: VerdictDisposition;
  reason?: DispositionReason;
  note?: string;
}

/** Persist a disposition. Resolves once the row is written; rejects with the API error. */
export type SetDisposition = (verdict: ClaimVerdict, input: DispositionInput) => Promise<void>;

const DISPOSITION_LABEL: Record<VerdictDisposition, string> = {
  true_catch: 'True catch',
  false_positive: 'False positive',
  unsure: 'Unsure',
};

const REASON_LABEL: Record<DispositionReason, string> = {
  missing_keyword: 'Missing keyword',
  bound_too_tight: 'Bound too tight',
  tolerance_too_tight: 'Tolerance too tight',
  stale_evidence: 'Stale evidence',
  extractor_misroute: 'Extractor misroute',
  judge_error: 'Judge error',
  other: 'Other',
};

const KNOB_KIND_LABEL: Record<VerdictKnob['kind'], string> = {
  rhetoric_filter: 'Rhetoric filter',
  deterministic_bounds: 'Deterministic bounds',
  personalized_tolerance: 'Personalized tolerance',
  athlete_data_record: 'Athlete data record',
  evidence_corpus: 'Evidence corpus',
  consistency_check: 'Consistency check',
  judge_prompt: 'Judge prompt',
};

/** Where to edit if the verdict was wrong — the pointer panel the detail read computed. */
function KnobPanel({ knob }: { knob: VerdictKnob | undefined }) {
  return (
    <section data-testid="verdict-knob">
      <h4 className="mb-1 text-xs font-semibold text-outline">What to adjust</h4>
      {knob ? (
        <div className="space-y-2 rounded-lg bg-surface-container-low p-3">
          <div className="flex flex-wrap items-center gap-2 text-xs">
            <span className="rounded-full bg-surface-container-high px-2 py-0.5 text-on-surface">
              {KNOB_KIND_LABEL[knob.kind]}
            </span>
            <code className="font-mono text-xs text-on-surface break-all">{knob.location}</code>
          </div>
          <p className="text-xs text-on-surface">{knob.detail}</p>
          {knob.propositions.length > 0 ? (
            <ul className="space-y-1">
              {knob.propositions.map((p) => (
                <li key={p.path} className="flex flex-wrap items-center gap-2 text-xs">
                  <code className="font-mono text-on-surface break-all">{p.path}</code>
                  <span className="text-outline">{p.strength}</span>
                  <span className="text-outline">score {p.score}</span>
                  {p.cited ? (
                    <span className="rounded-full bg-success/15 px-2 py-0.5 text-on-success-container">
                      cited
                    </span>
                  ) : null}
                </li>
              ))}
            </ul>
          ) : null}
        </div>
      ) : (
        <p className="text-xs text-outline">Resolving the knob…</p>
      )}
    </section>
  );
}

/** The three-way call, its reason and note, and the save that writes them. */
function DispositionControl({
  verdict,
  onSetDisposition,
}: {
  verdict: ClaimVerdict;
  onSetDisposition: SetDisposition;
}) {
  // The panel's copy is English by decision (operator chrome); the date still
  // follows the viewer's locale, as every other timestamp in the shell does.
  const { language } = useTranslation();
  const [disposition, setDisposition] = useState<VerdictDisposition | ''>(verdict.disposition ?? '');
  const [reason, setReason] = useState<DispositionReason | ''>(verdict.disposition_reason ?? '');
  const [note, setNote] = useState<string>(verdict.disposition_note ?? '');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const save = async () => {
    if (!disposition) return;
    setSaving(true);
    setError(null);
    try {
      await onSetDisposition(verdict, {
        disposition,
        reason: reason || undefined,
        note: note.trim() || undefined,
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <section data-testid="verdict-disposition" className="space-y-3">
      <h4 className="text-xs font-semibold text-outline">Disposition</h4>
      {verdict.disposed_by && verdict.disposed_at ? (
        <p className="text-xs text-outline">
          Disposed by {verdict.disposed_by} · {formatDateTime(verdict.disposed_at, language)}
        </p>
      ) : null}
      <Select
        label="Call"
        aria-label="Disposition call"
        value={disposition}
        onChange={(e) => setDisposition(e.target.value as VerdictDisposition | '')}
        options={[
          { value: '', label: 'Not disposed' },
          ...VERDICT_DISPOSITIONS.map((d) => ({ value: d, label: DISPOSITION_LABEL[d] })),
        ]}
      />
      <Select
        label="Reason"
        aria-label="Disposition reason"
        value={reason}
        onChange={(e) => setReason(e.target.value as DispositionReason | '')}
        options={[
          { value: '', label: 'No reason' },
          ...DISPOSITION_REASONS.map((r) => ({ value: r, label: REASON_LABEL[r] })),
        ]}
      />
      <Textarea
        label="Note"
        aria-label="Disposition note"
        value={note}
        onChange={(e) => setNote(e.target.value)}
        placeholder="What the triager saw"
        rows={2}
      />
      {error ? <p className="text-xs text-error">{error}</p> : null}
      <Button
        type="button"
        onClick={() => void save()}
        disabled={!disposition || saving}
        loading={saving}
        size="sm"
      >
        Save disposition
      </Button>
    </section>
  );
}

/**
 * Everything the admin adds under one verdict card: the knob behind the
 * verdict and the control that records what support made of it.
 *
 * The chat drawer renders this through its `renderTriage` slot and never
 * imports it, so the drawer an athlete opens carries none of this English.
 */
export default function VerdictTriagePanel({
  verdict,
  knob,
  onSetDisposition,
}: {
  verdict: ClaimVerdict;
  /** The knob the detail read resolved; absent while it loads. */
  knob: VerdictKnob | undefined;
  onSetDisposition: SetDisposition;
}) {
  return (
    <>
      <KnobPanel knob={knob} />
      <DispositionControl verdict={verdict} onSetDisposition={onSetDisposition} />
    </>
  );
}
