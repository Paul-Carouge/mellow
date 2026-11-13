#!/bin/bash
# Exports MELLOW_VERSION for the release jobs (ADR-0016). On a tag run, the tag must be that
# version and a commit of main; pull request dry runs only read the version.
set -euo pipefail

version="$(sed -n 's/^version = "\([^"]*\)".*/\1/p' Cargo.toml | head -n 1)"
# Plain X.Y.Z only: no pre-release may become the "latest" release that updaters install.
[[ "${version}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "version '${version}' is not X.Y.Z"; exit 1; }
if [[ "${GITHUB_REF_TYPE}" == "tag" ]]; then
  test "v${version}" = "${GITHUB_REF_NAME}" || { echo "tag ${GITHUB_REF_NAME} != v${version}"; exit 1; }
  git merge-base --is-ancestor "${GITHUB_SHA}" origin/main || { echo "${GITHUB_SHA} is not on main"; exit 1; }
fi
echo "MELLOW_VERSION=${version}" >> "${GITHUB_ENV}"
