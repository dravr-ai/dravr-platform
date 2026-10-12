// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The gear ledger both clients render — the React Query hooks over each client's gear API, and the rules a pair's form and picker follow
// ABOUTME: Distances read and typed in the athlete's units (km or mi), the bounds the server holds a pair to, and which pairs an activity is offered

import { useMemo } from 'react';
import {
  useMutation,
  useQuery,
  useQueryClient,
  type UseMutationResult,
  type UseQueryResult,
} from '@tanstack/react-query';
import {
  distanceInUnit,
  distanceSymbol,
  formatCount,
  metresPerDistanceUnit,
  type DistanceUnit,
} from '@pierre/chat-utils';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { GearApi } from '@pierre/api-client';
import {
  GEAR_DEFAULT_WARN_DISTANCE_METERS,
  GEAR_MAX_DISTANCE_METERS,
  GEAR_NAME_MAX_CHARS,
  type ActivityGearResponse,
  type CreateGearRequest,
  type GearItem,
  type GearListResponse,
  type GearSport,
  type UpdateGearRequest,
} from '@pierre/shared-types';

/**
 * Catalogue keys of the gear ledger's sentences that the shared rules below
 * produce, so both clients word a refused field the same way.
 */
export const GEAR_FORM_KEYS = {
  nameRequired: 'gear.form.nameRequired',
  nameTooLong: 'gear.form.nameTooLong',
  distanceInvalid: 'gear.form.distanceInvalid',
  warnInvalid: 'gear.form.warnInvalid',
} as const;

/**
 * A pair's distance as the ledger prints it: whole kilometres or miles, as
 * the athlete reads distances, grouped as `language` groups them —
 * `512 km`, `318 mi`.
 */
export function formatGearDistance(meters: number, unit: DistanceUnit, language: string): string {
  return `${formatCount(Math.round(distanceInUnit(meters, unit)), language)} ${distanceSymbol(unit)}`;
}

/**
 * The server's bound on a pair's distances ({@link GEAR_MAX_DISTANCE_METERS}),
 * rounded down to a whole kilometre or mile, in metres: the figure a refused
 * distance field names, so the athlete can type the largest one it says.
 */
export function gearDistanceLimit(unit: DistanceUnit): number {
  const perUnit = metresPerDistanceUnit(unit);
  return Math.floor(GEAR_MAX_DISTANCE_METERS / perUnit) * perUnit;
}

/**
 * What the athlete typed into a distance field, in metres, or `null` when it
 * is not a number of kilometres (or of miles, in `imperial`). A comma reads
 * as the decimal mark, as French and German write it.
 */
export function metersFromDistanceInput(typed: string, unit: DistanceUnit): number | null {
  const text = typed.trim().replace(',', '.');
  if (text === '' || !/^\d+(\.\d+)?$/.test(text)) return null;
  return Math.round(Number(text) * metresPerDistanceUnit(unit));
}

/** A distance in metres as a distance field shows it for editing, in `unit`: at most one decimal. */
export function distanceInputFromMeters(meters: number, unit: DistanceUnit): string {
  return String(Math.round(distanceInUnit(meters, unit) * 10) / 10);
}

/** What a pair's form holds, as typed. */
export interface GearDraft {
  name: string;
  sport: GearSport;
  /** The units the distances are typed in: the athlete's. */
  unit: DistanceUnit;
  /** The distance the pair already has, as typed in `unit`; blank for none. */
  initialDistance: string;
  /** The distance at which the pair reaches its warning, as typed in `unit`. */
  warnDistance: string;
  pinned: boolean;
  /**
   * The distances the form opened on, in metres. A field still showing what
   * it opened with keeps that distance exactly, so a figure rounded to a
   * tenth of a mile neither moves a pair's distance nor reads as an edit.
   */
  opened: { initialMeters: number; warnMeters: number };
}

/** An empty form for a new pair of `sport`, typed in `unit`, its warning at the server's default. */
export function emptyGearDraft(sport: GearSport, unit: DistanceUnit): GearDraft {
  return {
    name: '',
    sport,
    unit,
    initialDistance: '',
    warnDistance: distanceInputFromMeters(GEAR_DEFAULT_WARN_DISTANCE_METERS, unit),
    pinned: false,
    opened: { initialMeters: 0, warnMeters: GEAR_DEFAULT_WARN_DISTANCE_METERS },
  };
}

