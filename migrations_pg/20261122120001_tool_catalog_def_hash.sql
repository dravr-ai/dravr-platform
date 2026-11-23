-- ABOUTME: Records the hash of each tool's definition as tools/list last published it (PostgreSQL)
-- ABOUTME: Adds nullable tool_catalog.def_hash, rewritten whenever a tool's definition changes (owl row 38, carnet#1543)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- def_hash: dravr_tronc Tool::def_hash ('sha256:' + 64 hex digits) of the
-- tool's full definition as tools/list publishes it, the hash served to
-- clients at _meta["ai.dravr.vault/tool"].def_hash. A caller offered fewer
-- arguments (an athlete is not offered a coach tool's `athlete`) is served
-- another definition, with its own hash. Rewritten at boot and then every
-- five minutes (a contremaitre description sync changes a definition
-- between boots), for each tool whose definition changed, with a log line
-- naming it: the tools a client that pins definitions will alert on. NULL
-- until the first write after this migration. Additive and nullable.
ALTER TABLE tool_catalog ADD COLUMN IF NOT EXISTS def_hash VARCHAR(255);
