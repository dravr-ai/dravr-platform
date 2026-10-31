// ABOUTME: Catalogue keys of the hosted pages a chat link opens: connect picker, Intervals.icu form, hosted login
// ABOUTME: One constant per string, plus the slot-less set a page template names directly as `{{t:<key>}}`
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Keys of the server-rendered hosted pages.
//!
//! The pages `/connect` opens from a chat — the provider picker, the
//! Intervals.icu connect form, the hosted provider login, and the success and
//! link-error pages they end on — are rendered by `pierre-routes-auth` in the
//! athlete's own locale. Every string they show is a catalogue key declared
//! here, read through
//! [`MessagingStringsRegistry`](crate::MessagingStringsRegistry).
//!
//! A key whose text carries a positional slot (`{0}`) is filled by the
//! renderer that knows the value. A key with no slot is named by the template
//! itself, as `{{t:<key>}}`; [`TEMPLATE_KEYS`] is the set a renderer
//! substitutes, so a template can only name a key this module declares.

// ── Shared by the picker, the form and the hosted login ──

/// Key: "Connect your fitness account" No format placeholders.
pub const KEY_HOSTED_COMMON_SUBTITLE: &str = "hosted.common.subtitle";
/// Key: "Connect your `{0}` account"
pub const KEY_HOSTED_COMMON_SUBTITLE_NAMED: &str = "hosted.common.subtitleNamed";
/// Key: "Linked from `{0}`"
pub const KEY_HOSTED_COMMON_LINKED_FROM: &str = "hosted.common.linkedFrom";
/// Key: "your chat app" No format placeholders.
pub const KEY_HOSTED_COMMON_YOUR_CHAT_APP: &str = "hosted.common.yourChatApp";
/// Key: "Back" No format placeholders.
pub const KEY_HOSTED_COMMON_BACK: &str = "hosted.common.back";
/// Key: "`{0}` Account"
pub const KEY_HOSTED_COMMON_ACCOUNT_TITLE: &str = "hosted.common.accountTitle";
/// Key: "Email" No format placeholders.
pub const KEY_HOSTED_COMMON_EMAIL_LABEL: &str = "hosted.common.emailLabel";
/// Key: "Email or username" No format placeholders.
pub const KEY_HOSTED_COMMON_EMAIL_OR_USERNAME_LABEL: &str = "hosted.common.emailOrUsernameLabel";
/// Key: "Username" No format placeholders.
pub const KEY_HOSTED_COMMON_USERNAME_LABEL: &str = "hosted.common.usernameLabel";
/// Key: "Password" No format placeholders.
pub const KEY_HOSTED_COMMON_PASSWORD_LABEL: &str = "hosted.common.passwordLabel";
/// Key: "Log In" No format placeholders.
pub const KEY_HOSTED_COMMON_LOG_IN: &str = "hosted.common.logIn";
/// Key: "Credentials go directly to `{0}` — Dravr never stores your password."
pub const KEY_HOSTED_COMMON_CREDENTIALS_NOTE: &str = "hosted.common.credentialsNote";
/// Key: "Logging in" No format placeholders.
pub const KEY_HOSTED_COMMON_LOGGING_IN_ARIA: &str = "hosted.common.loggingInAria";
/// Key: "Connecting to `{0}`..."
pub const KEY_HOSTED_COMMON_CONNECTING_TO: &str = "hosted.common.connectingTo";
/// Key: "This may take up to 30 seconds." No format placeholders.
pub const KEY_HOSTED_COMMON_MAY_TAKE_TIME: &str = "hosted.common.mayTakeTime";
/// Key: "Loading..." No format placeholders.
pub const KEY_HOSTED_COMMON_LOADING: &str = "hosted.common.loading";
/// Key: "Verifying code..." No format placeholders.
pub const KEY_HOSTED_COMMON_VERIFYING_CODE: &str = "hosted.common.verifyingCode";
/// Key: "2-Step Verification" No format placeholders.
pub const KEY_HOSTED_COMMON_TWO_STEP_TITLE: &str = "hosted.common.twoStepTitle";
/// Key: "Waiting for approval" No format placeholders.
pub const KEY_HOSTED_COMMON_WAITING_APPROVAL_ARIA: &str = "hosted.common.waitingApprovalAria";
/// Key: "Check your phone" No format placeholders.
pub const KEY_HOSTED_COMMON_CHECK_PHONE: &str = "hosted.common.checkPhone";
/// Key: "Tap Yes on the notification" No format placeholders.
pub const KEY_HOSTED_COMMON_TAP_YES: &str = "hosted.common.tapYes";
/// Key: "Tap this number on your phone" No format placeholders.
pub const KEY_HOSTED_COMMON_TAP_NUMBER: &str = "hosted.common.tapNumber";
/// Key: "Check the notification on your device" No format placeholders.
pub const KEY_HOSTED_COMMON_CHECK_NOTIFICATION: &str = "hosted.common.checkNotification";
/// Key: "Waiting" No format placeholders.
pub const KEY_HOSTED_COMMON_WAITING_ARIA: &str = "hosted.common.waitingAria";
/// Key: "Verification Code" No format placeholders.
pub const KEY_HOSTED_COMMON_OTP_TITLE: &str = "hosted.common.otpTitle";
/// Key: "Enter the 6-digit code `{0}` sent you"
pub const KEY_HOSTED_COMMON_OTP_LABEL_NAMED: &str = "hosted.common.otpLabelNamed";
/// Key: "Verify" No format placeholders.
pub const KEY_HOSTED_COMMON_VERIFY: &str = "hosted.common.verify";
/// Key: "Connected!" No format placeholders.
pub const KEY_HOSTED_COMMON_CONNECTED: &str = "hosted.common.connected";
/// Key: "Login failed" No format placeholders.
pub const KEY_HOSTED_COMMON_LOGIN_FAILED: &str = "hosted.common.loginFailed";
/// Key: "`{0}` didn't accept that sign-in. Check your details and try again."
pub const KEY_HOSTED_COMMON_SIGN_IN_REJECTED: &str = "hosted.common.signInRejected";
/// Key: "We couldn't sign you in to `{0}` right now. This is usually temporary — please try again in a few minutes."
pub const KEY_HOSTED_COMMON_SIGN_IN_UNAVAILABLE: &str = "hosted.common.signInUnavailable";
/// Key: "`{0}` asked for an extra security check we can't complete automatically right now. Please try again in a little while."
pub const KEY_HOSTED_COMMON_SIGN_IN_CHALLENGED: &str = "hosted.common.signInChallenged";
/// Key: "This sign-in is no longer active. Please start the sign-in again." No format placeholders.
pub const KEY_HOSTED_COMMON_SIGN_IN_EXPIRED: &str = "hosted.common.signInExpired";
/// Key: "Try Again" No format placeholders.
pub const KEY_HOSTED_COMMON_TRY_AGAIN: &str = "hosted.common.tryAgain";

