// ABOUTME: HTML email templates for transactional emails
// ABOUTME: One branded document layout; each template supplies only its escaped card body
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Transactional email bodies.
//!
//! Every email shares one document: the head, the Dravr wordmark, the card the
//! message sits in and the footer. [`layout`] owns that chrome and escapes the
//! title and heading it is given; a template builds only the card's body,
//! escaping whatever it interpolates into it.

use html_escape::{encode_double_quoted_attribute, encode_text};

/// Password reset code TTL displayed to the user
const RESET_CODE_EXPIRY_MINUTES: u32 = 15;

/// Channel linking OTP code TTL displayed to the user
const CHANNEL_LINKING_CODE_EXPIRY_MINUTES: u32 = 10;

/// The branded document every email is: `title` names the message in the
/// client's tab and preview, `heading` tops the card, and `body_html` fills it.
///
/// `title` and `heading` are escaped here; `body_html` is markup the caller
/// built and escaped.
fn layout(title: &str, heading: &str, body_html: &str) -> String {
    let title = encode_text(title);
    let heading = encode_text(heading);
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>{title}</title>
</head>
<body style="margin:0;padding:0;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,sans-serif;background-color:#0a0a0f;color:#e5e7eb;">
  <table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="max-width:480px;margin:0 auto;padding:40px 20px;">
    <tr>
      <td style="text-align:center;padding-bottom:32px;">
        <h1 style="margin:0;font-size:24px;font-weight:700;color:#ffffff;">Dravr</h1>
      </td>
    </tr>
    <tr>
      <td style="background:linear-gradient(135deg,rgba(139,92,246,0.1),rgba(59,130,246,0.1));border:1px solid rgba(255,255,255,0.1);border-radius:16px;padding:32px;">
        <h2 style="margin:0 0 16px;font-size:20px;font-weight:600;color:#ffffff;">{heading}</h2>
        {body_html}
      </td>
    </tr>
    <tr>
      <td style="text-align:center;padding-top:24px;">
        <p style="margin:0;font-size:12px;color:#4b5563;">&copy; Dravr</p>
      </td>
    </tr>
  </table>
</body>
</html>"#
    )
}

/// The branded call-to-action button linking to `url`, labelled `label`.
///
/// `url` is attribute-escaped here; `label` is a fixed string of the caller's.
fn cta_button(url: &str, label: &str) -> String {
    let url_attr = encode_double_quoted_attribute(url);
    format!(
        r#"<a href="{url_attr}" style="display:inline-block;background:linear-gradient(135deg,#8b5cf6,#3b82f6);color:#ffffff;text-decoration:none;font-weight:600;font-size:15px;padding:12px 28px;border-radius:10px;">{label}</a>"#
    )
}

/// The button to `url` centred on its own row above the card's closing lines.
fn cta_block(url: &str, label: &str) -> String {
    format!(
        r#"<div style="text-align:center;margin:0 0 24px;">
          {}
        </div>"#,
        cta_button(url, label)
    )
}

/// Generate the HTML body for a password reset code email
///
/// The code is displayed prominently so the user can easily copy it into the reset
/// form. The code is a high-entropy alphanumeric token, so it is rendered to wrap
/// cleanly on narrow mobile email clients rather than as spaced single digits.
#[must_use]
pub fn password_reset_code_html(code: &str) -> String {
    let code = encode_text(code);
    layout(
        "Password Reset Code",
        "Reset your password",
        &format!(
            r#"<p style="margin:0 0 24px;font-size:15px;line-height:1.5;color:#9ca3af;">
          Enter this code to reset your password. It expires in {RESET_CODE_EXPIRY_MINUTES} minutes.
        </p>
        <div style="background:rgba(255,255,255,0.05);border:1px solid rgba(255,255,255,0.15);border-radius:12px;padding:20px;text-align:center;margin-bottom:24px;">
          <span style="font-size:18px;font-weight:700;letter-spacing:1px;color:#ffffff;font-family:'Courier New',monospace;word-break:break-all;">{code}</span>
        </div>
        <p style="margin:0;font-size:13px;line-height:1.5;color:#6b7280;">
          If you did not request a password reset, you can safely ignore this email.
          Your account remains secure.
        </p>"#
        ),
    )
}

