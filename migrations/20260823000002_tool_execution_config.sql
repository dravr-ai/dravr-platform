-- ABOUTME: Tenant-wide tool-loop budget configuration category
-- ABOUTME: Adds tool_execution config category backing tool_execution.max_iterations

-- Add tool execution config category
INSERT OR IGNORE INTO admin_config_categories (id, name, display_name, description, display_order, icon) VALUES
    ('tool_execution', 'tool_execution', 'Tool Execution', 'Tool-call loop budget applied to chat turns', 200, 'repeat');
