-- A per-guild counter of everything that changes who can see what: members, roles, role
-- assignments and the owner. The server caches each guild's members and roles for event fan-out
-- and checks this counter before using the cache, so triggers bump it: no write path, in this
-- process or another one, can forget to. Channel overwrites are read fresh and need no counter.
ALTER TABLE guilds ADD COLUMN permission_version INTEGER NOT NULL DEFAULT 0;

CREATE TRIGGER guild_members_insert_bumps_permission_version AFTER INSERT ON guild_members
BEGIN
    UPDATE guilds SET permission_version = permission_version + 1 WHERE id = NEW.guild_id;
END;

CREATE TRIGGER guild_members_delete_bumps_permission_version AFTER DELETE ON guild_members
BEGIN
    UPDATE guilds SET permission_version = permission_version + 1 WHERE id = OLD.guild_id;
END;

CREATE TRIGGER guild_members_update_bumps_permission_version AFTER UPDATE ON guild_members
BEGIN
    UPDATE guilds SET permission_version = permission_version + 1
    WHERE id IN (OLD.guild_id, NEW.guild_id);
END;

CREATE TRIGGER member_roles_insert_bumps_permission_version AFTER INSERT ON member_roles
BEGIN
    UPDATE guilds SET permission_version = permission_version + 1 WHERE id = NEW.guild_id;
END;

CREATE TRIGGER member_roles_delete_bumps_permission_version AFTER DELETE ON member_roles
BEGIN
    UPDATE guilds SET permission_version = permission_version + 1 WHERE id = OLD.guild_id;
END;

CREATE TRIGGER member_roles_update_bumps_permission_version AFTER UPDATE ON member_roles
BEGIN
    UPDATE guilds SET permission_version = permission_version + 1
    WHERE id IN (OLD.guild_id, NEW.guild_id);
END;

CREATE TRIGGER roles_insert_bumps_permission_version AFTER INSERT ON roles
BEGIN
    UPDATE guilds SET permission_version = permission_version + 1 WHERE id = NEW.guild_id;
END;

CREATE TRIGGER roles_delete_bumps_permission_version AFTER DELETE ON roles
BEGIN
    UPDATE guilds SET permission_version = permission_version + 1 WHERE id = OLD.guild_id;
END;

CREATE TRIGGER roles_update_bumps_permission_version AFTER UPDATE ON roles
BEGIN
    UPDATE guilds SET permission_version = permission_version + 1
    WHERE id IN (OLD.guild_id, NEW.guild_id);
END;

CREATE TRIGGER guilds_owner_update_bumps_permission_version AFTER UPDATE OF owner_id ON guilds
BEGIN
    UPDATE guilds SET permission_version = permission_version + 1 WHERE id = NEW.id;
END;
