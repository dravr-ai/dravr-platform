// ABOUTME: Admin tab editing the Guardian security policy (mode, budgets, taint severity, plan mode)
// ABOUTME: A save is a compare-and-set that lists its high-impact edits first; also shows the save history and parked calls

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState, useMemo } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import {
  adminApi,
  guardianRefusalOf,
  GUARDIAN_CONFIG_IMPACT,
  GUARDIAN_CONFIG_STALE,
  type GuardianConfigDocument,
  type AuditEvent,
  type GuardianConfigResponse,
  type GuardianEffectivePolicy,
  type GuardianExternalSend,
  type GuardianFieldSource,
  type GuardianFieldSources,
  type GuardianImpact,
} from '../services/api/admin';
import { Card, Button, Badge, Input, Section, Select } from './ui';
import { describeApiError } from '@pierre/ui-logic';
import { useTranslation } from '@pierre/i18n';

const GUARDIAN_CONFIG_QUERY_KEY = ['admin', 'guardian-config'] as const;
const GUARDIAN_HISTORY_QUERY_KEY = ['admin', 'guardian-history'] as const;
const GUARDIAN_PENDING_QUERY_KEY = ['admin', 'guardian-pending'] as const;

/** How many recorded saves the page shows. */
const HISTORY_LIMIT = 10;
/** The audit-trail action of a policy save, landed or refused. */
const HISTORY_ACTION = 'guardian.config';
/** A budget is a `u16` on the server. */
const BUDGET_MAX = 65_535;

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** The document's policy fields, in display order. */
const POLICY_FIELDS = [
  'mode',
  'tainted_destructive',
  'plan_mode',
  'max_destructive_per_turn',
  'max_writes_per_turn',
  'external_send',
] as const;

/** Compile-time fallback document used before the API responds. */
function defaultDocument(): GuardianConfigDocument {
  return { schema_version: 1 };
}

interface ValidationError {
  field: string;
  message: string;
}

/** The refusal a budget earns before it reaches the server, if any. */
function budgetError(value: number, min: number): string | null {
  if (!Number.isInteger(value) || value < min || value > BUDGET_MAX) {
    return `must be a whole number from ${min} to ${BUDGET_MAX}`;
  }
  return null;
}

function validate(doc: GuardianConfigDocument, planModeArmable: boolean): ValidationError[] {
  const errors: ValidationError[] = [];
  if (doc.max_writes_per_turn === 0) {
    errors.push({
      field: 'max_writes_per_turn',
      message: 'must be >= 1 (0 would deny every write-tool dispatch)',
    });
  } else if (doc.max_writes_per_turn != null) {
    const message = budgetError(doc.max_writes_per_turn, 1);
    if (message) errors.push({ field: 'max_writes_per_turn', message });
  }
  if (doc.max_destructive_per_turn != null) {
    const message = budgetError(doc.max_destructive_per_turn, 0);
    if (message) errors.push({ field: 'max_destructive_per_turn', message });
  }
  if (Array.isArray(doc.external_send)) {
    if (doc.external_send.length === 0) {
      errors.push({
        field: 'external_send',
        message: 'name at least one tenant UUID, or choose none',
      });
    }
    for (const id of doc.external_send) {
      if (!UUID_RE.test(id)) {
        errors.push({ field: 'external_send', message: `"${id}" is not a tenant UUID` });
      }
    }
  }
  if (doc.plan_mode === 'enforce' && !planModeArmable) {
    errors.push({ field: 'plan_mode', message: PLAN_MODE_NOT_ARMABLE });
  }
  return errors;
}

const PLAN_MODE_NOT_ARMABLE =
  'enforce is not available on this deployment: GUARDIAN_PLAN_MODE_ARMABLE is not set';

function parseTenants(text: string): string[] {
  return text
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean);
}

function tenantTextOf(doc: GuardianConfigDocument): string {
  return Array.isArray(doc.external_send) ? doc.external_send.join(', ') : '';
}

/** The document a save would send: the tenant list is what the field holds now. */
function withTenantText(doc: GuardianConfigDocument, tenantText: string): GuardianConfigDocument {
  return Array.isArray(doc.external_send)
    ? { ...doc, external_send: parseTenants(tenantText) }
    : doc;
}

