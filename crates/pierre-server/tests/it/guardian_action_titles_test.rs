// ABOUTME: Holds every tool a Guardian block reply can name to a localized title in every locale
// ABOUTME: The reply is localized, so a new consequential tool without a title would read in English
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A new tool that writes data or is irreversible can appear in a Guardian block's
//! reply. The stage falls back to the tool's English title rather than to nothing,
//! so without this test the gap would only ever show in a French reply.

use dravr_tronc::mcp::tool::ToolCapabilities;
use pierre_contremaitre::messaging_strings::{guardian_action_title_key, MessagingStringsRegistry};
use pierre_core::models::SUPPORTED_LOCALES;
use pierre_mcp_server::tools::registry_builtin::register_builtin_tools;
use pierre_tool_runtime::registry::ToolRegistry;

#[test]
fn every_tool_a_block_can_name_has_a_title_in_every_locale() {
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);
    let strings = MessagingStringsRegistry::new();

    let mut consequential = 0_usize;
    let mut missing = Vec::new();
    for name in registry.tool_names() {
        let tool = registry.get(name).expect("a listed tool resolves");
        if !tool.capabilities().contains(ToolCapabilities::WRITES_DATA)
            && !tool.security_class().is_irreversible()
        {
            continue;
        }
        consequential += 1;
        let key = guardian_action_title_key(name);
        for locale in SUPPORTED_LOCALES {
            // `sha256` answers for this exact locale, where `get` would fall back.
            if strings.sha256(&key, locale).is_none() {
                missing.push(format!("{key} [{locale}]"));
            }
        }
    }

    assert!(consequential > 0, "the registry must hold writing tools");
    assert_eq!(
        missing,
        Vec::<String>::new(),
        "add these to dravr-contremaitre strings/ and server-rendered-keys.txt"
    );
}
