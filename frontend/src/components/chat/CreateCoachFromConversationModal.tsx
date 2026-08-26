// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Modal for generating a coach from conversation history using LLM analysis
// ABOUTME: Shows analysis state, pre-fills form with suggestions, allows editing and saving

import { useState, useEffect, useCallback } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { coachesApi } from '../../services/api';
import type { CoachFormData } from './types';
import { DEFAULT_COACH_FORM_DATA } from './types';
import { Sparkles, RefreshCw, AlertCircle, MessageSquareText } from 'lucide-react';
import { QUERY_KEYS } from '../../constants/queryKeys';
import { Select, Textarea } from '../ui';

interface CreateCoachFromConversationModalProps {
  isOpen: boolean;
  conversationId: string;
  messageCount: number;
  onClose: () => void;
  onSuccess: () => void;
}

type AnalysisState = 'idle' | 'analyzing' | 'ready' | 'error';

const MAX_MESSAGES_ANALYZED = 10;

export default function CreateCoachFromConversationModal({
  isOpen,
  conversationId,
  messageCount,
  onClose,
  onSuccess,
}: CreateCoachFromConversationModalProps) {
  const queryClient = useQueryClient();
  const [analysisState, setAnalysisState] = useState<AnalysisState>('idle');
  const [messagesAnalyzed, setMessagesAnalyzed] = useState(0);
  const [totalMessages, setTotalMessages] = useState(0);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [formData, setFormData] = useState<CoachFormData>({
    ...DEFAULT_COACH_FORM_DATA,
    category: 'Custom',
  });

  const generateMutation = useMutation({
    mutationFn: () =>
      coachesApi.generateFromConversation({
        conversation_id: conversationId,
        max_messages: MAX_MESSAGES_ANALYZED,
      }),
    onSuccess: (data) => {
      setFormData({
        ...DEFAULT_COACH_FORM_DATA,
        title: data.title,
        description: data.description,
        system_prompt: data.system_prompt,
        category: data.category,
      });
      setMessagesAnalyzed(data.messages_analyzed);
      setTotalMessages(data.total_messages);
      setAnalysisState('ready');
      setErrorMessage(null);
    },
    onError: (error) => {
      const message = error instanceof Error ? error.message : 'Failed to analyze conversation';
      setErrorMessage(message);
      setAnalysisState('error');
    },
  });

  const createMutation = useMutation({
    // This modal only ever collects these four fields; naming them keeps the
    // request a `CreateCoachRequest` rather than the whole form blob.
    mutationFn: (data: CoachFormData) =>
      coachesApi.create({
        title: data.title,
        description: data.description || undefined,
        system_prompt: data.system_prompt,
        category: data.category,
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.lists() });
      onSuccess();
    },
  });

  const startAnalysis = useCallback(() => {
    setAnalysisState('analyzing');
    setErrorMessage(null);
    generateMutation.mutate();
  }, [generateMutation]);

  // Start analysis when modal opens
  useEffect(() => {
    if (isOpen && analysisState === 'idle') {
      startAnalysis();
    }
  }, [isOpen, analysisState, startAnalysis]);

  // Reset state when modal closes
  useEffect(() => {
    if (!isOpen) {
      setAnalysisState('idle');
      setErrorMessage(null);
      setFormData({
        ...DEFAULT_COACH_FORM_DATA,
        category: 'Custom',
      });
      setMessagesAnalyzed(0);
      setTotalMessages(0);
    }
  }, [isOpen]);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!formData.title.trim() || !formData.system_prompt.trim()) return;
    createMutation.mutate(formData);
  };

  const handleRegenerate = () => {
    startAnalysis();
  };

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      {/* Backdrop */}
      <div
        className="absolute inset-0 bg-black/50 backdrop-blur-sm"
        onClick={onClose}
      />
      {/* Modal Content */}
      <div className="relative bg-surface rounded-2xl shadow-2xl max-w-lg w-full mx-4 max-h-[90vh] overflow-y-auto">
        <div className="p-6">
          {/* Close button */}
          <button
            onClick={onClose}
            className="absolute top-4 right-4 p-2 text-on-surface-variant hover:text-on-surface-variant hover:bg-surface-container-high rounded-lg transition-colors"
            aria-label="Close"
          >
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>

          <div className="text-center mb-6">
            <div className="w-12 h-12 boreal-hero-gradient rounded-xl flex items-center justify-center mx-auto mb-4">
              <Sparkles className="w-6 h-6 text-on-primary" />
            </div>
            <h2 className="text-xl font-semibold text-on-surface mb-2">
              Create Coach from Conversation
            </h2>
            <p className="text-on-surface-variant text-sm">
              AI analyzes your conversation to generate a specialized coach
            </p>
          </div>

          {/* Analyzing State */}
          {analysisState === 'analyzing' && (
            <div className="text-center py-8">
              <div className="w-16 h-16 mx-auto mb-4 relative">
                <div className="absolute inset-0 bg-primary/10 rounded-full animate-ping" />
                <div className="relative w-16 h-16 bg-primary/10 rounded-full flex items-center justify-center">
                  <MessageSquareText className="w-8 h-8 text-primary animate-pulse" />
                </div>
              </div>
              <p className="text-on-surface-variant font-medium mb-2">Analyzing conversation...</p>
              <p className="text-on-surface-variant text-sm">
                Reading last {Math.min(messageCount, MAX_MESSAGES_ANALYZED)} of {messageCount} messages
              </p>
            </div>
          )}

          {/* Error State */}
          {analysisState === 'error' && (
            <div className="text-center py-8">
              <div className="w-16 h-16 mx-auto mb-4 bg-error/10 rounded-full flex items-center justify-center">
                <AlertCircle className="w-8 h-8 text-error" />
              </div>
              <p className="text-on-surface-variant font-medium mb-2">Analysis Failed</p>
              <p className="text-on-surface-variant text-sm mb-4">{errorMessage}</p>
              <button
                onClick={handleRegenerate}
                className="inline-flex items-center gap-2 px-4 py-2 text-sm font-medium text-on-primary bg-primary rounded-lg hover:bg-primary/90 transition-colors"
              >
                <RefreshCw className="w-4 h-4" />
                Try Again
              </button>
            </div>
          )}

          {/* Ready State - Form */}
          {analysisState === 'ready' && (
            <>
              {/* Analysis Info Banner */}
              <div className="mb-4 p-3 bg-primary/5 border border-primary/10 rounded-lg">
                <div className="flex items-center gap-2 text-sm text-primary">
                  <MessageSquareText className="w-4 h-4" />
                  <span>
                    Analyzed {messagesAnalyzed} of {totalMessages} messages
                  </span>
                  <button
                    onClick={handleRegenerate}
                    disabled={generateMutation.isPending}
                    className="ml-auto p-1.5 hover:bg-primary/10 rounded-lg transition-colors disabled:opacity-50"
                    title="Regenerate suggestions"
                  >
                    <RefreshCw className={`w-4 h-4 ${generateMutation.isPending ? 'animate-spin' : ''}`} />
                  </button>
                </div>
              </div>

              <form onSubmit={handleSubmit} className="space-y-4">
                <div>
                  <label className="block text-sm font-medium text-on-surface-variant mb-1">
                    Coach Name
                  </label>
                  <input
                    type="text"
                    placeholder="e.g., Marathon Training Coach"
                    value={formData.title}
                    onChange={(e) => setFormData({ ...formData, title: e.target.value })}
                    className="w-full px-3 py-2 text-sm border ghost-border rounded-lg focus:outline-none focus:ring-2 focus:ring-primary focus:border-transparent"
                    required
                  />
                </div>

                <div>
                  <label className="block text-sm font-medium text-on-surface-variant mb-1">
                    Description <span className="text-on-surface-variant">(optional)</span>
                  </label>
                  <input
                    type="text"
                    placeholder="Brief description of what this coach specializes in"
                    value={formData.description}
                    onChange={(e) => setFormData({ ...formData, description: e.target.value })}
                    className="w-full px-3 py-2 text-sm border ghost-border rounded-lg focus:outline-none focus:ring-2 focus:ring-primary focus:border-transparent"
                  />
                </div>

                <Textarea
                  label="System Prompt"
                  placeholder="Define your coach's personality, expertise, and communication style..."
                  value={formData.system_prompt}
                  onChange={(e) => setFormData({ ...formData, system_prompt: e.target.value })}
                  rows={6}
                  required
                  helpText={
                    formData.system_prompt
                      ? `~${Math.ceil(formData.system_prompt.length / 4)} tokens (${((Math.ceil(formData.system_prompt.length / 4) / 128000) * 100).toFixed(1)}% of context)`
                      : undefined
                  }
                />

                <Select
                  label="Category"
                  value={formData.category}
                  onChange={(e) => setFormData({ ...formData, category: e.target.value })}
                  options={[
                    { value: 'Training', label: 'Training' },
                    { value: 'Nutrition', label: 'Nutrition' },
                    { value: 'Recovery', label: 'Recovery' },
                    { value: 'Recipes', label: 'Recipes' },
                    { value: 'Mobility', label: 'Mobility' },
                    { value: 'Analysis', label: 'Analysis' },
                    { value: 'Custom', label: 'Custom' },
                  ]}
                />

                <div className="flex gap-3 pt-2">
                  <button
                    type="button"
                    onClick={onClose}
                    className="flex-1 px-4 py-2 text-sm font-medium text-on-surface-variant bg-surface-container-high rounded-lg hover:bg-surface-container-highest transition-colors"
                  >
                    Cancel
                  </button>
                  <button
                    type="submit"
                    disabled={createMutation.isPending || !formData.title.trim() || !formData.system_prompt.trim()}
                    className="flex-1 px-4 py-2 text-sm font-medium text-on-primary bg-primary rounded-lg hover:bg-primary/90 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
                  >
                    {createMutation.isPending ? 'Saving...' : 'Save Coach'}
                  </button>
                </div>

                {createMutation.isError && (
                  <p className="text-xs text-error text-center">
                    Failed to create coach. Please try again.
                  </p>
                )}
              </form>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