/** The fields two documents disagree on; unset and `null` are the same thing. */
function changedFields(a: GuardianConfigDocument, b: GuardianConfigDocument): string[] {
  return POLICY_FIELDS.filter(
    (field) => JSON.stringify(a[field] ?? null) !== JSON.stringify(b[field] ?? null),
  );
}

function sourceBadge(source: GuardianFieldSource) {
  if (source === 'env') return <Badge variant="warning">env-pinned</Badge>;
  if (source === 'database') return <Badge variant="success">persisted</Badge>;
  return <Badge variant="secondary">default</Badge>;
}

function externalSendLabel(value: GuardianExternalSend): string {
  if (value === 'none' || value === 'all') return value;
  return value.join(', ');
}

function storedCell(value: unknown): string {
  if (value === null || value === undefined) return 'default';
  if (Array.isArray(value)) return value.join(', ');
  return String(value);
}

/** What one recorded save moved, one phrase per field; a refused one says so. */
function describeSave(event: AuditEvent): string[] {
  if (event.outcome !== 'success') return [event.summary];
  const asFields = (value: unknown): Record<string, unknown> =>
    typeof value === 'object' && value !== null ? (value as Record<string, unknown>) : {};
  const before = asFields(event.metadata?.before);
  const after = asFields(event.metadata?.after);
  const phrases = POLICY_FIELDS.filter(
    (field) => JSON.stringify(before[field] ?? null) !== JSON.stringify(after[field] ?? null),
  ).map((field) => `${field}: ${storedCell(before[field])} → ${storedCell(after[field])}`);
  if (typeof event.metadata?.before === 'string') {
    phrases.unshift('The previous row was not a document');
  }
  return phrases.length > 0 ? phrases : ['No field changed'];
}

/** Who acted: a save by a service token names no account. */
function saveActor(event: AuditEvent): string {
  if (event.actor_email) return event.actor_email;
  return event.source === 'service' ? 'service token' : 'deleted account';
}

/** The edit in progress: what it started from, including the token a save echoes. */
interface EditBase {
  document: GuardianConfigDocument;
  /** Verbatim from the response: a `Date` would drop digits the server compares. */
  updatedAt: string | null;
}

