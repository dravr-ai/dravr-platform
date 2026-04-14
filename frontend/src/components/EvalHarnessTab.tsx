// ABOUTME: Admin eval harness tab — pierre-evals fixture browser and Tier 5.5 verdict calibration
// ABOUTME: Read-only view of static fixtures and live verdict drift for Phase B/D tenant admins
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect, useMemo, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  adminApi,
  type EvalFixtureBrowserResponse,
  type EvalFixtureSummary,
  type VerdictCalibrationStats,
  type VerdictStatusBreakdown,
} from '../services/api/admin';
import { useAuth } from '../hooks/useAuth';
import { Card, Button, Badge, ConfirmDialog } from './ui';

function totalAssertions(fixture: EvalFixtureSummary): number {
  return fixture.cases.reduce(
    (acc, c) => acc + c.must_contain_total + c.must_not_contain_total,
    0,
  );
}

/**
 * Verdict pass rate = supported / (supported + unsupported + contradicted).
 * Rhetorical and unverifiable verdicts are excluded because they do not
 * map onto the pass/fail axis — rhetorical claims never reached the
 * evidence layer, and unverifiable claims genuinely lack a ground truth.
 * Returns `null` when the denominator is zero so callers can render a
 * dash instead of `NaN%`.
 */
function verdictPassRate(totals: VerdictStatusBreakdown): number | null {
  const denom = totals.supported + totals.unsupported + totals.contradicted;
  if (denom === 0) return null;
  return totals.supported / denom;
}

function verdictTotalCount(totals: VerdictStatusBreakdown): number {
  return (
    totals.supported +
    totals.unsupported +
    totals.contradicted +
    totals.rhetorical +
    totals.unverifiable
  );
}

