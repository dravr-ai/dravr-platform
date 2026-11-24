// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach's editor for one session of a date in their athlete's plan — its title, sport, and the session written or built from a family
// ABOUTME: Edits a session, adds one to a date, or removes one; a write that finds its session moved says so and closes (carnet#1515, #1714, #1716, #1727)

import { useId, useMemo, useState, type KeyboardEvent } from 'react';
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
  familyBuildChanged,
  familySportFor,
  formatCivilDate,
  handDayRequest,
  handSessionChanged,
  handSessionComplete,
  handSessionMissing,
  handSessionOf,
  initialEditorMode,
  initialSessionBuild,
  isDateFull,
  isSessionMoved,
  newSessionDay,
  offeredFamilySport,
  pickSport,
  presetOf,
  replacementLine,
  restoreLine,
  unresolvedTargetLine,
  sessionExpect,
  setHand,
  sportChoices,
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
  useSessionPreview,
  useSubjectFamilies,
} from '../../hooks/useSessionEditor';
import {
  ALERT_TONE,
  Button,
  ConfirmDialog,
  SegmentedControl,
  Sheet,
  Switch,
  useErrorToast,
  useSuccessToast,
} from '../ui';
import { SessionEditorFamily } from './SessionEditorFamily';
import { SessionEditorHand } from './SessionEditorHand';
import { EditedRow, SessionEditorHeader } from './SessionEditorHeader';
import { SessionEditorSport } from './SessionEditorSport';

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

/** A refused save, as the footer keeps it: the server's sentence, for the session as it stood when refused. */
interface Refusal {
  text: string;
  hand: HandSession;
  build: SessionBuild | null;
  mode: EditorMode;
}

