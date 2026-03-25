-- ABOUTME: Tenant-level group coaching permission configuration
-- ABOUTME: Adds group_permissions config category and default policy overrides

-- Add group permissions config category
INSERT OR IGNORE INTO admin_config_categories (id, name, display_name, description, display_order, icon) VALUES
    ('group_permissions', 'group_permissions', 'Group Permissions', 'Control who can create and manage coaching groups', 16, 'users');
