// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Discover edit sheet for one of the athlete's own agents — loads it, saves it, deletes it
// ABOUTME: The only agent editor left outside the admin console; agent creation is the /agent create command

import { useEffect, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { coachesApi } from '../../services/api';
import { QUERY_KEYS } from '../../constants/queryKeys';
import { Button, ConfirmDialog } from '../ui';
import CoachFormModal from './CoachFormModal';
import { coachToFormData, formDataToUpdateRequest } from './coachForm';
import type { AgentFormData } from './coachForm';
import { useTranslation } from '@pierre/i18n';

/** Cache slot for one agent, under the `coaches` prefix every agent mutation invalidates. */
const coachKey = (agentId: string) => [...QUERY_KEYS.coaches.all, 'coach', agentId] as const;

export interface AgentEditSheetProps {
  /** The athlete's own agent — a personal agent or a copy installed from the store. */
  agentId: string;
  /** Called when the sheet is done: after a save, after a delete, or on cancel. */
  onClose: () => void;
}

export default function CoachEditSheet({ agentId, onClose }: AgentEditSheetProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [formData, setFormData] = useState<AgentFormData | null>(null);
  const [confirmingDelete, setConfirmingDelete] = useState(false);

  const {
    data: coach,
    isError,
    error,
  } = useQuery({
    queryKey: coachKey(agentId),
    queryFn: () => coachesApi.get(agentId),
  });

  // Hydrate once from the first agent that arrives, so a background refetch
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
    mutationFn: (data: AgentFormData) => coachesApi.update(agentId, formDataToUpdateRequest(data)),
    onSuccess: (updated) => {
      // The response is the stored agent, so the next open hydrates from it
      // rather than from the copy this sheet was opened on.
      queryClient.setQueryData(coachKey(agentId), updated);
      invalidateCoaches();
      onClose();
    },
  });

  const remove = useMutation({
    mutationFn: () => coachesApi.delete(agentId),
    onSuccess: () => {
      queryClient.removeQueries({ queryKey: coachKey(agentId) });
      invalidateCoaches();
      setConfirmingDelete(false);
      onClose();
    },
    onError: () => setConfirmingDelete(false),
  });

  if (isError) {
    return (
      <div className="fixed inset-0 z-50 flex items-center justify-center" role="alertdialog" aria-label={t('discover.agentLoadFailed')}>
        <div className="absolute inset-0 bg-black/50 backdrop-blur-sm" onClick={onClose} />
        <div className="relative bg-surface rounded-2xl max-w-sm w-full mx-4 p-6 text-center">
          <h2 className="text-lg font-semibold text-on-surface mb-2">{t('discover.agentLoadFailedTitle')}</h2>
          <p className="text-sm text-on-surface-variant mb-4">
            {error instanceof Error && error.message ? error.message : t('discover.agentDetailMissing')}
          </p>
          <Button variant="secondary" onClick={onClose}>{t('chat.close')}</Button>
        </div>
      </div>
    );
  }

  if (formData === null) {
    return (
      <div className="fixed inset-0 z-50 flex items-center justify-center" role="status" aria-label={t('discover.loadingAgent')}>
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
        title={t('discover.deleteAgentConfirm')}
        message={t('app.confirmDeleteAgent', { coach: formData.title })}
        confirmLabel={t('common.delete')}
        cancelLabel={t('common.cancel')}
        variant="danger"
        isLoading={remove.isPending}
      />
      {remove.isError && (
        <p role="alert" className="fixed bottom-4 left-1/2 -translate-x-1/2 z-50 px-4 py-2 rounded-lg bg-error/10 border border-error/30 text-sm text-error">
          {remove.error instanceof Error && remove.error.message ? remove.error.message : t('discover.deleteAgentFailed')}
        </p>
      )}
    </>
  );
}
