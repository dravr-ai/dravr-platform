// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach's day editor on the phone — a sheet over the plan thread that edits one session of a date, or adds one to it
// ABOUTME: One sport row for both modes, a footer that names what a save replaces or what it waits for; a dirty sheet asks before it closes (carnet#1515, #1714, #1716, #1727)

import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  ActivityIndicator,
  Alert,
  KeyboardAvoidingView,
  Platform,
  Pressable,
  ScrollView,
  Switch,
  Text,
  View,
} from 'react-native';
import { Feather } from '@expo/vector-icons';
import Toast from 'react-native-toast-message';
import { useTranslation } from '@pierre/i18n';
import type { PlanDay, SessionBuild } from '@pierre/shared-types';
import {
  API_ERROR_KEYS,
  canRemoveSession,
  dayPosition,
  type DayReplacement,
  describeApiError,
  editorDirty,
  type EditorMode,
  familyBuildChanged,
  familySportFor,
  formatCivilDate,
  handDayRequest,
  handSessionChanged,
  handSessionMissing,
  handSessionOf,
  initialEditorMode,
  initialSessionBuild,
  isDateFull,
  isSessionMoved,
  MAX_SESSIONS_PER_DATE,
  newSessionDay,
  offeredFamilySport,
  pickSport,
  presetOf,
  replacementLine,
  restoreLine,
  unresolvedTargetLine,
  ROW_DATE,
  SESSION_EDITOR_KEYS,
  type SessionAt,
  sessionExpect,
  setHand,
  sportChoices,
  type Translate,
} from '@pierre/ui-logic';
import { useThemeColors } from '../../constants/theme';
import { ALERT_TONE, alertInkColor, Button, RadioGroup, Sheet, switchColors } from '../../components/ui';
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
import { FamilyTitleHeader, SessionTitleHeader, SportChips } from './SessionEditorHeader';
import { SessionFamilyForm } from './SessionFamilyForm';
import { SessionHandForm } from './SessionHandForm';
import type { SessionEditorTarget } from './WorkoutPlanCard';

interface SessionEditorSheetProps {
  /** The coach's plan thread on the athlete. */
  conversationId: string;
  /** The athlete the thread is about, as its header names them. */
  athleteName: string;
  /** The entry to edit, or the date to add a session to; `null` while the sheet is closed. */
  target: SessionEditorTarget | null;
  onClose: () => void;
}

/**
 * The editor over a coach's plan thread: one session of a date, edited as
 * its own session or built from a workout family, or a new session added to
 * the date — saved into the coach's draft. The editor starts afresh on every
 * opening, from what the entry holds or from a blank session.
 *
 * Cancel, the back gesture and a tap on the scrim all go through one guard:
 * a sheet holding a change the coach has not saved asks before it closes.
 * A save, a restore or a remove closes it without asking — there is nothing
 * left to lose.
 */
export default function SessionEditorSheet({
  conversationId,
  athleteName,
  target,
  onClose,
}: SessionEditorSheetProps) {
  const { t, language } = useTranslation();
  // Whether the open editor holds a change the coach has not saved; the
  // editor reports it, the guard reads it when a close is asked for.
  const dirty = useRef(false);
  const onDirtyChange = useCallback((next: boolean) => {
    dirty.current = next;
  }, []);

  const requestClose = useCallback(() => {
    if (!dirty.current || target === null) {
      onClose();
      return;
    }
    Alert.alert(
      t(SESSION_EDITOR_KEYS.discardTitle),
      t(SESSION_EDITOR_KEYS.discardBody, {
        date: formatCivilDate(target.day.date, language, ROW_DATE),
      }),
      [
        { text: t(SESSION_EDITOR_KEYS.keepEditing), style: 'cancel' },
        {
          text: t(SESSION_EDITOR_KEYS.discardConfirm),
          style: 'destructive',
          onPress: onClose,
        },
      ],
    );
  }, [target, language, onClose, t]);

  return (
    <Sheet
      visible={target !== null}
      onClose={requestClose}
      flush
      testID="session-editor-sheet"
      backdropTestID="session-editor-scrim"
    >
      {target !== null ? (
        <SessionEditor
          key={`${target.mode}-${target.day.date}-${dayPosition(target.day)}`}
          conversationId={conversationId}
          athleteName={athleteName}
          target={target}
          onClose={onClose}
          requestClose={requestClose}
          onDirtyChange={onDirtyChange}
        />
      ) : null}
    </Sheet>
  );
}

