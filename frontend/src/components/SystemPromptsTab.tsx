// ABOUTME: Admin System Prompts management UI for contremaitre prompt hot-reload
// ABOUTME: Lists system prompts with an editor and GitHub write-back, and agent prompts to read per locale
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  LANGUAGE_NAMES,
  SUPPORTED_LANGUAGES,
  type SupportedLanguage,
} from "@pierre/i18n";
import { adminApi } from "../services/api";
import { useAdminScope } from "../hooks/useAdminScope";
import { Card, Button, Textarea } from "./ui";
import { clsx } from "clsx";
import { ADMIN_LOCALE } from "../utils/adminLocale";

const CONTREMAITRE_QUERY_KEYS = {
  status: ["admin", "contremaitre", "status"] as const,
  prompts: ["admin", "contremaitre", "prompts"] as const,
  prompt: (key: string) => ["admin", "contremaitre", "prompts", key] as const,
  agentPrompts: ["admin", "contremaitre", "agent-prompts"] as const,
  agentPrompt: (slug: string, locale: string) =>
    ["admin", "contremaitre", "agent-prompts", slug, locale] as const,
};

function formatPromptKey(key: string): string {
  return key.replace(/[_-]/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
}

/** The name of a locale the server returned, which may be outside the client's list. */
function languageName(locale: string): string {
  return LANGUAGE_NAMES[locale as SupportedLanguage] ?? locale;
}

/** Read-only view of one agent's prompt, with a language control. */
function AgentPromptPanel({ slug }: { slug: string }) {
  const [locale, setLocale] = useState<SupportedLanguage>("en");
  const {
    data: prompt,
    isLoading,
    isError,
  } = useQuery({
    queryKey: CONTREMAITRE_QUERY_KEYS.agentPrompt(slug, locale),
    queryFn: () => adminApi.getAgentPrompt(slug, locale),
  });

  return (
    <div className="flex flex-col h-full">
      <div className="p-4 border-b border-outline-variant space-y-3">
        <h3 className="text-base font-semibold text-on-surface">
          {formatPromptKey(slug)}
        </h3>
        <div
          className="flex flex-wrap gap-1"
          role="group"
          aria-label="Prompt language"
        >
          {SUPPORTED_LANGUAGES.map((language) => (
            <button
              key={language}
              type="button"
              aria-pressed={locale === language}
              onClick={() => setLocale(language)}
              className={clsx(
                "rounded px-3 py-1 text-xs font-medium",
                locale === language
                  ? "bg-primary text-on-primary"
                  : "bg-surface-container text-on-surface-variant hover:bg-surface-container-high",
              )}
            >
              {LANGUAGE_NAMES[language]}
            </button>
          ))}
        </div>
        {prompt &&
          (prompt.locale === prompt.requested_locale ? (
            <p className="text-xs text-on-surface-variant">
              Showing the {languageName(prompt.locale)} prompt.
            </p>
          ) : (
            <p className="text-xs text-on-surface" role="status">
              No {languageName(prompt.requested_locale)} translation is loaded.
              Showing the {languageName(prompt.locale)} prompt, which is what an
              athlete reading {languageName(prompt.requested_locale)} is served.
            </p>
          ))}
      </div>
      {isLoading ? (
        <div className="flex justify-center py-8">
          <div className="pierre-spinner" />
        </div>
      ) : isError ? (
        <div className="p-4 text-sm text-error" role="alert">
          This prompt could not be loaded in {languageName(locale)}.
        </div>
      ) : (
        prompt && (
          <div className="p-4 overflow-auto">
            <pre className="text-sm text-on-surface font-mono whitespace-pre-wrap break-words leading-relaxed">
              {prompt.content}
            </pre>
          </div>
        )
      )}
    </div>
  );
}

interface PromptSummary {
  key: string;
  sha256: string;
  source: "compiled_in" | "contremaitre";
  loaded_at: string;
  content_length: number;
}

export default function SystemPromptsTab() {
  const queryClient = useQueryClient();
  // The server refuses a plain admin the sync state and the system prompts, so
  // neither is asked for; agent prompts are every admin's to read.
  const { isSuperAdmin } = useAdminScope();
  const [selectedPrompt, setSelectedPrompt] = useState<string | null>(null);
  const [selectedAgent, setSelectedAgent] = useState<string | null>(null);
  const [editContent, setEditContent] = useState("");
  const [commitMessage, setCommitMessage] = useState("");
  const [isEditing, setIsEditing] = useState(false);

  // Fetch contremaitre status
  const { data: status } = useQuery({
    queryKey: CONTREMAITRE_QUERY_KEYS.status,
    queryFn: () => adminApi.getContremaitreStatus(),
    enabled: isSuperAdmin,
  });

  // Fetch system prompts list
  const {
    data: promptsData,
    isLoading,
    isError: promptsFailed,
  } = useQuery({
    queryKey: CONTREMAITRE_QUERY_KEYS.prompts,
    queryFn: () => adminApi.listSystemPrompts(),
    enabled: isSuperAdmin,
  });

  // Fetch the agents the registry holds
  const { data: agentPromptsData, isError: agentPromptsFailed } = useQuery({
    queryKey: CONTREMAITRE_QUERY_KEYS.agentPrompts,
    queryFn: () => adminApi.listAgentPrompts(),
  });

  // Fetch selected prompt detail
  const { data: promptDetail, isLoading: detailLoading } = useQuery({
    queryKey: CONTREMAITRE_QUERY_KEYS.prompt(selectedPrompt ?? ""),
    queryFn: () => adminApi.getSystemPrompt(selectedPrompt!),
    enabled: !!selectedPrompt,
  });

  // Update mutation
  const updateMutation = useMutation({
    mutationFn: (data: {
      key: string;
      content: string;
      commit_message?: string;
    }) =>
      adminApi.updateSystemPrompt(data.key, {
        content: data.content,
        commit_message: data.commit_message,
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: CONTREMAITRE_QUERY_KEYS.prompts,
      });
      if (selectedPrompt) {
        queryClient.invalidateQueries({
          queryKey: CONTREMAITRE_QUERY_KEYS.prompt(selectedPrompt),
        });
      }
      setIsEditing(false);
      setCommitMessage("");
    },
  });

  // Sync mutation
  const syncMutation = useMutation({
    mutationFn: () => adminApi.triggerContremaitreSync(),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: CONTREMAITRE_QUERY_KEYS.prompts,
      });
      queryClient.invalidateQueries({
        queryKey: CONTREMAITRE_QUERY_KEYS.agentPrompts,
      });
      queryClient.invalidateQueries({
        queryKey: CONTREMAITRE_QUERY_KEYS.status,
      });
    },
  });

  const prompts = promptsData?.prompts ?? [];
  const agentPrompts = agentPromptsData?.prompts ?? [];

  function handleSelectAgent(slug: string) {
    setSelectedAgent(slug);
    setSelectedPrompt(null);
    setIsEditing(false);
    setCommitMessage("");
  }

  function handleSelectPrompt(key: string) {
    setSelectedPrompt(key);
    setSelectedAgent(null);
    setIsEditing(false);
    setCommitMessage("");
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
    setCommitMessage("");
  }

  return (
    <div className="space-y-6">
      {/* Status bar */}
      <Card variant="dark" className="p-4">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-4">
            <h2 className="text-lg font-semibold text-on-surface">Prompts</h2>
            {!isSuperAdmin && (
              <span className="text-sm text-on-surface-variant">
                Agent prompts, read-only. System prompts and sync are for super
                admins.
              </span>
            )}
            {status && (
              <div className="flex items-center gap-2 text-sm text-on-surface-variant">
                <span
                  className={clsx(
                    "inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium",
                    status.configured
                      ? "bg-activity/10 text-on-activity-container"
                      : "bg-on-surface-variant/10 text-on-surface-variant",
                  )}
                >
                  <span
                    className={clsx(
                      "w-1.5 h-1.5 rounded-full",
                      status.configured
                        ? "bg-activity"
                        : "bg-on-surface-variant",
                    )}
                  />
                  {status.configured
                    ? "Sync configured"
                    : "Sync not configured"}
                </span>
                {status.repo && (
                  <span className="text-xs text-on-surface-variant">
                    {status.repo}
                  </span>
                )}
                <span
                  className="text-xs"
                  data-testid="prompt-counts"
                  title="Agent prompts are one per agent per language. Persona blocks are not listed on this screen. A seed copy is the text compiled into the server, which no sync has replaced."
                >
                  {status.system_prompt_count +
                    status.agent_prompt_count +
                    status.persona_count}{" "}
                  prompts: {status.system_prompt_count} system,{" "}
                  {status.agent_prompt_count} agent per language,{" "}
                  {status.persona_count} persona. {status.contremaitre_count}{" "}
                  synced from contremaitre, {status.compiled_in_count} seed
                  copies.
                </span>
              </div>
            )}
          </div>
          {isSuperAdmin && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => syncMutation.mutate()}
              disabled={syncMutation.isPending || !status?.configured}
              title={
                !status?.configured
                  ? "Contremaitre not configured"
                  : "Sync from GitHub"
              }
            >
              {syncMutation.isPending ? "Syncing..." : "Sync Now"}
            </Button>
          )}
        </div>
        {syncMutation.isSuccess && syncMutation.data && (
          <div className="mt-2 text-sm text-activity">
            Synced {syncMutation.data.synced}, skipped{" "}
            {syncMutation.data.skipped}
            {syncMutation.data.failed > 0 &&
              `, ${syncMutation.data.failed} failed`}
          </div>
        )}
      </Card>

      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
        {/* Prompt list */}
        <Card variant="dark" className="p-0 lg:col-span-1">
          {isSuperAdmin && (
            <>
              <div className="p-4 border-b border-outline-variant">
                <h3 className="text-sm font-medium text-on-surface-variant">
                  {promptsFailed
                    ? "System Prompts"
                    : `${prompts.length} System Prompt${prompts.length !== 1 ? "s" : ""}`}
                </h3>
                <p className="mt-1 text-xs text-on-surface-variant/70">
                  Written in English only: these are instructions to the model,
                  in every locale.
                </p>
              </div>
              {isLoading ? (
                <div className="flex justify-center py-8">
                  <div className="pierre-spinner" />
                </div>
              ) : promptsFailed ? (
                <p className="p-4 text-sm text-error" role="alert">
                  System prompts could not be loaded.
                </p>
              ) : (
                <div className="divide-y divide-outline-variant">
                  {prompts.map((prompt: PromptSummary) => (
                    <button
                      key={prompt.key}
                      onClick={() => handleSelectPrompt(prompt.key)}
                      className={clsx(
                        "w-full text-left px-4 py-3 hover:bg-surface-container-high transition-colors",
                        selectedPrompt === prompt.key &&
                          "bg-primary/5 border-l-2 border-primary",
                      )}
                    >
                      <div className="flex items-center justify-between">
                        <span className="text-sm font-medium text-on-surface truncate">
                          {formatPromptKey(prompt.key)}
                        </span>
                        <span
                          className={clsx(
                            "inline-flex items-center px-1.5 py-0.5 rounded text-xs font-medium",
                            prompt.source === "contremaitre"
                              ? "bg-activity/10 text-on-activity-container"
                              : "bg-on-surface-variant/10 text-on-surface-variant",
                          )}
                        >
                          {prompt.source === "contremaitre"
                            ? "synced"
                            : "seed copy"}
                        </span>
                      </div>
                      <div className="text-xs text-on-surface-variant mt-0.5">
                        {prompt.content_length.toLocaleString(ADMIN_LOCALE)}{" "}
                        chars
                      </div>
                    </button>
                  ))}
                </div>
              )}
            </>
          )}
          <div className="p-4 border-y border-outline-variant">
            <h3 className="text-sm font-medium text-on-surface-variant">
              {agentPromptsFailed
                ? "Agent Prompts"
                : `${agentPrompts.length} Agent Prompt${agentPrompts.length !== 1 ? "s" : ""}`}
            </h3>
            <p className="mt-1 text-xs text-on-surface-variant/70">
              Read-only here, in each language athletes are served.
            </p>
          </div>
          {agentPromptsFailed && (
            <p className="p-4 text-sm text-error" role="alert">
              Agent prompts could not be loaded.
            </p>
          )}
          <div className="divide-y divide-outline-variant">
            {agentPrompts.map((agent) => (
              <button
                key={agent.key}
                onClick={() => handleSelectAgent(agent.key)}
                className={clsx(
                  "w-full text-left px-4 py-3 hover:bg-surface-container-high transition-colors",
                  selectedAgent === agent.key &&
                    "bg-primary/5 border-l-2 border-primary",
                )}
              >
                <span className="block text-sm font-medium text-on-surface truncate">
                  {formatPromptKey(agent.key)}
                </span>
                <span className="block text-xs text-on-surface-variant mt-0.5">
                  {agent.locales.join(" · ")}
                </span>
              </button>
            ))}
          </div>
        </Card>

        {/* Prompt detail / editor. Drawn once the list has loaded: on a
            narrow screen it sits below the list, and painted first it would
            be pushed off screen when the list lands above it. */}
        {!isLoading && (
          <Card variant="dark" className="p-0 lg:col-span-2">
            {selectedAgent ? (
              <AgentPromptPanel key={selectedAgent} slug={selectedAgent} />
            ) : !selectedPrompt ? (
              <div className="flex items-center justify-center h-64 text-on-surface-variant">
                {isSuperAdmin
                  ? "Select a prompt to view or edit"
                  : "Select an agent prompt to read"}
              </div>
            ) : detailLoading ? (
              <div className="flex justify-center py-8">
                <div className="pierre-spinner" />
              </div>
            ) : promptDetail ? (
              <div className="flex flex-col h-full">
                {/* Header */}
                <div className="flex items-center justify-between p-4 border-b border-outline-variant">
                  <div>
                    <h3 className="text-base font-semibold text-on-surface">
                      {formatPromptKey(promptDetail.key)}
                    </h3>
                    <div className="flex items-center gap-2 mt-1">
                      <span
                        className={clsx(
                          "inline-flex items-center px-1.5 py-0.5 rounded text-xs font-medium",
                          promptDetail.source === "contremaitre"
                            ? "bg-activity/10 text-on-activity-container"
                            : "bg-on-surface-variant/10 text-on-surface-variant",
                        )}
                      >
                        {promptDetail.source === "contremaitre"
                          ? "synced"
                          : "seed copy"}
                      </span>
                      <span className="text-xs text-on-surface-variant font-mono">
                        {promptDetail.sha256.slice(0, 12)}
                      </span>
                      <span className="text-xs text-on-surface-variant">
                        English only
                      </span>
                    </div>
                  </div>
                  {!isEditing ? (
                    <Button
                      variant="primary"
                      size="sm"
                      onClick={handleStartEditing}
                    >
                      Edit
                    </Button>
                  ) : (
                    <div className="flex items-center gap-2">
                      <Button
                        variant="secondary"
                        size="sm"
                        onClick={handleCancelEdit}
                      >
                        Cancel
                      </Button>
                      <Button
                        variant="primary"
                        size="sm"
                        onClick={handleSave}
                        disabled={updateMutation.isPending}
                      >
                        {updateMutation.isPending ? "Saving..." : "Save"}
                      </Button>
                    </div>
                  )}
                </div>

                {/* Content */}
                {isEditing ? (
                  <div className="flex flex-col flex-1 p-4 gap-3">
                    <Textarea
                      value={editContent}
                      onChange={(e) => setEditContent(e.target.value)}
                      className="flex-1 min-h-[400px] font-mono resize-y"
                      spellCheck={false}
                    />
                    {status?.configured && (
                      <div>
                        <label
                          htmlFor="commit-message"
                          className="block text-xs font-medium text-on-surface-variant mb-1"
                        >
                          Commit message (optional)
                        </label>
                        <input
                          id="commit-message"
                          type="text"
                          value={commitMessage}
                          onChange={(e) => setCommitMessage(e.target.value)}
                          placeholder={`Update system prompt: ${selectedPrompt}`}
                          className="w-full px-3 py-2 rounded-lg bg-surface border border-outline-variant text-on-surface text-sm focus:outline-none focus:ring-2 focus:ring-primary/50"
                        />
                      </div>
                    )}
                    {updateMutation.isError && (
                      <div className="text-sm text-error">
                        Failed to save:{" "}
                        {(updateMutation.error as Error).message}
                      </div>
                    )}
                    {updateMutation.isSuccess &&
                      updateMutation.data?.commit_sha && (
                        <div className="text-sm text-activity">
                          Committed to GitHub:{" "}
                          {updateMutation.data.commit_sha.slice(0, 8)}
                        </div>
                      )}
                  </div>
                ) : (
                  <div className="p-4 overflow-auto">
                    <pre className="text-sm text-on-surface font-mono whitespace-pre-wrap break-words leading-relaxed">
                      {promptDetail.content}
                    </pre>
                  </div>
                )}
              </div>
            ) : null}
          </Card>
        )}
      </div>
    </div>
  );
}
