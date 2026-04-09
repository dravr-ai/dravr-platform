// ABOUTME: HTML email templates for transactional emails
// ABOUTME: Generates branded HTML for password reset codes, channel linking, and notifications
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Password reset code TTL displayed to the user
const RESET_CODE_EXPIRY_MINUTES: u32 = 15;

/// Channel linking OTP code TTL displayed to the user
const CHANNEL_LINKING_CODE_EXPIRY_MINUTES: u32 = 10;

/// Generate the HTML body for a password reset code email
///
/// The code is displayed prominently so the user can easily read and enter it
/// on the reset form. Designed to work well on mobile email clients.
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

/// Generate the HTML body for a channel linking verification code email
///
/// Displays a 6-digit code for the user to type back in their messaging app
/// to complete account linking. Follows the same branded layout as password reset.
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
