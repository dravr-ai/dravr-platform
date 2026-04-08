// ABOUTME: Admin System Prompts management UI for contremaitre prompt hot-reload
// ABOUTME: Lists system prompts with source indicators, inline editor, and GitHub write-back
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { adminApi } from '../services/api';
import { Card, Button } from './ui';
import { clsx } from 'clsx';

const CONTREMAITRE_QUERY_KEYS = {
  status: ['admin', 'contremaitre', 'status'] as const,
  prompts: ['admin', 'contremaitre', 'prompts'] as const,
  prompt: (key: string) => ['admin', 'contremaitre', 'prompts', key] as const,
};

interface PromptSummary {
  key: string;
  sha256: string;
  source: 'compiled_in' | 'contremaitre';
  loaded_at: string;
  content_length: number;
}

export default function SystemPromptsTab() {
  const queryClient = useQueryClient();
  const [selectedPrompt, setSelectedPrompt] = useState<string | null>(null);
  const [editContent, setEditContent] = useState('');
  const [commitMessage, setCommitMessage] = useState('');
  const [isEditing, setIsEditing] = useState(false);

  // Fetch contremaitre status
  const { data: status } = useQuery({
    queryKey: CONTREMAITRE_QUERY_KEYS.status,
    queryFn: () => adminApi.getContremaitreStatus(),
  });

  // Fetch system prompts list
  const { data: promptsData, isLoading } = useQuery({
    queryKey: CONTREMAITRE_QUERY_KEYS.prompts,
    queryFn: () => adminApi.listSystemPrompts(),
  });

  // Fetch selected prompt detail
  const { data: promptDetail, isLoading: detailLoading } = useQuery({
    queryKey: CONTREMAITRE_QUERY_KEYS.prompt(selectedPrompt ?? ''),
    queryFn: () => adminApi.getSystemPrompt(selectedPrompt!),
    enabled: !!selectedPrompt,
  });

  // Update mutation
  const updateMutation = useMutation({
    mutationFn: (data: { key: string; content: string; commit_message?: string }) =>
      adminApi.updateSystemPrompt(data.key, {
        content: data.content,
        commit_message: data.commit_message,
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: CONTREMAITRE_QUERY_KEYS.prompts });
      if (selectedPrompt) {
        queryClient.invalidateQueries({
          queryKey: CONTREMAITRE_QUERY_KEYS.prompt(selectedPrompt),
        });
      }
      setIsEditing(false);
      setCommitMessage('');
    },
  });

  // Sync mutation
  const syncMutation = useMutation({
    mutationFn: () => adminApi.triggerContremaitreSync(),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: CONTREMAITRE_QUERY_KEYS.prompts });
      queryClient.invalidateQueries({ queryKey: CONTREMAITRE_QUERY_KEYS.status });
    },
  });

  const prompts = promptsData?.prompts ?? [];

  function handleSelectPrompt(key: string) {
    setSelectedPrompt(key);
    setIsEditing(false);
    setCommitMessage('');
  }

  function handleStartEditing() {
    if (promptDetail) {
      setEditContent(promptDetail.content);
      setIsEditing(true);
    }
  }

  function handleSave() {
    if (!selectedPrompt) return;
    updateMutation.mutate({
      key: selectedPrompt,
      content: editContent,
      commit_message: commitMessage || undefined,
    });
  }

  function handleCancelEdit() {
    setIsEditing(false);
    setCommitMessage('');
  }

  function formatPromptKey(key: string): string {
    return key.replace(/_/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase());
  }

  return (
    <div className="space-y-6">
      {/* Status bar */}
      <Card variant="dark" className="p-4">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-4">
            <h3 className="text-lg font-semibold text-pierre-text">System Prompts</h3>
            {status && (
              <div className="flex items-center gap-2 text-sm text-pierre-text-secondary">
                <span
                  className={clsx(
                    'inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium',
                    status.configured
                      ? 'bg-pierre-activity/10 text-pierre-activity'
                      : 'bg-pierre-text-secondary/10 text-pierre-text-secondary',
                  )}
                >
                  <span
                    className={clsx(
                      'w-1.5 h-1.5 rounded-full',
                      status.configured ? 'bg-pierre-activity' : 'bg-pierre-text-secondary',
                    )}
                  />
                  {status.configured ? 'Connected' : 'Local only'}
                </span>
                {status.repo && (
                  <span className="text-xs text-pierre-text-secondary">{status.repo}</span>
                )}
                <span className="text-xs">
                  {status.contremaitre_count} synced / {status.compiled_in_count} compiled-in
                </span>
              </div>
            )}
          </div>
          <Button
            variant="outline"
            size="sm"
            onClick={() => syncMutation.mutate()}
            disabled={syncMutation.isPending || !status?.configured}
            title={!status?.configured ? 'Contremaitre not configured' : 'Sync from GitHub'}
          >
            {syncMutation.isPending ? 'Syncing...' : 'Sync Now'}
          </Button>
        </div>
        {syncMutation.isSuccess && syncMutation.data && (
          <div className="mt-2 text-sm text-pierre-activity">
            Synced {syncMutation.data.synced}, skipped {syncMutation.data.skipped}
            {syncMutation.data.failed > 0 && `, ${syncMutation.data.failed} failed`}
          </div>
        )}
      </Card>

      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
        {/* Prompt list */}
        <Card variant="dark" className="p-0 lg:col-span-1">
          <div className="p-4 border-b border-pierre-border">
            <h4 className="text-sm font-medium text-pierre-text-secondary">
              {prompts.length} Prompt{prompts.length !== 1 ? 's' : ''}
            </h4>
          </div>
          {isLoading ? (
            <div className="flex justify-center py-8">
              <div className="pierre-spinner" />
            </div>
          ) : (
            <div className="divide-y divide-pierre-border">
              {prompts.map((prompt: PromptSummary) => (
                <button
                  key={prompt.key}
                  onClick={() => handleSelectPrompt(prompt.key)}
                  className={clsx(
                    'w-full text-left px-4 py-3 hover:bg-pierre-surface-hover transition-colors',
                    selectedPrompt === prompt.key && 'bg-pierre-violet/5 border-l-2 border-pierre-violet',
                  )}
                >
                  <div className="flex items-center justify-between">
                    <span className="text-sm font-medium text-pierre-text truncate">
                      {formatPromptKey(prompt.key)}
                    </span>
                    <span
                      className={clsx(
                        'inline-flex items-center px-1.5 py-0.5 rounded text-[10px] font-medium',
                        prompt.source === 'contremaitre'
                          ? 'bg-pierre-activity/10 text-pierre-activity'
                          : 'bg-pierre-text-secondary/10 text-pierre-text-secondary',
                      )}
                    >
                      {prompt.source === 'contremaitre' ? 'git' : 'built-in'}
                    </span>
                  </div>
                  <div className="text-xs text-pierre-text-secondary mt-0.5">
                    {prompt.content_length.toLocaleString()} chars
                  </div>
                </button>
              ))}
            </div>
          )}
        </Card>

        {/* Prompt detail / editor */}
        <Card variant="dark" className="p-0 lg:col-span-2">
          {!selectedPrompt ? (
            <div className="flex items-center justify-center h-64 text-pierre-text-secondary">
              Select a prompt to view or edit
            </div>
          ) : detailLoading ? (
            <div className="flex justify-center py-8">
              <div className="pierre-spinner" />
            </div>
          ) : promptDetail ? (
            <div className="flex flex-col h-full">
              {/* Header */}
              <div className="flex items-center justify-between p-4 border-b border-pierre-border">
                <div>
                  <h4 className="text-base font-semibold text-pierre-text">
                    {formatPromptKey(promptDetail.key)}
                  </h4>
                  <div className="flex items-center gap-2 mt-1">
                    <span
                      className={clsx(
                        'inline-flex items-center px-1.5 py-0.5 rounded text-[10px] font-medium',
                        promptDetail.source === 'contremaitre'
                          ? 'bg-pierre-activity/10 text-pierre-activity'
                          : 'bg-pierre-text-secondary/10 text-pierre-text-secondary',
                      )}
                    >
                      {promptDetail.source === 'contremaitre' ? 'git' : 'built-in'}
                    </span>
                    <span className="text-xs text-pierre-text-secondary font-mono">
                      {promptDetail.sha256.slice(0, 12)}
                    </span>
                  </div>
                </div>
                {!isEditing ? (
                  <Button variant="primary" size="sm" onClick={handleStartEditing}>
                    Edit
                  </Button>
                ) : (
                  <div className="flex items-center gap-2">
                    <Button variant="secondary" size="sm" onClick={handleCancelEdit}>
                      Cancel
                    </Button>
                    <Button
                      variant="primary"
                      size="sm"
                      onClick={handleSave}
                      disabled={updateMutation.isPending}
                    >
                      {updateMutation.isPending ? 'Saving...' : 'Save'}
                    </Button>
                  </div>
                )}
              </div>

              {/* Content */}
              {isEditing ? (
                <div className="flex flex-col flex-1 p-4 gap-3">
                  <textarea
                    value={editContent}
                    onChange={(e) => setEditContent(e.target.value)}
                    className="flex-1 min-h-[400px] w-full p-3 rounded-lg bg-pierre-bg border border-pierre-border text-pierre-text text-sm font-mono resize-y focus:outline-none focus:ring-2 focus:ring-pierre-violet/50"
                    spellCheck={false}
                  />
                  {status?.configured && (
                    <div>
                      <label
                        htmlFor="commit-message"
                        className="block text-xs font-medium text-pierre-text-secondary mb-1"
                      >
                        Commit message (optional)
                      </label>
                      <input
                        id="commit-message"
                        type="text"
                        value={commitMessage}
                        onChange={(e) => setCommitMessage(e.target.value)}
                        placeholder={`Update system prompt: ${selectedPrompt}`}
                        className="w-full px-3 py-2 rounded-lg bg-pierre-bg border border-pierre-border text-pierre-text text-sm focus:outline-none focus:ring-2 focus:ring-pierre-violet/50"
                      />
                    </div>
                  )}
                  {updateMutation.isError && (
                    <div className="text-sm text-red-400">
                      Failed to save: {(updateMutation.error as Error).message}
                    </div>
                  )}
                  {updateMutation.isSuccess && updateMutation.data?.commit_sha && (
                    <div className="text-sm text-pierre-activity">
                      Committed to GitHub: {updateMutation.data.commit_sha.slice(0, 8)}
                    </div>
                  )}
                </div>
              ) : (
                <div className="p-4 overflow-auto">
                  <pre className="text-sm text-pierre-text font-mono whitespace-pre-wrap break-words leading-relaxed">
                    {promptDetail.content}
                  </pre>
                </div>
              )}
            </div>
          ) : null}
        </Card>
      </div>
    </div>
  );
}
