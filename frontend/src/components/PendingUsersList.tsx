// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { adminApi } from '../services/api';
import type { User } from '../types/api';
import { Button } from './ui';
import UserApprovalModal from './UserApprovalModal';
import { QUERY_KEYS } from '../constants/queryKeys';

export default function PendingUsersList() {
  const [selectedUser, setSelectedUser] = useState<User | null>(null);
  const [modalAction, setModalAction] = useState<'approve' | 'suspend'>('approve');
  const [isModalOpen, setIsModalOpen] = useState(false);

  const { 
    data: pendingUsers = [], 
    isLoading, 
    error,
    refetch 
  } = useQuery<User[]>({
    queryKey: QUERY_KEYS.adminUsers.pending(),
    queryFn: () => adminApi.getPendingUsers(),
    refetchInterval: 30000, // Refetch every 30 seconds for real-time updates
  });

  const handleApprove = (user: User) => {
    setSelectedUser(user);
    setModalAction('approve');
    setIsModalOpen(true);
  };

  const handleSuspend = (user: User) => {
    setSelectedUser(user);
    setModalAction('suspend');
    setIsModalOpen(true);
  };

  const handleCloseModal = () => {
    setIsModalOpen(false);
    setSelectedUser(null);
  };

  const formatDate = (dateString: string) => {
    const date = new Date(dateString);
    const now = new Date();
    const diffInHours = (now.getTime() - date.getTime()) / (1000 * 60 * 60);

    if (diffInHours < 24) {
      return `${Math.floor(diffInHours)}h ago`;
    } else if (diffInHours < 48) {
      return 'Yesterday';
    } else {
      return date.toLocaleDateString();
    }
  };

  if (isLoading) {
    return (
      <div>
        {[...Array(3)].map((_, i) => (
          <div key={i} className="flex animate-pulse items-center justify-between border-t ghost-border-faint py-3 first:border-t-0">
            <div className="space-y-2">
              <div className="h-3 w-48 rounded bg-surface-container-high"></div>
              <div className="h-3 w-32 rounded bg-surface-container-high"></div>
            </div>
            <div className="h-7 w-20 rounded bg-surface-container-high"></div>
          </div>
        ))}
      </div>
    );
  }

  if (error) {
    return (
      <div className="py-3">
        <p className="text-sm font-medium text-error">Failed to load pending users</p>
        <Button onClick={() => refetch()} variant="tertiary" size="sm" className="mt-1 px-0">
          Retry
        </Button>
      </div>
    );
  }

  if (pendingUsers.length === 0) {
    return (
      <div className="py-3">
        <p className="text-sm text-on-surface-variant">No pending users</p>
        <p className="mt-0.5 text-xs text-outline">All users have been processed</p>
      </div>
    );
  }

  return (
    <>
      <div>
        <div className="flex items-center justify-between pb-2">
          <h3 className="font-sans text-sm font-semibold tracking-normal text-on-surface">
            Pending Users ({pendingUsers.length})
          </h3>
          <Button
            onClick={() => refetch()}
            variant="outline"
            size="sm"
            className="flex items-center space-x-2"
          >
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
            </svg>
            <span>Refresh</span>
          </Button>
        </div>

        {pendingUsers.map((user) => (
          <div key={user.id} className="flex items-start justify-between border-t ghost-border-faint py-3 transition-colors hover:bg-surface-container-low/60">
            <div className="flex justify-between items-start w-full">
              <div className="flex-1">
                <div className="flex items-center space-x-2 mb-0.5">
                  <h4 className="font-sans text-sm font-medium tracking-normal text-on-surface">
                    {user.display_name || 'Unnamed User'}
                  </h4>
                  <span className="inline-flex items-center gap-1.5 text-xs text-on-surface-variant">
                    <span aria-hidden="true" className="h-1.5 w-1.5 rounded-full bg-warning" />
                    {user.user_status || 'pending'}
                  </span>
                </div>
                <p className="text-sm text-on-surface-variant mb-1">{user.email}</p>
                <div className="flex items-center space-x-4 text-xs text-outline">
                  <span>Registered: {formatDate(user.created_at)}</span>
                  <span className="capitalize">Tier: {user.tier}</span>
                </div>
              </div>

              <div className="flex flex-col space-y-2 ml-4">
                <Button
                  onClick={() => handleApprove(user)}
                  size="sm"
                  className="bg-activity hover:bg-activity/80 text-on-primary"
                >
                  <svg className="w-4 h-4 mr-1" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                  </svg>
                  Approve
                </Button>
                <Button
                  onClick={() => handleSuspend(user)}
                  size="sm"
                  variant="outline"
                  className="border-error/30 text-error hover:bg-error/10"
                >
                  <svg className="w-4 h-4 mr-1" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M18.364 18.364A9 9 0 005.636 5.636m12.728 12.728L5.636 5.636m12.728 12.728L18.364 5.636M5.636 18.364l12.728-12.728" />
                  </svg>
                  Suspend
                </Button>
              </div>
            </div>
          </div>
        ))}
      </div>

      <UserApprovalModal
        user={selectedUser}
        isOpen={isModalOpen}
        onClose={handleCloseModal}
        action={modalAction}
      />
    </>
  );
}