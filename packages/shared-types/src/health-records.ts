// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete's health records wire shapes — served records by kind, the lab review queue, uploaded lab reports
// ABOUTME: With their parsers: a body of any other shape reads as null, never as a half-filled record (owl phase 5, slice 2)

/** The kinds `GET /api/me/health-records?kind=` serves. */
export type HealthRecordKind = 'labs' | 'clinical' | 'glucose' | 'self_reports';

/** Every kind, in the order the clients offer them. */
export const HEALTH_RECORD_KINDS: readonly HealthRecordKind[] = ['labs', 'clinical', 'glucose', 'self_reports'];

/**
 * The largest lab report `POST /api/me/lab-documents` reads, in bytes
 * (`MAX_LAB_DOCUMENT_BYTES` in pierre-server); a larger body is refused.
 */
export const LAB_DOCUMENT_MAX_BYTES = 10 * 1024 * 1024;

/** One component of an Observation: a blood pressure's systolic and diastolic. */
export interface ObservationComponent {
  code: string | null;
  value: number | null;
  unit: string | null;
}

/** An Observation as owl §6.4 flattens it: codes and numbers only. */
export interface FlatObservation {
  /** The code's system, `http://loinc.org` in nearly all cases. */
  codeSystem: string | null;
  code: string | null;
  /** The terminology's display for the code. */
  display: string | null;
  value: number | null;
  /** Its UCUM unit. */
  unit: string | null;
  /** A coded result, when the value is no quantity. */
  valueCode: string | null;
  /** A v3 `ObservationInterpretation` code (`H`, `L`, `N`, `A`). */
  interpretation: string | null;
  /** The reference range's low end, in `unit`. */
  refLow: number | null;
  /** The reference range's high end, in `unit`. */
  refHigh: number | null;
  status: string | null;
  components: ObservationComponent[];
  effectiveAt: string | null;
}

/** A clinical record (a lab result included) as the athlete is served it. */
export interface ServedClinicalRecord {
  id: string;
  /** The FHIR resource type (`Observation`, `Condition`, …). */
  resourceType: string;
  tier: string;
  sensitivityClass: string | null;
  effectiveAt: string | null;
  source: string;
  /** The FHIR resource as read. */
  resource: unknown;
  flat: FlatObservation | null;
  derivation: string;
  /** What read it, for a lab report's record (`rules`, `model:<name>`). */
  readBy: string | null;
}

/** A self-reported cycle-tracking entry. */
export interface ServedSelfReport {
  id: string;
  kind: string;
  start: string;
  end: string;
  valueCode: string | null;
  value: number | null;
  unit: string | null;
  cycleStart: boolean;
  source: string;
}

/** One blood glucose reading. */
export interface GlucoseReading {
  at: string;
  mgDl: number;
  source: string;
}

/** One day's blood glucose value. */
export interface GlucoseDay {
  /** `YYYY-MM-DD`. */
  date: string;
  mgDl: number;
  source: string;
}

/** `GET /api/me/health-records` — the server omits an empty list, read here as empty. */
export interface HealthRecordsPage {
  records: ServedClinicalRecord[];
  selfReports: ServedSelfReport[];
  readings: GlucoseReading[];
  daily: GlucoseDay[];
}

/** The athlete's answer to a record read from a lab report. */
export type LabReviewDecision = 'confirmed' | 'rejected';

/** One record read from a lab report, as its review shows it. */
export interface LabRecord {
  /** Its id, which confirm and reject name. */
  id: string;
  flat: FlatObservation | null;
  /** The checks it failed (`flag_inconsistent`, `date_missing`, …). */
  checks: string[];
  /** Where in the document it was read (`text:page=P;line=L`). */
  locator: string | null;
  reviewDecision: LabReviewDecision | null;
  effectiveAt: string | null;
}

/** `read`, or `no_text` for a scan with no text layer. */
export type LabDocumentStatus = 'read' | 'no_text';

/** An uploaded lab report. */
export interface LabDocument {
  id: string;
  status: LabDocumentStatus;
  pages: number;
  sizeBytes: number;
  /** What read it (`rules`, `model:<name>`). */
  engine: string | null;
  /** Lines the reader distrusted and read nothing from. */
  flaggedLines: number;
  /** Result rows seen but not coded. */
  skipped: number;
  /** A model's results dropped by verification. */
  dropped: number;
  /** Why a configured model's reading was not used. */
  modelFailure: string | null;
  receivedAt: string;
}

