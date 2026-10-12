// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Settings › Shoes — the pairs the athlete declares, each with its distance and warning, and the form to add or change one
// ABOUTME: Retiring keeps a pair's distance and history; a pair is deleted only while no activity counts on it (carnet#711)

import { useState, type FormEvent, type ReactNode } from 'react';
import { useTranslation } from '@pierre/i18n';
import { distanceSymbol, formatDate } from '@pierre/chat-utils';
import {
  GEAR_NAME_MAX_CHARS,
  GEAR_SPORTS,
  isGearSport,
  type CreateGearRequest,
  type GearItem,
} from '@pierre/shared-types';
import {
  checkGearDraft,
  classifyApiError,
  describeApiError,
  emptyGearDraft,
  formatGearDistance,
  gearChanges,
  gearDistanceLimit,
  gearDraftOf,
  splitGear,
  useDistanceUnit,
  type GearDraft,
  type GearDraftErrors,
} from '@pierre/ui-logic';
import { Badge, Button, Checkbox, ConfirmDialog, Input, Section, Select } from '../ui';
import { sportLabel } from '../home/homeFormat';
import { useGearChanges, useGearLedger } from '../../hooks/useGear';
import { GearWarning } from './GearWarning';

/** The HTTP status the server refuses a delete with while activities count on the pair. */
const ATTRIBUTED_STATUS = 409;

interface GearFormProps {
  initial: GearDraft;
  submitLabel: string;
  /** The submit button's label while the request is in flight; the submit label when absent. */
  busyLabel?: string;
  isPending: boolean;
  /** The request a draft inside the server's bounds makes; a field out of them is named beside it instead. */
  onSubmit: (request: Required<CreateGearRequest>) => void;
  onCancel?: () => void;
  testId: string;
}

/**
 * The fields of a pair: its name, its sport, the distances, and whether it is
 * pinned. The distances are typed in the units the draft opened in, and
 * labelled with them.
 */
function GearForm({ initial, submitLabel, busyLabel, isPending, onSubmit, onCancel, testId }: GearFormProps) {
  const { t, language } = useTranslation();
  const [draft, setDraft] = useState<GearDraft>(initial);
  const [errors, setErrors] = useState<GearDraftErrors>({});
  const unitLabel = { unit: distanceSymbol(draft.unit) };
  const params = {
    characters: GEAR_NAME_MAX_CHARS,
    distance: formatGearDistance(gearDistanceLimit(draft.unit), draft.unit, language),
  };
  const error = (key: string | undefined) => (key === undefined ? undefined : t(key, params));
  const set = (change: Partial<GearDraft>) => setDraft((current) => ({ ...current, ...change }));
  const submit = (event: FormEvent) => {
    event.preventDefault();
    const check = checkGearDraft(draft);
    if (!check.ok) {
      setErrors(check.errors);
      return;
    }
    setErrors({});
    onSubmit(check.request);
  };
  return (
    <form className="grid gap-4 sm:grid-cols-2" onSubmit={submit} noValidate data-testid={testId}>
      <Input
        label={t('gear.form.name')}
        placeholder={t('gear.form.namePlaceholder')}
        value={draft.name}
        maxLength={GEAR_NAME_MAX_CHARS}
        onChange={(event) => set({ name: event.target.value })}
        error={error(errors.name)}
        data-testid={`${testId}-name`}
      />
      <Select
        label={t('gear.form.sport')}
        value={draft.sport}
        options={GEAR_SPORTS.map((sport) => ({ value: sport, label: sportLabel(t, sport) }))}
        onChange={(event) => {
          const sport = event.target.value;
          if (isGearSport(sport)) set({ sport });
        }}
        data-testid={`${testId}-sport`}
      />
      <Input
        label={t('gear.form.initialDistance', unitLabel)}
        inputMode="decimal"
        value={draft.initialDistance}
        onChange={(event) => set({ initialDistance: event.target.value })}
        error={error(errors.initialDistance)}
        data-testid={`${testId}-initial`}
      />
      <Input
        label={t('gear.form.warnDistance', unitLabel)}
        inputMode="decimal"
        value={draft.warnDistance}
        onChange={(event) => set({ warnDistance: event.target.value })}
        error={error(errors.warnDistance)}
        helpText={t('gear.form.warnHint')}
        data-testid={`${testId}-warn`}
      />
      <div className="sm:col-span-2">
        <Checkbox
          label={t('gear.form.pinned')}
          description={t('gear.form.pinnedHint')}
          checked={draft.pinned}
          onChange={(event) => set({ pinned: event.target.checked })}
          data-testid={`${testId}-pinned`}
        />
      </div>
      <div className="flex gap-2 sm:col-span-2">
        <Button type="submit" size="sm" disabled={isPending} data-testid={`${testId}-submit`}>
          {isPending ? (busyLabel ?? submitLabel) : submitLabel}
        </Button>
        {onCancel && (
          <Button type="button" variant="secondary" size="sm" onClick={onCancel} data-testid={`${testId}-cancel`}>
            {t('common.cancel')}
          </Button>
        )}
      </div>
    </form>
  );
}

