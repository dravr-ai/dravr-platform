// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Super-admin review of the global fuel-product bank — the proposal queue, a drawer to edit, approve or reject one, and the live catalogue
// ABOUTME: Each fact shows where it came from and can be kept, changed or cleared; the label image is served by the backend as a data URL

import { useMemo, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import { describeApiError } from '@pierre/ui-logic';
import { QUERY_KEYS } from '../constants/queryKeys';
import {
  adminApi,
  type FuelApprovalResult,
  type FuelFactOverrides,
  type FuelFacts,
  type FuelProductProposal,
  type FuelProposalStatus,
  type FuelRejectReason,
  type FuelRejectResult,
} from '../services/api/admin';
import { Badge, Button, ConfirmDialog, Input, Section, Select, Sheet, Tabs, Textarea } from './ui';

type FactKey = keyof FuelFacts;

/** Every fact the label can state, in the order the drawer lists them. */
const FACT_FIELDS: ReadonlyArray<{ key: FactKey; label: string; unit: string }> = [
  { key: 'serving_label', label: 'Serving as printed', unit: '' },
  { key: 'serving_g', label: 'Serving mass', unit: 'g' },
  { key: 'energy_kcal', label: 'Energy', unit: 'kcal' },
  { key: 'carbs_g', label: 'Carbohydrate', unit: 'g' },
  { key: 'sugars_g', label: 'of which sugars', unit: 'g' },
  { key: 'protein_g', label: 'Protein', unit: 'g' },
  { key: 'fat_g', label: 'Fat', unit: 'g' },
  { key: 'sodium_mg', label: 'Sodium', unit: 'mg' },
  { key: 'potassium_mg', label: 'Potassium', unit: 'mg' },
  { key: 'caffeine_mg', label: 'Caffeine', unit: 'mg' },
  { key: 'fluid_ml', label: 'Fluid', unit: 'ml' },
  { key: 'carbs_g_per_100g', label: 'Carbohydrate per 100 g', unit: 'g' },
  { key: 'sugars_g_per_100g', label: 'Sugars per 100 g', unit: 'g' },
  { key: 'protein_g_per_100g', label: 'Protein per 100 g', unit: 'g' },
  { key: 'fat_g_per_100g', label: 'Fat per 100 g', unit: 'g' },
  { key: 'sodium_mg_per_100g', label: 'Sodium per 100 g', unit: 'mg' },
  { key: 'salt_g_per_100g', label: 'Salt per 100 g', unit: 'g' },
  { key: 'glucose_fructose_ratio', label: 'Fructose per gram of glucose', unit: '' },
];

const STATUS_OPTIONS: ReadonlyArray<{ value: FuelProposalStatus; label: string }> = [
  { value: 'pending', label: 'Pending' },
  { value: 'approved', label: 'Approved' },
  { value: 'rejected', label: 'Rejected' },
];

const REJECT_REASONS: ReadonlyArray<{ value: FuelRejectReason; label: string }> = [
  { value: 'illegible', label: 'Label is illegible' },
  { value: 'wrong_product', label: 'Facts belong to another product' },
  { value: 'implausible', label: 'Facts are implausible' },
  { value: 'duplicate', label: 'Catalogue already holds it' },
  { value: 'not_fuel', label: 'Not a fuel product' },
  { value: 'other', label: 'Other (say what in the note)' },
];

const SOURCE_LABELS: Record<string, string> = {
  label_ocr: 'label OCR',
  athlete: 'athlete',
  admin_edit: 'operator edit',
  brand_page: 'brand page',
  shopify_ocr: 'Shopify label OCR',
  admin: 'operator',
};

/** One field's edit: untouched, a typed value, or cleared. */
type FieldEdit = { mode: 'set'; text: string } | { mode: 'clear' };

/** Whether a stored source URL is safe to open as a link: http(s) only, never `javascript:`. */
function isWebLink(url: string): boolean {
  return /^https?:\/\//i.test(url.trim());
}

function productName(p: { brand: string; name: string; variant: string }): string {
  return [p.brand, p.name, p.variant].filter((part) => part.trim().length > 0).join(' · ');
}

function formatFact(value: FuelFacts[FactKey], unit: string): string {
  if (value === undefined || value === null) return 'not stated';
  return unit ? `${value} ${unit}` : String(value);
}

/**
 * The overrides an approval sends, or the first field whose typed value is not
 * a number. A value equal to the proposed one is left out: it is not an edit.
 */
function buildOverrides(
  facts: FuelFacts,
  edits: Partial<Record<FactKey, FieldEdit>>,
): { overrides: FuelFactOverrides } | { invalid: FactKey } {
  const overrides: Record<string, string | number | null> = {};
  for (const { key } of FACT_FIELDS) {
    const edit = edits[key];
    if (!edit) continue;
    const proposed = facts[key];
    if (edit.mode === 'clear') {
      if (proposed !== undefined) overrides[key] = null;
      continue;
    }
    if (key === 'serving_label') {
      const text = edit.text.trim();
      if (text.length === 0) return { invalid: key };
      if (text !== proposed) overrides[key] = text;
      continue;
    }
    const value = Number(edit.text);
    if (edit.text.trim().length === 0 || !Number.isFinite(value)) return { invalid: key };
    if (value !== proposed) overrides[key] = value;
  }
  return { overrides: overrides as FuelFactOverrides };
}

function FactRow({
  field,
  proposal,
  edit,
  onEdit,
  editable,
}: {
  field: (typeof FACT_FIELDS)[number];
  proposal: FuelProductProposal;
  edit: FieldEdit | undefined;
  onEdit: (edit: FieldEdit | undefined) => void;
  editable: boolean;
}) {
  const proposed = proposal.facts[field.key];
  const provenance = proposal.provenance[field.key];
  const cleared = edit?.mode === 'clear';
  const text = edit?.mode === 'set' ? edit.text : proposed === undefined ? '' : String(proposed);
  const label = field.unit ? `${field.label} (${field.unit})` : field.label;
  const inputId = `fuel-fact-${field.key}`;

  return (
    <li className="py-2" data-testid={`fuel-fact-${field.key}`}>
      <div className="flex items-end gap-2">
        <div className="min-w-0 flex-1">
          {editable && !cleared ? (
            <Input
              id={inputId}
              label={label}
              size="sm"
              type={field.key === 'serving_label' ? 'text' : 'number'}
              inputMode={field.key === 'serving_label' ? 'text' : 'decimal'}
              step="any"
              value={text}
              placeholder="not stated"
              onChange={(e) => onEdit({ mode: 'set', text: e.target.value })}
            />
          ) : (
            <div>
              <p className="text-xs text-on-surface-variant">{label}</p>
              <p className="text-sm text-on-surface">
                {cleared ? <span className="line-through">{formatFact(proposed, field.unit)}</span> : formatFact(proposed, field.unit)}
                {cleared && <span className="ml-2 text-xs text-warning">cleared</span>}
              </p>
            </div>
          )}
        </div>
        {editable && (cleared || edit ? (
          <Button type="button" variant="tertiary" size="sm" onClick={() => onEdit(undefined)}>
            Undo
          </Button>
        ) : (
          proposed !== undefined && (
            <Button
              type="button"
              variant="tertiary"
              size="sm"
              aria-label={`Clear ${field.label}`}
              onClick={() => onEdit({ mode: 'clear' })}
            >
              Clear
            </Button>
          )
        ))}
      </div>
      <p className="mt-0.5 text-xs text-on-surface-variant">
        {provenance
          ? `From ${SOURCE_LABELS[provenance.source] ?? provenance.source}${
              provenance.confidence !== undefined ? `, confidence ${Math.round(provenance.confidence * 100)}%` : ''
            }`
          : 'No recorded source'}
        {edit && !cleared && ' · will be recorded as an operator edit'}
      </p>
    </li>
  );
}

function LabelImage({ proposal }: { proposal: FuelProductProposal }) {
  const image = useQuery({
    queryKey: QUERY_KEYS.adminFuelProducts.image(proposal.id),
    queryFn: () => adminApi.getFuelProposalSourceImage(proposal.id),
    enabled: proposal.has_source_image,
    staleTime: Infinity,
  });
  if (!proposal.has_source_image) {
    return <p className="text-sm text-on-surface-variant">No label image is stored with this proposal.</p>;
  }
  if (image.isPending) {
    return <div className="pierre-spinner h-5 w-5" role="status" aria-label="Loading the label image" />;
  }
  if (image.isError) {
    return (
      <p role="alert" className="text-sm text-error">
        The label image could not be loaded.
      </p>
    );
  }
  return (
    <img
      src={image.data}
      alt={`Label panel of ${productName(proposal)}`}
      className="max-h-80 w-full rounded-lg object-contain bg-surface-container-low"
      data-testid="fuel-proposal-image"
    />
  );
}

function outcomeMessage(result: FuelApprovalResult | FuelRejectResult): string {
  switch (result.outcome) {
    case 'approved':
      return `Approved: ${productName(result.product.product)} is live.`;
    case 'rejected':
      return `Rejected ${productName(result.proposal)}.`;
    case 'not_pending':
      return `Already reviewed: this proposal is ${result.status}.`;
    case 'identity_conflict':
      return 'Its barcode belongs to another catalogue product than the one it names. Reject it as a duplicate or wrong product.';
    case 'not_found':
      return 'This proposal no longer exists.';
    case 'implausible':
      return result.error;
  }
}

function ProposalDrawer({
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
  const [reason, setReason] = useState<FuelRejectReason | ''>('');
  const [note, setNote] = useState('');
  const [confirmReject, setConfirmReject] = useState(false);
  const [problem, setProblem] = useState<{ message: string; codes: string[] } | null>(null);

  const detail = useQuery({
    queryKey: QUERY_KEYS.adminFuelProducts.proposal(proposalId),
    queryFn: () => adminApi.getFuelProposal(proposalId),
  });

  const settle = (result: FuelApprovalResult | FuelRejectResult) => {
    if (result.outcome === 'approved' || result.outcome === 'rejected') {
      void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.adminFuelProducts.all });
      onDone(outcomeMessage(result));
      return;
    }
    if (result.outcome === 'not_pending' || result.outcome === 'not_found') {
      void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.adminFuelProducts.all });
    }
    setProblem({
      message: outcomeMessage(result),
      codes: result.outcome === 'implausible' ? result.violations.map((v) => `${v.code}: ${v.message}`) : [],
    });
  };

  const approve = useMutation({
    mutationFn: (overrides: FuelFactOverrides) => adminApi.approveFuelProposal(proposalId, overrides),
    onSuccess: settle,
    onError: (error) => setProblem({ message: describeApiError(error, { t, fallbackKey: 'errors.unknown' }), codes: [] }),
  });
  const reject = useMutation({
    mutationFn: () => adminApi.rejectFuelProposal(proposalId, reason as FuelRejectReason, note),
    onSuccess: (result) => {
      setConfirmReject(false);
      settle(result);
    },
    onError: (error) => {
      setConfirmReject(false);
      setProblem({ message: describeApiError(error, { t, fallbackKey: 'errors.unknown' }), codes: [] });
    },
  });

  const proposal = detail.data?.proposal;
  const built = useMemo(() => (proposal ? buildOverrides(proposal.facts, edits) : null), [proposal, edits]);
  const editCount =
    built && 'overrides' in built ? Object.keys(built.overrides).length : Object.keys(edits).length;
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
            {proposal.form.replace('_', ' ')} · from {SOURCE_LABELS[proposal.source_kind] ?? proposal.source_kind}
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

        <LabelImage proposal={proposal} />

        <Section title="Facts" description={pending ? 'Change a value, or clear one the label does not print.' : undefined} headingLevel={3}>
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
                Save and approve{editCount > 0 ? ` (${editCount} edit${editCount === 1 ? '' : 's'})` : ''}
              </Button>
            </div>
            {invalidField && (
              <p className="text-xs text-error">
                {FACT_FIELDS.find((f) => f.key === invalidField)?.label} needs a value, or clear it.
              </p>
            )}

            <Section title="Reject" headingLevel={3}>
              <div className="space-y-3">
                <Select
                  label="Reason"
                  value={reason}
                  placeholder="Choose a reason"
                  options={REJECT_REASONS.map((r) => ({ value: r.value, label: r.label }))}
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