// ── Provider picker ──

/// Key: "Connect a provider - Dravr" No format placeholders.
pub const KEY_HOSTED_PICKER_PAGE_TITLE: &str = "hosted.picker.pageTitle";
/// Key: "Choose a provider" No format placeholders.
pub const KEY_HOSTED_PICKER_HEADING: &str = "hosted.picker.heading";
/// Key: "You'll connect securely here — your password is never shared with this chat." No format placeholders.
pub const KEY_HOSTED_PICKER_SECURITY_NOTE: &str = "hosted.picker.securityNote";
/// Key: "Continue" No format placeholders.
pub const KEY_HOSTED_PICKER_CONTINUE: &str = "hosted.picker.continue";
/// Key: "Your data is now available. You can return to `{0}`."
pub const KEY_HOSTED_PICKER_DATA_AVAILABLE: &str = "hosted.picker.dataAvailable";
/// Key: "Something went wrong" No format placeholders.
pub const KEY_HOSTED_PICKER_ERROR_TITLE: &str = "hosted.picker.errorTitle";
/// Key: "Back to providers" No format placeholders.
pub const KEY_HOSTED_PICKER_BACK_TO_PROVIDERS: &str = "hosted.picker.backToProviders";
/// Key: "Connected" No format placeholders.
pub const KEY_HOSTED_PICKER_TAG_CONNECTED: &str = "hosted.picker.tagConnected";
/// Key: "Authorize" No format placeholders.
pub const KEY_HOSTED_PICKER_TAG_AUTHORIZE: &str = "hosted.picker.tagAuthorize";
/// Key: "Athlete ID / API key" No format placeholders.
pub const KEY_HOSTED_PICKER_TAG_API_KEY: &str = "hosted.picker.tagApiKey";
/// Key: "Username / password" No format placeholders.
pub const KEY_HOSTED_PICKER_TAG_USERNAME_PASSWORD: &str = "hosted.picker.tagUsernamePassword";
/// Key: "Email / password" No format placeholders.
pub const KEY_HOSTED_PICKER_TAG_EMAIL_PASSWORD: &str = "hosted.picker.tagEmailPassword";
/// Key: "Email or username / password" No format placeholders.
pub const KEY_HOSTED_PICKER_TAG_EMAIL_OR_USERNAME_PASSWORD: &str =
    "hosted.picker.tagEmailOrUsernamePassword";
