// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Delete action an uploaded activity's view carries in its header, behind a confirmation
// ABOUTME: Only the athlete's own uploads offer it; once deleted the view goes back, and a failure is said in the dialog

import { useState } from 'react';
import { Trash2 } from 'lucide-react';
import { useTranslation } from '@pierre/i18n';
import { ACTIVITY_DELETE_KEYS } from '@pierre/ui-logic';
import { ConfirmDialog, IconButton } from '../ui';
import { useDeleteUploadedActivity } from '../../hooks/useActivityDetail';

interface DeleteUploadedActivityProps {
  /** The uploaded activity's id. */
  activityId: string;
  /** Leave the deleted activity's view. */
  onDeleted: () => void;
}

export function DeleteUploadedActivity({ activityId, onDeleted }: DeleteUploadedActivityProps) {
  const { t } = useTranslation();
  const [confirming, setConfirming] = useState(false);
  const { deleteActivity, isDeleting, failed } = useDeleteUploadedActivity(() => {
    setConfirming(false);
    onDeleted();
  });
  const message = failed
    ? `${t(ACTIVITY_DELETE_KEYS.confirmMessage)} ${t(ACTIVITY_DELETE_KEYS.failed)}`
    : t(ACTIVITY_DELETE_KEYS.confirmMessage);
  return (
    <>
      <IconButton
        aria-label={t(ACTIVITY_DELETE_KEYS.actionLabel)}
        title={t(ACTIVITY_DELETE_KEYS.actionLabel)}
        onClick={() => setConfirming(true)}
        data-testid="activity-delete"
        className="ml-auto"
      >
        <Trash2 className="h-4 w-4" aria-hidden="true" />
      </IconButton>
      <ConfirmDialog
        isOpen={confirming}
        onClose={() => setConfirming(false)}
        onConfirm={() => deleteActivity(activityId)}
        title={t(ACTIVITY_DELETE_KEYS.confirmTitle)}
        message={message}
        confirmLabel={t(ACTIVITY_DELETE_KEYS.action)}
        cancelLabel={t('common.cancel')}
        variant="danger"
        isLoading={isDeleting}
      />
    </>
  );
}