/// Greeting line for account lifecycle emails
///
/// Returns "Hi {name}," when a display name is available, otherwise "Hi there,".
/// User-supplied names are HTML-escaped to prevent injection in the rendered email.
fn greeting(display_name: Option<&str>) -> String {
    display_name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map_or_else(
            || "Hi there,".to_owned(),
            |name| format!("Hi {},", encode_text(name)),
        )
}

/// Generate the HTML body for a "registration received, pending approval" email
///
/// Sent immediately after a user self-registers when their account lands in
/// Pending status. Informs them that an administrator will review the request
/// and that a follow-up email will arrive once the account is activated.
#[must_use]
pub fn registration_pending_html(display_name: Option<&str>) -> String {
    let greeting = greeting(display_name);
    layout(
        "Welcome to Dravr",
        "Thanks for signing up",
        &format!(
            r#"<p style="margin:0 0 16px;font-size:15px;line-height:1.5;color:#e5e7eb;">{greeting}</p>
        <p style="margin:0 0 16px;font-size:15px;line-height:1.5;color:#9ca3af;">
          We've received your Dravr registration. Your account is currently
          <strong style="color:#ffffff;">pending administrator review</strong>.
        </p>
        <p style="margin:0 0 24px;font-size:15px;line-height:1.5;color:#9ca3af;">
          You'll receive another email as soon as your account is approved and
          ready to sign in. No action is needed from you in the meantime.
        </p>
        <p style="margin:0;font-size:13px;line-height:1.5;color:#6b7280;">
          If you did not sign up for Dravr, you can safely ignore this email.
        </p>"#
        ),
    )
}

/// Generate the HTML body for a "your account has been approved" email
///
/// Sent after an administrator approves a pending registration (or after
/// auto-approval during registration). Includes a sign-in call-to-action when
/// a frontend URL is configured, and a plain notice otherwise.
#[must_use]
pub fn registration_approved_html(display_name: Option<&str>, sign_in_url: Option<&str>) -> String {
    let greeting = greeting(display_name);
    let cta = sign_in_url.map_or_else(String::new, |url| cta_block(url, "Sign in to Dravr"));
    layout(
        "Your Dravr account is approved",
        "You're in",
        &format!(
            r#"<p style="margin:0 0 16px;font-size:15px;line-height:1.5;color:#e5e7eb;">{greeting}</p>
        <p style="margin:0 0 24px;font-size:15px;line-height:1.5;color:#9ca3af;">
          Your Dravr account has been approved and is now active. You can sign
          in and start connecting your fitness data sources.
        </p>
        {cta}
        <p style="margin:0;font-size:13px;line-height:1.5;color:#6b7280;">
          Welcome aboard.
        </p>"#
        ),
    )
}

/// Generate the HTML body for an invitation message.
///
/// Sent when an operator pre-approves an address and asks for the invitee to be
/// told. The link is single-use and opens the app's accept page, where the
/// invitee chooses their own password and the account is created active, so no
/// password crosses this channel. `ttl_days` is stated in the copy so the
/// expiry is known before the link stops working.
#[must_use]
pub fn invitation_html(accept_url: &str, ttl_days: i64) -> String {
    let cta = cta_block(accept_url, "Create your account");
    layout(
        "You're invited to Dravr",
        "You're invited",
        &format!(
            r#"<p style="margin:0 0 24px;font-size:15px;line-height:1.5;color:#9ca3af;">
          You've been invited to Dravr. Choose your password with the link below
          and your account is ready &mdash; there's no approval queue to wait
          through.
        </p>
        {cta}
        <p style="margin:0 0 12px;font-size:13px;line-height:1.5;color:#6b7280;">
          This link works once and expires in {ttl_days} days. If it has
          expired, ask whoever invited you to send a new one.
        </p>
        <p style="margin:0;font-size:13px;line-height:1.5;color:#6b7280;">
          If you weren't expecting this, you can ignore it &mdash; nothing was
          created in your name.
        </p>"#
        ),
    )
}

