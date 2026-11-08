// ABOUTME: Phase B Sprint C3 — admin tab editing the global coaching harness configuration
// ABOUTME: Compaction thresholds + Tier 6 locale-aware text guardrails persisted via /admin/settings/harness
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import {
  useState,
  useEffect,
  useMemo,
  useCallback,
  type ReactNode,
} from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  adminApi,
  HARNESS_CONFIG_STALE,
  harnessRefusalOf,
  type HarnessConfigDocument,
  type HarnessConfigResponse,
  type LocaleGuardrails,
} from "../services/api/admin";
import { Card, Button, Badge, Input } from "./ui";
import { ADMIN_LOCALE, describeAdminError } from "../utils/adminLocale";

const HARNESS_CONFIG_QUERY_KEY = ["admin", "harness-config"] as const;

/// BCP-47 short codes editable from this tab. Server defaults ship the same
/// five; operators can add more by extending the locales map manually but
/// the UI surfaces the supported set.
const SUPPORTED_LOCALES: readonly string[] = ["en", "fr", "es", "de", "pt"];

const LOCALE_LABELS: Record<string, string> = {
  en: "English",
  fr: "Français",
  es: "Español",
  de: "Deutsch",
  pt: "Português",
};

function emptyLocaleGuardrails(): LocaleGuardrails {
  return { disclaimer_triggers: [] };
}

function parseCsv(value: string): string[] {
  return value
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
}

function joinCsv(values: string[]): string {
  return values.join(", ");
}

interface ValidationError {
  field: string;
  message: string;
}

/// The fewest turns a summary can compact; the server refuses less.
const MIN_SUMMARIZE_OLDEST_N = 2;

