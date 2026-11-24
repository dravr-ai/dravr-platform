// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The day editor's family mode on the phone — a family, its format and the values it lets the coach state, with the kernel's live preview
// ABOUTME: The catalogue's loading, failure and empty states live here; the sport is the sheet's chips, never picked twice (carnet#1515, #1727)

import React, { useState } from 'react';
import { ActivityIndicator, Text, View } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { useTranslation } from '@pierre/i18n';
import type {
  FamilyCatalogueResponse,
  SessionBuild,
  SessionParamKey,
  SessionParamRange,
  SessionPreview,
} from '@pierre/shared-types';
import {
  derivedRows,
  editableParams,
  familiesForSport,
  familyOf,
  flaggedParams,
  paramValue,
  presetOf,
  presetsForSport,
  selectFamily,
  selectPreset,
  SESSION_EDITOR_KEYS,
  SESSION_PARAM_LABEL_KEY,
  setParam,
} from '@pierre/ui-logic';
import { useThemeColors } from '../../constants/theme';
import { ALERT_TONE, alertInkColor, Badge, Input, RadioGroup } from '../../components/ui';
import { EvidenceChip } from './EvidenceChip';
import { Figure, StepText } from './WorkoutPlanCard';

interface SessionFamilyFormProps {
  athleteName: string;
  /** The athlete's families; `undefined` until the read answers. */
  catalogue: FamilyCatalogueResponse | undefined;
  loadFailed: boolean;
  build: SessionBuild | null;
  preview: SessionPreview | undefined;
  measuring: boolean;
  previewFailed: boolean;
  onChange: (build: SessionBuild) => void;
}

/**
 * "From an evidence-backed family": the catalogue's loading, failure and
 * empty states live here and nowhere else — the session mode never waits on
 * the catalogue. The sheet offers this mode only for a sport a family is
 * written for, so the build is always in the sport its chips picked.
 */
export function SessionFamilyForm({
  athleteName,
  catalogue,
  loadFailed,
  build,
  preview,
  measuring,
  previewFailed,
  onChange,
}: SessionFamilyFormProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  if (loadFailed) {
    return (
      <Text
        className="px-4 py-6 text-sm text-text-secondary"
        accessibilityRole="alert"
        testID="session-editor-load-failed"
      >
        {t(SESSION_EDITOR_KEYS.loadFailed)}
      </Text>
    );
  }
  if (catalogue === undefined) {
    return (
      <View className="items-center px-4 py-6" testID="session-editor-loading">
        <ActivityIndicator color={colors.tokens.primary} />
        <Text className="mt-2 text-sm text-text-secondary">{t(SESSION_EDITOR_KEYS.loading)}</Text>
      </View>
    );
  }
  if (build === null) {
    return (
      <Text className="px-4 py-6 text-sm text-text-secondary" testID="session-editor-no-families">
        {t(SESSION_EDITOR_KEYS.noFamilies, { name: athleteName })}
      </Text>
    );
  }
  return (
    <>
      <BuildForm catalogue={catalogue} build={build} flagged={flaggedParams(preview)} onChange={onChange} />
      <PreviewPanel preview={preview} measuring={measuring} failed={previewFailed} />
    </>
  );
}

interface BuildFormProps {
  catalogue: FamilyCatalogueResponse;
  build: SessionBuild;
  /** The values the kernel's preview found outside their range: the fields marked. */
  flagged: ReadonlySet<SessionParamKey>;
  onChange: (build: SessionBuild) => void;
}

/**
 * The families with a form in the build's sport, the family's formats in it,
 * and each value the format lets the coach state. Picking a family or a
 * format starts its values at their defaults in that sport.
 */