/// Generate the HTML body for an email-address verification message.
///
/// Sent immediately after registration. The link carries a single-use
/// `<selector>.<verifier>` token; the copy states the expiry because a link that
/// silently stops working is the most common way this flow strands someone.
#[must_use]
pub fn email_verification_html(
    display_name: Option<&str>,
    verify_url: &str,
    ttl_minutes: i64,
) -> String {
    let greeting = greeting(display_name);
    let cta = cta_block(verify_url, "Confirm my email");
    layout(
        "Confirm your email",
        "Confirm your email",
        &format!(
            r#"<p style="margin:0 0 16px;font-size:15px;line-height:1.5;color:#e5e7eb;">{greeting}</p>
        <p style="margin:0 0 24px;font-size:15px;line-height:1.5;color:#9ca3af;">
          Confirm this address to finish setting up your Dravr account.
        </p>
        {cta}
        <p style="margin:0 0 16px;font-size:13px;line-height:1.5;color:#6b7280;">
          This link works once and expires in {ttl_minutes} minutes. If it has
          expired, sign in and we'll send a fresh one.
        </p>
        <p style="margin:0;font-size:13px;line-height:1.5;color:#6b7280;">
          If you didn't create a Dravr account, you can ignore this email.
        </p>"#
        ),
    )
}

/// Generate the HTML body for the dravr.ai docs sign-in link email
///
/// `sign_in_url` carries a single-use `<selector>.<verifier>` token and lands on
/// the website's callback; `ttl_minutes` is stated in the copy so the expiry is
/// read rather than discovered.
#[must_use]
pub fn website_sign_in_html(sign_in_url: &str, ttl_minutes: i64) -> String {
    let cta = cta_block(sign_in_url, "Sign in to the docs");
    layout(
        "Sign in to the Dravr docs",
        "Sign in to the Dravr docs",
        &format!(
            r#"<p style="margin:0 0 24px;font-size:15px;line-height:1.5;color:#9ca3af;">
          Use this link to open the full Dravr documentation with your account.
        </p>
        {cta}
        <p style="margin:0 0 16px;font-size:13px;line-height:1.5;color:#6b7280;">
          This link works once and expires in {ttl_minutes} minutes. If it has
          expired, ask for a new one from any guide.
        </p>
        <p style="margin:0;font-size:13px;line-height:1.5;color:#6b7280;">
          If you didn't ask to sign in, you can ignore this email.
        </p>"#
        ),
    )
}

/// Generate the HTML body for a channel linking verification code email
///
/// Displays a 6-digit code for the user to type back in their messaging app
/// to complete account linking. Follows the same branded layout as password reset.
#[must_use]
pub fn channel_linking_code_html(code: &str, channel_name: &str) -> String {
    let code = encode_text(code);
    let channel_name = encode_text(channel_name);
    layout(
        "Your Dravr verification code",
        "Your verification code",
        &format!(
            r#"<p style="margin:0 0 24px;font-size:15px;line-height:1.5;color:#9ca3af;">
          Enter this code in {channel_name} to link your Dravr account. It expires in {CHANNEL_LINKING_CODE_EXPIRY_MINUTES} minutes.
        </p>
        <div style="background:rgba(255,255,255,0.05);border:1px solid rgba(255,255,255,0.15);border-radius:12px;padding:20px;text-align:center;margin-bottom:24px;">
          <span style="font-size:36px;font-weight:700;letter-spacing:8px;color:#ffffff;font-family:'Courier New',monospace;">{code}</span>
        </div>
        <p style="margin:0;font-size:13px;line-height:1.5;color:#6b7280;">
          If you did not request this code, you can safely ignore this email.
        </p>"#
        ),
    )
}

/// How a coach-access request came to be, which the operator email states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoachAccessOrigin {
    /// The coach asked for it, from the onboarding group step or group info.
    Asked,
    /// Filed for the coach, who asked nothing, because the Intervals.icu key
    /// they linked lists athletes it coaches.
    IntervalsIcuRoster {
        /// How many athletes that key coaches and Intervals.icu lists as active.
        coached_active_athletes: usize,
    },
}