export default function GuardianConfigTab() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const { data, isLoading, isError, error, refetch } = useQuery({
    queryKey: GUARDIAN_CONFIG_QUERY_KEY,
    queryFn: () => adminApi.getGuardianConfig(),
  });

  const [base, setBase] = useState<EditBase | null>(null);
  const [hydratedFrom, setHydratedFrom] = useState<GuardianConfigResponse | null>(null);
  const [draft, setDraft] = useState<GuardianConfigDocument>(defaultDocument());
  const [tenantText, setTenantText] = useState('');
  const [impacts, setImpacts] = useState<GuardianImpact[] | null>(null);
  const [serverError, setServerError] = useState<string | null>(null);
  const [staleNotice, setStaleNotice] = useState<string | null>(null);

  const resolved = useMemo(() => withTenantText(draft, tenantText), [draft, tenantText]);
  const isDirty = base !== null && changedFields(resolved, base.document).length > 0;
  const planModeArmable = data?.plan_mode_armable ?? false;

  const hydrate = (response: GuardianConfigResponse) => {
    setHydratedFrom(response);
    setBase({ document: response.config, updatedAt: response.updated_at });
    setDraft(response.config);
    setTenantText(tenantTextOf(response.config));
    setImpacts(null);
  };

  // A background refetch must not write over unsaved edits, and must not move
  // the token under them either: a save still names the version it started from,
  // so the server can refuse it rather than lose the other save.
  if (data && data !== hydratedFrom && !isDirty) {
    hydrate(data);
  }

  const validationErrors = useMemo(
    () => validate(resolved, planModeArmable),
    [resolved, planModeArmable],
  );
  const isInvalid = validationErrors.length > 0;
  const envPinned = useMemo(() => new Set(data?.env_pinned ?? []), [data]);

  const edit = (patch: Partial<GuardianConfigDocument>) => {
    setDraft({ ...draft, ...patch });
    // An acknowledgement covers the exact save it listed.
    setImpacts(null);
  };

  const editTenants = (next: string) => {
    setTenantText(next);
    setImpacts(null);
  };

  const reloadAfterStale = async (message: string) => {
    const discarded = base ? changedFields(resolved, base.document) : [];
    const fresh = await refetch();
    // A failed refetch still carries the last good `data`, the very response the
    // save was refused against, so `data` alone does not say the reload worked.
    if (fresh.isError || !fresh.data) {
      setStaleNotice(`${message} The policy could not be reloaded, so your edits are still here.`);
      return;
    }
    hydrate(fresh.data);
    setStaleNotice(
      discarded.length > 0
        ? `${message} The form now shows the stored policy. Your edits to ${discarded.join(', ')} were discarded.`
        : `${message} The form now shows the stored policy.`,
    );
    void queryClient.invalidateQueries({ queryKey: GUARDIAN_HISTORY_QUERY_KEY });
  };

  const mutation = useMutation({
    mutationFn: ({
      document,
      acknowledgeImpact,
    }: {
      document: GuardianConfigDocument;
      acknowledgeImpact: boolean;
    }) => adminApi.putGuardianConfig(document, base?.updatedAt ?? null, acknowledgeImpact),
    onSuccess: (resp: GuardianConfigResponse) => {
      setServerError(null);
      setStaleNotice(null);
      queryClient.setQueryData(GUARDIAN_CONFIG_QUERY_KEY, resp);
      hydrate(resp);
      void queryClient.invalidateQueries({ queryKey: GUARDIAN_HISTORY_QUERY_KEY });
    },
    onError: (err: unknown) => {
      setServerError(null);
      setStaleNotice(null);
      const refusal = guardianRefusalOf(err);
      if (refusal?.code === GUARDIAN_CONFIG_STALE) {
        void reloadAfterStale(refusal.message);
      } else if (refusal?.code === GUARDIAN_CONFIG_IMPACT) {
        setImpacts(refusal.impacted_fields ?? []);
      } else if (refusal) {
        setServerError(refusal.message);
      } else {
        setServerError(describeApiError(err, { t, fallbackKey: 'errors.unknown' }));
      }
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

  // A failed background refetch keeps the last good response, and the form with it.
  if (isError && !data) {
    return (
      <Card className="p-6">
        <p className="text-sm text-error">
          Failed to load guardian config: {describeApiError(error, { t, fallbackKey: 'errors.unknown' })}
        </p>
      </Card>
    );
  }

  const effective: GuardianEffectivePolicy | undefined = data?.effective;
  const defaults: GuardianEffectivePolicy | undefined = data?.defaults;
  const sources: GuardianFieldSources | undefined = data?.sources;
  const updatedElsewhere = isDirty && data !== undefined && data.updated_at !== base?.updatedAt;

  return (
    <div className="space-y-4">
      <Card className="p-6">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h2 className="text-xl font-semibold text-on-surface">
              Guardian security policy
            </h2>
            <p className="mt-1 text-sm text-on-surface-variant">
              Runtime policy for the tool-dispatch security gate: enforcement mode, per-turn
              budgets, taint severity, and the plan-then-verify posture. A save applies at once on
              the instance that took it and reaches every other instance within its settings poll
              (30 seconds unless PIERRE_SETTINGS_POLL_INTERVAL_SECS says otherwise), with no
              restart. Unset fields follow the compiled-in defaults, so they move with future
              releases.
            </p>
            {data ? (
              <p className="mt-2 text-xs text-on-surface-variant">
                Answered by instance <span className="font-mono">{data.instance_id}</span>, policy
                loaded {new Date(data.loaded_at).toLocaleString()}.
              </p>
            ) : null}
          </div>
          <div className="flex flex-col items-end gap-1">
            <Badge variant={data?.source === 'persisted' ? 'success' : 'secondary'}>
              {data?.source === 'persisted' ? 'persisted' : 'using defaults'}
            </Badge>
            {data?.in_sync === false ? <Badge variant="warning">not in sync</Badge> : null}
            {data?.degraded ? <Badge variant="error">degraded</Badge> : null}
            {base?.updatedAt ? (
              <span className="text-xs text-on-surface-variant">
                Saved {new Date(base.updatedAt).toLocaleString()}
              </span>
            ) : null}
          </div>
        </div>
        {data?.in_sync === false ? (
          <p className="mt-3 rounded bg-warning/15 px-3 py-2 text-xs text-on-warning-container">
            This instance does not yet enforce the stored policy. It catches up on its next
            settings poll, and the values marked in force below are what it enforces until then.
          </p>
        ) : null}
        {data?.degraded ? (
          <p role="alert" className="mt-3 rounded bg-error-container px-3 py-2 text-xs text-on-error-container">
            Degraded: {data.degraded_reason ?? 'the stored policy is not simply in force.'}
          </p>
        ) : null}
        {data?.stored_schema_version !== undefined ? (
          <p className="mt-3 rounded bg-warning/15 px-3 py-2 text-xs text-on-warning-container">
            The stored row is at schema version {data.stored_schema_version}. A save rewrites it at
            the current version.
          </p>
        ) : null}
        {data?.stored_unknown_fields?.length ? (
          <p className="mt-3 rounded bg-warning/15 px-3 py-2 text-xs text-on-warning-container">
            The stored row holds fields this version does not read, and a save drops them:{' '}
            {data.stored_unknown_fields.join(', ')}.
          </p>
        ) : null}
        {envPinned.size > 0 ? (
          <p className="mt-3 rounded bg-warning/15 px-3 py-2 text-xs text-on-warning-container">
            Env-pinned fields ({[...envPinned].join(', ')}) are locked by GUARDIAN_* environment
            variables. The stored values stay shadowed until the variable is unset at the next
            deploy.
          </p>
        ) : null}
      </Card>

      <Card className="p-6">
        <h3 className="mb-3 text-sm font-semibold text-on-surface-variant">
          Enforcement
        </h3>
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
          <EnumField
            label="Mode"
            help="enforce applies denials. off and observe behave the same at runtime: would-be denials are logged and nothing is blocked."
            options={['off', 'observe', 'enforce']}
            value={draft.mode ?? null}
            fallback={defaults?.mode}
            effective={effective?.mode}
            source={sources?.mode}
            disabled={envPinned.has('mode')}
            onChange={(v) => edit({ mode: v })}
          />
          <EnumField
            label="Tainted destructive"
            help="What happens when an irreversible or outbound-writing tool runs after untrusted content entered the turn: log lets it run and logs it, confirm waits for the athlete's /confirm, deny refuses it."
            options={['log', 'confirm', 'deny']}
            value={draft.tainted_destructive ?? null}
            fallback={defaults?.tainted_destructive}
            effective={effective?.tainted_destructive}
            source={sources?.tainted_destructive}
            disabled={envPinned.has('tainted_destructive')}
            onChange={(v) => edit({ tainted_destructive: v })}
          />
          <EnumField
            label="Plan mode"
            help={
              planModeArmable
                ? 'enforce replaces the interleaved tool loop with a statically verified up-front plan on every turn.'
                : `enforce replaces the interleaved tool loop with a statically verified up-front plan on every turn, and ${PLAN_MODE_NOT_ARMABLE}.`
            }
            options={['off', 'enforce']}
            disabledOptions={planModeArmable ? [] : ['enforce']}
            value={draft.plan_mode ?? null}
            fallback={defaults?.plan_mode}
            effective={effective?.plan_mode}
            source={sources?.plan_mode}
            // A row holding what the server refuses must stay fixable here, even
            // under a pin: every save would otherwise be blocked by a field locked
            // against the only edit that clears it.
            disabled={envPinned.has('plan_mode') && !(draft.plan_mode === 'enforce' && !planModeArmable)}
            onChange={(v) => edit({ plan_mode: v })}
          />
        </div>
      </Card>

      <Card className="p-6">
        <h3 className="mb-3 text-sm font-semibold text-on-surface-variant">
          Per-turn budgets
        </h3>
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
          <OptionalNumberField
            label="Max destructive per turn"
            help="Blast-radius cap on irreversible tool executions in one turn. 0 denies every one of them."
            value={draft.max_destructive_per_turn ?? null}
            fallback={defaults?.max_destructive_per_turn}
            effective={effective?.max_destructive_per_turn}
            source={sources?.max_destructive_per_turn}
            disabled={envPinned.has('max_destructive_per_turn')}
            min={0}
            onChange={(v) => edit({ max_destructive_per_turn: v })}
          />
          <OptionalNumberField
            label="Max writes per turn"
            help="Cap on data-writing tool executions in one turn (>= 1)."
            value={draft.max_writes_per_turn ?? null}
            fallback={defaults?.max_writes_per_turn}
            effective={effective?.max_writes_per_turn}
            source={sources?.max_writes_per_turn}
            disabled={envPinned.has('max_writes_per_turn')}
            min={1}
            onChange={(v) => edit({ max_writes_per_turn: v })}
          />
        </div>
      </Card>

      <Card className="p-6">
        <h3 className="mb-3 text-sm font-semibold text-on-surface-variant">
          External send allowlist
        </h3>
        <ExternalSendField
          value={draft.external_send ?? null}
          tenantText={tenantText}
          fallback={defaults ? externalSendLabel(defaults.external_send) : undefined}
          effective={effective ? externalSendLabel(effective.external_send) : undefined}
          source={sources?.external_send}
          disabled={envPinned.has('external_send')}
          onChange={(v) => {
            edit({ external_send: v });
            setTenantText('');
          }}
          onTenantTextChange={editTenants}
          onTenantTextBlur={() => setTenantText(parseTenants(tenantText).join(', '))}
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
          <p role="alert" className="text-sm text-error">{serverError}</p>
        </Card>
      ) : null}

      {staleNotice ? (
        <p role="alert" className="rounded bg-warning/15 px-3 py-2 text-sm text-on-warning-container">
          {staleNotice}
        </p>
      ) : null}

      {updatedElsewhere ? (
        <p className="rounded bg-warning/15 px-3 py-2 text-sm text-on-warning-container">
          The stored policy changed while you were editing. Your edits are still here, and saving
          them will be refused and reload the form.
        </p>
      ) : null}

      {impacts ? (
        <ImpactConfirmation
          impacts={impacts}
          pending={mutation.isPending}
          onConfirm={() => mutation.mutate({ document: resolved, acknowledgeImpact: true })}
          onCancel={() => setImpacts(null)}
        />
      ) : (
        <div className="flex justify-end gap-2">
          <Button
            variant="secondary"
            onClick={() => {
              if (data) hydrate(data);
              setServerError(null);
              setStaleNotice(null);
            }}
            disabled={mutation.isPending}
          >
            Reset
          </Button>
          <Button
            variant="primary"
            onClick={() => mutation.mutate({ document: resolved, acknowledgeImpact: false })}
            disabled={isInvalid || mutation.isPending}
          >
            {mutation.isPending ? 'Saving…' : 'Save changes'}
          </Button>
        </div>
      )}

      <div className="space-y-10 pt-6">
        <ParkedCallsSection />
        <SaveHistorySection />
      </div>
    </div>
  );
}

