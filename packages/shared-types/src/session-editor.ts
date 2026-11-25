// ABOUTME: Wire shapes of a coach's "build from family" editor — the family catalogue, a session's live preview, and the draft-day write
// ABOUTME: Mirrors the subject-plan family routes of carnet#1515; every number is the kernel's, so the clients render and never derive

import { PLAN_EVIDENCE_KINDS, PLAN_EVIDENCE_NOTE_KINDS } from './workout-plan.js';
import type { PlanDayEvidence, PlanStep } from './workout-plan.js';

/** The sport families a family preset is written for. */
export const SESSION_SPORTS = ['ride', 'run', 'swim'] as const;
export type SessionSport = (typeof SESSION_SPORTS)[number];

/** The values a coach states on a family session, in the order the editor lists them. */
export const SESSION_PARAM_KEYS = [
  'sets',
  'reps',
  'work_seconds',
  'rest_seconds',
  'set_rest_seconds',
  'duration_minutes',
] as const;
export type SessionParamKey = (typeof SESSION_PARAM_KEYS)[number];

/** Stated values, by key; a key left out takes the preset's default. */
export type SessionParams = Partial<Record<SessionParamKey, number>>;

/**
 * A preset's evidence range for one value. A value outside it is still saved
 * (founder decision D3): it earns a finding, never a refusal.
 */
export interface SessionParamRange {
  min: number;
  max: number;
  default: number;
}

/** One preset of a family: a named, evidence-bounded form of it. */
export interface FamilyPreset {
  key: string;
  /** Display name, as the catalogue writes it ("30/15 (Rønnestad)"). */
  name: string;
  /** One line on what the preset is for, when the catalogue has one. */
  summary?: string;
  /** The sports the preset has a form for. */
  sports: SessionSport[];
  /**
   * The values the coach may state, per sport the preset has a form for: the
   * kernel reads a range from the sport's form first, then the preset, then
   * the family, so one preset's ranges and defaults differ between sports. A
   * key left out is not a knob in that sport.
   */
  params: Partial<Record<SessionSport, Partial<Record<SessionParamKey, SessionParamRange>>>>;
}

/** One workout family the thread's athlete's agent offers. */
export interface WorkoutFamily {
  slug: string;
  name: string;
  /** The kernel's purpose word (`vo2max_short`, `endurance`, …). */
  purpose: string;
  /** The purpose in the reader's language, when the strings have it. */
  purpose_label?: string;
  /** The sports any of its presets has a form for. */
  sports: SessionSport[];
  /** The preset a session takes when none is named. */
  default_preset: string;
  presets: FamilyPreset[];
}

/** `GET /api/chat/conversations/{id}/subject-plan/families`. */
export interface FamilyCatalogueResponse {
  families: WorkoutFamily[];
}

/** A session as the coach builds it: what the preview measures and the draft-day write saves. */
export interface SessionBuild {
  family: string;
  /** The family's default preset when absent. */
  preset?: string;
  sport: SessionSport;
  params?: SessionParams;
}

/** `POST …/subject-plan/session-preview`: a build, for one of the athlete's days. */
export interface SessionPreviewRequest extends SessionBuild {
  /** `YYYY-MM-DD`. */
  date: string;
}

/**
 * What the kernel derives from a built session. Percentages are of the
 * athlete's aerobic maximum (`aerobic_max`: MAP or vVO2max); absent where the
 * kernel derives none for this session.
 */
export interface SessionDerived {
  sets: number;
  reps: number;
  work_seconds: number;
  rest_seconds: number;
  ratio?: number;
  work_pct?: number;
  rest_pct?: number;
  cycle_mean_pct?: number;
  amplitude_pct?: number;
  on_time_minutes: number;
  total_minutes: number;
  aerobic_max: string;
}

/** One envelope finding, worded by the server in the reader's language. */
export interface SessionFinding {
  /** The kernel's finding code. */
  code: string;
  text: string;
  /**
   * The stated value the finding is about, when it is one outside its
   * preset's range: the field the editor marks. The kernel alone decides it.
   */
  param?: SessionParamKey;
}

/** `POST …/subject-plan/session-preview`: the session the build makes, measured on the athlete's anchors. */
export interface SessionPreview {
  family: string;
  /** The preset the session resolved to. */
  preset: string;
  sport: SessionSport;
  duration_min: number;
  /** The session's structure, its targets written on the athlete's stored anchors. */
  steps: PlanStep[];
  derived?: SessionDerived;
  /** Empty when the session sits inside its family's evidence. */
  findings: SessionFinding[];
  /** The day card's "outside the evidence" warning, when a finding fired. */
  outside_evidence?: string;
  /** The day card's evidence chip, when the evidence is weaker than an unqualified trial. */
  evidence?: PlanDayEvidence;
  /** "Counts as <purpose>", when a rule relabelled the session. */
  counts_as?: string;
  /** The anchor line ("Anchor: FTP 280 W, your own."), when the targets rest on one. */
  anchor?: string;
}

