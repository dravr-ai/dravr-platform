// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The phone's form that sets one anchor — the athlete's own in Settings › Physiology, or a coached athlete's from Group info
// ABOUTME: A speed is typed as the pane shows it, m:ss per km or mi on the run and per 100 m in the pool (carnet#1561)

import React, { useState } from 'react';
import { Text, View } from 'react-native';
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
import { Button, Input, TextTabs } from './ui';

export interface PhysiologyAnchorFormProps {
  target: AnchorFormTarget;
  /** Every anchor each family can hold, as the server lists them. */
  offered: OfferedFamily[];
  /** Leave the form without saving, or once it saved. */
  onDone: () => void;
  /** Save one anchor; resolves once the server answered with the anchors re-read. */
  onSave: (save: AnchorSave) => Promise<unknown>;
}

const CONFIDENCES: readonly AnchorConfidence[] = ['low', 'medium', 'high'];
const KINDS: readonly AnchorKind[] = ['measured', 'estimated'];

/**
 * One anchor's form. On a new anchor the editor picks the sport and the
 * anchor first, from what the server says each sport can hold; on a stored
 * one they are fixed. How the value was found is always asked, never assumed:
 * Save waits for the answer.
 */
export function PhysiologyAnchorForm({ target, offered, onDone, onSave }: PhysiologyAnchorFormProps) {
  const { t } = useTranslation();
  const units = useDistanceUnit();
  const [form, setForm] = useState<AnchorForm | null>(() => initialAnchorForm(target, offered, units));
  const [problem, setProblem] = useState<AnchorFormProblem | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  if (form === null) return null;

  const per = form.unit === 'metres_per_second' ? paceDistanceFor(form.sport, units) : null;
  const fieldError = (field: AnchorFormProblem['field']) =>
    problem?.field === field && problem.reason !== 'kindRequired' ? t(PHYSIOLOGY_EDIT_KEY[problem.reason]) : undefined;
  const update = (patch: Partial<AnchorForm>) => {
    setForm({ ...form, ...patch });
    setProblem(null);
  };
  const familyAnchors = offered.find((entry) => entry.sport === form.sport)?.anchors ?? [];

  const chooseSport = (sport: string) => {
    const first = offered.find((entry) => entry.sport === sport)?.anchors[0];
    if (first) setForm(newAnchorForm(sport as SportFamily, first.anchor, first.unit));
    setProblem(null);
  };
  const chooseAnchor = (anchor: string) => {
    const offeredAnchor = familyAnchors.find((entry) => entry.anchor === anchor);
    if (offeredAnchor) {
      setForm({ ...newAnchorForm(form.sport, anchor as SportAnchorName, offeredAnchor.unit), kind: form.kind });
    }
    setProblem(null);
  };

  const submit = async () => {
    const read = readAnchorForm(form, units);
    if (!read.ok) {
      setProblem(read.problem);
      return;
    }
    setSaving(true);
    setSaveError(null);
    try {
      await onSave({ sport: form.sport, anchor: form.anchor, edit: read.edit });
      setSaving(false);
      onDone();
    } catch (error) {
      setSaving(false);
      setSaveError(describeApiError(error, { t, fallbackKey: PHYSIOLOGY_EDIT_KEY.saveFailed }));
    }
  };

  const numericKeyboard = per === null ? 'decimal-pad' : 'numbers-and-punctuation';

  return (
    <View className="gap-4" testID="physiology-editor">
      <Text className="text-lg font-bold text-text-primary">
        {target.mode === 'edit' ? t(PHYSIOLOGY_EDIT_KEY.titleEdit) : t(PHYSIOLOGY_EDIT_KEY.add)}
      </Text>
      {target.mode === 'edit' ? (
        <Text className="text-base font-medium text-text-primary" testID="physiology-editor-anchor">
          {t(PHYSIOLOGY_ANCHOR_LABEL_KEY[form.anchor])} · {t(SPORT_FAMILY_LABEL_KEY[form.sport])}
        </Text>
      ) : (
        <View className="gap-3">
          <Text className="text-sm text-text-secondary">{t(PHYSIOLOGY_EDIT_KEY.sport)}</Text>
          <TextTabs
            items={offered.map((entry) => ({ key: entry.sport, label: t(SPORT_FAMILY_LABEL_KEY[entry.sport]) }))}
            value={form.sport}
            onChange={chooseSport}
            testID="physiology-editor-sport"
          />
          <Text className="text-sm text-text-secondary">{t(PHYSIOLOGY_EDIT_KEY.anchor)}</Text>
          <TextTabs
            items={familyAnchors.map((entry) => ({
              key: entry.anchor,
              label: t(PHYSIOLOGY_ANCHOR_LABEL_KEY[entry.anchor]),
            }))}
            value={form.anchor}
            onChange={chooseAnchor}
            testID="physiology-editor-anchor-choice"
          />
        </View>
      )}
      <Input
        label={t(PHYSIOLOGY_EDIT_KEY.value, { unit: anchorEntryUnit(t, form.unit, per) })}
        value={form.value}
        onChangeText={(value) => update({ value })}
        keyboardType={numericKeyboard}
        error={fieldError('value')}
        testID="physiology-editor-value"
      />
      <View className="flex-row gap-3">
        <View className="flex-1">
          <Input
            label={t(PHYSIOLOGY_EDIT_KEY.rangeLow)}
            value={form.low}
            onChangeText={(low) => update({ low })}
            keyboardType={numericKeyboard}
            error={fieldError('low')}
            testID="physiology-editor-low"
          />
        </View>
        <View className="flex-1">
          <Input
            label={t(PHYSIOLOGY_EDIT_KEY.rangeHigh)}
            value={form.high}
            onChangeText={(high) => update({ high })}
            keyboardType={numericKeyboard}
            error={fieldError('high')}
            testID="physiology-editor-high"
          />
        </View>
      </View>
      <View className="gap-2">
        <Text className="text-sm text-text-secondary">{t(PHYSIOLOGY_EDIT_KEY.kind)}</Text>
        <TextTabs
          items={KINDS.map((kind) => ({ key: kind, label: t(ANCHOR_KIND_LABEL_KEY[kind]) }))}
          value={form.kind ?? ''}
          onChange={(kind) => update({ kind: kind as AnchorKind })}
          testID="physiology-editor-kind"
        />
      </View>
      <View className="gap-2">
        <Text className="text-sm text-text-secondary">{t(PHYSIOLOGY_EDIT_KEY.confidence)}</Text>
        <TextTabs
          items={CONFIDENCES.map((confidence) => ({
            key: confidence,
            label: t(ANCHOR_CONFIDENCE_LABEL_KEY[confidence]),
          }))}
          value={form.confidence}
          onChange={(confidence) => update({ confidence: confidence as AnchorConfidence })}
          testID="physiology-editor-confidence"
        />
        <Text className="text-xs text-text-tertiary">{t(PHYSIOLOGY_EDIT_KEY.confidenceHint)}</Text>
      </View>
      <Input
        label={t(PHYSIOLOGY_EDIT_KEY.method)}
        value={form.method}
        onChangeText={(method) => update({ method })}
        maxLength={PHYSIOLOGY_METHOD_MAX_CHARS}
        testID="physiology-editor-method"
      />
      <Input
        label={t(PHYSIOLOGY_EDIT_KEY.measuredOn)}
        value={form.measuredOn}
        onChangeText={(measuredOn) => update({ measuredOn })}
        keyboardType="numbers-and-punctuation"
        testID="physiology-editor-date"
      />
      {saveError !== null && (
        <Text className="text-sm text-error" accessibilityRole="alert" testID="physiology-editor-error">
          {saveError}
        </Text>
      )}
      <View className="flex-row justify-end gap-3">
        <Button title={t(PHYSIOLOGY_EDIT_KEY.cancel)} variant="secondary" onPress={onDone} testID="physiology-editor-cancel" />
        <Button
          title={saving ? t(PHYSIOLOGY_EDIT_KEY.saving) : t(PHYSIOLOGY_EDIT_KEY.save)}
          onPress={() => void submit()}
          loading={saving}
          disabled={form.kind === null || saving}
          testID="physiology-editor-save"
        />
      </View>
    </View>
  );
}