export default function EvalHarnessTab() {
  const [expandedFixture, setExpandedFixture] = useState<string | null>(null);
  const [editorState, setEditorState] = useState<FixtureEditorState | null>(null);
  const [deleteConfirmation, setDeleteConfirmation] = useState<string | null>(null);
  const { user } = useAuth();
  const tenantId = user?.tenant_id ?? '';
  const queryClient = useQueryClient();

  const { data, isLoading, isError, error, refetch, isFetching } =
    useQuery<EvalFixtureBrowserResponse>({
      queryKey: ['admin', 'evals', 'fixtures'] as const,
      queryFn: () => adminApi.getEvalFixtureBrowser(),
    });

  const deleteMutation = useMutation({
    mutationFn: (name: string) => adminApi.deleteEvalFixture(name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['admin', 'evals', 'fixtures'] });
      setDeleteConfirmation(null);
    },
  });

  const { data: calibration, isLoading: calibrationLoading, isError: calibrationError } =
    useQuery<VerdictCalibrationStats>({
      queryKey: ['admin', 'evals', 'verdict-stats', tenantId] as const,
      queryFn: () => adminApi.getVerdictCalibrationStats(tenantId, 30),
      enabled: Boolean(tenantId),
    });

  const personaCounts = useMemo(() => {
    if (!data) return new Map<string, number>();
    const counts = new Map<string, number>();
    for (const fixture of data.fixtures) {
      for (const c of fixture.cases) {
        counts.set(c.persona, (counts.get(c.persona) ?? 0) + 1);
      }
    }
    return counts;
  }, [data]);

  return (
    <div className="space-y-4">
      <Card className="p-6">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h2 className="text-xl font-semibold text-gray-900 dark:text-on-surface">
              Eval harness — golden fixtures
            </h2>
            <p className="mt-1 text-sm text-gray-500 dark:text-gray-400">
              Read-only browser over the <code className="rounded bg-gray-100 px-1 dark:bg-gray-800">pierre-evals</code>{' '}
              golden fixture set used by the Tier 5 evaluation harness.
              Live evaluation runs land in a follow-up sprint; for now
              this tab shows which scenarios ship with the release and
              what each case asserts.
            </p>
          </div>
          <div className="flex items-center gap-2">
            <Button
              onClick={() =>
                setEditorState({ mode: 'create', name: '', body: '', error: null })
              }
              variant="primary"
            >
              New fixture
            </Button>
            <Button onClick={() => refetch()} variant="secondary" disabled={isFetching}>
              {isFetching ? 'Refreshing…' : 'Refresh'}
            </Button>
          </div>
        </div>

        <div className="mt-6 grid grid-cols-2 gap-3 sm:grid-cols-4">
          <div className="rounded-lg border border-gray-200 bg-gray-50 p-3 dark:border-gray-700 dark:bg-gray-800">
            <div className="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">
              Fixture files
            </div>
            <div className="mt-1 text-2xl font-semibold text-gray-900 dark:text-on-surface">
              {data?.fixture_count ?? 0}
            </div>
          </div>
          <div className="rounded-lg border border-gray-200 bg-gray-50 p-3 dark:border-gray-700 dark:bg-gray-800">
            <div className="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">
              Total cases
            </div>
            <div className="mt-1 text-2xl font-semibold text-gray-900 dark:text-on-surface">
              {data?.case_total ?? 0}
            </div>
          </div>
          <div className="rounded-lg border border-gray-200 bg-gray-50 p-3 dark:border-gray-700 dark:bg-gray-800">
            <div className="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">
              Distinct personas
            </div>
            <div className="mt-1 text-2xl font-semibold text-gray-900 dark:text-on-surface">
              {personaCounts.size}
            </div>
          </div>
          <div className="rounded-lg border border-gray-200 bg-gray-50 p-3 dark:border-gray-700 dark:bg-gray-800">
            <div className="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">
              Assertions
            </div>
            <div className="mt-1 text-2xl font-semibold text-gray-900 dark:text-on-surface">
              {data?.fixtures.reduce((acc, f) => acc + totalAssertions(f), 0) ?? 0}
            </div>
          </div>
        </div>

        {data && (
          <p className="mt-4 text-xs text-gray-400 dark:text-gray-500">
            Scanned directory:{' '}
            <code className="rounded bg-gray-100 px-1 dark:bg-gray-800">
              {data.scanned_dir}
            </code>
          </p>
        )}
      </Card>

      <CalibrationPanel
        stats={calibration}
        isLoading={calibrationLoading}
        isError={calibrationError}
        hasTenant={Boolean(tenantId)}
      />

      {isLoading ? (
        <Card className="p-12">
          <div className="flex justify-center">
            <div className="pierre-spinner" />
          </div>
        </Card>
      ) : isError ? (
        <Card className="p-6">
          <p className="text-sm text-red-600 dark:text-red-400">
            Failed to load eval fixtures:{' '}
            {error instanceof Error ? error.message : String(error)}
          </p>
          <p className="mt-2 text-xs text-gray-500 dark:text-gray-400">
            Make sure the server was started with <code>tools-verification</code>{' '}
            enabled and <code>PIERRE_EVALS_FIXTURES_DIR</code> points at a
            valid directory.
          </p>
        </Card>
      ) : !data || data.fixtures.length === 0 ? (
        <Card className="p-12 text-center">
          <p className="text-gray-500 dark:text-gray-400">
            No fixture files found in the scanned directory.
          </p>
          <p className="mt-1 text-xs text-gray-400 dark:text-gray-500">
            Drop a <code>.jsonl</code> file into{' '}
            <code className="rounded bg-gray-100 px-1 dark:bg-gray-800">
              crates/pierre-evals/fixtures/
            </code>{' '}
            or point <code>PIERRE_EVALS_FIXTURES_DIR</code> elsewhere.
          </p>
        </Card>
      ) : (
        data.fixtures.map((fixture) => {
          const isExpanded = expandedFixture === fixture.name;
          return (
            <Card key={fixture.name} className="overflow-hidden">
              <button
                type="button"
                onClick={() =>
                  setExpandedFixture(isExpanded ? null : fixture.name)
                }
                className="flex w-full items-center justify-between gap-4 border-b border-gray-200 px-6 py-3 text-left hover:bg-gray-50 dark:border-gray-700 dark:hover:bg-gray-800"
                aria-expanded={isExpanded}
                aria-label={`Toggle ${fixture.name} fixture`}
              >
                <div>
                  <h3 className="text-sm font-semibold text-gray-900 dark:text-on-surface">
                    {fixture.name}
                  </h3>
                  <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
                    {fixture.case_count} case
                    {fixture.case_count === 1 ? '' : 's'} ·{' '}
                    {totalAssertions(fixture)} assertion
                    {totalAssertions(fixture) === 1 ? '' : 's'}
                  </p>
                </div>
                <div className="flex items-center gap-2">
                  {fixture.personas.map((p) => (
                    <Badge key={p} variant="info">
                      {p}
                    </Badge>
                  ))}
                </div>
              </button>
              {isExpanded && (
                <>
                  <div className="flex items-center justify-end gap-2 border-b border-gray-200 px-6 py-2 dark:border-gray-700">
                    <Button
                      variant="secondary"
                      onClick={async () => {
                        try {
                          const content = await adminApi.getEvalFixtureContent(fixture.name);
                          setEditorState({
                            mode: 'edit',
                            name: content.name,
                            body: content.body,
                            error: null,
                          });
                        } catch (e) {
                          setEditorState({
                            mode: 'edit',
                            name: fixture.name,
                            body: '',
                            error: e instanceof Error ? e.message : String(e),
                          });
                        }
                      }}
                    >
                      Edit
                    </Button>
                    <Button
                      variant="danger"
                      onClick={() => setDeleteConfirmation(fixture.name)}
                    >
                      Delete
                    </Button>
                  </div>
                  <div className="divide-y divide-gray-200 dark:divide-gray-700">
                    {fixture.cases.map((c) => (
                      <div
                        key={c.id}
                        className="flex items-center justify-between gap-4 px-6 py-3"
                      >
                        <div>
                          <p className="text-sm font-medium text-gray-900 dark:text-gray-100">
                            {c.label}
                          </p>
                          <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
                            <span className="font-mono">{c.id}</span> · persona{' '}
                            <span className="font-mono">{c.persona}</span>
                          </p>
                        </div>
                        <div className="flex items-center gap-3 text-right text-xs">
                          <div>
                            <div className="text-gray-500 dark:text-gray-400">Turns</div>
                            <div className="font-semibold text-gray-900 dark:text-on-surface">
                              {c.turn_count}
                            </div>
                          </div>
                          <div>
                            <div className="text-gray-500 dark:text-gray-400">Must</div>
                            <div className="font-semibold text-green-600 dark:text-green-400">
                              {c.must_contain_total}
                            </div>
                          </div>
                          <div>
                            <div className="text-gray-500 dark:text-gray-400">Must not</div>
                            <div className="font-semibold text-red-600 dark:text-red-400">
                              {c.must_not_contain_total}
                            </div>
                          </div>
                        </div>
                      </div>
                    ))}
                  </div>
                </>
              )}
            </Card>
          );
        })
      )}

      {editorState && (
        <FixtureEditorModal
          state={editorState}
          onClose={() => setEditorState(null)}
          onSaved={() => {
            setEditorState(null);
            queryClient.invalidateQueries({ queryKey: ['admin', 'evals', 'fixtures'] });
          }}
          onStateChange={setEditorState}
        />
      )}

      <ConfirmDialog
        isOpen={deleteConfirmation !== null}
        title="Delete eval fixture"
        message={`Delete fixture "${deleteConfirmation}"? This removes the JSONL file from the fixtures directory and cannot be undone.`}
        confirmLabel={deleteMutation.isPending ? 'Deleting…' : 'Delete'}
        cancelLabel="Cancel"
        variant="danger"
        onConfirm={() => {
          if (deleteConfirmation) {
            deleteMutation.mutate(deleteConfirmation);
          }
        }}
        onClose={() => setDeleteConfirmation(null)}
      />
    </div>
  );
}

