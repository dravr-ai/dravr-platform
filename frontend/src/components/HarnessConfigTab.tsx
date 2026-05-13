// ABOUTME: Phase B Sprint C3 — admin tab editing the global coaching harness configuration
// ABOUTME: Compaction thresholds + Tier 6 text guardrails persisted via /admin/settings/harness
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState, useEffect, useMemo } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import {
  adminApi,
  type HarnessConfigDocument,
  type HarnessConfigResponse,
} from '../services/api/admin';
import { Card, Button, Badge } from './ui';

const HARNESS_CONFIG_QUERY_KEY = ['admin', 'harness-config'] as const;

/// Default document used as the form fallback before the API responds.
function defaultDocument(): HarnessConfigDocument {
  return {
    schema_version: 1,
    compaction: {
      window_tokens: 128_000,
      warn_threshold: 0.7,
      emergency_threshold: 0.95,
      summarize_oldest_n: 6,
      sliding_drop_n: 4,
    },
    guardrails: {
      max_response_chars: 5_000,
      blocked_topics: [],
      disclaimer_triggers: ['injury', 'pain', 'medication', 'diagnose', 'doctor'],
      disclaimer_text:
        "**Medical disclaimer:** I'm a fitness coach, not a medical professional. " +
        'Please consult a qualified clinician for any injury, pain, or medication question.',
    },
  };
}

function parseCsv(value: string): string[] {
  return value
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean);
}

function joinCsv(values: string[]): string {
  return values.join(', ');
}

interface ValidationError {
  field: string;
  message: string;
}

function validate(doc: HarnessConfigDocument): ValidationError[] {
  const errors: ValidationError[] = [];
  const c = doc.compaction;
  if (!Number.isFinite(c.window_tokens) || c.window_tokens <= 0) {
    errors.push({ field: 'window_tokens', message: 'must be a positive integer' });
  }
  if (c.warn_threshold <= 0 || c.warn_threshold > 1) {
    errors.push({ field: 'warn_threshold', message: 'must be in (0, 1]' });
  }
  if (c.emergency_threshold <= 0 || c.emergency_threshold > 1) {
    errors.push({ field: 'emergency_threshold', message: 'must be in (0, 1]' });
  }
  if (c.warn_threshold >= c.emergency_threshold) {
    errors.push({
      field: 'warn_threshold',
      message: 'warn must be strictly less than emergency',
    });
  }
  const g = doc.guardrails;
  if (g.disclaimer_triggers.length > 0 && g.disclaimer_text.trim() === '') {
    errors.push({
      field: 'disclaimer_text',
      message: 'cannot be empty when triggers are configured',
    });
  }
  return errors;
}

