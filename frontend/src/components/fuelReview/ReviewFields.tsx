// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The fuel-proposal drawer's editable rows — the identity (brand, name, variant, form, barcode) and each label fact with where it came from
// ABOUTME: Each row keeps, changes or clears its value; the overrides are built from these edits by fuelReviewModel

import type { FuelFieldProvenance, FuelForm, FuelProductProposal } from '../../services/api/admin';
import { Button, Input, Select } from '../ui';
import {
  FACT_FIELDS,
  FORM_OPTIONS,
  SOURCE_LABELS,
  formLabel,
  formatFact,
  type FieldEdit,
  type IdentityEdits,
} from './fuelReviewModel';

function provenanceText(provenance: FuelFieldProvenance | undefined): string {
  if (!provenance) return 'No recorded source';
  const source = SOURCE_LABELS[provenance.source] ?? provenance.source;
  const confidence =
    provenance.confidence !== undefined ? `, confidence ${Math.round(provenance.confidence * 100)}%` : '';
  const operator = provenance.operator_user_id ? ` by operator ${provenance.operator_user_id.slice(0, 8)}` : '';
  return `From ${source}${confidence}${operator}`;
}

export function FactRow({
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
  const cleared = edit?.mode === 'clear';
  const text = edit?.mode === 'set' ? edit.text : proposed === undefined ? '' : String(proposed);
  const label = field.unit ? `${field.label} (${field.unit})` : field.label;

  return (
    <li className="py-2" data-testid={`fuel-fact-${field.key}`}>
      <div className="flex items-end gap-2">
        <div className="min-w-0 flex-1">
          {editable && !cleared ? (
            <Input
              id={`fuel-fact-${field.key}`}
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
                {cleared ? (
                  <span className="line-through">{formatFact(proposed, field.unit)}</span>
                ) : (
                  formatFact(proposed, field.unit)
                )}
                {cleared && <span className="ml-2 text-xs text-warning">cleared</span>}
              </p>
            </div>
          )}
        </div>
        {editable &&
          (cleared || edit ? (
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
        {provenanceText(proposal.provenance[field.key])}
        {edit && !cleared && ' · will be recorded as your edit'}
      </p>
    </li>
  );
}

/**
 * The identity a proposal files its facts under. Brand, name and form can
 * be changed but not emptied; a variant left blank means none; the barcode
 * can be typed as printed or cleared.
 */
export function IdentityFields({
  proposal,
  edits,
  onEdit,
  editable,
}: {
  proposal: FuelProductProposal;
  edits: IdentityEdits;
  onEdit: (next: IdentityEdits) => void;
  editable: boolean;
}) {
  if (!editable) {
    return (
      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm" data-testid="fuel-identity">
        <dt className="text-on-surface-variant">Brand</dt>
        <dd className="text-on-surface">{proposal.brand}</dd>
        <dt className="text-on-surface-variant">Name</dt>
        <dd className="text-on-surface">{proposal.name}</dd>
        <dt className="text-on-surface-variant">Variant</dt>
        <dd className="text-on-surface">{proposal.variant || 'none'}</dd>
        <dt className="text-on-surface-variant">Form</dt>
        <dd className="text-on-surface">{formLabel(proposal.form)}</dd>
        <dt className="text-on-surface-variant">Barcode</dt>
        <dd className="text-on-surface">{proposal.gtin ?? 'none'}</dd>
      </dl>
    );
  }
  const set = (patch: IdentityEdits) => onEdit({ ...edits, ...patch });
  const gtinCleared = edits.gtin === null;
  return (
    <div className="grid gap-3 sm:grid-cols-2" data-testid="fuel-identity">
      <Input
        id="fuel-identity-brand"
        label="Brand"
        size="sm"
        value={edits.brand ?? proposal.brand}
        onChange={(e) => set({ brand: e.target.value })}
      />
      <Input
        id="fuel-identity-name"
        label="Name"
        size="sm"
        value={edits.name ?? proposal.name}
        onChange={(e) => set({ name: e.target.value })}
      />
      <Input
        id="fuel-identity-variant"
        label="Variant"
        size="sm"
        value={edits.variant ?? proposal.variant}
        placeholder="none"
        onChange={(e) => set({ variant: e.target.value })}
      />
      <Select
        label="Form"
        size="sm"
        value={edits.form ?? proposal.form}
        options={FORM_OPTIONS.map((o) => ({ value: o.value, label: o.label }))}
        onChange={(e) => set({ form: e.target.value as FuelForm })}
      />
      <div className="flex items-end gap-2 sm:col-span-2">
        <div className="min-w-0 flex-1">
          {gtinCleared ? (
            <div>
              <p className="text-xs text-on-surface-variant">Barcode</p>
              <p className="text-sm text-on-surface">
                <span className="line-through">{proposal.gtin}</span>
                <span className="ml-2 text-xs text-warning">cleared</span>
              </p>
            </div>
          ) : (
            <Input
              id="fuel-identity-gtin"
              label="Barcode (UPC, EAN or GTIN-14)"
              size="sm"
              inputMode="numeric"
              value={edits.gtin ?? proposal.gtin ?? ''}
              placeholder="none"
              onChange={(e) => {
                const typed = e.target.value;
                // Emptying a field that had no barcode leaves it untouched.
                if (typed.trim() === '' && proposal.gtin === null) {
                  const next = { ...edits };
                  delete next.gtin;
                  onEdit(next);
                } else {
                  set({ gtin: typed });
                }
              }}
            />
          )}
        </div>
        {gtinCleared ? (
          <Button
            type="button"
            variant="tertiary"
            size="sm"
            onClick={() => {
              const next = { ...edits };
              delete next.gtin;
              onEdit(next);
            }}
          >
            Undo
          </Button>
        ) : (
          proposal.gtin !== null && (
            <Button
              type="button"
              variant="tertiary"
              size="sm"
              aria-label="Clear Barcode"
              onClick={() => set({ gtin: null })}
            >
              Clear
            </Button>
          )
        )}
      </div>
    </div>
  );
}