/** The form a pair is edited in, holding what was declared about it, typed in `unit`. */
export function gearDraftOf(pair: GearItem, unit: DistanceUnit): GearDraft {
  return {
    name: pair.name,
    sport: pair.sport,
    unit,
    initialDistance: distanceInputFromMeters(pair.initial_distance_meters, unit),
    warnDistance: distanceInputFromMeters(pair.warn_distance_meters, unit),
    pinned: pair.pinned,
    opened: { initialMeters: pair.initial_distance_meters, warnMeters: pair.warn_distance_meters },
  };
}

/** A distance field's value in metres: the distance it opened on while it still shows it, else what was typed. */
function typedMeters(typed: string, unit: DistanceUnit, openedMeters: number): number | null {
  if (typed.trim() === distanceInputFromMeters(openedMeters, unit)) return openedMeters;
  return metersFromDistanceInput(typed, unit);
}

/** The catalogue key of each field the draft holds out of bounds. */
export interface GearDraftErrors {
  name?: string;
  initialDistance?: string;
  warnDistance?: string;
}

/** A draft read against the server's bounds: the request it makes, or why it cannot be sent. */
export type GearDraftCheck =
  | { ok: true; request: Required<CreateGearRequest> }
  | { ok: false; errors: GearDraftErrors };

/**
 * Check a draft against the bounds the server holds a pair to — a name of 1 to
 * {@link GEAR_NAME_MAX_CHARS} characters once trimmed, a declared distance of
 * 0 to {@link GEAR_MAX_DISTANCE_METERS}, a warning above 0 and at most that —
 * so a refused field is said beside it rather than by a 400.
 */
export function checkGearDraft(draft: GearDraft): GearDraftCheck {
  const errors: GearDraftErrors = {};
  const name = draft.name.trim();
  if (name === '') {
    errors.name = GEAR_FORM_KEYS.nameRequired;
  } else if ([...name].length > GEAR_NAME_MAX_CHARS) {
    errors.name = GEAR_FORM_KEYS.nameTooLong;
  }
  const initial =
    draft.initialDistance.trim() === ''
      ? 0
      : typedMeters(draft.initialDistance, draft.unit, draft.opened.initialMeters);
  if (initial === null || initial > GEAR_MAX_DISTANCE_METERS) {
    errors.initialDistance = GEAR_FORM_KEYS.distanceInvalid;
  }
  const warn = typedMeters(draft.warnDistance, draft.unit, draft.opened.warnMeters);
  if (warn === null || warn <= 0 || warn > GEAR_MAX_DISTANCE_METERS) {
    errors.warnDistance = GEAR_FORM_KEYS.warnInvalid;
  }
  if (Object.keys(errors).length > 0 || initial === null || warn === null) {
    return { ok: false, errors };
  }
  return {
    ok: true,
    request: {
      name,
      sport: draft.sport,
      initial_distance_meters: initial,
      warn_distance_meters: warn,
      pinned: draft.pinned,
    },
  };
}

/**
 * The fields of a checked draft that differ from what `pair` declares, so an
 * edit sends only what changed. Empty when nothing did.
 */
export function gearChanges(pair: GearItem, request: Required<CreateGearRequest>): UpdateGearRequest {
  const changes: UpdateGearRequest = {};
  if (request.name !== pair.name) changes.name = request.name;
  if (request.sport !== pair.sport) changes.sport = request.sport;
  if (request.initial_distance_meters !== pair.initial_distance_meters) {
    changes.initial_distance_meters = request.initial_distance_meters;
  }
  if (request.warn_distance_meters !== pair.warn_distance_meters) {
    changes.warn_distance_meters = request.warn_distance_meters;
  }
  if (request.pinned !== pair.pinned) changes.pinned = request.pinned;
  return changes;
}

/** The pairs in use and the retired ones, each in the ledger's order (newest first). */
export function splitGear(ledger: readonly GearItem[]): { inUse: GearItem[]; retired: GearItem[] } {
  return {
    inUse: ledger.filter((pair) => pair.retired_at === null),
    retired: ledger.filter((pair) => pair.retired_at !== null),
  };
}

/**
 * The pairs an activity's picker offers: every pair in use, and the pair the
 * activity is attributed to when that one was retired since — shown so the
 * picker says what is recorded, though it can no longer be chosen.
 */
export function gearChoices(ledger: readonly GearItem[], attributedId: string | null): GearItem[] {
  return ledger.filter((pair) => pair.retired_at === null || pair.id === attributedId);
}

