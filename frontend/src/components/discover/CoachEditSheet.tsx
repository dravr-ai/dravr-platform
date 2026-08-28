// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Discover edit sheet for one of the athlete's own coaches — loads it, saves it, deletes it
// ABOUTME: The only coach editor left outside the admin console; coach creation is the /coach create command

import { useEffect, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { coachesApi } from '../../services/api';
import { QUERY_KEYS } from '../../constants/queryKeys';
import { Button, ConfirmDialog } from '../ui';
import CoachFormModal from './CoachFormModal';
import { coachToFormData, formDataToUpdateRequest } from './coachForm';
import type { CoachFormData } from './coachForm';
import { useTranslation } from '@pierre/i18n';

/** Cache slot for one coach, under the `coaches` prefix every coach mutation invalidates. */
const coachKey = (coachId: string) => [...QUERY_KEYS.coaches.all, 'coach', coachId] as const;

export interface CoachEditSheetProps {
  /** The athlete's own coach — a personal coach or a copy installed from the store. */
  coachId: string;
  /** Called when the sheet is done: after a save, after a delete, or on cancel. */
  onClose: () => void;
}

export default function CoachEditSheet({ coachId, onClose }: CoachEditSheetProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [formData, setFormData] = useState<CoachFormData | null>(null);
  const [confirmingDelete, setConfirmingDelete] = useState(false);

  const {
    data: coach,
    isError,
    error,
  } = useQuery({
    queryKey: coachKey(coachId),
    queryFn: () => coachesApi.get(coachId),
  });

  // Hydrate once from the first coach that arrives, so a background refetch
  // never overwrites what the athlete has already typed.
  useEffect(() => {
    if (coach && formData === null) {
      setFormData(coachToFormData(coach));
    }
  }, [coach, formData]);

  const invalidateCoaches = () => {
    queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.all });
  };

  const save = useMutation({
    mutationFn: (data: CoachFormData) => coachesApi.update(coachId, formDataToUpdateRequest(data)),
    onSuccess: (updated) => {
      // The response is the stored coach, so the next open hydrates from it
      // rather than from the copy this sheet was opened on.
      queryClient.setQueryData(coachKey(coachId), updated);
      invalidateCoaches();
      onClose();
    },
  });

  const remove = useMutation({
    mutationFn: () => coachesApi.delete(coachId),
    onSuccess: () => {
      queryClient.removeQueries({ queryKey: coachKey(coachId) });
      invalidateCoaches();
      setConfirmingDelete(false);
      onClose();
    },
    onError: () => setConfirmingDelete(false),
  });

  if (isError) {
    return (
      <div className="fixed inset-0 z-50 flex items-center justify-center" role="alertdialog" aria-label={t('discover.coachLoadFailed')}>
        <div className="absolute inset-0 bg-black/50 backdrop-blur-sm" onClick={onClose} />
        <div className="relative bg-surface rounded-2xl shadow-2xl max-w-sm w-full mx-4 p-6 text-center">
          <h2 className="text-lg font-semibold text-on-surface mb-2">{t('discover.coachLoadFailedTitle')}</h2>
          <p className="text-sm text-on-surface-variant mb-4">
            {error instanceof Error && error.message ? error.message : t('discover.coachDetailMissing')}
          </p>
          <Button variant="secondary" onClick={onClose}>{t('chat.close')}</Button>
        </div>
      </div>
    );
  }

  if (formData === null) {
    return (
      <div className="fixed inset-0 z-50 flex items-center justify-center" role="status" aria-label={t('discover.loadingCoach')}>
        <div className="absolute inset-0 bg-black/50 backdrop-blur-sm" onClick={onClose} />
        <div className="relative pierre-spinner w-8 h-8" />
      </div>
    );
  }

  return (
    <>
      <CoachFormModal
        isOpen
        formData={formData}
        onFormDataChange={setFormData}
        onSubmit={() => save.mutate(formData)}
        onClose={onClose}
        isSubmitting={save.isPending}
        submitError={save.isError}
        onDelete={() => setConfirmingDelete(true)}
      />
      <ConfirmDialog
        isOpen={confirmingDelete}
        onClose={() => setConfirmingDelete(false)}
        onConfirm={() => remove.mutate()}
        title={t('discover.deleteCoachConfirm')}
        message={`Delete coach "${formData.title}"? This cannot be undone.`}
        confirmLabel="Delete"
        cancelLabel={t('common.cancel')}
        variant="danger"
        isLoading={remove.isPending}
      />
      {remove.isError && (
        <p role="alert" className="fixed bottom-4 left-1/2 -translate-x-1/2 z-50 px-4 py-2 rounded-lg bg-error/10 border border-error/30 text-sm text-error">
          {remove.error instanceof Error && remove.error.message ? remove.error.message : t('discover.deleteCoachFailed')}
        </p>
      )}
    </>
  );
}