interface SessionEditorProps {
  conversationId: string;
  athleteName: string;
  target: SessionEditorTarget;
  /** Closes the sheet at once: after a save, a restore or a remove. */
  onClose: () => void;
  /** Closes the sheet through the unsaved-changes guard: Cancel. */
  requestClose: () => void;
  onDirtyChange: (dirty: boolean) => void;
}

/**
 * The sheet's content, top to bottom: the edited-session row, the session's
 * title over its summary line, the Rest switch, the sport chips, "Write" /
 * "From an evidence-backed family" when a family is written for the sport,
 * the mode's form, "Remove this session" on a date that holds more than one,
 * and the footer naming what a save replaces — or what it still waits for —
 * over Cancel and Save. One sport pick moves both modes; each mode otherwise
 * keeps its own state, so switching back finds the coach's work as they left
 * it. A family value outside its tested range is saved as typed and earns a
 * finding; Save is never held back for one (D3).
 *
 * Adding a session opens on a blank one in the sport of the date's first
 * session: no Rest switch — a date's rest is changed on its own entry — and
 * no replacement sentence, since a new session replaces nothing.
 *
 * Every write but the add names its session by position and as the card
 * showed it. A write refused because that session moved (409
 * `session_moved`) is never retried onto another: the card's plan is read
 * again, the sheet closes, and the coach is told why.
 */