/** Editor dialog state for the inline fixture CRUD (Sprint C21). */
interface FixtureEditorState {
  mode: 'create' | 'edit';
  name: string;
  body: string;
  error: string | null;
}

interface FixtureEditorModalProps {
  state: FixtureEditorState;
  onClose: () => void;
  onSaved: () => void;
  onStateChange: (next: FixtureEditorState) => void;
}

function FixtureEditorModal({
  state,
  onClose,
  onSaved,
  onStateChange,
}: FixtureEditorModalProps) {
  const [isSaving, setIsSaving] = useState(false);

  // Allow Escape to close when the user isn't mid-save.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !isSaving) onClose();
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [isSaving, onClose]);

  const handleSave = async (): Promise<void> => {
    if (!state.name.trim()) {
      onStateChange({ ...state, error: 'Name is required' });
      return;
    }
    setIsSaving(true);
    try {
      await adminApi.putEvalFixture(state.name.trim(), state.body);
      onSaved();
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      onStateChange({ ...state, error: message });
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
      data-testid="fixture-editor-modal"
    >
      <div className="w-full max-w-2xl overflow-hidden rounded-lg bg-white shadow-xl dark:bg-gray-900">
        <div className="border-b border-gray-200 px-6 py-4 dark:border-gray-700">
          <h3 className="text-lg font-semibold text-gray-900 dark:text-white">
            {state.mode === 'create' ? 'New fixture' : `Edit ${state.name}`}
          </h3>
          <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
            One GoldenCase JSON object per line. Lines starting with{' '}
            <code>#</code> are comments.
          </p>
        </div>
        <div className="space-y-4 px-6 py-4">
          {state.mode === 'create' && (
            <div>
              <label
                htmlFor="fixture-editor-name"
                className="block text-xs font-medium text-gray-700 dark:text-gray-300"
              >
                Name
              </label>
              <input
                id="fixture-editor-name"
                type="text"
                value={state.name}
                onChange={(e) =>
                  onStateChange({ ...state, name: e.target.value, error: null })
                }
                className="mt-1 w-full rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-800 dark:text-white"
                placeholder="injury_triage"
              />
            </div>
          )}
          <div>
            <label
              htmlFor="fixture-editor-body"
              className="block text-xs font-medium text-gray-700 dark:text-gray-300"
            >
              JSONL body
            </label>
            <textarea
              id="fixture-editor-body"
              value={state.body}
              onChange={(e) =>
                onStateChange({ ...state, body: e.target.value, error: null })
              }
              rows={14}
              className="mt-1 w-full rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-800 dark:text-white"
              data-testid="fixture-editor-body"
              spellCheck={false}
            />
          </div>
          {state.error && (
            <p className="text-sm text-red-600 dark:text-red-400">{state.error}</p>
          )}
        </div>
        <div className="flex items-center justify-end gap-2 border-t border-gray-200 px-6 py-3 dark:border-gray-700">
          <Button variant="secondary" onClick={onClose} disabled={isSaving}>
            Cancel
          </Button>
          <Button variant="primary" onClick={handleSave} disabled={isSaving}>
            {isSaving ? 'Saving…' : 'Save fixture'}
          </Button>
        </div>
      </div>
    </div>
  );
}

