# Mellow with Docker Compose

For hosts where every app runs as a Compose stack. The image is built locally from the signed
release binary: the build downloads `mellow-server` for the host's architecture and refuses it
unless its minisign signature verifies against `minisign.pub` with the file's own name as trusted
comment ([ADR-0016](../../docs/adr/0016-sign-releases-in-a-protected-ci-environment.md)). Without
Docker, run the release binary directly (see the main README).

## Install
The container runs as uid and gid 10001, which must own the directories it writes.
```bash
sudo mkdir -p /srv/stacks/mellow && cd /srv/stacks/mellow
# Copy Dockerfile, compose.yaml, minisign.pub and mellow-update from this directory, then:
sudo mkdir -p config data keys certificates
sudo chown 10001:10001 config data keys certificates
sudo chmod 700 data keys certificates
sudo docker compose build --pull
# config and keys are read-only for serve: mount them writable for init only.
sudo docker compose run --rm -v ./config:/config -v ./keys:/keys mellow init \
  --config /config/mellow.toml --data-directory /data --master-key-file /keys/master.key
```
Then, behind a TLS reverse proxy on the same Docker network (Caddy, Traefik…), set in
`config/mellow.toml`: `listen_address = "0.0.0.0:8080"`, `behind_tls_reverse_proxy = true` and
`trusted_proxy_networks = ["<the proxy container's address>/32"]` (never the whole Docker
network; give the proxy a fixed address, see "Behind a reverse proxy" in the main README), add the
service to the proxy's network in `compose.yaml`, and route the domain to `mellow:8080`.

Without a proxy, run `init` with `--domain chat.example.org --certificate-directory /certificates`
instead (built-in TLS with ACME, ADR-0013), publish port 443, and set the service's `environment`
to `MELLOW_HEALTH_URL: https://chat.example.org/api/v1/health` for the health check.

Registration is closed by default, so create the first administrator, then start:
```bash
sudo docker compose run --rm mellow admin create-user --config /config/mellow.toml \
  --username <name> --email <address> --administrator
sudo docker compose up -d
```
Back up `keys` separately from `data`: without the master key the database cannot be decrypted.

## Update
```bash
sudo install -m 0755 mellow-update /usr/local/sbin/mellow-update   # once
sudo mellow-update --check     # installed and latest versions
sudo mellow-update             # latest release
sudo mellow-update 0.2.0       # a given version
```
`mellow-update` (set `MELLOW_STACK_DIR` if the stack is not in `/srv/stacks/mellow`) moves both
version strings of `compose.yaml` forward, builds the image (download, signature check, fresh
Debian base), starts it and waits for the health check to report the new version. If the new
version does not come up and the database was not touched, it puts the previous `compose.yaml`
back and restarts the previous image. If a pre-migration backup appeared meanwhile, it rolls
nothing back, since an older server refuses a migrated database, and prints how to restore the
backup ([ADR-0018](../../docs/adr/0018-roll-back-the-executable-only-and-guard-the-install-location.md)).

The kit's own files can change between releases: when an update fails because of the `Dockerfile`
or a new `minisign.pub` (a key rotation announced in the release notes), copy them again from the
new release's `deploy/docker`.
