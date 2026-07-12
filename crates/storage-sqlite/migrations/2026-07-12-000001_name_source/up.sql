-- Add name_source to accounts to distinguish provider-sourced names from user overrides.
-- Default is provider so existing accounts are treated as provider-sourced.
ALTER TABLE accounts ADD COLUMN name_source TEXT NOT NULL DEFAULT 'provider';

-- Update provider_identities to track what display name the provider last returned.
ALTER TABLE provider_identities ADD COLUMN provider_display_name TEXT;
