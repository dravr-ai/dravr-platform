// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Group info's chat-app section — the group chat the room is connected to, and for its owner or coach the way to connect one
// ABOUTME: Connecting mints a one-time code: open the chat app to add the bot, scan it on desktop, or type the command (carnet#1683)

import { useEffect, useState, type ReactNode } from 'react';
import { Check, Copy, MessagesSquare, Plus } from 'lucide-react';
import type { GroupChatChannel, GroupChatConnectCode } from '@pierre/shared-types';
import { CHANNEL_BRAND } from '@pierre/shared-constants';
import { formatMessageTime } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';
import { apiErrorStatus, describeApiError } from '@pierre/ui-logic';
import {
  useCreateGroupChatConnection,
  useDeleteGroupChatConnection,
  useGroupChatConnection,
} from '../../hooks/useGroups';
import { useIsDesktop } from '../../hooks/useBreakpoint';
import { Button, ConfirmDialog, IconButton, Section, useErrorToast, useSuccessToast } from '../ui';
import QrPlate from '../QrPlate';
import { PersonRow, ROW_DIVIDER, RowBox } from './GroupInfoParts';

/** How long the copy button shows its tick after the command was copied. */
const COPIED_TICK_MS = 2000;

/** The chat apps offered, in order; only a `connectable` one can be chosen. */
const CHAT_APPS: { channel: GroupChatChannel; connectable: boolean }[] = [
  { channel: 'telegram', connectable: true },
  { channel: 'whatsapp', connectable: false },
];

/** The glyph a chat row leads with: a group chat, on the quiet tint. */
function ChatGlyph() {
  return (
    <span
      aria-hidden="true"
      className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-surface-container-high text-on-surface-variant"
    >
      <MessagesSquare className="h-[18px] w-[18px]" />
    </span>
  );
}

/**
 * The group's room in a chat app's group chat.
 *
 * Every member reads which chat the room is connected to; the server's
 * `can_manage` — the group's owner or its human coach, a member or not —
 * decides who may connect or disconnect it. Nothing is drawn before the first
 * read, and nothing for a member while no chat is connected.
 *
 * Connecting mints a one-time code. The code travels in a deep link that opens
 * the chat app's "add to group" picker, as a QR on a desktop, and as a command
 * for a chat the bot is already in. The connection is read again every few
 * seconds while the code is out, until the bot joins or the code expires.
 */