function BuildForm({ catalogue, build, flagged, onChange }: BuildFormProps) {
  const { t } = useTranslation();
  // What the coach typed into each value, so a field can be emptied and
  // retyped: an empty field states nothing, and the format's default applies.
  const [typed, setTyped] = useState<Partial<Record<SessionParamKey, string>>>({});
  const families = familiesForSport(catalogue, build.sport);
  const family = familyOf(catalogue, build);
  const preset = presetOf(catalogue, build);
  const formats = presetsForSport(family, build.sport);

  const pick = (next: SessionBuild) => {
    setTyped({});
    onChange(next);
  };

  const type = (key: SessionParamKey, text: string) => {
    // A whole number, nothing more: the range is the findings' business.
    const digits = text.replace(/[^0-9]/g, '');
    setTyped((current) => ({ ...current, [key]: digits }));
    onChange(setParam(build, key, digits === '' ? undefined : Number(digits)));
  };

  return (
    <View className="px-4" testID="session-editor-form">
      <View className="mb-4">
        <RadioGroup
          label={t(SESSION_EDITOR_KEYS.family)}
          options={families.map((f) => ({
            key: f.slug,
            label: f.name,
            description: f.slug === build.family ? f.purpose_label : undefined,
          }))}
          value={build.family}
          onChange={(slug) => {
            const next = families.find((f) => f.slug === slug);
            if (next) pick(selectFamily(next, build.sport));
          }}
          testID="session-editor-family"
        />
      </View>
      {formats.length > 0 ? (
        <View className="mb-4">
          <RadioGroup
            label={t(SESSION_EDITOR_KEYS.format)}
            options={formats.map((p) => ({
              key: p.key,
              label: p.name,
              description: p.key === preset?.key ? p.summary : undefined,
            }))}
            value={preset?.key ?? null}
            onChange={(key) => {
              const next = formats.find((p) => p.key === key);
              if (next) pick(selectPreset(build, next));
            }}
            testID="session-editor-format"
          />
        </View>
      ) : null}
      {editableParams(preset, build.sport).map(({ key, range }) => (
        <ParamField
          key={key}
          paramKey={key}
          range={range}
          text={typed[key] ?? String(paramValue(build, key, range))}
          outside={flagged.has(key)}
          onChangeText={(text) => type(key, text)}
        />
      ))}
    </View>
  );
}

interface ParamFieldProps {
  paramKey: SessionParamKey;
  range: SessionParamRange;
  text: string;
  /** The stated value sits outside the format's tested range. */
  outside: boolean;
  onChangeText: (text: string) => void;
}

/**
 * One stated value on the number pad, with the format's tested range under
 * it. Outside the range the line takes the warning ink and its mark; the
 * value is kept as typed, never clamped.
 */
function ParamField({ paramKey, range, text, outside, onChangeText }: ParamFieldProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const label = t(SESSION_PARAM_LABEL_KEY[paramKey]);
  const rangeText = t(SESSION_EDITOR_KEYS.testedRange, { min: range.min, max: range.max });
  return (
    <View>
      <Input
        label={label}
        value={text}
        onChangeText={onChangeText}
        keyboardType="number-pad"
        accessibilityLabel={label}
        accessibilityHint={rangeText}
        testID={`session-editor-param-${paramKey}`}
      />
      <View className="-mt-2 mb-4 flex-row items-center" testID={`session-editor-range-${paramKey}`}>
        {outside ? (
          <Feather
            name="alert-triangle"
            size={12}
            color={alertInkColor('warning', colors)}
            testID={`session-editor-outside-${paramKey}`}
          />
        ) : null}
        <Text className={`text-xs ${outside ? `ml-1 ${ALERT_TONE.warning.ink}` : 'text-text-secondary'}`}>
          {rangeText}
        </Text>
      </View>
    </View>
  );
}

interface PreviewPanelProps {
  /** The latest preview; the previous one while the next is measured. */
  preview: SessionPreview | undefined;
  measuring: boolean;
  failed: boolean;
}

/**
 * What the kernel derives from the build on the athlete's anchors, as the
 * server answered it: the day card's marks, the derived values, the steps
 * and the envelope's findings. The previous answer stays while the next one
 * is measured.
 */