/// The client's copy of `validate_document`, so the admin learns before saving. A
/// cleared number field is NaN, which every one of these refuses.
function validate(doc: HarnessConfigDocument): ValidationError[] {
  const errors: ValidationError[] = [];
  const c = doc.compaction;
  if (!Number.isFinite(c.window_tokens) || c.window_tokens <= 0) {
    errors.push({
      field: "window_tokens",
      message: "must be a positive integer",
    });
  }
  if (!(c.warn_threshold > 0 && c.warn_threshold <= 1)) {
    errors.push({ field: "warn_threshold", message: "must be in (0, 1]" });
  }
  if (!(c.emergency_threshold > 0 && c.emergency_threshold <= 1)) {
    errors.push({ field: "emergency_threshold", message: "must be in (0, 1]" });
  }
  if (!(c.max_messages > 0)) {
    errors.push({ field: "max_messages", message: "must be above 0" });
  }
  if (!(c.summarize_oldest_n >= MIN_SUMMARIZE_OLDEST_N)) {
    errors.push({
      field: "summarize_oldest_n",
      message: `must be at least ${MIN_SUMMARIZE_OLDEST_N}, the fewest turns a summary can compact`,
    });
  } else if (c.summarize_oldest_n + 2 > c.max_messages) {
    errors.push({
      field: "summarize_oldest_n",
      message:
        "plus 2 must not exceed max messages, or summarization can never run",
    });
  }
  if (!(c.sliding_drop_n >= 0)) {
    errors.push({ field: "sliding_drop_n", message: "must be 0 or more" });
  }
  if (!(doc.memory.dedup_candidate_limit > 0)) {
    errors.push({ field: "dedup_candidate_limit", message: "must be above 0" });
  }
  if (c.warn_threshold >= c.emergency_threshold) {
    errors.push({
      field: "warn_threshold",
      message: "warn must be strictly less than emergency",
    });
  }
  return errors;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function sameValue(a: unknown, b: unknown): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/// Dotted paths of every setting that differs between two documents. A locale code
/// has no dot, so a path reads unambiguously.
function changedPaths(before: unknown, after: unknown, prefix = ""): string[] {
  if (isRecord(before) && isRecord(after)) {
    const keys = new Set([...Object.keys(before), ...Object.keys(after)]);
    return [...keys]
      .sort()
      .flatMap((key) =>
        changedPaths(
          before[key],
          after[key],
          prefix ? `${prefix}.${key}` : key,
        ),
      );
  }
  return sameValue(before, after) ? [] : [prefix];
}

/// `stored` with only what the admin changed from `base` applied on top, so moving
/// their edits to the newer version keeps the other save's work.
function reapplyEdits(
  base: unknown,
  edited: unknown,
  stored: unknown,
): unknown {
  if (isRecord(base) && isRecord(edited) && isRecord(stored)) {
    const keys = new Set([...Object.keys(stored), ...Object.keys(edited)]);
    return Object.fromEntries(
      [...keys].map((key) => [
        key,
        reapplyEdits(base[key], edited[key], stored[key]),
      ]),
    );
  }
  return sameValue(base, edited) ? stored : edited;
}

/// A save the server refused as stale. `stored` is the version that beat it, or null
/// while it could not be read.
interface SaveConflict {
  message: string;
  stored: HarnessConfigResponse | null;
}

export default function HarnessConfigTab() {
  const queryClient = useQueryClient();
  const { data, isLoading, isError, error, refetch } = useQuery({
    queryKey: HARNESS_CONFIG_QUERY_KEY,
    queryFn: () => adminApi.getHarnessConfig(),
  });

  // Null until the server answers: the server is the only source of the defaults.
  const [draft, setDraft] = useState<HarnessConfigDocument | null>(null);
  const [activeLocale, setActiveLocale] = useState<string>("en");
  // Also the concurrency token a save echoes: verbatim from the response, since a
  // `Date` would drop digits the server compares.
  const [savedAt, setSavedAt] = useState<string | null>(null);
  const [serverError, setServerError] = useState<string | null>(null);
  const [conflict, setConflict] = useState<SaveConflict | null>(null);

  // What the edit started from, so a background refetch can tell an edit in progress.
  const [base, setBase] = useState<HarnessConfigDocument | null>(null);
  const isDirty =
    base !== null && JSON.stringify(draft) !== JSON.stringify(base);

  const hydrate = useCallback((response: HarnessConfigResponse) => {
    setBase(response.config);
    setDraft(response.config);
    setSavedAt(response.updated_at);
    setConflict(null);
  }, []);

  // A background refetch must not write over unsaved edits, and must not move the
  // token under them either: a save still names the version it started from, so
  // the server can refuse it rather than lose the other save.
  useEffect(() => {
    if (data?.config && !isDirty) {
      hydrate(data);
    }
  }, [data, isDirty, hydrate]);

  const validationErrors = useMemo(
    () => (draft ? validate(draft) : []),
    [draft],
  );
  const isInvalid = validationErrors.length > 0;

  // The draft and the token stay as they were, so a second press of Save would be
  // refused too: nothing can land over the other save until the admin has chosen.
  const readConflict = async (message: string) => {
    const fresh = await refetch();
    // A failed refetch still carries the last good `data`, the very response the
    // save was refused against, so `data` alone does not say the reload worked.
    setConflict({
      message,
      stored: fresh.isError || !fresh.data ? null : fresh.data,
    });
  };

  const mutation = useMutation({
    mutationFn: (doc: HarnessConfigDocument) =>
      adminApi.putHarnessConfig(doc, savedAt),
    onSuccess: (resp: HarnessConfigResponse) => {
      setServerError(null);
      hydrate(resp);
      queryClient.setQueryData(HARNESS_CONFIG_QUERY_KEY, resp);
    },
    onError: (err: unknown) => {
      setServerError(null);
      const refusal = harnessRefusalOf(err);
      if (refusal?.code === HARNESS_CONFIG_STALE) {
        void readConflict(refusal.message);
      } else if (refusal) {
        setServerError(refusal.message);
      } else {
        setServerError(describeAdminError(err));
      }
    },
  });

  if (isLoading || (!draft && !isError)) {
    return (
      <Card className="p-12">
        <div className="flex justify-center">
          <div className="pierre-spinner" />
        </div>
      </Card>
    );
  }

  // A failed background refetch keeps the last good response, and the form with it.
  if (!draft || (isError && !data)) {
    return (
      <Card className="p-6">
        <p className="text-sm text-error">
          Failed to load harness config: {describeAdminError(error)}
        </p>
      </Card>
    );
  }

  const activeLocaleRules: LocaleGuardrails =
    draft.guardrails.locales[activeLocale] ?? emptyLocaleGuardrails();

  function updateActiveLocale(
    doc: HarnessConfigDocument,
    next: LocaleGuardrails,
  ) {
    setDraft({
      ...doc,
      guardrails: {
        ...doc.guardrails,
        locales: { ...doc.guardrails.locales, [activeLocale]: next },
      },
    });
  }

  const tokensAt = (threshold: number): string =>
    Number.isFinite(threshold * draft.compaction.window_tokens)
      ? Math.round(threshold * draft.compaction.window_tokens).toLocaleString(
          ADMIN_LOCALE,
        )
      : "—";

  const keepEditsOnStored = (stored: HarnessConfigResponse) => {
    setDraft(reapplyEdits(base, draft, stored.config) as HarnessConfigDocument);
    setBase(stored.config);
    setSavedAt(stored.updated_at);
    setConflict(null);
  };

  return (
    <div className="space-y-4">
      <Card className="p-6">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h2 className="text-xl font-semibold text-on-surface">
              Coaching Harness Configuration
            </h2>
            <p className="mt-1 text-sm text-on-surface-variant">
              Tunables for conversation compaction and response text guardrails,
              one document for every tenant. A save applies at once on the
              instance that took it and reaches every other instance within its
              settings poll (30 seconds unless
              PIERRE_SETTINGS_POLL_INTERVAL_SECS says otherwise), with no
              restart.
            </p>
            {data?.in_force ? (
              <p
                className="mt-2 text-xs text-on-surface-variant"
                data-testid="harness-in-force"
              >
                {data.in_force.in_sync
                  ? "The instance that answered is running these values"
                  : "The instance that answered is not running these values yet"}{" "}
                (<span className="font-mono">{data.in_force.instance_id}</span>,
                loaded{" "}
                {new Date(data.in_force.loaded_at).toLocaleString(ADMIN_LOCALE)}
                ). Other instances are not checked here.
                {data.in_force.load_failure
                  ? ` ${data.in_force.load_failure}.`
                  : ""}
              </p>
            ) : null}
          </div>
          <div className="flex flex-col items-end gap-1">
            <Badge
              variant={data?.source === "persisted" ? "success" : "secondary"}
            >
              {data?.source === "persisted" ? "persisted" : "using defaults"}
            </Badge>
            {savedAt ? (
              <span className="text-xs text-on-surface-variant">
                Saved {new Date(savedAt).toLocaleString()}
              </span>
            ) : null}
          </div>
        </div>
      </Card>

      <Card className="p-6">
        <h3 className="mb-3 text-sm font-semibold text-on-surface-variant">
          Conversation compaction
        </h3>
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
          <NumericField
            label="Window tokens"
            help="The base the two thresholds are fractions of. Nothing sends this to the LLM, so lowering it moves both triggers."
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
            help={`Fraction of the window that triggers summarization: ${tokensAt(draft.compaction.warn_threshold)} tokens.`}
            value={draft.compaction.warn_threshold}
            min={0}
            max={1}
            step={0.0001}
            onChange={(v) =>
              setDraft({
                ...draft,
                compaction: { ...draft.compaction, warn_threshold: v },
              })
            }
          />
          <NumericField
            label="Emergency threshold"
            help={`Fraction of the window above which history is dropped with no summary: ${tokensAt(draft.compaction.emergency_threshold)} tokens.`}
            value={draft.compaction.emergency_threshold}
            min={0}
            max={1}
            step={0.0001}
            onChange={(v) =>
              setDraft({
                ...draft,
                compaction: { ...draft.compaction, emergency_threshold: v },
              })
            }
          />
          <NumericField
            label="Summarize oldest N"
            help={`How many oldest turns one summary compacts. From ${MIN_SUMMARIZE_OLDEST_N} up to max messages minus 2.`}
            value={draft.compaction.summarize_oldest_n}
            min={MIN_SUMMARIZE_OLDEST_N}
            max={Math.max(
              draft.compaction.max_messages - 2,
              MIN_SUMMARIZE_OLDEST_N,
            )}
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
            help="The fewest oldest turns a slide drops, so a thread barely over its budget does not slide again next turn. It drops more when fitting takes more."
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
          <NumericField
            label="Max messages"
            help="Non-system messages a prompt may hold. Over this, a turn summarizes or slides whatever its token count, so this is what usually fires."
            value={draft.compaction.max_messages}
            min={1}
            step={1}
            onChange={(v) =>
              setDraft({
                ...draft,
                compaction: { ...draft.compaction, max_messages: v },
              })
            }
          />
          <NumericField
            label="Memory dedup candidates"
            help="How many of an athlete's facts a new fact is compared with, and how many the extractor is shown to spot a restatement."
            value={draft.memory.dedup_candidate_limit}
            min={1}
            step={1}
            onChange={(v) =>
              setDraft({
                ...draft,
                memory: { ...draft.memory, dedup_candidate_limit: v },
              })
            }
          />
        </div>
      </Card>

      <Card className="p-6">
        <h3 className="mb-3 text-sm font-semibold text-on-surface-variant">
          Response text guardrails
        </h3>
        <div className="space-y-4">
          <CsvField
            label="Blocked topics (comma separated)"
            help="A reply containing any of these phrases (case insensitive) is replaced whole by a refusal. The match reads the words, not their intent: a reply quoting the phrase to debunk it is replaced too."
            value={joinCsv(draft.guardrails.blocked_topics)}
            onChange={(v) =>
              setDraft({
                ...draft,
                guardrails: {
                  ...draft.guardrails,
                  blocked_topics: parseCsv(v),
                },
              })
            }
          />

          <div>
            <div className="mb-2 text-sm font-medium text-on-surface-variant">
              Disclaimer triggers — per locale
            </div>
            <p className="mb-3 text-xs text-on-surface-variant">
              Triggers are matched against the assistant reply with Unicode word
              boundaries in the locale of the active conversation. Authoring
              per-locale prevents false-positives like the English word
              &ldquo;pain&rdquo; firing on the French word for bread. If the
              active turn locale has no entry, the server falls back to the
              English entry; if that is also absent, no disclaimer is prepended.
              The disclaimer wording is not edited here: it comes from the
              string catalogue, in the locale whose triggers matched.
            </p>
            <div className="mb-3 flex gap-1" role="tablist" aria-label="Locale">
              {SUPPORTED_LOCALES.map((loc) => (
                <button
                  key={loc}
                  type="button"
                  role="tab"
                  aria-selected={activeLocale === loc}
                  onClick={() => setActiveLocale(loc)}
                  className={`rounded px-3 py-1 text-xs font-medium ${
                    activeLocale === loc
                      ? "bg-primary text-on-primary"
                      : "bg-surface-container text-on-surface-variant hover:bg-surface-container-high"
                  }`}
                >
                  {LOCALE_LABELS[loc] ?? loc}
                </button>
              ))}
            </div>

            <CsvField
              label={`Triggers (${activeLocale})`}
              help={`Single words in ${LOCALE_LABELS[activeLocale] ?? activeLocale} that prepend the disclaimer when matched.`}
              value={joinCsv(activeLocaleRules.disclaimer_triggers)}
              onChange={(v) =>
                updateActiveLocale(draft, {
                  ...activeLocaleRules,
                  disclaimer_triggers: parseCsv(v),
                })
              }
            />
          </div>
        </div>
      </Card>

      <Card className="p-6">
        <h3 className="mb-3 text-sm font-semibold text-on-surface-variant">
          Claim verification
        </h3>
        <label className="flex items-start gap-3 text-sm">
          <input
            type="checkbox"
            checked={draft.verification?.runtime_judge ?? true}
            onChange={(e) =>
              setDraft({
                ...draft,
                verification: { runtime_judge: e.target.checked },
              })
            }
            className="mt-1 h-4 w-4"
          />
          <span>
            <span className="block font-medium text-on-surface-variant">
              Runtime LLM judge (Layer 5)
            </span>
            <span className="mt-1 block text-xs text-on-surface-variant">
              Lets claim verification consult the live chat provider for claims
              the deterministic layers leave inconclusive. The judge only runs
              as a tiebreaker, fails open on provider errors, and rides the
              cheap-tier provider chain. Unchecked keeps verification fully
              deterministic.
            </span>
          </span>
        </label>
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

      {data?.degraded_reason ? (
        <p
          role="alert"
          className="rounded bg-warning/15 px-3 py-2 text-sm text-on-warning-container"
        >
          Showing what this instance holds, not the stored row:{" "}
          {data.degraded_reason}. Saving replaces the row.
        </p>
      ) : null}

      {conflict ? (
        <ConflictPanel>
          <p>
            {conflict.message} Nothing was saved, and your edits are still in
            this form.
          </p>
          {conflict.stored ? (
            <>
              <ChangedSettings
                title="Changed by the other save"
                paths={changedPaths(base, conflict.stored.config)}
              />
              <ChangedSettings
                title="Your edits"
                paths={changedPaths(base, draft)}
              />
              <div className="mt-3 flex flex-wrap gap-2">
                <Button
                  variant="primary"
                  onClick={() =>
                    keepEditsOnStored(conflict.stored as HarnessConfigResponse)
                  }
                >
                  Apply my edits to the stored version
                </Button>
                <Button
                  variant="secondary"
                  onClick={() =>
                    hydrate(conflict.stored as HarnessConfigResponse)
                  }
                >
                  Discard my edits
                </Button>
              </div>
            </>
          ) : (
            <>
              <p className="mt-1">The stored version could not be read.</p>
              <div className="mt-3">
                <Button
                  variant="secondary"
                  onClick={() => void readConflict(conflict.message)}
                >
                  Try again
                </Button>
              </div>
            </>
          )}
        </ConflictPanel>
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
            if (data?.config) hydrate(data);
          }}
          disabled={mutation.isPending}
        >
          Reset
        </Button>
        <Button
          variant="primary"
          onClick={() => mutation.mutate(draft)}
          disabled={
            isInvalid || !isDirty || conflict !== null || mutation.isPending
          }
        >
          {mutation.isPending ? "Saving…" : "Save changes"}
        </Button>
      </div>
    </div>
  );
}

function ConflictPanel({ children }: { children: ReactNode }) {
  return (
    <div
      role="alert"
      className="rounded bg-warning/15 px-3 py-2 text-sm text-on-warning-container"
    >
      {children}
    </div>
  );
}

function ChangedSettings({ title, paths }: { title: string; paths: string[] }) {
  return (
    <div className="mt-2">
      <div className="font-medium">{title}</div>
      <ul className="font-mono text-xs">
        {paths.length === 0 ? (
          <li>nothing</li>
        ) : (
          paths.map((path) => <li key={path}>{path}</li>)
        )}
      </ul>
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

/// Holds what was typed, so a cleared field reads as empty and invalid. Reading it
/// straight as a number turned an empty box into 0, which saved as a real value.
function NumericField({
  label,
  help,
  value,
  min,
  max,
  step,
  onChange,
}: NumericFieldProps) {
  const [text, setText] = useState(String(value));
  useEffect(() => {
    const typed = text.trim() === "" ? Number.NaN : Number(text);
    if (!Object.is(typed, value)) {
      setText(Number.isNaN(value) ? "" : String(value));
    }
  }, [text, value]);
  return (
    <Input
      label={label}
      type="number"
      value={text}
      min={min}
      max={max}
      step={step}
      onChange={(e) => {
        setText(e.target.value);
        onChange(
          e.target.value.trim() === "" ? Number.NaN : Number(e.target.value),
        );
      }}
      helpText={help}
    />
  );
}

interface CsvFieldProps {
  label: string;
  help?: string;
  value: string;
  onChange: (next: string) => void;
}

/// Holds what was typed, so a space or a comma survives until the next word follows
/// it. Rendering the parsed list straight back stripped them on each keystroke.
function CsvField({ label, help, value, onChange }: CsvFieldProps) {
  const [text, setText] = useState(value);
  useEffect(() => {
    if (joinCsv(parseCsv(text)) !== value) {
      setText(value);
    }
  }, [text, value]);
  return (
    <label className="block text-sm">
      <span className="mb-1 block font-medium text-on-surface-variant">
        {label}
      </span>
      <input
        type="text"
        value={text}
        onChange={(e) => {
          setText(e.target.value);
          onChange(e.target.value);
        }}
        className="w-full rounded border border-outline-variant bg-surface-container-lowest px-3 py-2 text-sm text-on-surface"
      />
      {help ? (
        <span className="mt-1 block text-xs text-on-surface-variant">
          {help}
        </span>
      ) : null}
    </label>
  );
}