export default function HarnessConfigTab() {
  const queryClient = useQueryClient();
  const { data, isLoading, isError, error } = useQuery({
    queryKey: HARNESS_CONFIG_QUERY_KEY,
    queryFn: () => adminApi.getHarnessConfig(),
  });

  const [draft, setDraft] = useState<HarnessConfigDocument>(defaultDocument());
  const [savedAt, setSavedAt] = useState<string | null>(null);
  const [serverError, setServerError] = useState<string | null>(null);

  // Hydrate the form when the API responds. Avoid blowing away in-progress
  // edits if the user is mid-typing — only sync on the first successful load.
  useEffect(() => {
    if (data?.config) {
      setDraft(data.config);
      setSavedAt(data.updated_at);
    }
  }, [data]);

  const validationErrors = useMemo(() => validate(draft), [draft]);
  const isInvalid = validationErrors.length > 0;

  const mutation = useMutation({
    mutationFn: (doc: HarnessConfigDocument) => adminApi.putHarnessConfig(doc),
    onSuccess: (resp: HarnessConfigResponse) => {
      setServerError(null);
      setSavedAt(resp.updated_at);
      queryClient.setQueryData(HARNESS_CONFIG_QUERY_KEY, resp);
    },
    onError: (err: unknown) => {
      setServerError(err instanceof Error ? err.message : String(err));
    },
  });

  if (isLoading) {
    return (
      <Card className="p-12">
        <div className="flex justify-center">
          <div className="pierre-spinner" />
        </div>
      </Card>
    );
  }

  if (isError) {
    return (
      <Card className="p-6">
        <p className="text-sm text-red-600 dark:text-red-400">
          Failed to load harness config: {error instanceof Error ? error.message : String(error)}
        </p>
      </Card>
    );
  }

  return (
    <div className="space-y-4">
      <Card className="p-6">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h2 className="text-xl font-semibold text-gray-900 dark:text-on-surface">
              Coaching Harness Configuration
            </h2>
            <p className="mt-1 text-sm text-gray-500 dark:text-gray-400">
              Tunables for conversation compaction and Tier 6 text guardrails.
              Changes apply on the next coach turn.
            </p>
          </div>
          <div className="flex flex-col items-end gap-1">
            <Badge variant={data?.source === 'persisted' ? 'success' : 'secondary'}>
              {data?.source === 'persisted' ? 'persisted' : 'using defaults'}
            </Badge>
            {savedAt ? (
              <span className="text-xs text-gray-500 dark:text-gray-400">
                Saved {new Date(savedAt).toLocaleString()}
              </span>
            ) : null}
          </div>
        </div>
      </Card>

      <Card className="p-6">
        <h3 className="mb-3 text-sm font-semibold uppercase tracking-wide text-gray-500 dark:text-gray-400">
          Conversation compaction
        </h3>
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
          <NumericField
            label="Window tokens"
            help="Target context window for the active LLM."
            value={draft.compaction.window_tokens}
            min={1024}
            max={2_000_000}
            step={1024}
            onChange={(v) =>
              setDraft({
                ...draft,
                compaction: { ...draft.compaction, window_tokens: v },
              })
            }
          />
          <NumericField
            label="Warn threshold"
            help="Fraction of window that triggers summarization."
            value={draft.compaction.warn_threshold}
            min={0}
            max={1}
            step={0.05}
            onChange={(v) =>
              setDraft({
                ...draft,
                compaction: { ...draft.compaction, warn_threshold: v },
              })
            }
          />
          <NumericField
            label="Emergency threshold"
            help="Fraction of window that triggers sliding-window fallback."
            value={draft.compaction.emergency_threshold}
            min={0}
            max={1}
            step={0.01}
            onChange={(v) =>
              setDraft({
                ...draft,
                compaction: { ...draft.compaction, emergency_threshold: v },
              })
            }
          />
          <NumericField
            label="Summarize oldest N"
            help="How many oldest turns to compact when warn fires."
            value={draft.compaction.summarize_oldest_n}
            min={0}
            max={100}
            step={1}
            onChange={(v) =>
              setDraft({
                ...draft,
                compaction: { ...draft.compaction, summarize_oldest_n: v },
              })
            }
          />
          <NumericField
            label="Sliding drop N"
            help="How many oldest turns to drop under emergency."
            value={draft.compaction.sliding_drop_n}
            min={0}
            max={100}
            step={1}
            onChange={(v) =>
              setDraft({
                ...draft,
                compaction: { ...draft.compaction, sliding_drop_n: v },
              })
            }
          />
        </div>
      </Card>

      <Card className="p-6">
        <h3 className="mb-3 text-sm font-semibold uppercase tracking-wide text-gray-500 dark:text-gray-400">
          Text guardrails (Tier 6)
        </h3>
        <div className="space-y-4">
          <NumericField
            label="Max response characters"
            help="Coach replies longer than this are rewritten as a length-cap fallback. Use 0 to disable."
            value={draft.guardrails.max_response_chars}
            min={0}
            max={1_000_000}
            step={500}
            onChange={(v) =>
              setDraft({
                ...draft,
                guardrails: { ...draft.guardrails, max_response_chars: v },
              })
            }
          />
          <CsvField
            label="Blocked topics (comma separated)"
            help="Coach replies containing any of these substrings (case insensitive) are replaced with a refusal."
            value={joinCsv(draft.guardrails.blocked_topics)}
            onChange={(v) =>
              setDraft({
                ...draft,
                guardrails: { ...draft.guardrails, blocked_topics: parseCsv(v) },
              })
            }
          />
          <CsvField
            label="Disclaimer triggers (comma separated)"
            help="Coach replies containing any of these substrings get the disclaimer prepended."
            value={joinCsv(draft.guardrails.disclaimer_triggers)}
            onChange={(v) =>
              setDraft({
                ...draft,
                guardrails: { ...draft.guardrails, disclaimer_triggers: parseCsv(v) },
              })
            }
          />
          <label className="block text-sm">
            <span className="mb-1 block font-medium text-gray-700 dark:text-gray-300">
              Disclaimer text
            </span>
            <textarea
              value={draft.guardrails.disclaimer_text}
              onChange={(e) =>
                setDraft({
                  ...draft,
                  guardrails: {
                    ...draft.guardrails,
                    disclaimer_text: e.target.value,
                  },
                })
              }
              rows={4}
              className="w-full rounded border border-gray-300 bg-white px-3 py-2 text-sm dark:border-gray-600 dark:bg-gray-800 dark:text-on-surface"
            />
            <span className="mt-1 block text-xs text-gray-500 dark:text-gray-400">
              Markdown allowed. Prepended to the coach response when any trigger matches.
            </span>
          </label>
        </div>
      </Card>

      {validationErrors.length > 0 ? (
        <Card className="p-4">
          <h4 className="text-sm font-semibold text-red-600 dark:text-red-400">
            Fix these before saving
          </h4>
          <ul className="mt-2 space-y-1 text-sm text-red-600 dark:text-red-400">
            {validationErrors.map((err) => (
              <li key={`${err.field}:${err.message}`}>
                <span className="font-mono">{err.field}</span> — {err.message}
              </li>
            ))}
          </ul>
        </Card>
      ) : null}

      {serverError ? (
        <Card className="p-4">
          <p className="text-sm text-red-600 dark:text-red-400">{serverError}</p>
        </Card>
      ) : null}

      <div className="flex justify-end gap-2">
        <Button
          variant="secondary"
          onClick={() => {
            if (data?.config) setDraft(data.config);
          }}
          disabled={mutation.isPending}
        >
          Reset
        </Button>
        <Button
          variant="primary"
          onClick={() => mutation.mutate(draft)}
          disabled={isInvalid || mutation.isPending}
        >
          {mutation.isPending ? 'Saving…' : 'Save changes'}
        </Button>
      </div>
    </div>
  );
}

