// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The fuel-product review console's vocabulary and pure helpers — fact and identity fields, labels, and the overrides an approval sends
// ABOUTME: An edit equal to the proposed value is left out, so a form that sends every field back never relabels the label's own values as operator edits

import type {
  FuelFactOverrides,
  FuelFacts,
  FuelForm,
  FuelProductProposal,
  FuelProposalStatus,
  FuelRejectReason,
} from '../../services/api/admin';

export type FactKey = keyof FuelFacts;

/** Every fact the label can state, in the order the drawer lists them. */
export const FACT_FIELDS: ReadonlyArray<{
  key: FactKey;
  label: string;
  unit: string;
}> = [
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
  {
    key: 'glucose_fructose_ratio',
    label: 'Fructose per gram of glucose',
    unit: '',
  },
];

/** Every form the catalogue stores, in its stored-list order. */
export const FORM_OPTIONS: ReadonlyArray<{ value: FuelForm; label: string }> = [
  { value: 'gel', label: 'Gel' },
  { value: 'drink_mix', label: 'Drink mix' },
  { value: 'bar', label: 'Bar' },
  { value: 'chew', label: 'Chew' },
  { value: 'capsule', label: 'Capsule' },
  { value: 'tablet', label: 'Tablet' },
  { value: 'puree', label: 'Puree' },
  { value: 'waffle', label: 'Waffle' },
  { value: 'other', label: 'Other' },
];

export const STATUS_OPTIONS: ReadonlyArray<{
  value: FuelProposalStatus;
  label: string;
}> = [
  { value: 'pending', label: 'Pending' },
  { value: 'approved', label: 'Approved' },
  { value: 'rejected', label: 'Rejected' },
];

export const REJECT_REASONS: ReadonlyArray<{
  value: FuelRejectReason;
  label: string;
}> = [
  { value: 'illegible', label: 'Label is illegible' },
  { value: 'wrong_product', label: 'Facts belong to another product' },
  { value: 'implausible', label: 'Facts are implausible' },
  { value: 'duplicate', label: 'Catalogue already holds it' },
  { value: 'not_fuel', label: 'Not a fuel product' },
  { value: 'other', label: 'Other (say what in the note)' },
];

export const SOURCE_LABELS: Record<string, string> = {
  label_ocr: 'label OCR',
  athlete: 'athlete',
  admin_edit: 'operator edit',
  brand_page: 'brand page',
  shopify_ocr: 'Shopify label OCR',
  admin: 'operator',
};

/** Rows one page of the queue or the catalogue shows. */
export const PAGE_SIZE = 50;

/** One fact's edit: a typed value, or cleared. Absent means untouched. */
export type FieldEdit = { mode: 'set'; text: string } | { mode: 'clear' };

/** The identity fields an operator can correct. */
export type IdentityKey = 'brand' | 'name' | 'variant' | 'form' | 'gtin';

/** Typed identity values; a key left out is untouched. A `null` GTIN is cleared. */
export type IdentityEdits = Partial<Record<Exclude<IdentityKey, 'gtin'>, string>> & { gtin?: string | null };

export function formLabel(form: FuelForm): string {
  return FORM_OPTIONS.find((option) => option.value === form)?.label ?? form;
}

/** Whether a stored source URL is safe to open as a link: http(s) only, never `javascript:`. */
export function isWebLink(url: string): boolean {
  return /^https?:\/\//i.test(url.trim());
}

export function productName(p: { brand: string; name: string; variant: string }): string {
  return [p.brand, p.name, p.variant].filter((part) => part.trim().length > 0).join(' · ');
}

export function formatFact(value: FuelFacts[FactKey], unit: string): string {
  if (value === undefined || value === null) return 'not stated';
  return unit ? `${value} ${unit}` : String(value);
}

/** The identity edits an approval sends, or the first field left blank that must not be. */
function identityOverrides(
  proposal: FuelProductProposal,
  edits: IdentityEdits,
): { overrides: FuelFactOverrides } | { invalid: IdentityKey } {
  const overrides: FuelFactOverrides = {};
  for (const key of ['brand', 'name'] as const) {
    const typed = edits[key];
    if (typed === undefined) continue;
    const text = typed.trim();
    if (text.length === 0) return { invalid: key };
    if (text !== proposal[key]) overrides[key] = text;
  }
  if (edits.variant !== undefined) {
    const text = edits.variant.trim();
    if (text !== proposal.variant) overrides.variant = text.length === 0 ? null : text;
  }
  if (edits.form !== undefined && edits.form !== proposal.form) {
    overrides.form = edits.form as FuelForm;
  }
  if (edits.gtin === null) {
    if (proposal.gtin !== null) overrides.gtin = null;
  } else if (edits.gtin !== undefined) {
    const text = edits.gtin.trim();
    if (text.length === 0) return { invalid: 'gtin' };
    // The server normalizes a printed barcode; one that is already the
    // stored GTIN-14, spaces aside, is not an edit.
    if (text.replace(/[\s-]/g, '') !== proposal.gtin) overrides.gtin = text;
  }
  return { overrides };
}

/**
 * The overrides an approval sends, or the first field whose typed value is
 * refused before it is sent: a blank brand, name, GTIN or serving label, or
 * a fact that is not a number. A value equal to the proposed one is left
 * out: it is not an edit.
 */
export function buildOverrides(
  proposal: FuelProductProposal,
  factEdits: Partial<Record<FactKey, FieldEdit>>,
  identityEdits: IdentityEdits,
): { overrides: FuelFactOverrides } | { invalid: FactKey | IdentityKey } {
  const identity = identityOverrides(proposal, identityEdits);
  if ('invalid' in identity) return identity;
  const overrides: Record<string, string | number | null> = {
    ...identity.overrides,
  };
  const facts = proposal.facts;
  for (const { key } of FACT_FIELDS) {
    const edit = factEdits[key];
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

/** The label a field is named by in a message. */
export function fieldLabel(key: FactKey | IdentityKey): string {
  const identity: Record<IdentityKey, string> = {
    brand: 'Brand',
    name: 'Name',
    variant: 'Variant',
    form: 'Form',
    gtin: 'Barcode',
  };
  if (key in identity) return identity[key as IdentityKey];
  return FACT_FIELDS.find((f) => f.key === key)?.label ?? key;
}
