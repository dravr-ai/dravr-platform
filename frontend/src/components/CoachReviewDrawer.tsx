// ABOUTME: Side drawer component for reviewing coach submissions in detail
// ABOUTME: 480px width drawer with coach details, system prompt preview, and approve/reject actions
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { adminApi } from '../services/api';
import { Button, Card } from './ui';
import { clsx } from 'clsx';
import CoachRejectionModal from './CoachRejectionModal';
import { QUERY_KEYS } from '../constants/queryKeys';

// Category colors matching SystemCoachesTab
const CATEGORY_COLORS: Record<string, string> = {
  Training: 'bg-pierre-activity/10 text-pierre-activity border-pierre-activity/20',
  Nutrition: 'bg-pierre-nutrition/10 text-pierre-nutrition border-pierre-nutrition/20',
  Recovery: 'bg-pierre-recovery/10 text-pierre-recovery border-pierre-recovery/20',
  Recipes: 'bg-pierre-yellow-500/10 text-pierre-yellow-600 border-pierre-yellow-500/20',
  Mobility: 'bg-pierre-mobility/10 text-pierre-mobility border-pierre-mobility/20',
  Custom: 'bg-pierre-violet/10 text-pierre-violet-light border-pierre-violet/20',
};

function getCategoryColorClass(category: string): string {
  const normalized = category.charAt(0).toUpperCase() + category.slice(1).toLowerCase();
  return CATEGORY_COLORS[normalized] || CATEGORY_COLORS.Custom;
}

interface PendingCoach {
  id: string;
  title: string;
  description: string | null;
  category: string;
  tags: string[];
  sample_prompts: string[];
  token_count: number;
  install_count: number;
  icon_url: string | null;
  published_at: string | null;
  author_id: string | null;
  author_email?: string;
  system_prompt: string;
  created_at: string;
  submitted_at: string;
  publish_status: string;
}

interface CoachReviewDrawerProps {
  coach: PendingCoach | null;
  isOpen: boolean;
  onClose: () => void;
}

