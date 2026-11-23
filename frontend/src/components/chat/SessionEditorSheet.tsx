// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach's editor for one session of a date in their athlete's plan — "The day, opened": the session by hand, or a build from a family
// ABOUTME: Edits a session, adds one to a date, or removes one; a write that finds its session moved says so and closes (carnet#1515, #1714, #1716)

import { useId, useMemo, useState } from 'react';
import { useTranslation } from '@pierre/i18n';
import type {
  AddedSession,
  PlanDay,
  PlanSubject,
  SessionBuild,
} from '@pierre/shared-types';
import {
  API_ERROR_KEYS,
  MAX_SESSIONS_PER_DATE,
  ROW_DATE,
  SESSION_EDITOR_KEYS,
  canRemoveSession,
  dayPosition,
  describeApiError,
  editorDirty,
  familyMissingForDaySport,
  familyBuildChanged,
  formatCivilDate,
  handDayRequest,
  handSessionChanged,
  handSessionComplete,
  handSessionOf,
  handSportOptions,
  initialEditorMode,
  initialSessionBuild,
  isDateFull,
  isSessionMoved,
  newSessionDay,
  presetOf,
  replacementLine,
  restoreLine,
  sessionExpect,
  type DayReplacement,
  type EditorMode,
  type HandSession,
} from '@pierre/ui-logic';
import { useIsMobile } from '../../hooks/useBreakpoint';
import {
  useAddDraftSession,
  useRemoveDraftSession,
  useRereadPlan,
  useRestoreDraftDay,
  useSaveDraftDay,
  useSaveDraftHandDay,
  useSubjectFamilies,
} from '../../hooks/useSessionEditor';
import {
  Button,
  ConfirmDialog,
  SegmentedControl,
  Sheet,
  useErrorToast,
  useSuccessToast,
} from '../ui';
import { SessionEditorFamily } from './SessionEditorFamily';
import { SessionEditorHand } from './SessionEditorHand';

/** What the sheet opens on: an entry of a date and every entry on that date, to edit it or to add one after them. */
export interface SessionEditorOpening {
  /** The entry pressed, as the card shows it; in `add`, any entry of the date. */
  day: PlanDay;
  /** Every entry on the date, in order, from the card's week. */
  entries: readonly PlanDay[];
  mode: 'edit' | 'add';
}

interface SessionEditorSheetProps {
  /** The coach's plan thread on the athlete. */
  conversationId: string;
  /** The athlete the plan is for. */
  subject: PlanSubject;
  opening: SessionEditorOpening;
  onClose: () => void;
}

/**
 * The sheet an entry's "Edit day" (or "Edit session") opens, and the one
 * "Add a session" opens on a blank session after the date's entries. It
 * opens on the session's own words — a session a family built opens on its
 * family — and "From a family" is the other mode. Each mode keeps what the
 * coach did in it, so switching loses nothing; Save writes the mode on screen
 * into the coach's draft.
 *
 * Every write but the add names its session by position and as the card
 * showed it; one the server finds moved is never retried onto another
 * session: the sheet says so, the card reads the plan again, and it closes.
 *
 * The session mode never waits on the family catalogue: the catalogue is
 * read once the sheet opens, and its loading, error and empty states live in
 * the family mode alone.
 */
