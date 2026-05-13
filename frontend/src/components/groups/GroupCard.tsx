// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Card component for displaying a coaching group in a list view
// ABOUTME: Shows group name, member count, coach, active status, and the user's role badge

import { clsx } from 'clsx';
import { Users, Shield, Crown, User } from 'lucide-react';
import { Card } from '../ui';
import type { GroupSummary, GroupRole } from '@pierre/shared-types';

interface GroupCardProps {
  group: GroupSummary;
  onClick: (groupId: string) => void;
}

const ROLE_CONFIG: Record<GroupRole, { label: string; color: string; Icon: typeof Crown }> = {
  owner: {
    label: 'Owner',
    color: 'bg-amber-500/20 text-amber-400 border-amber-500/30',
    Icon: Crown,
  },
  admin: {
    label: 'Admin',
    color: 'bg-pierre-violet/20 text-pierre-violet-light border-pierre-violet/30',
    Icon: Shield,
  },
  member: {
    label: 'Member',
    color: 'bg-zinc-500/20 text-on-surface-variant border-zinc-500/30',
    Icon: User,
  },
};

export default function GroupCard({ group, onClick }: GroupCardProps) {
  const roleConfig = ROLE_CONFIG[group.my_role];
  const RoleIcon = roleConfig.Icon;

  return (
    <Card
      variant="dark"
      className={clsx(
        '!p-5 cursor-pointer transition-all hover:bg-surface-container',
        !group.is_active && 'opacity-60'
      )}
      onClick={() => onClick(group.id)}
    >
      <div className="flex items-start justify-between gap-4">
        {/* Left section: group info */}
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-3 mb-2">
            <h3 className="text-lg font-semibold text-on-surface truncate">{group.name}</h3>
            {!group.is_active && (
              <span className="px-2 py-0.5 text-xs font-medium bg-zinc-700/50 text-on-surface-variant rounded-full">
                Inactive
              </span>
            )}
          </div>

          {group.description && (
            <p
              className="text-sm text-on-surface-variant mb-3 line-clamp-2 break-words"
              title={group.description}
            >
              {group.description}
            </p>
          )}

          <div className="flex items-center gap-4 text-sm text-outline">
            <span className="flex items-center gap-1.5">
              <Users className="w-4 h-4" />
              {group.member_count} {group.member_count === 1 ? 'member' : 'members'}
            </span>
            {group.peer_data_sharing && (
              <span className="flex items-center gap-1.5 text-pierre-activity">
                <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8.684 13.342C8.886 12.938 9 12.482 9 12c0-.482-.114-.938-.316-1.342m0 2.684a3 3 0 110-2.684m0 2.684l6.632 3.316m-6.632-6l6.632-3.316m0 0a3 3 0 105.367-2.684 3 3 0 00-5.367 2.684zm0 9.316a3 3 0 105.368 2.684 3 3 0 00-5.368-2.684z" />
                </svg>
                Peer Sharing
              </span>
            )}
          </div>
        </div>

        {/* Right section: role badge */}
        <span
          className={clsx(
            'inline-flex items-center gap-1.5 px-3 py-1.5 rounded-full text-xs font-medium border flex-shrink-0',
            roleConfig.color
          )}
        >
          <RoleIcon className="w-3.5 h-3.5" />
          {roleConfig.label}
        </span>
      </div>
    </Card>
  );
}
