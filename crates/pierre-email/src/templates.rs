// ABOUTME: HTML email templates for transactional emails
// ABOUTME: Generates branded HTML for password reset codes, channel linking, and notifications
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use html_escape::{encode_double_quoted_attribute, encode_text};

/// Password reset code TTL displayed to the user
const RESET_CODE_EXPIRY_MINUTES: u32 = 15;

/// Channel linking OTP code TTL displayed to the user
const CHANNEL_LINKING_CODE_EXPIRY_MINUTES: u32 = 10;

/// Generate the HTML body for a password reset code email
///
/// The code is displayed prominently so the user can easily read and enter it
/// on the reset form. Designed to work well on mobile email clients.
#[must_use]
pub fn password_reset_code_html(code: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Password Reset Code</title>
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
        <h2 style="margin:0 0 16px;font-size:20px;font-weight:600;color:#ffffff;">Reset your password</h2>
        <p style="margin:0 0 24px;font-size:15px;line-height:1.5;color:#9ca3af;">
          Enter this code to reset your password. It expires in {RESET_CODE_EXPIRY_MINUTES} minutes.
        </p>
        <div style="background:rgba(255,255,255,0.05);border:1px solid rgba(255,255,255,0.15);border-radius:12px;padding:20px;text-align:center;margin-bottom:24px;">
          <span style="font-size:36px;font-weight:700;letter-spacing:8px;color:#ffffff;font-family:'Courier New',monospace;">{code}</span>
        </div>
        <p style="margin:0;font-size:13px;line-height:1.5;color:#6b7280;">
          If you did not request a password reset, you can safely ignore this email.
          Your account remains secure.
        </p>
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
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Welcome to Dravr</title>
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
        <h2 style="margin:0 0 16px;font-size:20px;font-weight:600;color:#ffffff;">Thanks for signing up</h2>
        <p style="margin:0 0 16px;font-size:15px;line-height:1.5;color:#e5e7eb;">{greeting}</p>
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
        </p>
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

/// Generate the HTML body for a "your account has been approved" email
///
/// Sent after an administrator approves a pending registration (or after
/// auto-approval during registration). Includes a sign-in call-to-action when
/// a frontend URL is configured; falls back to a plain notice otherwise.
#[must_use]
pub fn registration_approved_html(display_name: Option<&str>, sign_in_url: Option<&str>) -> String {
    let greeting = greeting(display_name);
    let cta_block = sign_in_url.map_or_else(String::new, |url| {
        let url_attr = encode_double_quoted_attribute(url);
        format!(
            r#"<div style="text-align:center;margin:0 0 24px;">
          <a href="{url_attr}" style="display:inline-block;background:linear-gradient(135deg,#8b5cf6,#3b82f6);color:#ffffff;text-decoration:none;font-weight:600;font-size:15px;padding:12px 28px;border-radius:10px;">Sign in to Dravr</a>
        </div>"#
        )
    });
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Your Dravr account is approved</title>
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
        <h2 style="margin:0 0 16px;font-size:20px;font-weight:600;color:#ffffff;">You're in</h2>
        <p style="margin:0 0 16px;font-size:15px;line-height:1.5;color:#e5e7eb;">{greeting}</p>
        <p style="margin:0 0 24px;font-size:15px;line-height:1.5;color:#9ca3af;">
          Your Dravr account has been approved and is now active. You can sign
          in and start connecting your fitness data sources.
        </p>
        {cta_block}
        <p style="margin:0;font-size:13px;line-height:1.5;color:#6b7280;">
          Welcome aboard.
        </p>
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

/// Generate the HTML body for a channel linking verification code email
///
/// Displays a 6-digit code for the user to type back in their messaging app
/// to complete account linking. Follows the same branded layout as password reset.
#[must_use]
pub fn channel_linking_code_html(code: &str, channel_name: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Your Dravr verification code</title>
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
        <h2 style="margin:0 0 16px;font-size:20px;font-weight:600;color:#ffffff;">Your verification code</h2>
        <p style="margin:0 0 24px;font-size:15px;line-height:1.5;color:#9ca3af;">
          Enter this code in {channel_name} to link your Dravr account. It expires in {CHANNEL_LINKING_CODE_EXPIRY_MINUTES} minutes.
        </p>
        <div style="background:rgba(255,255,255,0.05);border:1px solid rgba(255,255,255,0.15);border-radius:12px;padding:20px;text-align:center;margin-bottom:24px;">
          <span style="font-size:36px;font-weight:700;letter-spacing:8px;color:#ffffff;font-family:'Courier New',monospace;">{code}</span>
        </div>
        <p style="margin:0;font-size:13px;line-height:1.5;color:#6b7280;">
          If you did not request this code, you can safely ignore this email.
        </p>
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
