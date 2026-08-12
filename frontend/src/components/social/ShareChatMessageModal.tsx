// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Modal for sharing an insight to the social feed
// ABOUTME: Shows preview mode with Edit button, then allows editing before sharing

import { useState, useEffect } from 'react';
import { Pencil } from 'lucide-react';
import { socialApi } from '../../services/api';
import { Button, Modal, InsightPreview, Textarea } from '../ui';
import type { ShareVisibility } from '../../types/social';

interface ShareChatMessageModalProps {
  /** Whether the modal is open (defaults to true when component is rendered) */
  isOpen?: boolean;
  /** The message content to share */
  content: string;
  /** Callback when modal is closed */
  onClose: () => void;
  /** Callback when share is successful */
  onSuccess: () => void;
}

const VISIBILITY_OPTIONS: { value: ShareVisibility; label: string; description: string }[] = [
  { value: 'friends_only', label: 'Friends Only', description: 'Only your friends can see this' },
  { value: 'public', label: 'Public', description: 'Anyone can see this' },
];

export default function ShareChatMessageModal({
  isOpen = true,
  content,
  onClose,
  onSuccess,
}: ShareChatMessageModalProps) {
  const [visibility, setVisibility] = useState<ShareVisibility>('friends_only');
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [isEditing, setIsEditing] = useState(false);
  const [editedContent, setEditedContent] = useState(content);

  // Sync editedContent when content prop changes (component stays mounted)
  useEffect(() => {
    setEditedContent(content);
  }, [content]);

  const handleShare = async () => {
    setIsSubmitting(true);
    setError(null);

    try {
      await socialApi.shareFromActivity({
        content: editedContent,
        insight_type: 'coaching_insight',
        visibility,
      });
      onSuccess();
      onClose();
    } catch (err) {
      console.error('Failed to share message:', err);
      setError(err instanceof Error ? err.message : 'Failed to share. Please try again.');
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Modal isOpen={isOpen} onClose={onClose} title="Share Insight" size="2xl">
      <div className="space-y-6">
        {/* Message Preview or Edit Mode */}
        {isEditing ? (
          <div>
            <div className="flex items-center justify-between mb-2">
              <label className="block text-sm font-medium text-on-surface">
                Edit Your Insight
              </label>
              <button
                onClick={() => setIsEditing(false)}
                className="text-xs text-on-surface-variant hover:text-on-surface transition-colors"
              >
                Done Editing
              </button>
            </div>
            <Textarea
              value={editedContent}
              onChange={(e) => setEditedContent(e.target.value)}
              className="h-64 leading-relaxed"
              placeholder="Edit your insight..."
            />
          </div>
        ) : (
          <div>
            <div className="flex items-center justify-between mb-2">
              <label className="block text-sm font-medium text-on-surface">
                Message to Share
              </label>
              <button
                onClick={() => setIsEditing(true)}
                className="flex items-center gap-1 text-xs text-primary hover:text-primary/80 transition-colors"
              >
                <Pencil className="w-3 h-3" />
                Edit
              </button>
            </div>
            <InsightPreview
              content={editedContent}
              maxHeight="max-h-80"
            />
          </div>
        )}

        {/* Visibility Selection */}
        <div>
          <label className="block text-sm font-medium text-on-surface mb-3">
            Who can see this?
          </label>
          <div className="space-y-2">
            {VISIBILITY_OPTIONS.map((option) => (
              <label
                key={option.value}
                className={`flex items-start p-3 rounded-lg cursor-pointer transition-colors ${
                  visibility === option.value
                    ? 'bg-primary/20 border-primary border'
                    : 'bg-surface-container-low border-transparent border hover:bg-surface-container'
                }`}
              >
                <input
                  type="radio"
                  name="visibility"
                  value={option.value}
                  checked={visibility === option.value}
                  onChange={(e) => setVisibility(e.target.value as ShareVisibility)}
                  className="mt-1 mr-3"
                />
                <div>
                  <div className="text-sm font-medium text-on-surface">{option.label}</div>
                  <div className="text-xs text-on-surface-variant">{option.description}</div>
                </div>
              </label>
            ))}
          </div>
        </div>

        {/* Error Message */}
        {error && (
          <div className="bg-error/20 border border-error/30 rounded-lg p-3">
            <p className="text-sm text-error">{error}</p>
          </div>
        )}

        {/* Actions */}
        <div className="flex gap-3 justify-end">
          <Button variant="secondary" onClick={onClose} disabled={isSubmitting}>
            Cancel
          </Button>
          <Button
            variant="primary"
            onClick={handleShare}
            disabled={isSubmitting}
          >
            {isSubmitting ? 'Sharing...' : 'Share'}
          </Button>
        </div>
      </div>
    </Modal>
  );
}
