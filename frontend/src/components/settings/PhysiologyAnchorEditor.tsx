// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The form that sets one anchor — the athlete's own in Settings › Physiology, or a coached athlete's from Group info
// ABOUTME: A speed is typed as the pane shows it, m:ss per km or mi on the run and per 100 m in the pool (carnet#1561)

import { useState, type FormEvent } from 'react';
import { useTranslation } from '@pierre/i18n';
import {
  ANCHOR_CONFIDENCE_LABEL_KEY,
  ANCHOR_KIND_LABEL_KEY,
  PHYSIOLOGY_ANCHOR_LABEL_KEY,
  PHYSIOLOGY_EDIT_KEY,
  PHYSIOLOGY_METHOD_MAX_CHARS,
  SPORT_FAMILY_LABEL_KEY,
} from '@pierre/shared-constants';
import type { AnchorConfidence, AnchorKind, OfferedFamily, SportAnchorName, SportFamily } from '@pierre/shared-types';
import {
  anchorEntryUnit,
  describeApiError,
  initialAnchorForm,
  newAnchorForm,
  paceDistanceFor,
  readAnchorForm,
  useDistanceUnit,
  type AnchorForm,
  type AnchorFormProblem,
  type AnchorFormTarget,
  type AnchorSave,
} from '@pierre/ui-logic';
import { Button, Input, Modal, Select } from '../ui';

interface PhysiologyAnchorEditorProps {
  /** The anchor being set; `null` keeps the editor closed. */
  target: AnchorFormTarget | null;
  /** Every anchor each family can hold, as the server lists them. */
  offered: OfferedFamily[];
  onClose: () => void;
  /** Save one anchor; resolves once the server answered with the anchors re-read. */
  onSave: (save: AnchorSave) => Promise<unknown>;
  /**
   * Draw the form in place rather than in its own dialog: inside a view that
   * is already a dialog, such as the coach's view of one athlete.
   */
  inline?: boolean;
}

const CONFIDENCES: readonly AnchorConfidence[] = ['low', 'medium', 'high'];
const KINDS: readonly AnchorKind[] = ['measured', 'estimated'];

/**
 * One anchor's form in a dialog. On a new anchor the editor picks the sport
 * and the anchor first, from what the server says each sport can hold; on a
 * stored one they are fixed. How the value was found is always asked, never
 * assumed: Save waits for the answer.
 */
export default function PhysiologyAnchorEditor({ target, offered, onClose, onSave, inline = false }: PhysiologyAnchorEditorProps) {
  const units = useDistanceUnit();
  if (target === null) return null;
  const form = initialAnchorForm(target, offered, units);
  if (form === null) return null;
  // Each opening starts from its target's form: a new key mounts a fresh one.
  const key = target.mode === 'edit' ? `${target.sport}-${target.anchor.anchor}` : 'add';
  return (
    <EditorDialog
      key={key}
      target={target}
      initial={form}
      offered={offered}
      onClose={onClose}
      onSave={onSave}
      inline={inline}
    />
  );
}

interface EditorDialogProps extends Omit<PhysiologyAnchorEditorProps, 'target'> {
  target: AnchorFormTarget;
  initial: AnchorForm;
}

