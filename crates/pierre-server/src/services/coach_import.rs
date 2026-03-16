// ABOUTME: Business logic for coach markdown import operations
// ABOUTME: URL fetching with security validation, import warnings, and definition-to-request conversion
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::net::IpAddr;
use std::time::Duration;

use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::coaches::CreateCoachRequest;
use url::Url;

use crate::coaches::CoachDefinition;

/// Maximum body size for fetched markdown content (1 MB)
const MAX_BODY_SIZE_BYTES: usize = 1_048_576;

/// Timeout for HTTP requests when fetching markdown from URLs
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// User-Agent header sent when fetching coach definitions from URLs
const USER_AGENT: &str = "Pierre/1.0 CoachImport";

/// Token count threshold that triggers a size warning during import
const TOKEN_COUNT_WARNING_THRESHOLD: u32 = 10_000;

/// Fetch markdown content from a URL with security checks.
///
/// Validates the URL scheme (HTTPS only) and rejects private/loopback
/// IP addresses to prevent SSRF. Enforces a 10-second timeout and 1 MB
/// maximum body size.
///
/// # Errors
///
/// Returns error if URL is invalid, blocked by security policy,
/// or the HTTP request fails.
pub async fn fetch_markdown_from_url(url: &str) -> AppResult<String> {
    validate_url_security(url)?;

    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| AppError::internal(format!("Failed to create HTTP client: {e}")))?;

    let response = client.get(url).send().await.map_err(|e| {
        AppError::new(
            ErrorCode::ExternalServiceError,
            format!("Failed to fetch URL: {e}"),
        )
    })?;

    let status = response.status();
    if !status.is_success() {
        return Err(AppError::new(
            ErrorCode::ExternalServiceError,
            format!("URL returned HTTP {status}"),
        ));
    }

    // Check Content-Length header before downloading body
    if let Some(content_length) = response.content_length() {
        if content_length > MAX_BODY_SIZE_BYTES as u64 {
            return Err(AppError::new(
                ErrorCode::ValueOutOfRange,
                format!(
                    "Response body too large: {content_length} bytes (max {MAX_BODY_SIZE_BYTES} bytes)"
                ),
            ));
        }
    }

    let bytes = response.bytes().await.map_err(|e| {
        AppError::new(
            ErrorCode::ExternalServiceError,
            format!("Failed to read response body: {e}"),
        )
    })?;

    if bytes.len() > MAX_BODY_SIZE_BYTES {
        return Err(AppError::new(
            ErrorCode::ValueOutOfRange,
            format!(
                "Response body too large: {} bytes (max {} bytes)",
                bytes.len(),
                MAX_BODY_SIZE_BYTES
            ),
        ));
    }

    String::from_utf8(bytes.to_vec()).map_err(|e| {
        AppError::new(
            ErrorCode::InvalidFormat,
            format!("Response body is not valid UTF-8: {e}"),
        )
    })
}

/// Validate URL security to prevent SSRF attacks.
///
/// Rejects:
/// - Non-HTTPS schemes
/// - Private/loopback IP addresses (127.0.0.0/8, 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, `::1`)
/// - URLs without a host
///
/// # Errors
///
/// Returns `InvalidInput` error if the URL fails security validation.
pub fn validate_url_security(url: &str) -> AppResult<()> {
    let parsed = Url::parse(url)
        .map_err(|e| AppError::new(ErrorCode::InvalidInput, format!("Invalid URL: {e}")))?;

    // Only allow HTTPS
    if parsed.scheme() != "https" {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "Only HTTPS URLs are allowed for coach import",
        ));
    }

    // Must have a host
    let host_str = parsed
        .host_str()
        .ok_or_else(|| AppError::new(ErrorCode::InvalidInput, "URL must have a host"))?;

    // Check for private/loopback IP addresses
    if let Ok(ip) = host_str.parse::<IpAddr>() {
        if is_private_ip(&ip) {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Private and loopback IP addresses are not allowed",
            ));
        }
    }

    // Also reject known loopback hostnames
    let lower_host = host_str.to_lowercase();
    if lower_host == "localhost" || lower_host == "ip6-localhost" || lower_host == "ip6-loopback" {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "Loopback hostnames are not allowed",
        ));
    }

    Ok(())
}

/// Check if an IP address is private or loopback (SSRF protection).
///
/// Blocks: 127.0.0.0/8, 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, `::1`, link-local
fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()          // 127.0.0.0/8
                || v4.is_private()     // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
                || v4.is_link_local()  // 169.254.0.0/16
                || v4.is_unspecified() // 0.0.0.0
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()          // ::1
                || v6.is_unspecified() // ::
        }
    }
}

/// Generate warnings for an imported coach definition.
///
/// Checks for common quality issues that don't prevent import
/// but should be brought to the user's attention:
/// - Missing optional sections (`example_inputs`, `example_outputs`, `success_criteria`)
/// - Excessive token count (> 10,000)
/// - No tags defined
#[must_use]
pub fn generate_import_warnings(definition: &CoachDefinition) -> Vec<String> {
    let mut warnings = Vec::new();

    if definition.sections.example_inputs.is_none() {
        warnings.push("Missing optional section: Example Inputs".to_owned());
    }

    if definition.sections.example_outputs.is_none() {
        warnings.push("Missing optional section: Example Outputs".to_owned());
    }

    if definition.sections.success_criteria.is_none() {
        warnings.push("Missing optional section: Success Criteria".to_owned());
    }

    if definition.token_count > TOKEN_COUNT_WARNING_THRESHOLD {
        warnings.push(format!(
            "High token count: {} (recommended < {})",
            definition.token_count, TOKEN_COUNT_WARNING_THRESHOLD
        ));
    }

    if definition.frontmatter.tags.is_empty() {
        warnings.push("No tags defined — tags improve discoverability".to_owned());
    }

    warnings
}

/// Convert a `CoachDefinition` to a `CreateCoachRequest`, mapping all fields.
///
/// Extracts sample prompts from the `example_inputs` section (lines starting with `-`)
/// and maps all frontmatter and section fields to the request structure.
#[must_use]
pub fn definition_to_create_request(definition: &CoachDefinition) -> CreateCoachRequest {
    let sample_prompts = definition
        .sections
        .example_inputs
        .as_ref()
        .map(|inputs| {
            inputs
                .lines()
                .filter_map(|line| {
                    line.trim()
                        .strip_prefix('-')
                        .map(|s| s.trim().trim_matches('"').to_owned())
                })
                .collect()
        })
        .unwrap_or_default();

    CreateCoachRequest {
        title: definition.frontmatter.title.clone(),
        description: Some(definition.sections.purpose.clone()),
        system_prompt: definition.sections.instructions.clone(),
        category: definition.frontmatter.category,
        tags: definition.frontmatter.tags.clone(),
        sample_prompts,
        startup_query: definition.frontmatter.startup.query.clone(),
        data_requirements: definition.frontmatter.startup.data_requirements.clone(),
        purpose: Some(definition.sections.purpose.clone()),
        when_to_use: definition.sections.when_to_use.clone(),
        instructions: Some(definition.sections.instructions.clone()),
        example_inputs: definition.sections.example_inputs.clone(),
        example_outputs: definition.sections.example_outputs.clone(),
        success_criteria: definition.sections.success_criteria.clone(),
    }
}
