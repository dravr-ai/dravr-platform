// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach's "build from family" editor for one day of their athlete's plan — family, preset, sport and values, measured live
// ABOUTME: Every figure, step and finding is the kernel's, read from the preview; Save writes the day into the coach's draft and is never refused (D3)

import { useId, useMemo, useState } from 'react';
import { AlertTriangle } from 'lucide-react';
import { useTranslation } from '@pierre/i18n';
import type {
  FamilyCatalogueResponse,
  PlanDay,
  PlanSubject,
  SessionBuild,
  SessionParamKey,
  SessionParamRange,
  SessionPreview,
} from '@pierre/shared-types';
import {
  API_ERROR_KEYS,
  ROW_DATE,
  SESSION_EDITOR_KEYS,
  SESSION_PARAM_LABEL_KEY,
  derivedRows,
  describeApiError,
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
  sessionSportLabelKey,
  setParam,
} from '@pierre/ui-logic';
import { useIsMobile } from '../../hooks/useBreakpoint';
import { useSaveDraftDay, useSessionPreview, useSubjectFamilies } from '../../hooks/useSessionEditor';
import { ALERT_TONE, Badge, Button, Input, Radio, Select, Sheet, useErrorToast, useSuccessToast } from '../ui';
import { EvidenceChip } from './EvidenceChip';
import { PlanStepList } from './WorkoutPlanCard';

interface SessionEditorSheetProps {
  /** The coach's plan thread on the athlete. */
  conversationId: string;
  /** The athlete the plan is for. */
  subject: PlanSubject;
  /** The day the session is built for, as the card shows it. */
  day: PlanDay;
  onClose: () => void;
}

/** The small heading over each part of the preview. */
const PART_HEADING = 'mb-1.5 text-xs font-semibold text-on-surface-variant';

/**
 * The sheet a day's "Build from family" opens: the families the athlete's
 * agent offers, read in the coach's language, and once they answer the
 * editor on the build the day already holds.
 */
export default function SessionEditorSheet({ conversationId, subject, day, onClose }: SessionEditorSheetProps) {
  const { t, language } = useTranslation();
  const isMobile = useIsMobile();
  const families = useSubjectFamilies(conversationId, language);
  const catalogue = families.data;
  const initial = useMemo(() => (catalogue ? initialSessionBuild(day, catalogue) : null), [catalogue, day]);
  const name = subject.display_name.trim() || t('app.unknownMember');
  const title = t(SESSION_EDITOR_KEYS.title, { date: formatCivilDate(day.date, language, ROW_DATE) });

  let body;
  if (families.isError) {
    body = (
      <p role="alert" className={`${ALERT_TONE.error.tone} m-4 rounded-lg px-3 py-2 text-sm`} data-testid="session-editor-load-failed">
        {t(SESSION_EDITOR_KEYS.loadFailed)}
      </p>
    );
  } else if (!catalogue) {
    body = (
      <p role="status" className="px-4 py-4 text-sm text-on-surface-variant" data-testid="session-editor-loading">
        {t(SESSION_EDITOR_KEYS.loading)}
      </p>
    );
  } else if (!initial) {
    body = (
      <p className="px-4 py-4 text-sm text-on-surface-variant" data-testid="session-editor-no-families">
        {t(SESSION_EDITOR_KEYS.noFamilies, { name })}
      </p>
    );
  } else {
    body = (
      <SessionEditorForm
        conversationId={conversationId}
        athleteName={name}
        day={day}
        catalogue={catalogue}
        initial={initial}
        onClose={onClose}
      />
    );
  }

  return (
    <Sheet
      side={isMobile ? 'bottom' : 'right'}
      width="lg"
      title={title}
      onClose={onClose}
      data-testid="session-editor"
    >
      {body}
    </Sheet>
  );
}

interface SessionEditorFormProps {
  conversationId: string;
  /** The athlete's name, as the save's confirmation says it. */
  athleteName: string;
  day: PlanDay;
  catalogue: FamilyCatalogueResponse;
  /** The build the day opens on. */
  initial: SessionBuild;
  onClose: () => void;
}

/**
 * The picks, the values and the live preview, with Cancel and Save under
 * them. Findings never hold Save back: a session outside its family's
 * evidence is saved as built, and the card marks it.
 */