interface ImpactConfirmationProps {
  impacts: GuardianImpact[];
  pending: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

/** The second step of a high-impact save: the server's list, then one more click. */
function ImpactConfirmation({ impacts, pending, onConfirm, onCancel }: ImpactConfirmationProps) {
  return (
    <div role="alert" className="rounded bg-warning/15 p-4 text-on-warning-container">
      <h4 className="text-sm font-semibold">
        This save changes what athletes can do on every turn
      </h4>
      <p className="mt-1 text-sm">Nothing has been saved. Read each change before you confirm it.</p>
      <ul className="mt-3 space-y-2 text-sm">
        {impacts.map((impact) => (
          <li key={impact.field}>
            <span className="font-mono">{impact.field}</span>: {impact.from} → {impact.to}.{' '}
            {impact.effect}
          </li>
        ))}
      </ul>
      <div className="mt-4 flex justify-end gap-2">
        <Button variant="secondary" onClick={onCancel} disabled={pending}>
          Keep editing
        </Button>
        <Button variant="danger" onClick={onConfirm} disabled={pending}>
          {pending ? 'Saving…' : 'Confirm and save'}
        </Button>
      </div>
    </div>
  );
}

const TABLE_HEADING = 'px-4 py-2 text-left text-xs font-medium text-on-surface-variant';
const TABLE_CELL = 'px-4 py-2 text-sm text-on-surface';

/** The calls waiting for an athlete's /confirm. A super admin can only deny one. */
function ParkedCallsSection() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const parked = useQuery({
    queryKey: GUARDIAN_PENDING_QUERY_KEY,
    queryFn: () => adminApi.listGuardianPendingActions(),
  });
  const [confirming, setConfirming] = useState<string | null>(null);
  const [denyError, setDenyError] = useState<string | null>(null);