/// Key: "Import my history" No format placeholders.
pub const KEY_HOSTED_PICKER_TAG_IMPORT_HISTORY: &str = "hosted.picker.tagImportHistory";
/// Key: "Strava sign-in didn't complete — you can connect with your Strava email and password instead." No format placeholders.
pub const KEY_HOSTED_PICKER_STRAVA_FALLBACK: &str = "hosted.picker.stravaFallback";
/// Key: "That sign-in didn't complete. Please go back and try again." No format placeholders.
pub const KEY_HOSTED_PICKER_SIGN_IN_INCOMPLETE: &str = "hosted.picker.signInIncomplete";

// ── Hosted provider login ──

/// Key: "Connect your account - Dravr" No format placeholders.
pub const KEY_HOSTED_LOGIN_PAGE_TITLE: &str = "hosted.login.pageTitle";
/// Key: "Your `{0}` data is now available. You can return to `{1}`."
pub const KEY_HOSTED_LOGIN_DATA_AVAILABLE: &str = "hosted.login.dataAvailable";

// ── Intervals.icu connect form ──

/// Key: "Connect Intervals.icu - Dravr" No format placeholders.
pub const KEY_HOSTED_INTERVALS_PAGE_TITLE: &str = "hosted.intervals.pageTitle";
/// Key: "Dravr checks them with Intervals.icu before saving, and stores the key encrypted." No format placeholders.
pub const KEY_HOSTED_INTERVALS_STORAGE_NOTE: &str = "hosted.intervals.storageNote";
/// Key: "Enter your Athlete ID." No format placeholders.
pub const KEY_HOSTED_INTERVALS_ATHLETE_ID_REQUIRED: &str = "hosted.intervals.athleteIdRequired";
/// Key: "Enter your API key." No format placeholders.
pub const KEY_HOSTED_INTERVALS_API_KEY_REQUIRED: &str = "hosted.intervals.apiKeyRequired";
/// Key: "Intervals.icu rejected those credentials. Check your Athlete ID and API key, then try again." No format placeholders.
pub const KEY_HOSTED_INTERVALS_REJECTED: &str = "hosted.intervals.rejected";
/// Key: "Intervals.icu can't be connected right now. Please try again later." No format placeholders.
pub const KEY_HOSTED_INTERVALS_UNAVAILABLE: &str = "hosted.intervals.unavailable";
/// Key: "We could not save your Intervals.icu connection. Please try again in a moment." No format placeholders.
pub const KEY_HOSTED_INTERVALS_SAVE_FAILED: &str = "hosted.intervals.saveFailed";
/// Key: "This Intervals.icu account belonged to a deleted Dravr account. Start fresh with it from the Dravr app before connecting it here." No format placeholders.
pub const KEY_HOSTED_INTERVALS_TOMBSTONED: &str = "hosted.intervals.tombstoned";
/// Key: "You sign in on Intervals.icu and approve Dravr there — Dravr never sees your password." No format placeholders.
pub const KEY_HOSTED_INTERVALS_AUTHORIZE_NOTE: &str = "hosted.intervals.authorizeNote";
/// Key: "Human coaches link with their API key, so Dravr can read the athletes they train." No format placeholders.
pub const KEY_HOSTED_INTERVALS_COACH_NOTE: &str = "hosted.intervals.coachNote";

// ── Success page ──

