# Mellow with Docker Compose

For hosts where every app runs as a Compose stack. The image is built locally from the signed
release binary: the build downloads `mellow-server` for the host's architecture and refuses it
unless its minisign signature verifies against `minisign.pub` with the file's own name as trusted
comment ([ADR-0016](../../docs/adr/0016-sign-releases-in-a-protected-ci-environment.md)). Without
Docker, run the release binary directly (see the main README).

## Install
```bash
sudo mkdir -p /srv/stacks/mellow && cd /srv/stacks/mellow
# Copy Dockerfile, compose.yaml and minisign.pub from this directory, then:
mkdir -p config data keys
docker compose build
# config and keys are read-only for serve: mount them writable for init only.
docker compose run --rm -v ./config:/config -v ./keys:/keys mellow init \
  --config /config/mellow.toml --data-directory /data --master-key-file /keys/master.key
```
Then, behind a TLS reverse proxy on the same Docker network (Caddy, Traefik…), set in
`config/mellow.toml`: `listen_address = "0.0.0.0:8080"`, `behind_tls_reverse_proxy = true` and
`trusted_proxy_networks = ["<the proxy container's address>/32"]` (never the whole Docker
network), add the service to the proxy's network in `compose.yaml`, and route the domain to
`mellow:8080`. Without a proxy, run `init` with `--domain chat.example.org` instead (built-in TLS
with ACME, ADR-0013), publish port 443 and set the service's `environment` to
`MELLOW_HEALTH_URL: https://chat.example.org/api/v1/health` for the health check. Start with
`docker compose up -d`.

Back up `keys` separately from `data`: without the master key the database cannot be decrypted.

## Update
```bash
sudo install -m 0755 mellow-update /usr/local/sbin/mellow-update   # once
mellow-update --check     # installed and latest versions
mellow-update             # latest release
mellow-update 0.2.0       # a given version
```
`mellow-update` (set `MELLOW_STACK_DIR` if the stack is not in `/srv/stacks/mellow`) moves both
version strings of `compose.yaml` forward, builds the image (download and signature check), starts
it and waits for the health check to report the new version. If the new version does not become
healthy it puts the previous `compose.yaml` back and restarts the previous image, unless the
database was already migrated: an older server refuses a newer database, so the script stops and
says how to restore the pre-migration backup that the server wrote to `data/backups/`.