interface NumericFieldProps {
  label: string;
  help?: string;
  value: number;
  min?: number;
  max?: number;
  step?: number;
  onChange: (next: number) => void;
}

function NumericField({ label, help, value, min, max, step, onChange }: NumericFieldProps) {
  return (
    <label className="block text-sm">
      <span className="mb-1 block font-medium text-gray-700 dark:text-gray-300">{label}</span>
      <input
        type="number"
        value={value}
        min={min}
        max={max}
        step={step}
        onChange={(e) => onChange(Number(e.target.value))}
        className="w-full rounded border border-gray-300 bg-white px-3 py-2 text-sm dark:border-gray-600 dark:bg-gray-800 dark:text-on-surface"
      />
      {help ? (
        <span className="mt-1 block text-xs text-gray-500 dark:text-gray-400">{help}</span>
      ) : null}
    </label>
  );
}

interface CsvFieldProps {
  label: string;
  help?: string;
  value: string;
  onChange: (next: string) => void;
}

function CsvField({ label, help, value, onChange }: CsvFieldProps) {
  return (
    <label className="block text-sm">
      <span className="mb-1 block font-medium text-gray-700 dark:text-gray-300">{label}</span>
      <input
        type="text"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="w-full rounded border border-gray-300 bg-white px-3 py-2 text-sm dark:border-gray-600 dark:bg-gray-800 dark:text-on-surface"
      />
      {help ? (
        <span className="mt-1 block text-xs text-gray-500 dark:text-gray-400">{help}</span>
      ) : null}
    </label>
  );
}
