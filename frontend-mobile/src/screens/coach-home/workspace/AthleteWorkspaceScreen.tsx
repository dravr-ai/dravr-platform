// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A coach's workspace on one athlete on the phone — the plan's dates to accept, refuse, edit and publish, its history, the plan thread and the room
// ABOUTME: Text tabs over one screen each; reads the plan thread's subject plan and writes through the day editor's hooks, as the web workspace does (carnet#1818)

import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { ActivityIndicator, ScrollView, Text, View } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { KeyboardAvoidingView } from 'react-native-keyboard-controller';
import { useHeaderHeight } from 'expo-router/react-navigation';
import { Stack, useLocalSearchParams } from 'expo-router';
import { useTranslation } from '@pierre/i18n';
import type { CoachRosterAthlete, DraftState } from '@pierre/shared-types';
import {
  GROUP_ROOM_KEYS,
  PLAN_DRAFT_KEYS,
  PLAN_THREAD_KEYS,
  PRIVATE_LANE_KEYS,
  coachRoomName,
  prePublishChecks,
} from '@pierre/chat-utils';
import {
  PLAN_WORKSPACE_KEYS,
  openThreadRefusalText,
  planCalendarWeeks,
  workspaceAgentName,
  workspaceGroupTab,
  workspacePlanState,
  type WorkspaceAnswers,
  type WorkspaceGroupTab,
  type WorkspacePlanRead,
  type WorkspaceThreads,
} from '@pierre/ui-logic';
import { EmptyState, TextTabs, type TextTabItem } from '../../../components/ui';
import { useThemeColors } from '../../../constants/theme';
import { useAuth } from '../../../contexts/AuthContext';
import { useCoachRoster } from '../../../hooks/useCoachRoster';
import { useWorkspaceAnswers, useWorkspaceThreads } from '../../../hooks/useAthleteWorkspace';
import { useSubjectPlan } from '../../../hooks/useCoachPlan';
import PlanDraftControls from '../../chat/PlanDraftControls';
import type { WorkspacePlanSurface } from '../../chat/PlanChangeEntry';
import SessionEditorSheet from '../../chat/SessionEditorSheet';
import type { SessionEditorTarget } from '../../chat/WorkoutPlanCard';
import { useCoachDestinations } from '../useCoachDestinations';
import { PrePublishCheck } from './PrePublishCheck';
import { WorkspaceCalendar } from './WorkspaceCalendar';
import { WorkspaceHistory } from './WorkspaceHistory';
import { WorkspaceThread } from './WorkspaceThread';

/** The workspace's tabs: the plan, its history, the coach's private thread with the agent, the athlete's room. */
type WorkspaceTab = 'plan' | 'history' | 'dravr' | 'group';

const WORKSPACE_TABS: readonly WorkspaceTab[] = ['plan', 'history', 'dravr', 'group'];

/** A thread or a plan still being found, opened or read. */
function Pending({ testID }: { testID: string }) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  return (
    <View className="items-start px-4 py-6" accessibilityLabel={t('common.loading')} testID={testID}>
      <ActivityIndicator color={colors.tokens.primary} />
    </View>
  );
}

/**
 * The plan tab: the pre-publish check, the draft's publish controls, and the
 * two weeks as a list of dates. The plan is the plan thread's subject-plan
 * read: the draft when one is open, else the live plan; a date the read names
 * proposed — one the draft changes over the live plan — is a proposal the
 * coach accepts, for as long as the screen is open (decision W1), or refuses,
 * which puts the live plan back on it. The answers are the screen's, held
 * above the tabs, so reading another tab keeps what was accepted.
 */
