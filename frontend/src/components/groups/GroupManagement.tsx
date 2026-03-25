// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Main groups management page listing the user's coaching groups
// ABOUTME: Provides group creation via modal and navigation to group details

import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { Users, Plus, LogIn } from 'lucide-react';
import { useMyGroups, useCreateGroup } from '../../hooks/useGroups';
import { coachesApi } from '../../services/api';
import { Button, Card, Input, Select, Modal, ModalActions, useErrorToast, useSuccessToast } from '../ui';
import GroupCard from './GroupCard';
import JoinGroupModal from './JoinGroupModal';
import type { SelectOption } from '../ui';
import type { CreateGroupRequest } from '@pierre/shared-types';

interface GroupManagementProps {
  onSelectGroup: (groupId: string) => void;
}

const MAX_MEMBERS_OPTIONS: SelectOption[] = [
  { value: '5', label: '5 members' },
  { value: '10', label: '10 members' },
  { value: '15', label: '15 members' },
  { value: '20', label: '20 members' },
  { value: '25', label: '25 members' },
  { value: '50', label: '50 members' },
];

export default function GroupManagement({ onSelectGroup }: GroupManagementProps) {
  const { groups, isLoading } = useMyGroups();
  const { createGroup, isPending: isCreating } = useCreateGroup();
  const showError = useErrorToast();
  const showSuccess = useSuccessToast();

  const [isCreateOpen, setIsCreateOpen] = useState(false);
  const [isJoinOpen, setIsJoinOpen] = useState(false);
  const [formName, setFormName] = useState('');
  const [formDescription, setFormDescription] = useState('');
  const [formCoachId, setFormCoachId] = useState('');
  const [formMaxMembers, setFormMaxMembers] = useState('10');

  // Fetch available coaches for the create group form
  const { data: coachesList } = useQuery({
    queryKey: QUERY_KEYS.coaches.list(),
    queryFn: () => coachesApi.list(),
    staleTime: 60_000,
  });

  const coachOptions: SelectOption[] = (coachesList ?? []).map((coach) => ({
    value: coach.id,
    label: coach.name,
  }));

  const resetForm = () => {
    setFormName('');
    setFormDescription('');
    setFormCoachId('');
    setFormMaxMembers('10');
  };

  const handleOpenCreate = () => {
    resetForm();
    setIsCreateOpen(true);
  };

  const handleCreate = async () => {
    if (!formName.trim() || !formCoachId) return;

    const request: CreateGroupRequest = {
      name: formName.trim(),
      coach_id: formCoachId,
      max_members: parseInt(formMaxMembers, 10),
    };
    if (formDescription.trim()) {
      request.description = formDescription.trim();
    }

    try {
      const group = await createGroup(request);
      showSuccess('Group created', `${group.name} is ready for members.`);
      setIsCreateOpen(false);
      resetForm();
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to create group';
      showError('Creation failed', message);
    }
  };

  const isFormValid = formName.trim().length > 0 && formCoachId.length > 0;

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-xl font-semibold text-white">Coaching Groups</h2>
          <p className="text-sm text-zinc-400 mt-1">
            Train together with shared AI coaching
          </p>
        </div>
        <div className="flex gap-2">
          <Button variant="secondary" onClick={() => setIsJoinOpen(true)}>
            <span className="flex items-center gap-2">
              <LogIn className="w-4 h-4" />
              Join Group
            </span>
          </Button>
          <Button variant="primary" onClick={handleOpenCreate}>
            <span className="flex items-center gap-2">
              <Plus className="w-4 h-4" />
              Create Group
            </span>
          </Button>
        </div>
      </div>

      {/* Group list */}
      {isLoading ? (
        <div className="flex justify-center py-12">
          <div className="pierre-spinner" />
        </div>
      ) : groups.length === 0 ? (
        <Card variant="dark" className="!p-10">
          <div className="text-center">
            <div className="w-16 h-16 mx-auto mb-4 rounded-full bg-pierre-violet/20 flex items-center justify-center">
              <Users className="w-8 h-8 text-pierre-violet-light" />
            </div>
            <h3 className="text-lg font-semibold text-white mb-2">No groups yet</h3>
            <p className="text-zinc-400 mb-6 max-w-md mx-auto">
              Create a coaching group to train with others, or join an existing group with an invite code.
            </p>
            <div className="flex items-center justify-center gap-3">
              <Button variant="secondary" onClick={() => setIsJoinOpen(true)}>
                Join with Code
              </Button>
              <Button variant="primary" onClick={handleOpenCreate}>
                Create Group
              </Button>
            </div>
          </div>
        </Card>
      ) : (
        <div className="space-y-3">
          {groups.map((group) => (
            <GroupCard key={group.id} group={group} onClick={onSelectGroup} />
          ))}
        </div>
      )}

      {/* Create Group Modal */}
      <Modal
        isOpen={isCreateOpen}
        onClose={() => setIsCreateOpen(false)}
        title="Create Coaching Group"
        size="md"
        footer={
          <ModalActions>
            <Button variant="secondary" onClick={() => setIsCreateOpen(false)}>
              Cancel
            </Button>
            <Button
              variant="primary"
              onClick={handleCreate}
              loading={isCreating}
              disabled={!isFormValid}
            >
              Create Group
            </Button>
          </ModalActions>
        }
      >
        <div className="space-y-4">
          <Input
            label="Group Name"
            variant="dark"
            placeholder="e.g., Marathon Training Squad"
            value={formName}
            onChange={(e) => setFormName(e.target.value)}
            maxLength={100}
          />
          <div className="w-full">
            <label className="block text-sm font-medium text-zinc-300 mb-1.5">
              Description
            </label>
            <textarea
              className="w-full px-4 py-2.5 text-sm bg-[#151520] text-white placeholder-zinc-500 border border-white/10 rounded-lg transition-all focus:outline-none focus:ring-2 focus:ring-pierre-violet focus:ring-opacity-30 focus:border-pierre-violet resize-none"
              placeholder="What is this group about?"
              rows={3}
              value={formDescription}
              onChange={(e) => setFormDescription(e.target.value)}
              maxLength={500}
            />
          </div>
          <Select
            label="Coach"
            options={coachOptions}
            placeholder="Select a coach persona..."
            value={formCoachId}
            onChange={(e) => setFormCoachId(e.target.value)}
          />
          <Select
            label="Max Members"
            options={MAX_MEMBERS_OPTIONS}
            value={formMaxMembers}
            onChange={(e) => setFormMaxMembers(e.target.value)}
          />
        </div>
      </Modal>

      {/* Join Group Modal */}
      <JoinGroupModal
        isOpen={isJoinOpen}
        onClose={() => setIsJoinOpen(false)}
      />
    </div>
  );
}