/// Key: "Connected - Dravr" No format placeholders.
pub const KEY_HOSTED_SUCCESS_PAGE_TITLE: &str = "hosted.success.pageTitle";
/// Key: "`{0}` connected"
pub const KEY_HOSTED_SUCCESS_HEADING: &str = "hosted.success.heading";
/// Key: "Account connected" No format placeholders.
pub const KEY_HOSTED_SUCCESS_HEADING_GENERIC: &str = "hosted.success.headingGeneric";
/// Key: "Your `{0}` data is now available in Dravr."
pub const KEY_HOSTED_SUCCESS_DATA_AVAILABLE: &str = "hosted.success.dataAvailable";
/// Key: "Your data is now available in Dravr." No format placeholders.
pub const KEY_HOSTED_SUCCESS_DATA_AVAILABLE_GENERIC: &str = "hosted.success.dataAvailableGeneric";
/// Key: "You can return to `{0}` to continue chatting with your agent."
pub const KEY_HOSTED_SUCCESS_RETURN_TO_CHAT: &str = "hosted.success.returnToChat";
/// Key: "This window will close automatically." No format placeholders.
pub const KEY_HOSTED_SUCCESS_AUTO_CLOSE: &str = "hosted.success.autoClose";

// ── Link-error page ──

/// Key: "Link error - Dravr" No format placeholders.
pub const KEY_HOSTED_ERROR_PAGE_TITLE: &str = "hosted.error.pageTitle";
/// Key: "Can't connect your account" No format placeholders.
pub const KEY_HOSTED_ERROR_HEADING: &str = "hosted.error.heading";
/// Key: "This link is incomplete. Please request a fresh link from your chat." No format placeholders.
pub const KEY_HOSTED_ERROR_MISSING_TOKEN: &str = "hosted.error.missingToken";
/// Key: "This link is invalid or has expired. Please request a fresh link from your chat." No format placeholders.
pub const KEY_HOSTED_ERROR_INVALID_LINK: &str = "hosted.error.invalidLink";
/// Key: "This link is malformed. Please request a fresh link from your chat." No format placeholders.
pub const KEY_HOSTED_ERROR_MALFORMED_LINK: &str = "hosted.error.malformedLink";
/// Key: "This link has already been opened. For your security, each link can only be opened once. Please request a fresh link from your chat." No format placeholders.
pub const KEY_HOSTED_ERROR_LINK_ALREADY_OPENED: &str = "hosted.error.linkAlreadyOpened";
/// Key: "We could not load your connections. Please try again in a moment." No format placeholders.
pub const KEY_HOSTED_ERROR_LOAD_FAILED: &str = "hosted.error.loadFailed";
/// Key: "Connecting this provider needs its notice accepted first. Please go back, tick the notice and try again." No format placeholders.
pub const KEY_HOSTED_ERROR_NOTICE_REQUIRED: &str = "hosted.error.noticeRequired";
/// Key: "We couldn't start the connection for this provider. Please go back and try again." No format placeholders.
pub const KEY_HOSTED_ERROR_OAUTH_START_FAILED: &str = "hosted.error.oauthStartFailed";
/// Key: "Something went wrong connecting your account. Please request a fresh link from your chat." No format placeholders.
pub const KEY_HOSTED_ERROR_GENERIC: &str = "hosted.error.generic";

// ── Intervals.icu strings the app dialog shares ──
//
// The web and mobile Intervals.icu dialog reads these same keys, so the
// hosted form and the app name the two fields, where to find them, and the
// two ways in (authorize, or a coach's API key) in one wording.

/// Key: "Athlete ID" No format placeholders.
pub const KEY_INTERVALS_ATHLETE_ID: &str = "shell.intervalsAthleteId";
/// Key: "API Key" No format placeholders.
pub const KEY_INTERVALS_API_KEY_LABEL: &str = "shell.intervalsApiKeyLabel";
/// Key: "Connect" No format placeholders.
pub const KEY_INTERVALS_CONNECT_ACTION: &str = "shell.intervalsConnectAction";
/// Key: "Find your Athlete ID and API key in Intervals.icu: open Settings and scroll down to Developer Settings, near the bottom of the page." No format placeholders.
pub const KEY_INTERVALS_CREDENTIALS_HELP: &str = "shell.intervalsCredentialsHelp";
/// Key: "Authorize with Intervals.icu" No format placeholders.
pub const KEY_INTERVALS_AUTHORIZE_ACTION: &str = "shell.intervalsAuthorizeAction";
/// Key: "Coaching athletes? Link with your API key" No format placeholders.
pub const KEY_INTERVALS_COACH_KEY_ACTION: &str = "shell.intervalsCoachKeyAction";

