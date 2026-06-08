export const meta = {
  name: 'kms-phase1-delete-dead-system',
  description: 'ADR-017 Phase 1: delete the unwired TenantEncryptionManager/KeyRotationManager subsystem',
  phases: [
    { title: 'Map', detail: 'parallel read-only deletion manifest per area' },
    { title: 'Execute', detail: 'apply all deletions in the worktree, compile clean' },
    { title: 'Verify', detail: 'workspace build + targeted tests + per-crate clippy' },
  ],
}

const WT = '/Users/jeanfrancoisarcand/workspace/pierre_mcp_server-feature-kms-envelope-encryption'

const RULES = `Project rules (CLAUDE.md) you MUST obey:
- NEVER use #[allow(...)], dead_code, stubs, commented-out code, or _-prefixed throwaways to make it compile.
- NEVER use anyhow. Keep AppError/AppResult.
- If you cannot reach a clean build by genuine deletion, STOP and report the blocker — do not hack around it.
- Files start with two "ABOUTME: " comment lines; preserve that convention on any file you edit.
- All work happens under the worktree ${WT} using absolute paths. Do NOT touch the main checkout.`

const KEEP = `MUST KEEP (these are the LIVE encryption path and general security utils — do NOT remove):
- Database::encrypt_data_with_aad / decrypt_data_with_aad and the HasEncryption trait (pierre-database).
- SecurityRepository's encrypt_data_with_aad / decrypt_data_with_aad methods (only the KEY-VERSION methods are dead).
- pierre-auth security submodules: audit.rs, cookies.rs, csrf.rs, the headers module, and audit_security_headers().`

const MANIFEST_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['area', 'files_to_delete', 'edits', 'test_files_to_delete', 'migrations_to_add', 'notes'],
  properties: {
    area: { type: 'string' },
    files_to_delete: { type: 'array', items: { type: 'string' }, description: 'absolute paths to delete entirely' },
    edits: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        required: ['file', 'what', 'reason'],
        properties: {
          file: { type: 'string' },
          what: { type: 'string', description: 'exact symbol / trait method / use / re-export / mod decl to remove' },
          reason: { type: 'string', description: 'evidence it is dead (no live callers)' },
        },
      },
    },
    test_files_to_delete: { type: 'array', items: { type: 'string' } },
    migrations_to_add: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        required: ['path', 'sql'],
        properties: { path: { type: 'string' }, sql: { type: 'string' } },
      },
    },
    notes: { type: 'string', description: 'anything ambiguous or any live reference found (which would BLOCK deletion)' },
  },
}

phase('Map')
const AREAS = [
  {
    key: 'pierre-auth',
    prompt: `${RULES}\n${KEEP}\n\nMap every part of the DEAD encryption/rotation subsystem in crates/pierre-auth/src/security/ under ${WT}.
Dead symbols: TenantEncryptionManager, EncryptionMetadata, EncryptedData, EncryptionStats, EnhancedEncryptedToken, the entire key_rotation.rs (KeyRotationManager, KeyRotationStats), the \`use key_rotation::KeyVersion\`, and \`pub mod key_rotation\`.
Confirm via ripgrep that none of these have callers outside security/mod.rs and security/key_rotation.rs (tests included — list test files separately). Produce the manifest: key_rotation.rs deleted entirely; mod.rs edits to remove only the dead structs/impls while KEEPING audit/cookies/csrf/headers/audit_security_headers and the SecurityConfig headers module.`,
  },
  {
    key: 'pierre-database',
    prompt: `${RULES}\n${KEEP}\n\nIn crates/pierre-database/src under ${WT}, map the DEAD key-version methods of SecurityRepository: store_key_version, get_key_versions, update_key_version_status, delete_old_key_versions. Find the trait definition and EVERY impl (postgres backend, sqlite/Database impl in database/security_repository.rs, and any other). Confirm via ripgrep they have no live callers (only the deleted KeyRotationManager used them). For each, give the file + exact method signature to remove. Also locate the \`key_versions\` table in migrations/ and migrations_pg/ and propose a DROP migration for BOTH backends dated 20260529000001 (the table is empty in practice — nothing instantiates a writer). Do NOT remove encrypt_data_with_aad/decrypt_data_with_aad.`,
  },
  {
    key: 'pierre-core',
    prompt: `${RULES}\n\nIn crates/pierre-core/src/models under ${WT}, map the DEAD types in key_rotation.rs (KeyVersion, KeyRotationConfig, RotationStatus) and their re-export in models/mod.rs. Confirm via ripgrep that after the pierre-auth KeyRotationManager and pierre-database key-version methods are removed, these types have NO remaining references anywhere in crates/ (non-test and test). If anything live still references them, say so in notes — that BLOCKS deletion. Propose deleting models/key_rotation.rs and removing the re-export line.`,
  },
  {
    key: 'tests',
    prompt: `${RULES}\n\nUnder ${WT}, find every test file in crates/*/tests/ that references TenantEncryptionManager, KeyRotationManager, EnhancedEncryptedToken, EncryptionMetadata, EncryptionStats, KeyVersion, KeyRotationConfig, RotationStatus, store_key_version, get_key_versions, update_key_version_status, or delete_old_key_versions. For each, decide: delete the whole file (if it ONLY tests the dead subsystem) or list the specific tests/blocks to remove (if it mixes live + dead). Report under test_files_to_delete (whole-file) and edits (partial).`,
  },
]
const manifests = (await parallel(
  AREAS.map((a) => () => agent(a.prompt, { label: `map:${a.key}`, phase: 'Map', schema: MANIFEST_SCHEMA }))
)).filter(Boolean)

