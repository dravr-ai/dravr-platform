// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Gear ledger domain API — the shoes the athlete declares, retires or deletes, and the pair each activity was run in
// ABOUTME: Each answer is checked with its shared-types parser, so a malformed body is an error rather than a pair drawn from guesses

import type { AxiosInstance } from 'axios';
import {
  parseActivityGearResponse,
  parseGearItem,
  parseGearListResponse,
  type ActivityGearResponse,
  type CreateGearRequest,
  type GearItem,
  type GearListResponse,
  type UpdateGearRequest,
} from '@pierre/shared-types';
import { ENDPOINTS } from '../core/endpoints';
import { requireShape } from '../core/shape';

export type { ActivityGearResponse, CreateGearRequest, GearItem, GearListResponse, UpdateGearRequest };

/**
 * Creates the gear ledger API methods bound to an axios instance.
 *
 * The ledger is the athlete's alone: another athlete's pair or activity
 * answers 404, like one that does not exist.
 */
export function createGearApi(axios: AxiosInstance) {
  return {
    /**
     * Every pair the athlete declared, retired ones included, newest first.
     * Reading it first gives every activity the athlete has not attributed
     * its sport's default pair, so the distances it answers are settled.
     */
    async listGear(): Promise<GearListResponse> {
      const response = await axios.get<unknown>(ENDPOINTS.GEAR.LIST);
      return requireShape(parseGearListResponse(response.data), ENDPOINTS.GEAR.LIST);
    },

    /**
     * Declare a pair. Rejects with 400 for a field out of bounds (a blank or
     * too-long name, a distance past `GEAR_MAX_DISTANCE_METERS`) and once the
     * athlete holds the most pairs the server keeps.
     */
    async createGear(request: CreateGearRequest): Promise<GearItem> {
      const response = await axios.post<unknown>(ENDPOINTS.GEAR.LIST, request);
      return requireShape(parseGearItem(response.data), ENDPOINTS.GEAR.LIST);
    },

    /** Change what was declared about a pair; its history stays. */
    async updateGear(gearId: string, request: UpdateGearRequest): Promise<GearItem> {
      const endpoint = ENDPOINTS.GEAR.ITEM(gearId);
      const response = await axios.patch<unknown>(endpoint, request);
      return requireShape(parseGearItem(response.data), endpoint);
    },

    /** Retire a pair: it keeps its distance and history and is offered for no activity. */
    async retireGear(gearId: string): Promise<GearItem> {
      const endpoint = ENDPOINTS.GEAR.RETIRE(gearId);
      const response = await axios.post<unknown>(endpoint);
      return requireShape(parseGearItem(response.data), endpoint);
    },

    /** Put a retired pair back in use. */
    async restoreGear(gearId: string): Promise<GearItem> {
      const endpoint = ENDPOINTS.GEAR.RESTORE(gearId);
      const response = await axios.post<unknown>(endpoint);
      return requireShape(parseGearItem(response.data), endpoint);
    },

    /**
     * Delete a pair no activity is attributed to. Rejects with 409 when one
     * is: the pair is retired instead, which keeps its history.
     */
    async deleteGear(gearId: string): Promise<void> {
      await axios.delete(ENDPOINTS.GEAR.ITEM(gearId));
    },

    /**
     * The pair one activity was run in, addressed by the provider and id its
     * Home row names. `source` is `default` when the ledger gave it its
     * sport's default, `declared` when the athlete chose, and `null` when no
     * pair is attributed.
     */
    async getActivityGear(provider: string, activityId: string): Promise<ActivityGearResponse> {
      const endpoint = ENDPOINTS.GEAR.ACTIVITY(provider, activityId);
      const response = await axios.get<unknown>(endpoint);
      return requireShape(parseActivityGearResponse(response.data), endpoint);
    },

    /**
     * Record the pair the activity was run in, or `null` for none of the
     * athlete's — which no default overrides later. Rejects with 400 for a
     * retired pair.
     */
    async setActivityGear(
      provider: string,
      activityId: string,
      gearId: string | null,
    ): Promise<ActivityGearResponse> {
      const endpoint = ENDPOINTS.GEAR.ACTIVITY(provider, activityId);
      const response = await axios.put<unknown>(endpoint, { gear_id: gearId });
      return requireShape(parseActivityGearResponse(response.data), endpoint);
    },
  };
}

export type GearApi = ReturnType<typeof createGearApi>;