/** The `build` of `PUT …/subject-plan/draft/days/{date}/{position}`: saved over that session of the coach's draft. */
export type DraftDayRequest = SessionBuild;

/**
 * A session as the coach wrote it by hand, or a rest day, saved into the
 * coach's draft (carnet#1714). A hand-written session names no family, so
 * no envelope check runs on it.
 */
export type HandDayRequest =
  | { rest: true }
  | {
      rest: false;
      /** The day's sport, in the plan's own words (`run`, `gravel`, …). */
      sport: string;
      workout: string;
      duration_min?: number;
      intensity: string;
      steps: PlanStep[];
    };

/**
 * The session a write is about, as the client last saw it on the card: a
 * date's sessions are addressed by position, so a write whose position now
 * holds another session is refused (409 `session_moved`) rather than landing
 * on the wrong one (carnet#1716).
 */
export interface SessionExpect {
  sport: string;
  workout: string;
}

/** A session a coach adds to a date: built from a family, or written by hand (carnet#1716). */
export type AddedSession = { build: SessionBuild } | { day: HandDayRequest };

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isSessionSport(value: unknown): value is SessionSport {
  return typeof value === 'string' && (SESSION_SPORTS as readonly string[]).includes(value);
}

function sports(value: unknown): SessionSport[] | null {
  if (!Array.isArray(value) || !value.every(isSessionSport)) return null;
  return value;
}

function finiteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

function optionalNumber(value: unknown): number | undefined | null {
  if (value === undefined || value === null) return undefined;
  return finiteNumber(value) ? value : null;
}

function optionalString(value: unknown): string | undefined | null {
  if (value === undefined || value === null) return undefined;
  return typeof value === 'string' ? value : null;
}

function parseRanges(value: unknown): Partial<Record<SessionParamKey, SessionParamRange>> | null {
  if (!isRecord(value)) return null;
  const ranges: Partial<Record<SessionParamKey, SessionParamRange>> = {};
  for (const key of SESSION_PARAM_KEYS) {
    const raw = value[key];
    if (raw === undefined || raw === null) continue;
    if (!isRecord(raw) || !finiteNumber(raw.min) || !finiteNumber(raw.max) || !finiteNumber(raw.default)) {
      return null;
    }
    ranges[key] = { min: raw.min, max: raw.max, default: raw.default };
  }
  return ranges;
}

function parseSportRanges(value: unknown): FamilyPreset['params'] | null {
  if (!isRecord(value)) return null;
  const bySport: FamilyPreset['params'] = {};
  for (const sport of SESSION_SPORTS) {
    const raw = value[sport];
    if (raw === undefined || raw === null) continue;
    const ranges = parseRanges(raw);
    if (ranges === null) return null;
    bySport[sport] = ranges;
  }
  return bySport;
}

function parsePreset(value: unknown): FamilyPreset | null {
  if (!isRecord(value) || typeof value.key !== 'string' || typeof value.name !== 'string') return null;
  const presetSports = sports(value.sports);
  const params = parseSportRanges(value.params);
  const summary = optionalString(value.summary);
  if (presetSports === null || params === null || summary === null) return null;
  return {
    key: value.key,
    name: value.name,
    ...(summary === undefined ? {} : { summary }),
    sports: presetSports,
    params,
  };
}

function parseFamily(value: unknown): WorkoutFamily | null {
  if (
    !isRecord(value) ||
    typeof value.slug !== 'string' ||
    typeof value.name !== 'string' ||
    typeof value.purpose !== 'string' ||
    typeof value.default_preset !== 'string' ||
    !Array.isArray(value.presets)
  ) {
    return null;
  }
  const familySports = sports(value.sports);
  const purposeLabel = optionalString(value.purpose_label);
  if (familySports === null || purposeLabel === null) return null;
  const presets: FamilyPreset[] = [];
  for (const raw of value.presets) {
    const preset = parsePreset(raw);
    if (preset === null) return null;
    presets.push(preset);
  }
  return {
    slug: value.slug,
    name: value.name,
    purpose: value.purpose,
    ...(purposeLabel === undefined ? {} : { purpose_label: purposeLabel }),
    sports: familySports,
    default_preset: value.default_preset,
    presets,
  };
}

