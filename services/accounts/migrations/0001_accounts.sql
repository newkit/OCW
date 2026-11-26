-- The accounts backend on PostgreSQL (spec 14 Phase 4, 4O W5a): accounts,
-- their linked sign-in identities, refresh tokens (only their SHA-256)
-- and the encrypted provider tokens. Applied by `sqlx::migrate!` when the
-- backend starts (services/accounts/src/pg.rs); never edit an applied
-- migration, add a new one.

CREATE TABLE accounts (
    id           TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    created_at   BIGINT NOT NULL
);

CREATE TABLE identities (
    provider  TEXT NOT NULL,
    subject   TEXT NOT NULL,
    account   TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    name      TEXT NOT NULL,
    email     TEXT,
    linked_at BIGINT NOT NULL,
    upload    BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (provider, subject),
    -- One identity per provider and account (linking another replaces it).
    UNIQUE (account, provider)
);

CREATE TABLE refresh_tokens (
    hash           TEXT PRIMARY KEY,
    family         TEXT NOT NULL,
    account        TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    client_id      TEXT NOT NULL,
    scope          TEXT NOT NULL,
    auth_time      BIGINT NOT NULL,
    family_created BIGINT NOT NULL,
    created_at     BIGINT NOT NULL,
    used           BOOLEAN NOT NULL DEFAULT FALSE,
    revoked        BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE INDEX refresh_tokens_family ON refresh_tokens (family);
CREATE INDEX refresh_tokens_account ON refresh_tokens (account);
CREATE INDEX refresh_tokens_created ON refresh_tokens (created_at);

CREATE TABLE provider_secrets (
    account    TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    provider   TEXT NOT NULL,
    nonce      TEXT NOT NULL,
    ciphertext TEXT NOT NULL,
    PRIMARY KEY (account, provider)
);
