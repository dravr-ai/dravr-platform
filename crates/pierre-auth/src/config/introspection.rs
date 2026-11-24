// ABOUTME: The resource servers allowed to introspect this authorization server's tokens (OAUTH2_INTROSPECTION_CLIENTS)
// ABOUTME: A comma-separated list of confidential client ids, parsed once at startup; empty admits none
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// The client ids `configured` lists, comma-separated: each trimmed, blanks
/// and repeats dropped, in the order given. Unset or empty names none, so no
/// caller may introspect a token.
pub(super) fn resolve_introspection_clients(configured: Option<&str>) -> Vec<String> {
    let mut clients: Vec<String> = Vec::new();
    let named = configured
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty());
    for client in named {
        if !clients.iter().any(|known| known == client) {
            clients.push(client.to_owned());
        }
    }
    clients
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn introspection_clients_are_trimmed_deduplicated_and_kept_in_order() {
        assert_eq!(
            resolve_introspection_clients(None),
            Vec::<String>::new(),
            "bug: an unset variable named an introspection client"
        );
        assert_eq!(
            resolve_introspection_clients(Some(" , ")),
            Vec::<String>::new(),
            "bug: a blank entry named an introspection client"
        );
        assert_eq!(
            resolve_introspection_clients(Some(" mcp_client_b, mcp_client_a ,mcp_client_b,")),
            ["mcp_client_b", "mcp_client_a"],
            "bug: the list was not trimmed, deduplicated and kept in order"
        );
    }
}