function SessionEditor({
  conversationId,
  athleteName,
  target,
  onClose,
  requestClose,
  onDirtyChange,
}: SessionEditorProps) {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  const adding = target.mode === 'add';
  // What the editor starts from, and measures a change against: the entry
  // itself, or the blank session a new one starts as.
  const day = useMemo(() => (adding ? newSessionDay(target.day.date, target.entries) : target.day), [adding, target]);
  // Which session of the date a write is about, as the card showed it.
  const at = useMemo<SessionAt>(
    () => ({ date: day.date, position: dayPosition(day), expect: sessionExpect(day) }),
    [day],
  );
  const date = formatCivilDate(day.date, language, ROW_DATE);
  const [mode, setMode] = useState<EditorMode>(() => initialEditorMode(day));

  // "Write": the entry's own session, as the coach edits it by hand. The
  // sport chips are drawn from the opening session, so a pick never reorders
  // them under the coach's thumb.
  const openingHand = useMemo(() => handSessionOf(day), [day]);
  const choices = useMemo(() => sportChoices(openingHand.sport), [openingHand]);
  const [hand, updateHand] = useState(openingHand);
  // Whether the coach is naming a sport of their own under "Other…".
  const [otherOpen, setOtherOpen] = useState(false);

  // "From a family": the build the entry opens on, until the coach changes it.
  const families = useSubjectFamilies(conversationId, language);
  const catalogue = families.data;
  const opening = useMemo(() => (catalogue ? initialSessionBuild(day, catalogue) : null), [day, catalogue]);
  const [edited, setEdited] = useState<SessionBuild | null>(null);
  const build = edited ?? opening;
  const preset = catalogue && build ? presetOf(catalogue, build) : undefined;

  // The family mode is offered only for a sport the catalogue has a family
  // written for, with no note otherwise. While the catalogue is read, a day
  // a family built stays on its family's loading state; if the read fails,
  // the switch stays within reach for a family sport so the coach can still
  // write the session by hand.
  const familyWord = familySportFor(hand.sport, day);
  const familySport = offeredFamilySport(catalogue, hand.sport, day);
  const catalogueOut = catalogue === undefined && familyWord !== undefined;
  const modeOffered = !hand.rest && (familySport !== undefined || (families.isError && familyWord !== undefined));
  const shown: EditorMode = mode === 'family' && !hand.rest && (familySport !== undefined || catalogueOut) ? 'family' : 'session';

  // Measured only while the family mode is on screen: a hand edit has no preview.
  const preview = useSessionPreview(conversationId, day.date, shown === 'family' ? build : null, language);

  const handChanged = handSessionChanged(day, hand);
  const familyChanged = build !== null && opening !== null && familyBuildChanged(day, opening, build);
  let next: DayReplacement | null;
  if (shown === 'session') {
    next = !handChanged ? null : hand.rest ? { rest: true } : { rest: false, name: hand.workout.trim() };
  } else {
    next = familyChanged && preset ? { rest: false, name: preset.name } : null;
  }
  // A new session replaces nothing, so it has no sentence to say.
  const replacement = adding ? null : replacementLine(day, next);
  // What the written session still lacks: said in the footer while Save waits.
  const missing = shown === 'session' ? handSessionMissing(hand) : null;
  const missingText = missing ? t(missing.key, missing.values) : null;

  // A change the coach made in either mode; a family build the coach never
  // touched is not one, even on an entry no family built.
  const dirty = editorDirty(day, hand, opening, edited);
  useEffect(() => onDirtyChange(dirty), [dirty, onDirtyChange]);
  useEffect(() => () => onDirtyChange(false), [onDirtyChange]);

  const saveFamily = useSaveDraftDay(conversationId, language);
  const saveHand = useSaveDraftHandDay(conversationId, language);
  const add = useAddDraftSession(conversationId, language);
  const reread = useRereadPlan(conversationId);
  const saving = saveFamily.isPending || saveHand.isPending || add.isPending;
  // A new session saves once it is whole — in the family mode, once a build
  // exists; an edit saves once it changes something and is whole.
  const canSave = adding
    ? shown === 'family'
      ? build !== null
      : missing === null
    : next !== null && (shown === 'family' || missing === null);
  // Why the last save was refused, in the coach's words; `null` until one is.
  const [saveRefusal, setSaveRefusal] = useState<string | null>(null);

  // One sport pick for both modes: the hand session takes the word, the
  // family build follows it only when it moved (an untouched family mode
  // never reads as a change), and a sport no family is written for closes
  // the family mode.
  const pick = (sport: string) => {
    const picked = pickSport({ hand, build, mode }, sport, day, catalogue);
    updateHand(picked.hand);
    if (picked.build !== build) setEdited(picked.build);
    setMode(picked.mode);
  };
  const pickChip = (sport: string) => {
    setOtherOpen(false);
    pick(sport);
  };
  // "Other…" opens an empty field whose text is the sport word itself.
  const openOther = () => {
    if (otherOpen) return;
    setOtherOpen(true);
    pick('');
  };
  // A rest day is a session the coach writes, never a family build.
  const setRest = (rest: boolean) => {
    updateHand(setHand(hand, { rest }));
    if (rest) setMode('session');
  };

  const saved = () => {
    onClose();
    Toast.show({
      type: 'success',
      text1: t(SESSION_EDITOR_KEYS.draftNote, { name: athleteName }),
    });
  };
  // The session is no longer where the card showed it. Said once the sheet
  // has closed onto the plan read again: a toast is drawn under an open
  // modal and nobody would see it.
  const moved = () => {
    reread();
    onClose();
    Toast.show({ type: 'info', text1: t(SESSION_EDITOR_KEYS.sessionMoved) });
  };
  // Any other refusal is said in the sheet, which stays open for another try:
  // a step target the calendar cannot read names its step and target, and an
  // add on a date another write filled meanwhile says the date is full.
  const refused = (error: unknown) => {
    if (isSessionMoved(error)) {
      moved();
      return;
    }
    const unresolved = unresolvedTargetLine(error);
    setSaveRefusal(
      unresolved
        ? t(unresolved.key, unresolved.values)
        : isDateFull(error)
          ? t(SESSION_EDITOR_KEYS.dateFull, {
              date,
              max: String(MAX_SESSIONS_PER_DATE),
            })
          : describeApiError(error, { t, fallbackKey: API_ERROR_KEYS.unknown }),
    );
  };

  const onSave = () => {
    if (!canSave) return;
    setSaveRefusal(null);
    const settle = { onSuccess: saved, onError: refused };
    if (adding) {
      if (shown === 'session') {
        add.mutate({ date: day.date, session: { day: handDayRequest(hand) } }, settle);
      } else if (build !== null) {
        add.mutate({ date: day.date, session: { build } }, settle);
      }
    } else if (shown === 'session') {
      saveHand.mutate({ ...at, day: handDayRequest(hand) }, settle);
    } else if (build !== null) {
      saveFamily.mutate({ ...at, build }, settle);
    }
  };

  return (
    <KeyboardAvoidingView className="shrink" behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
      <Text
        className="mb-3 px-4 text-lg font-semibold text-text-primary"
        accessibilityRole="header"
        testID="session-editor-title"
      >
        {t(adding ? SESSION_EDITOR_KEYS.addTitle : SESSION_EDITOR_KEYS.editTitle, { date })}
      </Text>
      <ScrollView keyboardShouldPersistTaps="handled" showsVerticalScrollIndicator={false}>
        {!adding && (day.edited || day.added) ? (
          <EditedNotice
            conversationId={conversationId}
            day={day}
            at={at}
            date={date}
            onRestored={onClose}
            onMoved={moved}
          />
        ) : null}
        {hand.rest ? null : shown === 'family' ? (
          <FamilyTitleHeader presetName={preset?.name} sport={hand.sport} previewMinutes={preview.data?.duration_min} />
        ) : (
          <SessionTitleHeader session={hand} onTitle={(workout) => updateHand(setHand(hand, { workout }))} />
        )}
        {adding ? null : (
          <View className="mb-2 min-h-11 flex-row items-center justify-between gap-4 px-4">
            <Text className="flex-1 text-base text-text-primary">{t(SESSION_EDITOR_KEYS.restDay)}</Text>
            <Switch
              value={hand.rest}
              onValueChange={setRest}
              accessibilityLabel={t(SESSION_EDITOR_KEYS.restDay)}
              {...switchColors(colors.tokens, hand.rest)}
              testID="session-editor-rest"
            />
          </View>
        )}
        {hand.rest ? null : (
          <>
            <SportChips
              choices={choices}
              sport={hand.sport}
              otherOpen={otherOpen}
              onPick={pickChip}
              onOther={openOther}
              onOtherName={pick}
            />
            {modeOffered ? (
              <View className="mb-5 px-4">
                <RadioGroup
                  layout="segmented"
                  accessibilityLabel={t(SESSION_EDITOR_KEYS.buildHow)}
                  options={[
                    { key: 'session', label: t(SESSION_EDITOR_KEYS.modeWrite) },
                    { key: 'family', label: t(SESSION_EDITOR_KEYS.modeFamilyBacked) },
                  ]}
                  value={shown}
                  onChange={(key) => setMode(key === 'family' ? 'family' : 'session')}
                  testID="session-editor-mode"
                />
              </View>
            ) : null}
            {shown === 'session' ? (
              <SessionHandForm session={hand} familySport={familyWord} onChange={updateHand} />
            ) : (
              <SessionFamilyForm
                athleteName={athleteName}
                catalogue={catalogue}
                loadFailed={families.isError}
                build={build}
                preview={preview.data}
                measuring={preview.isFetching}
                previewFailed={preview.isError && !preview.isFetching}
                onChange={setEdited}
              />
            )}
          </>
        )}
        {!adding && canRemoveSession(target.entries) ? (
          <RemoveSession
            conversationId={conversationId}
            day={day}
            at={at}
            date={date}
            onRemoved={onClose}
            onMoved={moved}
          />
        ) : null}
      </ScrollView>
      <View className="mt-3 border-t border-border-faint px-4 pt-3">
        {missingText !== null ? (
          <Text
            className="mb-3 text-sm text-text-secondary"
            accessibilityLiveRegion="polite"
            testID="session-editor-missing"
          >
            {missingText}
          </Text>
        ) : replacement !== null ? (
          <Text
            className="mb-3 text-sm text-text-secondary"
            accessibilityLiveRegion="polite"
            testID="session-editor-replacement"
          >
            {t(replacement.key, replacement.values)}
          </Text>
        ) : null}
        {saveRefusal !== null ? (
          <View
            className={`mb-3 rounded-lg px-3 py-2 ${ALERT_TONE.error.fill}`}
            accessibilityRole="alert"
            accessibilityLiveRegion="polite"
            testID="session-editor-save-failed"
          >
            <Text className={`text-sm font-semibold ${ALERT_TONE.error.ink}`}>{t(SESSION_EDITOR_KEYS.saveFailed)}</Text>
            <Text className={`mt-0.5 text-sm ${ALERT_TONE.error.ink}`}>{saveRefusal}</Text>
          </View>
        ) : null}
        <View className="flex-row gap-3">
          <View className="flex-1">
            <Button
              title={t(SESSION_EDITOR_KEYS.cancel)}
              variant="ghost"
              onPress={requestClose}
              fullWidth
              testID="session-editor-cancel"
            />
          </View>
          <View className="flex-1">
            <Button
              title={saving ? t(SESSION_EDITOR_KEYS.saving) : t(SESSION_EDITOR_KEYS.save)}
              onPress={onSave}
              disabled={!canSave || saving}
              // A Save that waits says why, as the footer's sentence does.
              accessibilityHint={missingText ?? undefined}
              fullWidth
              testID="session-editor-save"
            />
          </View>
        </View>
      </View>
    </KeyboardAvoidingView>
  );
}