// ── Sciotte code-step strings the app dialog shares ──
//
// The web and mobile sign-in dialog reads this same key, so a code the
// provider refused is worded once on every surface.

/// Key: "That code wasn't accepted. Check it and enter it again." No format placeholders.
pub const KEY_SCIOTTE_CODE_REJECTED: &str = "shell.sciotteCodeRejected";

// ── The hosted import page (carnet#1221) ──
//
// The page the connect picker opens to import the athlete's own history
// shares its wording with the web and mobile import screens: every
// placeholder-free `providers.import.*` key it shows is read here, and the
// lines that carry a count are `hosted.import.*`, with positional slots.

/// Key: "Import your own data" No format placeholders.
pub const KEY_IMPORT_LINK: &str = "providers.import.link";
/// Key: "Bring in history you've exported from another service." No format placeholders.
pub const KEY_IMPORT_LINK_HINT: &str = "providers.import.linkHint";
/// Key: "Import your own data" No format placeholders.
pub const KEY_IMPORT_TITLE: &str = "providers.import.title";
/// Key: "Importing your own data isn't available here yet." No format placeholders.
pub const KEY_IMPORT_UNAVAILABLE: &str = "providers.import.unavailable";
/// Key: "Waiting for Strava's email · Paste the link" No format placeholders.
pub const KEY_IMPORT_STATE_AWAITING_LINK: &str = "providers.import.state.awaitingLink";
/// Key: "Starting your import" No format placeholders.
pub const KEY_IMPORT_STATE_QUEUED: &str = "providers.import.state.queued";
/// Key: "Downloading your export" No format placeholders.
pub const KEY_IMPORT_STATE_DOWNLOADING: &str = "providers.import.state.downloading";
/// Key: "History you import from your own Strava export is yours and stays, even if you disconnect Strava." No format placeholders.
pub const KEY_IMPORT_STRAVA_INTRO: &str = "providers.import.strava.intro";
/// Key: "Step 1 of 3 · Request your Strava export" No format placeholders.
pub const KEY_IMPORT_STRAVA_STEP1_TITLE: &str = "providers.import.strava.step1Title";
/// Key: "On Strava, open Settings, then My Account, … and choose Request Your Archive." No format placeholders.
pub const KEY_IMPORT_STRAVA_STEP1_BODY: &str = "providers.import.strava.step1Body";
/// Key: "Open Strava's export page" No format placeholders.
pub const KEY_IMPORT_STRAVA_STEP1_OPEN: &str = "providers.import.strava.step1Open";
/// Key: "I've requested it" No format placeholders.
pub const KEY_IMPORT_STRAVA_STEP1_REQUESTED: &str = "providers.import.strava.step1Requested";
/// Key: "Step 2 of 3 · Paste the link from Strava's email" No format placeholders.
pub const KEY_IMPORT_STRAVA_STEP2_TITLE: &str = "providers.import.strava.step2Title";
/// Key: "Link from Strava's email" No format placeholders.
pub const KEY_IMPORT_STRAVA_LINK_LABEL: &str = "providers.import.strava.linkLabel";
/// Key: "https://…" No format placeholders.
pub const KEY_IMPORT_STRAVA_LINK_PLACEHOLDER: &str = "providers.import.strava.linkPlaceholder";
/// Key: "Start import" No format placeholders.
pub const KEY_IMPORT_STRAVA_START: &str = "providers.import.strava.start";
/// Key: "Starting…" No format placeholders.
pub const KEY_IMPORT_STRAVA_STARTING: &str = "providers.import.strava.starting";
/// Key: "Step 3 of 3 · Importing" No format placeholders.
pub const KEY_IMPORT_STRAVA_STEP3_TITLE: &str = "providers.import.strava.step3Title";
/// Key: "Getting your export from Strava…" No format placeholders.
pub const KEY_IMPORT_STRAVA_PROGRESS_STARTING: &str = "providers.import.strava.progressStarting";
/// Key: "You can close this window: the import keeps going." No format placeholders.
pub const KEY_IMPORT_STRAVA_LEAVE_HINT: &str = "providers.import.strava.leaveHint";
/// Key: "Cancel import" No format placeholders.
pub const KEY_IMPORT_STRAVA_CANCEL: &str = "providers.import.strava.cancel";
/// Key: "Import a newer export" No format placeholders.
pub const KEY_IMPORT_STRAVA_IMPORT_NEWER: &str = "providers.import.strava.importNewer";
/// Key: "An import from Strava is already running." No format placeholders.
pub const KEY_IMPORT_STRAVA_IN_PROGRESS: &str = "providers.import.strava.inProgress";
/// Key: "Something went wrong. Try again in a moment." No format placeholders.
pub const KEY_IMPORT_STRAVA_GENERIC_ERROR: &str = "providers.import.strava.genericError";
/// Key: "Paste the link again to resume." No format placeholders.
pub const KEY_IMPORT_STRAVA_RETRY: &str = "providers.import.strava.retry";
/// Key: "Open Dravr to see which activities were not imported, and why." No format placeholders.
pub const KEY_HOSTED_IMPORT_DETAILS_IN_APP: &str = "hosted.import.detailsInApp";
/// Key: "Activities imported: `{0}`"
pub const KEY_HOSTED_IMPORT_DONE_TITLE: &str = "hosted.import.doneTitle";
/// Key: "Not imported: `{0}`"
pub const KEY_HOSTED_IMPORT_NOT_IMPORTED: &str = "hosted.import.notImported";
/// Key: "`{0}` of `{1}` activities read"
pub const KEY_HOSTED_IMPORT_PROGRESS: &str = "hosted.import.progress";
/// Key: "Import stopped · `{0}` imported"
pub const KEY_HOSTED_IMPORT_STOPPED: &str = "hosted.import.stopped";

