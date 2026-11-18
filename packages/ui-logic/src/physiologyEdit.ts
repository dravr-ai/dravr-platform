// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Setting one anchor from Settings › Physiology or a coach's view of an athlete — the form, its pace entry, and the saves
// ABOUTME: A speed is typed as the screen shows it, m:ss per km or mi on the run and per 100 m in the pool; the server converts (carnet#1561)

import { useMutation, useQuery, useQueryClient, type UseMutationResult } from '@tanstack/react-query';
import { PHYSIOLOGY_EDIT_KEY, QUERY_KEYS } from '@pierre/shared-constants';
import type { AthleteApi, ChatApi } from '@pierre/api-client';
import { METRES_PER_KILOMETRE, METRES_PER_MILE, type DistanceUnit } from '@pierre/chat-utils';
import type {
  AnchorConfidence,
  AnchorKind,
  AnchorUnit,
  OfferedFamily,
  PaceDistance,
  PhysiologyAnchor,
  PhysiologyAnchorEdit,
  PhysiologyResponse,
  SportAnchorName,
  SportFamily,
} from '@pierre/shared-types';
import { formatClock } from './activityView';
import { COACH_SHARING_REVOKED, planRefusalCode } from './coachPlanHooks';
import type { Translate } from './homeFormat';
import type { PhysiologyPlatform } from './physiology';
import { useDistanceUnit } from './unitsHook';

/** The metres in each distance a pace is entered per, as the server counts them. */
const PACE_METRES: Record<PaceDistance, number> = {
  km: METRES_PER_KILOMETRE,
  mi: METRES_PER_MILE,
  '100m': 100,
};

/** The distance a pace is written per, as the pane writes it. */
const PACE_DISTANCE_LABEL: Record<PaceDistance, string> = { km: 'km', mi: 'mi', '100m': '100 m' };

/** Seconds in a minute, for reading a typed pace. */
const SECONDS_PER_MINUTE = 60;

/** A pace typed as minutes and seconds: `4:10`, `12:05`. */
const PACE_TEXT = /^(\d{1,2}):([0-5]\d)$/;

/** A number typed with a dot or a comma for its decimals: `285`, `1.25`, `1,25`. */
const NUMBER_TEXT = /^\d+(?:[.,]\d+)?$/;

/**
 * The distance a pace in `sport` is entered per, as the screen shows it: per
 * 100 m in the pool in either unit system, per kilometre or mile on the run as
 * `units` reads; `null` on the bike, which holds no speed anchor.
 */
export function paceDistanceFor(sport: SportFamily, units: DistanceUnit): PaceDistance | null {
  if (sport === 'swim') return '100m';
  if (sport === 'run') return units === 'imperial' ? 'mi' : 'km';
  return null;
}

/** The whole seconds of pace per `per` at `speed` metres per second. */
export function paceSeconds(speed: number, per: PaceDistance): number {
  return Math.round(PACE_METRES[per] / speed);
}

/** The seconds a typed `m:ss` pace stands for, or `null` for any other text. */
export function parsePace(text: string): number | null {
  const match = PACE_TEXT.exec(text.trim());
  if (match === null) return null;
  const seconds = Number(match[1]) * SECONDS_PER_MINUTE + Number(match[2]);
  return seconds > 0 ? seconds : null;
}

/** The number typed, or `null` for any other text. */
export function parseEnteredNumber(text: string): number | null {
  const trimmed = text.trim();
  if (!NUMBER_TEXT.test(trimmed)) return null;
  return Number(trimmed.replace(',', '.'));
}

/** The form that sets one anchor, every field as typed. */
export interface AnchorForm {
  sport: SportFamily;
  anchor: SportAnchorName;
  unit: AnchorUnit;
  /** Watts or bpm as a number, or a speed as an `m:ss` pace. */
  value: string;
  /** The range's lower entry, as `value` is typed; empty for none. */
  low: string;
  /** The range's higher entry, as `value` is typed; empty for none. */
  high: string;
  /** How it was found: asked of the editor, never guessed, so `null` until chosen. */
  kind: AnchorKind | null;
  confidence: AnchorConfidence;
  method: string;
  /** `YYYY-MM-DD`, or empty for today. */
  measuredOn: string;
}

/** A stored value as the form shows it: a whole number, or a pace. */
function entryText(unit: AnchorUnit, value: number, per: PaceDistance | null): string {
  if (unit === 'metres_per_second' && per !== null) return formatClock(paceSeconds(value, per));
  return String(Math.round(value));
}

