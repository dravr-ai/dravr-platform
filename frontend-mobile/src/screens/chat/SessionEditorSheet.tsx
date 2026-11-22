// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach's "build from family" editor on the phone — a sheet over the plan thread that builds one day from a workout family
// ABOUTME: The kernel measures every number on the server; the sheet holds the coach's picks and lays out what the preview answers (carnet#1515)

import React, { useMemo, useState } from 'react';
import { ActivityIndicator, KeyboardAvoidingView, Platform, ScrollView, Text, View } from 'react-native';
import { Feather } from '@expo/vector-icons';
import Toast from 'react-native-toast-message';
import { useTranslation } from '@pierre/i18n';
import type {
  FamilyCatalogueResponse,
  PlanDay,
  SessionBuild,
  SessionParamKey,
  SessionParamRange,
  SessionPreview,
} from '@pierre/shared-types';
import {
  API_ERROR_KEYS,
  derivedRows,
  describeApiError,
  DRAFT_DATE,
  editableParams,
  familyOf,
  formatCivilDate,
  initialSessionBuild,
  flaggedParams,
  paramValue,
  presetOf,
  selectFamily,
  selectPreset,
  selectSport,
  SESSION_EDITOR_KEYS,
  SESSION_PARAM_LABEL_KEY,
  sessionSportLabelKey,
  setParam,
} from '@pierre/ui-logic';
import { useThemeColors } from '../../constants/theme';
import { ALERT_TONE, alertInkColor, Badge, Button, Input, Sheet, TextTabs } from '../../components/ui';
import { useSaveDraftDay, useSessionPreview, useSubjectFamilies } from '../../hooks/useSessionEditor';
import { EvidenceChip } from './EvidenceChip';
import { Figure, StepText } from './WorkoutPlanCard';

interface SessionEditorSheetProps {
  /** The coach's plan thread on the athlete. */
  conversationId: string;
  /** The athlete the thread is about, as its header names them. */
  athleteName: string;
  /** The plan day the editor is open on; `null` while it is closed. */
  day: PlanDay | null;
  onClose: () => void;
}

/**
 * The editor over a coach's plan thread: one plan day, built from a workout
 * family the athlete's agent offers, saved into the coach's draft. The build
 * starts afresh on every opening, from what the day holds.
 */
export default function SessionEditorSheet({ conversationId, athleteName, day, onClose }: SessionEditorSheetProps) {
  return (
    <Sheet visible={day !== null} onClose={onClose} flush testID="session-editor-sheet">
      {day !== null ? (
        <SessionEditor
          key={day.date}
          conversationId={conversationId}
          athleteName={athleteName}
          day={day}
          onClose={onClose}
        />
      ) : null}
    </Sheet>
  );
}

interface SessionEditorProps {
  conversationId: string;
  athleteName: string;
  day: PlanDay;
  onClose: () => void;
}

/**
 * The sheet's content: the families' read, the build, its live preview, and
 * Cancel and Save. A value outside its preset's range is saved as typed and
 * earns a finding; Save is never held back for one (D3).
 */
