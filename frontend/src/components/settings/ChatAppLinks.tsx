// ABOUTME: Settings → Messaging groups for the chat apps linked to the account and the ones still available to link
// ABOUTME: Web counterpart of mobile MessagingChannelsScreen — listLinks / initLink / deleteLink, link shown as QR + deep link

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import type { AvailableChannel, ChannelLink, LinkInitResponse } from '@pierre/api-client';
import { CHANNEL_LINK_POLL_INTERVAL_MS } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { describeApiError } from '@pierre/ui-logic';
import { messagingLinkApi } from '../../services/api';
import { ConfirmDialog, EmptyState, Modal, Section } from '../ui';
import ChannelLinkPanel from '../ChannelLinkPanel';
import { useAvailableChannels } from '../../hooks/useAvailableChannels';

// The same cache entries the onboarding steps read, so a link made in either
// place is already known to the other.
const LINKS_QUERY_KEY = ['messaging-links'] as const;

/** Stable empty list, so a missing or malformed links read keeps identity across renders. */
const EMPTY_LINKS: ChannelLink[] = [];

/** The links as an array — a malformed body (an HTML error page) reads as none, never as a crash. */
function asLinks(data: unknown): ChannelLink[] {
  return Array.isArray(data) ? (data as ChannelLink[]) : EMPTY_LINKS;
}

interface LinkInProgress {
  channel: AvailableChannel;
  link: LinkInitResponse;
}

/** The longest delay `setTimeout` honours (2^31 − 1 ms); a longer one overflows and fires at once. */
const MAX_TIMER_DELAY_MS = 2_147_483_647;

/** A started link the pane is still watching for: the channel and when its code expires (epoch ms). */
interface AwaitedLink {
  channel: string;
  until: number;
}

const ROW_CLASS =
  'flex min-h-[52px] items-center justify-between gap-3 border-t ghost-border-faint py-3 first:border-t-0';
const INK_ACTION_CLASS =
  'shrink-0 rounded text-sm font-medium text-primary transition-colors hover:text-primary-hover focus-ring disabled:opacity-50';

/**
 * The chat apps linked to the account, and the ones the workspace offers that
 * are not linked yet.
 *
 * A linked app can be unlinked after a confirmation; an available one can be
 * connected. Connecting opens the link panel the onboarding step uses — a QR
 * plus the deep link, since on a desktop the chat app is on the athlete's
 * phone — and the pane polls the linked channels until that link lands or its
 * code expires. The poll outlives the panel: closing it does not stop the
 * pane from noticing a link finished on the phone afterwards. The mobile
 * counterpart is `MessagingChannelsScreen`, on the same shared calls.
 */
