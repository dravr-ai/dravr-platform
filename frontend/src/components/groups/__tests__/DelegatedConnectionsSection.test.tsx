// ABOUTME: Tests for the coaching-platform link section — the coach's roster, picker, propose and each refusal's words
// ABOUTME: The member's consents at confirm and after, decline and unlink; never the server's English refusal text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import DelegatedConnectionsSection from '../DelegatedConnectionsSection';
import { ToastProvider } from '../../ui';
import type { DelegatedConnection, GroupMember } from '@pierre/shared-types';

const GROUP_ID = 'group-1';

vi.mock('../../../services/api', () => ({
  groupsApi: {
    getDelegationRoster: vi.fn(),
    proposeDelegatedConnection: vi.fn(),
    confirmDelegatedConnection: vi.fn(),
    setDelegatedLinkConsents: vi.fn(),
    endDelegatedConnection: vi.fn(),
  },
  providersApi: {
    getProvidersStatus: vi.fn(),
  },
}));

const { groupsApi, providersApi } = await import('../../../services/api');

function member(userId: string, displayName: string): GroupMember {
  return {
    id: `membership-${userId}`,
    group_id: GROUP_ID,
    user_id: userId,
    role: 'member',
    peer_sharing_consent: true,
    consent_given_at: '2026-01-01T00:00:00Z',
    joined_at: '2026-01-01T00:00:00Z',
    display_name: displayName,
  };
}

function link(overrides: Partial<DelegatedConnection> = {}): DelegatedConnection {
  return {
    id: 'dc-1',
    group_id: GROUP_ID,
    provider: 'trainingpeaks',
    coach_user_id: 'user-coach',
    coach_display_name: 'Casey Coach',
    member_user_id: 'user-alex',
    member_display_name: 'alex@example.com',
    provider_athlete_id: '900001',
    provider_athlete_name: 'Alex Athlete',
    status: 'proposed',
    proposed_at: '2026-09-24T08:00:00Z',
    confirmed_at: null,
    read_refused: null,
    calendar_write: false,
    calendar_write_supported: false,
    wellness_read: false,
    wellness_read_supported: false,
    wellness_read_skipped: null,
    ...overrides,
  };
}

/** An Intervals.icu link, whose platform offers both consents. */
function intervalsLink(overrides: Partial<DelegatedConnection> = {}): DelegatedConnection {
  return link({
    provider: 'intervals_icu',
    provider_athlete_id: 'i201',
    calendar_write_supported: true,
    wellness_read_supported: true,
    ...overrides,
  });
}

/** An axios-shaped refusal carrying `details.reason`. */
/**
 * A link step's refusal as the server sends it: the reason, and the platform
 * it is about whenever one is known (`null` for none).
 */
function refusal(status: number, reason: string, provider: string | null = 'trainingpeaks') {
  const details = provider ? { reason, provider } : { reason };
  return Object.assign(new Error(`Request failed with status code ${status}`), {
    response: { status, data: { code: 'InvalidInput', message: 'English for an API caller', details } },
  });
}

const MEMBERS = [member('user-alex', 'alex@example.com'), member('user-blair', 'blair@example.com')];

function renderSection(props: Partial<React.ComponentProps<typeof DelegatedConnectionsSection>> = {}) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <DelegatedConnectionsSection
          groupId={GROUP_ID}
          mode="coach"
          connections={[]}
          members={MEMBERS}
          {...props}
        />
      </ToastProvider>
    </QueryClientProvider>,
  );
}

