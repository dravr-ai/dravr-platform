// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A group's settings inside Group info — name, description, peer sharing, agent replies, weekly recap — each saved as it changes
// ABOUTME: No Save button: a field saves when it is left, a switch or a choice the moment it changes, and a refusal puts it back

import { useState, type ReactNode } from 'react';
import type { CoachingGroup, GroupDigestMode, GroupRespondMode, UpdateGroupRequest } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { describeApiError } from '@pierre/ui-logic';
import { useUpdateGroup } from '../../hooks/useGroups';
import { Checkbox, Input, Select, Textarea, useErrorToast, useSuccessToast } from '../ui';
import { DIGEST_MODES, RESPOND_MODE_LABEL_KEY } from './groupSettingsLabels';

interface GroupSettingsFormProps {
  group: CoachingGroup;
  /** An owner or admin sets every field; anyone else only the digest, when they may. */
  isAdmin: boolean;
  /**
   * The digest select: where the tenant's tier sends a digest at all, for an
   * admin or the group's attached coach — the one field the server takes
   * from the coach.
   */
  canSetDigest: boolean;
  /** What follows the fields: the invite links, where this viewer manages them here. */
  children?: ReactNode;
}

/**
 * Each field saves on its own, through the group's partial update, so there
 * is no Save button to compete with the drawer's one filled action, and
 * closing the drawer never drops an edit: nothing is lost and nothing asks.
 */
export default function GroupSettingsForm({ group, isAdmin, canSetDigest, children }: GroupSettingsFormProps) {
  const { t } = useTranslation();
  const { updateGroup } = useUpdateGroup(group.id);
  const showError = useErrorToast();
  const showSuccess = useSuccessToast();

  const [name, setName] = useState(group.name);
  const [description, setDescription] = useState(group.description ?? '');
  const [peerSharing, setPeerSharing] = useState(group.peer_data_sharing);
  const [respondMode, setRespondMode] = useState<GroupRespondMode>(group.respond_mode ?? 'all');
  const [digestMode, setDigestMode] = useState<GroupDigestMode>(group.digest_mode ?? 'off');

  /** Save one field; a refusal restores what the server still holds. */
  const save = async (patch: UpdateGroupRequest, restore: () => void) => {
    try {
      await updateGroup(patch);
      showSuccess(t('app.settingsSaved'), t('app.groupSettingsUpdated'));
    } catch (err) {
      restore();
      showError(t('app.saveFailed'), describeApiError(err, { t, fallbackKey: 'groups.saveFailed' }));
    }
  };

  const saveName = () => {
    const trimmed = name.trim();
    // A group always has a name: an emptied field goes back to the one it had.
    if (!trimmed) {
      setName(group.name);
      return;
    }
    if (trimmed !== group.name) void save({ name: trimmed }, () => setName(group.name));
  };

  const saveDescription = () => {
    const trimmed = description.trim();
    if (trimmed !== (group.description ?? '')) {
      void save({ description: trimmed }, () => setDescription(group.description ?? ''));
    }
  };

  return (
    <div className="space-y-4" data-testid="group-info-settings">
      <p className="text-xs text-on-surface-variant">{t('groups.info.autosaveHint')}</p>
      {isAdmin && (
        <>
          <Input
            label={t('groups.info.nameLabel')}
            value={name}
            onChange={(e) => setName(e.target.value)}
            onBlur={saveName}
            maxLength={100}
            data-testid="group-settings-name"
          />
          <Textarea
            label={t('chat.descriptionLabel')}
            rows={3}
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            onBlur={saveDescription}
            maxLength={500}
            data-testid="group-settings-description"
          />
          <Checkbox
            label={t('groups.peerSharingEnable')}
            description={t('groups.peerSharingDescription')}
            checked={peerSharing}
            onChange={(e) => {
              const next = e.target.checked;
              setPeerSharing(next);
              void save({ peer_data_sharing: next }, () => setPeerSharing(!next));
            }}
            data-testid="group-settings-peer-sharing"
          />
          <Select
            id="group-respond-mode"
            label={t('groups.respondMode')}
            value={respondMode}
            onChange={(e) => {
              const previous = respondMode;
              const next = e.target.value as GroupRespondMode;
              setRespondMode(next);
              void save({ respond_mode: next }, () => setRespondMode(previous));
            }}
            options={(Object.keys(RESPOND_MODE_LABEL_KEY) as GroupRespondMode[]).map((mode) => ({
              value: mode,
              label: t(RESPOND_MODE_LABEL_KEY[mode]),
            }))}
            helpText={t('groups.respondMentionedHint')}
          />
        </>
      )}
      {canSetDigest && (
        <Select
          id="group-digest-mode"
          label={t('groups.digestMode')}
          value={digestMode}
          onChange={(e) => {
            const previous = digestMode;
            const next = e.target.value as GroupDigestMode;
            setDigestMode(next);
            void save({ digest_mode: next }, () => setDigestMode(previous));
          }}
          options={(Object.keys(DIGEST_MODES) as GroupDigestMode[]).map((mode) => ({
            value: mode,
            label: t(DIGEST_MODES[mode].labelKey),
          }))}
          helpText={t(DIGEST_MODES[digestMode].hintKey)}
          data-testid="group-digest-mode"
        />
      )}
      {children}
    </div>
  );
}