/** A lab report and the records read from it (`POST` answers one, `GET` lists them). */
export interface LabDocumentView {
  document: LabDocument;
  records: LabRecord[];
  /** Whether the upload stored it; `false` when the athlete already held the same file. */
  isNew: boolean;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** A field the server writes as `null` or leaves out, read as `null`; `undefined` for a wrong type. */
function optString(value: unknown): string | null | undefined {
  if (value === null || value === undefined) return null;
  return typeof value === 'string' ? value : undefined;
}

function optNumber(value: unknown): number | null | undefined {
  if (value === null || value === undefined) return null;
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined;
}

function isCount(value: unknown): value is number {
  return typeof value === 'number' && Number.isInteger(value) && value >= 0;
}

/** Every element read, or `null` when one does not. */
function parseAll<T>(values: unknown, parse: (value: unknown) => T | null): T[] | null {
  if (!Array.isArray(values)) return null;
  const out: T[] = [];
  for (const value of values) {
    const parsed = parse(value);
    if (parsed === null) return null;
    out.push(parsed);
  }
  return out;
}

/** A list the server omits when empty: absent reads as empty. */
function parseOmittable<T>(values: unknown, parse: (value: unknown) => T | null): T[] | null {
  return values === undefined ? [] : parseAll(values, parse);
}

function parseComponent(value: unknown): ObservationComponent | null {
  if (!isRecord(value)) return null;
  const code = optString(value.code);
  const quantity = optNumber(value.value);
  const unit = optString(value.unit);
  if (code === undefined || quantity === undefined || unit === undefined) return null;
  return { code, value: quantity, unit };
}

/** Read a flat Observation, or `null` for any other shape. */
export function parseFlatObservation(body: unknown): FlatObservation | null {
  if (!isRecord(body)) return null;
  const strings = {
    codeSystem: optString(body.code_system),
    code: optString(body.code),
    display: optString(body.display),
    unit: optString(body.unit),
    valueCode: optString(body.value_code),
    interpretation: optString(body.interpretation),
    status: optString(body.status),
    effectiveAt: optString(body.effective_at),
  };
  const numbers = {
    value: optNumber(body.value),
    refLow: optNumber(body.ref_low),
    refHigh: optNumber(body.ref_high),
  };
  if (Object.values(strings).includes(undefined) || Object.values(numbers).includes(undefined)) return null;
  const components = parseOmittable(body.components, parseComponent);
  if (components === null) return null;
  return {
    codeSystem: strings.codeSystem ?? null,
    code: strings.code ?? null,
    display: strings.display ?? null,
    value: numbers.value ?? null,
    unit: strings.unit ?? null,
    valueCode: strings.valueCode ?? null,
    interpretation: strings.interpretation ?? null,
    refLow: numbers.refLow ?? null,
    refHigh: numbers.refHigh ?? null,
    status: strings.status ?? null,
    components,
    effectiveAt: strings.effectiveAt ?? null,
  };
}

/**
 * A `flat` column: `null` when none was stored, and when it holds another
 * shape — the server serves a column it cannot parse as `null` too, and the
 * record stays readable by its resource either way.
 */
function parseOptionalFlat(value: unknown): FlatObservation | null {
  if (value === null || value === undefined) return null;
  return parseFlatObservation(value);
}

function parseServedRecord(value: unknown): ServedClinicalRecord | null {
  if (!isRecord(value)) return null;
  const sensitivityClass = optString(value.sensitivity_class);
  const effectiveAt = optString(value.effective_at);
  const readBy = optString(value.read_by);
  const flat = parseOptionalFlat(value.flat);
  if (
    typeof value.id !== 'string' ||
    typeof value.resource_type !== 'string' ||
    typeof value.tier !== 'string' ||
    typeof value.source !== 'string' ||
    typeof value.derivation !== 'string' ||
    !('resource' in value) ||
    sensitivityClass === undefined ||
    effectiveAt === undefined ||
    readBy === undefined
  ) {
    return null;
  }
  return {
    id: value.id,
    resourceType: value.resource_type,
    tier: value.tier,
    sensitivityClass,
    effectiveAt,
    source: value.source,
    resource: value.resource,
    flat,
    derivation: value.derivation,
    readBy,
  };
}

function parseSelfReport(value: unknown): ServedSelfReport | null {
  if (!isRecord(value)) return null;
  const valueCode = optString(value.value_code);
  const quantity = optNumber(value.value);
  const unit = optString(value.unit);
  if (
    typeof value.id !== 'string' ||
    typeof value.kind !== 'string' ||
    typeof value.start !== 'string' ||
    typeof value.end !== 'string' ||
    typeof value.cycle_start !== 'boolean' ||
    typeof value.source !== 'string' ||
    valueCode === undefined ||
    quantity === undefined ||
    unit === undefined
  ) {
    return null;
  }
  return {
    id: value.id,
    kind: value.kind,
    start: value.start,
    end: value.end,
    valueCode,
    value: quantity,
    unit,
    cycleStart: value.cycle_start,
    source: value.source,
  };
}

function parseReading(value: unknown): GlucoseReading | null {
  if (!isRecord(value) || typeof value.at !== 'string' || typeof value.source !== 'string') return null;
  const mgDl = optNumber(value.mg_dl);
  if (mgDl === null || mgDl === undefined) return null;
  return { at: value.at, mgDl, source: value.source };
}

function parseDay(value: unknown): GlucoseDay | null {
  if (!isRecord(value) || typeof value.date !== 'string' || typeof value.source !== 'string') return null;
  const mgDl = optNumber(value.mg_dl);
  if (mgDl === null || mgDl === undefined) return null;
  return { date: value.date, mgDl, source: value.source };
}

/** Read a `GET /api/me/health-records` body, or `null` for any other shape. */
export function parseHealthRecordsPage(body: unknown): HealthRecordsPage | null {
  if (!isRecord(body)) return null;
  const records = parseOmittable(body.records, parseServedRecord);
  const selfReports = parseOmittable(body.self_reports, parseSelfReport);
  const readings = parseOmittable(body.readings, parseReading);
  const daily = parseOmittable(body.daily, parseDay);
  if (records === null || selfReports === null || readings === null || daily === null) return null;
  return { records, selfReports, readings, daily };
}

const DECISIONS: readonly LabReviewDecision[] = ['confirmed', 'rejected'];

/** Read one record of a lab report, or `null` for any other shape. */
export function parseLabRecord(value: unknown): LabRecord | null {
  if (!isRecord(value) || typeof value.id !== 'string') return null;
  const flat = parseOptionalFlat(value.flat);
  const locator = optString(value.locator);
  const decision = optString(value.review_decision);
  const effectiveAt = optString(value.effective_at);
  // A checks column that did not parse server-side is served as `null`.
  const checks =
    value.checks === null || value.checks === undefined
      ? []
      : parseAll(value.checks, (check) => (typeof check === 'string' ? check : null));
  if (
    locator === undefined ||
    decision === undefined ||
    effectiveAt === undefined ||
    checks === null ||
    (decision !== null && !DECISIONS.includes(decision as LabReviewDecision))
  ) {
    return null;
  }
  return {
    id: value.id,
    flat,
    checks,
    locator,
    reviewDecision: decision as LabReviewDecision | null,
    effectiveAt,
  };
}

/** Read a `GET /api/me/health-records/review` body: the pending records, or `null` for any other shape. */
export function parseHealthRecordReview(body: unknown): LabRecord[] | null {
  if (!isRecord(body)) return null;
  return parseAll(body.pending, parseLabRecord);
}

const STATUSES: readonly LabDocumentStatus[] = ['read', 'no_text'];

function parseLabDocument(value: unknown): LabDocument | null {
  if (!isRecord(value)) return null;
  const engine = optString(value.engine);
  const modelFailure = optString(value.model_failure);
  if (
    typeof value.id !== 'string' ||
    typeof value.status !== 'string' ||
    !STATUSES.includes(value.status as LabDocumentStatus) ||
    !isCount(value.pages) ||
    !isCount(value.size_bytes) ||
    !isCount(value.flagged_lines) ||
    !isCount(value.skipped) ||
    !isCount(value.dropped) ||
    typeof value.received_at !== 'string' ||
    engine === undefined ||
    modelFailure === undefined
  ) {
    return null;
  }
  return {
    id: value.id,
    status: value.status as LabDocumentStatus,
    pages: value.pages,
    sizeBytes: value.size_bytes,
    engine,
    flaggedLines: value.flagged_lines,
    skipped: value.skipped,
    dropped: value.dropped,
    modelFailure,
    receivedAt: value.received_at,
  };
}

/** Read a lab report and its records (the upload's answer), or `null` for any other shape. */
export function parseLabDocumentView(body: unknown): LabDocumentView | null {
  if (!isRecord(body) || typeof body.new !== 'boolean') return null;
  const document = parseLabDocument(body.document);
  const records = parseAll(body.records, parseLabRecord);
  if (document === null || records === null) return null;
  return { document, records, isNew: body.new };
}

/** Read a `GET /api/me/lab-documents` body, or `null` for any other shape. */
export function parseLabDocumentList(body: unknown): LabDocumentView[] | null {
  if (!isRecord(body)) return null;
  return parseAll(body.documents, parseLabDocumentView);
}