/** The name a session goes by in a sentence: its title, or "Rest" for a rest entry. */
function sessionName(t: Translate, day: PlanDay): string {
  return day.rest ? t('chat.restDay') : day.workout;
}

interface EditedNoticeProps {
  conversationId: string;
  /** The entry, edited or added by the coach. */
  day: PlanDay;
  /** Which session of the date the restore is about, as the card showed it. */
  at: SessionAt;
  /** The entry's date as the sheet's title writes it. */
  date: string;
  onRestored: () => void;
  /** The session moved since the card showed it: the sheet reads the plan again and closes. */
  onMoved: () => void;
}

/**
 * The quiet row an edited or added session opens with: that the coach
 * edited it, and "Back to the original" with what it puts back — or, on a
 * session the coach added, that going back removes it. It asks first: a
 * restore throws away every edit, and a session the coach added is asked
 * about as its remove is. A restore that lands closes the sheet and says so;
 * one refused is said here, and the sheet stays open.
 */
function EditedNotice({
  conversationId,
  day,
  at,
  date,
  onRestored,
  onMoved,
}: EditedNoticeProps) {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  const restore = useRestoreDraftDay(conversationId, language);
  const [refusal, setRefusal] = useState<string | null>(null);
  const line = restoreLine(day);
  const note = line ? t(line.key, line.values) : undefined;

  const run = () => {
    setRefusal(null);
    restore.mutate(at, {
      onSuccess: () => {
        onRestored();
        Toast.show({
          type: 'success',
          // Restoring a session the coach added takes it off the date.
          text1: day.added
            ? t(SESSION_EDITOR_KEYS.removed, {
                workout: sessionName(t, day),
                date,
              })
            : t(SESSION_EDITOR_KEYS.restored, { date }),
        });
      },
      onError: (error) => {
        if (isSessionMoved(error)) {
          onMoved();
          return;
        }
        setRefusal(
          describeApiError(error, { t, fallbackKey: API_ERROR_KEYS.unknown }),
        );
      },
    });
  };

  const onRestore = () => {
    if (day.added) {
      const workout = sessionName(t, day);
      Alert.alert(t(SESSION_EDITOR_KEYS.removeTitle), t(SESSION_EDITOR_KEYS.removeBody, { workout, date }), [
        { text: t(SESSION_EDITOR_KEYS.cancel), style: 'cancel' },
        { text: t(SESSION_EDITOR_KEYS.removeConfirm), style: 'destructive', onPress: run },
      ]);
      return;
    }
    Alert.alert(t(SESSION_EDITOR_KEYS.restoreTitle), note, [
      { text: t(SESSION_EDITOR_KEYS.keepEditing), style: 'cancel' },
      { text: t(SESSION_EDITOR_KEYS.discardConfirm), style: 'destructive', onPress: run },
    ]);
  };

  return (
    <View className="mb-4 px-4" testID="session-editor-edited">
      <Text className="text-sm text-text-secondary">
        {t(day.added ? SESSION_EDITOR_KEYS.addedNote : SESSION_EDITOR_KEYS.editedNote)}
      </Text>
      <Pressable
        className={`min-h-11 flex-row items-center self-start ${restore.isPending ? 'opacity-50' : ''}`}
        onPress={onRestore}
        disabled={restore.isPending}
        accessibilityRole="button"
        accessibilityLabel={t(SESSION_EDITOR_KEYS.restoreOriginal)}
        accessibilityHint={note}
        accessibilityState={{
          disabled: restore.isPending,
          busy: restore.isPending,
        }}
        testID="session-editor-restore"
      >
        <Feather name="rotate-ccw" size={14} color={colors.tokens.primary} />
        <Text className="ml-1.5 text-sm font-medium text-primary">
          {t(SESSION_EDITOR_KEYS.restoreOriginal)}
        </Text>
        {restore.isPending ? (
          <View className="ml-2">
            <ActivityIndicator size="small" color={colors.tokens.primary} />
          </View>
        ) : null}
      </Pressable>
      {note ? (
        <Text
          className="-mt-2 text-sm text-text-secondary"
          testID="session-editor-restore-note"
        >
          {note}
        </Text>
      ) : null}
      {refusal !== null ? (
        <View
          className={`mt-2 rounded-lg px-3 py-2 ${ALERT_TONE.error.fill}`}
          accessibilityRole="alert"
          accessibilityLiveRegion="polite"
          testID="session-editor-restore-failed"
        >
          <Text className={`text-sm font-semibold ${ALERT_TONE.error.ink}`}>
            {t(SESSION_EDITOR_KEYS.restoreFailed)}
          </Text>
          <Text className={`mt-0.5 text-sm ${ALERT_TONE.error.ink}`}>
            {refusal}
          </Text>
        </View>
      ) : null}
    </View>
  );
}

