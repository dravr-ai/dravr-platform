-- ABOUTME: Binds every agentless messaging DM to an agent — the athlete's selection, else the tenant's first system agent
-- ABOUTME: The forge now falls back the same way (conversation_forge::selected_or_system_agent); this repairs the rows it predates

-- A messaging DM forged while the athlete had no selected agent ran on the
-- house prompt with nothing to name it after. The room bootstrap always fell
-- back to the tenant's first system agent; the DM forge did not. Give the
-- legacy rows the answer the next turn would compute: the current selection,
-- else the newest system agent of the tenant (`list_system_agents` orders by
-- created_at DESC and the forge takes the first). A row in a tenant with no
-- system agent at all keeps NULL.
UPDATE chat_conversations
SET agent_id = COALESCE(
    (SELECT tu.selected_agent_id
       FROM tenant_users tu
      WHERE tu.user_id = chat_conversations.user_id
        AND tu.tenant_id = chat_conversations.tenant_id),
    (SELECT a.id
       FROM agents a
      WHERE a.tenant_id = chat_conversations.tenant_id
        AND a.is_system = 1
      ORDER BY a.created_at DESC
      LIMIT 1)
)
WHERE agent_id IS NULL
  AND group_id IS NULL
  AND channel_type NOT IN ('web', 'mobile');
