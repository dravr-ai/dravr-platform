// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A coached athlete's thresholds per sport, opened from a Group info member card, where their coach sets one (carnet#1561)
// ABOUTME: Worded for the coach; a value saved is stored as set by a coach and the athlete is told, through the one physiology writer

import { useState } from 'react';
import type { GroupMember } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { COACH_PHYSIOLOGY_KEY, PHYSIOLOGY_EDIT_KEY } from '@pierre/shared-constants';
import { describeApiError, type AnchorFormTarget } from '@pierre/ui-logic';
import { useAthletePhysiology, useAthletePhysiologyEdit } from '../../hooks/useCoachPhysiology';
import PhysiologyAnchorEditor from '../settings/PhysiologyAnchorEditor';
import PhysiologyAnchorList from '../settings/PhysiologyAnchorList';
import { Button, Modal } from '../ui';

interface AthletePhysiologyModalProps {
  /** The athlete whose thresholds are open; `null` keeps the modal closed. */
  member: GroupMember | null;
  /** The group the caller coaches them in. */
  groupId: string;
  onClose: () => void;
}

/** The thresholds of `member`, read and set as their coach. */
function AthletePhysiologyBody({ member, groupId }: { member: GroupMember; groupId: string }) {
  const { t } = useTranslation();
  const { read, isLoading, isError, error, refetch } = useAthletePhysiology(member.user_id, groupId);
  const save = useAthletePhysiologyEdit(member.user_id, groupId);
  const [target, setTarget] = useState<AnchorFormTarget | null>(null);
  const [saved, setSaved] = useState(false);
  const name = member.display_name ?? t('groups.memberFallback');

  if (isLoading) {
    return (
      <div className="flex justify-center py-10">
        <div className="pierre-spinner" role="status" aria-label={t('common.loading')} />
      </div>
    );
  }
  if (isError) {
    return (
      <div className="space-y-3 py-6 text-center" data-testid="athlete-physiology-error">
        <p className="text-sm text-on-surface-variant">
          {describeApiError(error, { t, fallbackKey: COACH_PHYSIOLOGY_KEY.loadFailed })}
        </p>
        <Button variant="secondary" size="sm" onClick={refetch}>
          {t('common.retry')}
        </Button>
      </div>
    );
  }
  if (read?.status === 'consentRevoked') {
    return (
      <p className="py-6 text-center text-sm text-on-surface-variant" data-testid="athlete-physiology-revoked">
        {t(COACH_PHYSIOLOGY_KEY.consentRevoked, { name })}
      </p>
    );
  }
  if (read?.status !== 'ok') return null;
  const { sports, offered } = read.physiology;

  // The form takes the modal's place while it is open: one dialog at a time.
  if (target !== null) {
    return (
      <PhysiologyAnchorEditor
        inline
        target={target}
        offered={offered}
        onClose={() => setTarget(null)}
        onSave={async (anchorSave) => {
          await save.mutateAsync(anchorSave);
          setSaved(true);
        }}
      />
    );
  }

  return (
    <div className="space-y-6" data-testid="athlete-physiology">
      <p className="text-xs text-outline">{t(COACH_PHYSIOLOGY_KEY.blurb, { name })}</p>
      {sports.length === 0 ? (
        <p className="text-sm text-on-surface-variant" data-testid="athlete-physiology-empty">
          {t(COACH_PHYSIOLOGY_KEY.empty, { name })}
        </p>
      ) : (
        <PhysiologyAnchorList
          sports={sports}
          onEdit={(sport, anchor) => {
            setSaved(false);
            setTarget({ mode: 'edit', sport, anchor });
          }}
        />
      )}
      <div className="flex flex-wrap items-center gap-3">
        <Button
          variant="outline"
          onClick={() => {
            setSaved(false);
            setTarget({ mode: 'add' });
          }}
          data-testid="athlete-physiology-add"
        >
          {t(PHYSIOLOGY_EDIT_KEY.add)}
        </Button>
        {saved && (
          <p role="status" className="text-sm text-on-surface-variant" data-testid="athlete-physiology-saved">
            {t(COACH_PHYSIOLOGY_KEY.saved, { name })}
          </p>
        )}
      </div>
    </div>
  );
}

/**
 * The thresholds a coached athlete's workouts are set on, as their coach
 * reads them from Group info. An athlete who stopped sharing with the coach,
 * and a read that failed, each say so.
 */
export default function AthletePhysiologyModal({ member, groupId, onClose }: AthletePhysiologyModalProps) {
  const { t } = useTranslation();
  const name = member?.display_name ?? t('groups.memberFallback');
  return (
    <Modal isOpen={member !== null} onClose={onClose} title={t(COACH_PHYSIOLOGY_KEY.title, { name })} size="2xl">
      {member !== null && <AthletePhysiologyBody member={member} groupId={groupId} />}
    </Modal>
  );
}