/// Generate the HTML body of the operator email announcing a coach-access
/// request (carnet#738).
///
/// Sent to every super-admin when a request opens: one a coach asked for
/// from the onboarding group step or group info, or one filed for them from
/// the Intervals.icu roster of a key they linked (`origin`), which is never
/// worded as their asking. The request grants nothing (ADR-018); the email
/// says whose it is, how it came, for which group, and links to the console
/// queue where the operator grants or declines it. Requester-supplied text
/// (the name, the group name) is escaped: it reaches an operator's inbox.
#[must_use]
pub fn coach_access_requested_html(
    requester_email: &str,
    requester_name: Option<&str>,
    group_name: Option<&str>,
    review_url: Option<&str>,
    origin: CoachAccessOrigin,
) -> String {
    let who = requester_name.map_or_else(
        || encode_text(requester_email).into_owned(),
        |name| format!("{} ({})", encode_text(name), encode_text(requester_email)),
    );
    let how = match origin {
        CoachAccessOrigin::Asked => format!("{who} asked for coach access."),
        CoachAccessOrigin::IntervalsIcuRoster {
            coached_active_athletes,
        } => {
            let athletes = if coached_active_athletes == 1 {
                "athlete"
            } else {
                "athletes"
            };
            format!(
                "{who}&rsquo;s Intervals.icu account coaches {coached_active_athletes} active \
                 {athletes}; a coach-access request was filed for them."
            )
        }
    };
    let no_group = match origin {
        CoachAccessOrigin::Asked => "They asked without naming a group.",
        CoachAccessOrigin::IntervalsIcuRoster { .. } => {
            "It names no group yet: the group they make to coach is named on it, so a grant \
             attaches them to it."
        }
    };
    let group = group_name.map_or_else(
        || no_group.to_owned(),
        |name| {
            format!(
                "A grant also makes them the coach of their group <strong>{}</strong>.",
                encode_text(name)
            )
        },
    );
    let action = review_url.map_or_else(
        || "Review it in the admin console, under Users &rarr; Coach access.".to_owned(),
        |url| cta_button(url, "Review the request"),
    );
    layout(
        "Coach access requested",
        "Coach access requested",
        &format!(
            r#"<p style="margin:0 0 16px;font-size:15px;line-height:1.5;color:#9ca3af;">
          {how} Nothing is granted until a super-admin decides.
        </p>
        <p style="margin:0 0 24px;font-size:15px;line-height:1.5;color:#9ca3af;">{group}</p>
        <div style="text-align:center;margin:0;">{action}</div>"#
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A value an athlete or requester could type that must never render as
    /// markup.
    const HOSTILE: &str = "<script>x</script>";

    /// One rendered template with what the layout and its body must show.
    struct Rendered {
        html: String,
        title: &'static str,
        heading: &'static str,
        body: &'static str,
    }

    /// Every template, each handed [`HOSTILE`] wherever it takes free text.
    fn every_template() -> Vec<Rendered> {
        let rendered = |html, title, heading, body| Rendered {
            html,
            title,
            heading,
            body,
        };
        vec![
            rendered(
                password_reset_code_html("ABC123"),
                "Password Reset Code",
                "Reset your password",
                "It expires in 15 minutes.",
            ),
            rendered(
                registration_pending_html(Some(HOSTILE)),
                "Welcome to Dravr",
                "Thanks for signing up",
                "pending administrator review",
            ),
            rendered(
                registration_approved_html(Some(HOSTILE), Some("https://app.example.test/login")),
                "Your Dravr account is approved",
                "You're in",
                r#"href="https://app.example.test/login""#,
            ),
            rendered(
                invitation_html("https://app.example.test/accept", 7),
                "You're invited to Dravr",
                "You're invited",
                "expires in 7 days",
            ),
            rendered(
                email_verification_html(Some(HOSTILE), "https://app.example.test/verify", 30),
                "Confirm your email",
                "Confirm your email",
                "expires in 30 minutes",
            ),
            rendered(
                website_sign_in_html("https://dravr.ai/docs/auth/callback", 15),
                "Sign in to the Dravr docs",
                "Sign in to the Dravr docs",
                "expires in 15 minutes",
            ),
            rendered(
                channel_linking_code_html("123456", HOSTILE),
                "Your Dravr verification code",
                "Your verification code",
                "It expires in 10 minutes.",
            ),
            rendered(
                coach_access_requested_html(
                    "coach@example.com",
                    Some(HOSTILE),
                    Some(HOSTILE),
                    None,
                ),
                "Coach access requested",
                "Coach access requested",
                "asked for coach access",
            ),
        ]
    }

    #[test]
    fn every_template_is_one_document_with_its_title_heading_and_body() {
        for t in every_template() {
            let html = &t.html;
            assert_eq!(
                html.matches("<!DOCTYPE html>").count(),
                1,
                "one document per email: {html}"
            );
            assert!(
                html.contains(&format!("<title>{}</title>", t.title)),
                "title {:?}: {html}",
                t.title
            );
            assert!(
                html.contains(&format!("color:#ffffff;\">{}</h2>", t.heading)),
                "heading {:?}: {html}",
                t.heading
            );
            assert!(html.contains(t.body), "body line {:?}: {html}", t.body);
            assert!(html.contains("&copy; Dravr"), "the footer: {html}");
        }
    }

    #[test]
    fn no_template_renders_a_script_it_was_handed() {
        for t in every_template() {
            assert!(!t.html.contains("<script>"), "{}: {}", t.title, t.html);
        }
        let html = registration_pending_html(Some(HOSTILE));
        assert!(html.contains("Hi &lt;script&gt;x&lt;/script&gt;,"));
        let html = channel_linking_code_html("123456", HOSTILE);
        assert!(html.contains("Enter this code in &lt;script&gt;x&lt;/script&gt; to link"));
    }

    #[test]
    fn the_layout_escapes_its_title_and_heading() {
        let html = layout("A <b> & C", "<i>Hi</i>", "<p>body</p>");
        assert!(html.contains("<title>A &lt;b&gt; &amp; C</title>"));
        assert!(html.contains("&lt;i&gt;Hi&lt;/i&gt;</h2>"));
        assert!(html.contains("<p>body</p>"));
    }

    #[test]
    fn the_coach_access_email_escapes_what_the_requester_typed() {
        let html = coach_access_requested_html(
            "coach@example.com",
            Some(HOSTILE),
            Some("A & B"),
            Some("https://app.dravr.ai/admin?tab=coach-access"),
            CoachAccessOrigin::Asked,
        );
        assert!(html.contains("&lt;script&gt;x&lt;/script&gt; (coach@example.com)"));
        assert!(html.contains("<strong>A &amp; B</strong>"));
        assert!(html.contains("href=\"https://app.dravr.ai/admin?tab=coach-access\""));
        assert!(!html.contains("<script>"));
    }

    #[test]
    fn the_coach_access_email_reads_without_a_name_group_or_link() {
        let html = coach_access_requested_html(
            "coach@example.com",
            None,
            None,
            None,
            CoachAccessOrigin::Asked,
        );
        assert!(html.contains("coach@example.com asked for coach access"));
        assert!(html.contains("without naming a group"));
        assert!(html.contains("Users &rarr; Coach access"));
    }

    #[test]
    fn a_request_filed_from_a_roster_says_it_was_filed_never_asked() {
        let html = coach_access_requested_html(
            "coach@example.com",
            Some("Casey"),
            None,
            None,
            CoachAccessOrigin::IntervalsIcuRoster {
                coached_active_athletes: 3,
            },
        );
        assert!(html.contains(
            "Casey (coach@example.com)&rsquo;s Intervals.icu account coaches 3 active athletes; \
             a coach-access request was filed for them."
        ));
        assert!(html.contains("the group they make to coach is named on it"));
        assert!(!html.contains("asked"), "{html}");

        let one = coach_access_requested_html(
            "coach@example.com",
            None,
            None,
            None,
            CoachAccessOrigin::IntervalsIcuRoster {
                coached_active_athletes: 1,
            },
        );
        assert!(one.contains("coaches 1 active athlete;"));
    }
}
