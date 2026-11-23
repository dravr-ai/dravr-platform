// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach's day editor on the phone — a sheet over the plan thread that edits one session of a date, or adds one to it
// ABOUTME: The footer names what a save replaces, a dirty sheet asks before it closes; a session restores, a date's extra one removes (carnet#1515, #1714, #1716)

import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import {
  ActivityIndicator,
  Alert,
  KeyboardAvoidingView,
  Platform,
  Pressable,
  ScrollView,
  Text,
  View,
} from 'react-native';
import { Feather } from '@expo/vector-icons';
import Toast from 'react-native-toast-message';
import { useTranslation } from '@pierre/i18n';
import type {
  FamilyCatalogueResponse,
  PlanDay,
  SessionBuild,
  SessionParamKey,
  SessionParamRange,
  SessionPreview,
  SessionSport,
} from '@pierre/shared-types';
import {
  API_ERROR_KEYS,
  canRemoveSession,
  catalogueSports,
  dayPosition,
  type DayReplacement,
  derivedRows,
  describeApiError,
  editableParams,
  editorDirty,
  type EditorMode,
  familiesForSport,
  familyBuildChanged,
  familyMissingForDaySport,
  familyOf,
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
  MAX_SESSIONS_PER_DATE,
  flaggedParams,
  newSessionDay,
  paramValue,
  presetOf,
  presetsForSport,
  replacementLine,
  restoreLine,
  ROW_DATE,
  selectBuildSport,
  selectFamily,
  selectPreset,
  SESSION_EDITOR_KEYS,
  SESSION_PARAM_LABEL_KEY,
  type SessionAt,
  sessionExpect,
  sessionSportLabelKey,
  setParam,
  type Translate,
} from '@pierre/ui-logic';
import { useThemeColors } from '../../constants/theme';
import {
  ALERT_TONE,
  alertInkColor,
  Badge,
  Button,
  Input,
  RadioGroup,
  Sheet,
} from '../../components/ui';
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
import { EvidenceChip } from './EvidenceChip';
import { SessionHandForm } from './SessionHandForm';
import { Figure, type SessionEditorTarget, StepText } from './WorkoutPlanCard';

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
 * The sheet's content: the edited-session row, the mode switch, the mode's
 * form, "Remove this session" on a date that holds more than one, and the
 * footer naming what a save replaces over Cancel and Save. Each mode keeps
 * its own state, so switching back finds the coach's work as they left it.
 * A family value outside its tested range is saved as typed and earns a
 * finding; Save is never held back for one (D3).
 *
 * Adding a session opens on a blank one in the sport of the date's first
 * session: no Rest switch — a date's rest is changed on its own entry — and
 * no footer sentence, since a new session replaces nothing.
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
  const adding = target.mode === 'add';
  // What the editor starts from, and measures a change against: the entry
  // itself, or the blank session a new one starts as.
  const day = useMemo(
    () =>
      adding ? newSessionDay(target.day.date, target.entries) : target.day,
    [adding, target],
  );
  // Which session of the date a write is about, as the card showed it.
  const at = useMemo<SessionAt>(
    () => ({
      date: day.date,
      position: dayPosition(day),
      expect: sessionExpect(day),
    }),
    [day],
  );
  const date = formatCivilDate(day.date, language, ROW_DATE);
  const [mode, setMode] = useState<EditorMode>(() => initialEditorMode(day));

  // "This session": the entry's own session, as the coach edits it by hand.
  // The sport row is drawn from the opening session, so a pick never
  // reorders it under the coach's thumb.
  const openingHand = useMemo(() => handSessionOf(day), [day]);
  const handSports = useMemo(
    () => handSportOptions(openingHand),
    [openingHand],
  );
  const [hand, setHand] = useState(openingHand);

  // "From a family": the build the entry opens on, until the coach changes it.
  const families = useSubjectFamilies(conversationId, language);
  const catalogue = families.data;
  const opening = useMemo(
    () => (catalogue ? initialSessionBuild(day, catalogue) : null),
    [day, catalogue],
  );
  const [edited, setEdited] = useState<SessionBuild | null>(null);
  const build = edited ?? opening;
  const preset = catalogue && build ? presetOf(catalogue, build) : undefined;
  // Measured only while the family mode is on screen: a hand edit has no preview.
  const preview = useSessionPreview(
    conversationId,
    day.date,
    mode === 'family' ? build : null,
    language,
  );

  const handChanged = handSessionChanged(day, hand);
  const familyChanged =
    build !== null &&
    opening !== null &&
    familyBuildChanged(day, opening, build);
  let next: DayReplacement | null;
  if (mode === 'session') {
    next = !handChanged
      ? null
      : hand.rest
        ? { rest: true }
        : { rest: false, name: hand.workout.trim() };
  } else {
    next = familyChanged && preset ? { rest: false, name: preset.name } : null;
  }
  // A new session replaces nothing, so it has no sentence to say.
  const replacement = adding ? null : replacementLine(day, next);

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
    ? mode === 'family'
      ? build !== null
      : handSessionComplete(hand)
    : next !== null && (mode === 'family' || handSessionComplete(hand));
  // Why the last save was refused, in the coach's words; `null` until one is.
  const [saveRefusal, setSaveRefusal] = useState<string | null>(null);

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
  // an add on a date another write filled meanwhile says the date is full.
  const refused = (error: unknown) => {
    if (isSessionMoved(error)) {
      moved();
      return;
    }
    setSaveRefusal(
      isDateFull(error)
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
      if (mode === 'session') {
        add.mutate(
          { date: day.date, session: { day: handDayRequest(hand) } },
          settle,
        );
      } else if (build !== null) {
        add.mutate({ date: day.date, session: { build } }, settle);
      }
    } else if (mode === 'session') {
      saveHand.mutate({ ...at, day: handDayRequest(hand) }, settle);
    } else if (build !== null) {
      saveFamily.mutate({ ...at, build }, settle);
    }
  };

  return (
    <KeyboardAvoidingView
      className="shrink"
      behavior={Platform.OS === 'ios' ? 'padding' : undefined}
    >
      <Text
        className="mb-3 px-4 text-lg font-semibold text-text-primary"
        accessibilityRole="header"
        testID="session-editor-title"
      >
        {t(
          adding ? SESSION_EDITOR_KEYS.addTitle : SESSION_EDITOR_KEYS.editTitle,
          { date },
        )}
      </Text>
      <ScrollView
        keyboardShouldPersistTaps="handled"
        showsVerticalScrollIndicator={false}
      >
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
        <View className="mb-5 px-4">
          <RadioGroup
            layout="segmented"
            options={[
              { key: 'session', label: t(SESSION_EDITOR_KEYS.modeSession) },
              { key: 'family', label: t(SESSION_EDITOR_KEYS.modeFamily) },
            ]}
            value={mode}
            onChange={(key) => setMode(key === 'family' ? 'family' : 'session')}
            testID="session-editor-mode"
          />
        </View>
        {mode === 'session' ? (
          <SessionHandForm
            session={hand}
            sports={handSports}
            restOffered={!adding}
            onChange={setHand}
          />
        ) : (
          <FamilyPane
            athleteName={athleteName}
            catalogue={catalogue}
            loadFailed={families.isError}
            build={build}
            missingSport={
              catalogue ? familyMissingForDaySport(day, catalogue) : null
            }
            preview={preview.data}
            measuring={preview.isFetching}
            previewFailed={preview.isError && !preview.isFetching}
            onChange={setEdited}
          />
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
        {replacement !== null ? (
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
            <Text className={`text-sm font-semibold ${ALERT_TONE.error.ink}`}>
              {t(SESSION_EDITOR_KEYS.saveFailed)}
            </Text>
            <Text className={`mt-0.5 text-sm ${ALERT_TONE.error.ink}`}>
              {saveRefusal}
            </Text>
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
              title={
                saving
                  ? t(SESSION_EDITOR_KEYS.saving)
                  : t(SESSION_EDITOR_KEYS.save)
              }
              onPress={onSave}
              disabled={!canSave || saving}
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
 * edited it, and "Undo your edits" with what it puts back — or, on a session
 * the coach added, that undoing removes it. A restore that lands closes the
 * sheet and says so; one refused is said here, and the sheet stays open.
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

  const onRestore = () => {
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
        accessibilityLabel={t(SESSION_EDITOR_KEYS.restore)}
        accessibilityHint={note}
        accessibilityState={{
          disabled: restore.isPending,
          busy: restore.isPending,
        }}
        testID="session-editor-restore"
      >
        <Feather name="rotate-ccw" size={14} color={colors.tokens.primary} />
        <Text className="ml-1.5 text-sm font-medium text-primary">
          {t(SESSION_EDITOR_KEYS.restore)}
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

interface FamilyPaneProps {
  athleteName: string;
  /** The athlete's families; `undefined` until the read answers. */
  catalogue: FamilyCatalogueResponse | undefined;
  loadFailed: boolean;
  build: SessionBuild | null;
  /** The day's sport when no family has a form in it; the build then opens in another sport. */
  missingSport: SessionSport | null;
  preview: SessionPreview | undefined;
  measuring: boolean;
  previewFailed: boolean;
  onChange: (build: SessionBuild) => void;
}

/**
 * "From a family": the catalogue's loading, failure and empty states live
 * here and nowhere else — the session mode never waits on the catalogue.
 */
function FamilyPane({
  athleteName,
  catalogue,
  loadFailed,
  build,
  missingSport,
  preview,
  measuring,
  previewFailed,
  onChange,
}: FamilyPaneProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  if (loadFailed) {
    return (
      <Text
        className="px-4 py-6 text-sm text-text-secondary"
        accessibilityRole="alert"
        testID="session-editor-load-failed"
      >
        {t(SESSION_EDITOR_KEYS.loadFailed)}
      </Text>
    );
  }
  if (catalogue === undefined) {
    return (
      <View className="items-center px-4 py-6" testID="session-editor-loading">
        <ActivityIndicator color={colors.tokens.primary} />
        <Text className="mt-2 text-sm text-text-secondary">
          {t(SESSION_EDITOR_KEYS.loading)}
        </Text>
      </View>
    );
  }
  if (build === null) {
    return (
      <Text
        className="px-4 py-6 text-sm text-text-secondary"
        testID="session-editor-no-families"
      >
        {t(SESSION_EDITOR_KEYS.noFamilies, { name: athleteName })}
      </Text>
    );
  }
  return (
    <>
      {missingSport !== null ? (
        <Text
          className="mb-4 px-4 text-sm text-text-secondary"
          testID="session-editor-missing-sport"
        >
          {t(SESSION_EDITOR_KEYS.noFamilyForSport, {
            sport: t(sessionSportLabelKey(missingSport)),
          })}
        </Text>
      ) : null}
      <BuildForm
        catalogue={catalogue}
        build={build}
        flagged={flaggedParams(preview)}
        onChange={onChange}
      />
      <PreviewPanel
        preview={preview}
        measuring={measuring}
        failed={previewFailed}
      />
    </>
  );
}

interface BuildFormProps {
  catalogue: FamilyCatalogueResponse;
  build: SessionBuild;
  /** The values the kernel's preview found outside their range: the fields marked. */
  flagged: ReadonlySet<SessionParamKey>;
  onChange: (build: SessionBuild) => void;
}

/**
 * Sport first: the sport among those a family has a form for, then the
 * families with a form in it, the family's formats in it, and each value
 * the format lets the coach state. Picking a sport, a family or a format
 * starts its values at their defaults in that sport.
 */
function BuildForm({ catalogue, build, flagged, onChange }: BuildFormProps) {
  const { t } = useTranslation();
  // What the coach typed into each value, so a field can be emptied and
  // retyped: an empty field states nothing, and the format's default applies.
  const [typed, setTyped] = useState<Partial<Record<SessionParamKey, string>>>(
    {},
  );
  const sports = catalogueSports(catalogue);
  const families = familiesForSport(catalogue, build.sport);
  const family = familyOf(catalogue, build);
  const preset = presetOf(catalogue, build);
  const formats = presetsForSport(family, build.sport);

  const pick = (next: SessionBuild) => {
    setTyped({});
    onChange(next);
  };

  const type = (key: SessionParamKey, text: string) => {
    // A whole number, nothing more: the range is the findings' business.
    const digits = text.replace(/[^0-9]/g, '');
    setTyped((current) => ({ ...current, [key]: digits }));
    onChange(setParam(build, key, digits === '' ? undefined : Number(digits)));
  };

  return (
    <View className="px-4" testID="session-editor-form">
      <View className="mb-4">
        <RadioGroup
          layout="segmented"
          label={t(SESSION_EDITOR_KEYS.sport)}
          options={sports.map((sport) => ({
            key: sport,
            label: t(sessionSportLabelKey(sport)),
          }))}
          value={build.sport}
          onChange={(key) => {
            const sport = sports.find((s) => s === key);
            if (sport) pick(selectBuildSport(catalogue, build, sport));
          }}
          testID="session-editor-sport"
        />
      </View>
      {families.length === 0 ? (
        <Text
          className="mb-4 text-sm text-text-secondary"
          testID="session-editor-no-family-for-sport"
        >
          {t(SESSION_EDITOR_KEYS.noFamilyForSport, {
            sport: t(sessionSportLabelKey(build.sport)),
          })}
        </Text>
      ) : (
        <>
          <View className="mb-4">
            <RadioGroup
              label={t(SESSION_EDITOR_KEYS.family)}
              options={families.map((f) => ({
                key: f.slug,
                label: f.name,
                description:
                  f.slug === build.family ? f.purpose_label : undefined,
              }))}
              value={build.family}
              onChange={(slug) => {
                const next = families.find((f) => f.slug === slug);
                if (next) pick(selectFamily(next, build.sport));
              }}
              testID="session-editor-family"
            />
          </View>
          {formats.length > 0 ? (
            <View className="mb-4">
              <RadioGroup
                label={t(SESSION_EDITOR_KEYS.format)}
                options={formats.map((p) => ({
                  key: p.key,
                  label: p.name,
                  description: p.key === preset?.key ? p.summary : undefined,
                }))}
                value={preset?.key ?? null}
                onChange={(key) => {
                  const next = formats.find((p) => p.key === key);
                  if (next) pick(selectPreset(build, next));
                }}
                testID="session-editor-format"
              />
            </View>
          ) : null}
          {editableParams(preset, build.sport).map(({ key, range }) => (
            <ParamField
              key={key}
              paramKey={key}
              range={range}
              text={typed[key] ?? String(paramValue(build, key, range))}
              outside={flagged.has(key)}
              onChangeText={(text) => type(key, text)}
            />
          ))}
        </>
      )}
    </View>
  );
}

interface ParamFieldProps {
  paramKey: SessionParamKey;
  range: SessionParamRange;
  text: string;
  /** The stated value sits outside the format's tested range. */
  outside: boolean;
  onChangeText: (text: string) => void;
}

/**
 * One stated value on the number pad, with the format's tested range under
 * it. Outside the range the line takes the warning ink and its mark; the
 * value is kept as typed, never clamped.
 */
function ParamField({ paramKey, range, text, outside, onChangeText }: ParamFieldProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const label = t(SESSION_PARAM_LABEL_KEY[paramKey]);
  const rangeText = t(SESSION_EDITOR_KEYS.testedRange, { min: range.min, max: range.max });
  return (
    <View>
      <Input
        label={label}
        value={text}
        onChangeText={onChangeText}
        keyboardType="number-pad"
        accessibilityLabel={label}
        accessibilityHint={rangeText}
        testID={`session-editor-param-${paramKey}`}
      />
      <View className="-mt-2 mb-4 flex-row items-center" testID={`session-editor-range-${paramKey}`}>
        {outside ? (
          <Feather
            name="alert-triangle"
            size={12}
            color={alertInkColor('warning', colors)}
            testID={`session-editor-outside-${paramKey}`}
          />
        ) : null}
        <Text className={`text-xs ${outside ? `ml-1 ${ALERT_TONE.warning.ink}` : 'text-text-secondary'}`}>
          {rangeText}
        </Text>
      </View>
    </View>
  );
}

interface PreviewPanelProps {
  /** The latest preview; the previous one while the next is measured. */
  preview: SessionPreview | undefined;
  measuring: boolean;
  failed: boolean;
}

/**
 * What the kernel derives from the build on the athlete's anchors, as the
 * server answered it: the day card's marks, the derived values, the steps
 * and the envelope's findings. The previous answer stays while the next one
 * is measured.
 */
function PreviewPanel({ preview, measuring, failed }: PreviewPanelProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  return (
    <View className="px-4" testID="session-editor-preview">
      {measuring ? (
        <View className="mb-2 flex-row items-center" accessibilityLiveRegion="polite" testID="session-editor-measuring">
          <ActivityIndicator size="small" color={colors.tokens.primary} />
          <Text className="ml-2 text-sm text-text-secondary">{t(SESSION_EDITOR_KEYS.measuring)}</Text>
        </View>
      ) : null}
      {failed ? (
        <View
          className={`mb-2 rounded-lg px-3 py-2 ${ALERT_TONE.error.fill}`}
          accessibilityRole="alert"
          testID="session-editor-preview-failed"
        >
          <Text className={`text-sm ${ALERT_TONE.error.ink}`}>{t(SESSION_EDITOR_KEYS.previewFailed)}</Text>
        </View>
      ) : null}
      {preview ? <PreviewBody preview={preview} /> : null}
    </View>
  );
}

/** A heading over one part of the preview. */
function PreviewHeading({ children }: { children: string }) {
  return (
    <Text className="mt-4 mb-1 text-sm font-semibold text-text-primary" accessibilityRole="header">
      {children}
    </Text>
  );
}

function PreviewBody({ preview }: { preview: SessionPreview }) {
  const { t, language } = useTranslation();
  const rows = preview.derived
    ? derivedRows(preview.derived, language, (count) => t(SESSION_EDITOR_KEYS.minutes, { count }))
    : [];
  return (
    <View>
      {preview.outside_evidence ? (
        <Badge
          variant="warning"
          label={preview.outside_evidence}
          className="mt-1 self-start"
          testID="session-editor-outside-evidence"
        />
      ) : null}
      {preview.evidence ? <EvidenceChip evidence={preview.evidence} /> : null}
      {preview.counts_as ? (
        <Text className="mt-1 text-sm text-text-secondary" testID="session-editor-counts-as">
          {preview.counts_as}
        </Text>
      ) : null}
      {preview.anchor ? (
        <Text className="mt-1 text-sm text-text-secondary" testID="session-editor-anchor">
          {preview.anchor}
        </Text>
      ) : null}

      {rows.length > 0 ? (
        <View testID="session-editor-derived">
          <PreviewHeading>{t(SESSION_EDITOR_KEYS.derivedHeading)}</PreviewHeading>
          {rows.map((row) => (
            <View key={row.key} className="flex-row py-0.5" testID={`session-editor-derived-${row.key}`}>
              <Text className="flex-1 text-sm text-text-secondary">{t(row.labelKey, row.labelValues)}</Text>
              <Text className="ml-3 text-sm text-text-primary">
                <Figure>{row.value}</Figure>
              </Text>
            </View>
          ))}
        </View>
      ) : null}

      {preview.steps.length > 0 ? (
        <View testID="session-editor-steps">
          <PreviewHeading>{t(SESSION_EDITOR_KEYS.stepsHeading)}</PreviewHeading>
          {preview.steps.map((step, index) => (
            <Text key={`${step.label}-${index}`} className="py-0.5 text-sm text-text-primary">
              <StepText step={step} />
            </Text>
          ))}
        </View>
      ) : null}

      <PreviewHeading>{t(SESSION_EDITOR_KEYS.findingsHeading)}</PreviewHeading>
      {preview.findings.length === 0 ? (
        <Text className="text-sm text-text-secondary" testID="session-editor-findings-none">
          {t(SESSION_EDITOR_KEYS.findingsInside)}
        </Text>
      ) : (
        <>
          <View className={`rounded-lg px-3 py-2 ${ALERT_TONE.warning.fill}`} testID="session-editor-findings">
            {preview.findings.map((finding, index) => (
              <Text
                key={`${finding.code}-${index}`}
                className={`text-sm ${ALERT_TONE.warning.ink}${index > 0 ? ' mt-1' : ''}`}
              >
                {finding.text}
              </Text>
            ))}
          </View>
          <Text className="mt-1 text-xs text-text-secondary" testID="session-editor-findings-note">
            {t(SESSION_EDITOR_KEYS.findingsNote)}
          </Text>
        </>
      )}
    </View>
  );
}