/** The hooks `createGearHooks` builds over one client's gear API. */
export interface GearHooks {
  /** The athlete's ledger: every pair, retired ones included, newest first. */
  useGearLedger: () => UseQueryResult<GearListResponse>;
  /** Declare, change, retire, restore and delete pairs; each settles every gear read. */
  useGearChanges: () => GearChanges;
  /** The pair one activity was run in, and the athlete's choice of it. */
  useActivityGear: (provider: string, activityId: string, enabled: boolean) => ActivityGearState;
}

/** The ledger's mutations, as `useGearChanges` hands them over. */
export interface GearChanges {
  create: UseMutationResult<GearItem, Error, CreateGearRequest>;
  update: UseMutationResult<GearItem, Error, { gearId: string; changes: UpdateGearRequest }>;
  retire: UseMutationResult<GearItem, Error, string>;
  restore: UseMutationResult<GearItem, Error, string>;
  remove: UseMutationResult<void, Error, string>;
}

/** One activity's pair, as `useActivityGear` hands it over. */
export interface ActivityGearState {
  /** What is attributed, or `undefined` until the server has answered. */
  attribution: ActivityGearResponse | undefined;
  isLoading: boolean;
  isError: boolean;
  /** Why the read failed, when it did. */
  error: unknown;
  /** Record the athlete's choice: a pair's id, or `null` for none of theirs. */
  choose: (gearId: string | null) => void;
  isChoosing: boolean;
  /** The last choice was refused or did not land. */
  chooseFailed: boolean;
  chooseError: unknown;
}

/**
 * Build the gear ledger hooks over one client's gear API.
 *
 * Any change to the ledger invalidates every gear read — the pairs' distances
 * and which pair is each sport's default move together — and a choice for an
 * activity is written into that activity's read at once, then the ledger is
 * read again for the distance it moved.
 */
export function createGearHooks(
  gearApi: Pick<
    GearApi,
    | 'listGear'
    | 'createGear'
    | 'updateGear'
    | 'retireGear'
    | 'restoreGear'
    | 'deleteGear'
    | 'getActivityGear'
    | 'setActivityGear'
  >,
): GearHooks {
  function useGearLedger() {
    return useQuery({ queryKey: QUERY_KEYS.gear.list(), queryFn: () => gearApi.listGear() });
  }

  function useGearChanges(): GearChanges {
    const queryClient = useQueryClient();
    const settle = () => queryClient.invalidateQueries({ queryKey: QUERY_KEYS.gear.all });
    const create = useMutation({
      mutationFn: (request: CreateGearRequest) => gearApi.createGear(request),
      onSuccess: settle,
    });
    const update = useMutation({
      mutationFn: ({ gearId, changes }: { gearId: string; changes: UpdateGearRequest }) =>
        gearApi.updateGear(gearId, changes),
      onSuccess: settle,
    });
    const retire = useMutation({ mutationFn: (gearId: string) => gearApi.retireGear(gearId), onSuccess: settle });
    const restore = useMutation({ mutationFn: (gearId: string) => gearApi.restoreGear(gearId), onSuccess: settle });
    const remove = useMutation({ mutationFn: (gearId: string) => gearApi.deleteGear(gearId), onSuccess: settle });
    return { create, update, retire, restore, remove };
  }

  function useActivityGear(provider: string, activityId: string, enabled: boolean): ActivityGearState {
    const queryClient = useQueryClient();
    const key = QUERY_KEYS.gear.activity(provider, activityId);
    const query = useQuery({
      queryKey: key,
      queryFn: () => gearApi.getActivityGear(provider, activityId),
      enabled,
    });
    const mutation = useMutation({
      mutationFn: (gearId: string | null) => gearApi.setActivityGear(provider, activityId, gearId),
      onSuccess: (answer) => {
        queryClient.setQueryData(key, answer);
        void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.gear.list() });
      },
    });
    const { mutate, isPending, isError, error } = mutation;
    return useMemo(
      () => ({
        attribution: query.data,
        isLoading: query.isLoading,
        isError: query.isError,
        error: query.error,
        choose: (gearId: string | null) => mutate(gearId),
        isChoosing: isPending,
        chooseFailed: isError,
        chooseError: error,
      }),
      [query.data, query.isLoading, query.isError, query.error, mutate, isPending, isError, error],
    );
  }

  return { useGearLedger, useGearChanges, useActivityGear };
}