function EditorDialog({ target, initial, offered, onClose, onSave, inline }: EditorDialogProps) {
  const { t } = useTranslation();
  const units = useDistanceUnit();
  const [form, setForm] = useState<AnchorForm>(initial);
  const [problem, setProblem] = useState<AnchorFormProblem | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const per = form.unit === 'metres_per_second' ? paceDistanceFor(form.sport, units) : null;
  const fieldError = (field: AnchorFormProblem['field']) =>
    problem?.field === field && problem.reason !== 'kindRequired' ? t(PHYSIOLOGY_EDIT_KEY[problem.reason]) : undefined;
  const update = (patch: Partial<AnchorForm>) => {
    setForm({ ...form, ...patch });
    setProblem(null);
  };
  const familyAnchors = offered.find((entry) => entry.sport === form.sport)?.anchors ?? [];

  const chooseSport = (sport: SportFamily) => {
    const first = offered.find((entry) => entry.sport === sport)?.anchors[0];
    if (first) setForm(newAnchorForm(sport, first.anchor, first.unit));
    setProblem(null);
  };
  const chooseAnchor = (anchor: SportAnchorName) => {
    const offeredAnchor = familyAnchors.find((entry) => entry.anchor === anchor);
    if (offeredAnchor) setForm({ ...newAnchorForm(form.sport, anchor, offeredAnchor.unit), kind: form.kind });
    setProblem(null);
  };

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const read = readAnchorForm(form, units);
    if (!read.ok) {
      setProblem(read.problem);
      return;
    }
    setSaving(true);
    setSaveError(null);
    try {
      await onSave({ sport: form.sport, anchor: form.anchor, edit: read.edit });
      onClose();
    } catch (error) {
      setSaveError(describeApiError(error, { t, fallbackKey: PHYSIOLOGY_EDIT_KEY.saveFailed }));
    } finally {
      setSaving(false);
    }
  };

  const title = target.mode === 'edit' ? t(PHYSIOLOGY_EDIT_KEY.titleEdit) : t(PHYSIOLOGY_EDIT_KEY.add);

  const body = (
    <form className="space-y-4" onSubmit={(event) => void submit(event)} data-testid="physiology-editor" noValidate>
      {target.mode === 'edit' ? (
        <p className="text-sm font-medium text-on-surface" data-testid="physiology-editor-anchor">
          {t(PHYSIOLOGY_ANCHOR_LABEL_KEY[form.anchor])} · {t(SPORT_FAMILY_LABEL_KEY[form.sport])}
        </p>
      ) : (
        <div className="grid gap-4 sm:grid-cols-2">
          <Select
            label={t(PHYSIOLOGY_EDIT_KEY.sport)}
            value={form.sport}
            onChange={(event) => chooseSport(event.target.value as SportFamily)}
            options={offered.map((entry) => ({ value: entry.sport, label: t(SPORT_FAMILY_LABEL_KEY[entry.sport]) }))}
            data-testid="physiology-editor-sport"
          />
          <Select
            label={t(PHYSIOLOGY_EDIT_KEY.anchor)}
            value={form.anchor}
            onChange={(event) => chooseAnchor(event.target.value as SportAnchorName)}
            options={familyAnchors.map((entry) => ({
              value: entry.anchor,
              label: t(PHYSIOLOGY_ANCHOR_LABEL_KEY[entry.anchor]),
            }))}
            data-testid="physiology-editor-anchor-choice"
          />
        </div>
      )}
      <Input
        label={t(PHYSIOLOGY_EDIT_KEY.value, { unit: anchorEntryUnit(t, form.unit, per) })}
        value={form.value}
        onChange={(event) => update({ value: event.target.value })}
        inputMode={per === null ? 'decimal' : 'text'}
        error={fieldError('value')}
        data-testid="physiology-editor-value"
      />
      <div className="grid gap-4 sm:grid-cols-2">
        <Input
          label={t(PHYSIOLOGY_EDIT_KEY.rangeLow)}
          value={form.low}
          onChange={(event) => update({ low: event.target.value })}
          error={fieldError('low')}
          data-testid="physiology-editor-low"
        />
        <Input
          label={t(PHYSIOLOGY_EDIT_KEY.rangeHigh)}
          value={form.high}
          onChange={(event) => update({ high: event.target.value })}
          error={fieldError('high')}
          data-testid="physiology-editor-high"
        />
      </div>
      <Select
        label={t(PHYSIOLOGY_EDIT_KEY.kind)}
        value={form.kind ?? ''}
        placeholder=" "
        onChange={(event) => update({ kind: event.target.value as AnchorKind })}
        options={KINDS.map((kind) => ({ value: kind, label: t(ANCHOR_KIND_LABEL_KEY[kind]) }))}
        data-testid="physiology-editor-kind"
      />
      <Select
        label={t(PHYSIOLOGY_EDIT_KEY.confidence)}
        helpText={t(PHYSIOLOGY_EDIT_KEY.confidenceHint)}
        value={form.confidence}
        onChange={(event) => update({ confidence: event.target.value as AnchorConfidence })}
        options={CONFIDENCES.map((confidence) => ({
          value: confidence,
          label: t(ANCHOR_CONFIDENCE_LABEL_KEY[confidence]),
        }))}
        data-testid="physiology-editor-confidence"
      />
      <Input
        label={t(PHYSIOLOGY_EDIT_KEY.method)}
        value={form.method}
        onChange={(event) => update({ method: event.target.value })}
        maxLength={PHYSIOLOGY_METHOD_MAX_CHARS}
        data-testid="physiology-editor-method"
      />
      <Input
        type="date"
        label={t(PHYSIOLOGY_EDIT_KEY.measuredOn)}
        value={form.measuredOn}
        onChange={(event) => update({ measuredOn: event.target.value })}
        data-testid="physiology-editor-date"
      />
      {saveError !== null && (
        <p role="alert" className="text-sm text-error" data-testid="physiology-editor-error">
          {saveError}
        </p>
      )}
      <div className="flex justify-end gap-2">
        <Button type="button" variant="outline" onClick={onClose} data-testid="physiology-editor-cancel">
          {t(PHYSIOLOGY_EDIT_KEY.cancel)}
        </Button>
        <Button type="submit" loading={saving} disabled={form.kind === null || saving} data-testid="physiology-editor-save">
          {saving ? t(PHYSIOLOGY_EDIT_KEY.saving) : t(PHYSIOLOGY_EDIT_KEY.save)}
        </Button>
      </div>
    </form>
  );

  if (inline) {
    return (
      <section className="space-y-4" aria-label={title} data-testid="physiology-editor-inline">
        <h3 className="font-sans text-sm font-semibold text-on-surface">{title}</h3>
        {body}
      </section>
    );
  }
  return (
    <Modal isOpen onClose={onClose} title={title} size="md">
      {body}
    </Modal>
  );
}