function PlanTab({
  athlete,
  name,
  threads,
  plan,
  answers,
  onDraftPlan,
}: {
  athlete: CoachRosterAthlete;
  name: string;
  threads: WorkspaceThreads;
  plan: WorkspacePlanRead;
  answers: WorkspaceAnswers;
  onDraftPlan: () => void;
}) {
  const { t } = useTranslation();
  const { planThreadId } = threads;
  const subject = plan.read?.status === 'ok' ? plan.read.plan : null;

  const shown = subject ? (subject.draft ?? subject.live) : null;
  // Which dates are proposed is the server's to say, by the rule a publish
  // announces them with: it names none when no draft is open.
  const proposed = useMemo(() => subject?.proposed_dates ?? [], [subject]);
  const weeks = useMemo(
    () => (shown && subject ? planCalendarWeeks(shown, proposed, subject.today) : []),
    [shown, subject, proposed],
  );
  const checks = useMemo(() => (subject?.draft ? prePublishChecks(subject.draft, subject.today) : []), [subject]);

  // The draft the publish controls act on, held past its publish so they can
  // say it landed; a newer draft replaces it.
  const draftState = subject?.draft_state ?? null;
  const [heldDraft, setHeldDraft] = useState<DraftState | null>(null);
  useEffect(() => {
    if (draftState !== null) setHeldDraft(draftState);
  }, [draftState]);
  const publishDraft = draftState ?? heldDraft;

  const [editing, setEditing] = useState<SessionEditorTarget | null>(null);

  // Which state the plan is in is decided once, for both clients; this only draws it.
  const planState = workspacePlanState(athlete, threads, plan);
  let body: React.ReactNode;
  switch (planState.kind) {
    case 'noGroup':
      body = <EmptyState testID="workspace-thread-failed">{t(PLAN_THREAD_KEYS.openError)}</EmptyState>;
      break;
    case 'threadFailed':
      body = (
        <EmptyState
          action={{ label: t('common.retry'), onPress: planState.retry, testID: 'workspace-thread-retry' }}
          testID="workspace-thread-failed"
        >
          {openThreadRefusalText(planState.error, name, t)}
        </EmptyState>
      );
      break;
    case 'groupsFailed':
      body = (
        <EmptyState
          action={{ label: t('common.retry'), onPress: planState.retry, testID: 'workspace-thread-retry' }}
          testID="workspace-thread-failed"
        >
          {t(PLAN_THREAD_KEYS.openError)}
        </EmptyState>
      );
      break;
    case 'loading':
      body = <Pending testID="workspace-plan-loading" />;
      break;
    case 'consentRevoked':
      body = <EmptyState testID="workspace-consent-revoked">{t(PLAN_DRAFT_KEYS.consentRevoked, { name })}</EmptyState>;
      break;
    case 'loadFailed':
      body = (
        <EmptyState
          action={{ label: t('common.retry'), onPress: planState.retry, testID: 'workspace-plan-retry' }}
          testID="workspace-plan-failed"
        >
          {t(PLAN_WORKSPACE_KEYS.loadFailed, { name })}
        </EmptyState>
      );
      break;
    case 'noPlan':
      body = (
        <EmptyState
          action={{ label: t(PLAN_THREAD_KEYS.starterLabel, { name }), onPress: onDraftPlan, testID: 'workspace-draft-plan' }}
          testID="workspace-no-plan"
        >
          {t(PLAN_WORKSPACE_KEYS.noPlan, { name })}
        </EmptyState>
      );
      break;
    case 'plan':
      body = (
        <>
          <PrePublishCheck
            items={checks}
            athleteName={name}
            onKeep={(item) => void answers.setCall(item.day, item.position, true)}
            keeping={answers.callSaving}
            onReview={(item) => setEditing({ day: item.day, entries: item.entries, mode: 'edit' })}
          />
          {publishDraft !== null ? (
            <PlanDraftControls
              key={publishDraft.draft_id}
              conversationId={planState.planThreadId}
              subject={planState.subject.athlete}
              draft={publishDraft}
            />
          ) : null}
          <WorkspaceCalendar
            weeks={weeks}
            isAccepted={answers.isAccepted}
            onAccept={answers.accept}
            onRefuse={(date) => void answers.refuse(date)}
            refusing={answers.refusing}
            onWithdrawCall={(day, position) => void answers.setCall(day, position, false)}
            callSaving={answers.callSaving}
            onEdit={setEditing}
          />
        </>
      );
      break;
  }

  return (
    <>
      <ScrollView
        contentContainerClassName="gap-8 pb-10 pt-4"
        accessibilityLabel={t(PLAN_WORKSPACE_KEYS.calendar)}
        testID="workspace-plan"
      >
        {body}
      </ScrollView>
      {planThreadId !== null && subject !== null ? (
        <SessionEditorSheet
          conversationId={planThreadId}
          athleteName={name}
          target={editing}
          onClose={() => setEditing(null)}
        />
      ) : null}
    </>
  );
}

