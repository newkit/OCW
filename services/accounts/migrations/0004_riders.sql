-- The rider of an account (spec 14 Phase 4, 4O W5b; 4N): one per account,
-- its name unique on the backend (`name_key`, ocw_core::name::key: case,
-- accents and look-alikes folded), the apps' rider profile as JSON.

CREATE TABLE riders (
    account    TEXT PRIMARY KEY REFERENCES accounts (id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    name_key   TEXT NOT NULL UNIQUE,
    doc        JSONB NOT NULL,
    updated_at BIGINT NOT NULL
);