describe('DelegatedConnectionsSection — coach', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(groupsApi.getDelegationRoster).mockResolvedValue({
      provider: 'trainingpeaks',
      athletes: [
        {
          provider_athlete_id: '900001',
          display_name: 'Alex Athlete',
          connection: null,
          suggested_member_user_id: 'user-alex',
        },
        {
          provider_athlete_id: '900002',
          display_name: 'Blair Biker',
          connection: link({
            id: 'dc-2',
            member_user_id: 'user-blair',
            member_display_name: 'blair@example.com',
            provider_athlete_id: '900002',
            provider_athlete_name: 'Blair Biker',
          }),
          suggested_member_user_id: null,
        },
      ],
    });
  });

  it('renders one row per roster athlete with the suggested member preselected', async () => {
    renderSection({
      connections: [link({ id: 'dc-2', member_user_id: 'user-blair', provider_athlete_id: '900002' })],
    });

    const alex = await screen.findByTestId('delegation-roster-row-900001');
    expect(alex).toHaveTextContent('Alex Athlete');
    const select = within(alex).getByTestId('delegation-member-select-900001') as HTMLSelectElement;
    expect(select.value).toBe('user-alex');
    // Blair already holds a live link, so the picker does not offer them again.
    expect(within(select).getAllByRole('option').map((o) => o.textContent)).toEqual([
      'Choose a member',
      'alex@example.com (suggested)',
    ]);

    const blair = screen.getByTestId('delegation-roster-row-900002');
    expect(blair).toHaveTextContent('Waiting for blair@example.com to confirm');
    expect(within(blair).getByTestId('delegation-withdraw-dc-2')).toHaveTextContent('Withdraw');
  });

  it('preselects a suggestion whose member arrives after the roster, and keeps a pick the coach made', async () => {
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
    });
    const view = (members: GroupMember[]) => (
      <QueryClientProvider client={queryClient}>
        <ToastProvider>
          <DelegatedConnectionsSection groupId={GROUP_ID} mode="coach" connections={[]} members={members} />
        </ToastProvider>
      </QueryClientProvider>
    );
    // The roster resolves before the group's members do.
    const { rerender } = render(view([]));
    const row = await screen.findByTestId('delegation-roster-row-900001');
    const select = () => within(row).getByTestId('delegation-member-select-900001') as HTMLSelectElement;
    expect(select().value).toBe('');

    rerender(view([...MEMBERS]));
    expect(select().value).toBe('user-alex');

    const user = userEvent.setup();
    await user.selectOptions(select(), 'user-blair');
    rerender(view([...MEMBERS]));
    expect(select().value).toBe('user-blair');
  });

  it('proposes the picked athlete and member', async () => {
    vi.mocked(groupsApi.proposeDelegatedConnection).mockResolvedValue(link());
    const user = userEvent.setup();
    renderSection();

    await user.click(await screen.findByTestId('delegation-propose-900001'));

    await waitFor(() =>
      expect(groupsApi.proposeDelegatedConnection).toHaveBeenCalledWith(GROUP_ID, {
        provider: 'trainingpeaks',
        provider_athlete_id: '900001',
        member_user_id: 'user-alex',
      }),
    );
    expect(await screen.findByText('Link sent')).toBeInTheDocument();
  });

  it('proposes on the platform the roster was read from', async () => {
    vi.mocked(groupsApi.getDelegationRoster).mockResolvedValue({
      provider: 'intervals_icu',
      athletes: [
        {
          provider_athlete_id: 'i201',
          display_name: 'Alex Athlete',
          connection: null,
          suggested_member_user_id: 'user-alex',
        },
      ],
    });
    vi.mocked(groupsApi.proposeDelegatedConnection).mockResolvedValue(link());
    const user = userEvent.setup();
    renderSection();

    await user.click(await screen.findByTestId('delegation-propose-i201'));

    await waitFor(() =>
      expect(groupsApi.proposeDelegatedConnection).toHaveBeenCalledWith(GROUP_ID, {
        provider: 'intervals_icu',
        provider_athlete_id: 'i201',
        member_user_id: 'user-alex',
      }),
    );
  });

  it('words a propose refusal from its reason, not the server message', async () => {
    vi.mocked(groupsApi.proposeDelegatedConnection).mockRejectedValue(refusal(409, 'already_proposed'));
    const user = userEvent.setup();
    renderSection();

    await user.click(await screen.findByTestId('delegation-propose-900001'));

    expect(
      await screen.findByText('This member already has a TrainingPeaks link in this group.'),
    ).toBeInTheDocument();
    expect(screen.queryByText('English for an API caller')).toBeNull();
  });

  it('withdraws a proposed link', async () => {
    vi.mocked(groupsApi.endDelegatedConnection).mockResolvedValue(undefined);
    const user = userEvent.setup();
    renderSection();

    await user.click(await screen.findByTestId('delegation-withdraw-dc-2'));

    await waitFor(() => expect(groupsApi.endDelegatedConnection).toHaveBeenCalledWith(GROUP_ID, 'dc-2'));
  });

  it('sends a coach with no coaching platform connected to their connections', async () => {
    vi.mocked(groupsApi.getDelegationRoster).mockRejectedValue(refusal(400, 'coach_platform_not_connected', null));
    const onOpenConnections = vi.fn();
    const user = userEvent.setup();
    renderSection({ onOpenConnections });

    // No platform is connected, so the sentence names every one a coach can connect.
    const refused = await screen.findByTestId('delegation-roster-refused');
    expect(refused).toHaveTextContent(
      'Connect TrainingPeaks / Intervals.icu with the account your athletes are on to link them.',
    );
    await user.click(within(refused).getByTestId('delegation-open-connections'));
    expect(onOpenConnections).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId('delegation-refresh')).toBeNull();
  });

  it('names the platform the coach links run on when the roster read fails', async () => {
    vi.mocked(groupsApi.getDelegationRoster).mockRejectedValue(
      Object.assign(new Error('Request failed with status code 500'), {
        response: { status: 500, data: { code: 'InternalError', message: 'English for an API caller' } },
      }),
    );
    renderSection({ connections: [link({ provider: 'intervals_icu' })] });

    // A transient failure names no platform; the coach's own links still do.
    const refused = await screen.findByTestId('delegation-roster-refused');
    expect(refused).toHaveTextContent('Your Intervals.icu roster could not be read. Try again in a moment.');
    expect(screen.getByTestId('delegation-refresh')).toBeInTheDocument();
  });

  it('asks an Intervals.icu coach linked by OAuth for their API key, naming Intervals.icu', async () => {
    vi.mocked(groupsApi.getDelegationRoster).mockRejectedValue(
      refusal(400, 'coach_platform_api_key_required', 'intervals_icu'),
    );
    const onOpenConnections = vi.fn();
    const user = userEvent.setup();
    renderSection({ onOpenConnections });

    const refused = await screen.findByTestId('delegation-roster-refused');
    expect(refused).toHaveTextContent(
      'Intervals.icu lists your athletes for an API key only. Reconnect Intervals.icu with your API key to read your roster.',
    );
    expect(refused).not.toHaveTextContent('TrainingPeaks');
    await user.click(within(refused).getByTestId('delegation-open-connections'));
    expect(onOpenConnections).toHaveBeenCalledTimes(1);
  });

  it('names Intervals.icu throughout an Intervals.icu roster', async () => {
    vi.mocked(groupsApi.getDelegationRoster).mockResolvedValue({ provider: 'intervals_icu', athletes: [] });
    renderSection();

    expect(await screen.findByText('Your Intervals.icu roster lists no athletes yet.')).toBeInTheDocument();
    expect(screen.getByTestId('delegation-section')).toHaveTextContent(
      'Link each athlete on your Intervals.icu roster to the member they are in this group.',
    );
    expect(screen.getByTestId('delegation-section')).not.toHaveTextContent('TrainingPeaks');
  });

  it('sends a coach whose TrainingPeaks account is not theirs by email to their connections', async () => {
    vi.mocked(groupsApi.getDelegationRoster).mockRejectedValue(refusal(400, 'coach_platform_email_mismatch'));
    const onOpenConnections = vi.fn();
    const user = userEvent.setup();
    renderSection({ onOpenConnections });

    const refused = await screen.findByTestId('delegation-roster-refused');
    expect(refused).toHaveTextContent(
      'The TrainingPeaks account you connected uses a different email than your Dravr account.',
    );
    expect(refused).not.toHaveTextContent('English for an API caller');
    await user.click(within(refused).getByTestId('delegation-open-connections'));
    expect(onOpenConnections).toHaveBeenCalledTimes(1);
  });

  it('words a propose refused because the athlete is not the member by email', async () => {
    vi.mocked(groupsApi.proposeDelegatedConnection).mockRejectedValue(refusal(400, 'athlete_email_mismatch'));
    const user = userEvent.setup();
    renderSection();

    await user.click(await screen.findByTestId('delegation-propose-900001'));

    expect(
      await screen.findByText(
        "This athlete's TrainingPeaks email is not the member's Dravr email, so they cannot be linked.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText('English for an API caller')).toBeNull();
  });

  it('tells the coach why a confirmed link reads nothing, and keeps the unlink', async () => {
    vi.mocked(groupsApi.getDelegationRoster).mockResolvedValue({
      provider: 'trainingpeaks',
      athletes: [
        {
          provider_athlete_id: '900001',
          display_name: 'Alex Athlete',
          connection: link({
            status: 'confirmed',
            confirmed_at: '2026-09-24T09:00:00Z',
            read_refused: 'athlete_email_missing',
          }),
          suggested_member_user_id: null,
        },
      ],
    });
    renderSection();

    expect(await screen.findByTestId('delegation-read-refused-dc-1')).toHaveTextContent(
      'TrainingPeaks shares no email for this athlete, so Dravr cannot match them to a member.',
    );
    expect(screen.queryByText(/Linked to/)).toBeNull();
    expect(screen.getByTestId('delegation-unlink-dc-1')).toBeInTheDocument();
  });

  it('says the platform gives the coach no access to an athlete they tried to link', async () => {
    vi.mocked(groupsApi.getDelegationRoster).mockResolvedValue({
      provider: 'intervals_icu',
      athletes: [
        { provider_athlete_id: 'i201', display_name: 'Alex Athlete', connection: null, suggested_member_user_id: null },
      ],
    });
    vi.mocked(groupsApi.proposeDelegatedConnection).mockRejectedValue(
      refusal(400, 'athlete_no_access', 'intervals_icu'),
    );
    const user = userEvent.setup();
    renderSection();

    const row = await screen.findByTestId('delegation-roster-row-i201');
    await user.selectOptions(within(row).getByTestId('delegation-member-select-i201'), 'user-alex');
    await user.click(within(row).getByTestId('delegation-propose-i201'));

    expect(
      await screen.findByText('Intervals.icu gives your account no access to this athlete, so they cannot be linked.'),
    ).toBeInTheDocument();
    expect(screen.queryByText('English for an API caller')).toBeNull();
  });

  it('tells the coach what each confirmed member shares beyond their workouts', async () => {
    vi.mocked(groupsApi.getDelegationRoster).mockResolvedValue({
      provider: 'intervals_icu',
      athletes: [
        {
          provider_athlete_id: 'i201',
          display_name: 'Alex Athlete',
          connection: intervalsLink({ status: 'confirmed', calendar_write: true, wellness_read: true }),
          suggested_member_user_id: null,
        },
        {
          provider_athlete_id: 'i202',
          display_name: 'Blair Biker',
          connection: intervalsLink({
            id: 'dc-2',
            member_user_id: 'user-blair',
            provider_athlete_id: 'i202',
            status: 'confirmed',
          }),
          suggested_member_user_id: null,
        },
        {
          provider_athlete_id: 'i203',
          display_name: 'Cam Climber',
          connection: intervalsLink({ id: 'dc-3', provider_athlete_id: 'i203' }),
          suggested_member_user_id: null,
        },
      ],
    });
    renderSection();

    const both = await screen.findByTestId('delegation-consents-dc-1');
    expect(within(both).getAllByRole('listitem').map((item) => item.textContent)).toEqual([
      'Can put workouts on their calendar',
      'Shares sleep, HRV, resting heart rate, weight and notes',
    ]);
    expect(screen.getByTestId('delegation-consents-dc-2')).toHaveTextContent('Workouts only');
    // A proposal has granted nothing yet, so it carries no line.
    expect(screen.getByTestId('delegation-waiting-dc-3')).toBeInTheDocument();
    expect(screen.queryByTestId('delegation-consents-dc-3')).toBeNull();
  });

  it('says where a granted wellness comes from when a source of the member own skips the link', async () => {
    vi.mocked(groupsApi.getDelegationRoster).mockResolvedValue({
      provider: 'intervals_icu',
      athletes: [
        {
          provider_athlete_id: 'i201',
          display_name: 'Alex Athlete',
          connection: intervalsLink({
            status: 'confirmed',
            calendar_write: true,
            wellness_read: true,
            wellness_read_skipped: 'own_wellness_source',
          }),
          suggested_member_user_id: null,
        },
        {
          provider_athlete_id: 'i202',
          display_name: 'Blair Biker',
          connection: intervalsLink({
            id: 'dc-2',
            member_user_id: 'user-blair',
            provider_athlete_id: 'i202',
            status: 'confirmed',
            wellness_read: true,
            wellness_read_skipped: 'own_connection',
          }),
          suggested_member_user_id: null,
        },
      ],
    });
    renderSection();

    const skipped = await screen.findByTestId('delegation-consents-dc-1');
    expect(within(skipped).getAllByRole('listitem').map((item) => item.textContent)).toEqual([
      'Can put workouts on their calendar',
      'Wellness comes from their own Garmin or WHOOP connection',
    ]);
    // A member's own Intervals.icu connection has no coach line of its own, so
    // nothing claims the link shares their wellness.
    const own = screen.getByTestId('delegation-consents-dc-2');
    expect(own).toHaveTextContent('Workouts only');
    expect(own).not.toHaveTextContent('Shares sleep');
  });

  it('gives a TrainingPeaks link no consent line, since TrainingPeaks offers none', async () => {
    vi.mocked(groupsApi.getDelegationRoster).mockResolvedValue({
      provider: 'trainingpeaks',
      athletes: [
        {
          provider_athlete_id: '900001',
          display_name: 'Alex Athlete',
          connection: link({ status: 'confirmed', confirmed_at: '2026-09-24T09:00:00Z' }),
          suggested_member_user_id: null,
        },
      ],
    });
    renderSection();

    expect(await screen.findByTestId('delegation-unlink-dc-1')).toBeInTheDocument();
    expect(screen.queryByTestId('delegation-consents-dc-1')).toBeNull();
  });

  it('tells a coach whose account trains that it has no roster, with nowhere to send them', async () => {
    vi.mocked(groupsApi.getDelegationRoster).mockRejectedValue(
      refusal(400, 'coach_platform_not_coach_account'),
    );
    renderSection({ onOpenConnections: vi.fn() });

    const refused = await screen.findByTestId('delegation-roster-refused');
    expect(refused).toHaveTextContent("is an athlete's own account, so it has no roster to link");
    expect(within(refused).queryByTestId('delegation-open-connections')).toBeNull();
  });
});