/** The family catalogue, or `null` when the body breaks its contract. */
export function parseFamilyCatalogueResponse(body: unknown): FamilyCatalogueResponse | null {
  if (!isRecord(body) || !Array.isArray(body.families)) return null;
  const families: WorkoutFamily[] = [];
  for (const raw of body.families) {
    const family = parseFamily(raw);
    if (family === null) return null;
    families.push(family);
  }
  return { families };
}

function isPlanStep(value: unknown): value is PlanStep {
  return (
    isRecord(value) &&
    typeof value.label === 'string' &&
    finiteNumber(value.duration_seconds) &&
    typeof value.target_zone === 'string'
  );
}

function parseDerived(value: unknown): SessionDerived | null {
  if (
    !isRecord(value) ||
    !finiteNumber(value.sets) ||
    !finiteNumber(value.reps) ||
    !finiteNumber(value.work_seconds) ||
    !finiteNumber(value.rest_seconds) ||
    !finiteNumber(value.on_time_minutes) ||
    !finiteNumber(value.total_minutes) ||
    typeof value.aerobic_max !== 'string'
  ) {
    return null;
  }
  const optional = {
    ratio: optionalNumber(value.ratio),
    work_pct: optionalNumber(value.work_pct),
    rest_pct: optionalNumber(value.rest_pct),
    cycle_mean_pct: optionalNumber(value.cycle_mean_pct),
    amplitude_pct: optionalNumber(value.amplitude_pct),
  };
  if (Object.values(optional).some((v) => v === null)) return null;
  const derived: SessionDerived = {
    sets: value.sets,
    reps: value.reps,
    work_seconds: value.work_seconds,
    rest_seconds: value.rest_seconds,
    on_time_minutes: value.on_time_minutes,
    total_minutes: value.total_minutes,
    aerobic_max: value.aerobic_max,
  };
  for (const [key, v] of Object.entries(optional)) {
    if (typeof v === 'number') derived[key as keyof typeof optional] = v;
  }
  return derived;
}

function parseEvidence(value: unknown): PlanDayEvidence | undefined | null {
  if (value === undefined || value === null) return undefined;
  if (!isRecord(value) || typeof value.label !== 'string') return null;
  const basis = optionalString(value.basis);
  const note = optionalString(value.note);
  if (basis === null || note === null) return null;
  // A kind this client does not know names no words it holds: the label still reads.
  const kind = PLAN_EVIDENCE_KINDS.find((known) => known === value.kind);
  const noteKind = PLAN_EVIDENCE_NOTE_KINDS.find((known) => known === value.note_kind);
  return {
    label: value.label,
    ...(kind === undefined ? {} : { kind }),
    ...(basis === undefined ? {} : { basis }),
    ...(note === undefined ? {} : { note }),
    ...(noteKind === undefined ? {} : { note_kind: noteKind }),
  };
}

/** A session preview, or `null` when the body breaks its contract. */
export function parseSessionPreview(body: unknown): SessionPreview | null {
  if (
    !isRecord(body) ||
    typeof body.family !== 'string' ||
    typeof body.preset !== 'string' ||
    !isSessionSport(body.sport) ||
    !finiteNumber(body.duration_min) ||
    !Array.isArray(body.steps) ||
    !body.steps.every(isPlanStep) ||
    !Array.isArray(body.findings)
  ) {
    return null;
  }
  const findings: SessionFinding[] = [];
  for (const raw of body.findings) {
    if (!isRecord(raw) || typeof raw.code !== 'string' || typeof raw.text !== 'string') return null;
    const param = optionalString(raw.param);
    if (param === null) return null;
    // A key this client does not know names no field it shows: the sentence still reads.
    const known = SESSION_PARAM_KEYS.find((key) => key === param);
    findings.push({ code: raw.code, text: raw.text, ...(known === undefined ? {} : { param: known }) });
  }
  const derivedOmitted = body.derived === undefined || body.derived === null;
  const derived = derivedOmitted ? undefined : parseDerived(body.derived);
  const evidence = parseEvidence(body.evidence);
  const outside = optionalString(body.outside_evidence);
  const countsAs = optionalString(body.counts_as);
  const anchor = optionalString(body.anchor);
  if (derived === null || evidence === null || outside === null || countsAs === null || anchor === null) {
    return null;
  }
  return {
    family: body.family,
    preset: body.preset,
    sport: body.sport,
    duration_min: body.duration_min,
    steps: body.steps,
    ...(derived === undefined ? {} : { derived }),
    findings,
    ...(outside === undefined ? {} : { outside_evidence: outside }),
    ...(evidence === undefined ? {} : { evidence }),
    ...(countsAs === undefined ? {} : { counts_as: countsAs }),
    ...(anchor === undefined ? {} : { anchor }),
  };
}
