// ABOUTME: Component displaying rejected coach submissions with rejection details
// ABOUTME: Lists rejected coaches with reason, date, and option to re-review
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useQuery } from '@tanstack/react-query';
import { adminApi } from '../services/api';
import { clsx } from 'clsx';
import { QUERY_KEYS } from '../constants/queryKeys';

// Category colors matching SystemCoachesTab
const CATEGORY_COLORS: Record<string, string> = {
  Training: 'bg-activity/10 text-on-activity-container border-activity/20',
  Nutrition: 'bg-nutrition/10 text-on-nutrition-container border-nutrition/20',
  Recovery: 'bg-recovery/10 text-on-recovery-container border-recovery/20',
  Recipes: 'bg-warning/10 text-on-warning-container border-warning/20',
  Mobility: 'bg-mobility/10 text-on-mobility-container border-mobility/20',
  Custom: 'bg-primary/10 text-primary border-primary/20',
};

function getCategoryColorClass(category: string): string {
  const normalized = category.charAt(0).toUpperCase() + category.slice(1).toLowerCase();
  return CATEGORY_COLORS[normalized] || CATEGORY_COLORS.Custom;
}

// Map rejection reason codes to human-readable labels
const REJECTION_REASON_LABELS: Record<string, string> = {
  inappropriate_content: 'Inappropriate content',
  quality_standards: 'Quality standards not met',
  duplicate_submission: 'Duplicate submission',
  incomplete_information: 'Incomplete information',
  other: 'Other',
};

function getReasonLabel(reason: string): string {
  return REJECTION_REASON_LABELS[reason] || reason;
}

export default function RejectedCoachesList() {
  // Fetch rejected coaches
  const { data, isLoading, error } = useQuery({
    queryKey: QUERY_KEYS.adminStore.rejected(),
    queryFn: () => adminApi.getRejectedStoreCoaches(),
  });

  const formatDate = (dateString: string) => {
    return new Date(dateString).toLocaleDateString('en-US', {
      month: 'short',
      day: 'numeric',
      year: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
    });
  };

  if (isLoading) {
    return (
      <div className="flex justify-center py-12">
        <div className="pierre-spinner w-8 h-8"></div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="py-3">
        <h3 className="font-sans text-sm font-medium tracking-normal text-on-surface">Failed to Load Rejected Coaches</h3>
        <p className="mt-0.5 text-xs text-outline">Unable to fetch rejected coaches. Please try again.</p>
      </div>
    );
  }

  const coaches = data?.coaches || [];

  if (coaches.length === 0) {
    return (
      <div className="py-3">
        <h3 className="font-sans text-sm font-medium tracking-normal text-on-surface">No Rejected Coaches</h3>
        <p className="mt-0.5 text-xs text-outline">Rejected coach submissions will appear here.</p>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      {/* Count */}
      <p className="text-sm text-on-surface-variant">
        {data?.total ?? coaches.length} rejected submission{coaches.length !== 1 ? 's' : ''}
      </p>

      {/* Rejected Coaches List */}
      <div className="space-y-3">
        {coaches.map((coach) => (
          <div
            key={coach.id}
            className="bg-surface-container-lowest border ghost-border rounded-xl p-4"
          >
            <div className="flex items-start justify-between gap-4">
              <div className="flex-1 min-w-0">
                {/* Header */}
                <div className="flex items-center gap-3 mb-2">
                  {coach.icon_url ? (
                    <img
                      src={coach.icon_url}
                      alt={coach.title}
                      className="w-10 h-10 rounded-lg object-cover border ghost-border flex-shrink-0"
                    />
                  ) : (
                    <div className="w-10 h-10 rounded-lg bg-primary-container flex items-center justify-center flex-shrink-0 opacity-60">
                      <span className="text-sm font-bold text-on-surface">
                        {coach.title.charAt(0).toUpperCase()}
                      </span>
                    </div>
                  )}
                  <div className="min-w-0">
                    <h3 className="font-semibold text-on-surface truncate">{coach.title}</h3>
                    <div className="flex items-center gap-2 mt-1">
                      <span className={clsx(
                        'px-2 py-0.5 text-xs font-medium rounded-full border',
                        getCategoryColorClass(coach.category)
                      )}>
                        {coach.category}
                      </span>
                      <span className="text-xs text-outline">
                        by {coach.author_email || 'Unknown'}
                      </span>
                    </div>
                  </div>
                </div>

                {/* Rejection Info */}
                <div className="mt-3 p-3 bg-error/10 border border-error/20 rounded-lg">
                  <div className="flex items-start gap-2">
                    <svg className="w-4 h-4 text-error flex-shrink-0 mt-0.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                    </svg>
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center justify-between gap-2">
                        <span className="text-sm font-medium text-error">
                          {getReasonLabel(coach.rejection_reason)}
                        </span>
                        <span className="text-xs text-outline flex-shrink-0">
                          {formatDate(coach.rejected_at)}
                        </span>
                      </div>
                      {coach.rejection_notes && (
                        <p className="text-xs text-on-surface-variant mt-1">
                          {coach.rejection_notes}
                        </p>
                      )}
                    </div>
                  </div>
                </div>

                {/* Tags */}
                {coach.tags.length > 0 && (
                  <div className="flex flex-wrap gap-1 mt-3">
                    {coach.tags.slice(0, 4).map((tag) => (
                      <span key={tag} className="px-2 py-0.5 text-xs bg-surface-container-high text-on-surface-variant rounded">
                        {tag}
                      </span>
                    ))}
                    {coach.tags.length > 4 && (
                      <span className="px-2 py-0.5 text-xs bg-surface-container-high text-outline rounded">
                        +{coach.tags.length - 4}
                      </span>
                    )}
                  </div>
                )}
              </div>

              {/* Metadata */}
              <div className="flex-shrink-0 text-right">
                <span className="text-xs text-outline">
                  {coach.token_count.toLocaleString()} tokens
                </span>
              </div>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