export default function GroupChatConnection({ groupId }: { groupId: string }) {
  const { t, language } = useTranslation();
  const isDesktop = useIsDesktop();
  const showError = useErrorToast();
  const showSuccess = useSuccessToast();

  const [choosing, setChoosing] = useState(false);
  const [code, setCode] = useState<GroupChatConnectCode | null>(null);
  const [expired, setExpired] = useState(false);
  const [copied, setCopied] = useState(false);
  const [confirmDisconnect, setConfirmDisconnect] = useState(false);

  const { connection, refetch } = useGroupChatConnection(groupId, code !== null && !expired);
  const { createCode, isPending: isCreating } = useCreateGroupChatConnection(groupId);
  const { disconnect, isPending: isDisconnecting } = useDeleteGroupChatConnection(groupId);

  // The server names the chat app whenever the room is connected.
  const connectedChannel = connection?.connected ? connection.channel : null;

  // The bot joined: the code did its job, and the section turns to the chat.
  useEffect(() => {
    if (connectedChannel === null || code === null) return;
    showSuccess(t('humanCoach.groupChat.web.connectedTo', { app: CHANNEL_BRAND[connectedChannel] }));
    setCode(null);
    setExpired(false);
  }, [connectedChannel, code, showSuccess, t]);

  // The code stops working at its expiry: say so then, and stop asking.
  useEffect(() => {
    if (code === null) return;
    const left = Date.parse(code.expires_at) - Date.now();
    if (!(left > 0)) {
      setExpired(true);
      return;
    }
    const timer = setTimeout(() => setExpired(true), left);
    return () => clearTimeout(timer);
  }, [code]);

  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), COPIED_TICK_MS);
    return () => clearTimeout(timer);
  }, [copied]);

  if (!connection) return null;
  const canManage = connection.can_manage;
  if (connectedChannel === null && !canManage) return null;

  const handleConnect = async (channel: GroupChatChannel) => {
    try {
      const minted = await createCode(channel);
      setExpired(false);
      setCopied(false);
      setCode(minted);
      setChoosing(false);
    } catch (err) {
      if (apiErrorStatus(err) === 409) {
        // Someone connected the room meanwhile: show the chat it is connected to.
        setCode(null);
        setChoosing(false);
        void refetch();
        showError(t('humanCoach.groupChat.web.alreadyConnected'));
        return;
      }
      showError(describeApiError(err, { t, fallbackKey: 'humanCoach.groupChat.web.connectFailed' }));
    }
  };

  const handleCancel = () => {
    setCode(null);
    setExpired(false);
    setChoosing(false);
  };

  const handleCopy = async (command: string) => {
    try {
      await navigator.clipboard.writeText(command);
      setCopied(true);
    } catch {
      showError(t('app.copyFailed'), t('app.couldNotCopyClipboard'));
    }
  };

  const handleDisconnect = async () => {
    try {
      await disconnect();
      setConfirmDisconnect(false);
      showSuccess(t('humanCoach.groupChat.web.disconnected'));
    } catch (err) {
      setConfirmDisconnect(false);
      // Already disconnected elsewhere: the refreshed read shows it.
      if (apiErrorStatus(err) === 404) return;
      showError(describeApiError(err, { t, fallbackKey: 'humanCoach.groupChat.web.disconnectFailed' }));
    }
  };

  const cancel = (
    <Button variant="tertiary" onClick={handleCancel} className="mt-2" data-testid="group-chat-cancel">
      {t('settingsUi.cancel')}
    </Button>
  );

  let body: ReactNode;
  if (connectedChannel !== null) {
    const app = CHANNEL_BRAND[connectedChannel];
    body = (
      <RowBox>
        <PersonRow
          data-testid="group-chat-connected"
          avatar={<ChatGlyph />}
          title={
            <span data-testid="group-chat-title">
              {connection.chat_title ?? t('humanCoach.groupChat.web.untitledChat', { app })}
            </span>
          }
          detail={
            <span className="inline-flex items-center gap-1.5">
              <span aria-hidden="true" className="h-2 w-2 shrink-0 rounded-full bg-success" />
              <span className="font-medium text-on-success-container" data-testid="group-chat-connected-to">
                {t('humanCoach.groupChat.web.connectedTo', { app })}
              </span>
            </span>
          }
          trailing={
            canManage ? (
              <Button
                variant="tertiary"
                size="sm"
                onClick={() => setConfirmDisconnect(true)}
                data-testid="group-chat-disconnect"
              >
                {t('app.disconnect')}
              </Button>
            ) : undefined
          }
        />
        {canManage && (
          <ConfirmDialog
            isOpen={confirmDisconnect}
            onClose={() => setConfirmDisconnect(false)}
            onConfirm={() => void handleDisconnect()}
            title={t('humanCoach.groupChat.web.disconnectTitle', { app })}
            message={t('humanCoach.groupChat.web.disconnectBody')}
            confirmLabel={t('app.disconnect')}
            variant="danger"
            isLoading={isDisconnecting}
          />
        )}
      </RowBox>
    );
  } else if (code !== null && expired) {
    body = (
      <>
        <RowBox className="space-y-3 p-4" data-testid="group-chat-expired">
          <p className="text-sm text-on-surface">{t('humanCoach.groupChat.web.expired')}</p>
          <Button
            variant="secondary"
            onClick={() => void handleConnect(code.channel)}
            loading={isCreating}
            data-testid="group-chat-new-code"
          >
            {t('humanCoach.groupChat.web.newCode')}
          </Button>
        </RowBox>
        {cancel}
      </>
    );
  } else if (code !== null) {
    const app = CHANNEL_BRAND[code.channel];
    body = (
      <>
        <RowBox className="space-y-4 p-4" data-testid="group-chat-pending">
          <p className="text-sm text-on-surface">{t('humanCoach.groupChat.web.pendingIntro', { app })}</p>
          <a
            href={code.deep_link_url}
            target="_blank"
            rel="noreferrer"
            className="btn-base btn-primary w-full"
            data-testid="group-chat-open-app"
          >
            {t('app.openChannel', { channel: app })}
          </a>
          {/* Scanned with the phone the chat app runs on: never offered on that phone. */}
          {isDesktop && code.qr_svg ? (
            <div className="flex flex-col items-center gap-2">
              <QrPlate svg={code.qr_svg} alt={t('frag.qrCodeFor', { app })} data-testid="group-chat-qr" />
              <p className="text-center text-xs text-on-surface-variant">{t('humanCoach.groupChat.web.scanHint', { app })}</p>
            </div>
          ) : null}
          <div>
            <p className="text-xs text-on-surface-variant">{t('humanCoach.groupChat.web.commandLabel')}</p>
            <div className="mt-1 flex items-center gap-2 rounded-lg bg-surface-container px-3 py-1.5">
              <code className="min-w-0 flex-1 truncate font-mono text-sm text-on-surface" data-testid="group-chat-command">
                {code.command}
              </code>
              <IconButton
                onClick={() => void handleCopy(code.command)}
                title={t('humanCoach.groupChat.web.copyCommand')}
                aria-label={t('humanCoach.groupChat.web.copyCommand')}
                data-testid="group-chat-copy-command"
              >
                {copied ? (
                  <Check className="h-4 w-4 text-primary" aria-hidden="true" />
                ) : (
                  <Copy className="h-4 w-4" aria-hidden="true" />
                )}
              </IconButton>
            </div>
          </div>
          <div className="flex items-center gap-2 text-xs text-on-surface-variant" role="status">
            <span className="pierre-spinner h-3.5 w-3.5 border-on-surface border-t-transparent" aria-hidden="true" />
            <span>{t('humanCoach.groupChat.web.waiting')}</span>
          </div>
          <p className="text-xs text-on-surface-variant" data-testid="group-chat-expiry">
            {t('humanCoach.groupChat.web.expiresAt', { time: formatMessageTime(code.expires_at, language) })}
          </p>
        </RowBox>
        {cancel}
      </>
    );
  } else if (choosing) {
    body = (
      <>
        <RowBox data-testid="group-chat-choose">
          {CHAT_APPS.map(({ channel, connectable }) => (
            <button
              key={channel}
              type="button"
              onClick={() => void handleConnect(channel)}
              disabled={!connectable || isCreating}
              data-testid={`group-chat-choose-${channel}`}
              className={`flex min-h-11 w-full items-center gap-3 rounded-xl px-4 py-2.5 text-left text-sm transition-colors enabled:hover:bg-surface-container-low focus-ring disabled:cursor-not-allowed ${ROW_DIVIDER}`}
            >
              <span className={`min-w-0 flex-1 font-medium ${connectable ? 'text-on-surface' : 'text-on-surface-variant'}`}>
                {CHANNEL_BRAND[channel]}
              </span>
              {!connectable ? (
                <span className="shrink-0 text-xs text-on-surface-variant">{t('humanCoach.groupChat.web.comingSoon')}</span>
              ) : isCreating ? (
                <span className="flex shrink-0 items-center gap-2 text-xs text-on-surface-variant">
                  <span className="pierre-spinner h-4 w-4 border-on-surface border-t-transparent" aria-hidden="true" />
                  {t('app.preparingLink', { channel: CHANNEL_BRAND[channel] })}
                </span>
              ) : (
                <span className="shrink-0 font-medium text-primary">{t('app.connect')}</span>
              )}
            </button>
          ))}
        </RowBox>
        {cancel}
      </>
    );
  } else {
    body = (
      <>
        <RowBox>
          <PersonRow
            data-testid="group-chat-not-connected"
            avatar={<ChatGlyph />}
            title={t('humanCoach.groupChat.web.notConnected')}
            detail={<span>{t('humanCoach.groupChat.web.notConnectedHint')}</span>}
          />
        </RowBox>
        <Button variant="tertiary" onClick={() => setChoosing(true)} className="mt-2" data-testid="group-chat-connect">
          <Plus className="mr-1.5 h-4 w-4" aria-hidden="true" />
          {t('humanCoach.groupChat.web.connect')}
        </Button>
      </>
    );
  }

  return (
    <Section headingLevel={3} title={t('humanCoach.groupChat.web.title')} data-testid="group-chat-connection">
      {body}
    </Section>
  );
}