function SessionEditor({ conversationId, athleteName, day, onClose }: SessionEditorProps) {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  const families = useSubjectFamilies(conversationId, language);
  const catalogue = families.data;
  // The build the day opens on, until the coach changes it.
  const opening = useMemo(() => (catalogue ? initialSessionBuild(day, catalogue) : null), [day, catalogue]);
  const [edited, setEdited] = useState<SessionBuild | null>(null);
  const build = edited ?? opening;
  const preview = useSessionPreview(conversationId, day.date, build, language);
  const save = useSaveDraftDay(conversationId, language);
  // Why the last save was refused, in the coach's words; `null` until one is.
  const [saveRefusal, setSaveRefusal] = useState<string | null>(null);

  const onSave = () => {
    if (build === null) return;
    setSaveRefusal(null);
    save.mutate(
      { date: day.date, build },
      {
        onSuccess: () => {
          onClose();
          Toast.show({ type: 'success', text1: t(SESSION_EDITOR_KEYS.draftNote, { name: athleteName }) });
        },
        // Said in the sheet, which stays open for another try: a toast is
        // drawn under an open modal and nobody would see it.
        onError: (error) => setSaveRefusal(describeApiError(error, { t, fallbackKey: API_ERROR_KEYS.unknown })),
      },
    );
  };

  let body: React.ReactNode;
  if (families.isError) {
    body = (
      <Text className="px-4 py-6 text-sm text-text-secondary" accessibilityRole="alert" testID="session-editor-load-failed">
        {t(SESSION_EDITOR_KEYS.loadFailed)}
      </Text>
    );
  } else if (catalogue === undefined) {
    body = (
      <View className="items-center px-4 py-6" testID="session-editor-loading">
        <ActivityIndicator color={colors.tokens.primary} />
        <Text className="mt-2 text-sm text-text-secondary">{t(SESSION_EDITOR_KEYS.loading)}</Text>
      </View>
    );
  } else if (build === null) {
    body = (
      <Text className="px-4 py-6 text-sm text-text-secondary" testID="session-editor-no-families">
        {t(SESSION_EDITOR_KEYS.noFamilies, { name: athleteName })}
      </Text>
    );
  } else {
    body = (
      <>
        {day.rest ? null : (
          <Text className="mb-4 px-4 text-sm text-text-secondary" testID="session-editor-replaces">
            {t(SESSION_EDITOR_KEYS.replaces)}
          </Text>
        )}
        <BuildForm catalogue={catalogue} build={build} flagged={flaggedParams(preview.data)} onChange={setEdited} />
        <PreviewPanel
          preview={preview.data}
          measuring={preview.isFetching}
          failed={preview.isError && !preview.isFetching}
        />
      </>
    );
  }

  return (
    <KeyboardAvoidingView className="shrink" behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
      <Text
        className="mb-3 px-4 text-lg font-semibold text-text-primary"
        accessibilityRole="header"
        testID="session-editor-title"
      >
        {t(SESSION_EDITOR_KEYS.title, { date: formatCivilDate(day.date, language, DRAFT_DATE) })}
      </Text>
      <ScrollView keyboardShouldPersistTaps="handled" showsVerticalScrollIndicator={false}>
        {body}
      </ScrollView>
      <View className="mt-3 border-t border-border-faint px-4 pt-3">
        {saveRefusal !== null ? (
          <View
            className={`mb-3 rounded-lg px-3 py-2 ${ALERT_TONE.error.fill}`}
            accessibilityRole="alert"
            accessibilityLiveRegion="polite"
            testID="session-editor-save-failed"
          >
            <Text className={`text-sm font-semibold ${ALERT_TONE.error.ink}`}>{t(SESSION_EDITOR_KEYS.saveFailed)}</Text>
            <Text className={`mt-0.5 text-sm ${ALERT_TONE.error.ink}`}>{saveRefusal}</Text>
          </View>
        ) : null}
        <View className="flex-row gap-3">
          <View className="flex-1">
            <Button
              title={t(SESSION_EDITOR_KEYS.cancel)}
              variant="ghost"
              onPress={onClose}
              fullWidth
              testID="session-editor-cancel"
            />
          </View>
          {build !== null ? (
            <View className="flex-1">
              <Button
                title={save.isPending ? t(SESSION_EDITOR_KEYS.saving) : t(SESSION_EDITOR_KEYS.save)}
                onPress={onSave}
                disabled={save.isPending}
                fullWidth
                testID="session-editor-save"
              />
            </View>
          ) : null}
        </View>
      </View>
    </KeyboardAvoidingView>
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
 * The coach's picks: the family, its preset, the sport among the preset's,
 * and each value the preset lets them state. Picking a family or a preset
 * starts its values at its defaults.
 */
function BuildForm({ catalogue, build, flagged, onChange }: BuildFormProps) {
  const { t } = useTranslation();
  // What the coach typed into each value, so a field can be emptied and
  // retyped: an empty field states nothing, and the preset's default applies.
  const [typed, setTyped] = useState<Partial<Record<SessionParamKey, string>>>({});
  const family = familyOf(catalogue, build);
  const preset = presetOf(catalogue, build);
  const sports = preset?.sports ?? family?.sports ?? [];

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
    <View testID="session-editor-form">
      <Choice
        label={t(SESSION_EDITOR_KEYS.family)}
        items={catalogue.families.map((f) => ({ key: f.slug, label: f.name }))}
        value={build.family}
        onChange={(slug) => {
          const next = catalogue.families.find((f) => f.slug === slug);
          if (next) pick(selectFamily(next, build.sport));
        }}
        note={family?.purpose_label}
        testID="session-editor-family"
      />
      {family ? (
        <Choice
          label={t(SESSION_EDITOR_KEYS.preset)}
          items={family.presets.map((p) => ({ key: p.key, label: p.name }))}
          value={preset?.key ?? family.default_preset}
          onChange={(key) => {
            const next = family.presets.find((p) => p.key === key);
            if (next) pick(selectPreset(build, next));
          }}
          note={preset?.summary}
          testID="session-editor-preset"
        />
      ) : null}
      {sports.length > 0 ? (
        <Choice
          label={t(SESSION_EDITOR_KEYS.sport)}
          items={sports.map((sport) => ({ key: sport, label: t(sessionSportLabelKey(sport)) }))}
          value={build.sport}
          onChange={(key) => {
            const next = sports.find((sport) => sport === key);
            if (next) onChange(selectSport(build, preset, next));
          }}
          testID="session-editor-sport"
        />
      ) : null}
      <View className="px-4">
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
    </View>
  );
}

interface ChoiceProps {
  label: string;
  items: { key: string; label: string }[];
  value: string;
  onChange: (key: string) => void;
  /** One quiet line under the row — the picked family's purpose, the picked preset's summary. */
  note?: string;
  testID: string;
}

/** One labelled choice: the text-tab row a choice in a form takes. */
function Choice({ label, items, value, onChange, note, testID }: ChoiceProps) {
  return (
    <View className="mb-4">
      <Text className="mb-1 px-4 text-sm font-medium text-text-secondary">{label}</Text>
      <TextTabs items={items} value={value} onChange={onChange} testID={testID} />
      {note ? <Text className="mt-2 px-4 text-xs text-text-secondary">{note}</Text> : null}
    </View>
  );
}

interface ParamFieldProps {
  paramKey: SessionParamKey;
  range: SessionParamRange;
  text: string;
  /** The stated value sits outside the preset's evidence range. */
  outside: boolean;
  onChangeText: (text: string) => void;
}

/**
 * One stated value on the number pad, with the preset's evidence range under
 * it. Outside the range the line takes the warning ink and its mark; the
 * value is kept as typed, never clamped.
 */
function ParamField({ paramKey, range, text, outside, onChangeText }: ParamFieldProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const label = t(SESSION_PARAM_LABEL_KEY[paramKey]);
  const rangeText = t(SESSION_EDITOR_KEYS.range, { min: range.min, max: range.max });
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
    <Text className="mt-4 mb-1 text-sm font-semibold text-text-primary" accessibilityRole="header">
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
          {t(SESSION_EDITOR_KEYS.findingsNone)}
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