function SessionEditorForm({ conversationId, athleteName, day, catalogue, initial, onClose }: SessionEditorFormProps) {
  const { t, language } = useTranslation();
  const showSuccess = useSuccessToast();
  const showError = useErrorToast();
  const sportName = useId();
  const [edited, setEdited] = useState<SessionBuild | null>(null);
  const build = edited ?? initial;
  const family = familyOf(catalogue, build);
  const preset = presetOf(catalogue, build);
  const preview = useSessionPreview(conversationId, day.date, build, language);
  // The fields the kernel found outside their range, from the preview it measured.
  const flagged = flaggedParams(preview.data);
  const save = useSaveDraftDay(conversationId, language);

  const onSave = async () => {
    try {
      await save.mutateAsync({ date: day.date, build });
      showSuccess(t(SESSION_EDITOR_KEYS.draftNote, { name: athleteName }));
      onClose();
    } catch (error) {
      showError(t(SESSION_EDITOR_KEYS.saveFailed), describeApiError(error, { t, fallbackKey: API_ERROR_KEYS.unknown }));
    }
  };

  return (
    <div className="flex min-h-full flex-col">
      <div className="flex-1 space-y-5 px-4 py-4">
        {!day.rest && (
          <p className="text-sm text-on-surface-variant" data-testid="session-editor-replaces">
            {t(SESSION_EDITOR_KEYS.replaces)}
          </p>
        )}

        <Select
          label={t(SESSION_EDITOR_KEYS.family)}
          value={build.family}
          options={catalogue.families.map((option) => ({ value: option.slug, label: option.name }))}
          helpText={family?.purpose_label}
          onChange={(event) => {
            const picked = catalogue.families.find((option) => option.slug === event.target.value);
            if (picked) setEdited(selectFamily(picked, build.sport));
          }}
          data-testid="session-editor-family"
        />

        <Select
          label={t(SESSION_EDITOR_KEYS.preset)}
          value={preset?.key ?? ''}
          options={(family?.presets ?? []).map((option) => ({ value: option.key, label: option.name }))}
          helpText={preset?.summary}
          onChange={(event) => {
            const picked = family?.presets.find((option) => option.key === event.target.value);
            if (picked) setEdited(selectPreset(build, picked));
          }}
          data-testid="session-editor-preset"
        />

        {preset && preset.sports.length > 0 && (
          <fieldset>
            <legend className="mb-2 block text-sm font-medium text-on-surface-variant">
              {t(SESSION_EDITOR_KEYS.sport)}
            </legend>
            <div className="flex flex-wrap gap-x-6 gap-y-2">
              {preset.sports.map((sport) => (
                <div key={sport}>
                  <Radio
                    name={sportName}
                    value={sport}
                    checked={build.sport === sport}
                    onChange={() => setEdited(selectSport(build, preset, sport))}
                    label={t(sessionSportLabelKey(sport))}
                  />
                </div>
              ))}
            </div>
          </fieldset>
        )}

        {editableParams(preset, build.sport).length > 0 && (
          <div className="grid grid-cols-1 gap-x-4 gap-y-4 sm:grid-cols-2">
            {editableParams(preset, build.sport).map(({ key, range }) => (
              <ParamField
                key={`${preset?.key ?? ''}-${key}`}
                paramKey={key}
                range={range}
                value={paramValue(build, key, range)}
                outside={flagged.has(key)}
                onChange={(value) => setEdited(setParam(build, key, value))}
              />
            ))}
          </div>
        )}

        <div className="space-y-4">
          {/* Present from the start and outside the busy preview, so a reader
              hears it each time the session is measured again; the previous
              preview stays on screen under it. */}
          <p role="status" className="min-h-4 text-xs text-on-surface-variant" data-testid="session-editor-measuring">
            {preview.isFetching ? t(SESSION_EDITOR_KEYS.measuring) : ''}
          </p>
          <section aria-busy={preview.isFetching} className="space-y-4" data-testid="session-editor-preview">
            {preview.isError && !preview.isFetching && (
              <p
                role="alert"
                className={`${ALERT_TONE.error.tone} rounded-lg px-3 py-2 text-sm`}
                data-testid="session-editor-preview-failed"
              >
                {t(SESSION_EDITOR_KEYS.previewFailed)}
              </p>
            )}
            {preview.data && <PreviewBody preview={preview.data} />}
          </section>
        </div>
      </div>

      <div className="sticky bottom-0 flex items-center justify-end gap-2 border-t ghost-border-faint bg-surface px-4 py-3">
        <Button variant="tertiary" onClick={onClose} data-testid="session-editor-cancel">
          {t(SESSION_EDITOR_KEYS.cancel)}
        </Button>
        <Button variant="primary" loading={save.isPending} onClick={() => void onSave()} data-testid="session-editor-save">
          {save.isPending ? t(SESSION_EDITOR_KEYS.saving) : t(SESSION_EDITOR_KEYS.save)}
        </Button>
      </div>
    </div>
  );
}

interface ParamFieldProps {
  paramKey: SessionParamKey;
  range: SessionParamRange;
  /** What the build states, else the preset's default. */
  value: number;
  /** Whether the kernel's preview found this value outside its range. */
  outside: boolean;
  /** A whole number typed, or `undefined` once the field is emptied. */
  onChange: (value: number | undefined) => void;
}

