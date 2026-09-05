// ABOUTME: Banner component displayed when a super admin is impersonating another user
// ABOUTME: Shows target user info and provides button to end impersonation session
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useAuth } from '../hooks/useAuth';
import { Button } from './ui';
import { useTranslation } from '@pierre/i18n';

export default function ImpersonationBanner() {
  const { t } = useTranslation();
  const { impersonation, endImpersonation } = useAuth();

  if (!impersonation.isImpersonating || !impersonation.targetUser) {
    return null;
  }

  const handleEndImpersonation = async () => {
    try {
      await endImpersonation();
    } catch (error) {
      console.error('Failed to end impersonation:', error);
    }
  };

  return (
    <div className="bg-warning text-on-primary px-4 py-2 sticky top-0 z-50">
      <div className="max-w-7xl mx-auto flex items-center justify-between">
        <div className="flex items-center gap-3">
          <svg
            className="w-5 h-5"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
          >
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"
            />
          </svg>
          <span className="font-medium">
            {t('frag.impersonating')}{' '}
            <span className="font-bold">
              {impersonation.targetUser.display_name || impersonation.targetUser.email}
            </span>
            {impersonation.targetUser.display_name && (
              <span className="text-warning ml-1">
                ({impersonation.targetUser.email})
              </span>
            )}
          </span>
          <span className="text-warning text-sm">
            {t('frag.roleLabel')} {impersonation.targetUser.role}
          </span>
        </div>
        <Button
          variant="secondary"
          size="sm"
          onClick={handleEndImpersonation}
          className="bg-white text-warning hover:bg-warning border-0"
        >
          {t('shell.impersonationEnd')}
        </Button>
      </div>
    </div>
  );
}