/**
 * The form for a new anchor: empty, at low confidence — the confidence the
 * platform takes when the editor states none — and dated today.
 */
export function newAnchorForm(sport: SportFamily, anchor: SportAnchorName, unit: AnchorUnit): AnchorForm {
  return { sport, anchor, unit, value: '', low: '', high: '', kind: null, confidence: 'low', method: '', measuredOn: '' };
}

/**
 * The form for a stored anchor: its value and range as the screen shows them
 * in `units` — a pace range runs from its faster end, the higher speed — its
 * stored confidence and method. Its day starts empty: a value changed now is
 * dated today unless the editor names another day.
 */
export function storedAnchorForm(sport: SportFamily, stored: PhysiologyAnchor, units: DistanceUnit): AnchorForm {
  const per = stored.unit === 'metres_per_second' ? paceDistanceFor(sport, units) : null;
  const isPace = per !== null;
  // A pace's fewer seconds are the higher speed.
  const lowEnd = isPace ? stored.high : stored.low;
  const highEnd = isPace ? stored.low : stored.high;
  return {
    sport,
    anchor: stored.anchor,
    unit: stored.unit,
    value: entryText(stored.unit, stored.value, per),
    low: lowEnd === null ? '' : entryText(stored.unit, lowEnd, per),
    high: highEnd === null ? '' : entryText(stored.unit, highEnd, per),
    kind: null,
    confidence: stored.confidence,
    method: stored.method ?? '',
    measuredOn: '',
  };
}

/** What the form opens on: a stored anchor to change, or a new one to add. */
export type AnchorFormTarget = { mode: 'edit'; sport: SportFamily; anchor: PhysiologyAnchor } | { mode: 'add' };

/**
 * The form `target` opens on, read in `units`: a stored anchor as the screen
 * shows it, or a new one on the first anchor the server offers; `null` when it
 * offers none.
 */
export function initialAnchorForm(
  target: AnchorFormTarget,
  offered: OfferedFamily[],
  units: DistanceUnit,
): AnchorForm | null {
  if (target.mode === 'edit') return storedAnchorForm(target.sport, target.anchor, units);
  const family = offered.find((entry) => entry.anchors.length > 0);
  const first = family?.anchors[0];
  return family && first ? newAnchorForm(family.sport, first.anchor, first.unit) : null;
}

/**
 * The unit a value is typed in, as the value's label names it: `W`, `bpm`, or
 * for a speed the `m:ss` pace per `per` in `t`'s words.
 */
export function anchorEntryUnit(t: Translate, unit: AnchorUnit, per: PaceDistance | null): string {
  if (unit === 'watts') return 'W';
  if (unit === 'beats_per_minute') return 'bpm';
  return t(PHYSIOLOGY_EDIT_KEY.paceUnit, { distance: per === null ? '' : PACE_DISTANCE_LABEL[per] });
}

/** A field the form refuses, and why. */
export interface AnchorFormProblem {
  field: 'value' | 'low' | 'high' | 'kind';
  reason: 'invalidNumber' | 'invalidPace' | 'kindRequired';
}

/** What reading the form gave: the edit to send, or the first field it refuses. */
export type AnchorFormRead = { ok: true; edit: PhysiologyAnchorEdit } | { ok: false; problem: AnchorFormProblem };

/**
 * The edit `form` states, read in `units`, or the first field it refuses: a
 * value that is not a number (or, for a speed, an `m:ss` pace), a range end
 * that is neither, or no answer to how it was found. The range's two entries
 * are sent lower number first, whichever order they were typed in. Bounds,
 * relationships to the other anchors and the day are the server's to check.
 */
export function readAnchorForm(form: AnchorForm, units: DistanceUnit): AnchorFormRead {
  const per = form.unit === 'metres_per_second' ? paceDistanceFor(form.sport, units) : null;
  const reason = per === null ? 'invalidNumber' : 'invalidPace';
  const parse = (text: string) => (per === null ? parseEnteredNumber(text) : parsePace(text));
  const value = parse(form.value);
  if (value === null) return { ok: false, problem: { field: 'value', reason } };
  const ends: number[] = [];
  for (const field of ['low', 'high'] as const) {
    if (form[field].trim() === '') continue;
    const end = parse(form[field]);
    if (end === null) return { ok: false, problem: { field, reason } };
    ends.push(end);
  }
  if (form.kind === null) return { ok: false, problem: { field: 'kind', reason: 'kindRequired' } };
  ends.sort((a, b) => a - b);
  let low: number | null = null;
  let high: number | null = null;
  if (ends.length === 2) {
    [low, high] = ends;
  } else if (ends.length === 1) {
    // One end alone bounds the side of the value it falls on.
    if (ends[0] <= value) low = ends[0];
    else high = ends[0];
  }
  const method = form.method.trim();
  return {
    ok: true,
    edit: {
      value,
      low,
      high,
      pace_per: per,
      kind: form.kind,
      confidence: form.confidence,
      method: method === '' ? null : method,
      measured_on: form.measuredOn.trim() === '' ? null : form.measuredOn.trim(),
    },
  };
}