interface RemoveSessionProps {
  conversationId: string;
  day: PlanDay;
  /** Which session of the date the remove is about, as the card showed it. */
  at: SessionAt;
  /** The entry's date as the sheet's title writes it. */
  date: string;
  onRemoved: () => void;
  /** The session moved since the card showed it: the sheet reads the plan again and closes. */
  onMoved: () => void;
}

/**
 * "Remove this session", at the foot of the sheet's content, offered only on
 * a date that holds more than one entry: a date's last entry is changed with
 * the Rest switch, never removed. It asks first, naming the session and the
 * date; a remove that lands closes the sheet and says so, one refused is said
 * here and the sheet stays open.
 */
function RemoveSession({
  conversationId,
  day,
  at,
  date,
  onRemoved,
  onMoved,
}: RemoveSessionProps) {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  const remove = useRemoveDraftSession(conversationId, language);
  const [refusal, setRefusal] = useState<string | null>(null);
  const workout = sessionName(t, day);

  const run = () => {
    setRefusal(null);
    remove.mutate(at, {
      onSuccess: () => {
        onRemoved();
        Toast.show({
          type: 'success',
          text1: t(SESSION_EDITOR_KEYS.removed, { workout, date }),
        });
      },
      onError: (error) => {
        if (isSessionMoved(error)) {
          onMoved();
          return;
        }
        setRefusal(
          describeApiError(error, { t, fallbackKey: API_ERROR_KEYS.unknown }),
        );
      },
    });
  };

  const confirm = () =>
    Alert.alert(
      t(SESSION_EDITOR_KEYS.removeTitle),
      t(SESSION_EDITOR_KEYS.removeBody, { workout, date }),
      [
        { text: t(SESSION_EDITOR_KEYS.cancel), style: 'cancel' },
        {
          text: t(SESSION_EDITOR_KEYS.removeConfirm),
          style: 'destructive',
          onPress: run,
        },
      ],
    );

  return (
    <View className="mt-6 px-4">
      <Pressable
        className={`min-h-11 flex-row items-center self-start ${remove.isPending ? 'opacity-50' : ''}`}
        onPress={confirm}
        disabled={remove.isPending}
        accessibilityRole="button"
        accessibilityLabel={t(SESSION_EDITOR_KEYS.removeSession)}
        accessibilityState={{
          disabled: remove.isPending,
          busy: remove.isPending,
        }}
        testID="session-editor-remove"
      >
        <Feather
          name="trash-2"
          size={14}
          color={alertInkColor('error', colors)}
        />
        <Text className={`ml-1.5 text-sm font-medium ${ALERT_TONE.error.ink}`}>
          {t(SESSION_EDITOR_KEYS.removeSession)}
        </Text>
        {remove.isPending ? (
          <View className="ml-2">
            <ActivityIndicator
              size="small"
              color={alertInkColor('error', colors)}
            />
          </View>
        ) : null}
      </Pressable>
      {refusal !== null ? (
        <View
          className={`mt-2 rounded-lg px-3 py-2 ${ALERT_TONE.error.fill}`}
          accessibilityRole="alert"
          accessibilityLiveRegion="polite"
          testID="session-editor-remove-failed"
        >
          <Text className={`text-sm font-semibold ${ALERT_TONE.error.ink}`}>
            {t(SESSION_EDITOR_KEYS.removeFailed)}
          </Text>
          <Text className={`mt-0.5 text-sm ${ALERT_TONE.error.ink}`}>
            {refusal}
          </Text>
        </View>
      ) : null}
    </View>
  );
}