/**
 * The sheet an entry's "Edit day" (or "Edit session") opens, and the one
 * "Add a session" opens on a blank session after the date's entries. It
 * opens on the session's own words — a session a family built opens on its
 * family — with its title as the headline, then the sport chips, then how
 * to build it: written step by step, or from an evidence-backed family when
 * one is written for the sport. Each mode keeps what the coach did in it, so
 * switching loses nothing; Save (or Cmd/Ctrl+Enter) writes the mode on
 * screen into the coach's draft.
 *
 * Every write but the add names its session by position and as the card
 * showed it; one the server finds moved is never retried onto another
 * session: the sheet says so, the card reads the plan again, and it closes.
 *
 * The written mode never waits on the family catalogue: the catalogue is
 * read once the sheet opens, the family mode appears once it has a family
 * for the sport, and its loading, error and empty states live in that mode.
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
  const footerLineId = useId();
  const refusalId = useId();
  const restId = useId();
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
  // Built once from the opening session, its own word among them: built from
  // the live one, the chips would reorder under the cursor on every pick.
  const choices = useMemo(() => sportChoices(startHand.sport), [startHand]);
  const [hand, updateHand] = useState<HandSession>(startHand);
  // Whether the coach is naming a sport of their own under "Other…".
  const [otherOpen, setOtherOpen] = useState(false);

  const families = useSubjectFamilies(conversationId, language);
  const catalogue = families.data;
  const startBuild = useMemo(
    () => (catalogue ? initialSessionBuild(baseline, catalogue) : null),
    [catalogue, baseline],
  );
  const [familyEdit, setFamilyEdit] = useState<SessionBuild | null>(null);
  const build = familyEdit ?? startBuild;
  const preset = catalogue && build ? presetOf(catalogue, build) : undefined;
  // The family mode is offered only for a sport a family is written for, once
  // the catalogue has one; hidden while it loads. A catalogue that failed keeps
  // the control for a family sport, so a day that opened on its family can
  // still reach "Write" past the failure.
  const familySport = familySportFor(hand.sport, baseline);
  const offered = offeredFamilySport(catalogue, hand.sport, baseline);
  const modesShown =
    !hand.rest && (offered !== undefined || (families.isError && familySport !== undefined));
  // The mode on screen: the family mode only while it is offered, or while the
  // catalogue a family sport waits on is still out (loading or failed). A
  // catalogue that answered with no family for the sport falls back to "Write",
  // as the phone does, so a family day never strands the coach.
  const shown: EditorMode =
    mode === 'family' &&
    !hand.rest &&
    (offered !== undefined || (catalogue === undefined && familySport !== undefined))
      ? 'family'
      : 'session';
  // The head's minutes in the family mode: the measure the family form asks
  // for, read from the same cache once the build settles; never asked for
  // while the written mode is on screen.
  const preview = useSessionPreview(
    conversationId,
    day.date,
    shown === 'family' ? build : null,
    language,
  );

  const handChanged = handSessionChanged(baseline, hand);
  const buildChanged =
    startBuild !== null &&
    build !== null &&
    familyBuildChanged(baseline, startBuild, build);
  // What a close would lose, in either mode: the one rule web and mobile share.
  // Opening the family mode is not a change; picking in it is.
  const dirty = editorDirty(baseline, hand, startBuild, familyEdit);

  let next: DayReplacement | null = null;
  if (shown === 'session' && handChanged) {
    next = hand.rest
      ? { rest: true }
      : { rest: false, name: hand.workout.trim() };
  } else if (shown === 'family' && buildChanged && preset) {
    next = { rest: false, name: preset.name };
  }
  // An added session replaces nothing: it saves once it is whole, or once a
  // family build stands.
  const canSave = adding
    ? shown === 'session'
      ? handSessionComplete(hand)
      : build !== null
    : next !== null && (shown === 'family' || handSessionComplete(hand));
  // No sentence while a session has no title to name yet, nor on an add.
  const replacement = adding ? null : replacementLine(day, next);
  // While Save waits on the written session, the footer says what it waits for.
  const missing = shown === 'session' && !canSave ? handSessionMissing(hand) : null;
  const footerLine = missing ?? replacement;
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

  // A refused save stays in the footer until the coach changes what was refused.
  const [refusal, setRefusal] = useState<Refusal | null>(null);
  const refused =
    refusal !== null &&
    refusal.hand === hand &&
    refusal.build === build &&
    refusal.mode === shown
      ? refusal.text
      : null;

  const [confirmingClose, setConfirmingClose] = useState(false);
  const [confirmingRemove, setConfirmingRemove] = useState(false);
  const [confirmingRestore, setConfirmingRestore] = useState(false);
  // Cancel, the close button, Escape and the scrim all come through here.
  const requestClose = () => {
    if (dirty) setConfirmingClose(true);
    else onClose();
  };

  /**
   * One sport pick, for both modes: the hand session takes the word, the
   * family build follows it only when the pick moved it (an untouched family
   * mode never reads as a change), and a sport no family is written for
   * leaves the written mode on screen.
   */
  const pick = (sport: string) => {
    const picked = pickSport({ hand, build, mode }, sport, baseline, catalogue);
    updateHand(picked.hand);
    if (picked.build !== build && picked.build !== null) setFamilyEdit(picked.build);
    setMode(picked.mode);
  };

  const onRest = (rest: boolean) => {
    updateHand(setHand(hand, { rest }));
    // A rest day is written by the hand save: the family mode steps aside.
    if (rest) setMode('session');
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
          shown === 'session'
            ? { day: handDayRequest(hand) }
            : build
              ? { build }
              : null;
        if (session === null) return;
        await add.mutateAsync({ date: day.date, session });
      } else if (shown === 'session') {
        await saveHand.mutateAsync({ ...at, day: handDayRequest(hand) });
      } else if (build) {
        await saveFamily.mutateAsync({ ...at, build });
      }
      showSuccess(t(SESSION_EDITOR_KEYS.draftNote, { name: athleteName }));
      onClose();
    } catch (error) {
      if (closedOnMove(error)) return;
      // A step target the calendar cannot read, or an add onto a date filled
      // meanwhile, is said in the coach's language; any other refused save
      // names what it refused. That sentence is the toast's body, and stays
      // in the footer while the coach fixes it.
      const unresolved = unresolvedTargetLine(error);
      const text = unresolved
        ? t(unresolved.key, unresolved.values)
        : adding && isDateFull(error)
          ? t(SESSION_EDITOR_KEYS.dateFull, {
              date,
              max: String(MAX_SESSIONS_PER_DATE),
            })
          : describeApiError(error, { t, fallbackKey: API_ERROR_KEYS.unknown });
      setRefusal({ text, hand, build, mode: shown });
      showError(t(SESSION_EDITOR_KEYS.saveFailed), text);
    }
  };

  // Cmd/Ctrl+Enter saves from anywhere in the sheet's content, once Save would.
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== 'Enter' || !(event.metaKey || event.ctrlKey)) return;
    if (!canSave || saving) return;
    event.preventDefault();
    void onSave();
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
      setConfirmingRestore(false);
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

  const describedBy =
    [refused ? refusalId : null, footerLine ? footerLineId : null]
      .filter((id): id is string => id !== null)
      .join(' ') || undefined;

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
      <div className="flex min-h-full flex-col" onKeyDown={onKeyDown}>
        <div className="flex-1 space-y-5 px-4 pb-6 pt-1">
          {restoreNote && (
            <EditedRow
              added={day.added === true}
              note={t(restoreNote.key, restoreNote.values)}
              onRestore={() => setConfirmingRestore(true)}
            />
          )}

          {!hand.rest && (
            <SessionEditorHeader
              hand={hand}
              onTitle={(workout) => updateHand(setHand(hand, { workout }))}
              family={
                shown === 'family'
                  ? {
                      name: preset?.name ?? null,
                      sport: build?.sport ?? null,
                      minutes: preview.data?.duration_min ?? null,
                    }
                  : null
              }
            />
          )}

          {!adding && (
            <div className="flex min-h-11 items-center justify-between gap-3">
              <span id={restId} className="text-sm font-medium text-on-surface">
                {t(SESSION_EDITOR_KEYS.restDay)}
              </span>
              <Switch
                checked={hand.rest}
                onChange={onRest}
                aria-labelledby={restId}
                data-testid="session-editor-rest"
              />
            </div>
          )}

          {!hand.rest && (
            <SessionEditorSport
              sport={hand.sport}
              choices={choices}
              otherOpen={otherOpen}
              onPick={(sport) => {
                setOtherOpen(false);
                pick(sport);
              }}
              onOther={() => {
                if (otherOpen) return;
                setOtherOpen(true);
                // The field opens empty: what the coach types is the sport.
                pick('');
              }}
              onName={pick}
            />
          )}

          {modesShown && (
            <SegmentedControl
              value={shown}
              onChange={setMode}
              stretch
              aria-label={t(SESSION_EDITOR_KEYS.buildHow)}
              data-testid="session-editor-mode"
              options={[
                {
                  value: 'session',
                  label: t(SESSION_EDITOR_KEYS.modeWrite),
                  'data-testid': 'session-editor-mode-session',
                },
                {
                  value: 'family',
                  label: t(SESSION_EDITOR_KEYS.modeFamilyBacked),
                  'data-testid': 'session-editor-mode-family',
                },
              ]}
            />
          )}

          {!hand.rest &&
            (shown === 'session' ? (
              <SessionEditorHand
                hand={hand}
                familySport={familySport}
                onChange={updateHand}
              />
            ) : (
              <SessionEditorFamily
                conversationId={conversationId}
                date={day.date}
                athleteName={athleteName}
                catalogue={catalogue}
                catalogueFailed={families.isError}
                build={build}
                onChange={setFamilyEdit}
              />
            ))}
        </div>

        {/* The sentence has the footer's width to itself: it names the sessions
            a save swaps, and a name broken over two lines reads as two. */}
        <div className="sticky bottom-0 space-y-2 border-t ghost-border-faint bg-surface px-4 py-3">
          {refused && (
            <p
              id={refusalId}
              role="alert"
              className={`${ALERT_TONE.error.tone} rounded-lg px-3 py-2 text-sm`}
              data-testid="session-editor-refusal"
            >
              {refused}
            </p>
          )}
          {footerLine && (
            <p
              id={footerLineId}
              className="text-pretty text-sm text-on-surface-variant"
              data-testid={missing ? 'session-editor-missing' : 'session-editor-replacement'}
            >
              {t(footerLine.key, footerLine.values)}
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
                aria-describedby={describedBy}
                aria-keyshortcuts="Meta+Enter Control+Enter"
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
      {(confirmingRemove || (confirmingRestore && day.added === true)) && (
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
          isLoading={confirmingRemove ? remove.isPending : restore.isPending}
          onClose={() => {
            setConfirmingRemove(false);
            setConfirmingRestore(false);
          }}
          // Taking back an added session is the restore: it removes it as the card showed it.
          onConfirm={() => void (confirmingRemove ? onRemove() : onRestore())}
        />
      )}
      {confirmingRestore && day.added !== true && restoreNote && (
        <ConfirmDialog
          isOpen
          variant="danger"
          title={t(SESSION_EDITOR_KEYS.restoreTitle)}
          message={t(restoreNote.key, restoreNote.values)}
          confirmLabel={t(SESSION_EDITOR_KEYS.discardConfirm)}
          cancelLabel={t(SESSION_EDITOR_KEYS.keepEditing)}
          isLoading={restore.isPending}
          onClose={() => setConfirmingRestore(false)}
          onConfirm={() => void onRestore()}
        />
      )}
    </Sheet>
  );
}