/// Key: the refusal of a link, `link_invalid`. No format placeholders.
pub const KEY_IMPORT_STRAVA_REFUSAL_LINK_INVALID: &str =
    "providers.import.strava.refusal.link_invalid";
/// Key: the refusal of a link, `link_host_not_allowed`. No format placeholders.
pub const KEY_IMPORT_STRAVA_REFUSAL_LINK_HOST_NOT_ALLOWED: &str =
    "providers.import.strava.refusal.link_host_not_allowed";
/// Key: the refusal of a link, `link_expired`. No format placeholders.
pub const KEY_IMPORT_STRAVA_REFUSAL_LINK_EXPIRED: &str =
    "providers.import.strava.refusal.link_expired";
/// Key: the refusal of a link, `link_unreachable`. No format placeholders.
pub const KEY_IMPORT_STRAVA_REFUSAL_LINK_UNREACHABLE: &str =
    "providers.import.strava.refusal.link_unreachable";
/// Key: the failure of an import, `download_failed`. No format placeholders.
pub const KEY_IMPORT_STRAVA_FAILURE_DOWNLOAD_FAILED: &str =
    "providers.import.strava.failure.download_failed";
/// Key: the failure of an import, `link_expired`. No format placeholders.
pub const KEY_IMPORT_STRAVA_FAILURE_LINK_EXPIRED: &str =
    "providers.import.strava.failure.link_expired";
/// Key: the failure of an import, `archive_too_large`. No format placeholders.
pub const KEY_IMPORT_STRAVA_FAILURE_ARCHIVE_TOO_LARGE: &str =
    "providers.import.strava.failure.archive_too_large";
/// Key: the failure of an import, `archive_unreadable`. No format placeholders.
pub const KEY_IMPORT_STRAVA_FAILURE_ARCHIVE_UNREADABLE: &str =
    "providers.import.strava.failure.archive_unreadable";
/// Key: the failure of an import, `archive_limit`. No format placeholders.
pub const KEY_IMPORT_STRAVA_FAILURE_ARCHIVE_LIMIT: &str =
    "providers.import.strava.failure.archive_limit";
/// Key: the failure of an import, `missing_activities_csv`. No format placeholders.
pub const KEY_IMPORT_STRAVA_FAILURE_MISSING_ACTIVITIES_CSV: &str =
    "providers.import.strava.failure.missing_activities_csv";
/// Key: the failure of an import, `unknown_locale`. No format placeholders.
pub const KEY_IMPORT_STRAVA_FAILURE_UNKNOWN_LOCALE: &str =
    "providers.import.strava.failure.unknown_locale";
/// Key: the failure of an import, `spool_missing`. No format placeholders.
pub const KEY_IMPORT_STRAVA_FAILURE_SPOOL_MISSING: &str =
    "providers.import.strava.failure.spool_missing";
/// Key: the failure of an import, `not_an_export`. No format placeholders.
pub const KEY_IMPORT_STRAVA_FAILURE_NOT_AN_EXPORT: &str =
    "providers.import.strava.failure.not_an_export";

