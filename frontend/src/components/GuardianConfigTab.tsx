// ABOUTME: Admin tab editing the Guardian security policy (mode, budgets, taint severity, plan mode)
// ABOUTME: Persisted via /api/admin/settings/guardian; env-pinned fields render read-only with a source badge
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState, useEffect, useMemo } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import {
  adminApi,
  type GuardianConfigDocument,
  type GuardianConfigResponse,
  type GuardianEffectivePolicy,
  type GuardianExternalSend,
  type GuardianFieldSource,
  type GuardianFieldSources,
} from '../services/api/admin';
import { Card, Button, Badge, Input, Select } from './ui';

const GUARDIAN_CONFIG_QUERY_KEY = ['admin', 'guardian-config'] as const;

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** Compile-time fallback document used before the API responds. */
function defaultDocument(): GuardianConfigDocument {
  return { schema_version: 1 };
}

interface ValidationError {
  field: string;
  message: string;
}

function validate(doc: GuardianConfigDocument): ValidationError[] {
  const errors: ValidationError[] = [];
  if (doc.max_writes_per_turn != null && doc.max_writes_per_turn < 1) {
    errors.push({
      field: 'max_writes_per_turn',
      message: 'must be >= 1 (0 would deny every write-tool dispatch)',
    });
  }
  if (doc.max_destructive_per_turn != null && doc.max_destructive_per_turn < 0) {
    errors.push({ field: 'max_destructive_per_turn', message: 'must be >= 0' });
  }
  if (Array.isArray(doc.external_send)) {
    for (const id of doc.external_send) {
      if (!UUID_RE.test(id)) {
        errors.push({ field: 'external_send', message: `"${id}" is not a tenant UUID` });
      }
    }
  }
  return errors;
}

function sourceBadge(source: GuardianFieldSource) {
  if (source === 'env') return <Badge variant="warning">env-pinned</Badge>;
  if (source === 'database') return <Badge variant="success">persisted</Badge>;
  return <Badge variant="secondary">default</Badge>;
}

function externalSendLabel(value: GuardianExternalSend): string {
  if (value === 'none' || value === 'all') return value;
  return `${value.length} tenant(s)`;
}