/**
 * One value the preset lets the coach state, with its evidence range under
 * it. A value the kernel's preview found outside the range turns the range
 * line to the warning tone and is kept as typed: the findings say what it
 * costs (D3). Digits
 * only; while the coach types, the field shows what they typed, so emptying
 * it to type a new number does not snap it back to the default.
 */
function ParamField({ paramKey, range, value, outside, onChange }: ParamFieldProps) {
  const { t } = useTranslation();
  const rangeId = useId();
  const [typing, setTyping] = useState<string | null>(null);
  return (
    <div>
      <Input
        label={t(SESSION_PARAM_LABEL_KEY[paramKey])}
        type="text"
        inputMode="numeric"
        pattern="[0-9]*"
        autoComplete="off"
        value={typing ?? String(value)}
        aria-describedby={rangeId}
        onFocus={() => setTyping(String(value))}
        onBlur={() => setTyping(null)}
        onChange={(event) => {
          const digits = event.target.value.replace(/\D/g, '');
          setTyping(digits);
          onChange(digits === '' ? undefined : Number(digits));
        }}
        data-testid={`session-editor-param-${paramKey}`}
      />
      <p
        id={rangeId}
        className={`mt-1.5 flex items-center gap-1 text-xs ${outside ? ALERT_TONE.warning.ink : 'text-outline'}`}
        data-testid={`session-editor-range-${paramKey}`}
        data-outside={outside ? 'true' : undefined}
      >
        {outside && <AlertTriangle className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />}
        {t(SESSION_EDITOR_KEYS.range, { min: range.min, max: range.max })}
      </p>
    </div>
  );
}

/**
 * What the kernel made of the build: the day card's warning and evidence
 * chip, the relabel and anchor lines, the derived figures, the steps on the
 * athlete's anchors, and the envelope findings. Laid out only; every word
 * of the server's arrives already in the coach's language.
 */
function PreviewBody({ preview }: { preview: SessionPreview }) {
  const { t, language } = useTranslation();
  const rows = preview.derived
    ? derivedRows(preview.derived, language, (count) => t(SESSION_EDITOR_KEYS.minutes, { count }))
    : [];
  return (
    <>
      {(preview.outside_evidence || preview.evidence) && (
        <div className="flex flex-wrap items-center gap-y-1">
          {preview.outside_evidence && (
            <Badge variant="warning" data-testid="session-editor-outside-evidence">
              {preview.outside_evidence}
            </Badge>
          )}
          {preview.evidence && <EvidenceChip evidence={preview.evidence} />}
        </div>
      )}
      {(preview.counts_as || preview.anchor) && (
        <div className="space-y-0.5 text-xs text-on-surface-variant">
          {preview.counts_as && <p data-testid="session-editor-counts-as">{preview.counts_as}</p>}
          {preview.anchor && <p data-testid="session-editor-anchor">{preview.anchor}</p>}
        </div>
      )}

      {rows.length > 0 && (
        <div>
          <h3 className={PART_HEADING}>{t(SESSION_EDITOR_KEYS.derivedHeading)}</h3>
          <dl className="text-sm" data-testid="session-editor-derived">
            {rows.map((row) => (
              <div key={row.key} className="flex items-baseline justify-between gap-3 border-b ghost-border-faint py-1.5 last:border-0">
                <dt className="text-on-surface-variant">{t(row.labelKey, row.labelValues)}</dt>
                <dd className="font-mono text-on-surface">{row.value}</dd>
              </div>
            ))}
          </dl>
        </div>
      )}

      {preview.steps.length > 0 && (
        <div>
          <h3 className={PART_HEADING}>{t(SESSION_EDITOR_KEYS.stepsHeading)}</h3>
          <PlanStepList steps={preview.steps} className="space-y-1 text-sm text-on-surface" />
        </div>
      )}

      <div data-testid="session-editor-findings">
        <h3 className={PART_HEADING}>{t(SESSION_EDITOR_KEYS.findingsHeading)}</h3>
        {preview.findings.length > 0 ? (
          <div className={`${ALERT_TONE.warning.tone} rounded-lg border px-3 py-2 text-sm`}>
            <ul className="space-y-1 text-on-surface">
              {preview.findings.map((finding, index) => (
                <li key={`${finding.code}-${index}`} className="flex gap-2">
                  <AlertTriangle className={`mt-0.5 h-4 w-4 shrink-0 ${ALERT_TONE.warning.ink}`} aria-hidden="true" />
                  <span>{finding.text}</span>
                </li>
              ))}
            </ul>
            <p className={`mt-1.5 text-xs ${ALERT_TONE.warning.ink}`}>{t(SESSION_EDITOR_KEYS.findingsNote)}</p>
          </div>
        ) : (
          <p className="text-sm text-on-surface-variant">{t(SESSION_EDITOR_KEYS.findingsNone)}</p>
        )}
      </div>
    </>
  );
}