function PreviewPanel({ preview, measuring, failed }: PreviewPanelProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  return (
    <View className="px-4" testID="session-editor-preview">
      {measuring ? (
        <View className="mb-2 flex-row items-center" accessibilityLiveRegion="polite" testID="session-editor-measuring">
          <ActivityIndicator size="small" color={colors.tokens.primary} />
          <Text className="ml-2 text-sm text-text-secondary">{t(SESSION_EDITOR_KEYS.measuring)}</Text>
        </View>
      ) : null}
      {failed ? (
        <View
          className={`mb-2 rounded-lg px-3 py-2 ${ALERT_TONE.error.fill}`}
          accessibilityRole="alert"
          testID="session-editor-preview-failed"
        >
          <Text className={`text-sm ${ALERT_TONE.error.ink}`}>{t(SESSION_EDITOR_KEYS.previewFailed)}</Text>
        </View>
      ) : null}
      {preview ? <PreviewBody preview={preview} /> : null}
    </View>
  );
}

/** A heading over one part of the preview. */
function PreviewHeading({ children }: { children: string }) {
  return (
    <Text className="mb-1 mt-4 text-sm font-semibold text-text-primary" accessibilityRole="header">
      {children}
    </Text>
  );
}

function PreviewBody({ preview }: { preview: SessionPreview }) {
  const { t, language } = useTranslation();
  const rows = preview.derived
    ? derivedRows(preview.derived, language, (count) => t(SESSION_EDITOR_KEYS.minutes, { count }))
    : [];
  return (
    <View>
      {preview.outside_evidence ? (
        <Badge
          variant="warning"
          label={preview.outside_evidence}
          className="mt-1 self-start"
          testID="session-editor-outside-evidence"
        />
      ) : null}
      {preview.evidence ? <EvidenceChip evidence={preview.evidence} /> : null}
      {preview.counts_as ? (
        <Text className="mt-1 text-sm text-text-secondary" testID="session-editor-counts-as">
          {preview.counts_as}
        </Text>
      ) : null}
      {preview.anchor ? (
        <Text className="mt-1 text-sm text-text-secondary" testID="session-editor-anchor">
          {preview.anchor}
        </Text>
      ) : null}

      {rows.length > 0 ? (
        <View testID="session-editor-derived">
          <PreviewHeading>{t(SESSION_EDITOR_KEYS.derivedHeading)}</PreviewHeading>
          {rows.map((row) => (
            <View key={row.key} className="flex-row py-0.5" testID={`session-editor-derived-${row.key}`}>
              <Text className="flex-1 text-sm text-text-secondary">{t(row.labelKey, row.labelValues)}</Text>
              <Text className="ml-3 text-sm text-text-primary">
                <Figure>{row.value}</Figure>
              </Text>
            </View>
          ))}
        </View>
      ) : null}

      {preview.steps.length > 0 ? (
        <View testID="session-editor-steps">
          <PreviewHeading>{t(SESSION_EDITOR_KEYS.stepsHeading)}</PreviewHeading>
          {preview.steps.map((step, index) => (
            <Text key={`${step.label}-${index}`} className="py-0.5 text-sm text-text-primary">
              <StepText step={step} />
            </Text>
          ))}
        </View>
      ) : null}

      <PreviewHeading>{t(SESSION_EDITOR_KEYS.findingsHeading)}</PreviewHeading>
      {preview.findings.length === 0 ? (
        <Text className="text-sm text-text-secondary" testID="session-editor-findings-none">
          {t(SESSION_EDITOR_KEYS.findingsInside)}
        </Text>
      ) : (
        <>
          <View className={`rounded-lg px-3 py-2 ${ALERT_TONE.warning.fill}`} testID="session-editor-findings">
            {preview.findings.map((finding, index) => (
              <Text
                key={`${finding.code}-${index}`}
                className={`text-sm ${ALERT_TONE.warning.ink}${index > 0 ? ' mt-1' : ''}`}
              >
                {finding.text}
              </Text>
            ))}
          </View>
          <Text className="mt-1 text-xs text-text-secondary" testID="session-editor-findings-note">
            {t(SESSION_EDITOR_KEYS.findingsNote)}
          </Text>
        </>
      )}
    </View>
  );
}
