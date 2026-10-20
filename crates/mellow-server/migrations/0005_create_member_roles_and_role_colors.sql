-- Named roles (ADR-0007 step 2): which members hold which roles, and role colours.

ALTER TABLE roles ADD COLUMN color INTEGER NOT NULL DEFAULT 0 CHECK (color BETWEEN 0 AND 16777215);

-- Leaving the guild or deleting the role drops the assignment. The application checks that the
-- role belongs to the same guild; `@everyone` (id = guild id) is never stored here.
CREATE TABLE member_roles (
    guild_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    role_id INTEGER NOT NULL REFERENCES roles (id) ON DELETE CASCADE,
    PRIMARY KEY (guild_id, user_id, role_id),
    FOREIGN KEY (guild_id, user_id) REFERENCES guild_members (guild_id, user_id) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;

-- ON DELETE CASCADE from roles looks assignments up by role.
CREATE INDEX member_roles_by_role_id ON member_roles (role_id);

CREATE INDEX roles_by_guild_id_and_position ON roles (guild_id, position);
DROP INDEX roles_by_guild_id;