export default function GuardianConfigTab() {
  const queryClient = useQueryClient();
  const { data, isLoading, isError, error } = useQuery({
    queryKey: GUARDIAN_CONFIG_QUERY_KEY,
    queryFn: () => adminApi.getGuardianConfig(),
  });

  const [draft, setDraft] = useState<GuardianConfigDocument>(defaultDocument());
  const [savedAt, setSavedAt] = useState<string | null>(null);
  const [serverError, setServerError] = useState<string | null>(null);

  useEffect(() => {
    if (data?.config) {
      setDraft(data.config);
      setSavedAt(data.updated_at);
    }
  }, [data]);

  const validationErrors = useMemo(() => validate(draft), [draft]);
  const isInvalid = validationErrors.length > 0;
  const envPinned = useMemo(() => new Set(data?.env_pinned ?? []), [data]);

  const mutation = useMutation({
    mutationFn: (doc: GuardianConfigDocument) => adminApi.putGuardianConfig(doc),
    onSuccess: (resp: GuardianConfigResponse) => {
      setServerError(null);
      setSavedAt(resp.updated_at);
      setDraft(resp.config);
      queryClient.setQueryData(GUARDIAN_CONFIG_QUERY_KEY, resp);
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
        <p className="text-sm text-error">
          Failed to load guardian config: {error instanceof Error ? error.message : String(error)}
        </p>
      </Card>
    );
  }

  const effective: GuardianEffectivePolicy | undefined = data?.effective;
  const sources: GuardianFieldSources | undefined = data?.sources;

  return (
    <div className="space-y-4">
      <Card className="p-6">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h2 className="text-xl font-semibold text-on-surface">
              Guardian Security Policy
            </h2>
            <p className="mt-1 text-sm text-on-surface-variant">
              Runtime policy for the tool-dispatch security gate: enforcement mode, per-turn
              budgets, taint severity, and the plan-then-verify posture. Saved changes apply on
              the next tool dispatch — no restart. Unset fields follow the compiled-in defaults,
              so they harden automatically with future releases.
            </p>
          </div>
          <div className="flex flex-col items-end gap-1">
            <Badge variant={data?.source === 'persisted' ? 'success' : 'secondary'}>
              {data?.source === 'persisted' ? 'persisted' : 'using defaults'}
            </Badge>
            {savedAt ? (
              <span className="text-xs text-on-surface-variant">
                Saved {new Date(savedAt).toLocaleString()}
              </span>
            ) : null}
          </div>
        </div>
        {envPinned.size > 0 ? (
          <p className="mt-3 rounded bg-warning px-3 py-2 text-xs text-warning/30">
            Env-pinned fields ({[...envPinned].join(', ')}) are locked by GUARDIAN_* environment
            variables. Edits to them persist but stay shadowed until the variable is unset at the
            next deploy.
          </p>
        ) : null}
      </Card>

      <Card className="p-6">
        <h3 className="mb-3 text-sm font-semibold uppercase tracking-wide text-on-surface-variant">
          Enforcement
        </h3>
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
          <EnumField
            label="Mode"
            help="off computes decisions for debugging only; observe logs would-be denials without blocking; enforce applies them."
            options={['off', 'observe', 'enforce']}
            value={draft.mode ?? null}
            effective={effective?.mode}
            source={sources?.mode}
            disabled={envPinned.has('mode')}
            onChange={(v) => setDraft({ ...draft, mode: v })}
          />
          <EnumField
            label="Tainted destructive"
            help="What happens when a destructive tool runs after untrusted content entered the turn: log, ask the user to confirm, or deny."
            options={['log', 'confirm', 'deny']}
            value={draft.tainted_destructive ?? null}
            effective={effective?.tainted_destructive}
            source={sources?.tainted_destructive}
            disabled={envPinned.has('tainted_destructive')}
            onChange={(v) => setDraft({ ...draft, tainted_destructive: v })}
          />
          <EnumField
            label="Plan mode"
            help="enforce replaces the interleaved ReAct loop with a statically verified up-front plan for every provider."
            options={['off', 'enforce']}
            value={draft.plan_mode ?? null}
            effective={effective?.plan_mode}
            source={sources?.plan_mode}
            disabled={envPinned.has('plan_mode')}
            onChange={(v) => setDraft({ ...draft, plan_mode: v })}
          />
        </div>
      </Card>

      <Card className="p-6">
        <h3 className="mb-3 text-sm font-semibold uppercase tracking-wide text-on-surface-variant">
          Per-turn budgets
        </h3>
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
          <OptionalNumberField
            label="Max destructive per turn"
            help="Blast-radius cap on IRREVERSIBLE tool executions in one turn."
            value={draft.max_destructive_per_turn ?? null}
            effective={effective?.max_destructive_per_turn}
            source={sources?.max_destructive_per_turn}
            disabled={envPinned.has('max_destructive_per_turn')}
            min={0}
            onChange={(v) => setDraft({ ...draft, max_destructive_per_turn: v })}
          />
          <OptionalNumberField
            label="Max writes per turn"
            help="Cap on WRITES_DATA tool executions in one turn (>= 1)."
            value={draft.max_writes_per_turn ?? null}
            effective={effective?.max_writes_per_turn}
            source={sources?.max_writes_per_turn}
            disabled={envPinned.has('max_writes_per_turn')}
            min={1}
            onChange={(v) => setDraft({ ...draft, max_writes_per_turn: v })}
          />
        </div>
      </Card>

      <Card className="p-6">
        <h3 className="mb-3 text-sm font-semibold uppercase tracking-wide text-on-surface-variant">
          External send allowlist
        </h3>
        <ExternalSendField
          value={draft.external_send ?? null}
          effective={effective ? externalSendLabel(effective.external_send) : undefined}
          source={sources?.external_send}
          disabled={envPinned.has('external_send')}
          onChange={(v) => setDraft({ ...draft, external_send: v })}
        />
      </Card>

      {validationErrors.length > 0 ? (
        <Card className="p-4">
          <h4 className="text-sm font-semibold text-error">
            Fix these before saving
          </h4>
          <ul className="mt-2 space-y-1 text-sm text-error">
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
          <p className="text-sm text-error">{serverError}</p>
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

interface EnumFieldProps<T extends string> {
  label: string;
  help: string;
  options: readonly T[];
  value: T | null;
  effective?: T;
  source?: GuardianFieldSource;
  disabled: boolean;
  onChange: (next: T | null) => void;
}

function EnumField<T extends string>({
  label,
  help,
  options,
  value,
  effective,
  source,
  disabled,
  onChange,
}: EnumFieldProps<T>) {
  return (
    <label className="block text-sm">
      <span className="mb-1 flex items-center gap-2 font-medium text-on-surface-variant">
        {label}
        {source ? sourceBadge(source) : null}
      </span>
      <Select
        value={value ?? ''}
        disabled={disabled}
        onChange={(e) => onChange(e.target.value === '' ? null : (e.target.value as T))}
        options={[
          { value: '', label: `Follow default${effective ? ` (currently: ${effective})` : ''}` },
          ...options.map((opt) => ({ value: String(opt), label: String(opt) })),
        ]}
        helpText={help}
      />
    </label>
  );
}

interface OptionalNumberFieldProps {
  label: string;
  help: string;
  value: number | null;
  effective?: number;
  source?: GuardianFieldSource;
  disabled: boolean;
  min: number;
  onChange: (next: number | null) => void;
}

function OptionalNumberField({
  label,
  help,
  value,
  effective,
  source,
  disabled,
  min,
  onChange,
}: OptionalNumberFieldProps) {
  return (
    <div className="block text-sm">
      <span className="mb-1 flex items-center gap-2 font-medium text-on-surface-variant">
        {source ? sourceBadge(source) : null}
      </span>
      <Input
        label={label}
        type="number"
        value={value ?? ''}
        placeholder={effective != null ? `default: ${effective}` : ''}
        min={min}
        max={65_535}
        step={1}
        disabled={disabled}
        onChange={(e) => onChange(e.target.value === '' ? null : Number(e.target.value))}
        helpText={`${help} Leave empty to follow the default.`}
      />
    </div>
  );
}

interface ExternalSendFieldProps {
  value: GuardianExternalSend | null;
  effective?: string;
  source?: GuardianFieldSource;
  disabled: boolean;
  onChange: (next: GuardianExternalSend | null) => void;
}

function ExternalSendField({ value, effective, source, disabled, onChange }: ExternalSendFieldProps) {
  const kind = value === null ? '' : Array.isArray(value) ? 'tenants' : value;
  const tenants = Array.isArray(value) ? value.join(', ') : '';
  return (
    <div className="space-y-2">
      <label className="block text-sm">
        <span className="mb-1 flex items-center gap-2 font-medium text-on-surface-variant">
          Which tenants may reach external-send tools
          {source ? sourceBadge(source) : null}
        </span>
        <Select
          value={kind}
          disabled={disabled}
          onChange={(e) => {
            const next = e.target.value;
            if (next === '') onChange(null);
            else if (next === 'tenants') onChange([]);
            else onChange(next as GuardianExternalSend);
          }}
          options={[
            { value: '', label: `Follow default${effective ? ` (currently: ${effective})` : ''}` },
            { value: 'none', label: 'none — no tenant' },
            { value: 'all', label: 'all — every tenant' },
            { value: 'tenants', label: 'specific tenants…' },
          ]}
        />
      </label>
      {kind === 'tenants' ? (
        <div className="block text-sm">
          <Input
            label="Tenant UUIDs (comma separated)"
            type="text"
            value={tenants}
            disabled={disabled}
            onChange={(e) =>
              onChange(
                e.target.value
                  .split(',')
                  .map((s) => s.trim())
                  .filter(Boolean),
              )
            }
          />
        </div>
      ) : null}
      <p className="text-xs text-on-surface-variant">
        Taint-independent egress gate. No external-send tools ship yet, so this is 0-impact
        today — it arms the boundary for when they do.
      </p>
    </div>
  );
}
