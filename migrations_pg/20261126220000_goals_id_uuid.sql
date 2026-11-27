-- ABOUTME: Declares goals.id a uuid on PostgreSQL, as the deployed database already holds it
-- ABOUTME: Guarded on the column's current type, so a database where it is already uuid is left untouched

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- 20260604000001 created `goals` with `id TEXT`, but `CREATE TABLE IF NOT
-- EXISTS` found the deployed database already holding a `goals` table whose
-- `id` is a uuid, and left it so. Every set_goal there then failed with
-- `column "id" is of type uuid but expression is of type text` (2026-11-26),
-- while a database built from these migrations — every CI run — kept TEXT and
-- passed. This brings the built databases to the deployed type, so the type
-- the code binds is the one it is tested against. create_goal binds a native
-- uuid (`NativeUuid`), and every stored id is a `Uuid::new_v4()` rendering, so
-- the cast holds for each row.
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
         WHERE table_schema = current_schema()
           AND table_name = 'goals'
           AND column_name = 'id'
           AND data_type = 'text'
    ) THEN
        ALTER TABLE goals ALTER COLUMN id TYPE uuid USING id::uuid;
    END IF;
END $$;
