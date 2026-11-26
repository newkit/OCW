-- The rider's injury carried over with the account (spec 14 Phase 12 Q6;
-- 4O part 4): the condition in per mille when it was kept and when (Unix
-- s), as the app reports it after a ride; NULL: fine.

ALTER TABLE accounts
    ADD COLUMN injury_pm INTEGER
        CONSTRAINT injury_pm_range CHECK (injury_pm BETWEEN 0 AND 999),
    ADD COLUMN injury_at BIGINT;