export default function ChatAppLinks() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [inProgress, setInProgress] = useState<LinkInProgress | null>(null);
  const [awaited, setAwaited] = useState<AwaitedLink | null>(null);
  const [linkToRemove, setLinkToRemove] = useState<ChannelLink | null>(null);
  const [actionError, setActionError] = useState<{ title: string; detail: string } | null>(null);

  const waitingFor = awaited?.channel ?? null;

  const linksQuery = useQuery({
    queryKey: LINKS_QUERY_KEY,
    queryFn: () => messagingLinkApi.listLinks(),
    // Poll only while a started link is still live; the wait ends the moment
    // the channel appears or the code expires, so an idle pane asks nothing.
    // A failed poll keeps polling: the interval outlives an error.
    refetchInterval: (query) =>
      waitingFor !== null && !asLinks(query.state.data).some((l) => l.channel === waitingFor)
        ? CHANNEL_LINK_POLL_INTERVAL_MS
        : false,
  });

  // Opening the pane re-reads what the workspace offers rather than trusting
  // the list onboarding cached at sign-in.
  const { channels: available, query: availableQuery } = useAvailableChannels({ staleTime: 0 });

  const initMutation = useMutation({
    mutationFn: (channel: AvailableChannel) => messagingLinkApi.initLink(channel.channel),
    onMutate: () => setActionError(null),
    onSuccess: (link, channel) => {
      setInProgress({ channel, link });
      setAwaited({ channel: channel.channel, until: Date.parse(link.expires_at) });
    },
    onError: (err, channel) => {
      setActionError({
        title: t('app.couldNotStartConnection', { channel: channel.display_name }),
        detail: describeApiError(err, { t, fallbackKey: 'app.failedLoadChannels' }),
      });
    },
  });

  const unlinkMutation = useMutation({
    mutationFn: (channel: string) => messagingLinkApi.deleteLink(channel),
    onMutate: () => setActionError(null),
    onSuccess: () => {
      setLinkToRemove(null);
      return queryClient.invalidateQueries({ queryKey: LINKS_QUERY_KEY });
    },
    onError: (err) => {
      setLinkToRemove(null);
      setActionError({
        title: t('app.couldNotUnlink'),
        detail: describeApiError(err, { t, fallbackKey: 'app.failedUnlinkChannel' }),
      });
    },
  });

  const links = asLinks(linksQuery.data);
  // The link landed server-side: stop watching and close the panel if it is
  // still open; the Linked group now shows it.
  useEffect(() => {
    if (waitingFor !== null && links.some((l) => l.channel === waitingFor)) {
      setAwaited(null);
      setInProgress(null);
    }
  }, [links, waitingFor]);

  // The code expired: nothing can land from it any more, so stop polling and
  // take down a panel still showing its dead QR. An unparseable expiry leaves
  // the watch to end when the link lands or the pane unmounts.
  const awaitedUntil = awaited?.until;
  useEffect(() => {
    if (awaitedUntil === undefined || !Number.isFinite(awaitedUntil)) return undefined;
    const timer = setTimeout(() => {
      setAwaited(null);
      setInProgress(null);
    }, Math.min(MAX_TIMER_DELAY_MS, Math.max(0, awaitedUntil - Date.now())));
    return () => clearTimeout(timer);
  }, [awaitedUntil]);

  // Only a read that never produced data is a load failure. A background
  // refetch that fails — one poll answered 502 while the QR is on screen —
  // keeps the cached lists, and the open link panel, where they are.
  const loadError =
    (linksQuery.data === undefined ? linksQuery.error : null) ??
    (availableQuery.data === undefined ? availableQuery.error : null);
  if (linksQuery.isLoading || availableQuery.isLoading) {
    return (
      <div className="flex justify-center py-8" data-testid="chat-app-links-loading">
        <div className="pierre-spinner w-6 h-6" />
      </div>
    );
  }

  if (loadError) {
    // Surface the failure rather than an empty list, which would read as "no
    // chat app linked" and invite re-linking one the athlete already has.
    return (
      <p className="py-3 text-sm text-error" role="alert" data-testid="chat-app-links-error">
        {describeApiError(loadError, { t, fallbackKey: 'app.failedLoadChannels' })}{' '}
        <button
          type="button"
          className="rounded font-medium text-primary hover:text-primary-hover focus-ring"
          onClick={() => {
            void linksQuery.refetch();
            void availableQuery.refetch();
          }}
          data-testid="chat-app-links-retry"
        >
          {t('common.retry')}
        </button>
      </p>
    );
  }

  const linked = links;
  const linkedChannels = new Set(linked.map((l) => l.channel));
  const unlinked = available.filter((c) => !linkedChannels.has(c.channel));
  const nameOf = (channel: string) => available.find((c) => c.channel === channel)?.display_name ?? channel;
  const busyInit = initMutation.isPending ? initMutation.variables?.channel : null;

  return (
    <>
      {actionError && (
        <p className="text-sm text-error" role="alert" data-testid="chat-app-links-action-error">
          <span className="font-medium">{actionError.title}</span> {actionError.detail}
        </p>
      )}

      <Section title={t('app.linked')} data-testid="chat-app-linked-section">
        {linked.length === 0 ? (
          <EmptyState data-testid="chat-app-no-links">
            {/* "Link one below" points at nothing when the workspace has no
                chat app set up, so that case says what is actually true. */}
            {available.length === 0 ? t('app.noChatAppsAvailableYet') : t('app.noChatAppsLinked')}
          </EmptyState>
        ) : (
          <div>
            {linked.map((link) => (
              <div key={link.channel} className={ROW_CLASS} data-testid={`chat-app-link-${link.channel}`}>
                <div className="min-w-0">
                  <p className="text-sm font-medium text-on-surface">{nameOf(link.channel)}</p>
                  <p className="truncate text-sm text-on-surface-variant">
                    {link.display_name ?? link.channel_user_id}
                  </p>
                </div>
                <button
                  type="button"
                  className={INK_ACTION_CLASS}
                  onClick={() => setLinkToRemove(link)}
                  disabled={unlinkMutation.isPending}
                  data-testid={`chat-app-unlink-${link.channel}`}
                >
                  {t('app.unlink')}
                </button>
              </div>
            ))}
          </div>
        )}
      </Section>

      <Section title={t('app.available')} data-testid="chat-app-available-section">
        {/* An empty list has two causes and they are not the same news:
            nothing set up for the workspace is not "you linked everything". */}
        {available.length === 0 ? (
          <EmptyState data-testid="chat-app-none-configured">{t('app.noChatAppsConfigured')}</EmptyState>
        ) : unlinked.length === 0 ? (
          <EmptyState data-testid="chat-app-all-linked">{t('app.everyChatAppLinked')}</EmptyState>
        ) : (
          <div>
            {unlinked.map((channel) => (
              <div key={channel.channel} className={ROW_CLASS} data-testid={`chat-app-add-${channel.channel}`}>
                <p className="min-w-0 text-sm font-medium text-on-surface">{channel.display_name}</p>
                {busyInit === channel.channel ? (
                  <span className="pierre-spinner w-4 h-4 border-on-surface border-t-transparent" />
                ) : (
                  <button
                    type="button"
                    className={INK_ACTION_CLASS}
                    onClick={() => initMutation.mutate(channel)}
                    disabled={initMutation.isPending}
                    data-testid={`chat-app-connect-${channel.channel}`}
                  >
                    {t('app.connect')}
                  </button>
                )}
              </div>
            ))}
          </div>
        )}
      </Section>

      <Modal
        isOpen={inProgress !== null}
        onClose={() => setInProgress(null)}
        title={inProgress ? t('app.connectChannelTitle', { channel: inProgress.channel.display_name }) : undefined}
        size="sm"
      >
        {inProgress && <ChannelLinkPanel link={inProgress.link} displayName={inProgress.channel.display_name} />}
      </Modal>

      <ConfirmDialog
        isOpen={linkToRemove !== null}
        onClose={() => setLinkToRemove(null)}
        onConfirm={() => linkToRemove && unlinkMutation.mutate(linkToRemove.channel)}
        title={t('app.confirmUnlinkChannel', { channel: linkToRemove ? nameOf(linkToRemove.channel) : '' })}
        message={t('app.unlinkChannelWarning')}
        confirmLabel={t('app.unlink')}
        cancelLabel={t('common.cancel')}
        variant="danger"
        isLoading={unlinkMutation.isPending}
      />
    </>
  );
}