const blockers = manifests.filter((m) => /live|BLOCK|still reference/i.test(m.notes || ''))
if (blockers.length) {
  log(`BLOCKERS found in mapping — not executing deletion: ${blockers.map((b) => b.area).join(', ')}`)
  return { blocked: true, manifests }
}

phase('Execute')
const EXEC_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['build_clean', 'files_deleted', 'files_edited', 'migrations_added', 'summary'],
  properties: {
    build_clean: { type: 'boolean', description: 'true only if cargo build of the touched crates succeeded' },
    files_deleted: { type: 'array', items: { type: 'string' } },
    files_edited: { type: 'array', items: { type: 'string' } },
    migrations_added: { type: 'array', items: { type: 'string' } },
    summary: { type: 'string' },
    blocker: { type: 'string', description: 'empty if none; otherwise why a clean build was not reached' },
  },
}
const execReport = await agent(
  `${RULES}\n${KEEP}\n\nApply this verified deletion manifest (JSON array, one entry per area):\n${JSON.stringify(manifests)}\n\n` +
    `Steps:\n1) Delete the files in files_to_delete and test_files_to_delete.\n` +
    `2) Apply every edit (remove the named symbol / trait method / use / re-export / mod decl).\n` +
    `3) Add the DROP migrations for key_versions (both backends) with proper ABOUTME headers.\n` +
    `4) Run \`cargo build -p pierre-core -p pierre-database -p pierre-auth\` in ${WT} and iterate by genuine deletion only until it is clean.\n` +
    `Set build_clean=true ONLY if that build passed. If you cannot get a clean build by deletion alone, set build_clean=false and explain in blocker.`,
  { label: 'execute-deletion', phase: 'Execute', schema: EXEC_SCHEMA }
)

if (!execReport || !execReport.build_clean) {
  log(`Execution did not reach a clean build: ${execReport ? execReport.blocker : 'agent returned null'}`)
  return { executed: false, manifests, execReport }
}

phase('Verify')
const VERIFY_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['ok', 'detail'],
  properties: { ok: { type: 'boolean' }, detail: { type: 'string' } },
}
const checks = [
  {
    label: 'verify:workspace-build',
    prompt: `In ${WT} run \`cargo build\` for the whole workspace. Report ok=true only if it builds with zero errors. Put the tail of any error in detail.`,
  },
  {
    label: 'verify:no-residue',
    prompt: `In ${WT} ripgrep for any remaining reference to TenantEncryptionManager, KeyRotationManager, EnhancedEncryptedToken, KeyVersion, KeyRotationConfig, RotationStatus, store_key_version, update_key_version_status, delete_old_key_versions across crates/ (including tests). ok=true only if ZERO matches remain. List any matches in detail.`,
  },
  {
    label: 'verify:clippy',
    prompt: `In ${WT} run per-crate clippy: \`cargo clippy -p pierre-core -p pierre-database -p pierre-auth --all-targets --all-features -- -D warnings\`. ok=true only if clean. Tail of warnings in detail.`,
  },
]
const verify = (await parallel(
  checks.map((c) => () => agent(c.prompt, { label: c.label, phase: 'Verify', schema: VERIFY_SCHEMA }))
)).filter(Boolean)

return { executed: true, manifests, execReport, verify }