/** One anchor to set: which, and the edit. */
export interface AnchorSave {
  sport: SportFamily;
  anchor: SportAnchorName;
  edit: PhysiologyAnchorEdit;
}

/**
 * Build `usePhysiologyEdit` over one client's athlete API: the athlete sets
 * one of their own anchors, and the pane's read takes the server's answer —
 * the anchors re-read after the save — rather than a guess.
 */
export function createPhysiologyEditHook(
  athleteApi: Pick<AthleteApi, 'updatePhysiologyAnchor'>,
  { useLanguage }: PhysiologyPlatform,
) {
  return function usePhysiologyEdit(): UseMutationResult<PhysiologyResponse, Error, AnchorSave> {
    const language = useLanguage();
    const units = useDistanceUnit();
    const queryClient = useQueryClient();
    return useMutation<PhysiologyResponse, Error, AnchorSave>({
      mutationFn: ({ sport, anchor, edit }) => athleteApi.updatePhysiologyAnchor(sport, anchor, edit, language),
      onSuccess: (physiology) => {
        queryClient.setQueryData(QUERY_KEYS.user.physiology(language, units), physiology);
      },
    });
  };
}

/** What a coach's read of one athlete's thresholds gave. */
export type AthletePhysiologyRead = { status: 'ok'; physiology: PhysiologyResponse } | { status: 'consentRevoked' };

/** What `useAthletePhysiology` hands the coach's view. */
export interface UseAthletePhysiologyResult {
  read: AthletePhysiologyRead | undefined;
  isLoading: boolean;
  isError: boolean;
  error: unknown;
  refetch: () => void;
}

/**
 * Build the coach's hooks over one client's chat API: the read of one coached
 * athlete's thresholds from Group info, and the save of one of them
 * (carnet#1561). Nothing is read until the view names an athlete.
 */
export function createCoachPhysiologyHooks(
  chatApi: Pick<ChatApi, 'getAthletePhysiology' | 'updateAthletePhysiologyAnchor'>,
  { useLanguage }: PhysiologyPlatform,
) {
  /**
   * The athlete's thresholds as their coach reads them. An athlete who
   * stopped sharing reads as `consentRevoked`; any other refusal fails the
   * read as usual.
   */
  function useAthletePhysiology(athleteUserId: string | null, groupId: string | null): UseAthletePhysiologyResult {
    const language = useLanguage();
    const units = useDistanceUnit();
    const query = useQuery({
      queryKey: QUERY_KEYS.chat.athletePhysiology(athleteUserId, groupId, language, units),
      queryFn: async (): Promise<AthletePhysiologyRead> => {
        try {
          return {
            status: 'ok',
            physiology: await chatApi.getAthletePhysiology(athleteUserId as string, groupId as string, language),
          };
        } catch (error) {
          if (planRefusalCode(error) === COACH_SHARING_REVOKED) return { status: 'consentRevoked' };
          throw error;
        }
      },
      enabled: athleteUserId !== null && groupId !== null,
    });
    return {
      read: query.data,
      isLoading: query.isLoading,
      isError: query.isError,
      error: query.error,
      refetch: () => void query.refetch(),
    };
  }

  /** The coach sets one of the athlete's thresholds; the view takes the server's answer. */
  function useAthletePhysiologyEdit(
    athleteUserId: string,
    groupId: string,
  ): UseMutationResult<PhysiologyResponse, Error, AnchorSave> {
    const language = useLanguage();
    const units = useDistanceUnit();
    const queryClient = useQueryClient();
    return useMutation<PhysiologyResponse, Error, AnchorSave>({
      mutationFn: ({ sport, anchor, edit }) =>
        chatApi.updateAthletePhysiologyAnchor(athleteUserId, groupId, sport, anchor, edit, language),
      onSuccess: (physiology) => {
        const read: AthletePhysiologyRead = { status: 'ok', physiology };
        queryClient.setQueryData(QUERY_KEYS.chat.athletePhysiology(athleteUserId, groupId, language, units), read);
      },
    });
  }

  return { useAthletePhysiology, useAthletePhysiologyEdit };
}
