-- The workshop and hearts of an account (spec 14 Phase 4, 4O W5c / W5d):
-- the balance cached in the account row (the sum of the ledger, written in
-- the same transaction, never below zero), the append-only hearts ledger,
-- the garage (items, bikes, the worn kit), idempotency keys and the rides
-- credited, held or refused.

ALTER TABLE accounts
    ADD COLUMN hearts BIGINT NOT NULL DEFAULT 0
        CONSTRAINT hearts_never_negative CHECK (hearts >= 0),
    -- The 5,000 ♥ start was booked and the starter garage made (Q21 (a)).
    ADD COLUMN workshop_started BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN active_bike TEXT NOT NULL DEFAULT '',
    ADD COLUMN next_item INTEGER NOT NULL DEFAULT 1;

CREATE TABLE hearts_ledger (
    id        BIGSERIAL PRIMARY KEY,
    account   TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    delta     BIGINT NOT NULL,
    reason    TEXT NOT NULL CHECK (reason IN (
        'start', 'ride', 'workout', 'event', 'achievement',
        'purchase', 'sale', 'repair', 'grant')),
    reference TEXT NOT NULL DEFAULT '',
    at        BIGINT NOT NULL
);
CREATE INDEX hearts_ledger_account ON hearts_ledger (account, id);
-- A ride is credited once.
CREATE UNIQUE INDEX hearts_ledger_ride_once ON hearts_ledger (account, reference)
    WHERE reason = 'ride';
-- One start per account.
CREATE UNIQUE INDEX hearts_ledger_start_once ON hearts_ledger (account)
    WHERE reason = 'start';

-- Append-only: lines are never changed (an account's deletion removes
-- them, spec 12).
CREATE FUNCTION hearts_ledger_append_only() RETURNS trigger
    LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'hearts_ledger is append-only';
END
$$;
CREATE TRIGGER hearts_ledger_no_update BEFORE UPDATE ON hearts_ledger
    FOR EACH ROW EXECUTE FUNCTION hearts_ledger_append_only();

CREATE TABLE items (
    account   TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    id        INTEGER NOT NULL,
    -- The catalogue part (assets/parts.toml).
    part      TEXT NOT NULL,
    paid      INTEGER NOT NULL CHECK (paid >= 0),
    bought    TEXT NOT NULL,
    ridden_m  DOUBLE PRECISION NOT NULL DEFAULT 0,
    -- The groupset item a crankset, cassette or chain came with.
    with_item INTEGER,
    PRIMARY KEY (account, id)
);

CREATE TABLE bikes (
    account   TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    id        TEXT NOT NULL,
    -- The hook on the wall (the garage's order).
    hook      INTEGER NOT NULL,
    profile   TEXT NOT NULL,
    paint     TEXT,
    starter   BOOLEAN NOT NULL DEFAULT FALSE,
    -- Slot id → item id; a slot without one has its base part (Q18 (a)).
    parts     JSONB NOT NULL DEFAULT '{}',
    -- Damage part id → condition per mille.
    condition JSONB NOT NULL DEFAULT '{}',
    PRIMARY KEY (account, id)
);

CREATE TABLE kit (
    account TEXT NOT NULL,
    slot    TEXT NOT NULL,
    item    INTEGER NOT NULL,
    PRIMARY KEY (account, slot),
    FOREIGN KEY (account, item) REFERENCES items (account, id) ON DELETE CASCADE
);

CREATE TABLE idempotency (
    account      TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    key          TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    answer       JSONB NOT NULL,
    at           BIGINT NOT NULL,
    PRIMARY KEY (account, key)
);

CREATE TABLE ride_uploads (
    account    TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    -- `server:<key id>:<ride>` (a signed credit) or `app:<ride>` (a summary).
    ride_id    TEXT NOT NULL,
    source     TEXT NOT NULL CHECK (source IN ('server', 'summary')),
    server_key TEXT,
    status     TEXT NOT NULL CHECK (status IN ('credited', 'flagged', 'refused')),
    -- What the ride is worth (credited only when status = 'credited').
    hearts     INTEGER NOT NULL CHECK (hearts >= 0),
    started    BIGINT NOT NULL,
    ended      BIGINT NOT NULL,
    reasons    JSONB NOT NULL DEFAULT '[]',
    body       JSONB NOT NULL,
    at         BIGINT NOT NULL,
    PRIMARY KEY (account, ride_id)
);
CREATE INDEX ride_uploads_time ON ride_uploads (account, started, ended);
