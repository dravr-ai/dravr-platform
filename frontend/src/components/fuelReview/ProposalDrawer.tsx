// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The fuel-proposal review drawer — every stored photo, the reader's notes, ingredients and blend claim, the identity and the facts
// ABOUTME: An operator corrects the identity and facts before approving, or rejects with a reason; each outcome code the server answers is explained

import { useMemo, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import { describeApiError } from '@pierre/ui-logic';
import { QUERY_KEYS } from '../../constants/queryKeys';
import {
  adminApi,
  type FuelApprovalResult,
  type FuelFactOverrides,
  type FuelProductProposal,
  type FuelRejectReason,
  type FuelRejectResult,
} from '../../services/api/admin';
import { Button, ConfirmDialog, Section, Select, Sheet, Textarea } from '../ui';
import ProposalPhotos from './ProposalPhotos';
import { FactRow, IdentityFields } from './ReviewFields';
import {
  FACT_FIELDS,
  REJECT_REASONS,
  SOURCE_LABELS,
  buildOverrides,
  fieldLabel,
  formLabel,
  isWebLink,
  productName,
  type FactKey,
  type FieldEdit,
  type IdentityEdits,
} from './fuelReviewModel';

function outcomeMessage(result: FuelApprovalResult | FuelRejectResult): string {
  switch (result.outcome) {
    case 'approved':
      return `Approved: ${productName(result.product.product)} is live.`;
    case 'rejected':
      return `Rejected ${productName(result.proposal)}.`;
    case 'not_pending':
      return `Already reviewed: this proposal is ${result.status}.`;
    case 'identity_conflict':
      return 'Its barcode or name lands on another catalogue product than the one it names, and an approval never renames a live product. Correct the identity, or reject it as a duplicate or wrong product.';
    case 'not_found':
      return 'This proposal no longer exists.';
    case 'implausible':
      return result.error;
  }
}

function blendClaim(stated: boolean | null): string {
  if (stated === null) return 'not read';
  return stated ? 'the pack claims a glucose + fructose blend' : 'the pack makes no glucose + fructose claim';
}

/** What the label reader left for the reviewer: its notes, the ingredients as printed, the blend claim. */
function ReaderNotes({ proposal }: { proposal: FuelProductProposal }) {
  return (
    <dl className="space-y-2 text-sm" data-testid="fuel-proposal-reading">
      <div>
        <dt className="text-xs text-on-surface-variant">Reader&apos;s notes</dt>
        <dd className="whitespace-pre-line text-on-surface" data-testid="fuel-proposal-notes">
          {proposal.notes ?? 'none'}
        </dd>
      </div>
      <div>
        <dt className="text-xs text-on-surface-variant">Ingredients as printed</dt>
        <dd className="whitespace-pre-line text-on-surface" data-testid="fuel-proposal-ingredients">
          {proposal.ingredients_text ?? 'not read'}
        </dd>
      </div>
      <div>
        <dt className="text-xs text-on-surface-variant">Multiple transportable carbohydrates</dt>
        <dd className="text-on-surface" data-testid="fuel-proposal-blend-claim">
          {blendClaim(proposal.multiple_transportable_stated)}
        </dd>
      </div>
    </dl>
  );
}

export default function ProposalDrawer({
  proposalId,
  onClose,
  onDone,
}: {
  proposalId: string;
  onClose: () => void;
  onDone: (message: string) => void;
}) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [edits, setEdits] = useState<Partial<Record<FactKey, FieldEdit>>>({});
  const [identityEdits, setIdentityEdits] = useState<IdentityEdits>({});
  const [reason, setReason] = useState<FuelRejectReason | ''>('');
  const [note, setNote] = useState('');
  const [confirmReject, setConfirmReject] = useState(false);
  const [problem, setProblem] = useState<{
    message: string;
    codes: string[];
  } | null>(null);

  const detail = useQuery({
    queryKey: QUERY_KEYS.adminFuelProducts.proposal(proposalId),
    queryFn: () => adminApi.getFuelProposal(proposalId),
  });

  const settle = (result: FuelApprovalResult | FuelRejectResult) => {
    if (result.outcome === 'approved' || result.outcome === 'rejected') {
      void queryClient.invalidateQueries({
        queryKey: QUERY_KEYS.adminFuelProducts.all,
      });
      onDone(outcomeMessage(result));
      return;
    }
    if (result.outcome === 'not_pending' || result.outcome === 'not_found') {
      void queryClient.invalidateQueries({
        queryKey: QUERY_KEYS.adminFuelProducts.all,
      });
    }
    setProblem({
      message: outcomeMessage(result),
      codes: result.outcome === 'implausible' ? result.violations.map((v) => `${v.code}: ${v.message}`) : [],
    });
  };
  const fail = (error: unknown) =>
    setProblem({
      message: describeApiError(error, { t, fallbackKey: 'errors.unknown' }),
      codes: [],
    });

  const approve = useMutation({
    mutationFn: (overrides: FuelFactOverrides) => adminApi.approveFuelProposal(proposalId, overrides),
    onSuccess: settle,
    onError: fail,
  });
  const reject = useMutation({
    mutationFn: () => adminApi.rejectFuelProposal(proposalId, reason as FuelRejectReason, note),
    onSuccess: (result) => {
      setConfirmReject(false);
      settle(result);
    },
    onError: (error) => {
      setConfirmReject(false);
      fail(error);
    },
  });

  const proposal = detail.data?.proposal;
  const built = useMemo(
    () => (proposal ? buildOverrides(proposal, edits, identityEdits) : null),
    [proposal, edits, identityEdits],
  );
  const editCount =
    built && 'overrides' in built
      ? Object.keys(built.overrides).length
      : Object.keys(edits).length + Object.keys(identityEdits).length;
  const busy = approve.isPending || reject.isPending;

  const renderBody = () => {
    if (detail.isPending) {
      return <div className="pierre-spinner h-5 w-5" role="status" aria-label={t('common.loading')} />;
    }
    if (detail.isError || !proposal) {
      return (
        <p role="alert" className="text-sm text-error">
          {describeApiError(detail.error, { t, fallbackKey: 'errors.unknown' })}
        </p>
      );
    }
    const pending = proposal.status === 'pending';
    const invalidField = built && 'invalid' in built ? built.invalid : null;
    return (
      <div className="space-y-6 px-4 pb-6">
        <div className="space-y-1 text-sm">
          <p className="text-on-surface">
            {formLabel(proposal.form)} · from {SOURCE_LABELS[proposal.source_kind] ?? proposal.source_kind}
            {proposal.gtin && ` · GTIN ${proposal.gtin}`}
          </p>
          {proposal.source_url && !isWebLink(proposal.source_url) && (
            <p className="break-all text-xs text-on-surface-variant" data-testid="fuel-proposal-source-text">
              Source: {proposal.source_url}
            </p>
          )}
          {proposal.source_url && isWebLink(proposal.source_url) && (
            <a
              href={proposal.source_url}
              target="_blank"
              rel="noopener noreferrer"
              className="break-all text-primary underline"
              data-testid="fuel-proposal-source-link"
            >
              {proposal.source_url}
            </a>
          )}
          {proposal.gate_flags.length > 0 && (
            <p className="text-xs text-warning" data-testid="fuel-proposal-gate-flags">
              Flagged when proposed: {proposal.gate_flags.join(', ')}
            </p>
          )}
          {proposal.status === 'rejected' && (
            <p className="text-xs text-on-surface-variant">
              Rejected as {proposal.reject_reason?.replace('_', ' ')}
              {proposal.reject_note && `: ${proposal.reject_note}`}
            </p>
          )}
        </div>

        <ProposalPhotos proposal={proposal} images={detail.data.images} />

        <Section title="What the reader saw" headingLevel={3}>
          <ReaderNotes proposal={proposal} />
        </Section>

        <Section
          title="Product"
          description={
            pending ? 'Correct what the reader named: it is what the catalogue files the facts under.' : undefined
          }
          headingLevel={3}
        >
          <IdentityFields
            proposal={proposal}
            edits={identityEdits}
            editable={pending}
            onEdit={(next) => {
              setProblem(null);
              setIdentityEdits(next);
            }}
          />
        </Section>

        <Section
          title="Facts"
          description={pending ? 'Change a value, or clear one the label does not print.' : undefined}
          headingLevel={3}
        >
          <ul className="divide-y divide-outline-variant">
            {FACT_FIELDS.map((field) => (
              <FactRow
                key={field.key}
                field={field}
                proposal={proposal}
                edit={edits[field.key]}
                editable={pending}
                onEdit={(edit) => {
                  setProblem(null);
                  setEdits((current) => {
                    const next = { ...current };
                    if (edit) next[field.key] = edit;
                    else delete next[field.key];
                    return next;
                  });
                }}
              />
            ))}
          </ul>
        </Section>

        {problem && (
          <div role="alert" className="text-sm text-error" data-testid="fuel-review-problem">
            <p>{problem.message}</p>
            {problem.codes.length > 0 && (
              <ul className="mt-1 list-disc pl-5 text-xs">
                {problem.codes.map((code) => (
                  <li key={code}>{code}</li>
                ))}
              </ul>
            )}
          </div>
        )}

        {pending && (
          <>
            <div className="flex flex-wrap gap-2">
              <Button
                type="button"
                variant="secondary"
                disabled={busy || editCount > 0}
                loading={approve.isPending && editCount === 0}
                onClick={() => approve.mutate({})}
              >
                Approve as proposed
              </Button>
              <Button
                type="button"
                disabled={busy || editCount === 0 || invalidField !== null}
                loading={approve.isPending && editCount > 0}
                onClick={() => {
                  if (built && 'overrides' in built) approve.mutate(built.overrides);
                }}
              >
                Save and approve
                {editCount > 0 ? ` (${editCount} edit${editCount === 1 ? '' : 's'})` : ''}
              </Button>
            </div>
            {invalidField && <p className="text-xs text-error">{fieldLabel(invalidField)} needs a value.</p>}

            <Section title="Reject" headingLevel={3}>
              <div className="space-y-3">
                <Select
                  label="Reason"
                  value={reason}
                  placeholder="Choose a reason"
                  options={REJECT_REASONS.map((r) => ({
                    value: r.value,
                    label: r.label,
                  }))}
                  onChange={(e) => setReason(e.target.value as FuelRejectReason | '')}
                />
                <Textarea
                  label="Note"
                  value={note}
                  maxLength={2000}
                  rows={3}
                  placeholder={reason === 'other' ? 'Required: what is wrong' : 'Optional'}
                  onChange={(e) => setNote(e.target.value)}
                />
                <Button
                  type="button"
                  variant="danger"
                  disabled={busy || reason === '' || (reason === 'other' && note.trim().length === 0)}
                  onClick={() => setConfirmReject(true)}
                >
                  Reject
                </Button>
              </div>
            </Section>
          </>
        )}
      </div>
    );
  };

  return (
    <Sheet
      side="right"
      width="full"
      title={proposal ? productName(proposal) : 'Fuel proposal'}
      onClose={onClose}
      data-testid="fuel-proposal-drawer"
    >
      {renderBody()}
      {confirmReject && (
        <ConfirmDialog
          isOpen
          variant="danger"
          title="Reject this proposal?"
          message={`Nothing from it reaches the catalogue. Reason: ${
            REJECT_REASONS.find((r) => r.value === reason)?.label ?? reason
          }.`}
          confirmLabel="Reject"
          isLoading={reject.isPending}
          onClose={() => setConfirmReject(false)}
          onConfirm={() => reject.mutate()}
        />
      )}
    </Sheet>
  );
}
