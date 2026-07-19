CREATE TABLE provider_account_connections (
    account_id TEXT PRIMARY KEY NOT NULL,
    provider TEXT NOT NULL,
    connection_id TEXT NOT NULL,
    provider_account_id TEXT NOT NULL,
    provider_generation INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(provider, connection_id, provider_account_id),
    FOREIGN KEY(account_id) REFERENCES accounts(id) ON DELETE CASCADE
);

CREATE TRIGGER provider_account_connections_identity_immutable
BEFORE UPDATE OF provider, connection_id, provider_account_id ON provider_account_connections
BEGIN
    SELECT RAISE(ABORT, 'provider account connection identity is immutable');
END;

-- Generation is a fencing contract, not provider identity. Relinking the same
-- immutable provider account may only advance it; stale or equal rewrites fail.
CREATE TRIGGER provider_account_connections_generation_monotonic
BEFORE UPDATE OF provider_generation ON provider_account_connections
WHEN NEW.provider_generation <= OLD.provider_generation
BEGIN
    SELECT RAISE(ABORT, 'provider account connection generation must increase');
END;
