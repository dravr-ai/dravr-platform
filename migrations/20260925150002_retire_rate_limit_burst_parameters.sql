-- ABOUTME: Retires the admin config rate_limiting category, whose three burst parameters nothing read
-- ABOUTME: Drops its overrides and category row so the catalog stops offering settings that do nothing

-- rate_limit.free_tier_burst, rate_limit.professional_burst and
-- rate_limit.enterprise_burst were the category's only parameters. No
-- request path enforces a per-minute burst, so an operator could change them
-- and nothing would happen; the catalog entries are gone from the code, and
-- the category row would otherwise render as an empty section.
DELETE FROM admin_config_overrides WHERE category = 'rate_limiting';
DELETE FROM admin_config_categories WHERE name = 'rate_limiting';