/** The figures under a pair's name: how many activities, when last used or retired, where it warns. */
function PairFacts({ pair }: { pair: GearItem }) {
  const { t, language } = useTranslation();
  const unit = useDistanceUnit();
  const count =
    pair.activity_count === 1
      ? t('gear.activityCountOne', { count: pair.activity_count })
      : t('gear.activityCountN', { count: pair.activity_count });
  return (
    <p className="mt-0.5 flex flex-wrap gap-x-3 text-xs text-on-surface-variant">
      <span>{count}</span>
      {pair.retired_at !== null ? (
        <span>{t('gear.retiredOn', { date: formatDate(pair.retired_at, language) })}</span>
      ) : (
        pair.last_used_at !== null && (
          <span>{t('gear.lastUsed', { date: formatDate(pair.last_used_at, language) })}</span>
        )
      )}
      <span>{t('gear.warnAt', { distance: formatGearDistance(pair.warn_distance_meters, unit, language) })}</span>
    </p>
  );
}

interface PairRowProps {
  pair: GearItem;
  actions: ReactNode;
  editor?: ReactNode;
}

/** One pair: its name, sport and badges, its distance, its facts, its warning, then its actions. */
function PairRow({ pair, actions, editor }: PairRowProps) {
  const { t, language } = useTranslation();
  const unit = useDistanceUnit();
  return (
    <li className="py-4" data-testid={`gear-${pair.id}`}>
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0">
          <p className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <span className="break-words text-sm font-medium text-on-surface">{pair.name}</span>
            <span className="text-xs text-on-surface-variant">{sportLabel(t, pair.sport)}</span>
            {pair.is_default && (
              <span title={t('gear.defaultHint')} data-testid={`gear-default-${pair.id}`}>
                <Badge variant="info">{t('gear.default')}</Badge>
              </span>
            )}
            {pair.pinned && <Badge variant="secondary">{t('gear.pinned')}</Badge>}
          </p>
          <p className="mt-1 font-mono text-base font-medium text-on-surface" data-testid={`gear-distance-${pair.id}`}>
            {formatGearDistance(pair.distance_meters, unit, language)}
          </p>
          <PairFacts pair={pair} />
          {pair.warn_reached && <GearWarning pair={pair} />}
        </div>
        <div className="flex shrink-0 flex-wrap gap-2">{actions}</div>
      </div>
      {editor && <div className="mt-4">{editor}</div>}
    </li>
  );
}