  const deny = useMutation({
    mutationFn: (id: string) => adminApi.denyGuardianPendingAction(id),
    onSuccess: () => {
      setDenyError(null);
      setConfirming(null);
    },
    onError: (err: unknown) => {
      setConfirming(null);
      setDenyError(describeApiError(err, { t, fallbackKey: 'errors.unknown' }));
    },
    // A failed denial usually means the athlete answered first, so reread either way.
    onSettled: () => queryClient.invalidateQueries({ queryKey: GUARDIAN_PENDING_QUERY_KEY }),
  });

  const entries = parked.data?.entries ?? [];

  return (
    <Section
      title="Parked calls"
      description="Tool calls waiting for an athlete's /confirm, which appear only under tainted destructive confirm. You can deny one; only its athlete can approve it. Their arguments are never shown."
      headingLevel={3}
    >
      {denyError ? <p role="alert" className="mb-2 text-sm text-error">{denyError}</p> : null}
      {parked.isPending ? <div className="pierre-spinner h-5 w-5" role="status" aria-label="Loading parked calls" /> : null}
      {parked.isError ? (
        <p role="alert" className="text-sm text-error">
          Failed to load parked calls: {describeApiError(parked.error, { t, fallbackKey: 'errors.unknown' })}
        </p>
      ) : null}
      {parked.isSuccess && entries.length === 0 ? (
        <p className="text-sm text-on-surface-variant">No calls are waiting.</p>
      ) : null}
      {entries.length > 0 ? (
        <div className="overflow-x-auto">
          <table className="min-w-full divide-y divide-outline-variant">
            <thead className="bg-surface-container">
              <tr>
                <th className={TABLE_HEADING}>Tool</th>
                <th className={TABLE_HEADING}>Tenant</th>
                <th className={TABLE_HEADING}>Athlete</th>
                <th className={TABLE_HEADING}>Reason</th>
                <th className={TABLE_HEADING}>Expires</th>
                <th className={`${TABLE_HEADING} text-right`}>Action</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-outline-variant">
              {entries.map((entry) => (
                <tr key={entry.id}>
                  <td className={`${TABLE_CELL} font-mono`}>{entry.tool_name}</td>
                  <td className={`${TABLE_CELL} font-mono`}>{entry.tenant_id}</td>
                  <td className={`${TABLE_CELL} font-mono`}>{entry.user_id}</td>
                  <td className={TABLE_CELL}>{entry.deny_reason}</td>
                  <td className={TABLE_CELL}>{new Date(entry.expires_at).toLocaleString()}</td>
                  <td className={`${TABLE_CELL} text-right`}>
                    {confirming === entry.id ? (
                      <div className="flex justify-end gap-2">
                        <Button variant="secondary" size="sm" onClick={() => setConfirming(null)} disabled={deny.isPending}>
                          Keep
                        </Button>
                        <Button variant="danger" size="sm" onClick={() => deny.mutate(entry.id)} disabled={deny.isPending}>
                          Confirm deny
                        </Button>
                      </div>
                    ) : (
                      <Button variant="secondary" size="sm" onClick={() => setConfirming(entry.id)} disabled={deny.isPending}>
                        Deny
                      </Button>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : null}
    </Section>
  );
}

/** Who changed the policy, when, and what moved. */
function SaveHistorySection() {
  const { t } = useTranslation();
  const history = useQuery({
    queryKey: GUARDIAN_HISTORY_QUERY_KEY,
    queryFn: () => adminApi.getAuditEvents({ action: HISTORY_ACTION, limit: HISTORY_LIMIT }),
  });
  const events = history.data?.events ?? [];

  return (
    <Section
      title="Change history"
      description={`The last ${HISTORY_LIMIT} recorded saves of this policy, refused ones included, newest first.`}
      headingLevel={3}
    >
      {history.isPending ? <div className="pierre-spinner h-5 w-5" role="status" aria-label="Loading change history" /> : null}
      {history.isError ? (
        <p role="alert" className="text-sm text-error">
          Failed to load change history: {describeApiError(history.error, { t, fallbackKey: 'errors.unknown' })}
        </p>
      ) : null}
      {history.isSuccess && events.length === 0 ? (
        <p className="text-sm text-on-surface-variant">No recorded saves yet.</p>
      ) : null}
      {events.length > 0 ? (
        <div className="overflow-x-auto">
          <table className="min-w-full divide-y divide-outline-variant">
            <thead className="bg-surface-container">
              <tr>
                <th className={TABLE_HEADING}>When</th>
                <th className={TABLE_HEADING}>Actor</th>
                <th className={TABLE_HEADING}>Change</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-outline-variant">
              {events.map((event) => (
                <tr key={event.id}>
                  <td className={TABLE_CELL}>{new Date(event.occurred_at).toLocaleString()}</td>
                  <td className={TABLE_CELL}>{saveActor(event)}</td>
                  <td className={TABLE_CELL}>
                    {describeSave(event).map((phrase) => (
                      <div key={phrase}>{phrase}</div>
                    ))}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : null}
    </Section>
  );
}

interface EnumFieldProps<T extends string> {
  label: string;
  help: string;
  options: readonly T[];
  disabledOptions?: readonly T[];
  value: T | null;
  /** The compiled-in default this field follows when unset. */
  fallback?: T;
  /** What the instance that answered enforces right now. */
  effective?: T;
  source?: GuardianFieldSource;
  disabled: boolean;
  onChange: (next: T | null) => void;
}

function EnumField<T extends string>({
  label,
  help,
  options,
  disabledOptions = [],
  value,
  fallback,
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
          { value: '', label: `Follow default${fallback ? ` (${fallback})` : ''}` },
          ...options.map((opt) => ({
            value: String(opt),
            label: String(opt),
            disabled: disabledOptions.includes(opt),
          })),
        ]}
        helpText={help}
      />
      {effective ? <InForce value={effective} /> : null}
    </label>
  );
}

function InForce({ value }: { value: string | number }) {
  return (
    <span className="mt-1 block text-xs text-on-surface-variant">
      In force on this instance: {value}
    </span>
  );
}

interface OptionalNumberFieldProps {
  label: string;
  help: string;
  value: number | null;
  /** The compiled-in default this field follows when unset. */
  fallback?: number;
  /** What the instance that answered enforces right now. */
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
  fallback,
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
        value={value === null || Number.isNaN(value) ? '' : value}
        placeholder={fallback != null ? `default: ${fallback}` : ''}
        min={min}
        max={BUDGET_MAX}
        step={1}
        disabled={disabled}
        onChange={(e) => {
          // The browser reports text it cannot read as a number as an empty value,
          // which must not read as "follow the default".
          if (e.target.validity?.badInput) onChange(Number.NaN);
          else onChange(e.target.value === '' ? null : Number(e.target.value));
        }}
        helpText={`${help} Leave empty to follow the default.`}
      />
      {effective != null ? <InForce value={effective} /> : null}
    </div>
  );
}

interface ExternalSendFieldProps {
  value: GuardianExternalSend | null;
  /** The list as typed: parsed on blur and on save, so a comma survives typing. */
  tenantText: string;
  /** The compiled-in default this field follows when unset. */
  fallback?: string;
  /** What the instance that answered enforces right now. */
  effective?: string;
  source?: GuardianFieldSource;
  disabled: boolean;
  onChange: (next: GuardianExternalSend | null) => void;
  onTenantTextChange: (next: string) => void;
  onTenantTextBlur: () => void;
}

function ExternalSendField({
  value,
  tenantText,
  fallback,
  effective,
  source,
  disabled,
  onChange,
  onTenantTextChange,
  onTenantTextBlur,
}: ExternalSendFieldProps) {
  const kind = value === null ? '' : Array.isArray(value) ? 'tenants' : value;
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
            { value: '', label: `Follow default${fallback ? ` (${fallback})` : ''}` },
            { value: 'none', label: 'none — no tenant' },
            { value: 'all', label: 'all — every tenant' },
            { value: 'tenants', label: 'specific tenants…' },
          ]}
        />
        {effective ? <InForce value={effective} /> : null}
      </label>
      {kind === 'tenants' ? (
        <div className="block text-sm">
          <Input
            label="Tenant UUIDs (comma separated)"
            type="text"
            value={tenantText}
            disabled={disabled}
            onChange={(e) => onTenantTextChange(e.target.value)}
            onBlur={onTenantTextBlur}
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
