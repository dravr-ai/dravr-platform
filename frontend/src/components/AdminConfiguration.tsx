// ABOUTME: Admin tool management UI wrapping the ToolAvailability component
// ABOUTME: Allows admins to view and manage per-tenant MCP tool availability
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { lazy, Suspense } from 'react';

const ToolAvailability = lazy(() => import('./ToolAvailability'));

export default function AdminConfiguration() {
  return (
    <div className="space-y-6">
      <Suspense
        fallback={
          <div className="flex items-center justify-center py-12">
            <div className="pierre-spinner w-8 h-8"></div>
          </div>
        }
      >
        <ToolAvailability />
      </Suspense>
    </div>
  );
}