/// The copy of each link refusal, by the code the import routes answer with.
pub const IMPORT_STRAVA_REFUSALS: [(&str, &str); 4] = [
    ("link_invalid", KEY_IMPORT_STRAVA_REFUSAL_LINK_INVALID),
    (
        "link_host_not_allowed",
        KEY_IMPORT_STRAVA_REFUSAL_LINK_HOST_NOT_ALLOWED,
    ),
    ("link_expired", KEY_IMPORT_STRAVA_REFUSAL_LINK_EXPIRED),
    (
        "link_unreachable",
        KEY_IMPORT_STRAVA_REFUSAL_LINK_UNREACHABLE,
    ),
];

/// The copy of each import failure, by its stable failure code.
pub const IMPORT_STRAVA_FAILURES: [(&str, &str); 9] = [
    ("download_failed", KEY_IMPORT_STRAVA_FAILURE_DOWNLOAD_FAILED),
    ("link_expired", KEY_IMPORT_STRAVA_FAILURE_LINK_EXPIRED),
    (
        "archive_too_large",
        KEY_IMPORT_STRAVA_FAILURE_ARCHIVE_TOO_LARGE,
    ),
    (
        "archive_unreadable",
        KEY_IMPORT_STRAVA_FAILURE_ARCHIVE_UNREADABLE,
    ),
    ("archive_limit", KEY_IMPORT_STRAVA_FAILURE_ARCHIVE_LIMIT),
    (
        "missing_activities_csv",
        KEY_IMPORT_STRAVA_FAILURE_MISSING_ACTIVITIES_CSV,
    ),
    ("unknown_locale", KEY_IMPORT_STRAVA_FAILURE_UNKNOWN_LOCALE),
    ("spool_missing", KEY_IMPORT_STRAVA_FAILURE_SPOOL_MISSING),
    ("not_an_export", KEY_IMPORT_STRAVA_FAILURE_NOT_AN_EXPORT),
];