interface CalibrationPanelProps {
  stats: VerdictCalibrationStats | undefined;
  isLoading: boolean;
  isError: boolean;
  hasTenant: boolean;
}

/**
 * Verdict calibration panel — surfaces pass rate, status mix, and daily
 * drift for the Tier 5.5 claim verification pipeline. Sources its data
 * from `GET /admin/evals/verdict-stats` over a 30-day window.
 */
function CalibrationPanel({ stats, isLoading, isError, hasTenant }: CalibrationPanelProps) {
  if (!hasTenant) {
    return (
      <Card className="p-6">
        <h3 className="text-lg font-semibold text-gray-900 dark:text-white">
          Verdict calibration
        </h3>
        <p className="mt-2 text-sm text-gray-500 dark:text-gray-400">
          Log in as a tenant admin to load Tier 5.5 calibration stats.
        </p>
      </Card>
    );
  }

  if (isLoading) {
    return (
      <Card className="p-12">
        <div className="flex justify-center">
          <div className="pierre-spinner" />
        </div>
      </Card>
    );
  }

  if (isError || !stats) {
    return (
      <Card className="p-6">
        <h3 className="text-lg font-semibold text-gray-900 dark:text-white">
          Verdict calibration
        </h3>
        <p className="mt-2 text-sm text-red-600 dark:text-red-400">
          Failed to load verdict calibration stats.
        </p>
      </Card>
    );
  }

  const total = verdictTotalCount(stats.totals);
  const passRate = verdictPassRate(stats.totals);
  const passRateLabel = passRate === null ? '—' : `${Math.round(passRate * 100)}%`;
  const maxDailyTotal = stats.daily.reduce(
    (m, d) => Math.max(m, verdictTotalCount(d.counts)),
    0,
  );

  return (
    <div data-testid="verdict-calibration-panel">
    <Card className="p-6">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h3 className="text-lg font-semibold text-gray-900 dark:text-white">
            Verdict calibration
          </h3>
          <p className="mt-1 text-sm text-gray-500 dark:text-gray-400">
            Tier 5.5 claim verifier mix over the last {stats.window_days} days. Pass
            rate excludes rhetorical and unverifiable claims.
          </p>
        </div>
        <div className="text-right">
          <div className="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">
            Pass rate
          </div>
          <div
            className="text-2xl font-semibold text-gray-900 dark:text-white"
            data-testid="calibration-pass-rate"
          >
            {passRateLabel}
          </div>
          <div className="text-xs text-gray-400 dark:text-gray-500">
            {total} verdict{total === 1 ? '' : 's'}
          </div>
        </div>
      </div>

      <div className="mt-4 grid grid-cols-2 gap-2 sm:grid-cols-5">
        <StatusPill label="Supported" count={stats.totals.supported} tone="positive" />
        <StatusPill label="Unsupported" count={stats.totals.unsupported} tone="warning" />
        <StatusPill label="Contradicted" count={stats.totals.contradicted} tone="negative" />
        <StatusPill label="Rhetorical" count={stats.totals.rhetorical} tone="neutral" />
        <StatusPill label="Unverifiable" count={stats.totals.unverifiable} tone="neutral" />
      </div>

      {stats.daily.length === 0 ? (
        <p className="mt-4 text-xs text-gray-400 dark:text-gray-500">
          No verdicts recorded in the selected window.
        </p>
      ) : (
        <div className="mt-5">
          <div className="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">
            Daily drift
          </div>
          <div
            className="mt-2 flex h-24 items-end gap-1"
            data-testid="calibration-daily-drift"
          >
            {stats.daily.map((day) => {
              const dayTotal = verdictTotalCount(day.counts);
              const dayPass = verdictPassRate(day.counts);
              const height = maxDailyTotal === 0 ? 0 : (dayTotal / maxDailyTotal) * 100;
              const tone =
                dayPass === null
                  ? 'bg-gray-300 dark:bg-gray-600'
                  : dayPass >= 0.8
                    ? 'bg-green-500'
                    : dayPass >= 0.5
                      ? 'bg-yellow-500'
                      : 'bg-red-500';
              return (
                <div
                  key={day.date}
                  className="flex-1"
                  title={`${day.date}: ${dayTotal} verdict${
                    dayTotal === 1 ? '' : 's'
                  }, pass rate ${dayPass === null ? '—' : `${Math.round(dayPass * 100)}%`}`}
                >
                  <div
                    className={`w-full rounded-t ${tone}`}
                    style={{ height: `${Math.max(height, 4)}%` }}
                  />
                </div>
              );
            })}
          </div>
          <div className="mt-1 flex justify-between text-[10px] text-gray-400 dark:text-gray-500">
            <span>{stats.daily[0]?.date}</span>
            <span>{stats.daily[stats.daily.length - 1]?.date}</span>
          </div>
        </div>
      )}
    </Card>
    </div>
  );
}

interface StatusPillProps {
  label: string;
  count: number;
  tone: 'positive' | 'negative' | 'warning' | 'neutral';
}

function StatusPill({ label, count, tone }: StatusPillProps) {
  const toneClasses: Record<StatusPillProps['tone'], string> = {
    positive: 'text-green-700 dark:text-green-400 border-green-200 dark:border-green-800',
    negative: 'text-red-700 dark:text-red-400 border-red-200 dark:border-red-800',
    warning: 'text-yellow-700 dark:text-yellow-400 border-yellow-200 dark:border-yellow-800',
    neutral: 'text-gray-700 dark:text-gray-400 border-gray-200 dark:border-gray-700',
  };
  return (
    <div
      className={`rounded-lg border bg-white px-3 py-2 dark:bg-gray-800 ${toneClasses[tone]}`}
    >
      <div className="text-[10px] uppercase tracking-wide">{label}</div>
      <div className="text-lg font-semibold">{count}</div>
    </div>
  );
}