export default function SessionEditorSheet({
  conversationId,
  subject,
  opening,
  onClose,
}: SessionEditorSheetProps) {
  const { t, language } = useTranslation();
  const isMobile = useIsMobile();
  const showSuccess = useSuccessToast();
  const showError = useErrorToast();
  const replacementId = useId();
  const { day, entries } = opening;
  const adding = opening.mode === 'add';
  const athleteName = subject.display_name.trim() || t('app.unknownMember');
  const date = formatCivilDate(day.date, language, ROW_DATE);
  const sessionName = day.rest ? t('chat.restDay') : day.workout;

  // What the sheet measures the coach's work against: the entry itself, or
  // the blank session an add starts from.
  const baseline = useMemo(
    () => (adding ? newSessionDay(day.date, entries) : day),
    [adding, day, entries],
  );
  const [mode, setMode] = useState<EditorMode>(() =>
    initialEditorMode(baseline),
  );
  const startHand = useMemo(() => handSessionOf(baseline), [baseline]);
  // Built once from the opening session, its own word first: built from the
  // live one, the options would reorder under the cursor on every pick.
  const sportOptions = useMemo(() => handSportOptions(startHand), [startHand]);
  const [hand, setHand] = useState<HandSession>(startHand);

  const families = useSubjectFamilies(conversationId, language);
  const catalogue = families.data;
  const startBuild = useMemo(
    () => (catalogue ? initialSessionBuild(baseline, catalogue) : null),
    [catalogue, baseline],
  );
  const [familyEdit, setFamilyEdit] = useState<SessionBuild | null>(null);
  const build = familyEdit ?? startBuild;
  const preset = catalogue && build ? presetOf(catalogue, build) : undefined;

  const handChanged = handSessionChanged(baseline, hand);
  const buildChanged =
    startBuild !== null &&
    build !== null &&
    familyBuildChanged(baseline, startBuild, build);
  // What a close would lose, in either mode: the one rule web and mobile share.
  // Opening the family mode is not a change; picking in it is.
  const dirty = editorDirty(baseline, hand, startBuild, familyEdit);

  let next: DayReplacement | null = null;
  if (mode === 'session' && handChanged) {
    next = hand.rest
      ? { rest: true }
      : { rest: false, name: hand.workout.trim() };
  } else if (mode === 'family' && buildChanged && preset) {
    next = { rest: false, name: preset.name };
  }
  // An added session replaces nothing: it saves once it is whole, or once a
  // family build stands.
  const canSave = adding
    ? mode === 'session'
      ? handSessionComplete(hand)
      : build !== null
    : next !== null && (mode === 'family' || handSessionComplete(hand));
  // No sentence while a session has no title to name yet, nor on an add.
  const replacement = adding ? null : replacementLine(day, next);
  const restoreNote = adding ? null : restoreLine(day);
  const removable = !adding && canRemoveSession(entries);

  const saveHand = useSaveDraftHandDay(conversationId, language);
  const saveFamily = useSaveDraftDay(conversationId, language);
  const add = useAddDraftSession(conversationId, language);
  const restore = useRestoreDraftDay(conversationId, language);
  const remove = useRemoveDraftSession(conversationId, language);
  const reread = useRereadPlan(conversationId);
  const saving = saveHand.isPending || saveFamily.isPending || add.isPending;
  // The session every write but the add is about, as the card showed it.
  const at = {
    date: day.date,
    position: dayPosition(day),
    expect: sessionExpect(day),
  };

  const [confirmingClose, setConfirmingClose] = useState(false);
  const [confirmingRemove, setConfirmingRemove] = useState(false);
  // Cancel, the close button, Escape and the scrim all come through here.
  const requestClose = () => {
    if (dirty) setConfirmingClose(true);
    else onClose();
  };

  /**
   * A write refused because the session at its position is no longer the
   * one the card showed: say so, read the plan again, and close. Never a
   * retry onto whichever session sits there now.
   */
  const closedOnMove = (error: unknown): boolean => {
    if (!isSessionMoved(error)) return false;
    showError(t(SESSION_EDITOR_KEYS.sessionMoved));
    reread();
    onClose();
    return true;
  };

  const onSave = async () => {
    try {
      if (adding) {
        const session: AddedSession | null =
          mode === 'session'
            ? { day: handDayRequest(hand) }
            : build
              ? { build }
              : null;
        if (session === null) return;
        await add.mutateAsync({ date: day.date, session });
      } else if (mode === 'session') {
        await saveHand.mutateAsync({ ...at, day: handDayRequest(hand) });
      } else if (build) {
        await saveFamily.mutateAsync({ ...at, build });
      }
      showSuccess(t(SESSION_EDITOR_KEYS.draftNote, { name: athleteName }));
      onClose();
    } catch (error) {
      if (closedOnMove(error)) return;
      // An add onto a date filled meanwhile says so in the coach's language.
      if (adding && isDateFull(error)) {
        showError(
          t(SESSION_EDITOR_KEYS.saveFailed),
          t(SESSION_EDITOR_KEYS.dateFull, {
            date,
            max: String(MAX_SESSIONS_PER_DATE),
          }),
        );
        return;
      }
      // A refused hand save names what it refused — a target outside the
      // calendar's zone words lists the words it takes — and that sentence
      // is the toast's body.
      showError(
        t(SESSION_EDITOR_KEYS.saveFailed),
        describeApiError(error, { t, fallbackKey: API_ERROR_KEYS.unknown }),
      );
    }
  };

  const onRestore = async () => {
    try {
      await restore.mutateAsync(at);
      // Restoring a session the coach added takes it off the date.
      showSuccess(
        day.added
          ? t(SESSION_EDITOR_KEYS.removed, { workout: sessionName, date })
          : t(SESSION_EDITOR_KEYS.restored, { date }),
      );
      onClose();
    } catch (error) {
      if (closedOnMove(error)) return;
      showError(
        t(SESSION_EDITOR_KEYS.restoreFailed),
        describeApiError(error, { t, fallbackKey: API_ERROR_KEYS.unknown }),
      );
    }
  };

  const onRemove = async () => {
    try {
      await remove.mutateAsync(at);
      showSuccess(
        t(SESSION_EDITOR_KEYS.removed, { workout: sessionName, date }),
      );
      onClose();
    } catch (error) {
      setConfirmingRemove(false);
      if (closedOnMove(error)) return;
      showError(
        t(SESSION_EDITOR_KEYS.removeFailed),
        describeApiError(error, { t, fallbackKey: API_ERROR_KEYS.unknown }),
      );
    }
  };

  return (
    <Sheet
      side={isMobile ? 'bottom' : 'right'}
      width="lg"
      title={t(
        adding ? SESSION_EDITOR_KEYS.addTitle : SESSION_EDITOR_KEYS.editTitle,
        { date },
      )}
      onClose={requestClose}
      data-testid="session-editor"
    >
      <div className="flex min-h-full flex-col">
        <div className="flex-1 space-y-5 px-4 pb-6 pt-1">
          {restoreNote && (
            <EditedRow
              added={day.added === true}
              note={t(restoreNote.key, restoreNote.values)}
              restoring={restore.isPending}
              onRestore={() => void onRestore()}
            />
          )}

          <SegmentedControl
            value={mode}
            onChange={setMode}
            stretch
            data-testid="session-editor-mode"
            options={[
              {
                value: 'session',
                label: t(SESSION_EDITOR_KEYS.modeSession),
                'data-testid': 'session-editor-mode-session',
              },
              {
                value: 'family',
                label: t(SESSION_EDITOR_KEYS.modeFamily),
                'data-testid': 'session-editor-mode-family',
              },
            ]}
          />

          {mode === 'session' ? (
            <SessionEditorHand
              hand={hand}
              sportOptions={sportOptions}
              restOffered={!adding}
              onChange={setHand}
            />
          ) : (
            <SessionEditorFamily
              conversationId={conversationId}
              date={day.date}
              athleteName={athleteName}
              catalogue={catalogue}
              catalogueFailed={families.isError}
              build={build}
              familyMissing={
                catalogue ? familyMissingForDaySport(baseline, catalogue) : null
              }
              onChange={setFamilyEdit}
            />
          )}
        </div>

        {/* The sentence has the footer's width to itself: it names the sessions
            a save swaps, and a name broken over two lines reads as two. */}
        <div className="sticky bottom-0 space-y-2 border-t ghost-border-faint bg-surface px-4 py-3">
          {replacement && (
            <p
              id={replacementId}
              className="text-pretty text-sm text-on-surface-variant"
              data-testid="session-editor-replacement"
            >
              {t(replacement.key, replacement.values)}
            </p>
          )}
          {/* Remove stands apart at the start; Cancel and Save keep together at
              the end, wrapping as one when the row is narrow. */}
          <div className="flex flex-wrap items-center gap-2">
            {removable && (
              <Button
                variant="danger-outline"
                aria-haspopup="dialog"
                onClick={() => setConfirmingRemove(true)}
                data-testid="session-editor-remove"
              >
                {t(SESSION_EDITOR_KEYS.removeSession)}
              </Button>
            )}
            <div className="ml-auto flex items-center gap-2">
              <Button
                variant="tertiary"
                onClick={requestClose}
                data-testid="session-editor-cancel"
              >
                {t(SESSION_EDITOR_KEYS.cancel)}
              </Button>
              <Button
                variant="primary"
                loading={saving}
                disabled={!canSave}
                aria-describedby={replacement ? replacementId : undefined}
                onClick={() => void onSave()}
                data-testid="session-editor-save"
              >
                {saving
                  ? t(SESSION_EDITOR_KEYS.saving)
                  : t(SESSION_EDITOR_KEYS.save)}
              </Button>
            </div>
          </div>
        </div>
      </div>

      {confirmingClose && (
        <ConfirmDialog
          isOpen
          variant="danger"
          title={t(SESSION_EDITOR_KEYS.discardTitle)}
          message={t(SESSION_EDITOR_KEYS.discardBody, { date })}
          confirmLabel={t(SESSION_EDITOR_KEYS.discardConfirm)}
          cancelLabel={t(SESSION_EDITOR_KEYS.keepEditing)}
          onClose={() => setConfirmingClose(false)}
          onConfirm={onClose}
        />
      )}
      {confirmingRemove && (
        <ConfirmDialog
          isOpen
          variant="danger"
          title={t(SESSION_EDITOR_KEYS.removeTitle)}
          message={t(SESSION_EDITOR_KEYS.removeBody, {
            workout: sessionName,
            date,
          })}
          confirmLabel={t(SESSION_EDITOR_KEYS.removeConfirm)}
          cancelLabel={t('common.cancel')}
          isLoading={remove.isPending}
          onClose={() => setConfirmingRemove(false)}
          onConfirm={() => void onRemove()}
        />
      )}
    </Sheet>
  );
}