function ProposalQueue({ onOpen }: { onOpen: (id: string) => void }) {
  const { t } = useTranslation();
  const [status, setStatus] = useState<FuelProposalStatus>('pending');
  const proposals = useQuery({
    queryKey: QUERY_KEYS.adminFuelProducts.proposals(status),
    queryFn: () => adminApi.listFuelProposals(status, 100),
  });

  const renderBody = () => {
    if (proposals.isPending) {
      return <div className="pierre-spinner h-5 w-5" role="status" aria-label={t('common.loading')} />;
    }
    if (proposals.isError) {
      return (
        <p role="alert" className="text-sm text-error">
          {describeApiError(proposals.error, { t, fallbackKey: 'errors.unknown' })}
        </p>
      );
    }
    if (proposals.data.proposals.length === 0) {
      return (
        <p className="text-sm text-on-surface-variant" data-testid="fuel-proposals-empty">
          No {status} proposals.
        </p>
      );
    }
    return (
      <ul className="divide-y divide-outline-variant" data-testid="fuel-proposals-list">
        {proposals.data.proposals.map((proposal) => (
          <li key={proposal.id} className="flex items-center justify-between gap-3 py-3">
            <div className="min-w-0">
              <p className="truncate text-sm font-medium text-on-surface">{productName(proposal)}</p>
              <p className="text-xs text-on-surface-variant">
                {proposal.form.replace('_', ' ')} · {SOURCE_LABELS[proposal.source_kind] ?? proposal.source_kind} ·{' '}
                {proposal.facts.carbs_g !== undefined ? `${proposal.facts.carbs_g} g carbs` : 'carbs not stated'} ·{' '}
                {new Date(proposal.created_at).toLocaleDateString()}
              </p>
            </div>
            <div className="flex shrink-0 items-center gap-2">
              {proposal.gate_flags.length > 0 && (
                <Badge variant="warning">
                  {proposal.gate_flags.length} flag{proposal.gate_flags.length === 1 ? '' : 's'}
                </Badge>
              )}
              <Button
                type="button"
                variant="secondary"
                size="sm"
                onClick={() => onOpen(proposal.id)}
                data-testid={`fuel-proposal-open-${proposal.id}`}
              >
                {status === 'pending' ? 'Review' : 'View'}
              </Button>
            </div>
          </li>
        ))}
      </ul>
    );
  };

  return (
    <Section
      title="Proposals"
      description="Label facts waiting for an operator. Nothing reaches athletes until it is approved."
      actions={
        <Select
          aria-label="Proposal status"
          size="sm"
          value={status}
          options={STATUS_OPTIONS.map((o) => ({ value: o.value, label: o.label }))}
          onChange={(e) => setStatus(e.target.value as FuelProposalStatus)}
        />
      }
      data-testid="fuel-proposals"
    >
      {renderBody()}
    </Section>
  );
}