export default function CoachReviewDrawer({ coach, isOpen, onClose }: CoachReviewDrawerProps) {
  const [isPromptExpanded, setIsPromptExpanded] = useState(false);
  const [showRejectionModal, setShowRejectionModal] = useState(false);
  const queryClient = useQueryClient();

  const approveMutation = useMutation({
    mutationFn: (coachId: string) => adminApi.approveStoreCoach(coachId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.adminStore.reviewQueue() });
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.adminStore.stats() });
      onClose();
    },
  });

  const handleApprove = () => {
    if (coach) {
      approveMutation.mutate(coach.id);
    }
  };

  const handleRejectionComplete = () => {
    setShowRejectionModal(false);
    onClose();
  };

  if (!isOpen || !coach) return null;

  const formatDate = (dateString: string) => {
    return new Date(dateString).toLocaleDateString('en-US', {
      year: 'numeric',
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
    });
  };

  // Truncate prompt for preview (show first 500 chars)
  const promptPreview = coach.system_prompt.length > 500 && !isPromptExpanded
    ? coach.system_prompt.slice(0, 500) + '...'
    : coach.system_prompt;

  return (
    <>
      {/* Backdrop */}
      <div
        className="fixed inset-0 bg-black/60 backdrop-blur-sm z-40"
        onClick={onClose}
        aria-hidden="true"
      />

      {/* Drawer */}
      <div className="fixed inset-y-0 right-0 w-full max-w-[480px] bg-surface shadow-2xl z-50 border-l ghost-border flex flex-col">
        {/* Header - Sticky */}
        <div className="sticky top-0 bg-surface/95 backdrop-blur-lg border-b ghost-border px-6 py-4 flex justify-between items-center z-10">
          <h2 className="text-xl font-semibold text-on-surface">Review Coach</h2>
          <button
            onClick={onClose}
            aria-label="Close drawer"
            className="text-on-surface-variant hover:text-on-surface transition-colors"
          >
            <svg className="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* Content - Scrollable */}
        <div className="flex-1 overflow-y-auto p-6 space-y-6 scrollbar-dark">
          {/* Coach Header */}
          <div className="flex items-start gap-4">
            {coach.icon_url ? (
              <img
                src={coach.icon_url}
                alt={coach.title}
                className="w-16 h-16 rounded-xl object-cover border ghost-border"
              />
            ) : (
              <div className="w-16 h-16 rounded-xl boreal-hero-gradient flex items-center justify-center">
                <span className="text-2xl font-bold text-on-primary">
                  {coach.title.charAt(0).toUpperCase()}
                </span>
              </div>
            )}
            <div className="flex-1 min-w-0">
              <h3 className="text-lg font-semibold text-on-surface truncate">{coach.title}</h3>
              <span className={clsx(
                'inline-block mt-1 px-2 py-0.5 text-xs font-medium rounded-full border',
                getCategoryColorClass(coach.category)
              )}>
                {coach.category}
              </span>
            </div>
          </div>

          {/* Description */}
          {coach.description && (
            <Card variant="dark" className="p-4 bg-surface-container-low">
              <h4 className="text-sm font-medium text-on-surface mb-2">Description</h4>
              <p className="text-sm text-on-surface-variant">{coach.description}</p>
            </Card>
          )}

          {/* Author Info */}
          <Card variant="dark" className="p-4 bg-surface-container-low">
            <h4 className="text-sm font-medium text-on-surface mb-3 flex items-center">
              <svg className="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M16 7a4 4 0 11-8 0 4 4 0 018 0zM12 14a7 7 0 00-7 7h14a7 7 0 00-7-7z" />
              </svg>
              Author
            </h4>
            <div className="space-y-2 text-sm">
              <div className="flex justify-between">
                <span className="text-outline">Email</span>
                <span className="text-on-surface font-medium">{coach.author_email || 'Unknown'}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-outline">Author ID</span>
                <span className="text-on-surface font-mono text-xs">{coach.author_id || '—'}</span>
              </div>
            </div>
          </Card>

          {/* System Prompt */}
          <Card variant="dark" className="p-4 bg-surface-container-low">
            <div className="flex items-center justify-between mb-3">
              <h4 className="text-sm font-medium text-on-surface flex items-center">
                <svg className="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4" />
                </svg>
                System Prompt
              </h4>
              <span className="text-xs text-outline">
                {coach.token_count.toLocaleString()} tokens
              </span>
            </div>
            <div className="p-3 bg-surface-container-low rounded-lg border ghost-border">
              <pre className="text-sm text-on-surface whitespace-pre-wrap font-mono">
                {promptPreview}
              </pre>
              {coach.system_prompt.length > 500 && (
                <button
                  onClick={() => setIsPromptExpanded(!isPromptExpanded)}
                  className="mt-2 text-sm text-pierre-violet-light hover:text-pierre-violet transition-colors"
                >
                  {isPromptExpanded ? 'Show less' : 'Show full prompt'}
                </button>
              )}
            </div>
          </Card>

          {/* Tags */}
          {coach.tags.length > 0 && (
            <Card variant="dark" className="p-4 bg-surface-container-low">
              <h4 className="text-sm font-medium text-on-surface mb-3">Tags</h4>
              <div className="flex flex-wrap gap-2">
                {coach.tags.map((tag) => (
                  <span key={tag} className="px-3 py-1 text-sm bg-surface-container-high text-on-surface rounded-full">
                    {tag}
                  </span>
                ))}
              </div>
            </Card>
          )}

          {/* Sample Prompts */}
          {coach.sample_prompts.length > 0 && (
            <Card variant="dark" className="p-4 bg-surface-container-low">
              <h4 className="text-sm font-medium text-on-surface mb-3">Sample Prompts</h4>
              <ul className="space-y-2">
                {coach.sample_prompts.map((prompt, idx) => (
                  <li key={idx} className="text-sm text-on-surface-variant flex items-start gap-2">
                    <span className="text-pierre-violet-light">•</span>
                    <span>{prompt}</span>
                  </li>
                ))}
              </ul>
            </Card>
          )}

          {/* Metadata */}
          <Card variant="dark" className="p-4 bg-surface-container-low">
            <h4 className="text-sm font-medium text-on-surface mb-3">Submission Details</h4>
            <div className="space-y-2 text-sm">
              <div className="flex justify-between">
                <span className="text-outline">Created</span>
                <span className="text-on-surface">{formatDate(coach.created_at)}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-outline">Submitted for Review</span>
                <span className="text-on-surface">{formatDate(coach.submitted_at)}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-outline">Status</span>
                <span className="px-2 py-0.5 text-xs bg-pierre-violet/20 text-pierre-violet-light rounded-full">
                  Pending Review
                </span>
              </div>
            </div>
          </Card>
        </div>

        {/* Footer - Sticky */}
        <div className="sticky bottom-0 bg-surface/95 backdrop-blur-lg border-t ghost-border px-6 py-4 flex gap-3">
          <Button
            onClick={() => setShowRejectionModal(true)}
            variant="secondary"
            className="flex-1 border-pierre-red-500/30 text-pierre-red-400 hover:bg-pierre-red-500/10"
          >
            <svg className="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
            Reject
          </Button>
          <Button
            onClick={handleApprove}
            disabled={approveMutation.isPending}
            className="flex-1 bg-pierre-activity hover:bg-pierre-activity/80 text-on-primary"
          >
            {approveMutation.isPending ? (
              <span className="flex items-center">
                <div className="pierre-spinner w-4 h-4 mr-2 border-white border-t-transparent" />
                Approving...
              </span>
            ) : (
              <>
                <svg className="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                </svg>
                Approve
              </>
            )}
          </Button>
        </div>

        {/* Error Message */}
        {approveMutation.isError && (
          <div className="px-6 pb-4">
            <div className="p-3 bg-pierre-red-500/15 border border-pierre-red-500/30 rounded-md">
              <p className="text-sm text-pierre-red-400">
                Failed to approve coach. Please try again.
              </p>
            </div>
          </div>
        )}
      </div>

      {/* Rejection Modal */}
      <CoachRejectionModal
        coach={coach}
        isOpen={showRejectionModal}
        onClose={() => setShowRejectionModal(false)}
        onComplete={handleRejectionComplete}
      />
    </>
  );
}
