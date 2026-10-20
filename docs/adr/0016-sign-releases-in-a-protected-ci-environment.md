# 0016. Build and sign releases in a protected CI environment

- Status: Accepted
- Date: 2026-10-20
- Amends: [ADR-0003](0003-self-update-with-signed-releases.md) (where the signing key lives)

## Context
ADR-0003 makes every update (server and, now, desktop) verify an Ed25519 minisign signature against a public key built into the running binary, and asked for the private key to be kept offline. Releases must exist for three operating systems and two CPU architectures; building them by hand on one maintainer's machine is slow and error-prone, and cross-compiling SQLCipher's vendored OpenSSL is fragile. Self-hosters running Docker (the user's own instance among them) also need a published binary to build their image from, the way other self-hosted apps are packaged.

## Decision
- A tag `vX.Y.Z` pushed to the repository starts `.github/workflows/release.yml`. It builds `mellow-server` and `mellow-desktop` natively on GitHub-hosted runners for `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `aarch64-apple-darwin` and `x86_64-pc-windows-msvc`, from the tagged commit with `--locked`.
- Signing runs in a separate job bound to the GitHub environment `release`, which only tags `v*` may use and which waits for the maintainer's manual approval. The minisign secret key is an environment secret (`MINISIGN_SECRET_KEY`), written to a private temporary file for the signing step only. Build jobs never see it.
- Each asset is named `<binary>-<version>-<target>[.exe]` and gets a `.minisig` whose trusted comment is `<asset name>`, so a valid signature cannot be replayed for another file, platform or version. A `SHA256SUMS` file is published too, for installers that only check digests.
- The public key is `release/minisign.pub` in the repository and is compiled into every binary that updates itself. Rotation: a release signed by the old key ships the new public key; the old key is then revoked by removing the secret.
- The maintainer keeps an offline copy of the secret key (password manager), so a lost GitHub account does not end updates.

## Consequences
- A release is one `git tag` plus one approval, with no maintainer machine involved.
- The secret key now lives in GitHub: whoever controls the maintainer's GitHub account, or a compromised workflow running in the `release` environment, can sign a release. Mitigations: the environment is limited to `v*` tags and needs approval, actions are pinned by commit hash, the signing job runs no build step and no action besides GitHub's own `download-artifact`.
- Releases depend on GitHub Actions being available to the account (it is blocked by billing as of this date).
- The trusted-comment check and the version comparison must be implemented by every updater; a signature alone does not prove which file it covers.

## Alternatives considered
- **Offline key, signing on the maintainer's Mac** (ADR-0003 as written): strongest protection of the key, but every release needs that machine and a manual download-sign-upload round trip; rejected by the maintainer for now.
- **Sigstore keyless signing (cosign)**: ties trust to GitHub's OIDC identity rather than a key we control, and would add a heavy verification dependency to the updater.
- **No signature, `SHA256SUMS` over HTTPS only**: a compromised release host would compromise every instance (ADR-0003).
