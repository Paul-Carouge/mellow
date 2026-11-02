-- A role whose members the member list shows under its name, above everyone else (Discord's
-- "hoist"). Existing roles are not displayed separately; `@everyone` never is (the domain refuses
-- it). Changing it bumps the guild's permission version through the existing roles trigger, so
-- cached snapshots carry the new value.
ALTER TABLE roles ADD COLUMN is_displayed_separately INTEGER NOT NULL DEFAULT 0
    CHECK (is_displayed_separately IN (0, 1));
