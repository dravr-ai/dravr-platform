// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hooks that upload a completed workout's .fit file and delete an uploaded activity, per client's athlete API
// ABOUTME: Says what happened in words keyed by the refusal's status, and refreshes every Home read the activity joins or leaves

import { useCallback, useMemo, useState } from 'react';
import { useMutation, useQueryClient, type QueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { AthleteApi } from '@pierre/api-client';
import { ACTIVITY_UPLOAD_MAX_BYTES, UPLOAD_PROVIDER, type ActivityUploadResponse } from '@pierre/shared-types';
import { classifyApiError } from './apiError';

/** Why an upload did not store anything. */
export type ActivityUploadFailure = 'tooLarge' | 'notActivity' | 'alreadyHeld' | 'failed';

/** What the last upload came to, for the line the surface shows. */
export type ActivityUploadOutcome =
  | { kind: 'uploaded'; response: ActivityUploadResponse }
  | { kind: 'refused'; failure: ActivityUploadFailure };

/** The translation key of each outcome's line. */
export const ACTIVITY_UPLOAD_KEYS = {
  uploaded: 'home.activities.upload.uploaded',
  tooLarge: 'home.activities.upload.tooLarge',
  notActivity: 'home.activities.upload.notActivity',
  alreadyHeld: 'home.activities.upload.alreadyHeld',
  failed: 'home.activities.upload.failed',
} as const;

/** The upload limit in whole megabytes, for the too-large line. */
export const ACTIVITY_UPLOAD_MAX_MEGABYTES = Math.round(ACTIVITY_UPLOAD_MAX_BYTES / (1024 * 1024));

/**
 * The failure a rejected upload names, read from its status: 413 is a file
 * past the limit, 400 one that is not a completed activity, 409 one whose
 * every session is already held. Anything else — a dropped connection, a
 * server error — is a failure worth trying again.
 */
export function activityUploadFailure(error: unknown): ActivityUploadFailure {
  const { status } = classifyApiError(error);
  if (status === 413) return 'tooLarge';
  if (status === 400) return 'notActivity';
  if (status === 409) return 'alreadyHeld';
  return 'failed';
}

/** The line an outcome is said in. */
export function describeActivityUpload(
  outcome: ActivityUploadOutcome,
  t: (key: string, params?: Record<string, string | number>) => string,
): string {
  if (outcome.kind === 'uploaded') return t(ACTIVITY_UPLOAD_KEYS.uploaded);
  if (outcome.failure === 'tooLarge') {
    return t(ACTIVITY_UPLOAD_KEYS.tooLarge, { megabytes: ACTIVITY_UPLOAD_MAX_MEGABYTES });
  }
  return t(ACTIVITY_UPLOAD_KEYS[outcome.failure]);
}

/** A file the athlete picked: its size when the picker knows it, and how to read its bytes. */
export interface PickedActivityFile {
  size?: number;
  read: () => Promise<ArrayBuffer>;
}

/** What `useActivityUpload` hands a surface. */
export interface UseActivityUploadResult {
  /**
   * Upload one picked file. A file past the limit is refused here, before
   * it is read or a byte is sent; a file that cannot be read is a failure
   * worth trying again.
   */
  uploadFile: (file: PickedActivityFile) => void;
  isUploading: boolean;
  /** The last upload's outcome, or null before the first one. */
  outcome: ActivityUploadOutcome | null;
  /** Forget the last outcome, so its line leaves the page. */
  dismiss: () => void;
}

/**
 * Refresh every Home read an uploaded activity joins or leaves: the recent
 * list, the calendar, the weekly volume and the training status.
 */
function refreshHomeReads(queryClient: QueryClient): void {
  for (const queryKey of [
    QUERY_KEYS.home.recentActivities().slice(0, 2),
    QUERY_KEYS.home.calendars,
    QUERY_KEYS.home.trainingVolume(),
    QUERY_KEYS.home.trainingStatus(),
  ]) {
    void queryClient.invalidateQueries({ queryKey });
  }
}

/**
 * Build `useActivityUpload` over one client's athlete API.
 *
 * A stored upload refreshes every Home read it joins — the recent list, the
 * calendar, the weekly volume and the training status — so the new workout
 * shows wherever the athlete looks next.
 */
export function createActivityUploadHook(athleteApi: Pick<AthleteApi, 'uploadActivityFile'>) {
  return function useActivityUpload(): UseActivityUploadResult {
    const queryClient = useQueryClient();
    const [outcome, setOutcome] = useState<ActivityUploadOutcome | null>(null);
    const mutation = useMutation({
      mutationFn: (bytes: ArrayBuffer | Uint8Array) => athleteApi.uploadActivityFile(bytes),
      onSuccess: (response) => {
        setOutcome({ kind: 'uploaded', response });
        refreshHomeReads(queryClient);
      },
      onError: (error) => setOutcome({ kind: 'refused', failure: activityUploadFailure(error) }),
    });
    const { mutate, isPending } = mutation;
    const uploadFile = useCallback(
      async (file: PickedActivityFile) => {
        const tooLarge = (size: number) => size > ACTIVITY_UPLOAD_MAX_BYTES;
        if (file.size !== undefined && tooLarge(file.size)) {
          setOutcome({ kind: 'refused', failure: 'tooLarge' });
          return;
        }
        setOutcome(null);
        let bytes: ArrayBuffer;
        try {
          bytes = await file.read();
        } catch {
          setOutcome({ kind: 'refused', failure: 'failed' });
          return;
        }
        if (tooLarge(bytes.byteLength)) {
          setOutcome({ kind: 'refused', failure: 'tooLarge' });
          return;
        }
        mutate(bytes);
      },
      [mutate],
    );
    const dismiss = useCallback(() => setOutcome(null), []);
    return useMemo(
      () => ({
        uploadFile: (file: PickedActivityFile) => void uploadFile(file),
        isUploading: isPending,
        outcome,
        dismiss,
      }),
      [uploadFile, isPending, outcome, dismiss],
    );
  };
}

/** The translation keys of the delete action, its confirmation and its failure. */
export const ACTIVITY_DELETE_KEYS = {
  action: 'home.activity.delete.action',
  actionLabel: 'home.activity.delete.actionLabel',
  confirmTitle: 'home.activity.delete.confirmTitle',
  confirmMessage: 'home.activity.delete.confirmMessage',
  failed: 'home.activity.delete.failed',
} as const;

/**
 * Whether the athlete can delete the activity filed under `provider`: only
 * their own uploads. A provider's copy is deleted on the provider, and its
 * sync removes it here.
 */
export function isDeletableActivity(provider: string): boolean {
  return provider === UPLOAD_PROVIDER;
}

/** What `useDeleteUploadedActivity` hands a surface. */
export interface UseDeleteUploadedActivityResult {
  /** Delete the uploaded activity; `onDeleted` runs once the server confirms. */
  deleteActivity: (activityId: string) => void;
  isDeleting: boolean;
  /** The last delete failed; the surface says so and offers it again. */
  failed: boolean;
}

/**
 * Build `useDeleteUploadedActivity` over one client's athlete API.
 *
 * A deleted activity leaves every Home read it was in — the recent list, the
 * calendar, the weekly volume and the training status — so those are
 * refreshed, and its own view's answer is dropped rather than kept for a
 * return visit. `onDeleted` is where the surface leaves the deleted
 * activity's view.
 */
export function createDeleteUploadedActivityHook(athleteApi: Pick<AthleteApi, 'deleteUploadedActivity'>) {
  return function useDeleteUploadedActivity(onDeleted: () => void): UseDeleteUploadedActivityResult {
    const queryClient = useQueryClient();
    const mutation = useMutation({
      mutationFn: (activityId: string) => athleteApi.deleteUploadedActivity(activityId),
      onSuccess: (_, activityId) => {
        queryClient.removeQueries({ queryKey: QUERY_KEYS.home.activityDetail(UPLOAD_PROVIDER, activityId) });
        refreshHomeReads(queryClient);
        onDeleted();
      },
    });
    const { mutate, isPending, isError } = mutation;
    return useMemo(
      () => ({
        deleteActivity: (activityId: string) => mutate(activityId),
        isDeleting: isPending,
        failed: isError,
      }),
      [mutate, isPending, isError],
    );
  };
}