/**
 * The Groupe tab: the athlete's own room, the room alone — no private lane,
 * the agent's thread is the tab beside it. Until the coach holds a
 * conversation in it, a button opens one; an athlete whose groups hold other
 * athletes has no room, and the tab says so.
 */
function GroupTab({
  athlete,
  group,
  threads,
  onWriteToGroup,
}: {
  athlete: CoachRosterAthlete;
  group: WorkspaceGroupTab;
  threads: WorkspaceThreads;
  onWriteToGroup: () => void;
}) {
  const { t } = useTranslation();
  const { user } = useAuth();
  if (group.kind === 'failed') {
    return (
      <EmptyState
        action={{ label: t('common.retry'), onPress: group.retry, testID: 'workspace-room-retry' }}
        testID="workspace-room-failed"
      >
        {t(GROUP_ROOM_KEYS.openError)}
      </EmptyState>
    );
  }
  if (group.kind === 'noRoom') {
    return (
      <EmptyState
        action={{ label: t('coachHome.action.writeToGroup'), onPress: onWriteToGroup, testID: 'workspace-write-group' }}
        testID="workspace-no-room"
      >
        {t(PLAN_WORKSPACE_KEYS.noRoom, { name: athlete.name })}
      </EmptyState>
    );
  }
  const { room } = group;
  return (
    <View className="flex-1" testID="workspace-room">
      <Text className="px-4 pb-1 pt-3 text-xs text-text-secondary">{room.group.name}</Text>
      {room.conversationId === null ? (
        <EmptyState
          action={{
            label: threads.roomOpening ? t('coachHome.action.openingGroup') : t('coachHome.action.writeToGroup'),
            onPress: threads.openRoom,
            testID: 'workspace-open-room',
          }}
          testID="workspace-room-unopened"
        >
          {threads.roomError ? t(GROUP_ROOM_KEYS.openError) : t(GROUP_ROOM_KEYS.empty, { group: room.group.name })}
        </EmptyState>
      ) : (
        <WorkspaceThread
          key={room.conversationId}
          conversationId={room.conversationId}
          groupId={room.group.id}
          conversationTitle={room.group.name}
          coachRoomName={coachRoomName(room.group, user?.id ?? null, room.group.name)}
        />
      )}
    </View>
  );
}