function LiveCatalogue() {
  const { t } = useTranslation();
  const catalogue = useQuery({
    queryKey: QUERY_KEYS.adminFuelProducts.catalogue(),
    queryFn: () => adminApi.listFuelCatalogue(100),
  });

  const renderBody = () => {
    if (catalogue.isPending) {
      return <div className="pierre-spinner h-5 w-5" role="status" aria-label={t('common.loading')} />;
    }
    if (catalogue.isError) {
      return (
        <p role="alert" className="text-sm text-error">
          {describeApiError(catalogue.error, { t, fallbackKey: 'errors.unknown' })}
        </p>
      );
    }
    if (catalogue.data.products.length === 0) {
      return (
        <p className="text-sm text-on-surface-variant" data-testid="fuel-catalogue-empty">
          The catalogue is empty: approve a proposal to put a product in front of athletes.
        </p>
      );
    }
    return (
      <ul className="divide-y divide-outline-variant" data-testid="fuel-catalogue-list">
        {catalogue.data.products.map((row) => {
          const facts = row.current.facts;
          return (
            <li key={row.product.id} className="py-3">
              <p className="text-sm font-medium text-on-surface">{productName(row.product)}</p>
              <p className="text-xs text-on-surface-variant">
                {row.product.form.replace('_', ' ')}
                {row.product.gtin && ` · GTIN ${row.product.gtin}`} · {formatFact(facts.carbs_g, 'g')} carbs per serving
                {facts.glucose_fructose_ratio !== undefined && ` · ratio 1:${facts.glucose_fructose_ratio}`} ·{' '}
                {row.version_count} version{row.version_count === 1 ? '' : 's'}
                {row.current.approved_at && ` · current since ${new Date(row.current.approved_at).toLocaleDateString()}`}
              </p>
            </li>
          );
        })}
      </ul>
    );
  };

  return (
    <Section
      title="Live catalogue"
      description="What athletes can carry today, with each product's current label facts."
      data-testid="fuel-catalogue"
    >
      {renderBody()}
    </Section>
  );
}

export default function FuelProductReviewTab() {
  const [view, setView] = useState<'queue' | 'catalogue'>('queue');
  const [openId, setOpenId] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  return (
    <div className="space-y-6" data-testid="fuel-product-review">
      <Tabs
        variant="underline"
        size="sm"
        activeTab={view}
        onChange={(id) => setView(id === 'catalogue' ? 'catalogue' : 'queue')}
        tabs={[
          { id: 'queue', label: 'Review queue' },
          { id: 'catalogue', label: 'Live catalogue' },
        ]}
      />
      {notice && (
        <p role="status" className="text-sm text-success" data-testid="fuel-review-notice">
          {notice}
        </p>
      )}
      {view === 'queue' ? (
        <ProposalQueue
          onOpen={(id) => {
            setNotice(null);
            setOpenId(id);
          }}
        />
      ) : (
        <LiveCatalogue />
      )}
      {openId && (
        <ProposalDrawer
          key={openId}
          proposalId={openId}
          onClose={() => setOpenId(null)}
          onDone={(message) => {
            setOpenId(null);
            setNotice(message);
          }}
        />
      )}
    </div>
  );
}