describe('DelegatedConnectionsSection — member', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(providersApi.getProvidersStatus).mockResolvedValue({ providers: [] });
  });

  it('asks for nothing beyond the read on a TrainingPeaks link, and confirms with no body', async () => {
    vi.mocked(groupsApi.confirmDelegatedConnection).mockResolvedValue(link({ status: 'confirmed' }));
    const user = userEvent.setup();
    renderSection({ mode: 'member', connections: [link()] });

    const request = await screen.findByTestId('delegation-request');
    expect(within(request).queryByTestId('delegation-consents')).toBeNull();
    await user.click(within(request).getByTestId('delegation-confirm'));

    await waitFor(() =>
      expect(groupsApi.confirmDelegatedConnection).toHaveBeenCalledWith(GROUP_ID, 'dc-1', undefined),
    );
  });

  it('offers both consents on an Intervals.icu proposal, off until ticked, and confirms with what was ticked', async () => {
    vi.mocked(groupsApi.confirmDelegatedConnection).mockResolvedValue(
      intervalsLink({ status: 'confirmed', calendar_write: true }),
    );
    const user = userEvent.setup();
    renderSection({ mode: 'member', connections: [intervalsLink()] });

    const request = await screen.findByTestId('delegation-request');
    const calendar = within(request).getByRole('checkbox', {
      name: /Let Casey Coach put workouts on your Intervals\.icu calendar/,
    });
    const wellness = within(request).getByRole('checkbox', {
      name: /Also share your Intervals\.icu sleep, HRV, resting heart rate, weight and daily notes/,
    });
    expect(calendar).not.toBeChecked();
    expect(wellness).not.toBeChecked();
    expect(request).toHaveTextContent(
      'Workouts Casey Coach plans for you in Dravr go on your Intervals.icu calendar, and from there to your devices.',
    );
    expect(request).toHaveTextContent('Turning this off removes what Dravr already read.');

    await user.click(calendar);
    expect(calendar).toBeChecked();
    await user.click(within(request).getByTestId('delegation-confirm'));

    await waitFor(() =>
      expect(groupsApi.confirmDelegatedConnection).toHaveBeenCalledWith(GROUP_ID, 'dc-1', {
        calendar_write: true,
        wellness_read: false,
      }),
    );
    expect(await screen.findByText('Intervals.icu linked')).toBeInTheDocument();
  });

  it('shows what a confirmed link shares, and changes one consent at a time', async () => {
    let answer: (link: DelegatedConnection) => void = () => undefined;
    vi.mocked(groupsApi.setDelegatedLinkConsents).mockReturnValue(
      new Promise((resolve) => {
        answer = resolve;
      }),
    );
    const user = userEvent.setup();
    renderSection({
      mode: 'member',
      connections: [
        intervalsLink({
          status: 'confirmed',
          confirmed_at: '2026-09-24T09:00:00Z',
          calendar_write: true,
          wellness_read: true,
        }),
      ],
    });

    const linked = await screen.findByTestId('delegation-linked');
    const calendar = within(linked).getByTestId('delegation-consent-calendar_write');
    const wellness = within(linked).getByTestId('delegation-consent-wellness_read');
    expect(calendar).toBeChecked();
    expect(wellness).toBeChecked();

    await user.click(calendar);

    await waitFor(() =>
      expect(groupsApi.setDelegatedLinkConsents).toHaveBeenCalledWith(GROUP_ID, 'dc-1', { calendar_write: false }),
    );
    // The change shows as made while it is on its way, and nothing else moves.
    expect(calendar).not.toBeChecked();
    expect(calendar).toBeDisabled();
    expect(wellness).toBeChecked();

    answer(intervalsLink({ status: 'confirmed', calendar_write: false, wellness_read: true }));
    expect(await screen.findByText('Sharing updated')).toBeInTheDocument();
  });

  it('asks before turning wellness off, and changes nothing when the member keeps sharing', async () => {
    vi.mocked(groupsApi.setDelegatedLinkConsents).mockResolvedValue(
      intervalsLink({ status: 'confirmed', wellness_read: false }),
    );
    const user = userEvent.setup();
    renderSection({
      mode: 'member',
      connections: [intervalsLink({ status: 'confirmed', confirmed_at: '2026-09-24T09:00:00Z', wellness_read: true })],
    });

    const wellness = await screen.findByTestId('delegation-consent-wellness_read');
    await user.click(wellness);
    const dialog = await screen.findByRole('dialog');
    expect(dialog).toHaveTextContent('Stop sharing wellness?');
    expect(dialog).toHaveTextContent(
      "Dravr stops reading your Intervals.icu wellness through Casey Coach's account and removes what it already read.",
    );
    await user.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(groupsApi.setDelegatedLinkConsents).not.toHaveBeenCalled();
    expect(wellness).toBeChecked();

    await user.click(wellness);
    await user.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Stop sharing' }));
    await waitFor(() =>
      expect(groupsApi.setDelegatedLinkConsents).toHaveBeenCalledWith(GROUP_ID, 'dc-1', { wellness_read: false }),
    );
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('turns wellness on without asking', async () => {
    vi.mocked(groupsApi.setDelegatedLinkConsents).mockResolvedValue(
      intervalsLink({ status: 'confirmed', wellness_read: true }),
    );
    const user = userEvent.setup();
    renderSection({
      mode: 'member',
      connections: [intervalsLink({ status: 'confirmed', confirmed_at: '2026-09-24T09:00:00Z' })],
    });

    await user.click(await screen.findByTestId('delegation-consent-wellness_read'));

    await waitFor(() =>
      expect(groupsApi.setDelegatedLinkConsents).toHaveBeenCalledWith(GROUP_ID, 'dc-1', { wellness_read: true }),
    );
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('notes under the wellness control when a source of the member own already gives Dravr it', async () => {
    const note =
      'Your own Garmin or WHOOP connection already gives Dravr this, so the link reads none of it while that connection stays.';
    renderSection({
      mode: 'member',
      connections: [
        intervalsLink({
          id: 'dc-linked',
          status: 'confirmed',
          confirmed_at: '2026-09-24T09:00:00Z',
          wellness_read: true,
          wellness_read_skipped: 'own_wellness_source',
        }),
        intervalsLink({ id: 'dc-asked', wellness_read_skipped: 'own_wellness_source' }),
        intervalsLink({ id: 'dc-plain', status: 'confirmed', confirmed_at: '2026-09-24T09:00:00Z', wellness_read: true }),
      ],
    });

    // The control stays, so the member can still revoke what they granted.
    const linked = await screen.findByTestId('delegation-member-link-dc-linked');
    expect(within(linked).getByTestId('delegation-consent-wellness_read')).toBeChecked();
    expect(within(linked).getByTestId('delegation-consent-note-wellness_read')).toHaveTextContent(note);
    // Before the confirm too, so the member knows what granting it does.
    const asked = screen.getByTestId('delegation-member-link-dc-asked');
    expect(within(asked).getByTestId('delegation-consent-note-wellness_read')).toHaveTextContent(note);
    const plain = screen.getByTestId('delegation-member-link-dc-plain');
    expect(within(plain).queryByTestId('delegation-consent-note-wellness_read')).toBeNull();
    expect(within(linked).queryByTestId('delegation-consent-note-calendar_write')).toBeNull();
  });

  it('words a refused consent change from its reason, not the server message', async () => {
    vi.mocked(groupsApi.setDelegatedLinkConsents).mockRejectedValue(
      refusal(403, 'consent_grant_needs_app', 'intervals_icu'),
    );
    const user = userEvent.setup();
    renderSection({
      mode: 'member',
      connections: [intervalsLink({ status: 'confirmed', confirmed_at: '2026-09-24T09:00:00Z' })],
    });

    await user.click(await screen.findByTestId('delegation-consent-calendar_write'));

    expect(await screen.findByText('That did not go through. Try again.')).toBeInTheDocument();
    expect(screen.queryByText('English for an API caller')).toBeNull();
    // Refused, the consent reads as it stands on the server again.
    expect(screen.getByTestId('delegation-consent-calendar_write')).not.toBeChecked();
  });

  it('declines a proposed link', async () => {
    vi.mocked(groupsApi.endDelegatedConnection).mockResolvedValue(undefined);
    const user = userEvent.setup();
    renderSection({ mode: 'member', connections: [link()] });

    await user.click(await screen.findByTestId('delegation-decline'));

    await waitFor(() => expect(groupsApi.endDelegatedConnection).toHaveBeenCalledWith(GROUP_ID, 'dc-1'));
    expect(await screen.findByText('Link declined')).toBeInTheDocument();
  });

  it('words a confirm refused for the member own login', async () => {
    vi.mocked(groupsApi.confirmDelegatedConnection).mockRejectedValue(refusal(409, 'own_connection'));
    const user = userEvent.setup();
    renderSection({ mode: 'member', connections: [link()] });

    await user.click(await screen.findByTestId('delegation-confirm'));

    expect(
      await screen.findByText(
        'You already connect TrainingPeaks with your own account, and Dravr keeps reading that one.',
      ),
    ).toBeInTheDocument();
  });

  it('words a confirm refused until the member verifies their email', async () => {
    vi.mocked(groupsApi.confirmDelegatedConnection).mockRejectedValue(refusal(400, 'dravr_email_unverified'));
    const user = userEvent.setup();
    renderSection({ mode: 'member', connections: [link()] });

    await user.click(await screen.findByTestId('delegation-confirm'));

    expect(
      await screen.findByText(
        'Verify your Dravr email first: open the link in the confirmation email we sent you.',
      ),
    ).toBeInTheDocument();
  });

  it('tells the member why their confirmed link reads nothing', async () => {
    vi.mocked(providersApi.getProvidersStatus).mockResolvedValue({ providers: [] });
    renderSection({
      mode: 'member',
      connections: [
        link({
          status: 'confirmed',
          confirmed_at: '2026-09-24T09:00:00Z',
          read_refused: 'athlete_email_mismatch',
        }),
      ],
    });

    const linked = await screen.findByTestId('delegation-linked');
    expect(within(linked).getByTestId('delegation-read-refused')).toHaveTextContent(
      "This athlete's TrainingPeaks email is not the member's Dravr email, so they cannot be linked.",
    );
    expect(linked).not.toHaveTextContent("Casey Coach's account");
  });

  it('says when the coach must reconnect, and unlinks behind a confirm', async () => {
    vi.mocked(providersApi.getProvidersStatus).mockResolvedValue({
      providers: [
        {
          provider: 'sciotte_trainingpeaks',
          display_name: 'TrainingPeaks',
          requires_oauth: false,
          connected: true,
          needs_reauth: false,
          capabilities: [],
          consent_required: false,
          delegation: {
            connection_id: 'dc-1',
            group_id: GROUP_ID,
            group_name: 'Marathon Squad',
            coach_display_name: 'Casey Coach',
            status: 'confirmed',
            coach_needs_reauth: true,
            read_refused: null,
          },
        },
      ],
    });
    vi.mocked(groupsApi.endDelegatedConnection).mockResolvedValue(undefined);
    const user = userEvent.setup();
    renderSection({
      mode: 'member',
      connections: [link({ status: 'confirmed', confirmed_at: '2026-09-24T09:00:00Z' })],
    });

    const linked = await screen.findByTestId('delegation-linked');
    expect(
      await within(linked).findByText(
        'Casey Coach needs to reconnect TrainingPeaks; your workouts are paused until then.',
      ),
    ).toBeInTheDocument();

    await user.click(within(linked).getByTestId('delegation-unlink-dc-1'));
    const dialog = await screen.findByRole('dialog');
    expect(dialog).toHaveTextContent('Unlink TrainingPeaks?');
    await user.click(within(dialog).getByRole('button', { name: 'Unlink' }));
    await waitFor(() => expect(groupsApi.endDelegatedConnection).toHaveBeenCalledWith(GROUP_ID, 'dc-1'));
  });
});