/// Every slot-less key above: the set a hosted template may name as
/// `{{t:<key>}}` and a renderer substitutes.
pub const TEMPLATE_KEYS: [&str; 94] = [
    KEY_HOSTED_COMMON_SUBTITLE,
    KEY_HOSTED_COMMON_YOUR_CHAT_APP,
    KEY_HOSTED_COMMON_BACK,
    KEY_HOSTED_COMMON_EMAIL_LABEL,
    KEY_HOSTED_COMMON_USERNAME_LABEL,
    KEY_HOSTED_COMMON_PASSWORD_LABEL,
    KEY_HOSTED_COMMON_LOG_IN,
    KEY_HOSTED_COMMON_LOGGING_IN_ARIA,
    KEY_HOSTED_COMMON_MAY_TAKE_TIME,
    KEY_HOSTED_COMMON_LOADING,
    KEY_HOSTED_COMMON_VERIFYING_CODE,
    KEY_HOSTED_COMMON_TWO_STEP_TITLE,
    KEY_HOSTED_COMMON_WAITING_APPROVAL_ARIA,
    KEY_HOSTED_COMMON_CHECK_PHONE,
    KEY_HOSTED_COMMON_TAP_YES,
    KEY_HOSTED_COMMON_TAP_NUMBER,
    KEY_HOSTED_COMMON_CHECK_NOTIFICATION,
    KEY_HOSTED_COMMON_WAITING_ARIA,
    KEY_HOSTED_COMMON_OTP_TITLE,
    KEY_HOSTED_COMMON_VERIFY,
    KEY_HOSTED_COMMON_CONNECTED,
    KEY_HOSTED_COMMON_LOGIN_FAILED,
    KEY_HOSTED_COMMON_SIGN_IN_EXPIRED,
    KEY_HOSTED_COMMON_TRY_AGAIN,
    KEY_HOSTED_PICKER_PAGE_TITLE,
    KEY_HOSTED_PICKER_HEADING,
    KEY_HOSTED_PICKER_SECURITY_NOTE,
    KEY_HOSTED_PICKER_CONTINUE,
    KEY_HOSTED_PICKER_ERROR_TITLE,
    KEY_HOSTED_PICKER_BACK_TO_PROVIDERS,
    KEY_HOSTED_PICKER_TAG_CONNECTED,
    KEY_HOSTED_PICKER_TAG_AUTHORIZE,
    KEY_HOSTED_PICKER_TAG_API_KEY,
    KEY_HOSTED_PICKER_TAG_USERNAME_PASSWORD,
    KEY_HOSTED_PICKER_TAG_EMAIL_PASSWORD,
    KEY_HOSTED_PICKER_STRAVA_FALLBACK,
    KEY_HOSTED_PICKER_SIGN_IN_INCOMPLETE,
    KEY_HOSTED_LOGIN_PAGE_TITLE,
    KEY_HOSTED_INTERVALS_PAGE_TITLE,
    KEY_HOSTED_INTERVALS_STORAGE_NOTE,
    KEY_HOSTED_INTERVALS_ATHLETE_ID_REQUIRED,
    KEY_HOSTED_INTERVALS_API_KEY_REQUIRED,
    KEY_HOSTED_INTERVALS_REJECTED,
    KEY_HOSTED_INTERVALS_UNAVAILABLE,
    KEY_HOSTED_INTERVALS_SAVE_FAILED,
    KEY_HOSTED_INTERVALS_TOMBSTONED,
    KEY_HOSTED_INTERVALS_AUTHORIZE_NOTE,
    KEY_HOSTED_INTERVALS_COACH_NOTE,
    KEY_HOSTED_SUCCESS_PAGE_TITLE,
    KEY_HOSTED_SUCCESS_HEADING_GENERIC,
    KEY_HOSTED_SUCCESS_DATA_AVAILABLE_GENERIC,
    KEY_HOSTED_SUCCESS_AUTO_CLOSE,
    KEY_HOSTED_ERROR_PAGE_TITLE,
    KEY_HOSTED_ERROR_HEADING,
    KEY_HOSTED_ERROR_MISSING_TOKEN,
    KEY_HOSTED_ERROR_INVALID_LINK,
    KEY_HOSTED_ERROR_MALFORMED_LINK,
    KEY_HOSTED_ERROR_LINK_ALREADY_OPENED,
    KEY_HOSTED_ERROR_LOAD_FAILED,
    KEY_HOSTED_ERROR_NOTICE_REQUIRED,
    KEY_HOSTED_ERROR_OAUTH_START_FAILED,
    KEY_HOSTED_ERROR_GENERIC,
    KEY_INTERVALS_ATHLETE_ID,
    KEY_INTERVALS_API_KEY_LABEL,
    KEY_INTERVALS_CONNECT_ACTION,
    KEY_INTERVALS_CREDENTIALS_HELP,
    KEY_INTERVALS_AUTHORIZE_ACTION,
    KEY_INTERVALS_COACH_KEY_ACTION,
    KEY_IMPORT_LINK,
    KEY_IMPORT_LINK_HINT,
    KEY_IMPORT_TITLE,
    KEY_IMPORT_UNAVAILABLE,
    KEY_IMPORT_STATE_AWAITING_LINK,
    KEY_IMPORT_STATE_QUEUED,
    KEY_IMPORT_STATE_DOWNLOADING,
    KEY_IMPORT_STRAVA_INTRO,
    KEY_IMPORT_STRAVA_STEP1_TITLE,
    KEY_IMPORT_STRAVA_STEP1_BODY,
    KEY_IMPORT_STRAVA_STEP1_OPEN,
    KEY_IMPORT_STRAVA_STEP1_REQUESTED,
    KEY_IMPORT_STRAVA_STEP2_TITLE,
    KEY_IMPORT_STRAVA_LINK_LABEL,
    KEY_IMPORT_STRAVA_LINK_PLACEHOLDER,
    KEY_IMPORT_STRAVA_START,
    KEY_IMPORT_STRAVA_STARTING,
    KEY_IMPORT_STRAVA_STEP3_TITLE,
    KEY_IMPORT_STRAVA_PROGRESS_STARTING,
    KEY_IMPORT_STRAVA_LEAVE_HINT,
    KEY_IMPORT_STRAVA_CANCEL,
    KEY_IMPORT_STRAVA_IMPORT_NEWER,
    KEY_IMPORT_STRAVA_IN_PROGRESS,
    KEY_IMPORT_STRAVA_GENERIC_ERROR,
    KEY_IMPORT_STRAVA_RETRY,
    KEY_HOSTED_IMPORT_DETAILS_IN_APP,
];
