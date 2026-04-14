// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//
// ABOUTME: Coach Store detail screen showing full coach info with install/uninstall actions
// ABOUTME: Displays system prompt preview, sample prompts, tags, and install count

import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { clsx } from 'clsx';
import { storeApi } from '../services/api';
import { ConfirmDialog } from './ui';
import { QUERY_KEYS } from '../constants/queryKeys';

// Coach category colors (dark theme)
const COACH_CATEGORY_COLORS: Record<string, string> = {
  training: 'bg-emerald-500/20 text-emerald-400',
  nutrition: 'bg-amber-500/20 text-amber-400',
  recovery: 'bg-indigo-500/20 text-indigo-400',
  recipes: 'bg-orange-500/20 text-orange-400',
  mobility: 'bg-pink-500/20 text-pink-400',
  custom: 'bg-violet-500/20 text-violet-400',
};

interface StoreCoachDetailProps {
  coachId: string;
  onBack: () => void;
  onNavigateToLibrary?: () => void;
}

export default function StoreCoachDetail({ coachId, onBack, onNavigateToLibrary }: StoreCoachDetailProps) {
  const queryClient = useQueryClient();
  const [showUninstallConfirm, setShowUninstallConfirm] = useState(false);
  const [successMessage, setSuccessMessage] = useState<string | null>(null);

  // Fetch coach details
  const { data: coach, isLoading, error } = useQuery({
    queryKey: QUERY_KEYS.store.coach(coachId),
    queryFn: () => storeApi.get(coachId),
    staleTime: 60_000,
  });

  // Check if coach is installed
  const { data: installations } = useQuery({
    queryKey: QUERY_KEYS.store.installations(),
    queryFn: () => storeApi.getInstallations(),
    staleTime: 30_000,
  });

  const isInstalled = installations?.coaches.some((c) => c.id === coachId) ?? false;

  // Install mutation
  const installMutation = useMutation({
    mutationFn: () => storeApi.install(coachId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.store.installations() });
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.store.coach(coachId) });
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.all });
      setSuccessMessage(`"${coach?.title}" has been added to your coaches.`);
    },
  });

  // Uninstall mutation
  const uninstallMutation = useMutation({
    mutationFn: () => storeApi.uninstall(coachId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.store.installations() });
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.store.coach(coachId) });
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.all });
      setShowUninstallConfirm(false);
      setSuccessMessage('Coach has been removed from My Coaches.');
    },
  });

  const handleInstall = () => {
    installMutation.mutate();
  };

  const handleUninstall = () => {
    uninstallMutation.mutate();
  };

  const categoryColors = coach ? COACH_CATEGORY_COLORS[coach.category] ?? 'bg-surface-container-high text-on-surface-variant' : '';

  if (isLoading) {
    return (
      <div className="h-full flex items-center justify-center bg-surface">
        <div className="text-center">
          <div className="w-8 h-8 border-2 border-pierre-violet border-t-transparent rounded-full animate-spin mx-auto" />
          <p className="mt-3 text-sm text-on-surface-variant">Loading coach details...</p>
        </div>
      </div>
    );
  }

  if (error || !coach) {
    return (
      <div className="h-full flex flex-col items-center justify-center bg-surface p-6">
        <svg
          className="w-12 h-12 text-on-surface-variant mb-4"
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
        </svg>
        <h3 className="text-lg font-medium text-on-surface">Coach not found</h3>
        <p className="text-sm text-on-surface-variant mt-1">This coach may have been removed or is no longer available.</p>
        <button
          onClick={onBack}
          className="mt-4 px-4 py-2 bg-pierre-violet text-on-surface rounded-lg hover:bg-pierre-violet/90 transition-colors shadow-ambient"
        >
          Go Back
        </button>
      </div>
    );
  }

  return (
    <div className="h-full flex flex-col bg-surface">
      {/* Header */}
      <div className="flex items-center gap-4 px-6 py-4 border-b ghost-border">
        <button
          onClick={onBack}
          className="p-2 rounded-lg hover:bg-surface-container transition-colors min-w-[44px] min-h-[44px] flex items-center justify-center"
          title="Back to Store"
          aria-label="Back to Store"
        >
          <svg className="w-5 h-5 text-on-surface-variant" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 19l-7-7 7-7" />
          </svg>
        </button>
        <h1 className="text-xl font-semibold text-on-surface flex-1 truncate">{coach.title}</h1>
      </div>

      {/* Success Message */}
      {successMessage && (
        <div className="mx-6 mt-4 p-4 bg-emerald-500/10 border border-emerald-500/30 rounded-lg flex items-start gap-3">
          <svg className="w-5 h-5 text-emerald-400 flex-shrink-0 mt-0.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z" />
          </svg>
          <div className="flex-1">
            <p className="text-sm text-emerald-300">{successMessage}</p>
            {successMessage.includes('added') && onNavigateToLibrary && (
              <button
                onClick={onNavigateToLibrary}
                className="text-sm text-emerald-400 hover:text-emerald-300 font-medium mt-1"
              >
                View My Coaches →
              </button>
            )}
          </div>
          <button
            onClick={() => setSuccessMessage(null)}
            className="text-emerald-400 hover:text-emerald-300"
          >
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>
      )}

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-6">
        {/* Category & Stats */}
        <div className="flex items-center justify-between mb-4">
          <span className={clsx('px-3 py-1 text-sm font-medium rounded-full capitalize', categoryColors)}>
            {coach.category}
          </span>
          <span className="text-sm text-on-surface-variant">
            {coach.install_count} {coach.install_count === 1 ? 'user' : 'users'}
          </span>
        </div>

        {/* Title */}
        <h2 className="text-2xl font-bold text-on-surface mb-3">{coach.title}</h2>

        {/* Description */}
        {coach.description && (
          <p className="text-on-surface mb-6 leading-relaxed">{coach.description}</p>
        )}

        {/* Tags */}
        {coach.tags.length > 0 && (
          <div className="mb-6">
            <h3 className="text-sm font-semibold text-outline uppercase tracking-wide mb-2">Tags</h3>
            <div className="flex flex-wrap gap-2">
              {coach.tags.map((tag, index) => (
                <span
                  key={index}
                  className="px-3 py-1 text-sm bg-surface-container-high text-on-surface rounded-full"
                >
                  {tag}
                </span>
              ))}
            </div>
          </div>
        )}

        {/* Sample Prompts */}
        {coach.sample_prompts.length > 0 && (
          <div className="mb-6">
            <h3 className="text-sm font-semibold text-outline uppercase tracking-wide mb-2">Sample Prompts</h3>
            <div className="space-y-2">
              {coach.sample_prompts.map((prompt, index) => (
                <div
                  key={index}
                  className="p-3 bg-surface-container-low border ghost-border rounded-lg text-sm text-on-surface"
                >
                  {prompt}
                </div>
              ))}
            </div>
          </div>
        )}

        {/* System Prompt Preview */}
        <div className="mb-6">
          <h3 className="text-sm font-semibold text-outline uppercase tracking-wide mb-2">System Prompt</h3>
          <div className="p-4 bg-surface-container-low border ghost-border rounded-lg">
            <pre className="text-sm text-on-surface-variant whitespace-pre-wrap font-mono leading-relaxed max-h-48 overflow-y-auto">
              {coach.system_prompt.length > 500
                ? `${coach.system_prompt.slice(0, 500)}...`
                : coach.system_prompt}
            </pre>
            {coach.system_prompt.length > 500 && (
              <p className="text-xs text-outline mt-2 italic">
                ...and more ({coach.token_count} tokens)
              </p>
            )}
          </div>
        </div>

        {/* Details */}
        <div className="mb-6">
          <h3 className="text-sm font-semibold text-outline uppercase tracking-wide mb-2">Details</h3>
          <div className="bg-surface-container-low border ghost-border rounded-lg overflow-hidden">
            <div className="flex justify-between items-center px-4 py-3 border-b ghost-border">
              <span className="text-sm text-on-surface-variant">Token Count</span>
              <span className="text-sm font-medium text-on-surface">{coach.token_count.toLocaleString()}</span>
            </div>
            {coach.published_at && (
              <div className="flex justify-between items-center px-4 py-3">
                <span className="text-sm text-on-surface-variant">Published</span>
                <span className="text-sm font-medium text-on-surface">
                  {new Date(coach.published_at).toLocaleDateString()}
                </span>
              </div>
            )}
          </div>
        </div>

        {/* Install/Uninstall Button */}
        <div className="mb-6">
          {isInstalled ? (
            <button
              onClick={() => setShowUninstallConfirm(true)}
              disabled={uninstallMutation.isPending}
              className="px-6 py-2.5 bg-surface-container-high text-on-surface rounded-lg font-medium hover:bg-surface-container-highest transition-colors disabled:opacity-50 flex items-center justify-center gap-2"
            >
              {uninstallMutation.isPending ? (
                <div className="w-5 h-5 border-2 border-zinc-400 border-t-transparent rounded-full animate-spin" />
              ) : (
                'Remove'
              )}
            </button>
          ) : (
            <button
              onClick={handleInstall}
              disabled={installMutation.isPending}
              className="px-6 py-2.5 bg-pierre-violet text-on-surface rounded-lg font-medium hover:bg-pierre-violet/90 transition-colors disabled:opacity-50 flex items-center justify-center gap-2 shadow-ambient"
            >
              {installMutation.isPending ? (
                <div className="w-5 h-5 border-2 border-white border-t-transparent rounded-full animate-spin" />
              ) : (
                'Add Coach'
              )}
            </button>
          )}
        </div>
      </div>

      {/* Uninstall Confirmation Dialog */}
      <ConfirmDialog
        isOpen={showUninstallConfirm}
        onClose={() => setShowUninstallConfirm(false)}
        onConfirm={handleUninstall}
        title="Remove Coach?"
        message={`Remove "${coach.title}" from your coaches? You can always add it again later.`}
        confirmLabel="Remove"
        variant="danger"
        isLoading={uninstallMutation.isPending}
      />
    </div>
  );
}
