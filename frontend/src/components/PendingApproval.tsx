// ABOUTME: Page shown to users whose accounts are pending admin approval
// ABOUTME: Displays status message and allows logout while waiting for approval
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useAuth } from '../hooks/useAuth';
import { Button, Card, Badge } from './ui';

// Clock icon for pending status
function ClockIcon({ className }: { className?: string }) {
  return (
    <svg className={className} fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={1.5}
        d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z"
      />
    </svg>
  );
}

// Pierre holistic node logo SVG
import { DravrLogo } from './DravrLogo';

export default function PendingApproval() {
  const { user, logout } = useAuth();

  return (
    <div className="min-h-screen flex items-center justify-center py-12 px-4 sm:px-6 lg:px-8 bg-surface-container-low">
      <div className="max-w-md w-full">
        <Card className="overflow-hidden">
          {/* Gradient accent bar at top */}
          <div className="h-1 w-full boreal-hero-gradient" />

          <div className="px-8 py-10">
            {/* Logo and icon */}
            <div className="flex flex-col items-center text-center">
              <DravrLogo size={64} />

              <div className="mt-6 mb-4">
                <ClockIcon className="w-16 h-16 text-pierre-nutrition mx-auto" />
              </div>

              <h1 className="text-xl font-bold text-on-surface">
                Account Pending Approval
              </h1>

              <p className="mt-3 text-sm text-on-surface-variant max-w-sm">
                Your account has been created successfully and is awaiting approval
                by an administrator. You&apos;ll receive an email notification once
                your account is approved.
              </p>
            </div>

            {/* Status card */}
            <div className="mt-8 bg-surface-container-low rounded-lg p-4 space-y-3">
              <div className="flex items-center justify-between">
                <span className="text-sm font-medium text-on-surface-variant">Status</span>
                <Badge variant="warning">Pending</Badge>
              </div>

              {user?.email && (
                <div className="flex items-center justify-between">
                  <span className="text-sm font-medium text-on-surface-variant">Email</span>
                  <span className="text-sm text-on-surface-variant">{user.email}</span>
                </div>
              )}

              {user?.display_name && (
                <div className="flex items-center justify-between">
                  <span className="text-sm font-medium text-on-surface-variant">Name</span>
                  <span className="text-sm text-on-surface-variant">{user.display_name}</span>
                </div>
              )}
            </div>

            {/* What happens next */}
            <div className="mt-6">
              <h2 className="text-sm font-semibold text-on-surface mb-3">
                What happens next?
              </h2>
              <ul className="text-sm text-on-surface-variant space-y-2">
                <li className="flex items-start gap-2">
                  <span className="text-pierre-activity mt-0.5">•</span>
                  <span>An administrator will review your registration</span>
                </li>
                <li className="flex items-start gap-2">
                  <span className="text-pierre-activity mt-0.5">•</span>
                  <span>You&apos;ll receive an email when approved</span>
                </li>
                <li className="flex items-start gap-2">
                  <span className="text-pierre-activity mt-0.5">•</span>
                  <span>Once approved, you can access Dravr&apos;s fitness intelligence</span>
                </li>
              </ul>
            </div>

            {/* Sign out button */}
            <div className="mt-8">
              <Button
                variant="secondary"
                onClick={logout}
                className="w-full"
              >
                Sign Out
              </Button>
            </div>
          </div>
        </Card>
      </div>
    </div>
  );
}
