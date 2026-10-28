# 0027. Instance administrators decide who creates servers

- Status: Accepted
- Date: 2026-10-28

## Context
The owner wants whoever hosts an instance to choose whether its other accounts may create servers, or whether only the host has servers and others join them through invites. Three roles must not be confused:
- an **instance administrator** is an account the host created with `mellow-server admin create-user --administrator` (`users.is_instance_administrator`). It is the only instance-wide role, and nothing in the web client grants it;
- a server's **owner** is the account that created it (`guilds.owner_id`); they hold every permission there and nowhere else;
- a server's **administrators** are members holding the `ADMINISTRATOR` permission through one of its roles (ADR-0007); it means nothing outside that server.

## Decision
- A new setting in `mellow.toml`, `guild_creation`, says who may create servers: `"administrators"` (instance administrators only) or `"everyone"`. It is a file setting like `registration_mode` (owner's choice, 2026-10-28): changing it means editing the file and restarting, with no web screen or API to change it.
- The default is `"administrators"`, also for configurations written before this setting existed, so an instance never opens server creation to strangers without the host saying so. `init` writes the line explicitly.
- `POST /api/v1/guilds` answers `MISSING_PERMISSIONS` when the rule refuses the user; the rule is one function (`GuildCreationPolicy::allows`), checked inside `Guilds::create_guild`, so every caller gets it. `Ready.may_create_guilds` tells clients, which then offer only "Join a server".
- Servers that already exist are untouched, whoever owns them; conversations, groups and joining by invite stay open to everyone.

## Consequences
- After updating, an instance whose members created servers stops letting them create new ones until the host writes `guild_creation = "everyone"`; the CHANGELOG says so. A host whose own account was not created with `--administrator` is in the same case until they run `mellow-server admin set-administrator --username <name>` (added in `v0.17.1`; `--revoke` takes it away).
- A server downgraded below this version refuses a configuration holding `guild_creation` (unknown fields are refused): remove the line first.
- Older clients still offer "Create a server" and get an error when they use it.
- shortcut: no admin screen yet; the setting moves there with the admin web UI (ADR-0009).