/**
 * The quiet row an edited or added session opens with: that the coach
 * changed it, and "Undo your edits", described by what it puts back — or
 * that it takes an added session off.
 */
function EditedRow({
  added,
  note,
  restoring,
  onRestore,
}: {
  /** Whether the coach added the session, rather than edited one that was there. */
  added: boolean;
  note: string;
  restoring: boolean;
  onRestore: () => void;
}) {
  const { t } = useTranslation();
  const noteId = useId();
  return (
    <div
      className="flex items-start justify-between gap-3 rounded-lg bg-surface-container-low px-3 py-2.5"
      data-testid="session-editor-edited"
    >
      <div className="min-w-0 space-y-0.5">
        <p className="text-sm text-on-surface">
          {t(
            added
              ? SESSION_EDITOR_KEYS.addedNote
              : SESSION_EDITOR_KEYS.editedNote,
          )}
        </p>
        <p id={noteId} className="text-xs text-on-surface-variant">
          {note}
        </p>
      </div>
      <Button
        variant="tertiary"
        size="sm"
        className="-mr-1 shrink-0"
        loading={restoring}
        aria-describedby={noteId}
        onClick={onRestore}
        data-testid="session-editor-restore"
      >
        {t(SESSION_EDITOR_KEYS.restore)}
      </Button>
    </div>
  );
}