export default function GearSettings() {
  const { t } = useTranslation();
  const unit = useDistanceUnit();
  const ledger = useGearLedger();
  const { create, update, retire, restore, remove } = useGearChanges();
  const [editing, setEditing] = useState<string | null>(null);
  const [deleting, setDeleting] = useState<GearItem | null>(null);
  const [addKey, setAddKey] = useState(0);

  const { inUse, retired } = splitGear(ledger.data?.gear ?? []);
  const changeError = retire.error ?? restore.error;
  // A pair an activity was counted on since the list was read is refused
  // with 409: the dialog then says to retire it instead.
  let deleteMessage = t('gear.deleteMessage');
  if (remove.isError) {
    deleteMessage =
      classifyApiError(remove.error).status === ATTRIBUTED_STATUS
        ? t('gear.deleteLocked')
        : `${t('gear.deleteMessage')} ${t('gear.changeFailed')}`;
  }

  const add = (request: Required<CreateGearRequest>) => {
    create.mutate(request, { onSuccess: () => setAddKey((key) => key + 1) });
  };

  const save = (pair: GearItem, request: Required<CreateGearRequest>) => {
    const changes = gearChanges(pair, request);
    if (Object.keys(changes).length === 0) {
      setEditing(null);
      return;
    }
    update.mutate({ gearId: pair.id, changes }, { onSuccess: () => setEditing(null) });
  };

  const renderInUse = () => {
    if (ledger.isPending) {
      return <div className="pierre-spinner h-5 w-5" role="status" aria-label={t('common.loading')} />;
    }
    if (ledger.isError) {
      return (
        <p role="alert" className="text-sm text-error" data-testid="gear-load-failed">
          {describeApiError(ledger.error, { t, fallbackKey: 'gear.loadFailed' })}
        </p>
      );
    }
    if (inUse.length === 0) {
      return (
        <p className="text-sm text-on-surface-variant" data-testid="gear-empty">
          {t('gear.empty')}
        </p>
      );
    }
    return (
      <ul className="divide-y divide-outline-variant" aria-label={t('gear.inUse')} data-testid="gear-in-use">
        {inUse.map((pair) => (
          <PairRow
            key={pair.id}
            pair={pair}
            actions={
              <>
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  onClick={() => setEditing(editing === pair.id ? null : pair.id)}
                  aria-expanded={editing === pair.id}
                  data-testid={`gear-edit-${pair.id}`}
                >
                  {t('common.edit')}
                </Button>
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  disabled={retire.isPending}
                  onClick={() => retire.mutate(pair.id)}
                  data-testid={`gear-retire-${pair.id}`}
                >
                  {t('gear.retire')}
                </Button>
                {pair.activity_count === 0 && (
                  <Button
                    type="button"
                    variant="secondary"
                    size="sm"
                    onClick={() => setDeleting(pair)}
                    data-testid={`gear-delete-${pair.id}`}
                  >
                    {t('common.delete')}
                  </Button>
                )}
              </>
            }
            editor={
              editing === pair.id ? (
                <>
                  <GearForm
                    key={unit}
                    initial={gearDraftOf(pair, unit)}
                    submitLabel={t('common.save')}
                    isPending={update.isPending}
                    onSubmit={(request) => save(pair, request)}
                    onCancel={() => setEditing(null)}
                    testId={`gear-form-${pair.id}`}
                  />
                  {update.isError && (
                    <p role="alert" className="mt-2 text-sm text-error">
                      {describeApiError(update.error, { t, fallbackKey: 'gear.saveFailed' })}
                    </p>
                  )}
                </>
              ) : undefined
            }
          />
        ))}
      </ul>
    );
  };

  return (
    <div className="space-y-10" data-testid="gear-settings">
      <Section title={t('gear.title')} description={t('gear.blurb')}>
        {renderInUse()}
        {changeError && (
          <p role="alert" className="mt-2 text-sm text-error" data-testid="gear-change-failed">
            {describeApiError(changeError, { t, fallbackKey: 'gear.changeFailed' })}
          </p>
        )}
      </Section>
      <Section title={t('gear.addTitle')}>
        {/* Keyed on the units too: they can resolve after the form is drawn. */}
        <GearForm
          key={`${addKey}-${unit}`}
          initial={emptyGearDraft('run', unit)}
          submitLabel={t('gear.add')}
          busyLabel={t('gear.adding')}
          isPending={create.isPending}
          onSubmit={add}
          testId="gear-add"
        />
        {create.isError && (
          <p role="alert" className="mt-2 text-sm text-error" data-testid="gear-add-failed">
            {describeApiError(create.error, { t, fallbackKey: 'gear.addFailed' })}
          </p>
        )}
      </Section>
      {retired.length > 0 && (
        <Section title={t('gear.retired')} description={t('gear.retiredHint')}>
          <ul className="divide-y divide-outline-variant" data-testid="gear-retired">
            {retired.map((pair) => (
              <PairRow
                key={pair.id}
                pair={pair}
                actions={
                  <>
                    <Button
                      type="button"
                      variant="secondary"
                      size="sm"
                      disabled={restore.isPending}
                      onClick={() => restore.mutate(pair.id)}
                      data-testid={`gear-restore-${pair.id}`}
                    >
                      {t('gear.restore')}
                    </Button>
                    {pair.activity_count === 0 && (
                      <Button
                        type="button"
                        variant="secondary"
                        size="sm"
                        onClick={() => setDeleting(pair)}
                        data-testid={`gear-delete-${pair.id}`}
                      >
                        {t('common.delete')}
                      </Button>
                    )}
                  </>
                }
              />
            ))}
          </ul>
        </Section>
      )}
      <ConfirmDialog
        isOpen={deleting !== null}
        onClose={() => {
          setDeleting(null);
          remove.reset();
        }}
        onConfirm={() =>
          deleting && remove.mutate(deleting.id, { onSuccess: () => setDeleting(null) })
        }
        title={t('gear.deleteTitle', { name: deleting?.name ?? '' })}
        message={deleteMessage}
        confirmLabel={t('common.delete')}
        cancelLabel={t('common.cancel')}
        variant="danger"
        isLoading={remove.isPending}
      />
    </div>
  );
}
