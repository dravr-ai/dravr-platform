// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The day editor's "From a family" mode — sport first, then the families and formats in it, the values, and the kernel's live preview
// ABOUTME: Every figure, step and finding is the kernel's, read from the preview; the catalogue's loading, error and empty states live here alone

import { useId, useState } from 'react';
import { AlertTriangle } from 'lucide-react';
import { useTranslation } from '@pierre/i18n';
import type {
  FamilyCatalogueResponse,
  SessionBuild,
  SessionParamKey,
  SessionParamRange,
  SessionPreview,
  SessionSport,
} from '@pierre/shared-types';
import {
  SESSION_EDITOR_KEYS,
  SESSION_PARAM_LABEL_KEY,
  catalogueSports,
  derivedRows,
  editableParams,
  familiesForSport,
  familyOf,
  flaggedParams,
  paramValue,
  presetOf,
  presetsForSport,
  selectBuildSport,
  selectFamily,
  selectPreset,
  sessionSportLabelKey,
  setParam,
  sportLabel,
} from '@pierre/ui-logic';
import { useSessionPreview } from '../../hooks/useSessionEditor';
import { ALERT_TONE, Badge, Input, Radio, Select } from '../ui';
import { EvidenceChip } from './EvidenceChip';
import { PlanStepList } from './WorkoutPlanCard';

/** The small heading over each part of the preview. */
const PART_HEADING = 'mb-1.5 text-xs font-semibold text-on-surface-variant';

interface SessionEditorFamilyProps {
  conversationId: string;
  /** The day the session is built for. */
  date: string;
  /** The athlete's name, as the empty catalogue says it. */
  athleteName: string;
  /** The thread's family catalogue, once it answered. */
  catalogue: FamilyCatalogueResponse | undefined;
  /** Whether the catalogue read failed. */
  catalogueFailed: boolean;
  /** The build on screen: the coach's, else the one the day opens on; `null` while there is no catalogue to build from. */
  build: SessionBuild | null;
  /** The day's sport when no family has a form in it: the build then opens in another sport, and says so. */
  familyMissing: SessionSport | null;
  onChange: (build: SessionBuild) => void;
}

/**
 * The family mode: the catalogue's states, then the sport, the families with
 * a form in it, the format, the values with their tested ranges, and the
 * preview the kernel measures once the typing settles.
 */
export function SessionEditorFamily({
  conversationId,
  date,
  athleteName,
  catalogue,
  catalogueFailed,
  build,
  familyMissing,
  onChange,
}: SessionEditorFamilyProps) {
  const { t } = useTranslation();

  if (catalogueFailed) {
    return (
      <p role="alert" className={`${ALERT_TONE.error.tone} rounded-lg px-3 py-2 text-sm`} data-testid="session-editor-load-failed">
        {t(SESSION_EDITOR_KEYS.loadFailed)}
      </p>
    );
  }
  if (!catalogue) {
    return (
      <p role="status" className="py-2 text-sm text-on-surface-variant" data-testid="session-editor-loading">
        {t(SESSION_EDITOR_KEYS.loading)}
      </p>
    );
  }
  if (!build) {
    return (
      <p className="py-2 text-sm text-on-surface-variant" data-testid="session-editor-no-families">
        {t(SESSION_EDITOR_KEYS.noFamilies, { name: athleteName })}
      </p>
    );
  }
  return (
    <div className="space-y-5">
      {familyMissing && (
        <p className="text-sm text-on-surface-variant" data-testid="session-editor-no-family-for-sport">
          {t(SESSION_EDITOR_KEYS.noFamilyForSport, { sport: sportLabel(t, familyMissing) })}
        </p>
      )}
      <FamilyBuildForm conversationId={conversationId} date={date} catalogue={catalogue} build={build} onChange={onChange} />
    </div>
  );
}

interface FamilyBuildFormProps {
  conversationId: string;
  date: string;
  catalogue: FamilyCatalogueResponse;
  build: SessionBuild;
  onChange: (build: SessionBuild) => void;
}

/**
 * The picks in the order a coach thinks of them: the sport the day is in,
 * then the families written for it, then the format. Findings never hold
 * Save back: a session outside its family's evidence is saved as built, and
 * the card marks it (D3).
 */
function FamilyBuildForm({ conversationId, date, catalogue, build, onChange }: FamilyBuildFormProps) {
  const { t, language } = useTranslation();
  const sportName = useId();
  const sports = catalogueSports(catalogue);
  // The families written for the build's sport. A format with no sport list
  // can leave the build in a sport no family names; the select then offers
  // every family rather than none.
  const inSport = familiesForSport(catalogue, build.sport);
  const families = inSport.length > 0 ? inSport : catalogue.families;
  const family = familyOf(catalogue, build);
  const preset = presetOf(catalogue, build);
  const params = editableParams(preset, build.sport);
  const preview = useSessionPreview(conversationId, date, build, language);
  // The fields the kernel found outside their range, from the preview it measured.
  const flagged = flaggedParams(preview.data);

  return (
    <div className="space-y-5">
      {sports.length > 0 && (
        <fieldset data-testid="session-editor-family-sport">
          <legend className="mb-2 block text-sm font-medium text-on-surface-variant">{t(SESSION_EDITOR_KEYS.sport)}</legend>
          <div className="flex flex-wrap gap-x-6 gap-y-2">
            {sports.map((sport) => (
              <div key={sport}>
                <Radio
                  name={sportName}
                  value={sport}
                  checked={build.sport === sport}
                  onChange={() => onChange(selectBuildSport(catalogue, build, sport))}
                  label={t(sessionSportLabelKey(sport))}
                />
              </div>
            ))}
          </div>
        </fieldset>
      )}

      <Select
        label={t(SESSION_EDITOR_KEYS.family)}
        value={build.family}
        options={families.map((option) => ({ value: option.slug, label: option.name }))}
        helpText={family?.purpose_label}
        onChange={(event) => {
          const picked = families.find((option) => option.slug === event.target.value);
          if (picked) onChange(selectFamily(picked, build.sport));
        }}
        data-testid="session-editor-family"
      />

      <Select
        label={t(SESSION_EDITOR_KEYS.format)}
        value={preset?.key ?? ''}
        options={presetsForSport(family, build.sport).map((option) => ({ value: option.key, label: option.name }))}
        helpText={preset?.summary}
        onChange={(event) => {
          const picked = family?.presets.find((option) => option.key === event.target.value);
          if (picked) onChange(selectPreset(build, picked));
        }}
        data-testid="session-editor-preset"
      />

      {params.length > 0 && (
        <div className="grid grid-cols-1 gap-x-4 gap-y-4 sm:grid-cols-2">
          {params.map(({ key, range }) => (
            <ParamField
              key={`${preset?.key ?? ''}-${key}`}
              paramKey={key}
              range={range}
              value={paramValue(build, key, range)}
              outside={flagged.has(key)}
              onChange={(value) => onChange(setParam(build, key, value))}
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
 * One value the format lets the coach state, with its tested range under
 * it. A value the kernel's preview found outside the range turns the range
 * line to the warning tone and is kept as typed: the findings say what it
 * costs (D3). Digits only; while the coach types, the field shows what they
 * typed, so emptying it to type a new number does not snap it back to the
 * default.
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
        {t(SESSION_EDITOR_KEYS.testedRange, { min: range.min, max: range.max })}
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
          <p className="text-sm text-on-surface-variant">{t(SESSION_EDITOR_KEYS.findingsInside)}</p>
        )}
      </div>
    </>
  );
}
