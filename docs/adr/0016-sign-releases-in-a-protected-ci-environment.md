# 0016. Build and sign releases in a protected CI environment

- Status: Accepted
- Date: 2026-10-20
- Amends: [ADR-0003](0003-self-update-with-signed-releases.md) (where the signing key lives)

## Context
ADR-0003 makes `mellow-server update` verify an Ed25519 minisign signature against a public key built into the running binary, and asked for the private key to be kept offline. Releases must exist for three operating systems and two CPU architectures; building them by hand on one maintainer's machine is slow and error-prone, and cross-compiling SQLCipher's vendored OpenSSL is fragile. Self-hosters running Docker (the maintainer's own instance among them) also need a published binary to build their image from, the way other self-hosted apps are packaged.

## Decision
- A tag `vX.Y.Z` pushed to the repository starts `.github/workflows/release.yml`. It builds `mellow-server` and `mellow-desktop` natively on GitHub-hosted runners for `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `aarch64-apple-darwin` and `x86_64-pc-windows-msvc`, from the tagged commit with `--locked` and no build cache. The tag must be a plain `X.Y.Z` equal to the workspace version, on a commit of `main`.
- Signing runs in a separate job bound to the GitHub environment `release`, which only tags `v*` may use and which waits for the maintainer's manual approval. The minisign secret key is an environment secret (`MINISIGN_SECRET_KEY`), written to a private temporary file for the signing step only. Build jobs never see it.
- Each asset is named `<binary>-<version>-<target>[.exe]` and gets a `.minisig` whose trusted comment is `<asset name>`, so a valid signature cannot be replayed for another file, platform or version. A `mellow-<version>-SHA256SUMS` digest list is published and signed the same way. The publish job refuses any set of assets other than the expected one.
- The public key is `release/minisign.pub` in the repository and is compiled into every binary that updates itself. Rotation is preventive only: a release signed by the old key ships the new public key, then the old secret is deleted. After a leak there is no automatic recovery, since deployed binaries keep trusting the leaked key: publish a security advisory and ask self-hosters to reinstall by hand.
- The maintainer keeps an offline copy of the secret key (password manager), so a lost GitHub account does not end updates; that copy is a second place the key can leak from.
- `v*` tags cannot be moved or deleted (repository ruleset), published releases are immutable, and administrators cannot bypass the environment's approval.

## Consequences
- A release is one `git tag` plus one approval, with no maintainer machine involved.
- The secret key now lives in GitHub: whoever controls the maintainer's GitHub account, or a compromised workflow running in the `release` environment, can sign a release. A classic token with the `repo` scope is enough to push a tag and approve the deployment through the API, without the web session and its second factor: the maintainer's day-to-day tokens (CLI, agents) should be fine-grained, limited to this repository and without deployment or administration rights. The workflow that receives the key is the one of the tagged commit, so the approver checks that commit first (README).
- Mitigations: the environment is limited to `v*` tags and needs approval, actions are pinned by commit hash, the signing job runs no build step and no action besides GitHub's own `download-artifact`. Its trusted base still includes GitHub's runner image and the Ubuntu archive it installs minisign from.
- A signature proves "built by this CI from this tag", not "safe": it is only as good as the dependency chain (`--locked`, `cargo deny`).
- A signature never expires, so a compromised release host can hold back a security update (freeze attack); updaters only refuse older versions.
- Releases depend on GitHub Actions being available to the account.
- The trusted-comment check and the version comparison must be implemented by every updater; a signature alone does not prove which file it covers.

## Alternatives considered
- **Offline key, signing on the maintainer's Mac** (ADR-0003 as written): strongest protection of the key, but every release needs that machine and a manual download-sign-upload round trip; rejected by the maintainer for now.
- **Sigstore keyless signing (cosign)**: ties trust to GitHub's OIDC identity rather than a key we control, and would add a heavy verification dependency to the updater.
- **No signature, `SHA256SUMS` over HTTPS only**: a compromised release host would compromise every instance (ADR-0003).