/** One athlete's workspace, once the roster names them. */
function AthleteWorkspace({ athlete }: { athlete: CoachRosterAthlete }) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const headerHeight = useHeaderHeight();
  const destinations = useCoachDestinations();
  const threads = useWorkspaceThreads(athlete);
  const { planThreadId } = threads;
  const plan = useSubjectPlan(planThreadId, planThreadId !== null);
  const subject = plan.read?.status === 'ok' ? plan.read.plan : null;
  const name = subject?.athlete.display_name.trim() || athlete.name;
  const answers = useWorkspaceAnswers(planThreadId);

  const [tab, setTab] = useState<WorkspaceTab>('plan');
  // « Rédiger le plan de … » with no plan yet: the agent's tab takes the
  // starter, as the plan thread's own one-tap start sends it.
  const [pendingSend, setPendingSend] = useState<string | null>(null);
  const consumePendingSend = useCallback(() => setPendingSend(null), []);
  const draftPlan = () => {
    setPendingSend(t(PLAN_THREAD_KEYS.starterMessage, { name }));
    setTab('dravr');
  };

  // « Voir sur le calendrier » from the agent's thread brings the plan forward.
  const showPlan = useCallback(() => setTab('plan'), []);
  const planSurface = useMemo<WorkspacePlanSurface>(() => ({ kind: 'workspace', onShowCalendar: showPlan }), [showPlan]);

  const agentName = workspaceAgentName(threads.planGroup, t('shell.brandName'));
  const groupTab = workspaceGroupTab(threads, athlete);

  const tabs: TextTabItem[] = [
    { key: 'plan', label: t('coachHome.detail.plan') },
    { key: 'history', label: t('home.personal.history') },
    { key: 'dravr', label: agentName },
    ...(groupTab !== null ? [{ key: 'group', label: t('app.group') }] : []),
  ];
  const active: WorkspaceTab = tabs.some((entry) => entry.key === tab) ? tab : 'plan';
  const selectTab = (key: string) => {
    const next = WORKSPACE_TABS.find((known) => known === key);
    if (next !== undefined) setTab(next);
  };

  let body: React.ReactNode;
  if (active === 'plan') {
    body = <PlanTab athlete={athlete} name={name} threads={threads} plan={plan} answers={answers} onDraftPlan={draftPlan} />;
  } else if (active === 'history') {
    body = <WorkspaceHistory planThreadId={planThreadId} agentName={agentName} />;
  } else if (active === 'dravr') {
    body =
      planThreadId === null ? (
        threads.planError !== null ? (
          <EmptyState
            action={{ label: t('common.retry'), onPress: threads.retryPlanThread, testID: 'workspace-thread-retry' }}
            testID="workspace-thread-failed"
          >
            {openThreadRefusalText(threads.planError, name, t)}
          </EmptyState>
        ) : (
          <Pending testID="workspace-thread-loading" />
        )
      ) : (
        <View className="flex-1" testID="workspace-dravr">
          <View className="flex-row items-center gap-1.5 px-4 pb-1 pt-3">
            <Feather name="lock" size={12} color={colors.text.secondary} />
            <Text className="text-xs text-text-secondary">{t(PRIVATE_LANE_KEYS.divider, { agent: agentName })}</Text>
          </View>
          <WorkspaceThread
            key={planThreadId}
            conversationId={planThreadId}
            groupId={null}
            conversationTitle={agentName}
            planSubjectName={name}
            planSurface={planSurface}
            pendingSend={pendingSend}
            onPendingSendConsumed={consumePendingSend}
          />
        </View>
      );
  } else if (groupTab !== null) {
    body = (
      <GroupTab
        athlete={athlete}
        group={groupTab}
        threads={threads}
        onWriteToGroup={() => destinations.writeToGroup(athlete, null)}
      />
    );
  }

  return (
    <View className="flex-1 bg-background-primary" testID="athlete-workspace">
      <Stack.Screen options={{ title: name }} />
      {/* One column the keyboard shortens, under the native header, as the chat screen's is. */}
      <KeyboardAvoidingView style={{ flex: 1 }} behavior="padding" keyboardVerticalOffset={headerHeight}>
        <TextTabs items={tabs} value={active} onChange={selectTab} testID="workspace-tabs" />
        <View className="flex-1" testID={`workspace-panel-${active}`}>
          {body}
        </View>
      </KeyboardAvoidingView>
    </View>
  );
}

/** A roster athlete's workspace, opened from their detail, the Chat tab's row or a thread's header. */
export function AthleteWorkspaceScreen() {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const { athleteId } = useLocalSearchParams<{ athleteId: string }>();
  const { roster, isLoading, isError, refetch } = useCoachRoster(true);
  const athlete = roster?.athletes.find((entry) => entry.user_id === athleteId);

  if (athlete === undefined) {
    return (
      <View className="flex-1 bg-background-primary" testID="athlete-workspace-missing">
        <Stack.Screen options={{ title: '' }} />
        {isLoading ? (
          <View className="items-center py-10">
            <ActivityIndicator color={colors.tokens.primary} />
          </View>
        ) : (
          <EmptyState action={isError ? { label: t('common.retry'), onPress: refetch } : undefined}>
            {isError ? t('coachHome.loadFailed') : t('coachHome.queue.selectAthlete')}
          </EmptyState>
        )}
      </View>
    );
  }

  return <AthleteWorkspace key={athlete.user_id} athlete={athlete} />;
}
